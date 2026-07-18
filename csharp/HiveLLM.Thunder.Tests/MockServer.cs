using System.Net;
using System.Net.Security;
using System.Net.Sockets;
using System.Security.Cryptography.X509Certificates;

namespace HiveLLM.Thunder.Tests;

/// <summary>
/// Loopback responder standing in for thunder-server: the behavioral suite
/// exercises the client contract end-to-end over real sockets (mirrors the
/// tokio responders in rust/thunder-client/tests/behavior.rs).
/// </summary>
internal sealed class MockServer : IDisposable
{
    /// <summary>Frame cap the loopback responders read with.</summary>
    internal const int ServerCap = 1024 * 1024;

    private readonly TcpListener _listener;

    internal MockServer()
    {
        _listener = new TcpListener(IPAddress.Loopback, 0);
        _listener.Start();
    }

    /// <summary>Bare host:port endpoint for the client.</summary>
    internal string Address => $"127.0.0.1:{((IPEndPoint)_listener.LocalEndpoint).Port}";

    internal async Task<ServerConn> AcceptAsync() =>
        new(await _listener.AcceptTcpClientAsync());

    /// <summary>
    /// Accept one connection and serve it over TLS: wrap the accepted stream in
    /// a server <see cref="SslStream"/> and authenticate with
    /// <paramref name="serverCertificate"/> (SPEC-008 CAN-020). No client auth,
    /// mirroring the Rust server (mTLS is a later, additive capability).
    /// </summary>
    internal async Task<ServerConn> AcceptTlsAsync(X509Certificate2 serverCertificate)
    {
        var tcp = await _listener.AcceptTcpClientAsync();
        var ssl = new SslStream(tcp.GetStream(), leaveInnerStreamOpen: false);
        await ssl.AuthenticateAsServerAsync(new SslServerAuthenticationOptions
        {
            ServerCertificate = serverCertificate,
            ClientCertificateRequired = false,
        });
        return new ServerConn(tcp, ssl);
    }

    public void Dispose() => _listener.Stop();
}

/// <summary>One accepted server-side connection.</summary>
internal sealed class ServerConn : IDisposable
{
    private readonly TcpClient _tcp;
    private readonly Stream _stream;
    private byte[] _buffer = new byte[4096];
    private int _buffered;

    internal ServerConn(TcpClient tcp)
        : this(tcp, tcp.GetStream())
    {
    }

    /// <summary>Serve over an already-established stream (a plaintext <see cref="NetworkStream"/> or a TLS <see cref="SslStream"/>).</summary>
    internal ServerConn(TcpClient tcp, Stream stream)
    {
        _tcp = tcp;
        _stream = stream;
    }

    /// <summary>Read one request frame (frame-accumulating decode).</summary>
    internal async Task<Request> ReadRequestAsync()
    {
        while (true)
        {
            if (FrameCodec.TryDecodeRequest(
                    _buffer.AsMemory(0, _buffered), MockServer.ServerCap,
                    out var request, out var consumed))
            {
                Array.Copy(_buffer, consumed, _buffer, 0, _buffered - consumed);
                _buffered -= consumed;
                return request!;
            }

            if (_buffered == _buffer.Length)
            {
                Array.Resize(ref _buffer, _buffer.Length * 2);
            }

            var read = await _stream.ReadAsync(_buffer.AsMemory(_buffered));
            if (read == 0)
            {
                throw new IOException("mock server: peer closed the connection");
            }

            _buffered += read;
        }
    }

    internal Task SendOkAsync(uint id, Value value) =>
        SendRawAsync(FrameCodec.EncodeResponse(Response.Ok(id, value)));

    internal Task SendErrAsync(uint id, string message) =>
        SendRawAsync(FrameCodec.EncodeResponse(Response.Err(id, message)));

    internal async Task SendRawAsync(byte[] bytes) =>
        await _stream.WriteAsync(bytes);

    public void Dispose() => _tcp.Dispose();
}

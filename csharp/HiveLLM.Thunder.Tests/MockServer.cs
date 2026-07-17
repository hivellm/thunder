using System.Net;
using System.Net.Sockets;

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

    public void Dispose() => _listener.Stop();
}

/// <summary>One accepted server-side connection.</summary>
internal sealed class ServerConn : IDisposable
{
    private readonly TcpClient _tcp;
    private readonly NetworkStream _stream;
    private byte[] _buffer = new byte[4096];
    private int _buffered;

    internal ServerConn(TcpClient tcp)
    {
        _tcp = tcp;
        _stream = tcp.GetStream();
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

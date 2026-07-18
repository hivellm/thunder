using System.Buffers.Binary;
using System.Collections.Concurrent;
using System.Net.Security;
using System.Net.Sockets;
using System.Security.Cryptography.X509Certificates;

namespace HiveLLM.Thunder;

/// <summary>
/// The multiplexed Thunder client (SPEC-003). One client owns one TCP
/// connection (CLT-001; pooling is a layer above) and demultiplexes
/// concurrent in-flight calls over it:
/// <list type="bullet">
/// <item>ids are monotonically increasing u32s skipping
/// <see cref="Wire.PushId"/>, wrapping at the u32 range (CLT-010);</item>
/// <item>a background reader routes each response to its caller's
/// <see cref="TaskCompletionSource{T}"/> by id (CLT-010), drops unknown ids
/// (CLT-013), and poisons the connection on malformed / oversized frames —
/// every pending call fails with the same typed error (CLT-014);</item>
/// <item>writes are serialized behind a semaphore so frames never
/// interleave (CLT-011);</item>
/// <item>in-flight calls are bounded by the config's
/// <see cref="Config.MaxInFlight"/> — excess calls wait, they are not
/// refused (CLT-012);</item>
/// <item>per-call timeouts and <see cref="CancellationToken"/>s remove the
/// pending entry so a late response falls under the unknown-id drop
/// (CLT-020/021);</item>
/// <item>when a call finds the connection dead, the client lazily re-dials
/// and re-handshakes up to 2 attempts with capped backoff; calls that were
/// pending when the connection died fail typed and are never replayed
/// (CLT-030/031);</item>
/// <item>frames with <c>id == PUSH_ID</c> go to the registered push handler
/// under <see cref="PushPolicy.Enabled"/> and poison the connection under
/// <see cref="PushPolicy.Reserved"/> (CLT-060).</item>
/// </list>
/// <para>
/// The two configs are distinct and both are needed: <see cref="Config"/> is
/// the application's <em>protocol</em> — the dialect, shared by everyone who
/// talks to it — while <see cref="ClientConfig"/> is <em>this caller's</em>
/// credentials, timeouts and client name, which never affect the dialect.
/// </para>
/// </summary>
public sealed class ThunderClient : IDisposable, IAsyncDisposable
{
    /// <summary>Re-dial budget when a call finds the connection dead (CLT-030).</summary>
    private const int ReconnectAttempts = 2;

    /// <summary>
    /// Reconnect backoff: the first re-dial retries after this delay,
    /// doubling up to <see cref="BackoffCap"/> (CLT-030 "capped backoff").
    /// </summary>
    private static readonly TimeSpan BackoffBase = TimeSpan.FromMilliseconds(50);
    private static readonly TimeSpan BackoffCap = TimeSpan.FromMilliseconds(500);

    /// <summary>The application's protocol config — the dialect (PRO-001).</summary>
    private readonly Config _config;

    /// <summary>This caller's credentials/timeouts — never the dialect (CLT-002).</summary>
    private readonly ClientConfig _clientConfig;
    private readonly Endpoint _endpoint;

    /// <summary>In-flight bound sized <c>config.MaxInFlight</c> (CLT-012).</summary>
    private readonly SemaphoreSlim _inFlight;

    /// <summary>Serializes re-dial attempts so one caller reconnects at a time.</summary>
    private readonly SemaphoreSlim _reconnectLock = new(1, 1);

    private readonly CancellationTokenSource _closedCts = new();
    private readonly object _stateLock = new();
    private Conn? _conn;
    private volatile bool _closed;

    /// <summary>Monotonic id allocator, skipping <see cref="Wire.PushId"/> (CLT-010).</summary>
    private uint _nextId;

    /// <summary>Responses whose id matched no pending call (CLT-013).</summary>
    private long _unknownDrops;

    /// <summary>Push hook shared with every connection's reader (CLT-060).</summary>
    private Action<Value>? _pushHandler;

    private HandshakeInfo _handshakeInfo = HandshakeInfo.Default;

    private ThunderClient(Endpoint endpoint, Config config, ClientConfig clientConfig)
    {
        _endpoint = endpoint;
        _config = config;
        _clientConfig = clientConfig;
        _inFlight = new SemaphoreSlim(config.MaxInFlight, config.MaxInFlight);
    }

    /// <summary>
    /// Connect and run the handshake <paramref name="config"/> describes,
    /// before user calls proceed (CLT-001/002). <paramref name="endpoint"/>
    /// accepts every form of <see cref="Endpoint.Parse"/> (CLT-070):
    /// <c>scheme://host[:port]</c> — where the scheme is the application's own
    /// <see cref="Config.Scheme"/> — or bare <c>host:port</c>.
    /// </summary>
    /// <param name="endpoint">The endpoint to dial (CLT-070).</param>
    /// <param name="config">The application's protocol config — the dialect (PRO-001).</param>
    /// <param name="clientConfig">This caller's credentials/timeouts; defaults apply when null.</param>
    /// <param name="cancellationToken">Cancels the dial and handshake.</param>
    /// <exception cref="ThunderConnectionException">The endpoint is invalid or the dial failed.</exception>
    /// <exception cref="ThunderTimeoutException">The connect timeout elapsed (CLT-001).</exception>
    /// <exception cref="ThunderAuthException">The server rejected the handshake (CLT-003).</exception>
    public static async Task<ThunderClient> ConnectAsync(
        string endpoint,
        Config config,
        ClientConfig? clientConfig = null,
        CancellationToken cancellationToken = default)
    {
        ArgumentNullException.ThrowIfNull(endpoint);
        ArgumentNullException.ThrowIfNull(config);
        var parsed = Endpoint.Parse(endpoint, config);
        var client = new ThunderClient(parsed, config, clientConfig ?? new ClientConfig());
        try
        {
            var conn = await client.EstablishAsync(cancellationToken).ConfigureAwait(false);
            lock (client._stateLock)
            {
                client._conn = conn;
            }

            return client;
        }
        catch
        {
            client.Close();
            throw;
        }
    }

    /// <summary>
    /// Issue one call with the client's default timeout (CLT-020).
    /// Concurrent callers multiplex over the one connection; completion
    /// order follows the server, not submission order (CLT-010). The
    /// <paramref name="cancellationToken"/> removes the pending entry on
    /// cancel (CLT-021).
    /// </summary>
    /// <exception cref="ThunderServerException">The server answered with the Err arm.</exception>
    /// <exception cref="ThunderAuthException">The server signalled an auth failure (CLT-051).</exception>
    /// <exception cref="ThunderTimeoutException">The per-call timeout elapsed (CLT-020).</exception>
    /// <exception cref="ThunderConnectionException">The connection died or the client is closed.</exception>
    /// <exception cref="ThunderFrameTooLargeException">The server sent a frame beyond the cap (CLT-014).</exception>
    /// <exception cref="ThunderDecodeException">The server sent a malformed frame (CLT-014).</exception>
    public Task<Value> CallAsync(
        string command,
        IReadOnlyList<Value>? args = null,
        CancellationToken cancellationToken = default) =>
        CallAsync(command, args ?? System.Array.Empty<Value>(), _clientConfig.CallTimeout, cancellationToken);

    /// <summary>Issue one call with a per-call timeout override (CLT-020).</summary>
    /// <exception cref="ThunderServerException">The server answered with the Err arm.</exception>
    /// <exception cref="ThunderAuthException">The server signalled an auth failure (CLT-051).</exception>
    /// <exception cref="ThunderTimeoutException">The per-call timeout elapsed (CLT-020).</exception>
    /// <exception cref="ThunderConnectionException">The connection died or the client is closed.</exception>
    /// <exception cref="ThunderFrameTooLargeException">The server sent a frame beyond the cap (CLT-014).</exception>
    /// <exception cref="ThunderDecodeException">The server sent a malformed frame (CLT-014).</exception>
    public async Task<Value> CallAsync(
        string command,
        IReadOnlyList<Value> args,
        TimeSpan timeout,
        CancellationToken cancellationToken = default)
    {
        ArgumentNullException.ThrowIfNull(command);
        ArgumentNullException.ThrowIfNull(args);
        if (_closed)
        {
            throw ClosedError();
        }

        // CLT-012: bounded in-flight — excess calls wait here, never refused.
        await WaitInFlightAsync(cancellationToken).ConfigureAwait(false);
        try
        {
            var budget = new RedialBudget { Left = ReconnectAttempts };
            while (true)
            {
                var conn = await LiveConnAsync(budget, cancellationToken).ConfigureAwait(false);
                try
                {
                    return await DispatchAsync(conn, command, args, timeout, cancellationToken)
                        .ConfigureAwait(false);
                }
                catch (WriteFailedException writeFailed)
                {
                    // The frame never hit the wire: reconnect and resend
                    // (not a replay — CLT-031 concerns frames that were sent).
                    if (budget.Left == 0)
                    {
                        throw writeFailed.Inner;
                    }
                }
            }
        }
        finally
        {
            _inFlight.Release();
        }
    }

    /// <summary>
    /// Register the push hook (CLT-060). Frames with <c>id == PUSH_ID</c>
    /// are routed here under <see cref="PushPolicy.Enabled"/> and never
    /// matched against pending calls. The handler runs on the reader —
    /// keep it fast and offload real work to a channel; handler exceptions
    /// are swallowed.
    /// </summary>
    public void OnPush(Action<Value> handler)
    {
        ArgumentNullException.ThrowIfNull(handler);
        Volatile.Write(ref _pushHandler, handler);
    }

    /// <summary>
    /// Explicit, idempotent close (CLT-004): fails all in-flight calls with
    /// a typed connection-closed error and shuts the socket down.
    /// </summary>
    public void Close()
    {
        _closed = true;
        try
        {
            _closedCts.Cancel();
        }
        catch (ObjectDisposedException)
        {
            // Already fully disposed — idempotent.
        }

        Conn? conn;
        lock (_stateLock)
        {
            conn = _conn;
            _conn = null;
        }

        conn?.Kill(ClosedError());
    }

    /// <inheritdoc />
    public void Dispose() => Close();

    /// <inheritdoc />
    public ValueTask DisposeAsync()
    {
        Close();
        return ValueTask.CompletedTask;
    }

    /// <summary>
    /// True once the current connection's handshake authenticated
    /// (CLT-003 — auth is sticky per connection).
    /// </summary>
    public bool IsAuthenticated
    {
        get
        {
            lock (_stateLock)
            {
                return _handshakeInfo.Authenticated;
            }
        }
    }

    /// <summary>Capabilities the server advertised in the <c>HELLO</c> reply.</summary>
    public IReadOnlyList<string> Capabilities
    {
        get
        {
            lock (_stateLock)
            {
                return _handshakeInfo.Capabilities;
            }
        }
    }

    /// <summary>Snapshot of what the handshake learned (CLT-002).</summary>
    public HandshakeInfo HandshakeInfo
    {
        get
        {
            lock (_stateLock)
            {
                return _handshakeInfo;
            }
        }
    }

    /// <summary>
    /// How many responses matched no pending call and were dropped
    /// (CLT-013 — client stats, never fatal).
    /// </summary>
    public long UnknownResponseDrops => Interlocked.Read(ref _unknownDrops);

    /// <summary>
    /// True while the current connection is live — not poisoned (CLT-014) and
    /// not closed (CLT-004). The optional pool (CLT-080) uses this to drop a
    /// dead connection instead of handing it back; ordinary callers rely on
    /// typed call errors and lazy reconnect (CLT-030) rather than polling this.
    /// </summary>
    public bool IsAlive => CurrentConn() is { IsAlive: true };

    /// <summary>
    /// The application's protocol config this client drives its behavior from
    /// (PRO-001) — not to be confused with <see cref="ClientConfig"/>.
    /// </summary>
    public Config Config => _config;

    // ── internals ──────────────────────────────────────────────────────

    private static ThunderConnectionException ClosedError() => new("client is closed");

    /// <summary>
    /// Allocate the next request id from <paramref name="counter"/>:
    /// monotonic, wrapping, skipping <see cref="Wire.PushId"/> (CLT-010).
    /// </summary>
    internal static uint AllocateId(ref uint counter)
    {
        while (true)
        {
            var id = Interlocked.Increment(ref counter);
            if (id != Wire.PushId)
            {
                return id;
            }
        }
    }

    private async Task WaitInFlightAsync(CancellationToken cancellationToken)
    {
        using var linked = CancellationTokenSource.CreateLinkedTokenSource(
            cancellationToken, _closedCts.Token);
        try
        {
            await _inFlight.WaitAsync(linked.Token).ConfigureAwait(false);
        }
        catch (OperationCanceledException) when (!cancellationToken.IsCancellationRequested)
        {
            throw ClosedError();
        }
    }

    private Conn? CurrentConn()
    {
        lock (_stateLock)
        {
            return _conn;
        }
    }

    /// <summary>
    /// Return the current live connection, lazily reconnecting when it is
    /// dead or absent: up to <see cref="ReconnectAttempts"/> re-dial +
    /// re-handshake attempts with capped backoff (CLT-030). Never replays
    /// in-flight calls — those already failed typed when the connection
    /// died (CLT-031).
    /// </summary>
    private async Task<Conn> LiveConnAsync(RedialBudget budget, CancellationToken cancellationToken)
    {
        if (_closed)
        {
            throw ClosedError();
        }

        var current = CurrentConn();
        if (current is { IsAlive: true })
        {
            return current;
        }

        await _reconnectLock.WaitAsync(cancellationToken).ConfigureAwait(false);
        try
        {
            if (_closed)
            {
                throw ClosedError();
            }

            // Another caller may have reconnected while we waited.
            current = CurrentConn();
            if (current is { IsAlive: true })
            {
                return current;
            }

            ThunderException lastError = new ThunderConnectionException("connection is dead");
            var backoff = BackoffBase;
            while (budget.Left > 0)
            {
                budget.Left--;
                try
                {
                    var conn = await EstablishAsync(cancellationToken).ConfigureAwait(false);
                    lock (_stateLock)
                    {
                        _conn = conn;
                    }

                    return conn;
                }
                catch (ThunderAuthException)
                {
                    // An auth rejection is deterministic — retrying cannot fix it.
                    throw;
                }
                catch (ThunderException e)
                {
                    lastError = e;
                    if (budget.Left > 0)
                    {
                        await Task.Delay(backoff, cancellationToken).ConfigureAwait(false);
                        backoff = backoff * 2 > BackoffCap ? BackoffCap : backoff * 2;
                    }
                }
            }

            throw lastError;
        }
        finally
        {
            _reconnectLock.Release();
        }
    }

    /// <summary>
    /// Dial (with the connect timeout, TCP_NODELAY on — CLT-001), start the
    /// background reader, and run the configured handshake (CLT-002).
    /// </summary>
    private async Task<Conn> EstablishAsync(CancellationToken cancellationToken)
    {
        var tcp = new TcpClient();
        Stream stream;
        try
        {
            using (var timeoutCts = CancellationTokenSource.CreateLinkedTokenSource(cancellationToken))
            {
                timeoutCts.CancelAfter(_clientConfig.ConnectTimeout);
                try
                {
                    await tcp.ConnectAsync(_endpoint.Host, _endpoint.Port, timeoutCts.Token)
                        .ConfigureAwait(false);
                }
                catch (OperationCanceledException) when (!cancellationToken.IsCancellationRequested)
                {
                    throw new ThunderTimeoutException();
                }
                catch (Exception e) when (e is SocketException or IOException)
                {
                    throw new ThunderConnectionException(
                        $"connect to {_endpoint.Host}:{_endpoint.Port} failed: {e.Message}", e);
                }

                tcp.NoDelay = true; // CLT-001
            }

            // CLT-001 / FR-29: when TLS is configured, complete the TLS
            // handshake before any Thunder frame; a TLS setup / handshake /
            // verification failure is the Connection class, exactly like a
            // plaintext dial failure. The plaintext path keeps the bare
            // NetworkStream and is byte-for-byte unchanged.
            stream = _clientConfig.Tls is { } tlsConfig
                ? await AuthenticateTlsAsync(tcp, tlsConfig, cancellationToken).ConfigureAwait(false)
                : tcp.GetStream();
        }
        catch
        {
            tcp.Dispose();
            throw;
        }

        var conn = new Conn(tcp, stream);
        _ = Task.Run(() => ReaderLoopAsync(conn), CancellationToken.None);
        try
        {
            var info = await HandshakeAsync(conn, cancellationToken).ConfigureAwait(false);
            lock (_stateLock)
            {
                _handshakeInfo = info;
            }

            return conn;
        }
        catch
        {
            // A failed handshake tears the connection down; the error keeps
            // its own class (auth stays auth, transport stays connection).
            conn.Kill(new ThunderConnectionException("handshake failed"));
            throw;
        }
    }

    /// <summary>
    /// Wrap the connected socket in a client <see cref="SslStream"/> and
    /// authenticate before any Thunder frame (FR-29). Verification is against
    /// <see cref="ClientTls.CaPath"/> when set (a custom validation callback
    /// pinning exactly those roots), else the platform's system trust. Every
    /// TLS setup / handshake / verification failure surfaces as
    /// <see cref="ThunderConnectionException"/>, never a silent plaintext
    /// downgrade.
    /// </summary>
    private async Task<SslStream> AuthenticateTlsAsync(
        TcpClient tcp,
        ClientTls tls,
        CancellationToken cancellationToken)
    {
        // The SNI / verification name: the configured ServerName, else the host.
        var targetHost = tls.ServerName ?? _endpoint.Host;

        RemoteCertificateValidationCallback? validation = null;
        if (tls.CaPath is not null)
        {
            var trusted = new X509Certificate2Collection();
            try
            {
                trusted.ImportFromPemFile(tls.CaPath);
            }
            catch (Exception e)
            {
                throw new ThunderConnectionException($"TLS setup failed: {e.Message}", e);
            }

            if (trusted.Count == 0)
            {
                throw new ThunderConnectionException(
                    $"TLS setup failed: no certificates in CA file '{tls.CaPath}'");
            }

            // Pin exactly the configured root(s): the server certificate must
            // chain to one of them, and its name must match (FR-29).
            validation = (_, certificate, _, sslPolicyErrors) =>
                VerifyPinnedCertificate(certificate, sslPolicyErrors, trusted);
        }

        var sslStream = new SslStream(tcp.GetStream(), leaveInnerStreamOpen: false, validation);
        try
        {
            await sslStream.AuthenticateAsClientAsync(
                    new SslClientAuthenticationOptions { TargetHost = targetHost },
                    cancellationToken)
                .ConfigureAwait(false);
        }
        catch (OperationCanceledException) when (cancellationToken.IsCancellationRequested)
        {
            await sslStream.DisposeAsync().ConfigureAwait(false);
            throw;
        }
        catch (Exception e)
        {
            await sslStream.DisposeAsync().ConfigureAwait(false);
            throw new ThunderConnectionException($"TLS handshake failed: {e.Message}", e);
        }

        return sslStream;
    }

    /// <summary>
    /// Validate the server certificate against the pinned root(s): reject a
    /// name mismatch outright, then rebuild the chain trusting only the
    /// configured CA(s) — the system trust store plays no part (FR-29).
    /// </summary>
    private static bool VerifyPinnedCertificate(
        X509Certificate? certificate,
        SslPolicyErrors sslPolicyErrors,
        X509Certificate2Collection trustedRoots)
    {
        if (certificate is null)
        {
            return false;
        }

        // The name check is the platform's; a mismatch is never acceptable. The
        // untrusted-root error is expected here and re-evaluated against the
        // pinned store below.
        if ((sslPolicyErrors & SslPolicyErrors.RemoteCertificateNameMismatch) != 0)
        {
            return false;
        }

        X509Certificate2? created = null;
        var serverCert = certificate as X509Certificate2;
        if (serverCert is null)
        {
            serverCert = new X509Certificate2(certificate);
            created = serverCert;
        }

        try
        {
            using var chain = new X509Chain();
            chain.ChainPolicy.TrustMode = X509ChainTrustMode.CustomRootTrust;
            chain.ChainPolicy.RevocationMode = X509RevocationMode.NoCheck;
            chain.ChainPolicy.CustomTrustStore.AddRange(trustedRoots);
            return chain.Build(serverCert);
        }
        finally
        {
            created?.Dispose();
        }
    }

    /// <summary>
    /// Run the handshake the <see cref="Config"/> describes, before user
    /// calls proceed (CLT-002):
    /// <see cref="Handshake.None"/> sends nothing;
    /// <see cref="Handshake.AuthCommand"/> sends the optional arg-less
    /// <c>HELLO</c> (when the config has one) then <c>AUTH</c> when
    /// credentials are configured;
    /// <see cref="Handshake.HelloMandatory"/> sends the <c>HELLO</c> map as
    /// the first frame and parses the reply.
    /// <para>
    /// Under <see cref="Handshake.AuthCommand"/>, no credentials means no
    /// <c>AUTH</c> frame — which is the correct behavior against a deployment
    /// that does not require them. Enforcement is the server's policy, not
    /// the protocol config's (PRO-001a).
    /// </para>
    /// </summary>
    private async Task<HandshakeInfo> HandshakeAsync(Conn conn, CancellationToken cancellationToken)
    {
        switch (_config.Handshake)
        {
            case Handshake.None:
                return HandshakeInfo.Default;

            case Handshake.AuthCommand:
            {
                var credentials = _clientConfig.Credentials;
                if (credentials is null)
                {
                    return HandshakeInfo.Default;
                }

                if (_config.HelloStyle == HelloStyle.ArgLess)
                {
                    // Optional metadata HELLO — takes no arguments; the reply
                    // carries {server, version, proto, id, authenticated}.
                    // Credentials go in AUTH below.
                    await HandshakeCallAsync(
                            conn, "HELLO", System.Array.Empty<Value>(), cancellationToken)
                        .ConfigureAwait(false);
                }

                var args = credentials.Kind switch
                {
                    CredentialKind.UserPass => new[]
                    {
                        Value.Str(credentials.User!),
                        Value.Str(credentials.Secret),
                    },
                    _ => new[] { Value.Str(credentials.Secret) },
                };
                await HandshakeCallAsync(conn, "AUTH", args, cancellationToken).ConfigureAwait(false);
                return new HandshakeInfo(true, System.Array.Empty<string>());
            }

            case Handshake.HelloMandatory:
            default:
            {
                var pairs = new List<KeyValuePair<Value, Value>>
                {
                    new(Value.Str("version"), Value.Int(Wire.WireVersion)),
                };
                switch (_clientConfig.Credentials?.Kind)
                {
                    case CredentialKind.Token:
                        pairs.Add(new(Value.Str("token"), Value.Str(_clientConfig.Credentials.Secret)));
                        break;
                    case CredentialKind.ApiKey:
                        pairs.Add(new(Value.Str("api_key"), Value.Str(_clientConfig.Credentials.Secret)));
                        break;
                    case CredentialKind.UserPass:
                        throw new ThunderAuthException(
                            "user/password credentials are not supported by HelloMandatory " +
                            "configs — use a token or api_key (PRO-001)");
                    case null:
                        break;
                }

                pairs.Add(new(
                    Value.Str("client_name"),
                    Value.Str(_clientConfig.ClientName ?? "thunder-client")));
                var reply = await HandshakeCallAsync(
                        conn, "HELLO", new[] { Value.Map(pairs) }, cancellationToken)
                    .ConfigureAwait(false);
                var authenticated = reply.MapGet("authenticated")?.AsBool() ?? false;
                var capabilities = reply.MapGet("capabilities")?.AsArray()
                    ?.Select(v => v.AsStr())
                    .OfType<string>()
                    .ToArray() ?? System.Array.Empty<string>();
                return new HandshakeInfo(authenticated, capabilities);
            }
        }
    }

    /// <summary>
    /// One handshake round-trip. Server rejections surface as the typed
    /// auth class, never a generic error (CLT-003); transport failures keep
    /// their own class.
    /// </summary>
    private async Task<Value> HandshakeCallAsync(
        Conn conn,
        string command,
        IReadOnlyList<Value> args,
        CancellationToken cancellationToken)
    {
        try
        {
            return await DispatchAsync(conn, command, args, _clientConfig.CallTimeout, cancellationToken)
                .ConfigureAwait(false);
        }
        catch (WriteFailedException writeFailed)
        {
            throw writeFailed.Inner;
        }
        catch (ThunderException e) when (
            e.ErrorClass is ThunderErrorClass.Server or ThunderErrorClass.Auth)
        {
            throw new ThunderAuthException(e.Message);
        }
    }

    /// <summary>
    /// One request/response attempt on one connection: register the pending
    /// entry, write the frame (serialized, CLT-011), await the demuxed
    /// response under the timeout (CLT-020) and cancellation (CLT-021).
    /// </summary>
    private async Task<Value> DispatchAsync(
        Conn conn,
        string command,
        IReadOnlyList<Value> args,
        TimeSpan timeout,
        CancellationToken cancellationToken)
    {
        var id = AllocateId(ref _nextId);
        var pendingTcs = new TaskCompletionSource<Response>(
            TaskCreationOptions.RunContinuationsAsynchronously);
        conn.Pending[id] = pendingTcs;
        if (!conn.IsAlive)
        {
            // The poisoner drains after marking dead: whichever of the two
            // of us ran second cleans this entry up.
            conn.Pending.TryRemove(id, out _);
            throw new WriteFailedException(new ThunderConnectionException("connection is dead"));
        }

        var frame = FrameCodec.EncodeRequest(new Request(id, command, args));
        try
        {
            await conn.WriteLock.WaitAsync(cancellationToken).ConfigureAwait(false);
        }
        catch (OperationCanceledException)
        {
            conn.Pending.TryRemove(id, out _);
            throw;
        }

        try
        {
            await conn.Stream.WriteAsync(frame, cancellationToken).ConfigureAwait(false);
        }
        catch (OperationCanceledException)
        {
            // The frame may be half-written: the stream is unusable.
            conn.Pending.TryRemove(id, out _);
            conn.Kill(new ThunderConnectionException("write cancelled mid-frame"));
            throw;
        }
        catch (Exception e)
        {
            conn.Pending.TryRemove(id, out _);
            var error = new ThunderConnectionException($"write failed: {e.Message}", e);
            conn.Kill(error);
            // The request never reached the wire — safe to resend on a
            // fresh connection.
            throw new WriteFailedException(error);
        }
        finally
        {
            conn.WriteLock.Release();
        }

        Response response;
        try
        {
            response = await pendingTcs.Task.WaitAsync(timeout, cancellationToken)
                .ConfigureAwait(false);
        }
        catch (TimeoutException)
        {
            // CLT-020: remove the pending entry on timeout; a late response
            // to this id is dropped per CLT-013.
            conn.Pending.TryRemove(id, out _);
            throw new ThunderTimeoutException();
        }
        catch (OperationCanceledException)
        {
            // CLT-021: cancellation removes the pending entry too.
            conn.Pending.TryRemove(id, out _);
            throw;
        }

        if (response.IsOk)
        {
            return response.Value!;
        }

        throw ThunderException.FromServerMessage(response.Error!, _config.ErrorCodes);
    }

    /// <summary>
    /// The background reader (CLT-010): reads frames with the config's cap
    /// checked between prefix and body allocation (WIRE-020), demuxes by id,
    /// routes push frames (CLT-060), drops unknown ids (CLT-013), and
    /// poisons the connection on any read failure (CLT-014).
    /// </summary>
    private async Task ReaderLoopAsync(Conn conn)
    {
        ThunderException error;
        try
        {
            var header = new byte[4];
            while (true)
            {
                await conn.Stream.ReadExactlyAsync(header).ConfigureAwait(false);
                var length = BinaryPrimitives.ReadUInt32LittleEndian(header);
                if (length > (uint)_config.MaxFrameBytes)
                {
                    // WIRE-020/021: refused on the prefix alone — the body
                    // buffer is never allocated.
                    error = new ThunderFrameTooLargeException(length, _config.MaxFrameBytes);
                    break;
                }

                var body = new byte[length];
                await conn.Stream.ReadExactlyAsync(body).ConfigureAwait(false);
                Response response;
                try
                {
                    response = WireCodec.DecodeResponseBody(body);
                }
                catch (ThunderDecodeException e)
                {
                    error = e;
                    break;
                }

                if (response.Id == Wire.PushId)
                {
                    if (_config.Push == PushPolicy.Enabled)
                    {
                        var handler = Volatile.Read(ref _pushHandler);
                        if (handler is not null && response.IsOk)
                        {
                            try
                            {
                                handler(response.Value!);
                            }
                            catch
                            {
                                // Handler faults never poison the connection.
                            }
                        }

                        continue;
                    }

                    // Protocol error: poison per CLT-014.
                    error = new ThunderDecodeException(
                        "server sent a push frame but the config reserves PUSH_ID (CLT-060)");
                    break;
                }

                if (conn.Pending.TryRemove(response.Id, out var tcs))
                {
                    tcs.TrySetResult(response);
                }
                else
                {
                    // CLT-013: unknown id — count and drop, never fatal.
                    Interlocked.Increment(ref _unknownDrops);
                }
            }
        }
        catch (Exception e)
        {
            error = new ThunderConnectionException($"connection lost: {e.Message}", e);
        }

        // CLT-014: fail all pending calls typed and close our side.
        conn.Kill(error);
    }

    /// <summary>State shared between one connection's callers and its reader.</summary>
    private sealed class Conn
    {
        private int _alive = 1;

        internal Conn(TcpClient tcp, Stream stream)
        {
            Tcp = tcp;
            Stream = stream;
        }

        internal TcpClient Tcp { get; }

        /// <summary>
        /// The transport the reader/writer see: a bare <see cref="NetworkStream"/>
        /// on the plaintext path, or an <see cref="SslStream"/> wrapping it under
        /// TLS (FR-29). Neither the codec nor the demux sees the difference.
        /// </summary>
        internal Stream Stream { get; }

        /// <summary>
        /// Writes serialize behind this semaphore so frames never interleave
        /// (CLT-011); reads belong to the reader alone.
        /// </summary>
        internal SemaphoreSlim WriteLock { get; } = new(1, 1);

        /// <summary>id → completion-source demux map (CLT-010).</summary>
        internal ConcurrentDictionary<uint, TaskCompletionSource<Response>> Pending { get; } = new();

        internal bool IsAlive => Volatile.Read(ref _alive) == 1;

        /// <summary>
        /// Poison: mark dead and fail every pending call with the same typed
        /// error (CLT-014). Idempotent.
        /// </summary>
        internal void Poison(ThunderException error)
        {
            Volatile.Write(ref _alive, 0);
            foreach (var id in Pending.Keys)
            {
                if (Pending.TryRemove(id, out var tcs))
                {
                    tcs.TrySetException(error);
                }
            }
        }

        /// <summary>Tear down: fail all pending calls typed and close the socket.</summary>
        internal void Kill(ThunderException error)
        {
            Poison(error);
            try
            {
                // Disposing the stream also tears down an SslStream's TLS state;
                // closing the socket breaks any read/write in flight.
                Stream.Dispose();
            }
            catch
            {
                // Best-effort close.
            }

            try
            {
                Tcp.Close();
            }
            catch
            {
                // Best-effort close.
            }
        }
    }

    /// <summary>Mutable re-dial budget shared across one call's attempts (CLT-030).</summary>
    private sealed class RedialBudget
    {
        internal int Left;
    }

    /// <summary>
    /// Wrapper distinguishing "the request never reached the wire — safe to
    /// resend on a fresh connection" from fatal outcomes (server / timeout /
    /// poison errors are never retried).
    /// </summary>
    private sealed class WriteFailedException : Exception
    {
        internal WriteFailedException(ThunderException inner)
            : base(inner.Message, inner)
        {
            Inner = inner;
        }

        internal ThunderException Inner { get; }
    }
}

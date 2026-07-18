namespace HiveLLM.Thunder;

/// <summary>
/// Optional connection pool (CLT-080) — a layer <b>above</b> the
/// single-connection <see cref="ThunderClient"/> (CLT-001: "pooling is a layer
/// above").
/// <para>
/// Under a mandatory-<c>HELLO</c> profile a fresh connection costs a handshake
/// round trip before the first request; a caller that opens a connection per
/// operation pays that every time. The pool amortizes it: <c>N</c> operations
/// over a checked-out connection pay <b>one</b> connect and <b>one</b>
/// handshake, not <c>N</c>.
/// </para>
/// <para>
/// The shape is deliberately minimal — a fixed number of connections bounded by
/// a <see cref="SemaphoreSlim"/>, an idle stack, lazy connect on first checkout,
/// and a guard (<see cref="PooledConn"/>) that returns the connection on
/// <see cref="PooledConn.DisposeAsync"/>. It is <b>not</b> an external pool
/// library: health checks, background reaping and min-idle warmup are out of
/// scope; a poisoned connection (CLT-014) is dropped on return and the next
/// checkout connects fresh, leaving reconnect to CLT-030 rather than the pool.
/// </para>
/// <para>
/// The pool adds <b>no wire behavior</b>: it builds the same
/// <see cref="ThunderClient"/> as <see cref="ThunderClient.ConnectAsync"/> from
/// a <see cref="Config"/> and <see cref="ClientConfig"/>, and the
/// single-connection client's API is unchanged.
/// <see cref="Config.MaxInFlight"/> (CLT-012) stays a per-connection bound; the
/// pool bounds connections, not in-flight calls.
/// </para>
/// <example>
/// <code>
/// var app = Config.Standard() with { Scheme = "myapp", DefaultPort = 9000 };
/// using var pool = new Pool("myapp://localhost", app, new ClientConfig(), 8);
/// await using var conn = await pool.AcquireAsync(); // reuses idle, or dials
/// var pong = await conn.CallAsync("PING");
/// // `conn` returns the connection to the pool when it is disposed.
/// </code>
/// </example>
/// </summary>
public sealed class Pool : IDisposable
{
    private readonly string _endpoint;
    private readonly Config _config;
    private readonly ClientConfig _clientConfig;

    /// <summary>Bounds live + checked-out connections to <see cref="MaxConnections"/>.</summary>
    private readonly SemaphoreSlim _permits;

    private readonly object _idleLock = new();

    /// <summary>Idle connections available for reuse — newest reused first (LIFO).</summary>
    private readonly Stack<ThunderClient> _idle;

    private bool _disposed;

    /// <summary>
    /// Build a pool for <paramref name="endpoint"/>. Opens no connections — the
    /// first <see cref="AcquireAsync"/> dials the first one.
    /// <paramref name="maxConnections"/> is clamped to at least 1.
    /// </summary>
    /// <param name="endpoint">The endpoint every pooled client dials (CLT-070).</param>
    /// <param name="config">The application's protocol config — the dialect (PRO-001).</param>
    /// <param name="clientConfig">This caller's credentials/timeouts, shared by every pooled client.</param>
    /// <param name="maxConnections">The fixed connection bound; clamped to <c>&gt;= 1</c>.</param>
    public Pool(string endpoint, Config config, ClientConfig clientConfig, int maxConnections)
    {
        ArgumentNullException.ThrowIfNull(endpoint);
        ArgumentNullException.ThrowIfNull(config);
        ArgumentNullException.ThrowIfNull(clientConfig);
        var max = Math.Max(1, maxConnections);
        _endpoint = endpoint;
        _config = config;
        _clientConfig = clientConfig;
        MaxConnections = max;
        _permits = new SemaphoreSlim(max, max);
        _idle = new Stack<ThunderClient>(max);
    }

    /// <summary>The fixed connection bound (clamped to <c>&gt;= 1</c>).</summary>
    public int MaxConnections { get; }

    /// <summary>
    /// Idle connections currently parked in the pool. For diagnostics and tests
    /// — production code should not branch on it.
    /// </summary>
    public int IdleCount
    {
        get
        {
            lock (_idleLock)
            {
                return _idle.Count;
            }
        }
    }

    /// <summary>
    /// Check out a connection. Reuses an idle, <b>live</b> connection when one
    /// is available; otherwise dials and handshakes a fresh one (CLT-002).
    /// Awaits a return when <see cref="MaxConnections"/> are already checked out.
    /// The returned <see cref="PooledConn"/> returns the connection to the pool
    /// when disposed.
    /// </summary>
    /// <param name="cancellationToken">Cancels the wait for a slot and the dial.</param>
    /// <exception cref="ThunderConnectionException">The dial failed, or the pool is disposed.</exception>
    /// <exception cref="ThunderTimeoutException">The connect timeout elapsed (CLT-001).</exception>
    /// <exception cref="ThunderAuthException">The server rejected the handshake (CLT-003).</exception>
    public async Task<PooledConn> AcquireAsync(CancellationToken cancellationToken = default)
    {
        try
        {
            await _permits.WaitAsync(cancellationToken).ConfigureAwait(false);
        }
        catch (ObjectDisposedException)
        {
            throw new ThunderConnectionException("connection pool is closed");
        }

        try
        {
            // Reuse the newest idle connection that is still live; discard any
            // that were poisoned (CLT-014) while sitting idle.
            ThunderClient? reused = null;
            lock (_idleLock)
            {
                while (_idle.Count > 0)
                {
                    var candidate = _idle.Pop();
                    if (candidate.IsAlive)
                    {
                        reused = candidate;
                        break;
                    }

                    candidate.Dispose();
                }
            }

            var client = reused ?? await ThunderClient
                .ConnectAsync(_endpoint, _config, _clientConfig, cancellationToken)
                .ConfigureAwait(false);
            return new PooledConn(this, client);
        }
        catch
        {
            // The checkout failed to hand out a guard — release the slot the
            // guard would otherwise have held.
            _permits.Release();
            throw;
        }
    }

    /// <summary>
    /// Return a checked-out connection (called by <see cref="PooledConn"/> on
    /// dispose). A live connection is parked for reuse; a poisoned or closed one
    /// is dropped and the next checkout dials fresh (CLT-014/030). The slot is
    /// released either way.
    /// </summary>
    internal void Return(ThunderClient client)
    {
        var parked = false;
        if (!_disposed && client.IsAlive)
        {
            lock (_idleLock)
            {
                if (!_disposed)
                {
                    // Park before releasing the slot so a waiter wakes to find
                    // this connection idle rather than dialing a redundant one.
                    _idle.Push(client);
                    parked = true;
                }
            }
        }

        if (!parked)
        {
            client.Dispose();
        }

        try
        {
            _permits.Release();
        }
        catch (ObjectDisposedException)
        {
            // The pool was disposed while this connection was checked out; the
            // slot no longer exists to release.
        }
    }

    /// <summary>
    /// Dispose the pool: close every idle connection. Checked-out connections
    /// close normally when their guard is disposed.
    /// </summary>
    public void Dispose()
    {
        List<ThunderClient> idle;
        lock (_idleLock)
        {
            if (_disposed)
            {
                return;
            }

            _disposed = true;
            idle = new List<ThunderClient>(_idle);
            _idle.Clear();
        }

        foreach (var client in idle)
        {
            client.Dispose();
        }

        _permits.Dispose();
    }
}

/// <summary>
/// Guard from <see cref="Pool.AcquireAsync"/>. Exposes the checked-out
/// <see cref="ThunderClient"/> and returns the connection to the pool on
/// <see cref="DisposeAsync"/> so the next checkout reuses it — unless the
/// connection was poisoned, in which case it is dropped and the next checkout
/// connects fresh (CLT-014/030). The .NET idiom for the Rust RAII guard:
/// <c>await using var conn = await pool.AcquireAsync();</c>.
/// </summary>
public sealed class PooledConn : IAsyncDisposable, IDisposable
{
    private readonly Pool _pool;
    private ThunderClient? _client;

    internal PooledConn(Pool pool, ThunderClient client)
    {
        _pool = pool;
        _client = client;
    }

    /// <summary>
    /// The checked-out client. Valid until this guard is disposed.
    /// </summary>
    /// <exception cref="ObjectDisposedException">The guard has been disposed.</exception>
    public ThunderClient Client =>
        _client ?? throw new ObjectDisposedException(nameof(PooledConn));

    /// <summary>Issue one call on the checked-out connection (CLT-020).</summary>
    public Task<Value> CallAsync(
        string command,
        IReadOnlyList<Value>? args = null,
        CancellationToken cancellationToken = default) =>
        Client.CallAsync(command, args, cancellationToken);

    /// <summary>Return the connection to the pool (or drop it if poisoned). Idempotent.</summary>
    public ValueTask DisposeAsync()
    {
        ReturnToPool();
        return ValueTask.CompletedTask;
    }

    /// <inheritdoc cref="DisposeAsync" />
    public void Dispose() => ReturnToPool();

    private void ReturnToPool()
    {
        var client = Interlocked.Exchange(ref _client, null);
        if (client is not null)
        {
            _pool.Return(client);
        }
    }
}

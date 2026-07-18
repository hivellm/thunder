using System.Net;
using System.Net.Sockets;

using Xunit;

namespace HiveLLM.Thunder.Tests;

/// <summary>
/// Connection-pool behavior (CLT-080). The pool is a layer above the
/// single-connection client; these tests exercise it end to end over a loopback
/// responder — checkout/return, the capacity bound, the poison drop, and the
/// property the whole layer exists for: <c>N</c> operations pay <b>one</b>
/// connection and <b>one</b> handshake, not <c>N</c>. Mirrors
/// rust/thunder/tests/pool.rs.
/// </summary>
public class PoolTests
{
    /// <summary>
    /// The standard profile (mandatory HELLO map + capabilities reply), so a
    /// real handshake happens on every new connection — the pool's reuse is what
    /// this suite measures.
    /// </summary>
    private static Config Profile() => Config.Standard() with { Scheme = "test", DefaultPort = 0 };

    private static ClientConfig ClientCfg() => new() { ClientName = "pool-test" };

    [Fact]
    public void New_opens_no_connections_and_clamps_capacity()
    {
        using var pool = new Pool("test://127.0.0.1:0", Profile(), ClientCfg(), 0);
        // No connection opened at construction, and max clamped to >= 1.
        Assert.Equal(0, pool.IdleCount);
        Assert.Equal(1, pool.MaxConnections);
    }

    [Fact]
    public async Task Checkout_returns_the_connection_for_reuse()
    {
        using var server = new CountingServer();
        using var pool = new Pool(server.Address, Profile(), ClientCfg(), 4);

        Assert.Equal(0, pool.IdleCount); // construction dials nothing
        await using (var conn = await pool.AcquireAsync())
        {
            Assert.Equal("PONG", (await conn.CallAsync("PING")).AsStr());
            Assert.Equal(0, pool.IdleCount); // checked out, so not idle
        }

        // The guard disposed: the connection returned to the pool.
        Assert.Equal(1, pool.IdleCount);
        Assert.Equal(1, server.Connections);
    }

    [Fact]
    public async Task N_operations_use_one_connection_and_handshake()
    {
        using var server = new CountingServer();
        using var pool = new Pool(server.Address, Profile(), ClientCfg(), 4);

        for (var i = 0; i < 10; i++)
        {
            await using var conn = await pool.AcquireAsync();
            Assert.Equal("PONG", (await conn.CallAsync("PING")).AsStr());
        }

        // The whole point of the layer: ten sequential operations reused one
        // connection, so the server saw one handshake, not ten.
        Assert.Equal(10, server.Calls);
        Assert.Equal(1, server.Connections);
    }

    [Fact]
    public async Task Pool_never_exceeds_max_connections()
    {
        using var server = new CountingServer();
        using var pool = new Pool(server.Address, Profile(), ClientCfg(), 2);

        var a = await pool.AcquireAsync();
        var b = await pool.AcquireAsync();

        // With both permits held, a third checkout must wait, not open a third
        // connection (CLT-080 fixed N).
        var third = pool.AcquireAsync();
        await Task.Delay(150);
        Assert.False(third.IsCompleted, "third checkout must block while max are held");

        // Release one; the waiter now completes.
        await a.DisposeAsync();
        var c = await third.WaitAsync(TimeSpan.FromSeconds(1));
        Assert.Equal("PONG", (await c.CallAsync("PING")).AsStr());

        // At most two connections ever existed.
        Assert.True(server.Connections <= 2);

        await b.DisposeAsync();
        await c.DisposeAsync();
    }

    [Fact]
    public async Task A_poisoned_connection_is_not_handed_to_the_next_caller()
    {
        using var server = new CountingServer();
        using var pool = new Pool(server.Address, Profile(), ClientCfg(), 4);

        await using (var conn = await pool.AcquireAsync())
        {
            Assert.Equal("PONG", (await conn.CallAsync("PING")).AsStr());
            // Kill this connection, then let the guard dispose.
            conn.Client.Close();
            Assert.False(conn.Client.IsAlive);
        }

        // CLT-014: the dead connection was dropped, not parked for reuse.
        Assert.Equal(0, pool.IdleCount);

        // The next checkout dials a fresh, working connection.
        await using var fresh = await pool.AcquireAsync();
        Assert.Equal("PONG", (await fresh.CallAsync("PING")).AsStr());
        Assert.Equal(2, server.Connections);
    }

    /// <summary>
    /// Loopback responder that serves the standard <c>HELLO</c> handshake then
    /// echoes <c>PING</c> → <c>PONG</c>, counting the connections it accepted
    /// (hence handshakes) and the calls it served — so a test can prove how many
    /// connections <c>N</c> pooled operations really used.
    /// </summary>
    private sealed class CountingServer : IDisposable
    {
        private readonly TcpListener _listener;
        private readonly CancellationTokenSource _cts = new();
        private int _connections;
        private int _calls;

        internal CountingServer()
        {
            _listener = new TcpListener(IPAddress.Loopback, 0);
            _listener.Start();
            _ = AcceptLoopAsync();
        }

        internal string Address => $"127.0.0.1:{((IPEndPoint)_listener.LocalEndpoint).Port}";

        /// <summary>Distinct connections accepted — one per handshake.</summary>
        internal int Connections => Volatile.Read(ref _connections);

        /// <summary>Non-handshake requests served.</summary>
        internal int Calls => Volatile.Read(ref _calls);

        public void Dispose()
        {
            _cts.Cancel();
            _listener.Stop();
            _cts.Dispose();
        }

        private async Task AcceptLoopAsync()
        {
            while (!_cts.IsCancellationRequested)
            {
                TcpClient tcp;
                try
                {
                    tcp = await _listener.AcceptTcpClientAsync(_cts.Token);
                }
                catch (Exception e) when (e is OperationCanceledException or SocketException or ObjectDisposedException)
                {
                    return;
                }

                Interlocked.Increment(ref _connections);
                _ = HandleConnectionAsync(tcp);
            }
        }

        private async Task HandleConnectionAsync(TcpClient tcp)
        {
            using var conn = new ServerConn(tcp);
            try
            {
                // HelloMandatory: the first frame is the HELLO map — accept it.
                var hello = await conn.ReadRequestAsync();
                await conn.SendOkAsync(hello.Id, Value.Map(
                    (Value.Str("authenticated"), Value.Bool(true))));

                while (true)
                {
                    var request = await conn.ReadRequestAsync();
                    Interlocked.Increment(ref _calls);
                    await conn.SendOkAsync(request.Id, Value.Str("PONG"));
                }
            }
            catch (Exception e) when (e is IOException or ObjectDisposedException)
            {
                // The client closed or poisoned the connection — done.
            }
        }
    }
}

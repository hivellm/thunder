using Xunit;

namespace HiveLLM.Thunder.Tests;

/// <summary>
/// Behavioral floor tests for the Thunder client (SPEC-003, feeds the
/// CLT-090 suite): loopback responders built on the frame codec stand in
/// for thunder-server — the client contract is exercised end-to-end over
/// real sockets. Mirrors rust/thunder-client/tests/behavior.rs, plus the
/// C#-specific CancellationToken scenario (CLT-021).
/// </summary>
public class BehaviorTests
{
    /// <summary>
    /// A custom profile (PRO-020): no handshake, push reserved, no error
    /// parsing — the neutral baseline the behavioral tests mutate.
    /// </summary>
    private static Profile PlainProfile() => new()
    {
        Name = "test",
        Scheme = "test",
        DefaultPort = 0,
        Handshake = Handshake.None,
        HelloStyle = HelloStyle.NotUsed,
        Push = PushPolicy.Reserved,
        MaxFrameBytes = MockServer.ServerCap,
        MaxInFlight = 64,
        ErrorCodes = ErrorConvention.None,
        Tls = TlsPolicy.Off,
    };

    private static Value HelloOkReply() => Value.Map(
        (Value.Str("authenticated"), Value.Bool(true)));

    // ── Multiplexing (CLT-010/011) ──────────────────────────────────────

    [Fact]
    public async Task Pipelined_calls_complete_out_of_order()
    {
        using var server = new MockServer();
        var serverTask = Task.Run(async () =>
        {
            using var conn = await server.AcceptAsync();
            // Read BOTH requests before answering, then answer in reverse:
            // completion order follows the server, not submission order.
            var first = await conn.ReadRequestAsync();
            var second = await conn.ReadRequestAsync();
            Assert.NotEqual(first.Id, second.Id); // ids must be distinct (CLT-010)
            await conn.SendOkAsync(second.Id, Value.Str(second.Command));
            await conn.SendOkAsync(first.Id, Value.Str(first.Command));
        });

        await using var client = await ThunderClient.ConnectAsync(server.Address, PlainProfile());
        var one = client.CallAsync("ONE");
        var two = client.CallAsync("TWO");
        await Task.WhenAll(one, two);
        Assert.Equal("ONE", (await one).AsStr());
        Assert.Equal("TWO", (await two).AsStr());
        await serverTask;
    }

    [Fact]
    public async Task In_flight_bound_backpressures_instead_of_refusing()
    {
        using var server = new MockServer();
        var serverTask = Task.Run(async () =>
        {
            using var conn = await server.AcceptAsync();
            // Strictly serial: with MaxInFlight = 1 the second call must
            // wait for the first permit, never be refused (CLT-012).
            for (var i = 0; i < 2; i++)
            {
                var request = await conn.ReadRequestAsync();
                await conn.SendOkAsync(request.Id, Value.Str(request.Command));
            }
        });

        var profile = PlainProfile() with { MaxInFlight = 1 };
        await using var client = await ThunderClient.ConnectAsync(server.Address, profile);
        var a = client.CallAsync("A");
        var b = client.CallAsync("B");
        await Task.WhenAll(a, b);
        Assert.Equal("A", (await a).AsStr());
        Assert.Equal("B", (await b).AsStr());
        await serverTask;
    }

    [Fact]
    public async Task Stray_response_id_is_dropped_never_fatal()
    {
        using var server = new MockServer();
        var serverTask = Task.Run(async () =>
        {
            using var conn = await server.AcceptAsync();
            var request = await conn.ReadRequestAsync();
            // A response nobody asked for, then the real one (CLT-013).
            await conn.SendOkAsync(9_999, Value.Null);
            await conn.SendOkAsync(request.Id, Value.Str("real"));
        });

        await using var client = await ThunderClient.ConnectAsync(server.Address, PlainProfile());
        var value = await client.CallAsync("GET");
        Assert.Equal("real", value.AsStr());
        Assert.Equal(1, client.UnknownResponseDrops);
        await serverTask;
    }

    // ── Handshakes (CLT-002/003) ────────────────────────────────────────

    [Fact]
    public async Task None_handshake_sends_nothing_before_user_calls()
    {
        using var server = new MockServer();
        var serverTask = Task.Run(async () =>
        {
            using var conn = await server.AcceptAsync();
            // The very first frame must be the user's command — no HELLO,
            // no AUTH (Handshake.None).
            var request = await conn.ReadRequestAsync();
            Assert.Equal("PING", request.Command);
            await conn.SendOkAsync(request.Id, Value.Str("PONG"));
        });

        // PlainProfile() is the genuine Handshake.None case. (This test used
        // to ride on Profile.Synap, which is AuthCommand since BN-023.)
        await using var client = await ThunderClient.ConnectAsync(server.Address, PlainProfile());
        Assert.False(client.IsAuthenticated);
        Assert.Equal("PONG", (await client.CallAsync("PING")).AsStr());
        await serverTask;
    }

    /// <summary>
    /// The client half of the shape/policy split, on the profile BN-023
    /// changed: <c>synap</c> is <see cref="Handshake.AuthCommand"/> now, but
    /// with no credentials configured it sends no <c>AUTH</c> at all — exactly
    /// right against an open deployment (<c>require_auth</c> off). It must
    /// also never send <c>HELLO</c> (<see cref="HelloStyle.NotUsed"/>).
    /// </summary>
    [Fact]
    public async Task Synap_profile_without_credentials_sends_nothing()
    {
        using var server = new MockServer();
        var serverTask = Task.Run(async () =>
        {
            using var conn = await server.AcceptAsync();
            var request = await conn.ReadRequestAsync();
            Assert.Equal("PING", request.Command); // no AUTH/HELLO without credentials
            await conn.SendOkAsync(request.Id, Value.Str("PONG"));
        });

        await using var client = await ThunderClient.ConnectAsync(server.Address, Profile.Synap);
        Assert.False(client.IsAuthenticated);
        Assert.Equal("PONG", (await client.CallAsync("PING")).AsStr());
        await serverTask;
    }

    /// <summary>
    /// BN-023 regression: the <c>synap</c> profile must be able to
    /// authenticate.
    /// <para>
    /// It used to be <see cref="Handshake.None"/>, so a credentialed client
    /// sent <b>nothing</b> and could never reach a <c>require_auth</c> Synap.
    /// Synap's RPC path has an <c>AUTH</c> handler (and no <c>HELLO</c>
    /// handler), so the profile is <see cref="Handshake.AuthCommand"/> +
    /// <see cref="HelloStyle.NotUsed"/>: <c>AUTH</c> goes out, <c>HELLO</c>
    /// never does.
    /// </para>
    /// </summary>
    [Fact]
    public async Task Synap_profile_sends_auth_and_never_hello()
    {
        using var server = new MockServer();
        var serverTask = Task.Run(async () =>
        {
            using var conn = await server.AcceptAsync();
            // First frame must be AUTH — Synap has no HELLO handler at all.
            var auth = await conn.ReadRequestAsync();
            Assert.Equal("AUTH", auth.Command);
            Assert.Equal(
                new[] { Value.Str("root"), Value.Str("hunter2") }, // AUTH <user> <password>
                auth.Args);
            await conn.SendOkAsync(auth.Id, Value.Str("OK"));
            var ping = await conn.ReadRequestAsync();
            Assert.Equal("PING", ping.Command);
            await conn.SendOkAsync(ping.Id, Value.Str("PONG"));
        });

        var config = new ClientConfig { Credentials = Credentials.UserPass("root", "hunter2") };
        await using var client = await ThunderClient.ConnectAsync(
            server.Address, Profile.Synap, config);
        Assert.True(client.IsAuthenticated);
        Assert.Equal("PONG", (await client.CallAsync("PING")).AsStr());
        await serverTask;
    }

    [Fact]
    public async Task Auth_command_handshake_sends_hello_then_auth_api_key()
    {
        using var server = new MockServer();
        var serverTask = Task.Run(async () =>
        {
            using var conn = await server.AcceptAsync();
            var hello = await conn.ReadRequestAsync();
            Assert.Equal("HELLO", hello.Command);
            // Nexus RPC HELLO takes no arguments — the positional [Int(1)] is
            // the RESP3 HELLO, a different surface (BN-023 errata).
            Assert.Empty(hello.Args);
            await conn.SendOkAsync(hello.Id, Value.Null);
            var auth = await conn.ReadRequestAsync();
            Assert.Equal("AUTH", auth.Command);
            Assert.Equal(new[] { Value.Str("k-123") }, auth.Args);
            await conn.SendOkAsync(auth.Id, Value.Str("OK"));
            var ping = await conn.ReadRequestAsync();
            Assert.Equal("PING", ping.Command);
            await conn.SendOkAsync(ping.Id, Value.Str("PONG"));
        });

        var config = new ClientConfig { Credentials = Credentials.ApiKey("k-123") };
        await using var client = await ThunderClient.ConnectAsync(
            server.Address, Profile.Nexus, config);
        Assert.True(client.IsAuthenticated);
        Assert.Equal("PONG", (await client.CallAsync("PING")).AsStr());
        await serverTask;
    }

    [Fact]
    public async Task Auth_command_handshake_sends_user_pass()
    {
        using var server = new MockServer();
        var serverTask = Task.Run(async () =>
        {
            using var conn = await server.AcceptAsync();
            var hello = await conn.ReadRequestAsync();
            Assert.Equal("HELLO", hello.Command);
            await conn.SendOkAsync(hello.Id, Value.Null);
            var auth = await conn.ReadRequestAsync();
            Assert.Equal("AUTH", auth.Command);
            Assert.Equal(new[] { Value.Str("admin"), Value.Str("hunter2") }, auth.Args);
            await conn.SendOkAsync(auth.Id, Value.Str("OK"));
        });

        var config = new ClientConfig { Credentials = Credentials.UserPass("admin", "hunter2") };
        await using var client = await ThunderClient.ConnectAsync(
            server.Address, Profile.Nexus, config);
        Assert.True(client.IsAuthenticated);
        await serverTask;
    }

    [Fact]
    public async Task Auth_command_without_credentials_sends_nothing()
    {
        using var server = new MockServer();
        var serverTask = Task.Run(async () =>
        {
            using var conn = await server.AcceptAsync();
            var request = await conn.ReadRequestAsync();
            Assert.Equal("PING", request.Command); // no HELLO/AUTH without credentials
            await conn.SendOkAsync(request.Id, Value.Str("PONG"));
        });

        await using var client = await ThunderClient.ConnectAsync(server.Address, Profile.Nexus);
        Assert.False(client.IsAuthenticated);
        await client.CallAsync("PING");
        await serverTask;
    }

    [Fact]
    public async Task Hello_mandatory_sends_hello_map_first_and_exposes_capabilities()
    {
        using var server = new MockServer();
        var serverTask = Task.Run(async () =>
        {
            using var conn = await server.AcceptAsync();
            var hello = await conn.ReadRequestAsync();
            Assert.Equal("HELLO", hello.Command); // HELLO must be the first frame
            var map = hello.Args[0];
            Assert.Equal(1L, map.MapGet("version")?.AsInt());
            // The token credential goes in the HELLO map.
            Assert.Equal("tok-1", map.MapGet("token")?.AsStr());
            Assert.Equal("itest", map.MapGet("client_name")?.AsStr());
            await conn.SendOkAsync(hello.Id, Value.Map(
                (Value.Str("authenticated"), Value.Bool(true)),
                (Value.Str("capabilities"), Value.Array(
                    Value.Str("search"), Value.Str("insert")))));
        });

        var config = new ClientConfig
        {
            Credentials = Credentials.Token("tok-1"),
            ClientName = "itest",
        };
        await using var client = await ThunderClient.ConnectAsync(
            server.Address, Profile.Vectorizer, config);
        Assert.True(client.IsAuthenticated);
        Assert.Equal(new[] { "search", "insert" }, client.Capabilities);
        await serverTask;
    }

    [Fact]
    public async Task Handshake_rejection_is_a_typed_auth_error()
    {
        using var server = new MockServer();
        var serverTask = Task.Run(async () =>
        {
            using var conn = await server.AcceptAsync();
            var hello = await conn.ReadRequestAsync();
            await conn.SendErrAsync(hello.Id, "[unauthorized] invalid api key");
        });

        var config = new ClientConfig { Credentials = Credentials.ApiKey("wrong") };
        // CLT-003: an auth failure is the auth class, not a generic error.
        var error = await Assert.ThrowsAsync<ThunderAuthException>(
            () => ThunderClient.ConnectAsync(server.Address, Profile.Vectorizer, config));
        Assert.Contains("unauthorized", error.Message, StringComparison.Ordinal);
        await serverTask;
    }

    // ── Timeouts and cancellation (CLT-020/021) ─────────────────────────

    [Fact]
    public async Task Per_call_timeout_fires_and_late_response_is_dropped()
    {
        using var server = new MockServer();
        var serverTask = Task.Run(async () =>
        {
            using var conn = await server.AcceptAsync();
            var slow = await conn.ReadRequestAsync();
            // Answer nothing until the NEXT request proves the timeout fired
            // client-side; then deliver the late response first.
            var next = await conn.ReadRequestAsync();
            await conn.SendOkAsync(slow.Id, Value.Str("late"));
            await conn.SendOkAsync(next.Id, Value.Str("fresh"));
        });

        await using var client = await ThunderClient.ConnectAsync(server.Address, PlainProfile());
        await Assert.ThrowsAsync<ThunderTimeoutException>(
            () => client.CallAsync(
                "SLOW", System.Array.Empty<Value>(), TimeSpan.FromMilliseconds(100)));
        // The pending entry was removed (CLT-020); the late response falls
        // under the unknown-id drop (CLT-013) and the connection lives on.
        Assert.Equal("fresh", (await client.CallAsync("NEXT")).AsStr());
        Assert.Equal(1, client.UnknownResponseDrops);
        await serverTask;
    }

    [Fact]
    public async Task Cancellation_removes_the_pending_entry()
    {
        using var server = new MockServer();
        var serverTask = Task.Run(async () =>
        {
            using var conn = await server.AcceptAsync();
            var slow = await conn.ReadRequestAsync();
            var next = await conn.ReadRequestAsync();
            await conn.SendOkAsync(slow.Id, Value.Str("late"));
            await conn.SendOkAsync(next.Id, Value.Str("fresh"));
        });

        await using var client = await ThunderClient.ConnectAsync(server.Address, PlainProfile());
        using var cts = new CancellationTokenSource(TimeSpan.FromMilliseconds(100));
        // CLT-021: cancellation surfaces as OperationCanceledException and
        // removes the pending entry — the late response becomes a stray drop.
        await Assert.ThrowsAnyAsync<OperationCanceledException>(
            () => client.CallAsync("SLOW", null, cts.Token));
        Assert.Equal("fresh", (await client.CallAsync("NEXT")).AsStr());
        Assert.Equal(1, client.UnknownResponseDrops);
        await serverTask;
    }

    // ── Reconnection (CLT-030/031) ──────────────────────────────────────

    [Fact]
    public async Task Reconnect_after_server_drop_succeeds()
    {
        using var server = new MockServer();
        var serverTask = Task.Run(async () =>
        {
            using (var conn = await server.AcceptAsync())
            {
                var request = await conn.ReadRequestAsync();
                await conn.SendOkAsync(request.Id, Value.Str("first"));
            } // connection dropped

            using var second = await server.AcceptAsync();
            var again = await second.ReadRequestAsync();
            await second.SendOkAsync(again.Id, Value.Str("second"));
        });

        await using var client = await ThunderClient.ConnectAsync(server.Address, PlainProfile());
        Assert.Equal("first", (await client.CallAsync("A")).AsStr());
        // Let the reader observe the EOF and mark the connection dead.
        await Task.Delay(200);
        // CLT-030: the call finds the connection dead and lazily re-dials.
        Assert.Equal("second", (await client.CallAsync("B")).AsStr());
        await serverTask;
    }

    [Fact]
    public async Task Reconnect_gives_up_after_two_attempts_with_typed_connection_error()
    {
        using var server = new MockServer();
        var accepts = 0;
        var serverTask = Task.Run(async () =>
        {
            using (var conn = await server.AcceptAsync())
            {
                // Connection 1: serve the handshake and one call, then drop.
                Interlocked.Increment(ref accepts);
                var hello = await conn.ReadRequestAsync();
                await conn.SendOkAsync(hello.Id, HelloOkReply());
                var request = await conn.ReadRequestAsync();
                await conn.SendOkAsync(request.Id, Value.Str("ok"));
            }

            // Re-dial attempts: accept and slam shut before the
            // HelloMandatory handshake can complete.
            for (var i = 0; i < 2; i++)
            {
                var conn = await server.AcceptAsync();
                Interlocked.Increment(ref accepts);
                conn.Dispose();
            }
        });

        var config = new ClientConfig { Credentials = Credentials.ApiKey("k") };
        await using var client = await ThunderClient.ConnectAsync(
            server.Address, Profile.Vectorizer, config);
        await client.CallAsync("PING");
        await Task.Delay(200);

        // The connection class after exhausted re-dials (CLT-030).
        await Assert.ThrowsAsync<ThunderConnectionException>(() => client.CallAsync("PING"));
        await serverTask;
        // Initial connect + exactly 2 re-dial attempts (CLT-030).
        Assert.Equal(3, accepts);
    }

    // ── Error mapping (CLT-050..052) ────────────────────────────────────

    [Fact]
    public async Task Resp3_error_mapping_over_the_wire()
    {
        using var server = new MockServer();
        var serverTask = Task.Run(async () =>
        {
            using var conn = await server.AcceptAsync();
            var get = await conn.ReadRequestAsync();
            await conn.SendErrAsync(get.Id, "NOAUTH Authentication required.");
            var foo = await conn.ReadRequestAsync();
            await conn.SendErrAsync(foo.Id, "ERR unknown command 'FOO'");
        });

        await using var client = await ThunderClient.ConnectAsync(server.Address, Profile.Nexus);
        var auth = await Assert.ThrowsAsync<ThunderAuthException>(() => client.CallAsync("GET"));
        Assert.Equal("NOAUTH Authentication required.", auth.Message);
        var serverError = await Assert.ThrowsAsync<ThunderServerException>(
            () => client.CallAsync("FOO"));
        Assert.Equal("ERR unknown command 'FOO'", serverError.Message);
        Assert.Null(serverError.Code);
        await serverTask;
    }

    [Fact]
    public async Task Bracket_error_mapping_over_the_wire()
    {
        using var server = new MockServer();
        var serverTask = Task.Run(async () =>
        {
            using var conn = await server.AcceptAsync();
            var hello = await conn.ReadRequestAsync();
            await conn.SendOkAsync(hello.Id, HelloOkReply());
            var search = await conn.ReadRequestAsync();
            await conn.SendErrAsync(search.Id, "[collection_not_found] no such collection: docs");
        });

        await using var client = await ThunderClient.ConnectAsync(
            server.Address, Profile.Vectorizer);
        var error = await Assert.ThrowsAsync<ThunderServerException>(
            () => client.CallAsync("SEARCH"));
        Assert.Equal("[collection_not_found] no such collection: docs", error.Message);
        Assert.Equal("collection_not_found", error.Code);
        await serverTask;
    }

    // ── Push frames (CLT-060) ───────────────────────────────────────────

    [Fact]
    public async Task Push_frames_route_to_handler_under_enabled()
    {
        using var server = new MockServer();
        var serverTask = Task.Run(async () =>
        {
            using var conn = await server.AcceptAsync();
            var request = await conn.ReadRequestAsync();
            // A push frame in front of the response: it must reach the
            // handler and never be matched against the pending call.
            await conn.SendRawAsync(
                FrameCodec.EncodeResponse(Response.Ok(Wire.PushId, Value.Str("evt"))));
            await conn.SendOkAsync(request.Id, Value.Str("PONG"));
        });

        var profile = PlainProfile() with { Push = PushPolicy.Enabled };
        await using var client = await ThunderClient.ConnectAsync(server.Address, profile);
        var pushed = new TaskCompletionSource<Value>(
            TaskCreationOptions.RunContinuationsAsynchronously);
        client.OnPush(value => pushed.TrySetResult(value));
        Assert.Equal("PONG", (await client.CallAsync("SUBSCRIBE")).AsStr());
        Assert.Equal("evt", (await pushed.Task.WaitAsync(TimeSpan.FromSeconds(5))).AsStr());
        Assert.Equal(0, client.UnknownResponseDrops);
        await serverTask;
    }

    [Fact]
    public async Task Push_frame_under_reserved_profile_poisons_connection()
    {
        using var server = new MockServer();
        var serverTask = Task.Run(async () =>
        {
            using (var conn = await server.AcceptAsync())
            {
                await conn.ReadRequestAsync();
                await conn.SendRawAsync(
                    FrameCodec.EncodeResponse(Response.Ok(Wire.PushId, Value.Null)));
                // Keep writing nothing; the client poisons on its own.
            }

            // The next call may reconnect (CLT-014/030): serve it.
            using var second = await server.AcceptAsync();
            var request = await second.ReadRequestAsync();
            await second.SendOkAsync(request.Id, Value.Str("recovered"));
        });

        await using var client = await ThunderClient.ConnectAsync(server.Address, PlainProfile());
        // Push under Reserved is a protocol error (CLT-060).
        await Assert.ThrowsAsync<ThunderDecodeException>(() => client.CallAsync("GET"));
        // Poisoned connection, lazy reconnect on the next call.
        Assert.Equal("recovered", (await client.CallAsync("GET")).AsStr());
        await serverTask;
    }

    // ── Poisoning (CLT-014) ─────────────────────────────────────────────

    [Fact]
    public async Task Oversized_inbound_frame_fails_typed_and_poisons()
    {
        using var server = new MockServer();
        var serverTask = Task.Run(async () =>
        {
            using (var conn = await server.AcceptAsync())
            {
                await conn.ReadRequestAsync();
                // A length prefix past the profile cap — the client must
                // refuse on the prefix alone, before any body exists.
                await conn.SendRawAsync(BitConverter.GetBytes(1_000u));
            }

            using var second = await server.AcceptAsync();
            var request = await second.ReadRequestAsync();
            await second.SendOkAsync(request.Id, Value.Str("recovered"));
        });

        var profile = PlainProfile() with { MaxFrameBytes = 64 };
        await using var client = await ThunderClient.ConnectAsync(server.Address, profile);
        await Assert.ThrowsAsync<ThunderFrameTooLargeException>(() => client.CallAsync("GET"));
        Assert.Equal("recovered", (await client.CallAsync("GET")).AsStr());
        await serverTask;
    }

    [Fact]
    public async Task Malformed_frame_poisons_with_decode_error()
    {
        using var server = new MockServer();
        var serverTask = Task.Run(async () =>
        {
            using var conn = await server.AcceptAsync();
            await conn.ReadRequestAsync();
            // Valid length prefix, garbage body (0xc1 is never valid
            // MessagePack).
            await conn.SendRawAsync(
                BitConverter.GetBytes(4u).Concat(new byte[] { 0xc1, 0xc1, 0xc1, 0xc1 }).ToArray());
        });

        await using var client = await ThunderClient.ConnectAsync(server.Address, PlainProfile());
        await Assert.ThrowsAsync<ThunderDecodeException>(() => client.CallAsync("GET"));
        await serverTask;
    }

    // ── Lifecycle (CLT-004) ─────────────────────────────────────────────

    [Fact]
    public async Task Close_is_idempotent_and_fails_in_flight_calls()
    {
        using var server = new MockServer();
        var serverTask = Task.Run(async () =>
        {
            using var conn = await server.AcceptAsync();
            try
            {
                // Swallow requests, never answer; wait out the client close.
                await conn.ReadRequestAsync();
                await conn.ReadRequestAsync();
            }
            catch (IOException)
            {
                // The client closed the socket — expected.
            }
        });

        var client = await ThunderClient.ConnectAsync(server.Address, PlainProfile());
        var pending = client.CallAsync("HANG");
        await Task.Delay(100);

        client.Close();
        client.Close(); // idempotent (CLT-004)

        // In-flight calls fail with the typed connection-closed error.
        await Assert.ThrowsAsync<ThunderConnectionException>(() => pending);
        await Assert.ThrowsAsync<ThunderConnectionException>(() => client.CallAsync("AFTER"));
        await serverTask;
    }

    // ── Endpoints (CLT-070) ─────────────────────────────────────────────

    [Fact]
    public async Task Http_url_is_rejected_at_connect()
    {
        var error = await Assert.ThrowsAsync<ThunderConnectionException>(
            () => ThunderClient.ConnectAsync("http://localhost:8080", PlainProfile()));
        Assert.Contains("RPC-only", error.Message, StringComparison.Ordinal);
        Assert.Contains("HTTP client", error.Message, StringComparison.Ordinal);
    }
}

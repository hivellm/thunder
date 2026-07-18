// Cross-language live-interop probe (C# client vs the Rust server).
//
//   dotnet run --project csharp/interop-probe -- client <port>
//
// Speaks the family standard config (mandatory HELLO map). Prints "OK" + exit 0
// on success, "FAIL: <why>" + exit 1 otherwise. The server is Rust-only
// (SPEC-004), so this probe is client-only.
using HiveLLM.Thunder;

if (args.Length != 2 || args[0] != "client")
{
    Console.Error.WriteLine("usage: interop-probe client <port> (server is Rust-only)");
    return 2;
}

const string payload = "cross-language-🌩";
var port = int.Parse(args[1]);
var config = Config.Standard() with { Scheme = "interop", DefaultPort = 0 };

ThunderClient client;
try
{
    client = await ThunderClient.ConnectAsync(
        $"127.0.0.1:{port}", config, new ClientConfig { ClientName = "csharp" });
}
catch (Exception e)
{
    Console.WriteLine($"FAIL: connect/handshake failed: {e.Message}");
    return 1;
}

try
{
    var pong = await client.CallAsync("PING");
    if (pong.AsStr() != "PONG")
    {
        Console.WriteLine($"FAIL: PING returned {pong}, want PONG");
        return 1;
    }

    var echo = await client.CallAsync("ECHO", new[] { Value.Str(payload) });
    if (echo.AsStr() != payload)
    {
        Console.WriteLine($"FAIL: ECHO returned {echo}, want {payload}");
        return 1;
    }

    var errored = false;
    try
    {
        await client.CallAsync("NOPE");
    }
    catch (ThunderException)
    {
        errored = true; // a typed error is exactly right
    }

    if (!errored)
    {
        Console.WriteLine("FAIL: NOPE returned ok, want a typed error");
        return 1;
    }
}
finally
{
    await client.DisposeAsync();
}

Console.WriteLine("OK");
return 0;

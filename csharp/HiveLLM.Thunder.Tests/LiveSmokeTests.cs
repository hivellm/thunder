using Xunit;

namespace HiveLLM.Thunder.Tests;

/// <summary>
/// Live interop smoke (TST-050) — the C# client against a REAL product
/// instance. Env-gated and skipped by default: set any of
/// THUNDER_LIVE_URL_SYNAP / _NEXUS / _VECTORIZER to a reachable endpoint
/// (e.g. <c>synap://host:port</c>) and this connects with that product's
/// deployment shape (BN-023), makes a PING-class call, one typed-error call,
/// and closes. With none set it skips and passes — not part of the always-on
/// floor.
/// </summary>
public class LiveSmokeTests
{
    private static Config Synap() => Config.Standard() with
    {
        Scheme = "synap",
        Handshake = Handshake.AuthCommand,
        HelloStyle = HelloStyle.NotUsed,
        ErrorCodes = ErrorConvention.Resp3Prefixes,
    };

    private static Config Nexus() => Config.Standard() with
    {
        Scheme = "nexus",
        Handshake = Handshake.AuthCommand,
        HelloStyle = HelloStyle.ArgLess,
        ErrorCodes = ErrorConvention.Resp3Prefixes,
    };

    private static Config Vectorizer() => Config.Standard() with { Scheme = "vectorizer" };

    [Fact]
    public async Task LiveInteropSmoke()
    {
        var products = new (string Env, Func<Config> Shape)[]
        {
            ("THUNDER_LIVE_URL_SYNAP", Synap),
            ("THUNDER_LIVE_URL_NEXUS", Nexus),
            ("THUNDER_LIVE_URL_VECTORIZER", Vectorizer),
        };

        foreach (var (env, shape) in products)
        {
            var url = Environment.GetEnvironmentVariable(env);
            if (string.IsNullOrEmpty(url))
            {
                // Skipped (release-path only) — nothing to assert.
                continue;
            }

            await using var client = await ThunderClient.ConnectAsync(
                url, shape(), new ClientConfig { ClientName = "thunder-live-smoke" });

            // A PING-class call must succeed.
            await client.CallAsync("PING");

            // A command no product implements must come back a typed error.
            var errored = false;
            try
            {
                await client.CallAsync("__thunder_live_smoke_unknown__");
            }
            catch (ThunderException)
            {
                errored = true;
            }

            Assert.True(errored, $"{env}: bogus command returned ok, expected a typed error");
        }
    }
}

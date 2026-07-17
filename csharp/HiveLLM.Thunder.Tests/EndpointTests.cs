using Xunit;

namespace HiveLLM.Thunder.Tests;

/// <summary>
/// Endpoint parsing (CLT-070/071) — mirrors
/// rust/thunder/src/client/endpoint.rs tests.
/// </summary>
public class EndpointTests
{
    /// <summary>
    /// An application's config — Thunder ships no schemes of its own, so the
    /// tests bring their own, exactly as an application does.
    /// </summary>
    private static Config App() =>
        Config.Standard() with { Scheme = "myapp", DefaultPort = 9000 };

    [Fact]
    public void The_configured_scheme_resolves_the_configured_default_port()
    {
        // CLT-071: scheme → default port comes from the application's own
        // config, not from any registry Thunder carries.
        var endpoint = Endpoint.Parse("myapp://db.example.com", App());
        Assert.Equal("db.example.com", endpoint.Host);
        Assert.Equal(9000, endpoint.Port);
    }

    [Fact]
    public void Any_application_can_pick_any_scheme_without_a_thunder_release()
    {
        // The whole point of dropping the registry: a scheme Thunder has
        // never heard of works because the application configured it.
        var future = Config.Standard() with
        {
            Scheme = "something-new-in-2030",
            DefaultPort = 4242,
        };
        Assert.Equal(4242, Endpoint.Parse("something-new-in-2030://host", future).Port);
    }

    [Fact]
    public void Explicit_port_wins_over_default()
    {
        Assert.Equal(new Endpoint("10.0.0.7", 9999), Endpoint.Parse("myapp://10.0.0.7:9999", App()));
    }

    [Fact]
    public void Bare_host_port_is_accepted_rpc_implied()
    {
        Assert.Equal(new Endpoint("localhost", 15501), Endpoint.Parse("localhost:15501", App()));
    }

    [Fact]
    public void Bare_host_port_works_even_with_no_scheme_configured()
    {
        // Config.Standard() has no identity until an application gives it
        // one; an explicit host:port needs none.
        Assert.Equal(15501, Endpoint.Parse("localhost:15501", Config.Standard()).Port);
    }

    [Fact]
    public void Bare_host_without_port_is_rejected()
    {
        var error = Assert.Throws<ThunderConnectionException>(
            () => Endpoint.Parse("localhost", App()));
        Assert.Equal(ThunderErrorClass.Connection, error.ErrorClass);
    }

    [Theory]
    [InlineData("http://vec.example.com:8080")]
    [InlineData("https://vec.example.com")]
    public void Http_and_https_are_rejected_with_pointer_to_http_client(string url)
    {
        var error = Assert.Throws<ThunderConnectionException>(() => Endpoint.Parse(url, App()));
        Assert.Contains("RPC-only", error.Message, StringComparison.Ordinal);
        Assert.Contains("HTTP client", error.Message, StringComparison.Ordinal);
    }

    [Fact]
    public void A_scheme_other_than_the_configured_one_is_rejected()
    {
        var error = Assert.Throws<ThunderConnectionException>(
            () => Endpoint.Parse("redis://h:1", App()));
        // The mismatch must name both the given and the configured scheme.
        Assert.Contains("redis", error.Message, StringComparison.Ordinal);
        Assert.Contains("myapp", error.Message, StringComparison.Ordinal);
    }

    [Fact]
    public void Ipv6_literals_parse_with_and_without_brackets()
    {
        Assert.Equal(new Endpoint("::1", 8080), Endpoint.Parse("[::1]:8080", App()));
        var endpoint = Endpoint.Parse("myapp://[fe80::1]", App());
        Assert.Equal("fe80::1", endpoint.Host);
        Assert.Equal(9000, endpoint.Port);
    }

    [Fact]
    public void Trailing_slash_is_tolerated_but_paths_are_not()
    {
        Assert.Equal(9000, Endpoint.Parse("myapp://h/", App()).Port);
        Assert.Throws<ThunderConnectionException>(() => Endpoint.Parse("myapp://h/db", App()));
    }

    [Theory]
    [InlineData("host:99999")]
    [InlineData("myapp://host:abc")]
    [InlineData(":1234")]
    [InlineData("myapp://:1234")]
    public void Invalid_ports_and_empty_hosts_are_rejected(string input)
    {
        Assert.Throws<ThunderConnectionException>(() => Endpoint.Parse(input, App()));
    }
}

using Xunit;

namespace HiveLLM.Thunder.Tests;

/// <summary>
/// Endpoint parsing (CLT-070/071) — mirrors
/// rust/thunder-client/src/endpoint.rs tests.
/// </summary>
public class EndpointTests
{
    [Fact]
    public void Every_registered_scheme_resolves_its_default_port()
    {
        // CLT-071: scheme → default port comes from the registry.
        foreach (var profile in Profile.Registry)
        {
            var endpoint = Endpoint.Parse($"{profile.Scheme}://db.example.com");
            Assert.Equal("db.example.com", endpoint.Host);
            Assert.Equal(profile.DefaultPort, endpoint.Port);
        }
    }

    [Fact]
    public void Explicit_port_wins_over_default()
    {
        Assert.Equal(new Endpoint("10.0.0.7", 9999), Endpoint.Parse("nexus://10.0.0.7:9999"));
    }

    [Fact]
    public void Bare_host_port_is_accepted_rpc_implied()
    {
        Assert.Equal(new Endpoint("localhost", 15501), Endpoint.Parse("localhost:15501"));
    }

    [Fact]
    public void Bare_host_without_port_is_rejected()
    {
        var error = Assert.Throws<ThunderConnectionException>(() => Endpoint.Parse("localhost"));
        Assert.Equal(ThunderErrorClass.Connection, error.ErrorClass);
    }

    [Theory]
    [InlineData("http://vec.example.com:8080")]
    [InlineData("https://vec.example.com")]
    public void Http_and_https_are_rejected_with_pointer_to_http_client(string url)
    {
        var error = Assert.Throws<ThunderConnectionException>(() => Endpoint.Parse(url));
        Assert.Contains("RPC-only", error.Message, StringComparison.Ordinal);
        Assert.Contains("HTTP client", error.Message, StringComparison.Ordinal);
    }

    [Fact]
    public void Unknown_scheme_is_rejected_listing_the_registry()
    {
        var error = Assert.Throws<ThunderConnectionException>(() => Endpoint.Parse("redis://h:1"));
        foreach (var scheme in new[] { "synap", "nexus", "vectorizer", "lexum" })
        {
            Assert.Contains(scheme, error.Message, StringComparison.Ordinal);
        }
    }

    [Fact]
    public void Ipv6_literals_parse_with_and_without_brackets()
    {
        Assert.Equal(new Endpoint("::1", 8080), Endpoint.Parse("[::1]:8080"));
        var endpoint = Endpoint.Parse("synap://[fe80::1]");
        Assert.Equal("fe80::1", endpoint.Host);
        Assert.Equal(Profile.Synap.DefaultPort, endpoint.Port);
    }

    [Fact]
    public void Trailing_slash_is_tolerated_but_paths_are_not()
    {
        Assert.Equal(Profile.Lexum.DefaultPort, Endpoint.Parse("lexum://h/").Port);
        Assert.Throws<ThunderConnectionException>(() => Endpoint.Parse("lexum://h/db"));
    }

    [Theory]
    [InlineData("host:99999")]
    [InlineData("synap://host:abc")]
    [InlineData(":1234")]
    public void Invalid_ports_and_empty_hosts_are_rejected(string input)
    {
        Assert.Throws<ThunderConnectionException>(() => Endpoint.Parse(input));
    }
}

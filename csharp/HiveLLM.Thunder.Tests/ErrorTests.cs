using Xunit;

namespace HiveLLM.Thunder.Tests;

/// <summary>
/// Error-string parsing per the profile conventions (CLT-050..052,
/// PRO-014) — mirrors rust/thunder-client/src/error.rs tests exactly.
/// </summary>
public class ErrorTests
{
    [Theory]
    [InlineData("NOAUTH Authentication required.")]
    [InlineData("WRONGPASS invalid username-password pair or user is disabled.")]
    [InlineData("NOPERM this user has no permissions")]
    [InlineData("NOAUTH")]
    public void Resp3_auth_prefixes_map_to_auth_class(string message)
    {
        var error = ThunderException.FromServerMessage(message, ErrorConvention.Resp3Prefixes);
        var auth = Assert.IsType<ThunderAuthException>(error);
        Assert.Equal(message, auth.Message);
        Assert.Equal(ThunderErrorClass.Auth, auth.ErrorClass);
    }

    [Fact]
    public void Resp3_err_prefix_is_generic_server_error_without_code()
    {
        var error = ThunderException.FromServerMessage(
            "ERR unknown command", ErrorConvention.Resp3Prefixes);
        var server = Assert.IsType<ThunderServerException>(error);
        Assert.Equal("ERR unknown command", server.Message);
        Assert.Null(server.Code);
    }

    [Fact]
    public void Resp3_prefix_must_be_word_aligned()
    {
        // "NOAUTHx" is not the NOAUTH prefix.
        var error = ThunderException.FromServerMessage(
            "NOAUTHx nope", ErrorConvention.Resp3Prefixes);
        Assert.IsType<ThunderServerException>(error);
    }

    [Fact]
    public void Bracket_code_extracts_structured_code_and_keeps_raw_message()
    {
        const string raw = "[collection_not_found] no such collection: docs";
        var error = ThunderException.FromServerMessage(raw, ErrorConvention.BracketCode);
        var server = Assert.IsType<ThunderServerException>(error);
        Assert.Equal(raw, server.Message);
        Assert.Equal("collection_not_found", server.Code);
    }

    [Fact]
    public void Bracket_code_still_maps_auth_prefixes_to_auth_class()
    {
        // CLT-051: auth prefixes win regardless of convention.
        const string raw = "[unauthorized] NOAUTH token expired";
        var error = ThunderException.FromServerMessage(raw, ErrorConvention.BracketCode);
        var auth = Assert.IsType<ThunderAuthException>(error);
        Assert.Equal(raw, auth.Message);
    }

    [Fact]
    public void Both_convention_composes_bracket_and_prefixes()
    {
        var auth = ThunderException.FromServerMessage(
            "[wrongpass] WRONGPASS bad credentials", ErrorConvention.Both);
        Assert.IsType<ThunderAuthException>(auth);

        var server = Assert.IsType<ThunderServerException>(
            ThunderException.FromServerMessage(
                "[index_missing] ERR no such index", ErrorConvention.Both));
        Assert.Equal("[index_missing] ERR no such index", server.Message);
        Assert.Equal("index_missing", server.Code);
    }

    [Fact]
    public void None_convention_never_parses()
    {
        var error = ThunderException.FromServerMessage(
            "NOAUTH raw passthrough", ErrorConvention.None);
        var server = Assert.IsType<ThunderServerException>(error);
        Assert.Equal("NOAUTH raw passthrough", server.Message);
        Assert.Null(server.Code);
    }

    [Theory]
    [InlineData("[] empty")]
    [InlineData("[has space] x")]
    [InlineData("[nospace]tail")]
    [InlineData("[unclosed")]
    public void Malformed_bracket_prefixes_are_left_alone(string message)
    {
        var error = ThunderException.FromServerMessage(message, ErrorConvention.BracketCode);
        var server = Assert.IsType<ThunderServerException>(error);
        Assert.Equal(message, server.Message);
        Assert.Null(server.Code);
    }
}

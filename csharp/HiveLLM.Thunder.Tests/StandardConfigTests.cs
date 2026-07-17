using System.Globalization;
using Xunit;
using YamlDotNet.RepresentationModel;

namespace HiveLLM.Thunder.Tests;

/// <summary>
/// Pins <see cref="Config.Standard"/> to <c>conformance/standard.yaml</c>
/// (PRO-013), and proves the model the standard replaced the registry with.
/// <para>
/// Thunder ships <b>one</b> standard and no product knowledge, so the pinning
/// is the whole registry check: a change to the standard that is not mirrored
/// in the language-neutral YAML — or vice versa — fails here, in all four
/// languages. That cross-language agreement was the only job the old
/// per-product registry legitimately did; it survives without any product
/// name (mirrors rust/thunder/tests/standard_config.rs).
/// </para>
/// </summary>
public class StandardConfigTests
{
    [Fact]
    public void Standard_matches_the_conformance_data_file()
    {
        var path = Path.Combine(TestSupport.ConformanceDir, "standard.yaml");
        Assert.True(File.Exists(path), $"{path} must exist");

        var stream = new YamlStream();
        using var reader = new StreamReader(path);
        stream.Load(reader);
        var root = (YamlMappingNode)stream.Documents[0].RootNode;

        var standard = Config.Standard();
        Assert.Equal(ParseHandshake(Scalar(root, "handshake")!), standard.Handshake);
        Assert.Equal(ParseHelloStyle(Scalar(root, "hello_style")), standard.HelloStyle);
        Assert.Equal(ParsePush(Scalar(root, "push")!), standard.Push);
        Assert.Equal(
            int.Parse(Scalar(root, "max_frame_bytes")!, CultureInfo.InvariantCulture),
            standard.MaxFrameBytes);
        Assert.Equal(
            int.Parse(Scalar(root, "max_in_flight")!, CultureInfo.InvariantCulture),
            standard.MaxInFlight);
        Assert.Equal(ParseErrors(Scalar(root, "error_codes")!), standard.ErrorCodes);
        Assert.Equal(ParseTls(Scalar(root, "tls")!), standard.Tls);
    }

    [Fact]
    public void The_standard_carries_no_identity()
    {
        // Identity is the application's: Thunder has no opinion about which
        // scheme or port an implementation answers on.
        var standard = Config.Standard();
        Assert.Equal("", standard.Scheme);
        Assert.Equal(0, standard.DefaultPort);
    }

    [Fact]
    public void An_application_configures_itself_without_a_thunder_release()
    {
        // The whole point: a product Thunder has never heard of — including
        // one that does not exist yet — is expressible today.
        var future = Config.Standard() with
        {
            Scheme = "nobody-shipped-this-yet",
            DefaultPort = 4242,
        };
        Assert.Equal("nobody-shipped-this-yet", future.Scheme);
        Assert.Equal(4242, future.DefaultPort);

        // …and it inherits every standard behavior it did not override.
        Assert.Equal(Config.Standard().Handshake, future.Handshake);
        Assert.Equal(Config.Standard().ErrorCodes, future.ErrorCodes);
    }

    [Fact]
    public void Overrides_compose_and_leave_the_rest_standard()
    {
        // A deployment that still diverges says so in its own repository.
        var diverging = Config.Standard() with
        {
            Scheme = "legacy",
            DefaultPort = 15501,
            Handshake = Handshake.AuthCommand,
            HelloStyle = HelloStyle.NotUsed,
            Push = PushPolicy.Enabled,
            MaxFrameBytes = 512 * 1024 * 1024,
            ErrorCodes = ErrorConvention.Resp3Prefixes,
        };

        Assert.Equal(Handshake.AuthCommand, diverging.Handshake);
        Assert.Equal(PushPolicy.Enabled, diverging.Push);
        Assert.Equal(512 * 1024 * 1024, diverging.MaxFrameBytes);

        // Untouched dimensions stay standard — convergence is "delete
        // overrides until only identity remains".
        Assert.Equal(Config.Standard().MaxInFlight, diverging.MaxInFlight);
        Assert.Equal(Config.Standard().Tls, diverging.Tls);
    }

    [Fact]
    public void A_config_is_still_a_plain_record()
    {
        // Configs are data (PRO-003): plain construction must keep working,
        // so nothing forces an application through Standard() + with.
        var literal = new Config
        {
            Scheme = "plain",
            DefaultPort = 1,
            Handshake = Handshake.None,
            HelloStyle = HelloStyle.NotUsed,
            Push = PushPolicy.Reserved,
            MaxFrameBytes = 1024,
            MaxInFlight = 2,
            ErrorCodes = ErrorConvention.None,
            Tls = TlsPolicy.Off,
        };
        Assert.Equal("plain", literal.Scheme);
    }

    private static string? Scalar(YamlMappingNode map, string key)
    {
        foreach (var entry in map.Children)
        {
            if (entry.Key is YamlScalarNode scalar && scalar.Value == key)
            {
                return (entry.Value as YamlScalarNode)?.Value;
            }
        }

        return null;
    }

    private static Handshake ParseHandshake(string raw) => raw switch
    {
        "none" => Handshake.None,
        "auth_command" => Handshake.AuthCommand,
        "hello_mandatory" => Handshake.HelloMandatory,
        _ => throw new InvalidDataException($"unknown handshake '{raw}'"),
    };

    private static HelloStyle ParseHelloStyle(string? raw) => raw switch
    {
        null or "" or "null" or "~" => HelloStyle.NotUsed,
        "arg_less" => HelloStyle.ArgLess,
        "map_payload" => HelloStyle.MapPayload,
        _ => throw new InvalidDataException($"unknown hello_style '{raw}'"),
    };

    private static PushPolicy ParsePush(string raw) => raw switch
    {
        "reserved" => PushPolicy.Reserved,
        "enabled" => PushPolicy.Enabled,
        _ => throw new InvalidDataException($"unknown push '{raw}'"),
    };

    private static ErrorConvention ParseErrors(string raw) => raw switch
    {
        "none" => ErrorConvention.None,
        "resp3_prefixes" => ErrorConvention.Resp3Prefixes,
        "bracket_code" => ErrorConvention.BracketCode,
        "both" => ErrorConvention.Both,
        _ => throw new InvalidDataException($"unknown error_codes '{raw}'"),
    };

    private static TlsPolicy ParseTls(string raw) => raw switch
    {
        "off" => TlsPolicy.Off,
        "optional_rustls" => TlsPolicy.Optional,
        "reserved_config" => TlsPolicy.Reserved,
        _ => throw new InvalidDataException($"unknown tls '{raw}'"),
    };
}

using System.Globalization;
using Xunit;
using YamlDotNet.RepresentationModel;

namespace HiveLLM.Thunder.Tests;

/// <summary>
/// Pins the <see cref="Profile"/> registry constants to the language-neutral
/// data files in <c>conformance/profiles/</c> (PRO-010/013): a registry edit
/// that is not mirrored in the YAML — or vice versa — fails here (mirrors
/// rust/thunder-wire/tests/profiles.rs).
/// </summary>
public class ProfileRegistryTests
{
    [Fact]
    public void Registry_lists_all_four_family_profiles()
    {
        Assert.Equal(
            new[] { "synap", "nexus", "vectorizer", "lexum" },
            Profile.Registry.Select(p => p.Name));
    }

    public static TheoryData<string> ProfileNames()
    {
        var data = new TheoryData<string>();
        foreach (var profile in Profile.Registry)
        {
            data.Add(profile.Name);
        }

        return data;
    }

    [Theory]
    [MemberData(nameof(ProfileNames))]
    public void Registry_constants_match_conformance_profiles(string name)
    {
        var profile = Profile.Registry.Single(p => p.Name == name);
        var path = Path.Combine(TestSupport.ConformanceDir, "profiles", $"{name}.yaml");
        Assert.True(File.Exists(path), $"profile yaml for {name} must exist");

        var stream = new YamlStream();
        using var reader = new StreamReader(path);
        stream.Load(reader);
        var root = (YamlMappingNode)stream.Documents[0].RootNode;

        Assert.Equal(Scalar(root, "name"), profile.Name);
        Assert.Equal(Scalar(root, "scheme"), profile.Scheme);
        Assert.Equal(
            ushort.Parse(Scalar(root, "default_port")!, CultureInfo.InvariantCulture),
            profile.DefaultPort);
        Assert.Equal(
            int.Parse(Scalar(root, "max_frame_bytes")!, CultureInfo.InvariantCulture),
            profile.MaxFrameBytes);
        Assert.Equal(
            int.Parse(Scalar(root, "max_in_flight")!, CultureInfo.InvariantCulture),
            profile.MaxInFlight);
        Assert.Equal(ParseHandshake(Scalar(root, "handshake")!), profile.Handshake);
        Assert.Equal(ParseHelloStyle(Scalar(root, "hello_style")), profile.HelloStyle);
        Assert.Equal(ParsePush(Scalar(root, "push")!), profile.Push);
        Assert.Equal(ParseErrors(Scalar(root, "error_codes")!), profile.ErrorCodes);
        Assert.Equal(ParseTls(Scalar(root, "tls")!), profile.Tls);
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

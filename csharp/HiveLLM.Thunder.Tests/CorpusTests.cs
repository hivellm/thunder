using System.Globalization;
using Xunit;
using YamlDotNet.RepresentationModel;

namespace HiveLLM.Thunder.Tests;

/// <summary>
/// Conformance-corpus loader (TST-020): walks <c>conformance/vectors/</c>
/// and asserts every vector per its <c>mode</c>. Runs in the default test
/// command — never gated, never skipped (NFR-03).
///
/// Mode semantics (TST-002, conformance/README.md):
/// bidirectional — encode(decoded) == frame byte-exact AND decode(frame) ==
/// decoded structurally (floats by bit pattern); decode-only — decode
/// succeeds and equals decoded, while the canonical encoding must NOT
/// reproduce the legacy bytes; stream — back-to-back frames decode one per
/// call, consuming the buffer exactly; incomplete — the decoder asks for
/// more bytes (no value, no error); reject — decode fails with the named
/// error class.
/// </summary>
public class CorpusTests
{
    /// <summary>Every vector file, one theory case each.</summary>
    public static TheoryData<string> VectorFiles()
    {
        var data = new TheoryData<string>();
        foreach (var path in VectorPaths())
        {
            data.Add(Path.GetFileName(path));
        }

        return data;
    }

    private static IEnumerable<string> VectorPaths() =>
        Directory.EnumerateFiles(
                Path.Combine(TestSupport.ConformanceDir, "vectors"), "*.yaml")
            .OrderBy(p => p, StringComparer.Ordinal);

    [Fact]
    public void Corpus_does_not_silently_shrink()
    {
        Assert.True(
            VectorPaths().Count() >= 38,
            $"corpus must not silently shrink (found {VectorPaths().Count()}, floor 38)");
    }

    [Theory]
    [MemberData(nameof(VectorFiles))]
    public void Corpus_vector_holds(string fileName)
    {
        var vector = LoadVector(
            Path.Combine(TestSupport.ConformanceDir, "vectors", fileName));
        var frame = TestSupport.ParseHex(vector.FrameHex);
        var max = vector.MaxFrameBytes ?? Wire.DefaultMaxFrameBytes;

        switch (vector.Mode)
        {
            case "bidirectional":
            {
                var expected = ParseDecoded(vector.Decoded!);
                // encode(decoded) == frame, byte-exact.
                Assert.Equal(TestSupport.ToHex(frame), TestSupport.ToHex(Encode(expected)));
                // decode(frame) == decoded, structurally (floats by bits).
                var consumed = AssertDecodes(expected, frame, max, vector.Name);
                Assert.Equal(frame.Length, consumed);
                break;
            }

            case "decode-only":
            {
                var expected = ParseDecoded(vector.Decoded!);
                var consumed = AssertDecodes(expected, frame, max, vector.Name);
                Assert.Equal(frame.Length, consumed);
                // Encoding this form is forbidden: the canonical encoding of
                // the same structure must NOT reproduce the legacy bytes.
                Assert.NotEqual(TestSupport.ToHex(frame), TestSupport.ToHex(Encode(expected)));
                break;
            }

            case "stream":
            {
                var offset = 0;
                var index = 0;
                foreach (var node in vector.Frames!)
                {
                    var expected = ParseDecoded(node);
                    offset += AssertDecodes(
                        expected,
                        frame.AsMemory(offset),
                        max,
                        $"{vector.Name}[{index}]");
                    index++;
                }

                Assert.Equal(frame.Length, offset);
                break;
            }

            case "incomplete":
            {
                Assert.False(
                    FrameCodec.TryDecodeRequest(frame, max, out _, out _),
                    $"{vector.Name}: incomplete input must ask for more bytes, not decode");
                break;
            }

            case "reject":
            {
                switch (vector.Error)
                {
                    case "frame_too_large":
                        Assert.Throws<ThunderFrameTooLargeException>(
                            () => FrameCodec.TryDecodeRequest(frame, max, out _, out _));
                        break;
                    case "decode":
                        Assert.Throws<ThunderDecodeException>(
                            () => FrameCodec.TryDecodeRequest(frame, max, out _, out _));
                        break;
                    default:
                        Assert.Fail($"{vector.Name}: unknown error class '{vector.Error}'");
                        break;
                }

                break;
            }

            default:
                Assert.Fail($"{vector.Name}: unknown mode '{vector.Mode}'");
                break;
        }
    }

    // ── vector model ────────────────────────────────────────────────────

    private sealed record Vector(
        string Name,
        string Mode,
        string FrameHex,
        YamlMappingNode? Decoded,
        IReadOnlyList<YamlMappingNode>? Frames,
        string? Error,
        int? MaxFrameBytes);

    private static Vector LoadVector(string path)
    {
        var stream = new YamlStream();
        using var reader = new StreamReader(path);
        stream.Load(reader);
        var root = (YamlMappingNode)stream.Documents[0].RootNode;
        return new Vector(
            Scalar(root, "name")!,
            Scalar(root, "mode")!,
            Scalar(root, "frame_hex")!,
            Get(root, "decoded") as YamlMappingNode,
            (Get(root, "frames") as YamlSequenceNode)?.Children
                .Cast<YamlMappingNode>().ToArray(),
            Scalar(root, "error"),
            Scalar(root, "max_frame_bytes") is { } cap
                ? int.Parse(cap, CultureInfo.InvariantCulture)
                : null);
    }

    private static YamlNode? Get(YamlMappingNode map, string key)
    {
        foreach (var entry in map.Children)
        {
            if (entry.Key is YamlScalarNode scalar && scalar.Value == key)
            {
                return entry.Value;
            }
        }

        return null;
    }

    private static string? Scalar(YamlMappingNode map, string key) =>
        (Get(map, key) as YamlScalarNode)?.Value;

    // ── decoded → Request / Response ────────────────────────────────────

    /// <summary>Parse a <c>decoded</c> node into a <see cref="Request"/> or <see cref="Response"/>.</summary>
    private static object ParseDecoded(YamlMappingNode node)
    {
        var kind = Scalar(node, "kind");
        var id = uint.Parse(Scalar(node, "id")!, CultureInfo.InvariantCulture);
        switch (kind)
        {
            case "request":
            {
                var args = (Get(node, "args") as YamlSequenceNode)?.Children
                    .Select(n => NodeToValue((YamlMappingNode)n))
                    .ToArray() ?? System.Array.Empty<Value>();
                return new Request(id, Scalar(node, "command")!, args);
            }

            case "response":
            {
                var ok = Get(node, "ok");
                var err = Scalar(node, "err");
                if (ok is YamlMappingNode okNode)
                {
                    return Response.Ok(id, NodeToValue(okNode));
                }

                Assert.NotNull(err);
                return Response.Err(id, err);
            }

            default:
                throw new InvalidDataException($"unknown decoded kind '{kind}'");
        }
    }

    /// <summary>
    /// One <c>{type, value}</c> node; floats MAY carry <c>bits</c> instead —
    /// the u64 IEEE-754 pattern in hex — and are compared by bit pattern
    /// (NaN never compares equal numerically; -0.0 == 0.0 would hide the
    /// sign bit).
    /// </summary>
    private static Value NodeToValue(YamlMappingNode node)
    {
        var type = Scalar(node, "type");
        switch (type)
        {
            case "null":
                return Value.Null;
            case "bool":
                return Value.Bool(Scalar(node, "value") == "true");
            case "int":
                return Value.Int(long.Parse(Scalar(node, "value")!, CultureInfo.InvariantCulture));
            case "float":
                var bits = Scalar(node, "bits");
                return bits is not null
                    ? Value.Float(BitConverter.UInt64BitsToDouble(
                        ulong.Parse(bits, NumberStyles.HexNumber, CultureInfo.InvariantCulture)))
                    : Value.Float(ParseYamlDouble(Scalar(node, "value")!));
            case "str":
                return Value.Str(Scalar(node, "value")!);
            case "bytes":
                return Value.Bytes(TestSupport.ParseHex(Scalar(node, "value") ?? string.Empty));
            case "array":
                return Value.Array(
                    ((YamlSequenceNode)Get(node, "value")!).Children
                    .Select(n => NodeToValue((YamlMappingNode)n)));
            case "map":
                return Value.Map(
                    ((YamlSequenceNode)Get(node, "value")!).Children
                    .Select(pair =>
                    {
                        var kv = (YamlSequenceNode)pair;
                        Assert.Equal(2, kv.Children.Count);
                        return new KeyValuePair<Value, Value>(
                            NodeToValue((YamlMappingNode)kv.Children[0]),
                            NodeToValue((YamlMappingNode)kv.Children[1]));
                    }));
            default:
                throw new InvalidDataException($"unknown corpus node type '{type}'");
        }
    }

    private static double ParseYamlDouble(string scalar) => scalar switch
    {
        ".inf" or "+.inf" => double.PositiveInfinity,
        "-.inf" => double.NegativeInfinity,
        ".nan" => double.NaN,
        _ => double.Parse(scalar, CultureInfo.InvariantCulture),
    };

    // ── assertions ──────────────────────────────────────────────────────

    private static byte[] Encode(object expected) => expected switch
    {
        Request request => FrameCodec.EncodeRequest(request),
        Response response => FrameCodec.EncodeResponse(response),
        _ => throw new InvalidDataException("expected a Request or Response"),
    };

    /// <summary>
    /// Decode one frame from <paramref name="buffer"/> under
    /// <paramref name="max"/> and assert it equals <paramref name="expected"/>
    /// structurally. Returns the bytes consumed.
    /// </summary>
    private static int AssertDecodes(
        object expected, ReadOnlyMemory<byte> buffer, int max, string name)
    {
        if (expected is Request wantRequest)
        {
            Assert.True(
                FrameCodec.TryDecodeRequest(buffer, max, out var gotRequest, out var consumed),
                $"{name}: a complete frame must decode");
            Assert.Equal(wantRequest, gotRequest);
            return consumed;
        }

        var wantResponse = (Response)expected;
        Assert.True(
            FrameCodec.TryDecodeResponse(buffer, max, out var gotResponse, out var used),
            $"{name}: a complete frame must decode");
        Assert.Equal(wantResponse, gotResponse);
        return used;
    }
}

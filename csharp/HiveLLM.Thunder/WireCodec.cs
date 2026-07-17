using System.Buffers;
using MessagePack;

namespace HiveLLM.Thunder;

/// <summary>
/// MessagePack body codec — the externally-tagged rmp-serde encoding over
/// the 8-variant <see cref="Value"/> model, written with the low-level
/// <see cref="MessagePackWriter"/> / <see cref="MessagePackReader"/> API
/// only (WIRE-031; <c>MessagePackSerializer.Typeless</c> and attribute-based
/// serialization are forbidden, NFR-02).
///
/// Canonical emission (SPEC-001 §2): unit variant as the bare string
/// <c>"Null"</c>, payload variants as single-key maps (<c>{"Int": 42}</c>),
/// <c>Result</c> as the nested <c>{"Ok": value}</c> / <c>{"Err": string}</c>
/// (WIRE-003), Bytes as bin (WIRE-010), array-encoded Request/Response
/// (WIRE-012), shortest-form ints and f64 floats (WIRE-014). Decode
/// tolerances: int-array Bytes (WIRE-011) and map-shaped Request (WIRE-013),
/// both normalized and never re-emitted.
/// </summary>
internal static class WireCodec
{
    internal static byte[] EncodeRequestBody(Request request)
    {
        var buffer = new ArrayBufferWriter<byte>();
        var writer = new MessagePackWriter(buffer);
        writer.WriteArrayHeader(3);
        writer.Write(request.Id);
        writer.Write(request.Command);
        writer.WriteArrayHeader(request.Args.Count);
        foreach (var arg in request.Args)
        {
            WriteValue(ref writer, arg);
        }

        writer.Flush();
        return buffer.WrittenSpan.ToArray();
    }

    internal static byte[] EncodeResponseBody(Response response)
    {
        var buffer = new ArrayBufferWriter<byte>();
        var writer = new MessagePackWriter(buffer);
        writer.WriteArrayHeader(2);
        writer.Write(response.Id);
        writer.WriteMapHeader(1);
        if (response.IsOk)
        {
            writer.Write("Ok");
            WriteValue(ref writer, response.Value!);
        }
        else
        {
            writer.Write("Err");
            writer.Write(response.Error);
        }

        writer.Flush();
        return buffer.WrittenSpan.ToArray();
    }

    internal static Request DecodeRequestBody(ReadOnlyMemory<byte> body)
    {
        try
        {
            var reader = new MessagePackReader(body);
            var request = ReadRequest(ref reader);
            EnsureFullyConsumed(ref reader);
            return request;
        }
        catch (Exception e) when (IsDecodeFailure(e))
        {
            throw new ThunderDecodeException($"decode error: {e.Message}", e);
        }
    }

    internal static Response DecodeResponseBody(ReadOnlyMemory<byte> body)
    {
        try
        {
            var reader = new MessagePackReader(body);
            var response = ReadResponse(ref reader);
            EnsureFullyConsumed(ref reader);
            return response;
        }
        catch (Exception e) when (IsDecodeFailure(e))
        {
            throw new ThunderDecodeException($"decode error: {e.Message}", e);
        }
    }

    private static void WriteValue(ref MessagePackWriter writer, Value value)
    {
        switch (value)
        {
            case NullValue:
                // Unit variant: the bare string "Null" (WIRE-003).
                writer.Write("Null");
                break;
            case BoolValue b:
                writer.WriteMapHeader(1);
                writer.Write("Bool");
                writer.Write(b.Value);
                break;
            case IntValue i:
                // Shortest-form packing: positive values take the unsigned
                // family, negative the signed one (WIRE-014).
                writer.WriteMapHeader(1);
                writer.Write("Int");
                writer.Write(i.Value);
                break;
            case FloatValue f:
                // Always f64 (0xcb); the bit pattern round-trips (WIRE-014).
                writer.WriteMapHeader(1);
                writer.Write("Float");
                writer.Write(f.Value);
                break;
            case BytesValue bytes:
                // Canonical bin emission (WIRE-010); never the int-array form.
                writer.WriteMapHeader(1);
                writer.Write("Bytes");
                writer.Write(bytes.Value.AsSpan());
                break;
            case StrValue s:
                writer.WriteMapHeader(1);
                writer.Write("Str");
                writer.Write(s.Value);
                break;
            case ArrayValue array:
                writer.WriteMapHeader(1);
                writer.Write("Array");
                writer.WriteArrayHeader(array.Items.Count);
                foreach (var item in array.Items)
                {
                    WriteValue(ref writer, item);
                }

                break;
            case MapValue map:
                // Map is an ordered pair LIST: an array of [key, value]
                // 2-arrays, not a MessagePack map (WIRE-002).
                writer.WriteMapHeader(1);
                writer.Write("Map");
                writer.WriteArrayHeader(map.Pairs.Count);
                foreach (var pair in map.Pairs)
                {
                    writer.WriteArrayHeader(2);
                    WriteValue(ref writer, pair.Key);
                    WriteValue(ref writer, pair.Value);
                }

                break;
            default:
                throw new ThunderDecodeException($"unknown value kind {value.Kind}");
        }
    }

    private static Value ReadValue(ref MessagePackReader reader)
    {
        switch (reader.NextMessagePackType)
        {
            case MessagePackType.String:
                var unit = reader.ReadString();
                return unit == "Null"
                    ? Value.Null
                    : throw new ThunderDecodeException($"unknown unit variant '{unit}'");
            case MessagePackType.Map:
                var count = reader.ReadMapHeader();
                if (count != 1)
                {
                    throw new ThunderDecodeException(
                        $"value variant must be a single-key map, got {count} keys");
                }

                var variant = reader.ReadString();
                switch (variant)
                {
                    case "Bool":
                        return Value.Bool(reader.ReadBoolean());
                    case "Int":
                        return Value.Int(reader.ReadInt64());
                    case "Float":
                        return Value.Float(reader.ReadDouble());
                    case "Bytes":
                        return ReadBytesPayload(ref reader);
                    case "Str":
                        return Value.Str(
                            reader.ReadString()
                            ?? throw new ThunderDecodeException("Str payload must not be nil"));
                    case "Array":
                    {
                        var n = reader.ReadArrayHeader();
                        var items = new Value[n];
                        for (var i = 0; i < n; i++)
                        {
                            items[i] = ReadValue(ref reader);
                        }

                        return new ArrayValue(items);
                    }

                    case "Map":
                    {
                        var n = reader.ReadArrayHeader();
                        var pairs = new KeyValuePair<Value, Value>[n];
                        for (var i = 0; i < n; i++)
                        {
                            var pairLen = reader.ReadArrayHeader();
                            if (pairLen != 2)
                            {
                                throw new ThunderDecodeException(
                                    $"map entry must be a [key, value] pair, got {pairLen} items");
                            }

                            var key = ReadValue(ref reader);
                            var val = ReadValue(ref reader);
                            pairs[i] = new KeyValuePair<Value, Value>(key, val);
                        }

                        return new MapValue(pairs);
                    }

                    default:
                        throw new ThunderDecodeException($"unknown value variant '{variant}'");
                }

            default:
                throw new ThunderDecodeException(
                    $"expected a value (string or single-key map), got {reader.NextMessagePackType}");
        }
    }

    /// <summary>
    /// Bytes payload: canonical bin, or the legacy int-array form normalized
    /// on decode (WIRE-011). Emitting the legacy form is forbidden.
    /// </summary>
    private static Value ReadBytesPayload(ref MessagePackReader reader)
    {
        switch (reader.NextMessagePackType)
        {
            case MessagePackType.Binary:
                var sequence = reader.ReadBytes()
                    ?? throw new ThunderDecodeException("Bytes payload must not be nil");
                return new BytesValue(sequence.ToArray());
            case MessagePackType.Array:
                var n = reader.ReadArrayHeader();
                var bytes = new byte[n];
                for (var i = 0; i < n; i++)
                {
                    var b = reader.ReadInt32();
                    if (b is < 0 or > 255)
                    {
                        throw new ThunderDecodeException(
                            $"legacy int-array Bytes element {b} is out of the 0-255 range");
                    }

                    bytes[i] = (byte)b;
                }

                return new BytesValue(bytes);
            default:
                throw new ThunderDecodeException(
                    $"Bytes payload must be bin or a legacy int array, got {reader.NextMessagePackType}");
        }
    }

    private static Request ReadRequest(ref MessagePackReader reader)
    {
        switch (reader.NextMessagePackType)
        {
            case MessagePackType.Array:
            {
                var n = reader.ReadArrayHeader();
                if (n != 3)
                {
                    throw new ThunderDecodeException(
                        $"request must be [id, command, args], got {n} elements");
                }

                var id = reader.ReadUInt32();
                var command = reader.ReadString()
                    ?? throw new ThunderDecodeException("request command must not be nil");
                return new Request(id, command, ReadArgs(ref reader));
            }

            case MessagePackType.Map:
            {
                // Map-shaped Request tolerance (WIRE-013, legacy encoders).
                var n = reader.ReadMapHeader();
                uint? id = null;
                string? command = null;
                IReadOnlyList<Value>? args = null;
                for (var i = 0; i < n; i++)
                {
                    var key = reader.ReadString();
                    switch (key)
                    {
                        case "id":
                            id = reader.ReadUInt32();
                            break;
                        case "command":
                            command = reader.ReadString();
                            break;
                        case "args":
                            args = ReadArgs(ref reader);
                            break;
                        default:
                            reader.Skip();
                            break;
                    }
                }

                if (id is null || command is null || args is null)
                {
                    throw new ThunderDecodeException(
                        "map-shaped request must carry id, command and args");
                }

                return new Request(id.Value, command, args);
            }

            default:
                throw new ThunderDecodeException(
                    $"request must be an array or map, got {reader.NextMessagePackType}");
        }
    }

    private static IReadOnlyList<Value> ReadArgs(ref MessagePackReader reader)
    {
        var n = reader.ReadArrayHeader();
        var args = new Value[n];
        for (var i = 0; i < n; i++)
        {
            args[i] = ReadValue(ref reader);
        }

        return args;
    }

    private static Response ReadResponse(ref MessagePackReader reader)
    {
        var n = reader.ReadArrayHeader();
        if (n != 2)
        {
            throw new ThunderDecodeException($"response must be [id, result], got {n} elements");
        }

        var id = reader.ReadUInt32();
        var arms = reader.ReadMapHeader();
        if (arms != 1)
        {
            throw new ThunderDecodeException(
                $"response result must be a single-key map, got {arms} keys");
        }

        var arm = reader.ReadString();
        switch (arm)
        {
            case "Ok":
                return Response.Ok(id, ReadValue(ref reader));
            case "Err":
                var message = reader.ReadString()
                    ?? throw new ThunderDecodeException("Err payload must be a string");
                return Response.Err(id, message);
            default:
                throw new ThunderDecodeException($"unknown result arm '{arm}'");
        }
    }

    private static void EnsureFullyConsumed(ref MessagePackReader reader)
    {
        if (!reader.End)
        {
            throw new ThunderDecodeException("trailing bytes after the frame body");
        }
    }

    /// <summary>
    /// Failures the MessagePack reader raises on malformed input — wrapped
    /// into the typed decode class so nothing throws uncontrolled (WIRE-023).
    /// </summary>
    private static bool IsDecodeFailure(Exception e) =>
        e is MessagePackSerializationException
            or EndOfStreamException
            or OverflowException
            or InvalidOperationException
            or FormatException;
}

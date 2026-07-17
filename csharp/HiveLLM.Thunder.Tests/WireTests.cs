using Xunit;

namespace HiveLLM.Thunder.Tests;

/// <summary>
/// Wire-layer unit tests mirroring the Rust reference suite
/// (rust/thunder-wire/src/frame.rs): golden family-pinned vectors,
/// round-trip matrix, framing edges. The full byte corpus lives in
/// <see cref="CorpusTests"/>.
/// </summary>
public class WireTests
{
    [Fact]
    public void Ping_request_matches_family_golden_vector()
    {
        var request = new Request(1, "PING");
        var frame = FrameCodec.EncodeRequest(request);
        Assert.Equal("08 00 00 00 93 01 a4 50 49 4e 47 90", TestSupport.ToHex(frame));
        Assert.True(FrameCodec.TryDecodeRequest(frame, out var decoded, out var consumed));
        Assert.Equal(request, decoded);
        Assert.Equal(frame.Length, consumed);
    }

    [Fact]
    public void Pong_response_matches_nested_ok_golden_vector()
    {
        var response = Response.Ok(1, Value.Str("PONG"));
        var frame = FrameCodec.EncodeResponse(response);
        // Result nests two one-key maps: {"Ok": {"Str": "PONG"}} (WIRE-003).
        Assert.Equal(
            "10 00 00 00 92 01 81 a2 4f 6b 81 a3 53 74 72 a4 50 4f 4e 47",
            TestSupport.ToHex(frame));
        Assert.True(FrameCodec.TryDecodeResponse(frame, out var decoded, out _));
        Assert.Equal(response, decoded);
    }

    [Fact]
    public void Round_trip_all_variants()
    {
        var all = Value.Array(
            Value.Null,
            Value.Bool(true),
            Value.Bool(false),
            Value.Int(0),
            Value.Int(long.MinValue),
            Value.Int(long.MaxValue),
            Value.Int(-32),
            Value.Int(127),
            Value.Int(255),
            Value.Int(65535),
            Value.Float(0.0),
            Value.Float(-0.0),
            Value.Float(double.PositiveInfinity),
            Value.Float(double.NegativeInfinity),
            Value.Bytes(System.Array.Empty<byte>()),
            Value.Bytes(new byte[] { 0, 1, 2, 255 }),
            Value.Str(string.Empty),
            Value.Str("héllo wörld"),
            Value.Array(),
            Value.Map(),
            Value.Map(
                (Value.Str("k"), Value.Int(1)),
                (Value.Int(2), Value.Str("non-string key"))));
        var frame = FrameCodec.EncodeResponse(Response.Ok(7, all));
        Assert.True(FrameCodec.TryDecodeResponse(frame, out var decoded, out var consumed));
        Assert.Equal(Response.Ok(7, all), decoded);
        Assert.Equal(frame.Length, consumed);
    }

    [Fact]
    public void Nan_bit_pattern_survives()
    {
        var bits = 0x7ff8_dead_beef_0001UL;
        var frame = FrameCodec.EncodeResponse(
            Response.Ok(1, Value.Float(BitConverter.UInt64BitsToDouble(bits))));
        Assert.True(FrameCodec.TryDecodeResponse(frame, out var decoded, out _));
        var value = decoded!.Value!.AsFloat();
        Assert.NotNull(value);
        Assert.Equal(bits, BitConverter.DoubleToUInt64Bits(value.Value));
    }

    [Fact]
    public void Error_response_round_trips_with_prefix_conventions()
    {
        foreach (var message in new[]
                 {
                     "ERR unknown command",
                     "NOAUTH Authentication required.",
                     "WRONGPASS invalid username-password pair or user is disabled.",
                     "[collection_not_found] no such collection: docs",
                 })
        {
            var frame = FrameCodec.EncodeResponse(Response.Err(9, message));
            Assert.True(FrameCodec.TryDecodeResponse(frame, out var decoded, out _));
            Assert.False(decoded!.IsOk);
            Assert.Equal(message, decoded.Error);
        }
    }

    [Fact]
    public void Partial_header_and_partial_body_ask_for_more_bytes()
    {
        var frame = FrameCodec.EncodeRequest(new Request(1, "PING"));
        foreach (var cut in new[] { 0, 1, 3, 4, frame.Length - 1 })
        {
            Assert.False(
                FrameCodec.TryDecodeRequest(frame.AsMemory(0, cut), out _, out _),
                $"cut at {cut} must ask for more bytes");
        }
    }

    [Fact]
    public void Two_frames_in_one_buffer_consume_exactly_one_each()
    {
        var first = FrameCodec.EncodeResponse(Response.Ok(1, Value.Int(1)));
        var second = FrameCodec.EncodeResponse(Response.Ok(2, Value.Int(2)));
        var buffer = first.Concat(second).ToArray();
        Assert.True(FrameCodec.TryDecodeResponse(buffer, out var one, out var used));
        Assert.Equal(1u, one!.Id);
        Assert.Equal(first.Length, used);
        Assert.True(FrameCodec.TryDecodeResponse(buffer.AsMemory(used), out var two, out var used2));
        Assert.Equal(2u, two!.Id);
        Assert.Equal(second.Length, used2);
    }

    [Fact]
    public void Oversized_prefix_rejected_before_body_arrives()
    {
        // Only the 4-byte prefix claiming cap+1: the check fires without the
        // body being present at all — allocation cannot have happened.
        var prefix = BitConverter.GetBytes((uint)(Wire.DefaultMaxFrameBytes + 1));
        var error = Assert.Throws<ThunderFrameTooLargeException>(
            () => FrameCodec.TryDecodeRequest(prefix, out _, out _));
        Assert.Equal(Wire.DefaultMaxFrameBytes + 1L, error.BodyBytes);
        Assert.Equal(Wire.DefaultMaxFrameBytes, error.MaxBytes);
    }

    [Fact]
    public void Custom_limit_is_honored()
    {
        var frame = FrameCodec.EncodeResponse(Response.Ok(1, Value.Str(new string('x', 100))));
        Assert.Throws<ThunderFrameTooLargeException>(
            () => FrameCodec.TryDecodeResponse(frame, 8, out _, out _));
    }

    [Fact]
    public void Garbage_body_is_a_typed_error_not_a_crash()
    {
        // 0xc1 is never valid MessagePack.
        var buffer = BitConverter.GetBytes(4u).Concat(new byte[] { 0xc1, 0xc1, 0xc1, 0xc1 }).ToArray();
        Assert.Throws<ThunderDecodeException>(
            () => FrameCodec.TryDecodeRequest(buffer, out _, out _));
    }

    [Fact]
    public void Push_id_is_reserved_u32_max()
    {
        Assert.Equal(uint.MaxValue, Wire.PushId);
    }

    [Fact]
    public void Id_allocation_skips_push_id_and_wraps()
    {
        // CLT-010: monotonic ids, PUSH_ID skipped, u32 wrap-around.
        var counter = 0u;
        Assert.Equal(1u, ThunderClient.AllocateId(ref counter));
        Assert.Equal(2u, ThunderClient.AllocateId(ref counter));

        counter = uint.MaxValue - 1;
        Assert.Equal(0u, ThunderClient.AllocateId(ref counter)); // PUSH_ID skipped, wrapped
        Assert.Equal(1u, ThunderClient.AllocateId(ref counter));
    }
}

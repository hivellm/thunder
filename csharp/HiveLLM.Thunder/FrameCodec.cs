using System.Buffers.Binary;

namespace HiveLLM.Thunder;

/// <summary>
/// Length-prefixed MessagePack frame codec: one frame is a <c>u32 LE</c>
/// length (body bytes only) followed by the MessagePack body (WIRE-001).
///
/// The cap is validated against the length prefix <b>before</b> the body is
/// touched (WIRE-020/021), so a hostile prefix cannot exhaust memory.
/// Decoders report "need more bytes" on partial input, consume exactly one
/// frame per decode, and support back-to-back frames in one buffer
/// (WIRE-022). This layer is pure — no sockets, no timers, no product
/// knowledge (WIRE-030).
/// </summary>
public static class FrameCodec
{
    /// <summary>Encode one complete request frame (prefix + body).</summary>
    public static byte[] EncodeRequest(Request request)
    {
        ArgumentNullException.ThrowIfNull(request);
        return Frame(WireCodec.EncodeRequestBody(request));
    }

    /// <summary>Encode one complete response frame (prefix + body).</summary>
    public static byte[] EncodeResponse(Response response)
    {
        ArgumentNullException.ThrowIfNull(response);
        return Frame(WireCodec.EncodeResponseBody(response));
    }

    /// <summary>
    /// Decode one request frame with the default 64 MiB cap. Returns false
    /// when the buffer does not yet hold a complete frame (read more and
    /// retry — WIRE-022). On success <paramref name="consumed"/> is
    /// <c>4 + body</c> — the frame size for metrics.
    /// </summary>
    /// <exception cref="ThunderFrameTooLargeException">The prefix declares a body beyond the cap (WIRE-021).</exception>
    /// <exception cref="ThunderDecodeException">The body is malformed MessagePack (WIRE-023).</exception>
    public static bool TryDecodeRequest(
        ReadOnlyMemory<byte> buffer,
        out Request? request,
        out int consumed) =>
        TryDecodeRequest(buffer, Wire.DefaultMaxFrameBytes, out request, out consumed);

    /// <summary>
    /// Decode one request frame, rejecting bodies larger than
    /// <paramref name="maxFrameBytes"/> before any body inspection
    /// (WIRE-020/021).
    /// </summary>
    /// <exception cref="ThunderFrameTooLargeException">The prefix declares a body beyond the cap (WIRE-021).</exception>
    /// <exception cref="ThunderDecodeException">The body is malformed MessagePack (WIRE-023).</exception>
    public static bool TryDecodeRequest(
        ReadOnlyMemory<byte> buffer,
        int maxFrameBytes,
        out Request? request,
        out int consumed)
    {
        request = null;
        if (!TryReadBody(buffer, maxFrameBytes, out var body, out consumed))
        {
            return false;
        }

        request = WireCodec.DecodeRequestBody(body);
        return true;
    }

    /// <summary>
    /// Decode one response frame with the default 64 MiB cap. Returns false
    /// when the buffer does not yet hold a complete frame (WIRE-022).
    /// </summary>
    /// <exception cref="ThunderFrameTooLargeException">The prefix declares a body beyond the cap (WIRE-021).</exception>
    /// <exception cref="ThunderDecodeException">The body is malformed MessagePack (WIRE-023).</exception>
    public static bool TryDecodeResponse(
        ReadOnlyMemory<byte> buffer,
        out Response? response,
        out int consumed) =>
        TryDecodeResponse(buffer, Wire.DefaultMaxFrameBytes, out response, out consumed);

    /// <summary>
    /// Decode one response frame, rejecting bodies larger than
    /// <paramref name="maxFrameBytes"/> before any body inspection
    /// (WIRE-020/021).
    /// </summary>
    /// <exception cref="ThunderFrameTooLargeException">The prefix declares a body beyond the cap (WIRE-021).</exception>
    /// <exception cref="ThunderDecodeException">The body is malformed MessagePack (WIRE-023).</exception>
    public static bool TryDecodeResponse(
        ReadOnlyMemory<byte> buffer,
        int maxFrameBytes,
        out Response? response,
        out int consumed)
    {
        response = null;
        if (!TryReadBody(buffer, maxFrameBytes, out var body, out consumed))
        {
            return false;
        }

        response = WireCodec.DecodeResponseBody(body);
        return true;
    }

    private static byte[] Frame(byte[] body)
    {
        var frame = new byte[4 + body.Length];
        BinaryPrimitives.WriteUInt32LittleEndian(frame, (uint)body.Length);
        body.CopyTo(frame.AsSpan(4));
        return frame;
    }

    /// <summary>
    /// Framing core: prefix parse, cap check (before the body is even
    /// sliced — WIRE-020/021), completeness check (WIRE-022).
    /// </summary>
    private static bool TryReadBody(
        ReadOnlyMemory<byte> buffer,
        int maxFrameBytes,
        out ReadOnlyMemory<byte> body,
        out int consumed)
    {
        body = default;
        consumed = 0;
        if (buffer.Length < 4)
        {
            return false;
        }

        var length = BinaryPrimitives.ReadUInt32LittleEndian(buffer.Span);
        if (maxFrameBytes < 0 || length > (uint)maxFrameBytes)
        {
            throw new ThunderFrameTooLargeException(length, maxFrameBytes);
        }

        var total = 4L + length;
        if (buffer.Length < total)
        {
            return false;
        }

        body = buffer.Slice(4, (int)length);
        consumed = (int)total;
        return true;
    }
}

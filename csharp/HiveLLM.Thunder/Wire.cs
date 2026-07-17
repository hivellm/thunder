namespace HiveLLM.Thunder;

/// <summary>
/// HiveLLM binary RPC constants (wire v1, frozen). One frame is a
/// <c>u32 LE</c> length prefix + MessagePack body; the body is a
/// <see cref="Request"/> or <see cref="Response"/> in the externally-tagged
/// encoding over the 8-variant <see cref="Value"/> model (WIRE-001). The
/// codec uses the low-level <c>MessagePackWriter</c>/<c>Reader</c> API only
/// (<c>MessagePackSerializer.Typeless</c> is forbidden — WIRE-031, NFR-02).
/// </summary>
public static class Wire
{
    /// <summary>Negotiated wire protocol version. v1 is the only version anywhere (WIRE-004).</summary>
    public const int WireVersion = 1;

    /// <summary>
    /// Reserved frame id for server push frames (WIRE-005). Clients never
    /// use it as a request id; demultiplexers route it to the push hook.
    /// </summary>
    public const uint PushId = uint.MaxValue;

    /// <summary>Default frame-body cap: 64 MiB, checked against the length prefix before allocation (WIRE-020).</summary>
    public const int DefaultMaxFrameBytes = 64 * 1024 * 1024;
}

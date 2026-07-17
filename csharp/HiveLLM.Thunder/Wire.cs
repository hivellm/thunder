namespace HiveLLM.Thunder;

/// <summary>
/// HiveLLM binary RPC constants (wire v1, frozen). The wire codec and the
/// multiplexed client land at DAG T3.3 (phase3_csharp-package), corpus-first
/// per SPEC-005, using the low-level MessagePackWriter/Reader API only
/// (Typeless is forbidden, NFR-02).
/// </summary>
public static class Wire
{
    /// <summary>Negotiated wire protocol version. v1 is the only version anywhere.</summary>
    public const int WireVersion = 1;

    /// <summary>Reserved frame id for server push frames (WIRE-005).</summary>
    public const uint PushId = uint.MaxValue;

    /// <summary>Default frame-body cap: 64 MiB, checked before allocation (WIRE-020).</summary>
    public const int DefaultMaxFrameBytes = 64 * 1024 * 1024;
}

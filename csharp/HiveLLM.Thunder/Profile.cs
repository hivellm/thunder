namespace HiveLLM.Thunder;

/// <summary>Handshake style (PRO-001).</summary>
public enum Handshake
{
    /// <summary>No RPC-layer auth (Synap v1 legacy).</summary>
    None,

    /// <summary>
    /// <c>HELLO</c> optional; <c>AUTH [api_key]</c> or <c>[user, pass]</c>;
    /// pre-auth allowlist <c>PING/HELLO/AUTH/QUIT</c> (Nexus).
    /// </summary>
    AuthCommand,

    /// <summary>
    /// <c>HELLO</c> must be the first frame, carrying credentials
    /// (Vectorizer / Lexum).
    /// </summary>
    HelloMandatory,
}

/// <summary>HELLO payload style (PRO-001).</summary>
public enum HelloStyle
{
    /// <summary>No HELLO in the profile (Synap).</summary>
    NotUsed,

    /// <summary>Positional <c>[Int(version)]</c> (Nexus).</summary>
    PositionalVersion,

    /// <summary>
    /// Map with <c>version</c>, <c>token</c> | <c>api_key</c>,
    /// <c>client_name</c>; the reply carries <c>capabilities</c>
    /// (Vectorizer / Lexum).
    /// </summary>
    MapPayload,
}

/// <summary>Server-push policy (PRO-001).</summary>
public enum PushPolicy
{
    /// <summary><see cref="Wire.PushId"/> reserved: servers refuse it from clients and never emit it.</summary>
    Reserved,

    /// <summary>Push frames flow (Synap <c>SUBSCRIBE</c>).</summary>
    Enabled,
}

/// <summary>Which error-string prefix conventions the client parses (PRO-014).</summary>
public enum ErrorConvention
{
    /// <summary>No prefix parsing.</summary>
    None,

    /// <summary><c>ERR</c> / <c>NOAUTH</c> / <c>WRONGPASS</c> / <c>NOPERM</c> prefixes (Nexus, Synap).</summary>
    Resp3Prefixes,

    /// <summary>Leading <c>"[&lt;code&gt;] "</c> machine-readable code (Vectorizer).</summary>
    BracketCode,

    /// <summary>Both conventions composed (Lexum).</summary>
    Both,
}

/// <summary>Transport-security policy (PRO-001).</summary>
public enum TlsPolicy
{
    /// <summary>Plain TCP.</summary>
    Off,

    /// <summary>TLS available behind configuration.</summary>
    Optional,

    /// <summary>Config keys reserved; not wired yet.</summary>
    Reserved,
}

/// <summary>
/// One product's protocol profile (PRO-001) — the declarative description of
/// how one product uses the shared wire. Profiles are data, never behavior:
/// no profile may alter wire bytes (PRO-003). The family registry constants
/// are pinned to <c>conformance/profiles/*.yaml</c> by a test (PRO-010);
/// custom construction stays public (PRO-020) so a new product never waits
/// for a Thunder release.
/// </summary>
public sealed record Profile
{
    /// <summary>Registry name (<c>synap</c>, <c>nexus</c>, …) or a custom identifier.</summary>
    public required string Name { get; init; }

    /// <summary>URL scheme the endpoint parser registers for this profile (PRO-012).</summary>
    public required string Scheme { get; init; }

    /// <summary>Default RPC port for the scheme (PRO-012).</summary>
    public required ushort DefaultPort { get; init; }

    /// <summary>Handshake style (PRO-001).</summary>
    public required Handshake Handshake { get; init; }

    /// <summary>HELLO payload style (PRO-001).</summary>
    public required HelloStyle HelloStyle { get; init; }

    /// <summary>Server-push policy (PRO-001).</summary>
    public required PushPolicy Push { get; init; }

    /// <summary>Frame cap (WIRE-020). Defaults to 64 MiB.</summary>
    public int MaxFrameBytes { get; init; } = Wire.DefaultMaxFrameBytes;

    /// <summary>Per-connection in-flight request bound (CLT-012).</summary>
    public required int MaxInFlight { get; init; }

    /// <summary>Error-string convention the client parses (PRO-014).</summary>
    public required ErrorConvention ErrorCodes { get; init; }

    /// <summary>Transport-security policy (PRO-001).</summary>
    public required TlsPolicy Tls { get; init; }

    /// <summary>
    /// Synap — protocol origin. No RPC-layer auth, push enabled, 512 MiB cap
    /// (matches <c>synap-protocol</c>'s <c>MAX_FRAME_SIZE</c>).
    /// </summary>
    public static Profile Synap { get; } = new()
    {
        Name = "synap",
        Scheme = "synap",
        DefaultPort = 15501,
        Handshake = Handshake.None,
        HelloStyle = HelloStyle.NotUsed,
        Push = PushPolicy.Enabled,
        MaxFrameBytes = 512 * 1024 * 1024,
        MaxInFlight = 256,
        ErrorCodes = ErrorConvention.Resp3Prefixes,
        Tls = TlsPolicy.Off,
    };

    /// <summary>Nexus — canonical spec author. Optional HELLO + AUTH, 64 MiB cap.</summary>
    public static Profile Nexus { get; } = new()
    {
        Name = "nexus",
        Scheme = "nexus",
        DefaultPort = 15475,
        Handshake = Handshake.AuthCommand,
        HelloStyle = HelloStyle.PositionalVersion,
        Push = PushPolicy.Reserved,
        MaxFrameBytes = Wire.DefaultMaxFrameBytes,
        MaxInFlight = 1024,
        ErrorCodes = ErrorConvention.Resp3Prefixes,
        Tls = TlsPolicy.Off,
    };

    /// <summary>Vectorizer — HELLO-mandatory with credentials, <c>[code]</c> prefixes.</summary>
    public static Profile Vectorizer { get; } = new()
    {
        Name = "vectorizer",
        Scheme = "vectorizer",
        DefaultPort = 15503,
        Handshake = Handshake.HelloMandatory,
        HelloStyle = HelloStyle.MapPayload,
        Push = PushPolicy.Reserved,
        MaxFrameBytes = Wire.DefaultMaxFrameBytes,
        MaxInFlight = 256,
        ErrorCodes = ErrorConvention.BracketCode,
        Tls = TlsPolicy.Optional,
    };

    /// <summary>Lexum — Vectorizer-style handshake, both error conventions.</summary>
    public static Profile Lexum { get; } = new()
    {
        Name = "lexum",
        Scheme = "lexum",
        DefaultPort = 17001,
        Handshake = Handshake.HelloMandatory,
        HelloStyle = HelloStyle.MapPayload,
        Push = PushPolicy.Reserved,
        MaxFrameBytes = Wire.DefaultMaxFrameBytes,
        MaxInFlight = 256,
        ErrorCodes = ErrorConvention.Both,
        Tls = TlsPolicy.Reserved,
    };

    /// <summary>Every registered family profile (PRO-010).</summary>
    public static IReadOnlyList<Profile> Registry { get; } =
        new[] { Synap, Nexus, Vectorizer, Lexum };
}

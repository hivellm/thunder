namespace HiveLLM.Thunder;

/// <summary>Handshake style (PRO-001).</summary>
public enum Handshake
{
    /// <summary>No RPC-layer handshake at all: the connection is usable immediately.</summary>
    None,

    /// <summary>
    /// <c>HELLO</c> optional; <c>AUTH [api_key]</c> / <c>[user, pass]</c> /
    /// <c>[password]</c>; pre-auth allowlist <c>PING/HELLO/AUTH/QUIT</c>.
    /// <para>
    /// Whether a deployment <em>enforces</em> credentials is its own config
    /// (<see cref="ClientConfig.Credentials"/> on this side, an
    /// <c>auth_required</c> switch on the server's), not a protocol dialect: a
    /// client with no credentials configured simply sends no <c>AUTH</c>,
    /// which is correct against an open deployment (PRO-001a).
    /// </para>
    /// </summary>
    AuthCommand,

    /// <summary>
    /// <c>HELLO</c> must be the first frame, carrying credentials.
    /// <b>The standard</b> — see <see cref="Config.Standard"/>.
    /// </summary>
    HelloMandatory,
}

/// <summary>HELLO payload style (PRO-001).</summary>
public enum HelloStyle
{
    /// <summary>The application has no <c>HELLO</c> command.</summary>
    NotUsed,

    /// <summary>
    /// <c>HELLO</c> with <b>no arguments</b>; the reply is a metadata Map
    /// <c>{server, version, proto, id, authenticated}</c>. Credentials travel
    /// via <c>AUTH</c>, never inside the HELLO.
    /// </summary>
    ArgLess,

    /// <summary>
    /// Map with <c>version</c>, <c>token</c> | <c>api_key</c>,
    /// <c>client_name</c>; the reply carries <c>proto</c> and
    /// <c>capabilities</c>. <b>The standard</b> — the only style that
    /// negotiates a version and advertises capabilities, which is what an
    /// evolving protocol needs.
    /// </summary>
    MapPayload,
}

/// <summary>Server-push policy (PRO-001).</summary>
public enum PushPolicy
{
    /// <summary>
    /// <see cref="Wire.PushId"/> reserved: servers refuse it from clients and
    /// never emit it. <b>The standard</b> — emitting push is a capability an
    /// application opts into by shipping a push-producing command.
    /// </summary>
    Reserved,

    /// <summary>Push frames flow to the client's push hook.</summary>
    Enabled,
}

/// <summary>Which error-string prefix conventions the client parses (PRO-014).</summary>
public enum ErrorConvention
{
    /// <summary>No prefix parsing.</summary>
    None,

    /// <summary><c>ERR</c> / <c>NOAUTH</c> / <c>WRONGPASS</c> / <c>NOPERM</c> prefixes.</summary>
    Resp3Prefixes,

    /// <summary>Leading <c>"[&lt;code&gt;] "</c> machine-readable code.</summary>
    BracketCode,

    /// <summary>
    /// Both conventions composed. <b>The standard</b> — a strict superset, so
    /// it parses either grammar and needs no negotiation.
    /// </summary>
    Both,
}

/// <summary>Transport-security policy (PRO-001).</summary>
public enum TlsPolicy
{
    /// <summary>
    /// Plain TCP. <b>The standard default</b> — TLS is an additive capability
    /// a deployment turns on, never a dialect.
    /// </summary>
    Off,

    /// <summary>TLS available behind configuration.</summary>
    Optional,

    /// <summary>Config keys reserved; not wired yet.</summary>
    Reserved,
}

/// <summary>
/// One application's protocol configuration (PRO-001) — the declarative
/// description of how <b>one application</b> uses the shared wire. Pure data:
/// the codec never depends on it; <see cref="ThunderClient"/> drives its
/// behavior from it.
///
/// <para>
/// <b>Thunder ships one standard and zero product knowledge.</b> There are no
/// named configurations here — no per-product statics, no registry. Thunder
/// was born from three products' RPC implementations, but a protocol library
/// that must serve implementations which do not exist yet cannot ship a
/// hardcoded list of the ones that did.
/// </para>
///
/// <para>
/// Instead: <see cref="Standard"/> is <b>the</b> family standard, and every
/// dimension is a knob. An application that matches the standard writes its
/// identity and nothing else:
/// </para>
/// <code>
/// var config = Config.Standard() with { Scheme = "myapp", DefaultPort = 9000 };
/// </code>
///
/// <para>
/// An application that still diverges says so <b>in its own repository</b>,
/// where that knowledge belongs:
/// </para>
/// <code>
/// // A deployment whose RPC path authenticates via AUTH and has no HELLO
/// // handler, and which ships a push-producing command.
/// var config = Config.Standard() with
/// {
///     Scheme = "legacy",
///     DefaultPort = 15501,
///     Handshake = Handshake.AuthCommand,
///     HelloStyle = HelloStyle.NotUsed,
///     Push = PushPolicy.Enabled,
/// };
/// </code>
///
/// <para>
/// Convergence is therefore visible and per-application: delete overrides
/// until only <see cref="Scheme"/> and <see cref="DefaultPort"/> remain.
/// Nobody waits on a Thunder release for a row in a registry, and Thunder
/// never carries behavior it does not own.
/// </para>
///
/// <para>
/// Configs are <b>data, never behavior</b>: no config may alter wire bytes
/// (PRO-003) — it selects among behaviors Thunder already implements. Build
/// one with <see cref="Standard"/> plus a <c>with</c> expression, or as a
/// plain <c>new Config { … }</c>; both are supported and neither requires a
/// Thunder release. The record's own <c>with</c> <em>is</em> the builder — it
/// returns a new <see cref="Config"/> per override and composes exactly like
/// the chainable setters the Rust reference needs, so this type ships no
/// hand-written builder to drift from it.
/// </para>
///
/// <para>
/// <b>Not to be confused with <see cref="ClientConfig"/>.</b> The two are
/// deliberately distinct and both are needed to connect: <see cref="Config"/>
/// is the <em>protocol</em> — the dialect an application speaks, shared by
/// every client and server that talks to it. <see cref="ClientConfig"/> is
/// <em>this caller's</em> knobs — credentials, timeouts, client name — which
/// vary per process and never affect the dialect.
/// <see cref="ThunderClient.ConnectAsync"/> takes them in that order.
/// </para>
///
/// <para>
/// The standard's values are pinned to <c>conformance/standard.yaml</c> by a
/// test in every language, so the four implementations can never disagree
/// about what "standard" means — the one guarantee the old per-product
/// registry legitimately provided.
/// </para>
/// </summary>
public sealed record Config
{
    /// <summary>
    /// URL scheme the endpoint parser accepts for this application (PRO-012).
    /// Identity — Thunder has no default for it.
    /// </summary>
    public required string Scheme { get; init; }

    /// <summary>
    /// Default RPC port for the scheme (PRO-012). Identity — Thunder has no
    /// default for it.
    /// </summary>
    public required ushort DefaultPort { get; init; }

    /// <summary>Handshake style (PRO-001).</summary>
    public required Handshake Handshake { get; init; }

    /// <summary>HELLO payload style (PRO-001).</summary>
    public required HelloStyle HelloStyle { get; init; }

    /// <summary>Server-push policy (PRO-001).</summary>
    public required PushPolicy Push { get; init; }

    /// <summary>Frame cap (WIRE-020). Defaults to 64 MiB.</summary>
    public int MaxFrameBytes { get; init; } = Wire.DefaultMaxFrameBytes;

    /// <summary>Per-connection in-flight request bound (CLT-012 / SRV-003).</summary>
    public required int MaxInFlight { get; init; }

    /// <summary>Error-string conventions the client parses (PRO-014).</summary>
    public required ErrorConvention ErrorCodes { get; init; }

    /// <summary>Transport-security policy (PRO-001).</summary>
    public required TlsPolicy Tls { get; init; }

    /// <summary>
    /// <b>The</b> family standard (pinned by <c>conformance/standard.yaml</c>).
    /// <para>
    /// Mandatory <c>HELLO</c> map with <c>proto</c> negotiation and a
    /// capabilities reply; the <c>[CODE]</c> error superset; 64 MiB frames;
    /// 256 in-flight; push reserved; TLS off.
    /// </para>
    /// <para>
    /// <see cref="Scheme"/> is <c>""</c> and <see cref="DefaultPort"/> is
    /// <c>0</c> — identity is the application's to supply, and a
    /// <see cref="Config"/> that never sets them is only usable with an
    /// explicit <c>host:port</c> endpoint.
    /// </para>
    /// </summary>
    public static Config Standard() => new()
    {
        Scheme = "",
        DefaultPort = 0,
        Handshake = Handshake.HelloMandatory,
        HelloStyle = HelloStyle.MapPayload,
        Push = PushPolicy.Reserved,
        MaxFrameBytes = Wire.DefaultMaxFrameBytes,
        MaxInFlight = 256,
        ErrorCodes = ErrorConvention.Both,
        Tls = TlsPolicy.Off,
    };
}

namespace HiveLLM.Thunder;

/// <summary>How a <see cref="Credentials"/> value authenticates (CLT-002).</summary>
public enum CredentialKind
{
    /// <summary>Bearer token (<c>token</c> key under HelloMandatory).</summary>
    Token,

    /// <summary>API key (<c>api_key</c> key under HelloMandatory, single-arg <c>AUTH</c> under AuthCommand).</summary>
    ApiKey,

    /// <summary>User + password (<c>AUTH [user, pass]</c> under AuthCommand).</summary>
    UserPass,
}

/// <summary>
/// Credentials for the configured handshake (CLT-002). Auth state is
/// per-connection and sticky — there are no per-call credentials (CLT-003).
/// </summary>
public sealed class Credentials
{
    private Credentials(CredentialKind kind, string secret, string? user)
    {
        Kind = kind;
        Secret = secret;
        User = user;
    }

    /// <summary>Which credential style this is.</summary>
    public CredentialKind Kind { get; }

    internal string Secret { get; }

    internal string? User { get; }

    /// <summary>Bearer token (<c>token</c> key under HelloMandatory).</summary>
    public static Credentials Token(string token)
    {
        ArgumentNullException.ThrowIfNull(token);
        return new Credentials(CredentialKind.Token, token, null);
    }

    /// <summary>API key (<c>api_key</c> under HelloMandatory, single-arg <c>AUTH</c> under AuthCommand).</summary>
    public static Credentials ApiKey(string apiKey)
    {
        ArgumentNullException.ThrowIfNull(apiKey);
        return new Credentials(CredentialKind.ApiKey, apiKey, null);
    }

    /// <summary>User + password (<c>AUTH [user, pass]</c> under AuthCommand configs).</summary>
    public static Credentials UserPass(string user, string pass)
    {
        ArgumentNullException.ThrowIfNull(user);
        ArgumentNullException.ThrowIfNull(pass);
        return new Credentials(CredentialKind.UserPass, pass, user);
    }
}

/// <summary>
/// Client configuration: connect timeout default <b>10 s</b> (CLT-001),
/// per-call timeout default <b>30 s</b> (CLT-020), optional credentials and
/// client name for the handshake (CLT-002).
/// </summary>
public sealed record ClientConfig
{
    /// <summary>TCP connect timeout (CLT-001). Default 10 s.</summary>
    public TimeSpan ConnectTimeout { get; init; } = TimeSpan.FromSeconds(10);

    /// <summary>
    /// Default per-call timeout (CLT-020); override per call with the
    /// timeout-taking <see cref="ThunderClient.CallAsync(string, IReadOnlyList{Value}, TimeSpan, CancellationToken)"/>
    /// overload. Default 30 s.
    /// </summary>
    public TimeSpan CallTimeout { get; init; } = TimeSpan.FromSeconds(30);

    /// <summary>Handshake credentials, when the protocol config wants them.</summary>
    public Credentials? Credentials { get; init; }

    /// <summary>Client identifier sent in the <c>HELLO</c> map (HelloMandatory). Default <c>thunder-client</c>.</summary>
    public string? ClientName { get; init; }
}

/// <summary>What the handshake learned about this connection (CLT-002).</summary>
/// <param name="Authenticated">
/// True once the server accepted the credentials (<c>AUTH</c> succeeded or
/// the <c>HELLO</c> reply said so).
/// </param>
/// <param name="Capabilities">Capability names from the <c>HELLO</c> reply (HelloMandatory).</param>
public sealed record HandshakeInfo(bool Authenticated, IReadOnlyList<string> Capabilities)
{
    /// <summary>Unauthenticated, no capabilities.</summary>
    public static HandshakeInfo Default { get; } = new(false, System.Array.Empty<string>());
}

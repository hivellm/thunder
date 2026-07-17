namespace HiveLLM.Thunder;

/// <summary>
/// The stable error classes of the client contract (CLT-050/052). Product
/// SDKs and user code branch on the class and <see cref="ThunderException.Code"/>,
/// never on message text.
/// </summary>
public enum ThunderErrorClass
{
    /// <summary>
    /// Authentication / authorization failure — handshake rejections
    /// (CLT-003) and NOAUTH/WRONGPASS/NOPERM-prefixed replies (CLT-051).
    /// </summary>
    Auth,

    /// <summary>The server answered the call with the Err arm.</summary>
    Server,

    /// <summary>
    /// Transport-level failure: dial, write, or the connection dying while
    /// the call was pending (CLT-004/030/031). Also raised for invalid
    /// endpoints (CLT-070).
    /// </summary>
    Connection,

    /// <summary>The per-call (or connect) timeout elapsed (CLT-020).</summary>
    Timeout,

    /// <summary>
    /// A frame larger than the cap, rejected on the length prefix before any
    /// body allocation (WIRE-020/021).
    /// </summary>
    FrameTooLarge,

    /// <summary>
    /// A malformed frame (WIRE-023), or a push frame under a Reserved
    /// config (CLT-060).
    /// </summary>
    Decode,
}

/// <summary>
/// Base type of every typed Thunder error (CLT-050..052). Server error
/// strings are parsed per the config's <see cref="ErrorConvention"/> via
/// <see cref="FromServerMessage"/>; <see cref="Exception.Message"/> carries
/// the raw server message verbatim for the Auth and Server classes.
/// </summary>
public class ThunderException : Exception
{
    /// <summary>Create a typed error.</summary>
    protected ThunderException(
        ThunderErrorClass errorClass,
        string message,
        string? code = null,
        Exception? innerException = null)
        : base(message, innerException)
    {
        ErrorClass = errorClass;
        Code = code;
    }

    /// <summary>The stable error class (CLT-052).</summary>
    public ThunderErrorClass ErrorClass { get; }

    /// <summary>
    /// Machine-readable code extracted from a leading <c>"[code] "</c>
    /// prefix under the BracketCode / Both conventions (PRO-014).
    /// </summary>
    public string? Code { get; }

    private static readonly string[] AuthPrefixes = { "NOAUTH", "WRONGPASS", "NOPERM" };

    /// <summary>
    /// Parse a server error string per the config's convention (CLT-050,
    /// PRO-014).
    /// <list type="bullet">
    /// <item><see cref="ErrorConvention.Resp3Prefixes"/>: NOAUTH/WRONGPASS/NOPERM
    /// map to <see cref="ThunderAuthException"/>; everything else (ERR
    /// included) to <see cref="ThunderServerException"/>.</item>
    /// <item><see cref="ErrorConvention.BracketCode"/>: a leading
    /// <c>"[code] "</c> is extracted into <see cref="Code"/>; the auth
    /// prefixes still map to the auth class regardless of convention
    /// (CLT-051).</item>
    /// <item><see cref="ErrorConvention.Both"/>: composes the two — bracket
    /// code first, then prefixes.</item>
    /// <item><see cref="ErrorConvention.None"/>: no parsing.</item>
    /// </list>
    /// The exception message always carries the raw string, verbatim.
    /// </summary>
    public static ThunderException FromServerMessage(string message, ErrorConvention convention)
    {
        ArgumentNullException.ThrowIfNull(message);
        switch (convention)
        {
            case ErrorConvention.Resp3Prefixes:
                return StartsWithAuthPrefix(message)
                    ? new ThunderAuthException(message)
                    : new ThunderServerException(message);
            case ErrorConvention.BracketCode:
            case ErrorConvention.Both:
                var (code, rest) = SplitBracketCode(message);
                return StartsWithAuthPrefix(rest)
                    ? new ThunderAuthException(message)
                    : new ThunderServerException(message, code);
            default:
                return new ThunderServerException(message);
        }
    }

    /// <summary>
    /// True when the message starts with one of the auth prefixes both
    /// family conventions use for authentication failures (CLT-051). The
    /// prefix must be word-aligned: the whole message or followed by a space.
    /// </summary>
    private static bool StartsWithAuthPrefix(string message)
    {
        foreach (var prefix in AuthPrefixes)
        {
            if (message.StartsWith(prefix, StringComparison.Ordinal) &&
                (message.Length == prefix.Length || message[prefix.Length] == ' '))
            {
                return true;
            }
        }

        return false;
    }

    /// <summary>
    /// Split a leading <c>"[code] "</c> prefix. The code must be non-empty
    /// and whitespace-free (machine-readable); anything else leaves the
    /// message untouched.
    /// </summary>
    private static (string? Code, string Remainder) SplitBracketCode(string message)
    {
        if (message.StartsWith('['))
        {
            var end = message.IndexOf(']', 1);
            if (end > 0)
            {
                var code = message[1..end];
                var after = message[(end + 1)..];
                if (code.Length > 0 && !code.Any(char.IsWhiteSpace) && after.StartsWith(' '))
                {
                    return (code, after[1..]);
                }
            }
        }

        return (null, message);
    }
}

/// <summary>Authentication / authorization failure (CLT-003/051). The message is the raw server string.</summary>
public sealed class ThunderAuthException : ThunderException
{
    /// <summary>Create an auth error carrying the raw server message.</summary>
    public ThunderAuthException(string message)
        : base(ThunderErrorClass.Auth, message)
    {
    }
}

/// <summary>The server answered with the Err arm (CLT-050). The message is the raw server string, any <c>[code]</c> prefix included.</summary>
public sealed class ThunderServerException : ThunderException
{
    /// <summary>Create a server error carrying the raw message and optional bracket code.</summary>
    public ThunderServerException(string message, string? code = null)
        : base(ThunderErrorClass.Server, message, code)
    {
    }
}

/// <summary>Transport-level failure (CLT-004/030/031) or invalid endpoint (CLT-070).</summary>
public sealed class ThunderConnectionException : ThunderException
{
    /// <summary>Create a connection error.</summary>
    public ThunderConnectionException(string message, Exception? innerException = null)
        : base(ThunderErrorClass.Connection, message, null, innerException)
    {
    }
}

/// <summary>The per-call or connect timeout elapsed (CLT-001/020).</summary>
public sealed class ThunderTimeoutException : ThunderException
{
    /// <summary>Create a timeout error.</summary>
    public ThunderTimeoutException()
        : base(ThunderErrorClass.Timeout, "timed out")
    {
    }
}

/// <summary>
/// A frame body larger than the cap, rejected on the length prefix before
/// any body allocation (WIRE-020/021).
/// </summary>
public sealed class ThunderFrameTooLargeException : ThunderException
{
    /// <summary>Create a frame-too-large error from the declared body size and the cap.</summary>
    public ThunderFrameTooLargeException(long bodyBytes, long maxBytes)
        : base(ThunderErrorClass.FrameTooLarge, $"frame body {bodyBytes} bytes exceeds limit {maxBytes} bytes")
    {
        BodyBytes = bodyBytes;
        MaxBytes = maxBytes;
    }

    /// <summary>The body size the length prefix declared.</summary>
    public long BodyBytes { get; }

    /// <summary>The cap in force (WIRE-020).</summary>
    public long MaxBytes { get; }
}

/// <summary>A malformed frame body (WIRE-023), or a push frame under a push-reserved config (CLT-060).</summary>
public sealed class ThunderDecodeException : ThunderException
{
    /// <summary>Create a decode error.</summary>
    public ThunderDecodeException(string message, Exception? innerException = null)
        : base(ThunderErrorClass.Decode, message, null, innerException)
    {
    }
}

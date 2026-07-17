using System.Globalization;

namespace HiveLLM.Thunder;

/// <summary>
/// A resolved RPC endpoint: host plus concrete port (CLT-070/071).
///
/// <see cref="Parse"/> accepts <c>scheme://host[:port]</c> for every scheme
/// in the profile registry — scheme → default-port resolution is data-driven
/// (PRO-012), products never fork the parser — plus bare <c>host:port</c>
/// (RPC implied; the caller supplies the profile). <c>http(s)://</c> URLs
/// are rejected with a pointer to the product's HTTP client: Thunder is
/// RPC-only. Parse failures use the connection error class — an endpoint
/// that cannot be parsed is an endpoint that cannot be dialed.
/// </summary>
/// <param name="Host">Host name or IP literal (IPv6 without brackets).</param>
/// <param name="Port">Concrete port — explicit, or the scheme's registry default.</param>
public sealed record Endpoint(string Host, ushort Port)
{
    /// <summary>
    /// Parse an endpoint string (CLT-070). Accepted forms:
    /// <list type="bullet">
    /// <item><c>scheme://host[:port]</c> for every registered profile scheme;
    /// a missing port resolves to the scheme's registry default (CLT-071).</item>
    /// <item>bare <c>host:port</c> (RPC implied — the caller supplies the profile).</item>
    /// <item><c>[v6::addr]:port</c> / <c>scheme://[v6::addr][:port]</c> for IPv6 literals.</item>
    /// </list>
    /// </summary>
    /// <exception cref="ThunderConnectionException">The endpoint cannot be parsed (CLT-070).</exception>
    public static Endpoint Parse(string input)
    {
        ArgumentNullException.ThrowIfNull(input);
        var trimmed = input.Trim();
        var schemeSplit = trimmed.IndexOf("://", StringComparison.Ordinal);
        if (schemeSplit >= 0)
        {
            var scheme = trimmed[..schemeSplit].ToLowerInvariant();
            var rest = trimmed[(schemeSplit + 3)..];
            if (scheme is "http" or "https")
            {
                throw Invalid(
                    $"'{trimmed}' is an HTTP URL and Thunder is RPC-only — use the product's " +
                    "HTTP client for REST endpoints, or pass an RPC endpoint such as " +
                    "'vectorizer://host:port' or bare 'host:port'");
            }

            var profile = Profile.Registry.FirstOrDefault(p => p.Scheme == scheme)
                ?? throw Invalid(
                    $"unknown endpoint scheme '{scheme}' — registered schemes: " +
                    $"{string.Join(", ", Profile.Registry.Select(p => p.Scheme))}; " +
                    "or use bare 'host:port'");
            if (rest.EndsWith('/'))
            {
                rest = rest[..^1];
            }

            if (rest.Contains('/'))
            {
                throw Invalid(
                    $"endpoint '{trimmed}' must not carry a path — expected {scheme}://host[:port]");
            }

            var (host, port) = SplitHostPort(rest);
            return new Endpoint(host, port ?? profile.DefaultPort);
        }

        var (bareHost, barePort) = SplitHostPort(trimmed);
        return barePort is null
            ? throw Invalid(
                $"bare endpoint '{trimmed}' needs an explicit port ('host:port') — only " +
                "scheme-prefixed endpoints resolve a registry default port")
            : new Endpoint(bareHost, barePort.Value);
    }

    /// <summary>Split <c>host[:port]</c>, handling bracketed IPv6 literals.</summary>
    private static (string Host, ushort? Port) SplitHostPort(string s)
    {
        if (s.Length == 0)
        {
            throw Invalid("endpoint host is empty");
        }

        if (s.StartsWith('['))
        {
            var close = s.IndexOf(']', StringComparison.Ordinal);
            if (close < 0)
            {
                throw Invalid($"unterminated '[' in endpoint host '{s}'");
            }

            var host = s[1..close];
            if (host.Length == 0)
            {
                throw Invalid("endpoint host is empty");
            }

            var tail = s[(close + 1)..];
            if (tail.Length == 0)
            {
                return (host, null);
            }

            return tail.StartsWith(':')
                ? (host, ParsePort(tail[1..], s))
                : throw Invalid($"expected ':port' after ']' in endpoint '{s}'");
        }

        var lastColon = s.LastIndexOf(':');
        if (lastColon < 0)
        {
            return (s, null);
        }

        // More than one ':' without brackets: an IPv6 literal, no port.
        if (s.IndexOf(':', StringComparison.Ordinal) != lastColon)
        {
            return (s, null);
        }

        var head = s[..lastColon];
        return head.Length == 0
            ? throw Invalid("endpoint host is empty")
            : (head, ParsePort(s[(lastColon + 1)..], s));
    }

    private static ushort ParsePort(string port, string whole) =>
        ushort.TryParse(port, NumberStyles.None, CultureInfo.InvariantCulture, out var parsed)
            ? parsed
            : throw Invalid($"invalid port '{port}' in endpoint '{whole}'");

    private static ThunderConnectionException Invalid(string message) => new(message);
}

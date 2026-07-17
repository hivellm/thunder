//! Endpoint parsing (CLT-070/071).
//!
//! Accepts `scheme://host[:port]` for every scheme in the profile
//! registry — scheme → default-port resolution is data-driven (PRO-012),
//! products never fork the parser — plus bare `host:port` (RPC implied;
//! the caller supplies the profile). `http(s)://` URLs are rejected with
//! a pointer to the product's HTTP client: Thunder is RPC-only.
//!
//! Parse failures use the [`ClientError::Connection`] class — an endpoint
//! that cannot be parsed is an endpoint that cannot be dialed.

use crate::wire::Profile;

use crate::client::error::ClientError;

/// A resolved RPC endpoint: host plus concrete port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    /// Host name or IP literal (IPv6 without brackets).
    pub host: String,
    /// Concrete port — explicit, or the scheme's registry default.
    pub port: u16,
}

/// Parse an endpoint string (CLT-070).
///
/// Accepted forms:
/// - `scheme://host[:port]` for every registered profile scheme
///   (`synap`, `nexus`, `vectorizer`, `lexum`); a missing port resolves
///   to the scheme's registry default (CLT-071).
/// - bare `host:port` (RPC implied — the caller supplies the profile).
/// - `[v6::addr]:port` / `scheme://[v6::addr][:port]` for IPv6 literals.
///
/// `http://` / `https://` are rejected: Thunder is RPC-only; REST
/// endpoints belong to the product's HTTP client.
pub fn parse_endpoint(input: &str) -> Result<Endpoint, ClientError> {
    let input = input.trim();
    if let Some((scheme, rest)) = input.split_once("://") {
        let scheme = scheme.to_ascii_lowercase();
        if scheme == "http" || scheme == "https" {
            return Err(invalid(format!(
                "'{input}' is an HTTP URL and Thunder is RPC-only — use the product's HTTP \
                 client for REST endpoints, or pass an RPC endpoint such as \
                 'vectorizer://host:port' or bare 'host:port'"
            )));
        }
        let profile = Profile::registry()
            .into_iter()
            .find(|p| p.scheme == scheme)
            .ok_or_else(|| {
                let known = Profile::registry().map(|p| p.scheme).join(", ");
                invalid(format!(
                    "unknown endpoint scheme '{scheme}' — registered schemes: {known}; \
                     or use bare 'host:port'"
                ))
            })?;
        let rest = rest.strip_suffix('/').unwrap_or(rest);
        if rest.contains('/') {
            return Err(invalid(format!(
                "endpoint '{input}' must not carry a path — expected {scheme}://host[:port]"
            )));
        }
        let (host, port) = split_host_port(rest)?;
        Ok(Endpoint {
            host,
            port: port.unwrap_or(profile.default_port),
        })
    } else {
        let (host, port) = split_host_port(input)?;
        let port = port.ok_or_else(|| {
            invalid(format!(
                "bare endpoint '{input}' needs an explicit port ('host:port') — only \
                 scheme-prefixed endpoints resolve a registry default port"
            ))
        })?;
        Ok(Endpoint { host, port })
    }
}

/// Split `host[:port]`, handling bracketed IPv6 literals.
fn split_host_port(s: &str) -> Result<(String, Option<u16>), ClientError> {
    if s.is_empty() {
        return Err(invalid("endpoint host is empty".to_owned()));
    }
    if let Some(inner) = s.strip_prefix('[') {
        let (host, tail) = inner
            .split_once(']')
            .ok_or_else(|| invalid(format!("unterminated '[' in endpoint host '{s}'")))?;
        if host.is_empty() {
            return Err(invalid("endpoint host is empty".to_owned()));
        }
        return match tail {
            "" => Ok((host.to_owned(), None)),
            t => {
                let port = t.strip_prefix(':').ok_or_else(|| {
                    invalid(format!("expected ':port' after ']' in endpoint '{s}'"))
                })?;
                Ok((host.to_owned(), Some(parse_port(port, s)?)))
            }
        };
    }
    match s.rsplit_once(':') {
        // More than one ':' without brackets: an IPv6 literal, no port.
        Some((head, _)) if head.contains(':') => Ok((s.to_owned(), None)),
        Some((host, port)) => {
            if host.is_empty() {
                return Err(invalid("endpoint host is empty".to_owned()));
            }
            Ok((host.to_owned(), Some(parse_port(port, s)?)))
        }
        None => Ok((s.to_owned(), None)),
    }
}

fn parse_port(port: &str, whole: &str) -> Result<u16, ClientError> {
    port.parse::<u16>()
        .map_err(|_| invalid(format!("invalid port '{port}' in endpoint '{whole}'")))
}

fn invalid(message: String) -> ClientError {
    ClientError::Connection { message }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn every_registered_scheme_resolves_its_default_port() {
        // CLT-071: scheme → default port comes from the registry.
        for profile in Profile::registry() {
            let ep = parse_endpoint(&format!("{}://db.example.com", profile.scheme)).unwrap();
            assert_eq!(ep.host, "db.example.com");
            assert_eq!(ep.port, profile.default_port, "{}", profile.scheme);
        }
    }

    #[test]
    fn explicit_port_wins_over_default() {
        let ep = parse_endpoint("nexus://10.0.0.7:9999").unwrap();
        assert_eq!(
            ep,
            Endpoint {
                host: "10.0.0.7".to_owned(),
                port: 9999
            }
        );
    }

    #[test]
    fn bare_host_port_is_accepted_rpc_implied() {
        let ep = parse_endpoint("localhost:15501").unwrap();
        assert_eq!(
            ep,
            Endpoint {
                host: "localhost".to_owned(),
                port: 15501
            }
        );
    }

    #[test]
    fn bare_host_without_port_is_rejected() {
        let err = parse_endpoint("localhost").unwrap_err();
        assert!(matches!(err, ClientError::Connection { .. }));
    }

    #[test]
    fn http_and_https_are_rejected_with_pointer_to_http_client() {
        for url in ["http://vec.example.com:8080", "https://vec.example.com"] {
            let err = parse_endpoint(url).unwrap_err();
            let ClientError::Connection { message } = err else {
                panic!("expected the connection class, got {err:?}");
            };
            assert!(
                message.contains("RPC-only") && message.contains("HTTP client"),
                "rejection must point at the product HTTP client: {message}"
            );
        }
    }

    #[test]
    fn unknown_scheme_is_rejected_listing_the_registry() {
        let err = parse_endpoint("redis://h:1").unwrap_err();
        let ClientError::Connection { message } = err else {
            panic!("expected the connection class");
        };
        for scheme in ["synap", "nexus", "vectorizer", "lexum"] {
            assert!(message.contains(scheme), "must list '{scheme}': {message}");
        }
    }

    #[test]
    fn ipv6_literals_parse_with_and_without_brackets() {
        let ep = parse_endpoint("[::1]:8080").unwrap();
        assert_eq!(
            ep,
            Endpoint {
                host: "::1".to_owned(),
                port: 8080
            }
        );
        let ep = parse_endpoint("synap://[fe80::1]").unwrap();
        assert_eq!(ep.host, "fe80::1");
        assert_eq!(ep.port, Profile::synap().default_port);
    }

    #[test]
    fn trailing_slash_is_tolerated_but_paths_are_not() {
        let ep = parse_endpoint("lexum://h/").unwrap();
        assert_eq!(ep.port, Profile::lexum().default_port);
        assert!(parse_endpoint("lexum://h/db").is_err());
    }

    #[test]
    fn invalid_ports_are_rejected() {
        assert!(parse_endpoint("host:99999").is_err());
        assert!(parse_endpoint("synap://host:abc").is_err());
        assert!(parse_endpoint(":1234").is_err());
    }
}

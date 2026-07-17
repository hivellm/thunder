//! Endpoint parsing (CLT-070/071).
//!
//! Accepts `scheme://host[:port]` where the scheme is **the application's
//! own**, taken from its [`Config`] — Thunder has no registry of schemes to
//! consult and no product's parser to fork (PRO-012) — plus bare
//! `host:port` (RPC implied). `http(s)://` URLs are rejected with a pointer
//! to the application's HTTP client: Thunder is RPC-only.
//!
//! Parse failures use the [`ClientError::Connection`] class — an endpoint
//! that cannot be parsed is an endpoint that cannot be dialed.

use crate::wire::Config;

use crate::client::error::ClientError;

/// A resolved RPC endpoint: host plus concrete port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    /// Host name or IP literal (IPv6 without brackets).
    pub host: String,
    /// Concrete port — explicit, or the config's `default_port`.
    pub port: u16,
}

/// Parse an endpoint string against the application's [`Config`] (CLT-070).
///
/// Accepted forms:
/// - `scheme://host[:port]` where `scheme` is `config.scheme`; a missing
///   port resolves to `config.default_port` (CLT-071).
/// - bare `host:port` (RPC implied).
/// - `[v6::addr]:port` / `scheme://[v6::addr][:port]` for IPv6 literals.
///
/// `http://` / `https://` are rejected: Thunder is RPC-only; REST
/// endpoints belong to the application's HTTP client.
pub fn parse_endpoint(input: &str, config: &Config) -> Result<Endpoint, ClientError> {
    let input = input.trim();
    if let Some((scheme, rest)) = input.split_once("://") {
        let scheme = scheme.to_ascii_lowercase();
        if scheme == "http" || scheme == "https" {
            return Err(invalid(format!(
                "'{input}' is an HTTP URL and Thunder is RPC-only — use the application's HTTP \
                 client for REST endpoints, or pass an RPC endpoint such as \
                 'scheme://host:port' or bare 'host:port'"
            )));
        }
        if scheme != config.scheme {
            let configured = config.scheme;
            return Err(invalid(format!(
                "endpoint scheme '{scheme}' does not match this client's configured scheme \
                 '{configured}' — set the scheme on the Config, or use bare 'host:port'"
            )));
        }
        let rest = rest.strip_suffix('/').unwrap_or(rest);
        if rest.contains('/') {
            return Err(invalid(format!(
                "endpoint '{input}' must not carry a path — expected {scheme}://host[:port]"
            )));
        }
        let (host, port) = split_host_port(rest)?;
        Ok(Endpoint {
            host,
            port: port.unwrap_or(config.default_port),
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

    /// An application's config — Thunder ships no schemes of its own, so
    /// the tests bring their own, exactly as an application does.
    fn app() -> Config {
        Config::standard().scheme("myapp").port(9000)
    }

    #[test]
    fn the_configured_scheme_resolves_the_configured_default_port() {
        // CLT-071: scheme → default port comes from the application's own
        // config, not from any registry Thunder carries.
        let ep = parse_endpoint("myapp://db.example.com", &app()).unwrap();
        assert_eq!(ep.host, "db.example.com");
        assert_eq!(ep.port, 9000);
    }

    #[test]
    fn any_application_can_pick_any_scheme_without_a_thunder_release() {
        // The whole point of dropping the registry: a scheme Thunder has
        // never heard of works because the application configured it.
        let future = Config::standard()
            .scheme("something-new-in-2030")
            .port(4242);
        let ep = parse_endpoint("something-new-in-2030://host", &future).unwrap();
        assert_eq!(ep.port, 4242);
    }

    #[test]
    fn explicit_port_wins_over_default() {
        let ep = parse_endpoint("myapp://10.0.0.7:9999", &app()).unwrap();
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
        let ep = parse_endpoint("localhost:15501", &app()).unwrap();
        assert_eq!(
            ep,
            Endpoint {
                host: "localhost".to_owned(),
                port: 15501
            }
        );
    }

    #[test]
    fn bare_host_port_works_even_with_no_scheme_configured() {
        // Config::standard() has no identity until an application gives it
        // one; an explicit host:port needs none.
        let ep = parse_endpoint("localhost:15501", &Config::standard()).unwrap();
        assert_eq!(ep.port, 15501);
    }

    #[test]
    fn bare_host_without_port_is_rejected() {
        let err = parse_endpoint("localhost", &app()).unwrap_err();
        assert!(matches!(err, ClientError::Connection { .. }));
    }

    #[test]
    fn http_and_https_are_rejected_with_pointer_to_http_client() {
        for url in ["http://vec.example.com:8080", "https://vec.example.com"] {
            let err = parse_endpoint(url, &app()).unwrap_err();
            let ClientError::Connection { message } = err else {
                panic!("expected the connection class, got {err:?}");
            };
            assert!(
                message.contains("RPC-only") && message.contains("HTTP client"),
                "rejection must point at the application's HTTP client: {message}"
            );
        }
    }

    #[test]
    fn a_scheme_other_than_the_configured_one_is_rejected() {
        let err = parse_endpoint("redis://h:1", &app()).unwrap_err();
        let ClientError::Connection { message } = err else {
            panic!("expected the connection class");
        };
        assert!(
            message.contains("redis") && message.contains("myapp"),
            "the mismatch must name both the given and the configured scheme: {message}"
        );
    }

    #[test]
    fn ipv6_literals_parse_with_and_without_brackets() {
        let ep = parse_endpoint("[::1]:8080", &app()).unwrap();
        assert_eq!(
            ep,
            Endpoint {
                host: "::1".to_owned(),
                port: 8080
            }
        );
        let ep = parse_endpoint("myapp://[fe80::1]", &app()).unwrap();
        assert_eq!(ep.host, "fe80::1");
        assert_eq!(ep.port, 9000);
    }

    #[test]
    fn trailing_slash_is_tolerated_but_paths_are_not() {
        let ep = parse_endpoint("myapp://h/", &app()).unwrap();
        assert_eq!(ep.port, 9000);
        assert!(parse_endpoint("myapp://h/db", &app()).is_err());
    }

    #[test]
    fn invalid_ports_are_rejected() {
        assert!(parse_endpoint("host:99999", &app()).is_err());
        assert!(parse_endpoint("host:abc", &app()).is_err());
    }

    #[test]
    fn empty_host_is_rejected() {
        assert!(parse_endpoint("myapp://:1234", &app()).is_err());
        assert!(parse_endpoint(":1234", &app()).is_err());
    }
}

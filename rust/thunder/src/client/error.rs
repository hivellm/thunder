//! Typed client errors (CLT-050..052).
//!
//! `Result::Err(string)` replies are parsed per the profile's
//! `error_codes` convention (PRO-014) into a [`ClientError`] carrying the
//! raw message, an optional machine-readable `code` (from a leading
//! `"[code] "` prefix), and a stable error **class**. Product SDKs and
//! user code branch on the class and `code`, never on message text
//! (CLT-052).

use crate::wire::config::ErrorConvention;

/// The stable error classes of the client contract (CLT-050).
///
/// Variants are the public API — matching on them is supported forever.
/// `Clone` is required so one connection failure can fan out to every
/// pending call (CLT-014).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ClientError {
    /// Authentication / authorization failure — handshake rejections
    /// (CLT-003) and `NOAUTH`/`WRONGPASS`/`NOPERM`-prefixed replies
    /// (CLT-051).
    #[error("auth error: {message}")]
    Auth {
        /// Raw server message, verbatim.
        message: String,
    },
    /// The server answered the call with `Result::Err`.
    #[error("server error: {message}")]
    Server {
        /// Raw server message, verbatim (any `[code]` prefix included).
        message: String,
        /// Machine-readable code extracted from a leading `"[code] "`
        /// prefix under `BracketCode` / `Both` conventions (PRO-014).
        code: Option<String>,
    },
    /// Transport-level failure: dial, write, or the connection dying
    /// while the call was pending (CLT-004/030/031). Also raised for
    /// invalid endpoints (CLT-070).
    #[error("connection error: {message}")]
    Connection {
        /// Human-readable cause.
        message: String,
    },
    /// The per-call (or connect) timeout elapsed (CLT-020). The pending
    /// entry was removed; a late response is dropped per CLT-013.
    #[error("timed out")]
    Timeout,
    /// The server sent a frame larger than the profile cap; the
    /// connection was poisoned (WIRE-020 via CLT-014).
    #[error("frame too large: {message}")]
    FrameTooLarge {
        /// Human-readable cause.
        message: String,
    },
    /// The server sent a malformed frame (or a push frame under a
    /// `Reserved` profile, CLT-060); the connection was poisoned
    /// (CLT-014).
    #[error("decode error: {message}")]
    Decode {
        /// Human-readable cause.
        message: String,
    },
}

impl ClientError {
    /// Parse a server error string per the profile's convention
    /// (CLT-050, PRO-014).
    ///
    /// - `Resp3Prefixes`: `NOAUTH`/`WRONGPASS`/`NOPERM` → [`Self::Auth`];
    ///   everything else (`ERR …` included) → [`Self::Server`].
    /// - `BracketCode`: a leading `"[code] "` is extracted into `code`;
    ///   the auth prefixes still map to [`Self::Auth`] regardless of
    ///   convention (CLT-051).
    /// - `Both`: composes the two — bracket code first, then prefixes.
    /// - `None`: no parsing; the raw message becomes [`Self::Server`].
    ///
    /// `message` always carries the raw string, verbatim.
    pub fn from_server_message(message: impl Into<String>, convention: ErrorConvention) -> Self {
        let message = message.into();
        match convention {
            ErrorConvention::None => Self::Server {
                message,
                code: None,
            },
            ErrorConvention::Resp3Prefixes => {
                if starts_with_auth_prefix(&message) {
                    Self::Auth { message }
                } else {
                    Self::Server {
                        message,
                        code: None,
                    }
                }
            }
            ErrorConvention::BracketCode | ErrorConvention::Both => {
                let (code, rest) = split_bracket_code(&message);
                if starts_with_auth_prefix(rest) {
                    Self::Auth { message }
                } else {
                    Self::Server { message, code }
                }
            }
        }
    }
}

/// True when the message starts with one of the auth prefixes both
/// family conventions use for authentication failures (CLT-051).
fn starts_with_auth_prefix(message: &str) -> bool {
    ["NOAUTH", "WRONGPASS", "NOPERM"].iter().any(|prefix| {
        message
            .strip_prefix(prefix)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with(' '))
    })
}

/// Split a leading `"[code] "` prefix. The code must be non-empty and
/// whitespace-free (machine-readable, Vectorizer-style); anything else
/// leaves the message untouched.
fn split_bracket_code(message: &str) -> (Option<String>, &str) {
    if let Some(inner) = message.strip_prefix('[') {
        if let Some(end) = inner.find(']') {
            let code = &inner[..end];
            let after = &inner[end + 1..];
            if !code.is_empty() && !code.contains(char::is_whitespace) {
                if let Some(rest) = after.strip_prefix(' ') {
                    return (Some(code.to_owned()), rest);
                }
            }
        }
    }
    (None, message)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn resp3_auth_prefixes_map_to_auth_class() {
        for msg in [
            "NOAUTH Authentication required.",
            "WRONGPASS invalid username-password pair or user is disabled.",
            "NOPERM this user has no permissions",
            "NOAUTH",
        ] {
            let err = ClientError::from_server_message(msg, ErrorConvention::Resp3Prefixes);
            assert_eq!(
                err,
                ClientError::Auth {
                    message: msg.to_owned()
                },
                "{msg} must map to the auth class (CLT-051)"
            );
        }
    }

    #[test]
    fn resp3_err_prefix_is_generic_server_error_without_code() {
        let err =
            ClientError::from_server_message("ERR unknown command", ErrorConvention::Resp3Prefixes);
        assert_eq!(
            err,
            ClientError::Server {
                message: "ERR unknown command".to_owned(),
                code: None,
            }
        );
    }

    #[test]
    fn resp3_prefix_must_be_word_aligned() {
        // "NOAUTHx" is not the NOAUTH prefix.
        let err = ClientError::from_server_message("NOAUTHx nope", ErrorConvention::Resp3Prefixes);
        assert!(matches!(err, ClientError::Server { .. }));
    }

    #[test]
    fn bracket_code_extracts_structured_code_and_keeps_raw_message() {
        let raw = "[collection_not_found] no such collection: docs";
        let err = ClientError::from_server_message(raw, ErrorConvention::BracketCode);
        assert_eq!(
            err,
            ClientError::Server {
                message: raw.to_owned(),
                code: Some("collection_not_found".to_owned()),
            }
        );
    }

    #[test]
    fn bracket_code_still_maps_auth_prefixes_to_auth_class() {
        // CLT-051: auth prefixes win regardless of convention.
        let raw = "[unauthorized] NOAUTH token expired";
        let err = ClientError::from_server_message(raw, ErrorConvention::BracketCode);
        assert_eq!(
            err,
            ClientError::Auth {
                message: raw.to_owned()
            }
        );
    }

    #[test]
    fn both_convention_composes_bracket_and_prefixes() {
        let err = ClientError::from_server_message(
            "[wrongpass] WRONGPASS bad credentials",
            ErrorConvention::Both,
        );
        assert!(matches!(err, ClientError::Auth { .. }));

        let err = ClientError::from_server_message(
            "[index_missing] ERR no such index",
            ErrorConvention::Both,
        );
        assert_eq!(
            err,
            ClientError::Server {
                message: "[index_missing] ERR no such index".to_owned(),
                code: Some("index_missing".to_owned()),
            }
        );
    }

    #[test]
    fn none_convention_never_parses() {
        let err = ClientError::from_server_message("NOAUTH raw passthrough", ErrorConvention::None);
        assert_eq!(
            err,
            ClientError::Server {
                message: "NOAUTH raw passthrough".to_owned(),
                code: None,
            }
        );
    }

    #[test]
    fn malformed_bracket_prefixes_are_left_alone() {
        for msg in ["[] empty", "[has space] x", "[nospace]tail", "[unclosed"] {
            let err = ClientError::from_server_message(msg, ErrorConvention::BracketCode);
            assert_eq!(
                err,
                ClientError::Server {
                    message: msg.to_owned(),
                    code: None,
                },
                "{msg} must not yield a code"
            );
        }
    }
}

//! Error-string helpers for the two family conventions (SRV-021).
//!
//! `Response.result` errors are plain strings that travel verbatim
//! (WIRE-040); the family models exactly two prefix conventions —
//! `ERR`/`NOAUTH`/`WRONGPASS` (Nexus, Synap) and `"[code] message"`
//! (Vectorizer; Lexum composes both). These helpers exist so products
//! never hand-roll them.

/// Family-pinned auth-required error (`Resp3Prefixes`, SRV-011).
pub const NOAUTH: &str = "NOAUTH Authentication required.";

/// Family-pinned bad-credentials error (`Resp3Prefixes`).
pub const WRONGPASS: &str = "WRONGPASS invalid username-password pair or user is disabled.";

/// Format the Vectorizer/Lexum machine-readable convention:
/// `"[<code>] <message>"` (PRO-014 `BracketCode`).
pub fn format_bracket_code(code: &str, message: &str) -> String {
    format!("[{code}] {message}")
}

/// Format the Nexus/Synap generic-error convention: `"ERR <message>"`
/// (PRO-014 `Resp3Prefixes`).
pub fn format_err(message: &str) -> String {
    format!("ERR {message}")
}

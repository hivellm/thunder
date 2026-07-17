//! Protocol profiles (SPEC-002) — the declarative description of how one
//! product uses the shared wire. Pure data: the codec never depends on it;
//! `thunder-client` / `thunder-server` drive their behavior from it.
//!
//! The family registry constants below are generated-by-hand from
//! `conformance/profiles/*.yaml` (PRO-010) and pinned to those files by a
//! test — server and SDKs of one product can never disagree. Custom
//! construction stays public (PRO-020): new products never wait for a
//! Thunder release.

use crate::wire::DEFAULT_MAX_FRAME_BYTES;

/// Handshake style (PRO-001).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handshake {
    /// No RPC-layer auth (Synap v1 legacy).
    None,
    /// `HELLO` optional; `AUTH [api_key]` or `[user, pass]`; pre-auth
    /// allowlist `PING/HELLO/AUTH/QUIT` (Nexus).
    AuthCommand,
    /// `HELLO` must be the first frame, carrying credentials
    /// (Vectorizer / Lexum).
    HelloMandatory,
}

/// HELLO payload style (PRO-001).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelloStyle {
    /// No HELLO in the profile (Synap).
    NotUsed,
    /// Positional `[Int(version)]` (Nexus).
    PositionalVersion,
    /// Map with `version`, `token` | `api_key`, `client_name`; reply
    /// carries `capabilities` (Vectorizer / Lexum).
    MapPayload,
}

/// Server-push policy (PRO-001).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushPolicy {
    /// `PUSH_ID` reserved: servers refuse it from clients and never emit it.
    Reserved,
    /// Push frames flow (Synap `SUBSCRIBE`).
    Enabled,
}

/// Which error-string prefix conventions the client parses (PRO-014).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorConvention {
    /// No prefix parsing.
    None,
    /// `ERR` / `NOAUTH` / `WRONGPASS` / `NOPERM` prefixes (Nexus, Synap).
    Resp3Prefixes,
    /// Leading `"[<code>] "` machine-readable code (Vectorizer).
    BracketCode,
    /// Both conventions composed (Lexum).
    Both,
}

/// Transport-security policy (PRO-001).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TlsPolicy {
    /// Plain TCP.
    Off,
    /// TLS available behind configuration (rustls).
    Optional,
    /// Config keys reserved; not wired yet.
    Reserved,
}

/// One product's protocol profile (PRO-001). Profiles are data, never
/// behavior: no profile may alter wire bytes (PRO-003).
#[derive(Debug, Clone, PartialEq)]
pub struct Profile {
    /// Registry name (`synap`, `nexus`, …) or a custom identifier.
    pub name: &'static str,
    /// URL scheme the endpoint parser registers for this profile (PRO-012).
    pub scheme: &'static str,
    /// Default RPC port for the scheme (PRO-012).
    pub default_port: u16,
    pub handshake: Handshake,
    pub hello_style: HelloStyle,
    pub push: PushPolicy,
    /// Frame cap (WIRE-020).
    pub max_frame_bytes: usize,
    /// Per-connection in-flight request bound (CLT-012 / SRV-003).
    pub max_in_flight: usize,
    pub error_codes: ErrorConvention,
    pub tls: TlsPolicy,
}

impl Profile {
    /// Synap — protocol origin. No RPC-layer auth, push enabled, 512 MiB cap
    /// (matches `synap-protocol`'s `MAX_FRAME_SIZE`).
    pub const fn synap() -> Self {
        Self {
            name: "synap",
            scheme: "synap",
            default_port: 15501,
            handshake: Handshake::None,
            hello_style: HelloStyle::NotUsed,
            push: PushPolicy::Enabled,
            max_frame_bytes: 512 * 1024 * 1024,
            max_in_flight: 256,
            error_codes: ErrorConvention::Resp3Prefixes,
            tls: TlsPolicy::Off,
        }
    }

    /// Nexus — canonical spec author. Optional HELLO + AUTH, 64 MiB cap.
    pub const fn nexus() -> Self {
        Self {
            name: "nexus",
            scheme: "nexus",
            default_port: 15475,
            handshake: Handshake::AuthCommand,
            hello_style: HelloStyle::PositionalVersion,
            push: PushPolicy::Reserved,
            max_frame_bytes: DEFAULT_MAX_FRAME_BYTES,
            max_in_flight: 1024,
            error_codes: ErrorConvention::Resp3Prefixes,
            tls: TlsPolicy::Off,
        }
    }

    /// Vectorizer — HELLO-mandatory with credentials, `[code]` prefixes.
    pub const fn vectorizer() -> Self {
        Self {
            name: "vectorizer",
            scheme: "vectorizer",
            default_port: 15503,
            handshake: Handshake::HelloMandatory,
            hello_style: HelloStyle::MapPayload,
            push: PushPolicy::Reserved,
            max_frame_bytes: DEFAULT_MAX_FRAME_BYTES,
            max_in_flight: 256,
            error_codes: ErrorConvention::BracketCode,
            tls: TlsPolicy::Optional,
        }
    }

    /// Lexum — Vectorizer-style handshake, both error conventions.
    pub const fn lexum() -> Self {
        Self {
            name: "lexum",
            scheme: "lexum",
            default_port: 17001,
            handshake: Handshake::HelloMandatory,
            hello_style: HelloStyle::MapPayload,
            push: PushPolicy::Reserved,
            max_frame_bytes: DEFAULT_MAX_FRAME_BYTES,
            max_in_flight: 256,
            error_codes: ErrorConvention::Both,
            tls: TlsPolicy::Reserved,
        }
    }

    /// Every registered family profile (PRO-010).
    pub const fn registry() -> [Self; 4] {
        [
            Self::synap(),
            Self::nexus(),
            Self::vectorizer(),
            Self::lexum(),
        ]
    }
}

//! Protocol profiles (SPEC-002) — the declarative description of how one
//! product uses the shared wire. Pure data: the codec never depends on it;
//! `thunder::client` / `thunder::server` drive their behavior from it.
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
    /// No RPC-layer handshake at all: the connection is usable immediately.
    ///
    /// No registered family profile uses this. It was the mistaken reading
    /// of Synap, whose RPC path *does* authenticate (`AUTH` handler behind
    /// its `require_auth` toggle) — see the BN-023 errata. It stays
    /// available for custom profiles (PRO-020).
    None,
    /// `HELLO` optional; `AUTH [api_key]` or `[user, pass]`; pre-auth
    /// allowlist `PING/HELLO/AUTH/QUIT` (Nexus, Synap).
    ///
    /// Whether a deployment *enforces* credentials is its own config
    /// (`auth_required` / `require_auth`), not a protocol dialect: a client
    /// with no credentials configured simply sends no `AUTH`, which is
    /// correct against an open deployment.
    AuthCommand,
    /// `HELLO` must be the first frame, carrying credentials
    /// (Vectorizer / Lexum).
    HelloMandatory,
}

/// HELLO payload style (PRO-001).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelloStyle {
    /// The profile has no `HELLO` command (Synap: its RPC path ships an
    /// `AUTH` handler but no `HELLO` handler at all).
    NotUsed,
    /// `HELLO` with **no arguments**; the reply is a metadata Map
    /// `{server, version, proto, id, authenticated}` (Nexus). Credentials
    /// travel via `AUTH`, never inside the HELLO.
    ArgLess,
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
    /// Synap — protocol origin. `AUTH`-command auth with **no HELLO**, push
    /// enabled, 512 MiB cap (matches `synap-protocol`'s `MAX_FRAME_SIZE`).
    ///
    /// Its RPC listener authenticates inline in the read loop (`AUTH` →
    /// shared `UserManager`, `NOAUTH` gate, `NOPERM` admin ACL) behind the
    /// `require_auth` config toggle; it simply has no `HELLO` handler. The
    /// registry previously said `handshake: none`, which described only the
    /// `require_auth = false` posture and left this profile unable to
    /// authenticate at all (BN-023 errata).
    pub const fn synap() -> Self {
        Self {
            name: "synap",
            scheme: "synap",
            default_port: 15501,
            handshake: Handshake::AuthCommand,
            hello_style: HelloStyle::NotUsed,
            push: PushPolicy::Enabled,
            max_frame_bytes: 512 * 1024 * 1024,
            max_in_flight: 256,
            error_codes: ErrorConvention::Resp3Prefixes,
            tls: TlsPolicy::Off,
        }
    }

    /// Nexus — canonical spec author. Optional arg-less HELLO + AUTH,
    /// 64 MiB cap.
    ///
    /// Its RPC `HELLO` takes no arguments and answers with a metadata Map;
    /// the positional `[Int(1)]` the registry used to claim is the *RESP3*
    /// HELLO, a different surface (BN-023 errata).
    pub const fn nexus() -> Self {
        Self {
            name: "nexus",
            scheme: "nexus",
            default_port: 15475,
            handshake: Handshake::AuthCommand,
            hello_style: HelloStyle::ArgLess,
            push: PushPolicy::Reserved,
            max_frame_bytes: DEFAULT_MAX_FRAME_BYTES,
            max_in_flight: 1024,
            error_codes: ErrorConvention::Resp3Prefixes,
            tls: TlsPolicy::Off,
        }
    }

    /// Vectorizer — HELLO-mandatory with credentials, `[code]` prefixes.
    ///
    /// TLS is described in its RPC spec but never wired — its `RpcConfig`
    /// exposes no cert/key keys and the listener binds plain TCP — so the
    /// profile records the capability as reserved, not optional (BN-023
    /// errata). No family product runs RPC TLS today.
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
            tls: TlsPolicy::Reserved,
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

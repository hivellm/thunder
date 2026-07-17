//! Protocol configuration (SPEC-002) — the declarative description of how
//! **one application** uses the shared wire. Pure data: the codec never
//! depends on it; `thunder::client` / `thunder::server` drive their
//! behavior from it.
//!
//! # Thunder ships one standard and zero product knowledge
//!
//! There are no named configurations here — no `synap()`, no `nexus()`, no
//! registry of products. Thunder was born from three products' RPC
//! implementations, but a protocol library that must serve implementations
//! which do not exist yet cannot ship a hardcoded list of the ones that did.
//!
//! Instead: [`Config::standard()`] is **the** family standard, and every
//! dimension is a knob. An application that matches the standard writes its
//! identity and nothing else:
//!
//! ```
//! use thunder::Config;
//!
//! let config = Config::standard().scheme("myapp").port(9000);
//! ```
//!
//! An application that still diverges says so **in its own repository**,
//! where that knowledge belongs:
//!
//! ```
//! use thunder::wire::config::{Handshake, HelloStyle, PushPolicy};
//! use thunder::Config;
//!
//! // A deployment whose RPC path authenticates via AUTH and has no HELLO
//! // handler, and which ships a push-producing command.
//! let config = Config::standard()
//!     .scheme("legacy")
//!     .port(15501)
//!     .handshake(Handshake::AuthCommand)
//!     .hello_style(HelloStyle::NotUsed)
//!     .push(PushPolicy::Enabled);
//! ```
//!
//! Convergence is therefore visible and per-application: delete overrides
//! until only `scheme` and `port` remain. Nobody waits on a Thunder release
//! for a row in a registry, and Thunder never carries behavior it does not
//! own.
//!
//! The standard's values are pinned to `conformance/standard.yaml` by a
//! test in every language, so the four implementations can never disagree
//! about what "standard" means — the one guarantee the old per-product
//! registry legitimately provided.

use crate::wire::DEFAULT_MAX_FRAME_BYTES;

/// Handshake style (PRO-001).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handshake {
    /// No RPC-layer handshake at all: the connection is usable immediately.
    None,
    /// `HELLO` optional; `AUTH [api_key]` / `[user, pass]` / `[password]`;
    /// pre-auth allowlist `PING/HELLO/AUTH/QUIT`.
    ///
    /// Whether a deployment *enforces* credentials is its own config
    /// (`ListenerConfig::auth_required`), not a protocol dialect: a client
    /// with no credentials configured simply sends no `AUTH`, which is
    /// correct against an open deployment (PRO-001a).
    AuthCommand,
    /// `HELLO` must be the first frame, carrying credentials. **The
    /// standard** — see [`Config::standard`].
    HelloMandatory,
}

/// HELLO payload style (PRO-001).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelloStyle {
    /// The application has no `HELLO` command.
    NotUsed,
    /// `HELLO` with **no arguments**; the reply is a metadata Map
    /// `{server, version, proto, id, authenticated}`. Credentials travel
    /// via `AUTH`, never inside the HELLO.
    ArgLess,
    /// Map with `version`, `token` | `api_key`, `client_name`; the reply
    /// carries `proto` and `capabilities`. **The standard** — the only
    /// style that negotiates a version and advertises capabilities, which
    /// is what an evolving protocol needs.
    MapPayload,
}

/// Server-push policy (PRO-001).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushPolicy {
    /// `PUSH_ID` reserved: servers refuse it from clients and never emit
    /// it. **The standard** — emitting push is a capability an application
    /// opts into by shipping a push-producing command.
    Reserved,
    /// Push frames flow to the client's push hook.
    Enabled,
}

/// Which error-string prefix conventions the client parses (PRO-014).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorConvention {
    /// No prefix parsing.
    None,
    /// `ERR` / `NOAUTH` / `WRONGPASS` / `NOPERM` prefixes.
    Resp3Prefixes,
    /// Leading `"[<code>] "` machine-readable code.
    BracketCode,
    /// Both conventions composed. **The standard** — a strict superset, so
    /// it parses either grammar and needs no negotiation.
    Both,
}

/// Transport-security policy (PRO-001).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TlsPolicy {
    /// Plain TCP. **The standard default** — TLS is an additive capability
    /// a deployment turns on, never a dialect.
    Off,
    /// TLS available behind configuration (rustls).
    Optional,
    /// Config keys reserved; not wired yet.
    Reserved,
}

/// One application's protocol configuration (PRO-001).
///
/// Configs are **data, never behavior**: no config may alter wire bytes
/// (PRO-003) — it selects among behaviors Thunder already implements.
/// Construct with [`Config::standard`] and the builder, or as a plain
/// struct literal; both are supported and neither requires a Thunder
/// release.
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    /// URL scheme the endpoint parser registers for this application
    /// (PRO-012). Identity — Thunder has no default for it.
    pub scheme: &'static str,
    /// Default RPC port for the scheme (PRO-012). Identity — Thunder has
    /// no default for it.
    pub default_port: u16,
    /// Handshake style.
    pub handshake: Handshake,
    /// HELLO payload style.
    pub hello_style: HelloStyle,
    /// Server-push policy.
    pub push: PushPolicy,
    /// Frame cap (WIRE-020).
    pub max_frame_bytes: usize,
    /// Per-connection in-flight request bound (CLT-012 / SRV-003).
    pub max_in_flight: usize,
    /// Error-string conventions the client parses.
    pub error_codes: ErrorConvention,
    /// Transport-security policy.
    pub tls: TlsPolicy,
}

impl Config {
    /// **The** family standard (pinned by `conformance/standard.yaml`).
    ///
    /// Mandatory `HELLO` map with `proto` negotiation and a capabilities
    /// reply; the `[CODE]` error superset; 64 MiB frames; 256 in-flight;
    /// push reserved; TLS off.
    ///
    /// `scheme` is `""` and `default_port` is `0` — identity is the
    /// application's to supply, and a `Config` that never sets them is only
    /// usable with an explicit `host:port` endpoint.
    pub const fn standard() -> Self {
        Self {
            scheme: "",
            default_port: 0,
            handshake: Handshake::HelloMandatory,
            hello_style: HelloStyle::MapPayload,
            push: PushPolicy::Reserved,
            max_frame_bytes: DEFAULT_MAX_FRAME_BYTES,
            max_in_flight: 256,
            error_codes: ErrorConvention::Both,
            tls: TlsPolicy::Off,
        }
    }

    /// Set the URL scheme this application answers on (PRO-012).
    #[must_use]
    pub const fn scheme(mut self, scheme: &'static str) -> Self {
        self.scheme = scheme;
        self
    }

    /// Set the default RPC port for the scheme (PRO-012).
    #[must_use]
    pub const fn port(mut self, port: u16) -> Self {
        self.default_port = port;
        self
    }

    /// Override the handshake style.
    #[must_use]
    pub const fn handshake(mut self, handshake: Handshake) -> Self {
        self.handshake = handshake;
        self
    }

    /// Override the HELLO payload style.
    #[must_use]
    pub const fn hello_style(mut self, hello_style: HelloStyle) -> Self {
        self.hello_style = hello_style;
        self
    }

    /// Override the server-push policy.
    #[must_use]
    pub const fn push(mut self, push: PushPolicy) -> Self {
        self.push = push;
        self
    }

    /// Override the frame cap (WIRE-020).
    #[must_use]
    pub const fn max_frame_bytes(mut self, max_frame_bytes: usize) -> Self {
        self.max_frame_bytes = max_frame_bytes;
        self
    }

    /// Override the per-connection in-flight bound.
    #[must_use]
    pub const fn max_in_flight(mut self, max_in_flight: usize) -> Self {
        self.max_in_flight = max_in_flight;
        self
    }

    /// Override the error-string conventions parsed.
    #[must_use]
    pub const fn error_codes(mut self, error_codes: ErrorConvention) -> Self {
        self.error_codes = error_codes;
        self
    }

    /// Override the transport-security policy.
    #[must_use]
    pub const fn tls(mut self, tls: TlsPolicy) -> Self {
        self.tls = tls;
        self
    }
}

impl Default for Config {
    /// The standard (see [`Config::standard`]).
    fn default() -> Self {
        Self::standard()
    }
}

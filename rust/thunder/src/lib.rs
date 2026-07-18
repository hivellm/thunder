//! HiveLLM binary RPC — **one crate** for the whole Rust stack.
//!
//! Thunder ships as a single publishable crate with three feature-gated
//! layers, so a product depends on `thunder` once instead of tracking
//! `thunder-wire` / `thunder-client` / `thunder-server` as separate
//! releases:
//!
//! - [`wire`] — the frozen wire v1 codec (value model, `Request`/`Response`,
//!   length-prefixed MessagePack frames). Always compiled; pure, no sockets
//!   (SPEC-001, `WIRE-xxx`).
//! - [`client`] — the multiplexed, profile-driven RPC client (SPEC-003,
//!   `CLT-xxx`). Feature `client` (default on).
//! - [`server`] — the family server hot path: accept loop, dispatch trait,
//!   profile enforcement (SPEC-004, `SRV-xxx`). Feature `server` (default on).
//!
//! Both `client` and `server` are on by default. A client-only SDK builds
//! with `default-features = false, features = ["client"]`; a server binary
//! with `["server"]`. The wire layer alone is `default-features = false`.
//!
//! The most-used items are re-exported at the crate root; the full surface
//! of each layer lives under its module. `client::Credentials` and
//! `server::Credentials` are distinct types and are reached through their
//! modules (they are deliberately not re-exported at the root).

pub mod wire;

pub use wire::{
    decode_frame, decode_frame_with_limit, encode_frame, Config, DecodeError, Request, Response,
    Value, DEFAULT_MAX_FRAME_BYTES, PUSH_ID,
};

pub mod tls;

pub use tls::{ClientTls, ServerTls};

#[cfg(feature = "client")]
pub mod client;

#[cfg(feature = "client")]
pub use client::{
    parse_endpoint, Client, ClientConfig, ClientError, Endpoint, HandshakeInfo, Pool, PooledConn,
};

#[cfg(feature = "server")]
pub mod server;

#[cfg(feature = "server")]
pub use server::{
    format_bracket_code, format_err, spawn_listener, AuthError, Dispatch, ListenerConfig,
    ListenerHandle, MetricsSnapshot, Principal, PushClosed, PushSender, ServerInfo, Session,
    NOAUTH, NOPERM, WRONGPASS,
};

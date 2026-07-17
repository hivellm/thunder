//! HiveLLM binary RPC wire layer — **wire v1, frozen**.
//!
//! One frame is `u32 LE length` + MessagePack body; the body is a
//! [`Request`] or [`Response`] in rmp-serde's externally-tagged encoding
//! over the 8-variant [`Value`] model. The normative byte definition lives
//! in `docs/spec/` (transplanted family spec); this crate is bound to it by
//! `docs/specs/SPEC-001-wire-format.md` (`WIRE-xxx` requirements).
//!
//! Canonicalization over the donor implementations (SPEC-001 §2):
//! - `Bytes` is emitted as MessagePack **bin** (WIRE-010) — ~33% smaller
//!   than the int-array form every pre-Thunder Rust server emits — while
//!   the legacy int-array form is accepted on decode forever (WIRE-011).
//! - `Request`/`Response` are emitted as array-encoded structs (WIRE-012);
//!   map-shaped requests decode fine (WIRE-013, rmp-serde leniency).
//!
//! This crate is pure: no sockets, no product knowledge (WIRE-030). Async
//! frame helpers are available behind the `tokio` feature.

mod frame;
mod value;

pub use frame::{decode_frame, decode_frame_with_limit, encode_frame, DecodeError};
pub use value::{Request, Response, Value};

#[cfg(feature = "tokio")]
pub use frame::{
    read_frame, read_request, read_request_with_limit, read_response, read_response_with_limit,
    write_frame, write_request, write_response,
};

/// Reserved frame id for server-initiated push frames (WIRE-005).
///
/// Clients must never use it as a request id; servers refuse requests
/// carrying it; client demultiplexers route it to the push hook.
pub const PUSH_ID: u32 = u32::MAX;

/// Default frame-body cap: 64 MiB, validated against the length prefix
/// *before* any allocation (WIRE-020). Operators tune it per profile.
pub const DEFAULT_MAX_FRAME_BYTES: usize = 64 * 1024 * 1024;

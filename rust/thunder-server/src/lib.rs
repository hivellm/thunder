//! Thunder RPC server — the one listener every family server derives from.
//!
//! Products integrate by implementing [`Dispatch`] (SRV-020) and calling
//! [`spawn_listener`] with their [`Profile`](thunder_wire::Profile);
//! everything else is Thunder's:
//!
//! - lifecycle — accept loop, per-connection writer task, graceful drain,
//!   per-connection failure isolation (SRV-001..005);
//! - the Synap-derived hot path — `BufWriter` + drain-then-flush, exactly
//!   one serialization per response, `TCP_NODELAY`, per-read idle timeout
//!   (SRV-006..009);
//! - sessions and profile enforcement — lock-free auth flag, handshake
//!   gating, Thunder-built HELLO replies, PUSH_ID policy
//!   (SRV-010..014, PRO-030/031);
//! - plain-atomic metrics recorded after successful writes (SRV-030).
//!
//! Contract: `docs/specs/SPEC-004-server.md`. TLS (SRV-040) is pending the
//! T0 family decision and is intentionally not wired yet.

mod dispatch;
mod errors;
mod listener;
mod metrics;
mod session;

pub use dispatch::{AuthError, Credentials, Dispatch, Principal};
pub use errors::{format_bracket_code, format_err, NOAUTH, WRONGPASS};
pub use listener::{spawn_listener, ListenerConfig, ListenerHandle, ServerInfo};
pub use metrics::MetricsSnapshot;
pub use session::{PushClosed, PushSender, Session};

pub use thunder_wire as wire;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests;

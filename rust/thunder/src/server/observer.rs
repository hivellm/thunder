//! Optional per-command metrics observer (SRV-030 extension).
//!
//! [`MetricsSnapshot`](crate::server::MetricsSnapshot) gives an exporter
//! cumulative totals, read whenever it likes. That is enough to graph rates,
//! and it is not enough for two things a product exporter usually already has:
//!
//! - **Per-command dimensions.** `commands_total` is listener-wide, so a
//!   `{command}` label cannot be recovered from it. Timing inside
//!   [`Dispatch::dispatch`](crate::server::Dispatch) is not the same
//!   measurement — that is the dispatch window, while the listener records the
//!   frame-received-to-frame-sent window, and the two disagree.
//! - **Distributions.** A histogram of frame sizes cannot be reconstructed from
//!   a byte total.
//!
//! Without a callback the only ingestion path is sampling, which adds a task,
//! adds staleness up to the sample interval, and cannot see anything that
//! happened between two ticks.
//!
//! So: an optional observer, invoked at exactly the point the built-in metrics
//! record — **after the successful socket write** — with values the listener
//! already holds. It is `None` by default and costs nothing when unset; the
//! command label is not even materialized unless an observer is installed.

use std::time::Duration;

/// Receives one callback per completed command, plus connection lifecycle.
///
/// Every method must be cheap and must not block: they run on the connection's
/// writer task, so time spent here is time the socket is not being written.
/// Anything expensive belongs behind a channel.
pub trait MetricsObserver: Send + Sync + 'static {
    /// One command completed and its response left the socket.
    ///
    /// `in_bytes` is the request frame size from the decoder and `out_bytes`
    /// the encoded response length — neither is ever re-encoded to be measured
    /// (SRV-007). `duration` is the dispatch time, and `is_error` reflects the
    /// response carrying `Err`, not a transport failure.
    fn command_completed(
        &self,
        command: &str,
        in_bytes: usize,
        out_bytes: usize,
        duration: Duration,
        is_error: bool,
    );

    /// A connection was accepted.
    fn connection_opened(&self) {}

    /// A connection finished draining and closed.
    fn connection_closed(&self) {}

    /// An accept was refused at the `max_connections` ceiling.
    fn connection_refused(&self) {}

    /// A server-initiated frame was written (`id == PUSH_ID`, WIRE-005).
    fn push_emitted(&self, out_bytes: usize) {
        let _ = out_bytes;
    }
}

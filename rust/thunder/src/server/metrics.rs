//! Server metrics as plain atomics (SRV-030) — snapshot-friendly for any
//! exporter, no metrics-framework dependency. Every series records **after**
//! a successful socket write, per the writer contract; byte counts
//! come from the decoder's frame size (in) and the single encoded response
//! buffer (out) — nothing is ever re-encoded to be measured (SRV-007).

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// The SRV-030 atomic series. Interior to the listener; consumers read it
/// through [`MetricsSnapshot`].
#[derive(Debug, Default)]
pub(crate) struct Metrics {
    connections: AtomicU64,
    connections_refused_total: AtomicU64,
    commands_total: AtomicU64,
    commands_error_total: AtomicU64,
    command_duration_microseconds_total: AtomicU64,
    frame_bytes_in_total: AtomicU64,
    frame_bytes_out_total: AtomicU64,
    slow_commands_total: AtomicU64,
    non_hello_first_frames_total: AtomicU64,
}

impl Metrics {
    /// Gauge up: one connection accepted.
    pub(crate) fn connection_opened(&self) {
        self.connections.fetch_add(1, Ordering::Relaxed);
    }

    /// One accept refused because the listener was at its connection ceiling
    /// (`ListenerConfig::max_connections`). Counted so a ceiling that is
    /// engaging is visible instead of looking like client-side failures.
    pub(crate) fn connection_refused(&self) {
        self.connections_refused_total
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Gauge down: one connection fully drained and closed.
    pub(crate) fn connection_closed(&self) {
        self.connections.fetch_sub(1, Ordering::Relaxed);
    }

    /// Record one completed command — called by the writer task after the
    /// response left the socket (SRV-030). A zero `slow_threshold`
    /// disables the slow counter.
    pub(crate) fn record_command(
        &self,
        in_bytes: usize,
        out_bytes: usize,
        duration: Duration,
        is_error: bool,
        slow_threshold: Duration,
    ) {
        self.commands_total.fetch_add(1, Ordering::Relaxed);
        if is_error {
            self.commands_error_total.fetch_add(1, Ordering::Relaxed);
        }
        self.command_duration_microseconds_total
            .fetch_add(duration.as_micros() as u64, Ordering::Relaxed);
        self.frame_bytes_in_total
            .fetch_add(in_bytes as u64, Ordering::Relaxed);
        self.frame_bytes_out_total
            .fetch_add(out_bytes as u64, Ordering::Relaxed);
        if !slow_threshold.is_zero() && duration >= slow_threshold {
            self.slow_commands_total.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Record one push frame (SRV-013): only out-bytes — pushes are not
    /// commands.
    pub(crate) fn record_push(&self, out_bytes: usize) {
        self.frame_bytes_out_total
            .fetch_add(out_bytes as u64, Ordering::Relaxed);
    }

    /// Record one connection whose first frame was **not** a canonical
    /// `HELLO` (SPEC-008 handshake section): the adoption signal a product
    /// watches while migrating its clients to lead with `HELLO`, before it
    /// cuts a legacy first-frame path. Cumulative; zero under a profile whose
    /// clients always lead with `HELLO` (`HelloMandatory`).
    pub(crate) fn record_non_hello_first_frame(&self) {
        self.non_hello_first_frames_total
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Point-in-time copy of every series.
    pub(crate) fn snapshot(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            connections: self.connections.load(Ordering::Relaxed),
            connections_refused_total: self.connections_refused_total.load(Ordering::Relaxed),
            commands_total: self.commands_total.load(Ordering::Relaxed),
            commands_error_total: self.commands_error_total.load(Ordering::Relaxed),
            command_duration_microseconds_total: self
                .command_duration_microseconds_total
                .load(Ordering::Relaxed),
            frame_bytes_in_total: self.frame_bytes_in_total.load(Ordering::Relaxed),
            frame_bytes_out_total: self.frame_bytes_out_total.load(Ordering::Relaxed),
            slow_commands_total: self.slow_commands_total.load(Ordering::Relaxed),
            non_hello_first_frames_total: self.non_hello_first_frames_total.load(Ordering::Relaxed),
        }
    }
}

/// One consistent-enough read of the listener's counters (SRV-030),
/// exporter-agnostic by design.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MetricsSnapshot {
    /// Currently open connections (gauge).
    pub connections: u64,
    /// Accepts refused at the `max_connections` ceiling.
    pub connections_refused_total: u64,
    /// Responses written, success or error.
    pub commands_total: u64,
    /// Error responses written.
    pub commands_error_total: u64,
    /// Total dispatch time across all commands, microseconds.
    pub command_duration_microseconds_total: u64,
    /// Request bytes as counted by the decoder's length prefix (SRV-007).
    pub frame_bytes_in_total: u64,
    /// Response/push bytes as counted from the encoded buffers (SRV-007).
    pub frame_bytes_out_total: u64,
    /// Commands slower than the configured threshold (SRV-030).
    pub slow_commands_total: u64,
    /// Connections whose first frame was not a canonical `HELLO` — the
    /// lead-with-`HELLO` adoption signal (SPEC-008 handshake section).
    pub non_hello_first_frames_total: u64,
}

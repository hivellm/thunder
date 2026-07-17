//! Per-connection session state (SRV-010) and the typed push channel
//! (SRV-013).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use crate::wire::{Response, Value, PUSH_ID};
use tokio::sync::mpsc;

use crate::server::dispatch::Principal;

/// One job for the connection's writer task (SRV-002).
///
/// Dispatch tasks and read-loop built-ins enqueue [`WriteJob::Response`];
/// product code enqueues [`WriteJob::Push`] through a [`PushSender`]; the
/// read side enqueues [`WriteJob::Shutdown`] once every in-flight dispatch
/// task has finished, so the writer exits even while product-held
/// `PushSender` clones keep the channel open.
#[derive(Debug)]
pub(crate) enum WriteJob {
    /// A response to a client request plus the metadata the writer records
    /// after the successful write (SRV-030).
    Response {
        response: Response,
        /// Request frame size straight from the decoder (SRV-007) — never
        /// re-encoded.
        in_bytes: usize,
        /// Dispatch duration for the duration / slow counters.
        duration: Duration,
    },
    /// A server-initiated frame (`id == PUSH_ID`, WIRE-005).
    Push(Response),
    /// Drain what is queued, flush, exit.
    Shutdown,
}

/// Session state shared between the read loop and every dispatch task
/// (SRV-010): the auth flag is a lock-free atomic flipped by `HELLO`/`AUTH`
/// and read by the dispatch path without locks.
#[derive(Debug)]
pub struct Session {
    connection_id: u64,
    authenticated: AtomicBool,
    principal: Mutex<Option<Principal>>,
    push: Option<PushSender>,
}

impl Session {
    /// New session. `pre_authenticated` is true for `Handshake::None`
    /// profiles (no RPC-layer auth, SRV-011); `push` is present only under
    /// `PushPolicy::Enabled` (SRV-013).
    pub(crate) fn new(
        connection_id: u64,
        pre_authenticated: bool,
        push: Option<PushSender>,
    ) -> Self {
        Self {
            connection_id,
            authenticated: AtomicBool::new(pre_authenticated),
            principal: Mutex::new(None),
            push,
        }
    }

    /// Listener-scoped connection id, surfaced in the metadata-shape HELLO
    /// reply (SRV-014).
    pub fn connection_id(&self) -> u64 {
        self.connection_id
    }

    /// Lock-free read of the auth flag (SRV-010).
    pub fn is_authenticated(&self) -> bool {
        self.authenticated.load(Ordering::Acquire)
    }

    /// The principal resolved by the last successful `HELLO`/`AUTH`.
    pub fn principal(&self) -> Option<Principal> {
        self.principal
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Typed push channel for this connection (SRV-013). `Some` only under
    /// `push = Enabled` profiles; under `Reserved` no push emission is
    /// possible.
    pub fn push_sender(&self) -> Option<&PushSender> {
        self.push.as_ref()
    }

    /// Store the authenticated principal, then flip the flag — the
    /// `Release`/`Acquire` pair makes the principal visible to any task
    /// that observes `is_authenticated() == true` (SRV-010).
    pub(crate) fn set_principal(&self, principal: Principal) {
        *self
            .principal
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(principal);
        self.authenticated.store(true, Ordering::Release);
    }
}

/// Typed, clonable handle for server-initiated push frames (SRV-013).
///
/// Handed to product code via [`Session::push_sender`] under
/// `push = Enabled` profiles. It wraps the connection's writer channel and
/// forces `id = PUSH_ID`, so product code can never collide with request
/// ids. Clones stay valid for the connection's lifetime — subscription
/// flows (a subscribe-style command) may emit long after the registering request
/// completed.
#[derive(Debug, Clone)]
pub struct PushSender {
    tx: mpsc::Sender<WriteJob>,
}

impl PushSender {
    pub(crate) fn new(tx: mpsc::Sender<WriteJob>) -> Self {
        Self { tx }
    }

    /// Emit one push frame carrying `value`. Fails once the connection has
    /// closed and its writer drained (SRV-004).
    pub async fn push(&self, value: Value) -> Result<(), PushClosed> {
        self.tx
            .send(WriteJob::Push(Response::ok(PUSH_ID, value)))
            .await
            .map_err(|_| PushClosed)
    }
}

/// The connection behind a [`PushSender`] is gone; the frame was dropped.
#[derive(Debug, thiserror::Error)]
#[error("connection closed; push frame dropped")]
pub struct PushClosed;

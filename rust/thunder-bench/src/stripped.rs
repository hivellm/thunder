//! **Diagnostic lane**: Thunder's wire on a bare listener (BEN-001 fairness).
//!
//! # Why this exists
//!
//! The T4.3 matrix showed Thunder losing cells to peers that move far more
//! bytes (HTTP out-throughputs it at depth 16 while sending 4192 B/op against
//! Thunder's 83). That is only explicable if the cost is **per-request
//! execution**, not the wire — but the matrix could not prove it, because the
//! two sides were not comparable:
//!
//! | | `thunder::server` | the RESP3 / Bolt / HTTP peers |
//! |---|---|---|
//! | dispatch | `tokio::spawn` per request | inline in the read loop |
//! | bound | `Semaphore::acquire_owned` per request | none |
//! | per request | 3 `Arc::clone` + mpsc send | direct write |
//! | also | session state, handshake gating, PUSH_ID check, per-op metrics | none |
//!
//! Comparing those measures Thunder's **server features** against the peers'
//! **absence of them** — a confound BEN-001 forbids ("isolate the transport").
//!
//! # What this is
//!
//! The same [`thunder::wire`] codec on a listener as bare as the peers': read
//! a frame, dispatch inline, write it back, count bytes. No sessions, no
//! handshake, no semaphore, no spawn, no per-op metrics beyond the byte
//! counters every lane keeps.
//!
//! Driven by the **same** [`thunder::client::Client`] as the real Thunder
//! lane, so the client side is held constant. That gives a two-way
//! decomposition the matrix alone cannot:
//!
//! - `stripped` vs `resp3`/`bolt`/`http` → what the **wire** costs;
//! - `thunder` vs `stripped` → what the **server features** cost.
//!
//! # What this is NOT
//!
//! Not a product, not a proposal, and never a G5 lane. It answers one
//! question. It deliberately drops guarantees `thunder::server` must keep —
//! most importantly it **head-of-line blocks**: a slow command stalls its
//! connection, which is exactly what SRV-002/003's spawn-per-request buys and
//! why the answer here cannot simply be "stop spawning".

use std::io;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use thunder::wire::{encode_frame, read_request_with_limit, Response};
use tokio::io::{AsyncWriteExt, BufReader, BufWriter};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, watch};

use crate::backend::NoopBackend;
use crate::driver::bench_profile;

/// Server-side counters, sampled around a measured window — the same
/// measurement point as every other lane (after the successful write).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StrippedMetricsSnapshot {
    /// Requests answered.
    pub requests: u64,
    /// Request bytes read off the wire.
    pub bytes_in: u64,
    /// Response bytes written to the wire.
    pub bytes_out: u64,
}

#[derive(Debug, Default)]
struct StrippedMetrics {
    requests: AtomicU64,
    bytes_in: AtomicU64,
    bytes_out: AtomicU64,
}

impl StrippedMetrics {
    fn snapshot(&self) -> StrippedMetricsSnapshot {
        StrippedMetricsSnapshot {
            requests: self.requests.load(Ordering::Relaxed),
            bytes_in: self.bytes_in.load(Ordering::Relaxed),
            bytes_out: self.bytes_out.load(Ordering::Relaxed),
        }
    }
}

/// Handle to the running bare-wire listener.
#[derive(Debug)]
pub struct StrippedHandle {
    addr: SocketAddr,
    shutdown: watch::Sender<bool>,
    metrics: Arc<StrippedMetrics>,
    done: Option<mpsc::Receiver<()>>,
}

impl StrippedHandle {
    /// The bound address.
    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    /// Current server-side counters.
    pub fn snapshot(&self) -> StrippedMetricsSnapshot {
        self.metrics.snapshot()
    }

    /// Graceful shutdown.
    pub async fn stop(mut self) {
        let _ = self.shutdown.send(true);
        if let Some(mut done) = self.done.take() {
            let _ = done.recv().await;
        }
    }
}

impl Drop for StrippedHandle {
    fn drop(&mut self) {
        let _ = self.shutdown.send(true);
    }
}

/// Spawn the bare-wire listener over the shared no-op backend.
pub async fn spawn_stripped_listener(
    backend: Arc<NoopBackend>,
    addr: SocketAddr,
) -> io::Result<StrippedHandle> {
    let listener = TcpListener::bind(addr).await?;
    let addr = listener.local_addr()?;
    let metrics = Arc::new(StrippedMetrics::default());
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let (done_tx, done_rx) = mpsc::channel::<()>(1);

    let accept_metrics = Arc::clone(&metrics);
    tokio::spawn(async move {
        let _done = done_tx;
        let mut shutdown = shutdown_rx;
        loop {
            let accepted = tokio::select! {
                _ = shutdown.wait_for(|stop| *stop) => break,
                accepted = listener.accept() => accepted,
            };
            let Ok((stream, _)) = accepted else { break };
            let backend = Arc::clone(&backend);
            let metrics = Arc::clone(&accept_metrics);
            let conn_shutdown = shutdown.clone();
            tokio::spawn(serve_conn(stream, backend, metrics, conn_shutdown));
        }
    });

    Ok(StrippedHandle {
        addr,
        shutdown: shutdown_tx,
        metrics,
        done: Some(done_rx),
    })
}

/// One connection: read, dispatch inline, write. Nothing else.
///
/// Mirrors the peers' `BufWriter` + drain-then-flush so the write path is not
/// the difference either — only the dispatch model is.
async fn serve_conn(
    stream: TcpStream,
    backend: Arc<NoopBackend>,
    metrics: Arc<StrippedMetrics>,
    mut shutdown: watch::Receiver<bool>,
) {
    let _ = stream.set_nodelay(true);
    let (read_half, write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);
    let mut writer = BufWriter::new(write_half);
    let cap = bench_profile().max_frame_bytes;

    loop {
        let read = tokio::select! {
            _ = shutdown.wait_for(|stop| *stop) => break,
            read = read_request_with_limit(&mut reader, cap) => read,
        };
        let Ok((req, in_bytes)) = read else { break };
        metrics
            .bytes_in
            .fetch_add(in_bytes as u64, Ordering::Relaxed);

        // Inline dispatch — the whole point of this lane.
        let response = match backend.respond(&req.command, req.args) {
            Ok(value) => Response::ok(req.id, value),
            Err(message) => Response::err(req.id, message),
        };
        let Ok(frame) = encode_frame(&response) else {
            break;
        };
        if writer.write_all(&frame).await.is_err() {
            break;
        }
        metrics
            .bytes_out
            .fetch_add(frame.len() as u64, Ordering::Relaxed);
        metrics.requests.fetch_add(1, Ordering::Relaxed);

        // Drain-then-flush (SRV-006 analog): only pay the syscall once the
        // pipeline is empty, exactly like the other lanes.
        if reader.buffer().is_empty() && writer.flush().await.is_err() {
            break;
        }
    }
    let _ = writer.flush().await;
}

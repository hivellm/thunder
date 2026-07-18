//! **NATS request/reply** lane — a minimal broker of ours, driven by the real
//! `async-nats` client, serving the same no-op backend (BEN-001, BEN-002) in
//! the same process, host, runtime and allocator as the Thunder listener.
//!
//! # ⚠ This lane measures a different SHAPE, not just a different protocol
//!
//! Every other lane in the shootout is point-to-point: the client's request
//! crosses one socket to the server, and the reply crosses back. **NATS
//! request/reply crosses four.**
//!
//! ```text
//!   RPC lanes:   client ──▶ server ──▶ client                  (2 traversals)
//!   this lane:   requester ──▶ broker ──▶ responder
//!                requester ◀── broker ◀── responder            (4 traversals)
//! ```
//!
//! That is not a flaw in the measurement — it *is* the measurement. NATS has
//! no server-side request/reply concept at all: the broker only routes
//! subjects, and "request/reply" is a convention where the requester
//! subscribes to an inbox and puts that inbox in the message's reply-to field.
//! Anyone choosing NATS over an RPC transport is choosing that topology, and
//! this lane prices it honestly. **Comparing this lane's latency against
//! Thunder's as though both were transports would be a category error** — the
//! comparison being drawn is architectural: what does putting a broker in the
//! middle cost?
//!
//! To keep it faithful the harness runs a **real responder**: a separate
//! `async-nats` connection that subscribes to the request subject, calls the
//! shared backend, and publishes to the reply subject. Collapsing the
//! responder into the broker would have made the numbers look better and the
//! comparison meaningless.
//!
//! # Real client, our broker — because no Rust NATS broker exists
//!
//! The reference NATS server is Go and has no Rust port; the ecosystem is
//! client-only (`async-nats` is the official client, `nats` is deprecated,
//! everything else is abandoned). So the split follows the policy the rest of
//! the expansion uses: **real where a real implementation exists, ours where
//! none does**. The client on both the requester and responder side is
//! `async-nats` 0.49 — which also validates the broker: a production client
//! completing round trips against it is strong evidence the wire is right.
//!
//! # Scope (honesty note, BEN-002)
//!
//! A **benchmark broker, not a NATS server**. Seven verbs: `INFO` and `MSG`
//! and `PONG` out; `CONNECT`, `PUB`, `SUB`, `UNSUB`, `PING`, `PONG` in.
//! Subject matching covers literals and the `*` single-token wildcard (which
//! `async-nats` needs for its multiplexed inbox). `headers:false` is
//! advertised, so clients never send `HPUB` and never enable `no_responders`.
//! No queue groups, no JetStream, no auth, no TLS, no clustering, no
//! `verbose` mode (`async-nats` sets `verbose:false`; a `+OK` path exists
//! anyway for correctness).

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use futures::stream::StreamExt;
use thunder::wire::Value;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, watch};

use crate::backend::NoopBackend;
use crate::driver::{CellSpec, Measured, RunConfig};
use crate::stats::compute;

/// The subject the responder serves.
const REQUEST_SUBJECT: &str = "bench.call";
/// Payload cap advertised in `INFO` — mirrors the Thunder frame cap
/// (WIRE-020) so an oversized declared length cannot drive an unbounded
/// allocation.
const MAX_PAYLOAD: usize = thunder::wire::DEFAULT_MAX_FRAME_BYTES;

/// Ride through a poisoned lock: the guarded state stays consistent.
fn lock<T>(mutex: &StdMutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

// ── Subject routing ─────────────────────────────────────────────────────────

/// Does a subscription subject match a published subject?
///
/// Literals compare token-wise; `*` matches exactly one token; `>` matches all
/// remaining tokens. `async-nats` uses a `*`-terminated multiplexed inbox, so
/// the single-token wildcard is not optional.
pub fn subject_matches(pattern: &str, subject: &str) -> bool {
    let mut pattern_tokens = pattern.split('.');
    let mut subject_tokens = subject.split('.');
    loop {
        match (pattern_tokens.next(), subject_tokens.next()) {
            (Some(">"), Some(_)) => return true,
            (Some("*"), Some(_)) => continue,
            (Some(p), Some(s)) if p == s => continue,
            (None, None) => return true,
            _ => return false,
        }
    }
}

/// One subscription: which connection wants it, under which subscriber id.
#[derive(Debug, Clone)]
struct Subscription {
    connection: u64,
    sid: String,
    subject: String,
}

/// The broker's routing table plus per-connection outboxes.
#[derive(Debug, Default)]
struct Router {
    subscriptions: Vec<Subscription>,
    outboxes: HashMap<u64, mpsc::UnboundedSender<Vec<u8>>>,
}

impl Router {
    /// Deliver a published message to every matching subscriber.
    fn publish(&self, subject: &str, reply_to: Option<&str>, payload: &[u8]) {
        for subscription in &self.subscriptions {
            if !subject_matches(&subscription.subject, subject) {
                continue;
            }
            let Some(outbox) = self.outboxes.get(&subscription.connection) else {
                continue;
            };
            let _ = outbox.send(encode_msg(subject, &subscription.sid, reply_to, payload));
        }
    }
}

/// `MSG <subject> <sid> [reply-to] <#bytes>\r\n<payload>\r\n`
fn encode_msg(subject: &str, sid: &str, reply_to: Option<&str>, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + subject.len() + 32);
    out.extend_from_slice(b"MSG ");
    out.extend_from_slice(subject.as_bytes());
    out.push(b' ');
    out.extend_from_slice(sid.as_bytes());
    if let Some(reply) = reply_to {
        out.push(b' ');
        out.extend_from_slice(reply.as_bytes());
    }
    out.push(b' ');
    out.extend_from_slice(payload.len().to_string().as_bytes());
    out.extend_from_slice(b"\r\n");
    out.extend_from_slice(payload);
    out.extend_from_slice(b"\r\n");
    out
}

/// The `INFO` line every client waits for on connect.
fn info_line(addr: SocketAddr) -> Vec<u8> {
    format!(
        "INFO {{\"server_id\":\"thunder-bench\",\"server_name\":\"thunder-bench\",\
         \"version\":\"2.10.0\",\"proto\":1,\"go\":\"\",\"host\":\"{}\",\"port\":{},\
         \"headers\":false,\"max_payload\":{},\"client_id\":1}}\r\n",
        addr.ip(),
        addr.port(),
        MAX_PAYLOAD,
    )
    .into_bytes()
}

// ── Broker ──────────────────────────────────────────────────────────────────

/// Server-side counters, sampled around a measured window.
#[derive(Debug, Default)]
struct NatsMetrics {
    /// Messages routed (one request delivery + one reply delivery per round
    /// trip, so this is roughly 2x the request count).
    routed: AtomicU64,
    bytes_in: AtomicU64,
    bytes_out: AtomicU64,
}

/// Handle to the running broker plus its responder.
#[derive(Debug)]
pub struct NatsHandle {
    addr: SocketAddr,
    metrics: Arc<NatsMetrics>,
    shutdown: watch::Sender<bool>,
}

impl NatsHandle {
    /// The bound address.
    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    /// Bytes read across every broker connection.
    pub fn bytes_in(&self) -> u64 {
        self.metrics.bytes_in.load(Ordering::Relaxed)
    }

    /// Bytes written across every broker connection.
    pub fn bytes_out(&self) -> u64 {
        self.metrics.bytes_out.load(Ordering::Relaxed)
    }

    /// Graceful shutdown.
    pub async fn stop(self) {
        let _ = self.shutdown.send(true);
    }
}

impl Drop for NatsHandle {
    fn drop(&mut self) {
        let _ = self.shutdown.send(true);
    }
}

/// Spawn the broker and the responder that serves the shared no-op backend.
pub async fn spawn_nats_broker(
    backend: Arc<NoopBackend>,
    addr: SocketAddr,
) -> std::io::Result<NatsHandle> {
    let listener = TcpListener::bind(addr).await?;
    let addr = listener.local_addr()?;
    let metrics = Arc::new(NatsMetrics::default());
    let router = Arc::new(StdMutex::new(Router::default()));
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    let accept_metrics = Arc::clone(&metrics);
    let accept_router = Arc::clone(&router);
    let mut accept_shutdown = shutdown_rx.clone();
    tokio::spawn(async move {
        let mut next_connection = 0u64;
        loop {
            let accepted = tokio::select! {
                _ = accept_shutdown.wait_for(|stop| *stop) => break,
                accepted = listener.accept() => accepted,
            };
            let Ok((stream, _)) = accepted else { break };
            next_connection += 1;
            tokio::spawn(serve_conn(
                stream,
                next_connection,
                addr,
                Arc::clone(&accept_router),
                Arc::clone(&accept_metrics),
                accept_shutdown.clone(),
            ));
        }
    });

    // The responder: a real NATS client subscribing to the request subject.
    // Keeping it a separate connection is what makes the lane's four-traversal
    // shape faithful — see the module docs.
    let responder = async_nats::connect(format!("nats://{addr}"))
        .await
        .map_err(|e| std::io::Error::other(format!("nats responder connect failed: {e}")))?;
    let mut requests = responder
        .subscribe(REQUEST_SUBJECT)
        .await
        .map_err(|e| std::io::Error::other(format!("nats responder subscribe failed: {e}")))?;
    let mut responder_shutdown = shutdown_rx;
    tokio::spawn(async move {
        loop {
            let message = tokio::select! {
                _ = responder_shutdown.wait_for(|stop| *stop) => break,
                message = requests.next() => message,
            };
            let Some(message) = message else { break };
            let Some(reply) = message.reply.clone() else {
                continue;
            };
            let (command, payload) = split_request(&message.payload);
            let value = match backend.respond(&command, command_args(&command, payload)) {
                Ok(value) => value_to_bytes(value),
                Err(error) => error.into_bytes(),
            };
            let _ = responder.publish(reply, value.into()).await;
            let _ = responder.flush().await;
        }
    });

    Ok(NatsHandle {
        addr,
        metrics,
        shutdown: shutdown_tx,
    })
}

/// A request payload is `<COMMAND> <payload>` — the same shape the PostgreSQL
/// lane uses, since NATS subjects carry no method name of their own here.
fn split_request(bytes: &[u8]) -> (String, Vec<u8>) {
    match bytes.iter().position(|byte| *byte == b' ') {
        Some(at) => (
            String::from_utf8_lossy(&bytes[..at]).into_owned(),
            bytes[at + 1..].to_vec(),
        ),
        None => (String::from_utf8_lossy(bytes).into_owned(), Vec::new()),
    }
}

/// A backend reply value as raw payload bytes.
fn value_to_bytes(value: Value) -> Vec<u8> {
    match value {
        Value::Str(s) => s.into_bytes(),
        Value::Bytes(b) => b.to_vec(),
        _ => Vec::new(),
    }
}

/// Turn a parsed request into backend args.
fn command_args(command: &str, payload: Vec<u8>) -> Vec<Value> {
    match command {
        "ECHO" if !payload.is_empty() => vec![Value::bytes(payload)],
        _ => vec![],
    }
}

/// One broker connection: parse the line protocol, route, and pump the
/// connection's outbox back out.
async fn serve_conn(
    stream: TcpStream,
    connection: u64,
    addr: SocketAddr,
    router: Arc<StdMutex<Router>>,
    metrics: Arc<NatsMetrics>,
    mut shutdown: watch::Receiver<bool>,
) {
    let _ = stream.set_nodelay(true);
    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);
    let (outbox_tx, mut outbox_rx) = mpsc::unbounded_channel::<Vec<u8>>();
    lock(&router).outboxes.insert(connection, outbox_tx);

    // Greet, then pump this connection's outbox on its own task.
    let info = info_line(addr);
    metrics
        .bytes_out
        .fetch_add(info.len() as u64, Ordering::Relaxed);
    if write_half.write_all(&info).await.is_err() {
        lock(&router).outboxes.remove(&connection);
        return;
    }
    let writer_metrics = Arc::clone(&metrics);
    let writer = tokio::spawn(async move {
        while let Some(frame) = outbox_rx.recv().await {
            writer_metrics
                .bytes_out
                .fetch_add(frame.len() as u64, Ordering::Relaxed);
            if write_half.write_all(&frame).await.is_err() {
                break;
            }
        }
    });

    let mut line = String::new();
    loop {
        line.clear();
        let read = tokio::select! {
            _ = shutdown.wait_for(|stop| *stop) => break,
            read = reader.read_line(&mut line) => read,
        };
        match read {
            Ok(0) | Err(_) => break,
            Ok(bytes) => metrics.bytes_in.fetch_add(bytes as u64, Ordering::Relaxed),
        };
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            continue;
        }
        let mut parts = trimmed.split_whitespace();
        let Some(verb) = parts.next() else { continue };
        let args: Vec<&str> = parts.collect();

        match verb.to_ascii_uppercase().as_str() {
            "CONNECT" => { /* accepted as-is; verbose mode is not exercised */ }
            "PING" => {
                if send(&router, connection, b"PONG\r\n".to_vec()).is_err() {
                    break;
                }
            }
            "PONG" => {}
            "SUB" => {
                // SUB <subject> [queue-group] <sid>
                let (Some(subject), Some(sid)) = (args.first(), args.last()) else {
                    break;
                };
                lock(&router).subscriptions.push(Subscription {
                    connection,
                    sid: (*sid).to_owned(),
                    subject: (*subject).to_owned(),
                });
            }
            "UNSUB" => {
                let Some(sid) = args.first() else { break };
                lock(&router)
                    .subscriptions
                    .retain(|s| !(s.connection == connection && s.sid == *sid));
            }
            "PUB" => {
                // PUB <subject> [reply-to] <#bytes>
                let Some(subject) = args.first().map(|s| (*s).to_owned()) else {
                    break;
                };
                let Some(len) = args.last().and_then(|n| n.parse::<usize>().ok()) else {
                    break;
                };
                if len > MAX_PAYLOAD {
                    break;
                }
                let reply_to = if args.len() == 3 {
                    Some(args[1].to_owned())
                } else {
                    None
                };
                // Payload is exactly `len` bytes, then CRLF.
                let mut payload = vec![0u8; len + 2];
                if reader.read_exact(&mut payload).await.is_err() {
                    break;
                }
                metrics
                    .bytes_in
                    .fetch_add(payload.len() as u64, Ordering::Relaxed);
                payload.truncate(len);
                metrics.routed.fetch_add(1, Ordering::Relaxed);
                lock(&router).publish(&subject, reply_to.as_deref(), &payload);
            }
            _ => {
                let _ = send(
                    &router,
                    connection,
                    b"-ERR 'Unknown Protocol Operation'\r\n".to_vec(),
                );
                break;
            }
        }
    }

    {
        let mut guard = lock(&router);
        guard.outboxes.remove(&connection);
        guard.subscriptions.retain(|s| s.connection != connection);
    }
    writer.abort();
}

/// Queue a frame on one connection's outbox.
fn send(router: &Arc<StdMutex<Router>>, connection: u64, frame: Vec<u8>) -> Result<(), ()> {
    let guard = lock(router);
    let Some(outbox) = guard.outboxes.get(&connection) else {
        return Err(());
    };
    outbox.send(frame).map_err(|_| ())
}

// ── Driver ──────────────────────────────────────────────────────────────────

/// Build the request payload the matrix asked for.
fn build_payload(command: &str, args: &[Value]) -> Result<Vec<u8>, String> {
    let mut out = command.as_bytes().to_vec();
    match args.first() {
        Some(Value::Str(s)) => {
            out.push(b' ');
            out.extend_from_slice(s.as_bytes());
        }
        Some(Value::Bytes(b)) => {
            out.push(b' ');
            out.extend_from_slice(b);
        }
        Some(other) => return Err(format!("nats lane: unsupported arg {other:?}")),
        None => {}
    }
    Ok(out)
}

/// Measure one matrix cell on the NATS lane.
///
/// `connections` maps to independent `async-nats` clients; `depth` to
/// concurrent outstanding requests on each. A NATS client multiplexes many
/// in-flight requests over one connection (correlated by inbox subject), so
/// the window is expressed as concurrent requests, as in the gRPC lane.
pub async fn cell(
    handle: &NatsHandle,
    spec: &CellSpec,
    cfg: &RunConfig,
) -> Result<Measured, String> {
    let addr = handle.local_addr();
    let payload = build_payload(spec.command, &spec.args)?;
    let mut clients = Vec::with_capacity(spec.connections);
    for _ in 0..spec.connections {
        clients.push(
            async_nats::connect(format!("nats://{addr}"))
                .await
                .map_err(|e| format!("nats connect failed: {e}"))?,
        );
    }

    if cfg.warmup > 0 {
        for client in &clients {
            nats_window(client, spec.depth, cfg.warmup, &payload).await?;
        }
    }

    let before_in = handle.bytes_in();
    let before_out = handle.bytes_out();
    let mut reps = Vec::with_capacity(cfg.repetitions);
    let mut ops = 0u64;
    for _ in 0..cfg.repetitions {
        let per_conn = (cfg.ops / clients.len().max(1)).max(spec.depth).max(1);
        let started = Instant::now();
        let mut handles = Vec::with_capacity(clients.len());
        for client in &clients {
            let client = client.clone();
            let payload = payload.clone();
            let depth = spec.depth;
            handles.push(tokio::spawn(async move {
                nats_window(&client, depth, per_conn, &payload).await
            }));
        }
        let mut all = Vec::with_capacity(per_conn * clients.len());
        for handle in handles {
            all.extend(
                handle
                    .await
                    .map_err(|e| format!("nats worker panicked: {e}"))??,
            );
        }
        ops += all.len() as u64;
        reps.push(compute(&mut all, started.elapsed()));
    }
    let after_in = handle.bytes_in();
    let after_out = handle.bytes_out();
    drop(clients);

    // Bytes are counted at the BROKER, across all four traversals of a round
    // trip — requester→broker, broker→responder, responder→broker,
    // broker→requester. So per-op byte counts here are not comparable with a
    // point-to-point lane's, and the module docs say why.
    let ops = ops.max(1) as f64;
    Ok((
        reps,
        (after_in - before_in) as f64 / ops,
        (after_out - before_out) as f64 / ops,
    ))
}

/// One continuously-full window of `depth` in-flight requests on one client.
async fn nats_window(
    client: &async_nats::Client,
    depth: usize,
    ops: usize,
    payload: &[u8],
) -> Result<Vec<Duration>, String> {
    let results: Vec<Result<Duration, String>> = futures::stream::iter(0..ops)
        .map(|_| {
            let client = client.clone();
            let payload = payload.to_vec();
            async move {
                let started = Instant::now();
                client
                    .request(REQUEST_SUBJECT, payload.into())
                    .await
                    .map_err(|e| format!("nats request failed: {e}"))?;
                Ok(started.elapsed())
            }
        })
        .buffer_unordered(depth.max(1))
        .collect()
        .await;
    results.into_iter().collect()
}

/// The connection-storm cell: connect + one request/reply round trip,
/// repeated.
pub async fn storm(
    handle: &NatsHandle,
    storms: usize,
    cfg: &RunConfig,
) -> Result<Measured, String> {
    let addr = handle.local_addr();
    let payload = build_payload("PING", &[])?;
    for _ in 0..cfg.warmup.min(storms) {
        storm_once(addr, &payload).await?;
    }
    let before_in = handle.bytes_in();
    let before_out = handle.bytes_out();
    let mut reps = Vec::with_capacity(cfg.repetitions);
    let mut ops = 0u64;
    for _ in 0..cfg.repetitions {
        let mut lats = Vec::with_capacity(storms);
        let started = Instant::now();
        for _ in 0..storms {
            lats.push(storm_once(addr, &payload).await?);
            ops += 1;
        }
        reps.push(compute(&mut lats, started.elapsed()));
    }
    let after_in = handle.bytes_in();
    let after_out = handle.bytes_out();

    let ops = ops.max(1) as f64;
    Ok((
        reps,
        (after_in - before_in) as f64 / ops,
        (after_out - before_out) as f64 / ops,
    ))
}

async fn storm_once(addr: SocketAddr, payload: &[u8]) -> Result<Duration, String> {
    let started = Instant::now();
    let client = async_nats::connect(format!("nats://{addr}"))
        .await
        .map_err(|e| format!("storm connect failed: {e}"))?;
    client
        .request(REQUEST_SUBJECT, payload.to_vec().into())
        .await
        .map_err(|e| format!("storm request failed: {e}"))?;
    Ok(started.elapsed())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::backend::STATIC_REPLY_BYTES;

    #[test]
    fn literal_subjects_match_exactly() {
        assert!(subject_matches("bench.call", "bench.call"));
        assert!(!subject_matches("bench.call", "bench.other"));
        assert!(!subject_matches("bench.call", "bench.call.extra"));
        assert!(!subject_matches("bench.call.extra", "bench.call"));
    }

    /// `async-nats` uses a `*`-terminated multiplexed inbox, so this is the
    /// wildcard the lane cannot work without.
    #[test]
    fn the_star_wildcard_matches_exactly_one_token() {
        assert!(subject_matches("_INBOX.abc.*", "_INBOX.abc.1"));
        assert!(!subject_matches("_INBOX.abc.*", "_INBOX.abc.1.2"));
        assert!(!subject_matches("_INBOX.abc.*", "_INBOX.abc"));
    }

    #[test]
    fn the_arrow_wildcard_matches_the_rest() {
        assert!(subject_matches("bench.>", "bench.call"));
        assert!(subject_matches("bench.>", "bench.call.deep"));
        assert!(!subject_matches("bench.>", "other.call"));
    }

    #[test]
    fn msg_frames_carry_the_reply_subject_when_present() {
        let framed = encode_msg("bench.call", "3", Some("_INBOX.x.1"), b"hello");
        assert_eq!(
            framed,
            b"MSG bench.call 3 _INBOX.x.1 5\r\nhello\r\n".to_vec()
        );
        let bare = encode_msg("bench.call", "3", None, b"hi");
        assert_eq!(bare, b"MSG bench.call 3 2\r\nhi\r\n".to_vec());
    }

    #[test]
    fn requests_split_into_command_and_payload() {
        let (command, payload) = split_request(b"ECHO hello");
        assert_eq!(command, "ECHO");
        assert_eq!(payload, b"hello");
        let (bare, empty) = split_request(b"PING");
        assert_eq!(bare, "PING");
        assert!(empty.is_empty());
    }

    #[test]
    fn the_responder_serves_the_shared_backend() {
        let backend = NoopBackend::new();
        let (command, payload) = split_request(b"STATIC");
        let value = backend
            .respond(&command, command_args(&command, payload))
            .unwrap();
        assert_eq!(value_to_bytes(value).len(), STATIC_REPLY_BYTES);
    }

    #[test]
    fn info_advertises_headers_off_so_clients_never_send_hpub() {
        let line = info_line("127.0.0.1:4222".parse().unwrap());
        let text = String::from_utf8(line).unwrap();
        assert!(text.starts_with("INFO {"));
        assert!(text.ends_with("}\r\n"));
        assert!(text.contains("\"headers\":false"));
        assert!(text.contains("\"proto\":1"));
    }
}

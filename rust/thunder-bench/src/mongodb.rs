//! Minimal MongoDB **OP_MSG** wire lane over the same no-op backend
//! (BEN-001, BEN-002) — same process, host, runtime and allocator as the
//! Thunder listener.
//!
//! # Why this lane
//!
//! MongoDB's modern wire is `OP_MSG` (opcode 2013): a 16-byte message header
//! whose first field is the total length (so, like Thunder, it is
//! length-prefixed and reads cleanly async), then a `uint32` flag word and one
//! or more sections, each a **BSON** document. It is the natural **codec**
//! comparison for the shootout: BSON is to MongoDB what MessagePack is to
//! Thunder — a self-describing binary document format — so this lane isolates
//! "how much does the document codec cost?" from the transport design. Like
//! RESP3/Bolt/Memcached it is FIFO per connection (ordered request/response).
//!
//! # Scope (honesty note, BEN-002)
//!
//! A **benchmark peer, not a MongoDB server**. One opcode (`OP_MSG`), one
//! section kind (0, body), and a two-field command document `{c, v}` — `c`
//! selects the backend mode (`ECHO`/`STATIC`/`SINK`/`PING`), `v` carries the
//! payload. Replies are `{ok: 1, r: <value>}`. No cursors, no checksum flag, no
//! auth, no real command catalog.
//!
//! **The BSON itself is the real thing**: documents are encoded and decoded by
//! the `bson` crate, not by a hand-rolled subset. That matters more here than
//! in any other lane — this lane exists to compare BSON against MessagePack,
//! so a homemade encoder would have made the codec under test a homemade
//! codec, and any result would have measured this file rather than BSON. Only
//! the OP_MSG framing around it is ours (no Rust MongoDB *server* exists to
//! borrow it from; the official crate is a client).
//!
//! BSON strings are UTF-8 by definition, so payloads must be valid UTF-8 —
//! the matrix's are ASCII by construction and [`build_mongodb_request`]
//! refuses anything else rather than mangling it.
//!
//! # Parity (BEN-003)
//!
//! The driver keeps a continuously-full in-flight window per connection (a
//! semaphore slot per outstanding request, replies matched FIFO), identical in
//! shape to the RESP3/Memcached drivers. Server-side bytes are counted at the
//! socket after the successful write, the same measurement point as every lane.

use std::collections::VecDeque;
use std::io;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use bson::{doc, Document};
use thunder::wire::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{watch, OwnedSemaphorePermit, Semaphore};

use crate::backend::NoopBackend;
use crate::driver::{CellSpec, Measured, RunConfig};
use crate::stats::compute;

/// Fixed MongoDB message-header size.
const HEADER_LEN: usize = 16;
/// The one opcode this peer models.
const OP_MSG: i32 = 2013;
/// Message cap — mirrors the Thunder frame cap (WIRE-020) so an oversized
/// length prefix cannot drive an unbounded allocation.
const MAX_MSG_LEN: usize = thunder::wire::DEFAULT_MAX_FRAME_BYTES;

/// Ride through a poisoned lock: the guarded state stays consistent.
fn lock<T>(mutex: &StdMutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

// ── BSON (the real codec — `bson` crate) ────────────────────────────────────

/// Encode `{ "c": <cmd>, "v": <payload> }` as one BSON document.
fn bson_command(cmd: &str, payload: &str) -> Result<Vec<u8>, String> {
    doc! { "c": cmd, "v": payload }
        .to_vec()
        .map_err(|e| format!("bson encode failed: {e}"))
}

/// Encode `{ "ok": 1, "r": <value> }` as one BSON document.
fn bson_reply(value: &str) -> Result<Vec<u8>, String> {
    doc! { "ok": 1i32, "r": value }
        .to_vec()
        .map_err(|e| format!("bson encode failed: {e}"))
}

/// Extract the `c` and `v` fields from a request document. A missing `v` is
/// an empty payload — the sentinels carry none.
fn parse_command(bytes: &[u8]) -> Option<(String, String)> {
    let document = Document::from_reader(bytes).ok()?;
    let cmd = document.get_str("c").ok()?.to_owned();
    let payload = document.get_str("v").unwrap_or_default().to_owned();
    Some((cmd, payload))
}

/// A backend reply value as the string carried in the reply's `r` field.
/// BSON strings are UTF-8 by definition, and the matrix's payloads are ASCII
/// by construction, so this is total for the traffic this lane carries.
fn value_to_string(value: Value) -> String {
    match value {
        Value::Str(s) => s,
        Value::Bytes(b) => String::from_utf8_lossy(&b).into_owned(),
        _ => String::new(),
    }
}

/// Frame a BSON body into an OP_MSG message (header + flags + section 0).
fn encode_op_msg(request_id: i32, response_to: i32, bson: &[u8]) -> Vec<u8> {
    // body = flagBits(4) + sectionKind(1) + bson
    let body_len = 4 + 1 + bson.len();
    let total = HEADER_LEN + body_len;
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(&(total as i32).to_le_bytes()); // messageLength
    out.extend_from_slice(&request_id.to_le_bytes());
    out.extend_from_slice(&response_to.to_le_bytes());
    out.extend_from_slice(&OP_MSG.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // flagBits
    out.push(0x00); // section kind 0 (body)
    out.extend_from_slice(bson);
    out
}

// ── Listener ────────────────────────────────────────────────────────────────

/// Server-side counters, sampled around a measured window.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MongoMetricsSnapshot {
    /// Requests answered.
    pub requests: u64,
    /// Request bytes read off the wire.
    pub bytes_in: u64,
    /// Response bytes written to the wire.
    pub bytes_out: u64,
}

#[derive(Debug, Default)]
struct MongoMetrics {
    requests: AtomicU64,
    bytes_in: AtomicU64,
    bytes_out: AtomicU64,
}

impl MongoMetrics {
    fn snapshot(&self) -> MongoMetricsSnapshot {
        MongoMetricsSnapshot {
            requests: self.requests.load(Ordering::Relaxed),
            bytes_in: self.bytes_in.load(Ordering::Relaxed),
            bytes_out: self.bytes_out.load(Ordering::Relaxed),
        }
    }
}

/// Handle to the running OP_MSG listener — same shape as the other lanes.
#[derive(Debug)]
pub struct MongoHandle {
    addr: SocketAddr,
    shutdown: watch::Sender<bool>,
    metrics: Arc<MongoMetrics>,
    done: Option<tokio::sync::mpsc::Receiver<()>>,
}

impl MongoHandle {
    /// The bound address.
    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    /// Current server-side counters.
    pub fn snapshot(&self) -> MongoMetricsSnapshot {
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

impl Drop for MongoHandle {
    fn drop(&mut self) {
        let _ = self.shutdown.send(true);
    }
}

/// Spawn the OP_MSG listener over the shared no-op backend.
pub async fn spawn_mongodb_listener(
    backend: Arc<NoopBackend>,
    addr: SocketAddr,
) -> io::Result<MongoHandle> {
    let listener = TcpListener::bind(addr).await?;
    let addr = listener.local_addr()?;
    let metrics = Arc::new(MongoMetrics::default());
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let (done_tx, done_rx) = tokio::sync::mpsc::channel::<()>(1);

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

    Ok(MongoHandle {
        addr,
        shutdown: shutdown_tx,
        metrics,
        done: Some(done_rx),
    })
}

/// One connection: read an OP_MSG request, answer via the backend, write the
/// reply. Drain-then-flush mirrors every other lane (SRV-006 analog).
async fn serve_conn(
    stream: TcpStream,
    backend: Arc<NoopBackend>,
    metrics: Arc<MongoMetrics>,
    mut shutdown: watch::Receiver<bool>,
) {
    let _ = stream.set_nodelay(true);
    let (read_half, write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);
    let mut writer = BufWriter::new(write_half);

    loop {
        let mut header = [0u8; HEADER_LEN];
        let read = tokio::select! {
            _ = shutdown.wait_for(|stop| *stop) => break,
            read = reader.read_exact(&mut header) => read,
        };
        if read.is_err() {
            break;
        }
        let total = i32::from_le_bytes([header[0], header[1], header[2], header[3]]) as usize;
        let request_id = i32::from_le_bytes([header[4], header[5], header[6], header[7]]);
        if !(HEADER_LEN..=MAX_MSG_LEN).contains(&total) {
            break;
        }
        let mut body = vec![0u8; total - HEADER_LEN];
        if reader.read_exact(&mut body).await.is_err() {
            break;
        }
        metrics.bytes_in.fetch_add(total as u64, Ordering::Relaxed);

        // body = flagBits(4) + sectionKind(1) + BSON document.
        let reply_bson = if body.len() < 5 || body[4] != 0x00 {
            bson_reply("")
        } else {
            match parse_command(&body[5..]) {
                Some((cmd, payload)) => {
                    let args = command_args(&cmd, payload);
                    match backend.respond(&cmd, args) {
                        Ok(value) => bson_reply(&value_to_string(value)),
                        Err(message) => bson_reply(&message),
                    }
                }
                None => bson_reply(""),
            }
        };
        let Ok(reply_bson) = reply_bson else { break };

        let frame = encode_op_msg(next_response_id(request_id), request_id, &reply_bson);
        if writer.write_all(&frame).await.is_err() {
            break;
        }
        metrics
            .bytes_out
            .fetch_add(frame.len() as u64, Ordering::Relaxed);
        metrics.requests.fetch_add(1, Ordering::Relaxed);

        if reader.buffer().is_empty() && writer.flush().await.is_err() {
            break;
        }
    }
    let _ = writer.flush().await;
}

/// The reply's requestID — a fresh id derived from the request (opaque to the
/// FIFO driver, which correlates by order).
fn next_response_id(request_id: i32) -> i32 {
    request_id.wrapping_add(1)
}

/// Turn a parsed `(cmd, payload)` into backend args: ECHO carries the payload,
/// the sentinels carry nothing.
fn command_args(cmd: &str, payload: String) -> Vec<Value> {
    match cmd {
        "ECHO" if !payload.is_empty() => vec![Value::Str(payload)],
        _ => vec![],
    }
}

// ── Driver ────────────────────────────────────────────────────────────────

/// One driver connection: a raw write half (direct writes, nodelay) and a
/// buffered read half.
struct MongoConn {
    reader: BufReader<OwnedReadHalf>,
    writer: OwnedWriteHalf,
}

impl MongoConn {
    async fn connect(addr: SocketAddr) -> io::Result<Self> {
        let stream = TcpStream::connect(addr).await?;
        stream.set_nodelay(true)?;
        let (read_half, write_half) = stream.into_split();
        Ok(Self {
            reader: BufReader::new(read_half),
            writer: write_half,
        })
    }
}

/// Read + fully consume one OP_MSG reply (the measurement point, BEN-003).
async fn read_reply(reader: &mut BufReader<OwnedReadHalf>) -> Result<(), String> {
    let mut header = [0u8; HEADER_LEN];
    reader
        .read_exact(&mut header)
        .await
        .map_err(|e| format!("mongodb read failed: {e}"))?;
    let total = i32::from_le_bytes([header[0], header[1], header[2], header[3]]) as usize;
    let op = i32::from_le_bytes([header[12], header[13], header[14], header[15]]);
    if op != OP_MSG {
        return Err(format!("mongodb bad opcode {op}"));
    }
    if !(HEADER_LEN..=MAX_MSG_LEN).contains(&total) {
        return Err(format!("mongodb message length {total} out of range"));
    }
    let mut body = vec![0u8; total - HEADER_LEN];
    reader
        .read_exact(&mut body)
        .await
        .map_err(|e| format!("mongodb body read failed: {e}"))?;
    Ok(())
}

/// Build one request frame from the matrix `(command, args)`.
fn build_mongodb_request(command: &str, args: &[Value]) -> Result<Vec<u8>, String> {
    let payload: String = match args.first() {
        Some(value) => value_text(value)?,
        None => String::new(),
    };
    let bson = bson_command(command, &payload)?;
    Ok(encode_op_msg(1, 0, &bson))
}

/// A matrix arg as the payload text. BSON strings are UTF-8, so a non-UTF-8
/// payload is refused rather than silently mangled — the matrix's payloads are
/// ASCII by construction.
fn value_text(value: &Value) -> Result<String, String> {
    match value {
        Value::Str(s) => Ok(s.clone()),
        Value::Bytes(b) => String::from_utf8(b.clone())
            .map_err(|_| "mongodb lane: BSON string payloads must be UTF-8".to_owned()),
        other => Err(format!("mongodb lane: unsupported arg {other:?}")),
    }
}

/// Measure one matrix cell on the MongoDB OP_MSG lane.
pub async fn cell(
    handle: &MongoHandle,
    spec: &CellSpec,
    cfg: &RunConfig,
) -> Result<Measured, String> {
    let addr = handle.local_addr();
    let request = Arc::new(build_mongodb_request(spec.command, &spec.args)?);
    let mut conns = Vec::with_capacity(spec.connections);
    for _ in 0..spec.connections {
        conns.push(
            MongoConn::connect(addr)
                .await
                .map_err(|e| format!("mongodb connect failed: {e}"))?,
        );
    }

    if cfg.warmup > 0 {
        let (warmed, _lats, _elapsed) =
            mongo_window(conns, spec.depth, cfg.warmup, &request).await?;
        conns = warmed;
    }
    let before = handle.snapshot();
    let mut reps = Vec::with_capacity(cfg.repetitions);
    for _ in 0..cfg.repetitions {
        let (returned, mut lats, elapsed) =
            mongo_window(conns, spec.depth, cfg.ops, &request).await?;
        conns = returned;
        reps.push(compute(&mut lats, elapsed));
    }
    let after = handle.snapshot();
    drop(conns);

    let ops = (after.requests - before.requests).max(1) as f64;
    Ok((
        reps,
        (after.bytes_in - before.bytes_in) as f64 / ops,
        (after.bytes_out - before.bytes_out) as f64 / ops,
    ))
}

/// One continuously-full window across all connections.
async fn mongo_window(
    conns: Vec<MongoConn>,
    depth: usize,
    total_ops: usize,
    request: &Arc<Vec<u8>>,
) -> Result<(Vec<MongoConn>, Vec<Duration>, Duration), String> {
    let per_conn = (total_ops / conns.len().max(1)).max(depth).max(1);
    let started = Instant::now();
    let mut handles = Vec::with_capacity(conns.len());
    for conn in conns {
        let request = Arc::clone(request);
        handles.push(tokio::spawn(mongo_conn_window(
            conn, depth, per_conn, request,
        )));
    }
    let mut returned = Vec::with_capacity(handles.len());
    let mut all = Vec::with_capacity(per_conn * handles.len());
    for handle in handles {
        let (conn, lats) = handle
            .await
            .map_err(|e| format!("mongodb worker panicked: {e}"))??;
        returned.push(conn);
        all.extend(lats);
    }
    Ok((returned, all, started.elapsed()))
}

/// FIFO pipeline window on one connection (BEN-003), same shape as the RESP3
/// and Memcached drivers.
async fn mongo_conn_window(
    mut conn: MongoConn,
    depth: usize,
    ops: usize,
    request: Arc<Vec<u8>>,
) -> Result<(MongoConn, Vec<Duration>), String> {
    let window = Arc::new(Semaphore::new(depth.max(1)));
    let pending: Arc<StdMutex<VecDeque<(Instant, OwnedSemaphorePermit)>>> =
        Arc::new(StdMutex::new(VecDeque::with_capacity(depth.max(1))));
    let writer = &mut conn.writer;
    let reader = &mut conn.reader;

    let sender = {
        let window = Arc::clone(&window);
        let pending = Arc::clone(&pending);
        async move {
            for _ in 0..ops {
                let permit = Arc::clone(&window)
                    .acquire_owned()
                    .await
                    .map_err(|_| "pipeline window closed".to_owned())?;
                lock(&pending).push_back((Instant::now(), permit));
                writer
                    .write_all(&request)
                    .await
                    .map_err(|e| format!("mongodb write failed: {e}"))?;
            }
            Ok::<(), String>(())
        }
    };
    let receiver = {
        let pending = Arc::clone(&pending);
        async move {
            let mut lats = Vec::with_capacity(ops);
            for _ in 0..ops {
                read_reply(reader).await?;
                let (sent, permit) = lock(&pending)
                    .pop_front()
                    .ok_or_else(|| "mongodb reply without a pending request".to_owned())?;
                lats.push(sent.elapsed());
                drop(permit);
            }
            Ok::<Vec<Duration>, String>(lats)
        }
    };

    let (sent, received) = tokio::join!(sender, receiver);
    sent?;
    let lats = received?;
    Ok((conn, lats))
}

/// The connection-storm cell: connect + one request + first reply, repeated.
pub async fn storm(
    handle: &MongoHandle,
    storms: usize,
    cfg: &RunConfig,
) -> Result<Measured, String> {
    let addr = handle.local_addr();
    let request = build_mongodb_request("PING", &[])?;
    for _ in 0..cfg.warmup.min(storms) {
        mongo_storm_once(addr, &request).await?;
    }
    let before = handle.snapshot();
    let mut reps = Vec::with_capacity(cfg.repetitions);
    for _ in 0..cfg.repetitions {
        let mut lats = Vec::with_capacity(storms);
        let started = Instant::now();
        for _ in 0..storms {
            lats.push(mongo_storm_once(addr, &request).await?);
        }
        reps.push(compute(&mut lats, started.elapsed()));
    }
    let after = handle.snapshot();
    let ops = (after.requests - before.requests).max(1) as f64;
    Ok((
        reps,
        (after.bytes_in - before.bytes_in) as f64 / ops,
        (after.bytes_out - before.bytes_out) as f64 / ops,
    ))
}

async fn mongo_storm_once(addr: SocketAddr, request: &[u8]) -> Result<Duration, String> {
    let started = Instant::now();
    let mut conn = MongoConn::connect(addr)
        .await
        .map_err(|e| format!("storm connect failed: {e}"))?;
    conn.writer
        .write_all(request)
        .await
        .map_err(|e| format!("storm write failed: {e}"))?;
    read_reply(&mut conn.reader).await?;
    Ok(started.elapsed())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::backend::STATIC_REPLY_BYTES;

    #[test]
    fn command_doc_round_trips() {
        let doc = bson_command("ECHO", "hello world").unwrap();
        let (cmd, payload) = parse_command(&doc).unwrap();
        assert_eq!(cmd, "ECHO");
        assert_eq!(payload, "hello world");
    }

    #[test]
    fn empty_payload_round_trips() {
        let doc = bson_command("PING", "").unwrap();
        let (cmd, payload) = parse_command(&doc).unwrap();
        assert_eq!(cmd, "PING");
        assert!(payload.is_empty());
    }

    #[test]
    fn bson_length_prefix_is_the_whole_document() {
        let doc = bson_command("ECHO", "xyz").unwrap();
        let declared = i32::from_le_bytes([doc[0], doc[1], doc[2], doc[3]]) as usize;
        assert_eq!(declared, doc.len(), "BSON length prefix must cover the doc");
        assert_eq!(*doc.last().unwrap(), 0, "doc ends with the terminator");
    }

    /// Read the `r` string field out of a reply document.
    fn reply_r(bytes: &[u8]) -> String {
        let document = Document::from_reader(bytes).unwrap();
        assert_eq!(document.get_i32("ok").unwrap(), 1, "replies carry ok:1");
        document.get_str("r").unwrap().to_owned()
    }

    #[test]
    fn echo_reply_carries_the_payload() {
        let request = bson_command("ECHO", &"x".repeat(64)).unwrap();
        let (cmd, payload) = parse_command(&request).unwrap();
        let backend = NoopBackend::new();
        let value = backend.respond(&cmd, command_args(&cmd, payload)).unwrap();
        let reply = bson_reply(&value_to_string(value)).unwrap();
        assert_eq!(reply_r(&reply), "x".repeat(64));
    }

    #[test]
    fn static_reply_is_4kib() {
        let request = bson_command("STATIC", "").unwrap();
        let (cmd, payload) = parse_command(&request).unwrap();
        let backend = NoopBackend::new();
        let value = backend.respond(&cmd, command_args(&cmd, payload)).unwrap();
        let reply = bson_reply(&value_to_string(value)).unwrap();
        assert_eq!(reply_r(&reply).len(), STATIC_REPLY_BYTES);
    }

    #[test]
    fn non_utf8_payloads_are_refused_not_mangled() {
        let err = build_mongodb_request("ECHO", &[Value::Bytes(vec![0xff, 0xfe])]).unwrap_err();
        assert!(err.contains("UTF-8"), "unexpected error: {err}");
    }

    #[test]
    fn op_msg_length_prefix_covers_the_frame() {
        let frame = build_mongodb_request("ECHO", &[Value::Str("hi".to_owned())]).unwrap();
        let total = i32::from_le_bytes([frame[0], frame[1], frame[2], frame[3]]) as usize;
        assert_eq!(total, frame.len());
        let op = i32::from_le_bytes([frame[12], frame[13], frame[14], frame[15]]);
        assert_eq!(op, OP_MSG);
    }
}

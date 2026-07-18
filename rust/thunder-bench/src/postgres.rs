//! Minimal **PostgreSQL v3** frontend/backend wire lane over the same no-op
//! backend (BEN-001, BEN-002) — same process, host, runtime and allocator as
//! the Thunder listener.
//!
//! # Why this lane
//!
//! PostgreSQL's v3 protocol is the most heavily-optimized, longest-lived
//! binary DB wire in the field: three decades of production tuning behind a
//! deliberately plain shape. Every message after startup is
//! `type(1) + length(int32) + body`, which makes it **typed and
//! length-prefixed** — the same async-friendly framing Thunder uses — and,
//! like RESP3/Bolt/Memcached/OP_MSG, strictly FIFO per connection. What it
//! adds over the other reference lanes is a **multi-message response cycle**:
//! one query is answered by four messages (`RowDescription`, `DataRow`,
//! `CommandComplete`, `ReadyForQuery`), so this lane measures what
//! per-response *structure* costs a mature wire, not just per-byte codec
//! cost.
//!
//! # Scope (honesty note, BEN-002)
//!
//! A **benchmark peer, not a PostgreSQL server**. Startup is
//! `StartupMessage` → `AuthenticationOk` → `ReadyForQuery` (no auth methods,
//! no `ParameterStatus`, no `BackendKeyData` — none are on the measured
//! path). The measured path is the **simple query** message (`Q`), whose text
//! is `"<CMD> <payload>"`: the first token selects the backend mode
//! (`ECHO`/`STATIC`/`SINK`/`PING`), the rest is the payload. The reply is a
//! one-column, one-row result in text format. No SQL parser, no catalog, no
//! types beyond `text`, no cursors, no COPY, no cancellation.
//!
//! **The extended query protocol is deliberately out of scope.** `Parse` /
//! `Bind` / `Describe` / `Execute` / `Sync` spends five client messages per
//! operation; measuring it against lanes that spend one would compare
//! protocol chattiness, not transport, and break the one-request/one-reply
//! parity every other lane holds (BEN-003). The simple query is the faithful
//! single-round-trip shape, so that is what the matrix drives.
//!
//! Query text is a NUL-terminated cstring, so payloads must be NUL-free; the
//! matrix's payloads are `"x".repeat(n)` and [`build_postgres_request`]
//! rejects anything else rather than silently truncating.
//!
//! # Parity (BEN-003)
//!
//! The driver keeps a continuously-full in-flight window per connection (a
//! semaphore slot per outstanding request, replies matched FIFO), identical
//! in shape to the RESP3/Memcached/MongoDB drivers, and consumes the whole
//! response cycle through `ReadyForQuery` before counting a reply. Startup
//! happens in `connect`, before any measured window — the connection-storm
//! cell is the exception, where setup *is* the thing measured. Server-side
//! bytes are counted at the socket after the successful write, the same
//! measurement point as every lane.

use std::collections::VecDeque;
use std::io;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use thunder::wire::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{watch, OwnedSemaphorePermit, Semaphore};

use crate::backend::NoopBackend;
use crate::driver::{CellSpec, Measured, RunConfig};
use crate::stats::compute;

/// Protocol version 3.0, as the startup message encodes it (`3 << 16`).
const PROTOCOL_V3: i32 = 196_608;
/// Message cap — mirrors the Thunder frame cap (WIRE-020) so an oversized
/// length prefix cannot drive an unbounded allocation.
const MAX_MSG_LEN: usize = thunder::wire::DEFAULT_MAX_FRAME_BYTES;
/// `text` type OID — the one column type this peer describes.
const TEXT_OID: i32 = 25;

// Frontend message types.
/// Simple query.
const MSG_QUERY: u8 = b'Q';
/// Terminate.
const MSG_TERMINATE: u8 = b'X';

// Backend message types.
/// Authentication request (`Ok` when the payload is 0).
const MSG_AUTHENTICATION: u8 = b'R';
/// Row description.
const MSG_ROW_DESCRIPTION: u8 = b'T';
/// One data row.
const MSG_DATA_ROW: u8 = b'D';
/// Command complete.
const MSG_COMMAND_COMPLETE: u8 = b'C';
/// Ready for query — the end of a response cycle.
const MSG_READY_FOR_QUERY: u8 = b'Z';

/// Ride through a poisoned lock: the guarded state stays consistent.
fn lock<T>(mutex: &StdMutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

// ── Message encoding ────────────────────────────────────────────────────────

/// Frame a typed backend/frontend message: `type(1) + length(int32, counting
/// itself) + body`.
fn encode_message(kind: u8, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len() + 5);
    out.push(kind);
    // PostgreSQL is big-endian on the wire, throughout.
    out.extend_from_slice(&((body.len() + 4) as i32).to_be_bytes());
    out.extend_from_slice(body);
    out
}

/// The untyped `StartupMessage`: `length(int32) + version(int32) +
/// key\0value\0… + \0`.
fn encode_startup() -> Vec<u8> {
    let mut params = Vec::with_capacity(32);
    params.extend_from_slice(b"user\0bench\0");
    params.extend_from_slice(b"database\0bench\0");
    params.push(0); // terminator for the parameter list
    let total = 4 + 4 + params.len();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(&(total as i32).to_be_bytes());
    out.extend_from_slice(&PROTOCOL_V3.to_be_bytes());
    out.extend_from_slice(&params);
    out
}

/// `AuthenticationOk` — the int32 `0` sub-code.
fn encode_authentication_ok() -> Vec<u8> {
    encode_message(MSG_AUTHENTICATION, &0i32.to_be_bytes())
}

/// `ReadyForQuery` with the idle transaction status.
fn encode_ready_for_query() -> Vec<u8> {
    encode_message(MSG_READY_FOR_QUERY, b"I")
}

/// A simple query message carrying `"<CMD> <payload>"` (or just `"<CMD>"`
/// when the payload is empty).
fn encode_query(command: &str, payload: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(command.len() + payload.len() + 2);
    body.extend_from_slice(command.as_bytes());
    if !payload.is_empty() {
        body.push(b' ');
        body.extend_from_slice(payload);
    }
    body.push(0); // cstring terminator
    encode_message(MSG_QUERY, &body)
}

/// `RowDescription` for the single `r` text column this peer returns.
fn encode_row_description() -> Vec<u8> {
    let mut body = Vec::with_capacity(32);
    body.extend_from_slice(&1i16.to_be_bytes()); // one field
    body.extend_from_slice(b"r\0"); // field name
    body.extend_from_slice(&0i32.to_be_bytes()); // table OID — not from a table
    body.extend_from_slice(&0i16.to_be_bytes()); // column attribute number
    body.extend_from_slice(&TEXT_OID.to_be_bytes());
    body.extend_from_slice(&(-1i16).to_be_bytes()); // variable type length
    body.extend_from_slice(&(-1i32).to_be_bytes()); // no type modifier
    body.extend_from_slice(&0i16.to_be_bytes()); // text format
    encode_message(MSG_ROW_DESCRIPTION, &body)
}

/// `DataRow` with one column carrying `value`.
fn encode_data_row(value: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(value.len() + 6);
    body.extend_from_slice(&1i16.to_be_bytes()); // one column
    body.extend_from_slice(&(value.len() as i32).to_be_bytes());
    body.extend_from_slice(value);
    encode_message(MSG_DATA_ROW, &body)
}

/// `CommandComplete` for a one-row select.
fn encode_command_complete() -> Vec<u8> {
    encode_message(MSG_COMMAND_COMPLETE, b"SELECT 1\0")
}

/// The whole response cycle for one query, as a single write (the analog of
/// every other lane emitting one reply frame).
fn encode_response_cycle(value: &[u8]) -> Vec<u8> {
    let mut out = encode_row_description();
    out.extend_from_slice(&encode_data_row(value));
    out.extend_from_slice(&encode_command_complete());
    out.extend_from_slice(&encode_ready_for_query());
    out
}

/// Split a query's text into `(command, payload)` at the first space.
fn parse_query(text: &[u8]) -> (String, Vec<u8>) {
    match text.iter().position(|byte| *byte == b' ') {
        Some(at) => (
            String::from_utf8_lossy(&text[..at]).into_owned(),
            text[at + 1..].to_vec(),
        ),
        None => (String::from_utf8_lossy(text).into_owned(), Vec::new()),
    }
}

/// A backend reply value as the bytes carried in the row's one column.
fn value_to_bytes(value: Value) -> Vec<u8> {
    match value {
        Value::Str(s) => s.into_bytes(),
        Value::Bytes(b) => b,
        _ => Vec::new(),
    }
}

/// Turn a parsed `(cmd, payload)` into backend args: ECHO carries the
/// payload, the sentinels carry nothing.
fn command_args(cmd: &str, payload: Vec<u8>) -> Vec<Value> {
    match cmd {
        "ECHO" if !payload.is_empty() => vec![Value::Bytes(payload)],
        _ => vec![],
    }
}

// ── Listener ────────────────────────────────────────────────────────────────

/// Server-side counters, sampled around a measured window.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PgMetricsSnapshot {
    /// Queries answered.
    pub requests: u64,
    /// Request bytes read off the wire.
    pub bytes_in: u64,
    /// Response bytes written to the wire.
    pub bytes_out: u64,
}

#[derive(Debug, Default)]
struct PgMetrics {
    requests: AtomicU64,
    bytes_in: AtomicU64,
    bytes_out: AtomicU64,
}

impl PgMetrics {
    fn snapshot(&self) -> PgMetricsSnapshot {
        PgMetricsSnapshot {
            requests: self.requests.load(Ordering::Relaxed),
            bytes_in: self.bytes_in.load(Ordering::Relaxed),
            bytes_out: self.bytes_out.load(Ordering::Relaxed),
        }
    }
}

/// Handle to the running PostgreSQL v3 listener — same shape as the other
/// lanes.
#[derive(Debug)]
pub struct PgHandle {
    addr: SocketAddr,
    shutdown: watch::Sender<bool>,
    metrics: Arc<PgMetrics>,
    done: Option<tokio::sync::mpsc::Receiver<()>>,
}

impl PgHandle {
    /// The bound address.
    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    /// Current server-side counters.
    pub fn snapshot(&self) -> PgMetricsSnapshot {
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

impl Drop for PgHandle {
    fn drop(&mut self) {
        let _ = self.shutdown.send(true);
    }
}

/// Spawn the PostgreSQL v3 listener over the shared no-op backend.
pub async fn spawn_postgres_listener(
    backend: Arc<NoopBackend>,
    addr: SocketAddr,
) -> io::Result<PgHandle> {
    let listener = TcpListener::bind(addr).await?;
    let addr = listener.local_addr()?;
    let metrics = Arc::new(PgMetrics::default());
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

    Ok(PgHandle {
        addr,
        shutdown: shutdown_tx,
        metrics,
        done: Some(done_rx),
    })
}

/// One connection: startup, then a query → response-cycle loop.
/// Drain-then-flush mirrors every other lane (SRV-006 analog).
async fn serve_conn(
    stream: TcpStream,
    backend: Arc<NoopBackend>,
    metrics: Arc<PgMetrics>,
    mut shutdown: watch::Receiver<bool>,
) {
    let _ = stream.set_nodelay(true);
    let (read_half, write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);
    let mut writer = BufWriter::new(write_half);

    if server_startup(&mut reader, &mut writer, &metrics)
        .await
        .is_err()
    {
        return;
    }

    loop {
        let mut header = [0u8; 5];
        let read = tokio::select! {
            _ = shutdown.wait_for(|stop| *stop) => break,
            read = reader.read_exact(&mut header) => read,
        };
        if read.is_err() {
            break;
        }
        let kind = header[0];
        let len = i32::from_be_bytes([header[1], header[2], header[3], header[4]]) as usize;
        if !(4..=MAX_MSG_LEN).contains(&len) {
            break;
        }
        let mut body = vec![0u8; len - 4];
        if reader.read_exact(&mut body).await.is_err() {
            break;
        }
        metrics
            .bytes_in
            .fetch_add((len + 1) as u64, Ordering::Relaxed);
        if kind == MSG_TERMINATE {
            break;
        }
        if kind != MSG_QUERY {
            break;
        }

        // The query text is a cstring; drop the terminator before parsing.
        let text = body.strip_suffix(b"\0").unwrap_or(&body);
        let (cmd, payload) = parse_query(text);
        let reply = match backend.respond(&cmd, command_args(&cmd, payload)) {
            Ok(value) => value_to_bytes(value),
            Err(message) => message.into_bytes(),
        };
        let frame = encode_response_cycle(&reply);
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

/// Server side of startup: read the untyped `StartupMessage`, answer
/// `AuthenticationOk` + `ReadyForQuery`. Errors (and closes) on a bad length
/// or an unknown protocol version.
async fn server_startup(
    reader: &mut BufReader<OwnedReadHalf>,
    writer: &mut BufWriter<OwnedWriteHalf>,
    metrics: &PgMetrics,
) -> io::Result<()> {
    let mut len_bytes = [0u8; 4];
    reader.read_exact(&mut len_bytes).await?;
    let len = i32::from_be_bytes(len_bytes) as usize;
    if !(8..=MAX_MSG_LEN).contains(&len) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "startup length out of range",
        ));
    }
    let mut body = vec![0u8; len - 4];
    reader.read_exact(&mut body).await?;
    let version = i32::from_be_bytes([body[0], body[1], body[2], body[3]]);
    if version != PROTOCOL_V3 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unsupported protocol version",
        ));
    }
    metrics.bytes_in.fetch_add(len as u64, Ordering::Relaxed);

    let mut reply = encode_authentication_ok();
    reply.extend_from_slice(&encode_ready_for_query());
    writer.write_all(&reply).await?;
    writer.flush().await?;
    metrics
        .bytes_out
        .fetch_add(reply.len() as u64, Ordering::Relaxed);
    Ok(())
}

// ── Driver ────────────────────────────────────────────────────────────────

/// One driver connection, started up and ready for queries: a raw write half
/// (direct writes, nodelay) and a buffered read half.
struct PgConn {
    reader: BufReader<OwnedReadHalf>,
    writer: OwnedWriteHalf,
}

impl PgConn {
    /// Dial and complete startup. Everything here is session setup: the cell
    /// driver calls it *before* any measurement so startup never lands inside
    /// a measured window (the storm scenario is the exception — there setup is
    /// the thing measured).
    async fn connect(addr: SocketAddr) -> Result<Self, String> {
        let stream = TcpStream::connect(addr)
            .await
            .map_err(|e| format!("postgres connect failed: {e}"))?;
        stream
            .set_nodelay(true)
            .map_err(|e| format!("postgres nodelay failed: {e}"))?;
        let (read_half, write_half) = stream.into_split();
        let mut conn = Self {
            reader: BufReader::new(read_half),
            writer: write_half,
        };
        conn.writer
            .write_all(&encode_startup())
            .await
            .map_err(|e| format!("postgres startup write failed: {e}"))?;
        read_until_ready(&mut conn.reader).await?;
        Ok(conn)
    }
}

/// Read and fully consume backend messages up to and including
/// `ReadyForQuery` — one whole response cycle, the measurement point
/// (BEN-003).
async fn read_until_ready(reader: &mut BufReader<OwnedReadHalf>) -> Result<(), String> {
    loop {
        let mut header = [0u8; 5];
        reader
            .read_exact(&mut header)
            .await
            .map_err(|e| format!("postgres read failed: {e}"))?;
        let kind = header[0];
        let len = i32::from_be_bytes([header[1], header[2], header[3], header[4]]) as usize;
        if !(4..=MAX_MSG_LEN).contains(&len) {
            return Err(format!("postgres message length {len} out of range"));
        }
        let mut body = vec![0u8; len - 4];
        reader
            .read_exact(&mut body)
            .await
            .map_err(|e| format!("postgres body read failed: {e}"))?;
        match kind {
            MSG_READY_FOR_QUERY => return Ok(()),
            MSG_AUTHENTICATION | MSG_ROW_DESCRIPTION | MSG_DATA_ROW | MSG_COMMAND_COMPLETE => {
                continue
            }
            other => return Err(format!("postgres unexpected message '{}'", other as char)),
        }
    }
}

/// Build one query message from the matrix `(command, args)`.
fn build_postgres_request(command: &str, args: &[Value]) -> Result<Vec<u8>, String> {
    let payload: Vec<u8> = match args.first() {
        Some(value) => value_bytes(value)?,
        None => Vec::new(),
    };
    // The query is a cstring: a NUL in the payload would truncate it, so
    // refuse rather than measure a shorter request than the matrix asked for.
    if payload.contains(&0) {
        return Err("postgres lane: query payloads must be NUL-free".to_owned());
    }
    Ok(encode_query(command, &payload))
}

/// A matrix arg as raw payload bytes.
fn value_bytes(value: &Value) -> Result<Vec<u8>, String> {
    match value {
        Value::Str(s) => Ok(s.clone().into_bytes()),
        Value::Bytes(b) => Ok(b.clone()),
        other => Err(format!("postgres lane: unsupported arg {other:?}")),
    }
}

/// Measure one matrix cell on the PostgreSQL v3 lane.
pub async fn cell(handle: &PgHandle, spec: &CellSpec, cfg: &RunConfig) -> Result<Measured, String> {
    let addr = handle.local_addr();
    let request = Arc::new(build_postgres_request(spec.command, &spec.args)?);
    let mut conns = Vec::with_capacity(spec.connections);
    for _ in 0..spec.connections {
        conns.push(PgConn::connect(addr).await?);
    }

    if cfg.warmup > 0 {
        let (warmed, _lats, _elapsed) = pg_window(conns, spec.depth, cfg.warmup, &request).await?;
        conns = warmed;
    }
    let before = handle.snapshot();
    let mut reps = Vec::with_capacity(cfg.repetitions);
    for _ in 0..cfg.repetitions {
        let (returned, mut lats, elapsed) = pg_window(conns, spec.depth, cfg.ops, &request).await?;
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
async fn pg_window(
    conns: Vec<PgConn>,
    depth: usize,
    total_ops: usize,
    request: &Arc<Vec<u8>>,
) -> Result<(Vec<PgConn>, Vec<Duration>, Duration), String> {
    let per_conn = (total_ops / conns.len().max(1)).max(depth).max(1);
    let started = Instant::now();
    let mut handles = Vec::with_capacity(conns.len());
    for conn in conns {
        let request = Arc::clone(request);
        handles.push(tokio::spawn(pg_conn_window(conn, depth, per_conn, request)));
    }
    let mut returned = Vec::with_capacity(handles.len());
    let mut all = Vec::with_capacity(per_conn * handles.len());
    for handle in handles {
        let (conn, lats) = handle
            .await
            .map_err(|e| format!("postgres worker panicked: {e}"))??;
        returned.push(conn);
        all.extend(lats);
    }
    Ok((returned, all, started.elapsed()))
}

/// FIFO pipeline window on one connection (BEN-003), same shape as the RESP3,
/// Memcached and MongoDB drivers.
async fn pg_conn_window(
    mut conn: PgConn,
    depth: usize,
    ops: usize,
    request: Arc<Vec<u8>>,
) -> Result<(PgConn, Vec<Duration>), String> {
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
                    .map_err(|e| format!("postgres write failed: {e}"))?;
            }
            Ok::<(), String>(())
        }
    };
    let receiver = {
        let pending = Arc::clone(&pending);
        async move {
            let mut lats = Vec::with_capacity(ops);
            for _ in 0..ops {
                read_until_ready(reader).await?;
                let (sent, permit) = lock(&pending)
                    .pop_front()
                    .ok_or_else(|| "postgres reply without a pending request".to_owned())?;
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

/// The connection-storm cell: connect + startup + one query + its response
/// cycle, repeated.
pub async fn storm(handle: &PgHandle, storms: usize, cfg: &RunConfig) -> Result<Measured, String> {
    let addr = handle.local_addr();
    let request = build_postgres_request("PING", &[])?;
    for _ in 0..cfg.warmup.min(storms) {
        pg_storm_once(addr, &request).await?;
    }
    let before = handle.snapshot();
    let mut reps = Vec::with_capacity(cfg.repetitions);
    for _ in 0..cfg.repetitions {
        let mut lats = Vec::with_capacity(storms);
        let started = Instant::now();
        for _ in 0..storms {
            lats.push(pg_storm_once(addr, &request).await?);
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

async fn pg_storm_once(addr: SocketAddr, request: &[u8]) -> Result<Duration, String> {
    let started = Instant::now();
    let mut conn = PgConn::connect(addr).await?;
    conn.writer
        .write_all(request)
        .await
        .map_err(|e| format!("storm write failed: {e}"))?;
    read_until_ready(&mut conn.reader).await?;
    Ok(started.elapsed())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::backend::STATIC_REPLY_BYTES;

    /// Walk a stream of typed messages, returning `(kind, body)` pairs.
    fn split_messages(mut bytes: &[u8]) -> Vec<(u8, Vec<u8>)> {
        let mut out = Vec::new();
        while bytes.len() >= 5 {
            let kind = bytes[0];
            let len = i32::from_be_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]) as usize;
            let body = bytes[5..5 + len - 4].to_vec();
            out.push((kind, body));
            bytes = &bytes[1 + len..];
        }
        out
    }

    /// The one column's bytes out of a `DataRow` body.
    fn data_row_value(body: &[u8]) -> Vec<u8> {
        let len = i32::from_be_bytes([body[2], body[3], body[4], body[5]]) as usize;
        body[6..6 + len].to_vec()
    }

    #[test]
    fn query_round_trips() {
        let frame = encode_query("ECHO", b"hello world");
        let messages = split_messages(&frame);
        assert_eq!(messages.len(), 1);
        let (kind, body) = &messages[0];
        assert_eq!(*kind, MSG_QUERY);
        let (cmd, payload) = parse_query(body.strip_suffix(b"\0").unwrap());
        assert_eq!(cmd, "ECHO");
        assert_eq!(payload, b"hello world");
    }

    #[test]
    fn bare_command_round_trips_without_a_payload() {
        let frame = encode_query("PING", b"");
        let (_, body) = split_messages(&frame).remove(0);
        let (cmd, payload) = parse_query(body.strip_suffix(b"\0").unwrap());
        assert_eq!(cmd, "PING");
        assert!(payload.is_empty());
    }

    #[test]
    fn message_length_counts_itself_but_not_the_type_byte() {
        let frame = encode_query("ECHO", b"xyz");
        let declared = i32::from_be_bytes([frame[1], frame[2], frame[3], frame[4]]) as usize;
        assert_eq!(declared, frame.len() - 1, "length excludes the type byte");
    }

    #[test]
    fn startup_declares_protocol_v3_and_its_own_length() {
        let startup = encode_startup();
        let declared =
            i32::from_be_bytes([startup[0], startup[1], startup[2], startup[3]]) as usize;
        assert_eq!(declared, startup.len());
        let version = i32::from_be_bytes([startup[4], startup[5], startup[6], startup[7]]);
        assert_eq!(version, PROTOCOL_V3);
        assert_eq!(*startup.last().unwrap(), 0, "parameter list is terminated");
    }

    #[test]
    fn response_cycle_is_description_row_complete_ready() {
        let kinds: Vec<u8> = split_messages(&encode_response_cycle(b"hi"))
            .into_iter()
            .map(|(kind, _)| kind)
            .collect();
        assert_eq!(
            kinds,
            vec![
                MSG_ROW_DESCRIPTION,
                MSG_DATA_ROW,
                MSG_COMMAND_COMPLETE,
                MSG_READY_FOR_QUERY,
            ]
        );
    }

    #[test]
    fn echo_data_row_carries_the_payload() {
        let (cmd, payload) = parse_query(b"ECHO xxxxxxxx");
        let backend = NoopBackend::new();
        let value = backend.respond(&cmd, command_args(&cmd, payload)).unwrap();
        let cycle = encode_response_cycle(&value_to_bytes(value));
        let (_, body) = split_messages(&cycle).remove(1);
        assert_eq!(data_row_value(&body), b"xxxxxxxx");
    }

    #[test]
    fn static_data_row_is_4kib() {
        let (cmd, payload) = parse_query(b"STATIC");
        let backend = NoopBackend::new();
        let value = backend.respond(&cmd, command_args(&cmd, payload)).unwrap();
        let cycle = encode_response_cycle(&value_to_bytes(value));
        let (_, body) = split_messages(&cycle).remove(1);
        assert_eq!(data_row_value(&body).len(), STATIC_REPLY_BYTES);
    }

    #[test]
    fn nul_payloads_are_refused_not_truncated() {
        let err = build_postgres_request("ECHO", &[Value::Bytes(vec![b'a', 0, b'b'])]).unwrap_err();
        assert!(err.contains("NUL-free"), "unexpected error: {err}");
    }
}

//! **PostgreSQL v3** wire lane — the real `pgwire` server over the same no-op
//! backend (BEN-001, BEN-002), in the same process, host, runtime and
//! allocator as the Thunder listener.
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
//! # The server is real; the driver is ours
//!
//! The **server side is the `pgwire` crate** — production protocol code, not a
//! subset written for this harness. That removes the worst bias in a shootout:
//! a peer that loses because the benchmark author implemented it badly. All
//! this file supplies server-side is a [`SimpleQueryHandler`] that forwards to
//! the shared [`NoopBackend`], so the engine stays out of the measurement
//! (BEN-001) exactly as it does for every other lane.
//!
//! The **driver** stays ours, as it is in every lane: BEN-003 parity demands
//! one concurrency model and one measurement point across all of them, which
//! no protocol-specific client library would honour. The thing under
//! measurement is the server.
//!
//! # Two consequences worth stating plainly
//!
//! **Bytes are counted at the driver, not the listener.** `process_socket`
//! takes a concrete `TcpStream`, so a counting wrapper cannot be interposed
//! without reimplementing pgwire's private connection loop. The driver counts
//! the same bytes at the other end of the same socket: request bytes are
//! exact (one prebuilt buffer per cell) and response bytes are summed as each
//! cycle is consumed.
//!
//! **`pgwire` flushes about twice per query.** `send_query_response` feeds
//! each `DataRow` but *sends* (feed + flush) the `CommandComplete`, and
//! `send_ready_for_query` sends again. Every other lane defers its flush while
//! more input is already buffered (the drain-then-flush shape, SRV-006
//! analog), so on the pipelined cells this lane pays syscalls the others do
//! not. That is a property of the real implementation, not an artifact of this
//! harness — it is what a production pgwire server does, and reading it as
//! "the PostgreSQL protocol is slow" would be wrong.
//!
//! # Scope (honesty note, BEN-002)
//!
//! A **benchmark peer, not a database**. Startup is pgwire's default no-op
//! (`NoopStartupHandler`: accepts everything, no auth, no TLS), which happens
//! in `connect` before any measured window. The measured path is the **simple
//! query** message (`Q`), whose text is `"<CMD> <payload>"`: the first token
//! selects the backend mode (`ECHO`/`STATIC`/`SINK`/`PING`), the rest is the
//! payload. The reply is a one-column, one-row `text` result. No SQL parser,
//! no catalog, no cursors, no COPY, no cancellation.
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

use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures::{stream, StreamExt};
use pgwire::api::query::SimpleQueryHandler;
use pgwire::api::results::{DataRowEncoder, FieldFormat, FieldInfo, QueryResponse, Response};
use pgwire::api::{ClientInfo, PgWireServerHandlers, Type};
use pgwire::error::PgWireResult;
use pgwire::tokio::process_socket;
use thunder::wire::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{watch, OwnedSemaphorePermit, Semaphore};

use crate::backend::NoopBackend;
use crate::driver::{CellSpec, Measured, RunConfig};
use crate::stats::compute;

use std::collections::VecDeque;
use std::sync::{Mutex as StdMutex, MutexGuard, PoisonError};

/// Protocol version 3.0, as the startup message encodes it (`3 << 16`).
const PROTOCOL_V3: i32 = 196_608;
/// Message cap — mirrors the Thunder frame cap (WIRE-020) so an oversized
/// length prefix cannot drive an unbounded allocation.
const MAX_MSG_LEN: usize = thunder::wire::DEFAULT_MAX_FRAME_BYTES;

/// Simple query — the one frontend message the driver sends after startup.
const MSG_QUERY: u8 = b'Q';
/// Ready for query — the end of a response cycle.
const MSG_READY_FOR_QUERY: u8 = b'Z';

/// Ride through a poisoned lock: the guarded state stays consistent.
fn lock<T>(mutex: &StdMutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

// ── Server: the pgwire handler over the shared backend ──────────────────────

/// Serves the no-op backend as one-column, one-row `text` results.
///
/// The schema is built once and shared by `Arc`, and one [`DataRowEncoder`] is
/// reused across the row stream — pgwire's intended fast path (a fresh encoder
/// per row is the documented slow way).
struct BenchHandler {
    backend: Arc<NoopBackend>,
    schema: Arc<Vec<FieldInfo>>,
}

impl BenchHandler {
    fn new(backend: Arc<NoopBackend>) -> Self {
        let schema = Arc::new(vec![FieldInfo::new(
            "r".to_owned(),
            None,
            None,
            Type::TEXT,
            FieldFormat::Text,
        )]);
        Self { backend, schema }
    }
}

#[async_trait]
impl SimpleQueryHandler for BenchHandler {
    async fn do_query<C>(&self, _client: &mut C, query: &str) -> PgWireResult<Vec<Response>>
    where
        C: ClientInfo + Unpin + Send + Sync,
    {
        let (command, payload) = parse_query(query);
        let value = match self
            .backend
            .respond(&command, command_args(&command, payload))
        {
            Ok(value) => value_to_string(value),
            Err(message) => message,
        };
        let schema = Arc::clone(&self.schema);
        let mut encoder = DataRowEncoder::new(Arc::clone(&schema));
        let rows = stream::iter(std::iter::once(value)).map(move |value| {
            encoder.encode_field(&value)?;
            Ok(encoder.take_row())
        });
        Ok(vec![Response::Query(QueryResponse::new(schema, rows))])
    }
}

/// Handler factory — every other handler defaults to pgwire's `NoopHandler`,
/// which is what makes startup a no-op (accepts everything, no auth).
struct BenchFactory {
    handler: Arc<BenchHandler>,
}

impl PgWireServerHandlers for BenchFactory {
    fn simple_query_handler(&self) -> Arc<impl SimpleQueryHandler> {
        Arc::clone(&self.handler)
    }
}

/// Split a query's text into `(command, payload)` at the first space.
fn parse_query(text: &str) -> (String, String) {
    match text.split_once(' ') {
        Some((command, payload)) => (command.to_owned(), payload.to_owned()),
        None => (text.to_owned(), String::new()),
    }
}

/// A backend reply value as the row's one column.
fn value_to_string(value: Value) -> String {
    match value {
        Value::Str(s) => s,
        Value::Bytes(b) => String::from_utf8_lossy(&b).into_owned(),
        _ => String::new(),
    }
}

/// Turn a parsed `(cmd, payload)` into backend args: ECHO carries the
/// payload, the sentinels carry nothing.
fn command_args(cmd: &str, payload: String) -> Vec<Value> {
    match cmd {
        "ECHO" if !payload.is_empty() => vec![Value::Str(payload)],
        _ => vec![],
    }
}

// ── Listener ────────────────────────────────────────────────────────────────

/// Handle to the running pgwire listener — same shape as the other lanes.
///
/// Only the accept loop is ours; each accepted connection is driven to
/// completion by [`process_socket`], which ends when the driver drops its end.
#[derive(Debug)]
pub struct PgHandle {
    addr: SocketAddr,
    shutdown: watch::Sender<bool>,
    done: Option<tokio::sync::mpsc::Receiver<()>>,
}

impl PgHandle {
    /// The bound address.
    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    /// Graceful shutdown of the accept loop.
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

/// Spawn the pgwire listener over the shared no-op backend.
pub async fn spawn_postgres_listener(
    backend: Arc<NoopBackend>,
    addr: SocketAddr,
) -> io::Result<PgHandle> {
    let listener = TcpListener::bind(addr).await?;
    let addr = listener.local_addr()?;
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let (done_tx, done_rx) = tokio::sync::mpsc::channel::<()>(1);

    let factory = Arc::new(BenchFactory {
        handler: Arc::new(BenchHandler::new(backend)),
    });

    tokio::spawn(async move {
        let _done = done_tx;
        let mut shutdown = shutdown_rx;
        loop {
            let accepted = tokio::select! {
                _ = shutdown.wait_for(|stop| *stop) => break,
                accepted = listener.accept() => accepted,
            };
            let Ok((stream, _)) = accepted else { break };
            let factory = Arc::clone(&factory);
            tokio::spawn(async move {
                let _ = process_socket(stream, None, factory).await;
            });
        }
    });

    Ok(PgHandle {
        addr,
        shutdown: shutdown_tx,
        done: Some(done_rx),
    })
}

// ── Driver ────────────────────────────────────────────────────────────────

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

/// A simple query message carrying `"<CMD> <payload>"` (or just `"<CMD>"`
/// when the payload is empty): `type(1) + length(int32, counting itself) +
/// cstring`.
fn encode_query(command: &str, payload: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(command.len() + payload.len() + 2);
    body.extend_from_slice(command.as_bytes());
    if !payload.is_empty() {
        body.push(b' ');
        body.extend_from_slice(payload);
    }
    body.push(0); // cstring terminator
    let mut out = Vec::with_capacity(body.len() + 5);
    out.push(MSG_QUERY);
    // PostgreSQL is big-endian on the wire, throughout.
    out.extend_from_slice(&((body.len() + 4) as i32).to_be_bytes());
    out.extend_from_slice(&body);
    out
}

/// One driver connection, started up and ready for queries.
struct PgConn {
    reader: BufReader<OwnedReadHalf>,
    writer: OwnedWriteHalf,
}

impl PgConn {
    /// Dial and complete startup. Everything here is session setup: the cell
    /// driver calls it *before* any measurement so startup never lands inside
    /// a measured window (the storm scenario is the exception — there setup is
    /// the thing measured).
    async fn connect(addr: SocketAddr) -> Result<(Self, usize), String> {
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
        let startup = encode_startup();
        conn.writer
            .write_all(&startup)
            .await
            .map_err(|e| format!("postgres startup write failed: {e}"))?;
        // pgwire answers startup with AuthenticationOk + ParameterStatus(s) +
        // BackendKeyData + ReadyForQuery; consuming through ReadyForQuery
        // leaves the connection at the same point every lane starts from.
        let bytes = read_until_ready(&mut conn.reader).await?;
        Ok((conn, bytes))
    }
}

/// Read and fully consume backend messages up to and including
/// `ReadyForQuery` — one whole response cycle, the measurement point
/// (BEN-003). Returns the bytes consumed, which is how this lane counts
/// response bytes (see the module docs).
async fn read_until_ready(reader: &mut BufReader<OwnedReadHalf>) -> Result<usize, String> {
    let mut total = 0usize;
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
        total += len + 1; // the type byte is outside the declared length
        if kind == MSG_READY_FOR_QUERY {
            return Ok(total);
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
        Value::Bytes(b) => Ok(b.to_vec()),
        other => Err(format!("postgres lane: unsupported arg {other:?}")),
    }
}

/// Measure one matrix cell on the PostgreSQL v3 lane.
pub async fn cell(handle: &PgHandle, spec: &CellSpec, cfg: &RunConfig) -> Result<Measured, String> {
    let addr = handle.local_addr();
    let request = Arc::new(build_postgres_request(spec.command, &spec.args)?);
    let mut conns = Vec::with_capacity(spec.connections);
    for _ in 0..spec.connections {
        let (conn, _startup_bytes) = PgConn::connect(addr).await?;
        conns.push(conn);
    }

    if cfg.warmup > 0 {
        let warmed = pg_window(conns, spec.depth, cfg.warmup, &request).await?;
        conns = warmed.conns;
    }
    let mut reps = Vec::with_capacity(cfg.repetitions);
    let mut bytes_out = 0u64;
    let mut ops = 0u64;
    for _ in 0..cfg.repetitions {
        let mut window = pg_window(conns, spec.depth, cfg.ops, &request).await?;
        conns = window.conns;
        bytes_out += window.bytes_out;
        ops += window.latencies.len() as u64;
        reps.push(compute(&mut window.latencies, window.elapsed));
    }
    drop(conns);

    let ops = ops.max(1) as f64;
    Ok((reps, request.len() as f64, bytes_out as f64 / ops))
}

/// What one measured window produced.
struct Window {
    conns: Vec<PgConn>,
    latencies: Vec<Duration>,
    elapsed: Duration,
    bytes_out: u64,
}

/// One continuously-full window across all connections.
async fn pg_window(
    conns: Vec<PgConn>,
    depth: usize,
    total_ops: usize,
    request: &Arc<Vec<u8>>,
) -> Result<Window, String> {
    let per_conn = (total_ops / conns.len().max(1)).max(depth).max(1);
    let started = Instant::now();
    let mut handles = Vec::with_capacity(conns.len());
    for conn in conns {
        let request = Arc::clone(request);
        handles.push(tokio::spawn(pg_conn_window(conn, depth, per_conn, request)));
    }
    let mut returned = Vec::with_capacity(handles.len());
    let mut all = Vec::with_capacity(per_conn * handles.len());
    let mut bytes_out = 0u64;
    for handle in handles {
        let (conn, lats, bytes) = handle
            .await
            .map_err(|e| format!("postgres worker panicked: {e}"))??;
        returned.push(conn);
        all.extend(lats);
        bytes_out += bytes;
    }
    Ok(Window {
        conns: returned,
        latencies: all,
        elapsed: started.elapsed(),
        bytes_out,
    })
}

/// FIFO pipeline window on one connection (BEN-003), same shape as the RESP3,
/// Memcached, MongoDB and MessagePack-RPC drivers.
async fn pg_conn_window(
    mut conn: PgConn,
    depth: usize,
    ops: usize,
    request: Arc<Vec<u8>>,
) -> Result<(PgConn, Vec<Duration>, u64), String> {
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
            let mut bytes = 0u64;
            for _ in 0..ops {
                bytes += read_until_ready(reader).await? as u64;
                let (sent, permit) = lock(&pending)
                    .pop_front()
                    .ok_or_else(|| "postgres reply without a pending request".to_owned())?;
                lats.push(sent.elapsed());
                drop(permit);
            }
            Ok::<(Vec<Duration>, u64), String>((lats, bytes))
        }
    };

    let (sent, received) = tokio::join!(sender, receiver);
    sent?;
    let (lats, bytes) = received?;
    Ok((conn, lats, bytes))
}

/// The connection-storm cell: connect + startup + one query + its response
/// cycle, repeated. Setup is inside the measurement here by design.
pub async fn storm(handle: &PgHandle, storms: usize, cfg: &RunConfig) -> Result<Measured, String> {
    let addr = handle.local_addr();
    let request = build_postgres_request("PING", &[])?;
    for _ in 0..cfg.warmup.min(storms) {
        pg_storm_once(addr, &request).await?;
    }
    let mut reps = Vec::with_capacity(cfg.repetitions);
    let mut bytes_out = 0u64;
    let mut ops = 0u64;
    for _ in 0..cfg.repetitions {
        let mut lats = Vec::with_capacity(storms);
        let started = Instant::now();
        for _ in 0..storms {
            let (latency, bytes) = pg_storm_once(addr, &request).await?;
            lats.push(latency);
            bytes_out += bytes;
            ops += 1;
        }
        reps.push(compute(&mut lats, started.elapsed()));
    }
    let ops = ops.max(1) as f64;
    // A storm op is a whole session: the startup message plus the query.
    let bytes_in = (encode_startup().len() + request.len()) as f64;
    Ok((reps, bytes_in, bytes_out as f64 / ops))
}

/// One storm iteration, returning its latency and the response bytes it read
/// (startup reply included — the storm measures session setup).
async fn pg_storm_once(addr: SocketAddr, request: &[u8]) -> Result<(Duration, u64), String> {
    let started = Instant::now();
    let (mut conn, startup_bytes) = PgConn::connect(addr).await?;
    conn.writer
        .write_all(request)
        .await
        .map_err(|e| format!("storm write failed: {e}"))?;
    let reply_bytes = read_until_ready(&mut conn.reader).await?;
    Ok((started.elapsed(), (startup_bytes + reply_bytes) as u64))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::backend::STATIC_REPLY_BYTES;

    #[test]
    fn query_text_splits_into_command_and_payload() {
        let (command, payload) = parse_query("ECHO hello world");
        assert_eq!(command, "ECHO");
        assert_eq!(payload, "hello world");
    }

    #[test]
    fn a_bare_command_has_no_payload() {
        let (command, payload) = parse_query("PING");
        assert_eq!(command, "PING");
        assert!(payload.is_empty());
    }

    #[test]
    fn message_length_counts_itself_but_not_the_type_byte() {
        let frame = encode_query("ECHO", b"xyz");
        assert_eq!(frame[0], MSG_QUERY);
        let declared = i32::from_be_bytes([frame[1], frame[2], frame[3], frame[4]]) as usize;
        assert_eq!(declared, frame.len() - 1, "length excludes the type byte");
    }

    #[test]
    fn a_query_frame_round_trips_through_the_parser() {
        let frame = build_postgres_request("ECHO", &[Value::Str("x".repeat(64))]).unwrap();
        let text = std::str::from_utf8(&frame[5..frame.len() - 1]).unwrap();
        let (command, payload) = parse_query(text);
        assert_eq!(command, "ECHO");
        assert_eq!(payload, "x".repeat(64));
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
    fn the_handler_echoes_through_the_shared_backend() {
        let backend = NoopBackend::new();
        let (command, payload) = parse_query("ECHO xxxxxxxx");
        let value = backend
            .respond(&command, command_args(&command, payload))
            .unwrap();
        assert_eq!(value_to_string(value), "xxxxxxxx");
    }

    #[test]
    fn the_handler_serves_the_4kib_static_reply() {
        let backend = NoopBackend::new();
        let (command, payload) = parse_query("STATIC");
        let value = backend
            .respond(&command, command_args(&command, payload))
            .unwrap();
        assert_eq!(value_to_string(value).len(), STATIC_REPLY_BYTES);
    }

    #[test]
    fn nul_payloads_are_refused_not_truncated() {
        let err = build_postgres_request("ECHO", &[Value::bytes(vec![b'a', 0, b'b'])]).unwrap_err();
        assert!(err.contains("NUL-free"), "unexpected error: {err}");
    }
}

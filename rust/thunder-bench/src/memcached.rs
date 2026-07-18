//! Minimal Memcached **binary** protocol lane over the same no-op backend
//! (BEN-001, BEN-002) — same process, host, runtime and allocator as the
//! Thunder listener.
//!
//! # Why this lane
//!
//! Memcached's binary protocol is the leanest request/response wire in wide
//! use: a fixed 24-byte header, a key, an optional value, and nothing else —
//! no framing negotiation, no type system, strictly ordered per connection
//! (FIFO). It is the **performance ceiling** for the shootout: if Thunder
//! comes close to it on the wire, there is little left to give. Like RESP3 it
//! is FIFO (no out-of-order demux), so it shares RESP3's cheap-per-call /
//! no-pipelining-coalescing profile — the mirror image of Thunder's
//! multiplexed design.
//!
//! # Scope (honesty note, BEN-002)
//!
//! A **benchmark peer, not a Memcached server**. Exactly one opcode is
//! modelled — `GET` (0x00) — because it is all the echo/static/sink workloads
//! need. The key carries the workload: the sentinels `STATIC` / `SINK` / `PING`
//! map to the backend's modes, and any other key is echoed back as the value
//! (so a 64-byte ECHO payload becomes a 64-byte key echoed as a 64-byte value).
//! The real protocol caps keys at 250 bytes; the matrix respects that by
//! routing the only large payload (medium-4KiB) through the small `STATIC`
//! sentinel, whose 4 KiB reply is the backend's static value — a faithful
//! "GET of a pre-stored 4 KiB item". No storage, no CAS, no flags semantics.
//!
//! # Parity (BEN-003)
//!
//! The driver keeps a continuously-full in-flight window per connection (a
//! semaphore slot per outstanding request, replies matched FIFO), identical in
//! shape to the RESP3 and HTTP drivers. Server-side bytes are counted at the
//! socket after the successful write, the same measurement point as every lane.

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

/// Binary-protocol magic: request header.
const MAGIC_REQ: u8 = 0x80;
/// Binary-protocol magic: response header.
const MAGIC_RES: u8 = 0x81;
/// The one opcode this peer models (BEN-002).
const OP_GET: u8 = 0x00;
/// Fixed header size, both directions.
const HEADER_LEN: usize = 24;
/// Response `status` for success.
const STATUS_OK: u16 = 0x0000;
/// Response `status` for a backend error (maps a `SINK`/unknown rejection).
const STATUS_ERR: u16 = 0x0084;
/// The real protocol's key cap; a request over it is rejected, not truncated.
const MAX_KEY_LEN: usize = 250;
/// Body cap — mirrors the Thunder frame cap (WIRE-020) so an oversized length
/// prefix cannot drive an unbounded allocation.
const MAX_BODY_LEN: usize = thunder::wire::DEFAULT_MAX_FRAME_BYTES;

/// Ride through a poisoned lock: the guarded state stays consistent.
fn lock<T>(mutex: &StdMutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Server-side counters, sampled around a measured window — the same
/// measurement point as every other lane (after the successful write).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct McMetricsSnapshot {
    /// Requests answered.
    pub requests: u64,
    /// Request bytes read off the wire.
    pub bytes_in: u64,
    /// Response bytes written to the wire.
    pub bytes_out: u64,
}

#[derive(Debug, Default)]
struct McMetrics {
    requests: AtomicU64,
    bytes_in: AtomicU64,
    bytes_out: AtomicU64,
}

impl McMetrics {
    fn snapshot(&self) -> McMetricsSnapshot {
        McMetricsSnapshot {
            requests: self.requests.load(Ordering::Relaxed),
            bytes_in: self.bytes_in.load(Ordering::Relaxed),
            bytes_out: self.bytes_out.load(Ordering::Relaxed),
        }
    }
}

/// Handle to the running Memcached-binary listener — same shape as the other
/// lanes' handles.
#[derive(Debug)]
pub struct McHandle {
    addr: SocketAddr,
    shutdown: watch::Sender<bool>,
    metrics: Arc<McMetrics>,
    done: Option<tokio::sync::mpsc::Receiver<()>>,
}

impl McHandle {
    /// The bound address.
    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    /// Current server-side counters.
    pub fn snapshot(&self) -> McMetricsSnapshot {
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

impl Drop for McHandle {
    fn drop(&mut self) {
        let _ = self.shutdown.send(true);
    }
}

/// Spawn the Memcached-binary listener over the shared no-op backend.
pub async fn spawn_memcached_listener(
    backend: Arc<NoopBackend>,
    addr: SocketAddr,
) -> io::Result<McHandle> {
    let listener = TcpListener::bind(addr).await?;
    let addr = listener.local_addr()?;
    let metrics = Arc::new(McMetrics::default());
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

    Ok(McHandle {
        addr,
        shutdown: shutdown_tx,
        metrics,
        done: Some(done_rx),
    })
}

/// One connection: read a binary request, answer via the backend, write the
/// reply. Drain-then-flush mirrors every other lane (SRV-006 analog).
async fn serve_conn(
    stream: TcpStream,
    backend: Arc<NoopBackend>,
    metrics: Arc<McMetrics>,
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
        if read.is_err() || header[0] != MAGIC_REQ {
            break;
        }
        let key_len = u16::from_be_bytes([header[2], header[3]]) as usize;
        let extras_len = header[4] as usize;
        let body_len = u32::from_be_bytes([header[8], header[9], header[10], header[11]]) as usize;
        if body_len > MAX_BODY_LEN || key_len > body_len || extras_len > body_len {
            break;
        }
        let mut body = vec![0u8; body_len];
        if reader.read_exact(&mut body).await.is_err() {
            break;
        }
        metrics
            .bytes_in
            .fetch_add((HEADER_LEN + body_len) as u64, Ordering::Relaxed);

        // The key drives the workload (see the scope note). extras precede the
        // key in the body; GET carries no extras, but stay general.
        let key = &body[extras_len..extras_len + key_len];
        let (command, args) = classify(key);
        let (status, value) = match backend.respond(command, args) {
            Ok(value) => (STATUS_OK, value_to_bytes(value)),
            Err(message) => (STATUS_ERR, message.into_bytes()),
        };

        let mut frame = Vec::with_capacity(HEADER_LEN + 4 + value.len());
        encode_get_response(&mut frame, status, &value);
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

/// Map a request key to a backend command (see the scope note). The sentinels
/// route to the backend's modes; any other key is echoed back.
fn classify(key: &[u8]) -> (&'static str, Vec<Value>) {
    match key {
        b"STATIC" => ("STATIC", vec![]),
        b"SINK" => ("SINK", vec![]),
        b"PING" => ("PING", vec![]),
        b"ECHO" => ("ECHO", vec![]),
        other => ("ECHO", vec![Value::Bytes(other.to_vec())]),
    }
}

/// A backend reply value as the bytes Memcached would carry in a GET value.
fn value_to_bytes(value: Value) -> Vec<u8> {
    match value {
        Value::Str(s) => s.into_bytes(),
        Value::Bytes(b) => b,
        _ => Vec::new(),
    }
}

/// Encode a GET response: 24-byte header + 4-byte flags extras + value.
fn encode_get_response(out: &mut Vec<u8>, status: u16, value: &[u8]) {
    const EXTRAS_LEN: usize = 4; // flags
    let body_len = EXTRAS_LEN + value.len();
    out.push(MAGIC_RES);
    out.push(OP_GET);
    out.extend_from_slice(&0u16.to_be_bytes()); // key length
    out.push(EXTRAS_LEN as u8);
    out.push(0); // data type
    out.extend_from_slice(&status.to_be_bytes());
    out.extend_from_slice(&(body_len as u32).to_be_bytes());
    out.extend_from_slice(&0u32.to_be_bytes()); // opaque
    out.extend_from_slice(&0u64.to_be_bytes()); // CAS
    out.extend_from_slice(&0u32.to_be_bytes()); // flags (extras)
    out.extend_from_slice(value);
}

/// Encode a GET request: 24-byte header + key (no extras, no value).
fn encode_get_request(key: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER_LEN + key.len());
    out.push(MAGIC_REQ);
    out.push(OP_GET);
    out.extend_from_slice(&(key.len() as u16).to_be_bytes());
    out.push(0); // extras length
    out.push(0); // data type
    out.extend_from_slice(&0u16.to_be_bytes()); // vbucket
    out.extend_from_slice(&(key.len() as u32).to_be_bytes()); // body length = key length
    out.extend_from_slice(&0u32.to_be_bytes()); // opaque
    out.extend_from_slice(&0u64.to_be_bytes()); // CAS
    out.extend_from_slice(key);
    out
}

// ── Driver ────────────────────────────────────────────────────────────────

/// One driver connection: a raw write half (direct writes, nodelay — the lean
/// FIFO client) and a buffered read half.
struct McConn {
    reader: BufReader<OwnedReadHalf>,
    writer: OwnedWriteHalf,
}

impl McConn {
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

/// Read + fully consume one reply (the measurement point, BEN-003): 24-byte
/// header + body. A non-OK status is a lane error.
async fn read_reply(reader: &mut BufReader<OwnedReadHalf>) -> Result<(), String> {
    let mut header = [0u8; HEADER_LEN];
    reader
        .read_exact(&mut header)
        .await
        .map_err(|e| format!("memcached read failed: {e}"))?;
    if header[0] != MAGIC_RES {
        return Err(format!("memcached bad magic 0x{:02x}", header[0]));
    }
    let status = u16::from_be_bytes([header[6], header[7]]);
    let body_len = u32::from_be_bytes([header[8], header[9], header[10], header[11]]) as usize;
    if body_len > MAX_BODY_LEN {
        return Err(format!("memcached body {body_len} over cap"));
    }
    let mut body = vec![0u8; body_len];
    reader
        .read_exact(&mut body)
        .await
        .map_err(|e| format!("memcached body read failed: {e}"))?;
    if status != STATUS_OK {
        return Err(format!("memcached status 0x{status:04x}"));
    }
    Ok(())
}

/// Build one request from the matrix `(command, args)`. The key encodes the
/// workload (see the scope note); a payload over the 250-byte key cap is a
/// build error, never silently truncated.
fn build_memcached_request(command: &str, args: &[Value]) -> Result<Vec<u8>, String> {
    let key: Vec<u8> = match command {
        "ECHO" | "PING" => match args.first() {
            Some(value) => value_bytes(value)?,
            None => command.as_bytes().to_vec(),
        },
        // STATIC / SINK sentinels — the large-payload path routes here.
        other => other.as_bytes().to_vec(),
    };
    if key.len() > MAX_KEY_LEN {
        return Err(format!(
            "memcached key {} over the {MAX_KEY_LEN}-byte cap — large payloads must use a sentinel",
            key.len()
        ));
    }
    Ok(encode_get_request(&key))
}

/// A matrix arg as raw key bytes.
fn value_bytes(value: &Value) -> Result<Vec<u8>, String> {
    match value {
        Value::Str(s) => Ok(s.clone().into_bytes()),
        Value::Bytes(b) => Ok(b.clone()),
        other => Err(format!("memcached lane: unsupported arg {other:?}")),
    }
}

/// Measure one matrix cell on the Memcached-binary lane.
pub async fn cell(handle: &McHandle, spec: &CellSpec, cfg: &RunConfig) -> Result<Measured, String> {
    let addr = handle.local_addr();
    let request = Arc::new(build_memcached_request(spec.command, &spec.args)?);
    let mut conns = Vec::with_capacity(spec.connections);
    for _ in 0..spec.connections {
        conns.push(
            McConn::connect(addr)
                .await
                .map_err(|e| format!("memcached connect failed: {e}"))?,
        );
    }

    if cfg.warmup > 0 {
        let (warmed, _lats, _elapsed) = mc_window(conns, spec.depth, cfg.warmup, &request).await?;
        conns = warmed;
    }
    let before = handle.snapshot();
    let mut reps = Vec::with_capacity(cfg.repetitions);
    for _ in 0..cfg.repetitions {
        let (returned, mut lats, elapsed) = mc_window(conns, spec.depth, cfg.ops, &request).await?;
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
async fn mc_window(
    conns: Vec<McConn>,
    depth: usize,
    total_ops: usize,
    request: &Arc<Vec<u8>>,
) -> Result<(Vec<McConn>, Vec<Duration>, Duration), String> {
    let per_conn = (total_ops / conns.len().max(1)).max(depth).max(1);
    let started = Instant::now();
    let mut handles = Vec::with_capacity(conns.len());
    for conn in conns {
        let request = Arc::clone(request);
        handles.push(tokio::spawn(mc_conn_window(conn, depth, per_conn, request)));
    }
    let mut returned = Vec::with_capacity(handles.len());
    let mut all = Vec::with_capacity(per_conn * handles.len());
    for handle in handles {
        let (conn, lats) = handle
            .await
            .map_err(|e| format!("memcached worker panicked: {e}"))??;
        returned.push(conn);
        all.extend(lats);
    }
    Ok((returned, all, started.elapsed()))
}

/// FIFO pipeline window on one connection: the sender keeps up to `depth`
/// requests on the wire (a semaphore slot each), the receiver reads replies in
/// order and frees slots — continuous pipelining, no inter-batch gaps
/// (BEN-003). Same shape as `resp3_conn_window`.
async fn mc_conn_window(
    mut conn: McConn,
    depth: usize,
    ops: usize,
    request: Arc<Vec<u8>>,
) -> Result<(McConn, Vec<Duration>), String> {
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
                    .map_err(|e| format!("memcached write failed: {e}"))?;
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
                    .ok_or_else(|| "memcached reply without a pending request".to_owned())?;
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
pub async fn storm(handle: &McHandle, storms: usize, cfg: &RunConfig) -> Result<Measured, String> {
    let addr = handle.local_addr();
    let request = build_memcached_request("PING", &[])?;
    for _ in 0..cfg.warmup.min(storms) {
        mc_storm_once(addr, &request).await?;
    }
    let before = handle.snapshot();
    let mut reps = Vec::with_capacity(cfg.repetitions);
    for _ in 0..cfg.repetitions {
        let mut lats = Vec::with_capacity(storms);
        let started = Instant::now();
        for _ in 0..storms {
            lats.push(mc_storm_once(addr, &request).await?);
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

async fn mc_storm_once(addr: SocketAddr, request: &[u8]) -> Result<Duration, String> {
    let started = Instant::now();
    let mut conn = McConn::connect(addr)
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

    /// Round-trip a request through the server's classify + backend + response
    /// encode, then parse the value out, in-process (no socket).
    fn server_value_for(key: &[u8]) -> (u16, Vec<u8>) {
        let backend = NoopBackend::new();
        let (command, args) = classify(key);
        let (status, value) = match backend.respond(command, args) {
            Ok(v) => (STATUS_OK, value_to_bytes(v)),
            Err(m) => (STATUS_ERR, m.into_bytes()),
        };
        let mut frame = Vec::new();
        encode_get_response(&mut frame, status, &value);
        // Parse it back: header + 4 flags + value.
        assert_eq!(frame[0], MAGIC_RES);
        let body_len = u32::from_be_bytes([frame[8], frame[9], frame[10], frame[11]]) as usize;
        let parsed_status = u16::from_be_bytes([frame[6], frame[7]]);
        let value = frame[HEADER_LEN + 4..HEADER_LEN + body_len].to_vec();
        (parsed_status, value)
    }

    #[test]
    fn echo_key_is_echoed_as_value() {
        let key = b"x".repeat(64);
        let (status, value) = server_value_for(&key);
        assert_eq!(status, STATUS_OK);
        assert_eq!(value, key, "a non-sentinel key echoes back as the value");
    }

    #[test]
    fn static_sentinel_returns_the_4kib_reply() {
        let (status, value) = server_value_for(b"STATIC");
        assert_eq!(status, STATUS_OK);
        assert_eq!(value.len(), STATIC_REPLY_BYTES);
    }

    #[test]
    fn ping_sentinel_returns_pong() {
        let (status, value) = server_value_for(b"PING");
        assert_eq!(status, STATUS_OK);
        assert_eq!(value, b"PONG");
    }

    #[test]
    fn sink_sentinel_returns_empty_value() {
        let (status, value) = server_value_for(b"SINK");
        assert_eq!(status, STATUS_OK);
        assert!(value.is_empty());
    }

    #[test]
    fn request_encodes_the_echo_payload_as_the_key() {
        let request = build_memcached_request("ECHO", &[Value::Str("x".repeat(64))]).unwrap();
        assert_eq!(request[0], MAGIC_REQ);
        assert_eq!(request[1], OP_GET);
        let key_len = u16::from_be_bytes([request[2], request[3]]) as usize;
        assert_eq!(key_len, 64);
        assert_eq!(&request[HEADER_LEN..HEADER_LEN + 64], &b"x".repeat(64)[..]);
    }

    #[test]
    fn static_workload_routes_through_the_small_sentinel() {
        // medium-4KiB never puts 4 KiB in the key: it uses the STATIC sentinel.
        let request = build_memcached_request("STATIC", &[]).unwrap();
        let key_len = u16::from_be_bytes([request[2], request[3]]) as usize;
        assert_eq!(key_len, b"STATIC".len());
    }

    #[test]
    fn a_payload_over_the_key_cap_is_rejected_not_truncated() {
        let big = build_memcached_request("ECHO", &[Value::Str("x".repeat(300))]);
        assert!(big.is_err(), "over-250-byte key must be a build error");
    }
}

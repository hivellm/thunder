//! Minimal **MessagePack-RPC** lane over the same no-op backend (BEN-001,
//! BEN-002) — same process, host, runtime and allocator as the Thunder
//! listener.
//!
//! # Why this lane
//!
//! MessagePack-RPC is Thunder's **sibling**: the same codec (MessagePack,
//! via the same `rmp-serde` this workspace already builds Thunder's frames
//! with) and the same correlation idea (a message id, so the protocol
//! permits out-of-order replies). One thing differs — **framing**.
//! MessagePack-RPC is a bare self-delimiting stream: a request is the
//! MessagePack array `[0, msgid, method, params]` and nothing precedes it, so
//! a reader must *structurally scan* the codec to find where a message ends.
//! Thunder prefixes every body with a `u32` length (WIRE-001), so its reader
//! knows the boundary before it parses a byte.
//!
//! With the codec held constant, that is the one variable this lane isolates:
//! **what the length prefix buys**. Any difference measured here is framing,
//! not serialization.
//!
//! # Scope (honesty note, BEN-002)
//!
//! A **benchmark peer, not a MessagePack-RPC implementation**. Requests are
//! `[0, msgid, method, params]`, replies are `[1, msgid, error, result]` with
//! `error` always `nil`; `method` selects the backend mode
//! (`ECHO`/`STATIC`/`SINK`/`PING`) and `params` carries the payload.
//! Notifications (type 2) are not modelled — the matrix is entirely
//! request/response. The structural scanner ([`scan_value`]) covers the
//! MessagePack types this traffic actually contains (nil, bool, ints, floats,
//! str, bin, array, map) and refuses ext types rather than guessing at them.
//!
//! **Replies are FIFO, and the driver correlates FIFO.** The protocol permits
//! reordering, but this peer answers sequentially per connection like every
//! other reference lane, so a demux table would sit idle and only add cost no
//! real msgpack-rpc server would pay here. The msgid still round-trips on the
//! wire (its bytes are measured, and a test pins that the reply echoes the
//! request's id); proving what *multiplexing* costs is the designated job of
//! the Phase 4 gRPC lane, which is genuinely multiplexed end to end.
//!
//! # Parity (BEN-003)
//!
//! The driver keeps a continuously-full in-flight window per connection (a
//! semaphore slot per outstanding request, replies matched FIFO), identical in
//! shape to the RESP3/Memcached/MongoDB/PostgreSQL drivers. Like every other
//! lane it sends one prebuilt request per cell — client-side encoding is not
//! in the measurement — so the msgid is constant across a cell. The driver
//! scans each reply to its boundary and consumes it without decoding the
//! payload: the scan *is* this protocol's mandatory framing work, and it is
//! exactly what the length prefix would have replaced. Server-side bytes are
//! counted at the socket after the successful write, the same measurement
//! point as every lane.

use std::collections::VecDeque;
use std::io;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use thunder::wire::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufWriter};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{watch, OwnedSemaphorePermit, Semaphore};

use crate::backend::NoopBackend;
use crate::driver::{CellSpec, Measured, RunConfig};
use crate::stats::compute;

/// Message type: request.
const TYPE_REQUEST: u8 = 0;
/// Message type: response.
const TYPE_RESPONSE: u8 = 1;
/// The one message id the matrix's prebuilt request carries.
const MSGID: u32 = 1;
/// Buffer cap — mirrors the Thunder frame cap (WIRE-020). A stream protocol
/// has no length prefix to validate, so the *buffer* is what must be bounded:
/// a peer that never completes a message cannot grow it without limit.
const MAX_BUFFERED: usize = thunder::wire::DEFAULT_MAX_FRAME_BYTES;
/// Nesting cap for the structural scanner — this traffic is two levels deep;
/// anything deeper is a malformed stream, not a workload.
const MAX_DEPTH: usize = 32;
/// Socket read chunk.
const READ_CHUNK: usize = 8192;

/// Ride through a poisoned lock: the guarded state stays consistent.
fn lock<T>(mutex: &StdMutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

// ── Structural scanner ──────────────────────────────────────────────────────

/// Why a scan did not yield a message boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanError {
    /// The buffer holds a valid prefix — read more bytes and try again.
    NeedMore,
    /// The bytes are not MessagePack this peer models (or nest too deep).
    Malformed,
}

/// Advance past exactly one MessagePack value starting at `at`, returning the
/// offset just past it.
///
/// This is the work a length prefix removes: to find where a message ends you
/// must walk its type tags. It is the honest cost of a self-delimiting stream
/// and it is on this lane's measured path.
pub fn scan_value(buf: &[u8], at: usize) -> Result<usize, ScanError> {
    scan_at(buf, at, 0)
}

fn scan_at(buf: &[u8], at: usize, depth: usize) -> Result<usize, ScanError> {
    if depth > MAX_DEPTH {
        return Err(ScanError::Malformed);
    }
    let tag = *buf.get(at).ok_or(ScanError::NeedMore)?;
    let at = at + 1;
    match tag {
        // positive fixint, nil, false, true, negative fixint — tag only
        0x00..=0x7f | 0xc0 | 0xc2 | 0xc3 | 0xe0..=0xff => Ok(at),
        0x80..=0x8f => scan_n(buf, at, 2 * (tag & 0x0f) as usize, depth), // fixmap
        0x90..=0x9f => scan_n(buf, at, (tag & 0x0f) as usize, depth),     // fixarray
        0xa0..=0xbf => skip(buf, at, (tag & 0x1f) as usize),              // fixstr
        0xc4 | 0xd9 => sized(buf, at, 1),                                 // bin8 / str8
        0xc5 | 0xda => sized(buf, at, 2),                                 // bin16 / str16
        0xc6 | 0xdb => sized(buf, at, 4),                                 // bin32 / str32
        0xcc | 0xd0 => skip(buf, at, 1),                                  // uint8 / int8
        0xcd | 0xd1 => skip(buf, at, 2),                                  // uint16 / int16
        0xca | 0xce | 0xd2 => skip(buf, at, 4), // float32 / uint32 / int32
        0xcb | 0xcf | 0xd3 => skip(buf, at, 8), // float64 / uint64 / int64
        0xdc => counted(buf, at, 2, 1, depth),  // array16
        0xdd => counted(buf, at, 4, 1, depth),  // array32
        0xde => counted(buf, at, 2, 2, depth),  // map16
        0xdf => counted(buf, at, 4, 2, depth),  // map32
        // ext / fixext — never emitted by this peer, refused rather than guessed
        _ => Err(ScanError::Malformed),
    }
}

/// Skip `n` raw bytes.
fn skip(buf: &[u8], at: usize, n: usize) -> Result<usize, ScanError> {
    let end = at.checked_add(n).ok_or(ScanError::Malformed)?;
    if end > buf.len() {
        return Err(ScanError::NeedMore);
    }
    Ok(end)
}

/// Read a big-endian length of `width` bytes, then skip that many bytes.
fn sized(buf: &[u8], at: usize, width: usize) -> Result<usize, ScanError> {
    let (len, at) = read_len(buf, at, width)?;
    skip(buf, at, len)
}

/// Read a big-endian count of `width` bytes, then scan `count * per` values.
fn counted(
    buf: &[u8],
    at: usize,
    width: usize,
    per: usize,
    depth: usize,
) -> Result<usize, ScanError> {
    let (count, at) = read_len(buf, at, width)?;
    let items = count.checked_mul(per).ok_or(ScanError::Malformed)?;
    scan_n(buf, at, items, depth)
}

/// Scan `count` consecutive values.
fn scan_n(buf: &[u8], mut at: usize, count: usize, depth: usize) -> Result<usize, ScanError> {
    for _ in 0..count {
        at = scan_at(buf, at, depth + 1)?;
    }
    Ok(at)
}

/// A big-endian unsigned length of `width` bytes.
fn read_len(buf: &[u8], at: usize, width: usize) -> Result<(usize, usize), ScanError> {
    let end = at.checked_add(width).ok_or(ScanError::Malformed)?;
    if end > buf.len() {
        return Err(ScanError::NeedMore);
    }
    let mut len = 0usize;
    for byte in &buf[at..end] {
        len = (len << 8) | *byte as usize;
    }
    Ok((len, end))
}

// ── Messages ────────────────────────────────────────────────────────────────

/// Encode a request: `[0, msgid, method, params]`.
fn encode_request(msgid: u32, method: &str, params: &[Value]) -> Result<Vec<u8>, String> {
    rmp_serde::to_vec(&(TYPE_REQUEST, msgid, method, params))
        .map_err(|e| format!("msgpack-rpc encode failed: {e}"))
}

/// Encode a response: `[1, msgid, error, result]`.
fn encode_response(msgid: u32, error: Value, result: Value) -> Result<Vec<u8>, String> {
    rmp_serde::to_vec(&(TYPE_RESPONSE, msgid, error, result))
        .map_err(|e| format!("msgpack-rpc encode failed: {e}"))
}

/// Decode a request into `(msgid, method, params)`.
fn decode_request(bytes: &[u8]) -> Result<(u32, String, Vec<Value>), String> {
    let (kind, msgid, method, params): (u8, u32, String, Vec<Value>) =
        rmp_serde::from_slice(bytes).map_err(|e| format!("msgpack-rpc decode failed: {e}"))?;
    if kind != TYPE_REQUEST {
        return Err(format!("msgpack-rpc unexpected message type {kind}"));
    }
    Ok((msgid, method, params))
}

// ── Listener ────────────────────────────────────────────────────────────────

/// Server-side counters, sampled around a measured window.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MsgpackMetricsSnapshot {
    /// Requests answered.
    pub requests: u64,
    /// Request bytes read off the wire.
    pub bytes_in: u64,
    /// Response bytes written to the wire.
    pub bytes_out: u64,
}

#[derive(Debug, Default)]
struct MsgpackMetrics {
    requests: AtomicU64,
    bytes_in: AtomicU64,
    bytes_out: AtomicU64,
}

impl MsgpackMetrics {
    fn snapshot(&self) -> MsgpackMetricsSnapshot {
        MsgpackMetricsSnapshot {
            requests: self.requests.load(Ordering::Relaxed),
            bytes_in: self.bytes_in.load(Ordering::Relaxed),
            bytes_out: self.bytes_out.load(Ordering::Relaxed),
        }
    }
}

/// Handle to the running MessagePack-RPC listener — same shape as the other
/// lanes.
#[derive(Debug)]
pub struct MsgpackHandle {
    addr: SocketAddr,
    shutdown: watch::Sender<bool>,
    metrics: Arc<MsgpackMetrics>,
    done: Option<tokio::sync::mpsc::Receiver<()>>,
}

impl MsgpackHandle {
    /// The bound address.
    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    /// Current server-side counters.
    pub fn snapshot(&self) -> MsgpackMetricsSnapshot {
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

impl Drop for MsgpackHandle {
    fn drop(&mut self) {
        let _ = self.shutdown.send(true);
    }
}

/// Spawn the MessagePack-RPC listener over the shared no-op backend.
pub async fn spawn_msgpack_rpc_listener(
    backend: Arc<NoopBackend>,
    addr: SocketAddr,
) -> io::Result<MsgpackHandle> {
    let listener = TcpListener::bind(addr).await?;
    let addr = listener.local_addr()?;
    let metrics = Arc::new(MsgpackMetrics::default());
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

    Ok(MsgpackHandle {
        addr,
        shutdown: shutdown_tx,
        metrics,
        done: Some(done_rx),
    })
}

/// One connection: scan the stream for a complete request, answer via the
/// backend, write the reply. The flush is deferred while another complete
/// message is already buffered — the drain-then-flush shape every lane holds
/// (SRV-006 analog). Here it falls out of the scanner: "is more input ready?"
/// is answered by scanning, not by a length prefix.
async fn serve_conn(
    stream: TcpStream,
    backend: Arc<NoopBackend>,
    metrics: Arc<MsgpackMetrics>,
    mut shutdown: watch::Receiver<bool>,
) {
    let _ = stream.set_nodelay(true);
    let (mut read_half, write_half) = stream.into_split();
    let mut writer = BufWriter::new(write_half);
    let mut buf: Vec<u8> = Vec::with_capacity(READ_CHUNK);
    let mut start = 0usize;
    let mut chunk = [0u8; READ_CHUNK];

    'conn: loop {
        // Find one complete request, reading more bytes while the buffer
        // holds only a prefix.
        let end = loop {
            match scan_value(&buf, start) {
                Ok(end) => break end,
                Err(ScanError::Malformed) => break 'conn,
                Err(ScanError::NeedMore) => {
                    if start > 0 {
                        buf.drain(..start);
                        start = 0;
                    }
                    if buf.len() > MAX_BUFFERED {
                        break 'conn;
                    }
                    let read = tokio::select! {
                        _ = shutdown.wait_for(|stop| *stop) => break 'conn,
                        read = read_half.read(&mut chunk) => read,
                    };
                    match read {
                        Ok(0) | Err(_) => break 'conn,
                        Ok(n) => buf.extend_from_slice(&chunk[..n]),
                    }
                }
            }
        };

        let request = &buf[start..end];
        metrics
            .bytes_in
            .fetch_add(request.len() as u64, Ordering::Relaxed);
        let reply = match decode_request(request) {
            Ok((msgid, method, params)) => match backend.respond(&method, params) {
                Ok(value) => encode_response(msgid, Value::Null, value),
                Err(message) => encode_response(msgid, Value::Str(message), Value::Null),
            },
            Err(_) => break 'conn,
        };
        let Ok(frame) = reply else { break 'conn };
        start = end;

        if writer.write_all(&frame).await.is_err() {
            break 'conn;
        }
        metrics
            .bytes_out
            .fetch_add(frame.len() as u64, Ordering::Relaxed);
        metrics.requests.fetch_add(1, Ordering::Relaxed);

        // Another complete request already buffered? Then keep the reply in
        // the writer and let the next one leave in the same flush.
        if scan_value(&buf, start).is_err() && writer.flush().await.is_err() {
            break 'conn;
        }
    }
    let _ = writer.flush().await;
}

// ── Driver ────────────────────────────────────────────────────────────────

/// One driver connection: a raw write half (direct writes, nodelay), the read
/// half, and the stream buffer the scanner works over — a self-delimiting
/// protocol has no frame boundary to read to, so the buffer is part of the
/// connection's state.
struct MsgpackConn {
    reader: OwnedReadHalf,
    writer: OwnedWriteHalf,
    buf: Vec<u8>,
    start: usize,
    chunk: Box<[u8; READ_CHUNK]>,
}

impl MsgpackConn {
    async fn connect(addr: SocketAddr) -> io::Result<Self> {
        let stream = TcpStream::connect(addr).await?;
        stream.set_nodelay(true)?;
        let (reader, writer) = stream.into_split();
        Ok(Self {
            reader,
            writer,
            buf: Vec::with_capacity(READ_CHUNK),
            start: 0,
            chunk: Box::new([0u8; READ_CHUNK]),
        })
    }

    /// Read + fully consume one reply (the measurement point, BEN-003). The
    /// scan to the message boundary is this protocol's framing work.
    async fn read_reply(&mut self) -> Result<(), String> {
        loop {
            match scan_value(&self.buf, self.start) {
                Ok(end) => {
                    self.start = end;
                    if self.start == self.buf.len() {
                        self.buf.clear();
                        self.start = 0;
                    }
                    return Ok(());
                }
                Err(ScanError::Malformed) => return Err("msgpack-rpc malformed reply".to_owned()),
                Err(ScanError::NeedMore) => {
                    if self.start > 0 {
                        self.buf.drain(..self.start);
                        self.start = 0;
                    }
                    if self.buf.len() > MAX_BUFFERED {
                        return Err("msgpack-rpc reply exceeded the buffer cap".to_owned());
                    }
                    let n = self
                        .reader
                        .read(self.chunk.as_mut())
                        .await
                        .map_err(|e| format!("msgpack-rpc read failed: {e}"))?;
                    if n == 0 {
                        return Err("msgpack-rpc connection closed".to_owned());
                    }
                    self.buf.extend_from_slice(&self.chunk[..n]);
                }
            }
        }
    }
}

/// Build one request from the matrix `(command, args)`.
fn build_msgpack_request(command: &str, args: &[Value]) -> Result<Vec<u8>, String> {
    encode_request(MSGID, command, args)
}

/// Measure one matrix cell on the MessagePack-RPC lane.
pub async fn cell(
    handle: &MsgpackHandle,
    spec: &CellSpec,
    cfg: &RunConfig,
) -> Result<Measured, String> {
    let addr = handle.local_addr();
    let request = Arc::new(build_msgpack_request(spec.command, &spec.args)?);
    let mut conns = Vec::with_capacity(spec.connections);
    for _ in 0..spec.connections {
        conns.push(
            MsgpackConn::connect(addr)
                .await
                .map_err(|e| format!("msgpack-rpc connect failed: {e}"))?,
        );
    }

    if cfg.warmup > 0 {
        let (warmed, _lats, _elapsed) =
            msgpack_window(conns, spec.depth, cfg.warmup, &request).await?;
        conns = warmed;
    }
    let before = handle.snapshot();
    let mut reps = Vec::with_capacity(cfg.repetitions);
    for _ in 0..cfg.repetitions {
        let (returned, mut lats, elapsed) =
            msgpack_window(conns, spec.depth, cfg.ops, &request).await?;
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
async fn msgpack_window(
    conns: Vec<MsgpackConn>,
    depth: usize,
    total_ops: usize,
    request: &Arc<Vec<u8>>,
) -> Result<(Vec<MsgpackConn>, Vec<Duration>, Duration), String> {
    let per_conn = (total_ops / conns.len().max(1)).max(depth).max(1);
    let started = Instant::now();
    let mut handles = Vec::with_capacity(conns.len());
    for conn in conns {
        let request = Arc::clone(request);
        handles.push(tokio::spawn(msgpack_conn_window(
            conn, depth, per_conn, request,
        )));
    }
    let mut returned = Vec::with_capacity(handles.len());
    let mut all = Vec::with_capacity(per_conn * handles.len());
    for handle in handles {
        let (conn, lats) = handle
            .await
            .map_err(|e| format!("msgpack-rpc worker panicked: {e}"))??;
        returned.push(conn);
        all.extend(lats);
    }
    Ok((returned, all, started.elapsed()))
}

/// FIFO pipeline window on one connection (BEN-003), same shape as the RESP3,
/// Memcached, MongoDB and PostgreSQL drivers.
async fn msgpack_conn_window(
    mut conn: MsgpackConn,
    depth: usize,
    ops: usize,
    request: Arc<Vec<u8>>,
) -> Result<(MsgpackConn, Vec<Duration>), String> {
    let window = Arc::new(Semaphore::new(depth.max(1)));
    let pending: Arc<StdMutex<VecDeque<(Instant, OwnedSemaphorePermit)>>> =
        Arc::new(StdMutex::new(VecDeque::with_capacity(depth.max(1))));
    let writer = &mut conn.writer;
    let reader = &mut conn.reader;
    let buf = &mut conn.buf;
    let start = &mut conn.start;
    let chunk = conn.chunk.as_mut();

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
                    .map_err(|e| format!("msgpack-rpc write failed: {e}"))?;
            }
            Ok::<(), String>(())
        }
    };
    let receiver = {
        let pending = Arc::clone(&pending);
        async move {
            let mut lats = Vec::with_capacity(ops);
            for _ in 0..ops {
                read_one(reader, buf, start, chunk).await?;
                let (sent, permit) = lock(&pending)
                    .pop_front()
                    .ok_or_else(|| "msgpack-rpc reply without a pending request".to_owned())?;
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

/// Read + consume one reply from the connection's stream buffer. Split out of
/// [`MsgpackConn::read_reply`] so the pipeline window can borrow the read half
/// and the write half independently.
async fn read_one(
    reader: &mut OwnedReadHalf,
    buf: &mut Vec<u8>,
    start: &mut usize,
    chunk: &mut [u8; READ_CHUNK],
) -> Result<(), String> {
    loop {
        match scan_value(buf, *start) {
            Ok(end) => {
                *start = end;
                if *start == buf.len() {
                    buf.clear();
                    *start = 0;
                }
                return Ok(());
            }
            Err(ScanError::Malformed) => return Err("msgpack-rpc malformed reply".to_owned()),
            Err(ScanError::NeedMore) => {
                if *start > 0 {
                    buf.drain(..*start);
                    *start = 0;
                }
                if buf.len() > MAX_BUFFERED {
                    return Err("msgpack-rpc reply exceeded the buffer cap".to_owned());
                }
                let n = reader
                    .read(chunk)
                    .await
                    .map_err(|e| format!("msgpack-rpc read failed: {e}"))?;
                if n == 0 {
                    return Err("msgpack-rpc connection closed".to_owned());
                }
                buf.extend_from_slice(&chunk[..n]);
            }
        }
    }
}

/// The connection-storm cell: connect + one request + first reply, repeated.
pub async fn storm(
    handle: &MsgpackHandle,
    storms: usize,
    cfg: &RunConfig,
) -> Result<Measured, String> {
    let addr = handle.local_addr();
    let request = build_msgpack_request("PING", &[])?;
    for _ in 0..cfg.warmup.min(storms) {
        msgpack_storm_once(addr, &request).await?;
    }
    let before = handle.snapshot();
    let mut reps = Vec::with_capacity(cfg.repetitions);
    for _ in 0..cfg.repetitions {
        let mut lats = Vec::with_capacity(storms);
        let started = Instant::now();
        for _ in 0..storms {
            lats.push(msgpack_storm_once(addr, &request).await?);
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

async fn msgpack_storm_once(addr: SocketAddr, request: &[u8]) -> Result<Duration, String> {
    let started = Instant::now();
    let mut conn = MsgpackConn::connect(addr)
        .await
        .map_err(|e| format!("storm connect failed: {e}"))?;
    conn.writer
        .write_all(request)
        .await
        .map_err(|e| format!("storm write failed: {e}"))?;
    conn.read_reply().await?;
    Ok(started.elapsed())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::backend::STATIC_REPLY_BYTES;

    /// Decode a response into `(msgid, error, result)`.
    fn decode_response(bytes: &[u8]) -> (u32, Value, Value) {
        let (kind, msgid, error, result): (u8, u32, Value, Value) =
            rmp_serde::from_slice(bytes).unwrap();
        assert_eq!(kind, TYPE_RESPONSE);
        (msgid, error, result)
    }

    #[test]
    fn request_round_trips() {
        let bytes = encode_request(7, "ECHO", &[Value::Str("hello".to_owned())]).unwrap();
        let (msgid, method, params) = decode_request(&bytes).unwrap();
        assert_eq!(msgid, 7);
        assert_eq!(method, "ECHO");
        assert_eq!(params, vec![Value::Str("hello".to_owned())]);
    }

    #[test]
    fn a_message_is_a_four_element_array_with_no_length_prefix() {
        let bytes = encode_request(1, "PING", &[]).unwrap();
        assert_eq!(bytes[0], 0x94, "first byte is fixarray(4), not a length");
        // The scanner must consume the message exactly — no more, no less.
        assert_eq!(scan_value(&bytes, 0).unwrap(), bytes.len());
    }

    #[test]
    fn the_reply_echoes_the_request_msgid() {
        let request = encode_request(4242, "ECHO", &[Value::Str("x".to_owned())]).unwrap();
        let (msgid, method, params) = decode_request(&request).unwrap();
        let backend = NoopBackend::new();
        let value = backend.respond(&method, params).unwrap();
        let reply = encode_response(msgid, Value::Null, value).unwrap();
        let (echoed, error, result) = decode_response(&reply);
        assert_eq!(echoed, 4242);
        assert_eq!(error, Value::Null);
        assert_eq!(result, Value::Str("x".to_owned()));
    }

    #[test]
    fn static_reply_is_4kib() {
        let request = encode_request(1, "STATIC", &[]).unwrap();
        let (msgid, method, params) = decode_request(&request).unwrap();
        let backend = NoopBackend::new();
        let value = backend.respond(&method, params).unwrap();
        let reply = encode_response(msgid, Value::Null, value).unwrap();
        let (_, _, result) = decode_response(&reply);
        match result {
            Value::Str(s) => assert_eq!(s.len(), STATIC_REPLY_BYTES),
            other => panic!("expected Str, got {other:?}"),
        }
    }

    #[test]
    fn the_scanner_finds_each_boundary_in_a_pipelined_stream() {
        let mut stream = Vec::new();
        let sizes = [1usize, 64, 4096];
        for (i, size) in sizes.iter().enumerate() {
            stream.extend_from_slice(
                &encode_request(i as u32, "ECHO", &[Value::Str("x".repeat(*size))]).unwrap(),
            );
        }
        let mut at = 0;
        for (i, size) in sizes.iter().enumerate() {
            let end = scan_value(&stream, at).unwrap();
            let (msgid, _, params) = decode_request(&stream[at..end]).unwrap();
            assert_eq!(msgid, i as u32);
            assert_eq!(params, vec![Value::Str("x".repeat(*size))]);
            at = end;
        }
        assert_eq!(at, stream.len(), "the stream is fully consumed");
    }

    #[test]
    fn a_truncated_message_asks_for_more_at_every_prefix() {
        let bytes = encode_request(1, "ECHO", &[Value::Str("x".repeat(300))]).unwrap();
        for cut in 1..bytes.len() {
            assert_eq!(
                scan_value(&bytes[..cut], 0),
                Err(ScanError::NeedMore),
                "prefix of {cut} bytes must be incomplete, not malformed"
            );
        }
        assert_eq!(scan_value(&bytes, 0), Ok(bytes.len()));
    }

    #[test]
    fn ext_types_are_refused_not_guessed() {
        // 0xd4 is fixext1 — a type this peer never emits.
        assert_eq!(
            scan_value(&[0xd4, 0x00, 0x00], 0),
            Err(ScanError::Malformed)
        );
    }

    #[test]
    fn bin_and_nil_payloads_scan() {
        let bytes = encode_request(1, "SINK", &[Value::Bytes(vec![0u8; 1000])]).unwrap();
        assert_eq!(scan_value(&bytes, 0).unwrap(), bytes.len());
        let reply = encode_response(1, Value::Null, Value::Null).unwrap();
        assert_eq!(scan_value(&reply, 0).unwrap(), reply.len());
    }
}

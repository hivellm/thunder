//! **Apache Thrift** lane — the real `thrift` crate's `TCompactProtocol` over
//! framed transport, serving the same no-op backend (BEN-001, BEN-002) in the
//! same process, host, runtime and allocator as the Thunder listener.
//!
//! # Why this lane
//!
//! Thrift is the other long-lived cross-language binary RPC, and
//! `TCompactProtocol` is the most aggressively **size-optimized** encoding in
//! the shootout: varint + zigzag integers, field ids written as *deltas* from
//! the previous field rather than absolutely, and booleans folded into their
//! own field header. Where MessagePack and protobuf each spend a byte per
//! small field tag, compact often spends less. This lane prices that
//! aggressiveness — what does squeezing the encoding buy, and what does it
//! cost to decode?
//!
//! Its framing is the interesting part for us: **framed transport is a 4-byte
//! big-endian length prefix**, the same shape as Thunder's `u32` (WIRE-001).
//! So Thrift and Thunder agree on framing and differ on encoding — the mirror
//! image of the MessagePack-RPC lane, which shares Thunder's encoding and
//! differs on framing. Between the two, both variables are isolated.
//!
//! # Real codec, our I/O — and why that is the honest split here
//!
//! The `thrift` crate ships a server (`TServer`), but it is **blocking**: a
//! thread-per-connection pool built on `std::io`. Running it would put a
//! second, different execution model in the process, which breaks BEN-001's
//! "same runtime" requirement far more seriously than a hand-rolled peer ever
//! could — the lane would be measuring thread-pool scheduling against async
//! tasks.
//!
//! So the split is drawn where it does the least damage: **the codec is the
//! real one** (`TCompactInputProtocol` / `TCompactOutputProtocol`, the actual
//! production encoders), and only the socket I/O and the length prefix are
//! ours. This costs nothing in fidelity, because the codec operates on
//! in-memory buffers either way: the frame is read asynchronously, then
//! decoded from a `Cursor` that never blocks. What a `TServer` would have
//! added is exactly the part we must not import.
//!
//! # Scope (honesty note, BEN-002)
//!
//! A **benchmark peer, not a Thrift service**. One method, `call`, with args
//! `{1: string command, 2: string payload}` returning `{0: string value}` —
//! hand-written where `thrift --gen rs` would have generated it, because
//! generating requires the Thrift compiler binary and the emitted encoding is
//! byte-identical to this. `command` selects the backend mode
//! (`ECHO`/`STATIC`/`SINK`/`PING`). No multiplexed protocol, no oneway calls,
//! no exceptions beyond the backend's error string, no service metadata.
//!
//! # Parity (BEN-003)
//!
//! The driver keeps a continuously-full in-flight window per connection (a
//! semaphore slot per outstanding request, replies matched FIFO) — Thrift is
//! FIFO per connection like RESP3/Memcached/OP_MSG/PostgreSQL. Server-side
//! bytes are counted at the socket after the successful write, the same
//! measurement point as every lane.

use std::collections::VecDeque;
use std::io::{self, Cursor};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use thrift::protocol::{
    TCompactInputProtocol, TCompactOutputProtocol, TFieldIdentifier, TInputProtocol,
    TMessageIdentifier, TMessageType, TOutputProtocol, TStructIdentifier, TType,
};
use thunder::wire::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{watch, OwnedSemaphorePermit, Semaphore};

use crate::backend::NoopBackend;
use crate::driver::{CellSpec, Measured, RunConfig};
use crate::stats::compute;

/// The one method this peer serves.
const METHOD: &str = "call";
/// Framed-transport prefix width — 4 bytes, big-endian, exactly Thunder's
/// shape (WIRE-001) with the opposite byte order.
const FRAME_PREFIX: usize = 4;
/// Frame cap — mirrors the Thunder frame cap (WIRE-020) so an oversized
/// length prefix cannot drive an unbounded allocation.
const MAX_FRAME: usize = thunder::wire::DEFAULT_MAX_FRAME_BYTES;

/// Ride through a poisoned lock: the guarded state stays consistent.
fn lock<T>(mutex: &StdMutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

// ── Encoding (the real TCompactProtocol, over in-memory buffers) ────────────

/// Encode a `call` request body: the message header plus
/// `{1: command, 2: payload}`.
fn encode_request(command: &str, payload: &str, sequence: i32) -> Result<Vec<u8>, String> {
    let mut buffer = Vec::with_capacity(payload.len() + 64);
    {
        let mut protocol = TCompactOutputProtocol::new(&mut buffer);
        protocol
            .write_message_begin(&TMessageIdentifier::new(
                METHOD,
                TMessageType::Call,
                sequence,
            ))
            .map_err(|e| format!("thrift write header failed: {e}"))?;
        protocol
            .write_struct_begin(&TStructIdentifier::new("call_args"))
            .map_err(|e| format!("thrift write struct failed: {e}"))?;
        write_string_field(&mut protocol, "command", 1, command)?;
        write_string_field(&mut protocol, "payload", 2, payload)?;
        protocol
            .write_field_stop()
            .map_err(|e| format!("thrift write stop failed: {e}"))?;
        protocol
            .write_struct_end()
            .map_err(|e| format!("thrift write struct end failed: {e}"))?;
        protocol
            .write_message_end()
            .map_err(|e| format!("thrift write message end failed: {e}"))?;
    }
    Ok(buffer)
}

/// Encode a `call` reply body: `{0: value}`.
fn encode_reply(value: &str, sequence: i32) -> Result<Vec<u8>, String> {
    let mut buffer = Vec::with_capacity(value.len() + 64);
    {
        let mut protocol = TCompactOutputProtocol::new(&mut buffer);
        protocol
            .write_message_begin(&TMessageIdentifier::new(
                METHOD,
                TMessageType::Reply,
                sequence,
            ))
            .map_err(|e| format!("thrift write header failed: {e}"))?;
        protocol
            .write_struct_begin(&TStructIdentifier::new("call_result"))
            .map_err(|e| format!("thrift write struct failed: {e}"))?;
        // Field 0 is the success return value, by Thrift convention.
        write_string_field(&mut protocol, "success", 0, value)?;
        protocol
            .write_field_stop()
            .map_err(|e| format!("thrift write stop failed: {e}"))?;
        protocol
            .write_struct_end()
            .map_err(|e| format!("thrift write struct end failed: {e}"))?;
        protocol
            .write_message_end()
            .map_err(|e| format!("thrift write message end failed: {e}"))?;
    }
    Ok(buffer)
}

/// Write one `id: string` field.
fn write_string_field(
    protocol: &mut TCompactOutputProtocol<&mut Vec<u8>>,
    name: &'static str,
    id: i16,
    value: &str,
) -> Result<(), String> {
    protocol
        .write_field_begin(&TFieldIdentifier::new(name, TType::String, id))
        .map_err(|e| format!("thrift write field failed: {e}"))?;
    protocol
        .write_string(value)
        .map_err(|e| format!("thrift write string failed: {e}"))?;
    protocol
        .write_field_end()
        .map_err(|e| format!("thrift write field end failed: {e}"))?;
    Ok(())
}

/// Decode a `call` request body into `(command, payload, sequence)`.
fn decode_request(bytes: &[u8]) -> Result<(String, String, i32), String> {
    let mut protocol = TCompactInputProtocol::new(Cursor::new(bytes));
    let header = protocol
        .read_message_begin()
        .map_err(|e| format!("thrift read header failed: {e}"))?;
    if header.message_type != TMessageType::Call {
        return Err(format!(
            "thrift unexpected message {:?}",
            header.message_type
        ));
    }
    protocol
        .read_struct_begin()
        .map_err(|e| format!("thrift read struct failed: {e}"))?;
    let mut command = String::new();
    let mut payload = String::new();
    loop {
        let field = protocol
            .read_field_begin()
            .map_err(|e| format!("thrift read field failed: {e}"))?;
        if field.field_type == TType::Stop {
            break;
        }
        match (field.id, field.field_type) {
            (Some(1), TType::String) => {
                command = protocol
                    .read_string()
                    .map_err(|e| format!("thrift read string failed: {e}"))?;
            }
            (Some(2), TType::String) => {
                payload = protocol
                    .read_string()
                    .map_err(|e| format!("thrift read string failed: {e}"))?;
            }
            _ => return Err("thrift unexpected field in call args".to_owned()),
        }
        protocol
            .read_field_end()
            .map_err(|e| format!("thrift read field end failed: {e}"))?;
    }
    protocol
        .read_struct_end()
        .map_err(|e| format!("thrift read struct end failed: {e}"))?;
    protocol
        .read_message_end()
        .map_err(|e| format!("thrift read message end failed: {e}"))?;
    Ok((command, payload, header.sequence_number))
}

/// Frame a body with the 4-byte big-endian length prefix.
fn frame(body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len() + FRAME_PREFIX);
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(body);
    out
}

/// A backend reply value as the reply's `success` field.
fn value_to_string(value: Value) -> String {
    match value {
        Value::Str(s) => s,
        Value::Bytes(b) => String::from_utf8_lossy(&b).into_owned(),
        _ => String::new(),
    }
}

/// Turn a request into backend args: ECHO carries the payload, the sentinels
/// carry nothing.
fn command_args(command: &str, payload: String) -> Vec<Value> {
    match command {
        "ECHO" if !payload.is_empty() => vec![Value::Str(payload)],
        _ => vec![],
    }
}

// ── Listener ────────────────────────────────────────────────────────────────

/// Server-side counters, sampled around a measured window.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ThriftMetricsSnapshot {
    /// Requests answered.
    pub requests: u64,
    /// Request bytes read off the wire.
    pub bytes_in: u64,
    /// Response bytes written to the wire.
    pub bytes_out: u64,
}

#[derive(Debug, Default)]
struct ThriftMetrics {
    requests: AtomicU64,
    bytes_in: AtomicU64,
    bytes_out: AtomicU64,
}

impl ThriftMetrics {
    fn snapshot(&self) -> ThriftMetricsSnapshot {
        ThriftMetricsSnapshot {
            requests: self.requests.load(Ordering::Relaxed),
            bytes_in: self.bytes_in.load(Ordering::Relaxed),
            bytes_out: self.bytes_out.load(Ordering::Relaxed),
        }
    }
}

/// Handle to the running Thrift listener — same shape as the other lanes.
#[derive(Debug)]
pub struct ThriftHandle {
    addr: SocketAddr,
    shutdown: watch::Sender<bool>,
    metrics: Arc<ThriftMetrics>,
    done: Option<tokio::sync::mpsc::Receiver<()>>,
}

impl ThriftHandle {
    /// The bound address.
    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    /// Current server-side counters.
    pub fn snapshot(&self) -> ThriftMetricsSnapshot {
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

impl Drop for ThriftHandle {
    fn drop(&mut self) {
        let _ = self.shutdown.send(true);
    }
}

/// Spawn the Thrift listener over the shared no-op backend.
pub async fn spawn_thrift_listener(
    backend: Arc<NoopBackend>,
    addr: SocketAddr,
) -> io::Result<ThriftHandle> {
    let listener = TcpListener::bind(addr).await?;
    let addr = listener.local_addr()?;
    let metrics = Arc::new(ThriftMetrics::default());
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

    Ok(ThriftHandle {
        addr,
        shutdown: shutdown_tx,
        metrics,
        done: Some(done_rx),
    })
}

/// One connection: read a framed request, decode with the real compact
/// protocol, answer via the backend, write the framed reply. Drain-then-flush
/// mirrors every other lane (SRV-006 analog).
async fn serve_conn(
    stream: TcpStream,
    backend: Arc<NoopBackend>,
    metrics: Arc<ThriftMetrics>,
    mut shutdown: watch::Receiver<bool>,
) {
    let _ = stream.set_nodelay(true);
    let (read_half, write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);
    let mut writer = BufWriter::new(write_half);

    loop {
        let mut prefix = [0u8; FRAME_PREFIX];
        let read = tokio::select! {
            _ = shutdown.wait_for(|stop| *stop) => break,
            read = reader.read_exact(&mut prefix) => read,
        };
        if read.is_err() {
            break;
        }
        let len = u32::from_be_bytes(prefix) as usize;
        if len == 0 || len > MAX_FRAME {
            break;
        }
        let mut body = vec![0u8; len];
        if reader.read_exact(&mut body).await.is_err() {
            break;
        }
        metrics
            .bytes_in
            .fetch_add((len + FRAME_PREFIX) as u64, Ordering::Relaxed);

        let Ok((command, payload, sequence)) = decode_request(&body) else {
            break;
        };
        let value = match backend.respond(&command, command_args(&command, payload)) {
            Ok(value) => value_to_string(value),
            Err(message) => message,
        };
        let Ok(reply) = encode_reply(&value, sequence) else {
            break;
        };
        let framed = frame(&reply);
        if writer.write_all(&framed).await.is_err() {
            break;
        }
        metrics
            .bytes_out
            .fetch_add(framed.len() as u64, Ordering::Relaxed);
        metrics.requests.fetch_add(1, Ordering::Relaxed);

        if reader.buffer().is_empty() && writer.flush().await.is_err() {
            break;
        }
    }
    let _ = writer.flush().await;
}

// ── Driver ────────────────────────────────────────────────────────────────

/// One driver connection: a raw write half (direct writes, nodelay) and a
/// buffered read half.
struct ThriftConn {
    reader: BufReader<OwnedReadHalf>,
    writer: OwnedWriteHalf,
}

impl ThriftConn {
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

/// Read + fully consume one framed reply (the measurement point, BEN-003).
async fn read_reply(reader: &mut BufReader<OwnedReadHalf>) -> Result<(), String> {
    let mut prefix = [0u8; FRAME_PREFIX];
    reader
        .read_exact(&mut prefix)
        .await
        .map_err(|e| format!("thrift read failed: {e}"))?;
    let len = u32::from_be_bytes(prefix) as usize;
    if len == 0 || len > MAX_FRAME {
        return Err(format!("thrift frame length {len} out of range"));
    }
    let mut body = vec![0u8; len];
    reader
        .read_exact(&mut body)
        .await
        .map_err(|e| format!("thrift body read failed: {e}"))?;
    Ok(())
}

/// Build one framed request from the matrix `(command, args)`.
fn build_thrift_request(command: &str, args: &[Value]) -> Result<Vec<u8>, String> {
    let payload = match args.first() {
        Some(Value::Str(s)) => s.clone(),
        Some(Value::Bytes(b)) => String::from_utf8(b.clone())
            .map_err(|_| "thrift lane: string payloads must be UTF-8".to_owned())?,
        Some(other) => return Err(format!("thrift lane: unsupported arg {other:?}")),
        None => String::new(),
    };
    Ok(frame(&encode_request(command, &payload, 1)?))
}

/// Measure one matrix cell on the Thrift lane.
pub async fn cell(
    handle: &ThriftHandle,
    spec: &CellSpec,
    cfg: &RunConfig,
) -> Result<Measured, String> {
    let addr = handle.local_addr();
    let request = Arc::new(build_thrift_request(spec.command, &spec.args)?);
    let mut conns = Vec::with_capacity(spec.connections);
    for _ in 0..spec.connections {
        conns.push(
            ThriftConn::connect(addr)
                .await
                .map_err(|e| format!("thrift connect failed: {e}"))?,
        );
    }

    if cfg.warmup > 0 {
        let (warmed, _lats, _elapsed) =
            thrift_window(conns, spec.depth, cfg.warmup, &request).await?;
        conns = warmed;
    }
    let before = handle.snapshot();
    let mut reps = Vec::with_capacity(cfg.repetitions);
    for _ in 0..cfg.repetitions {
        let (returned, mut lats, elapsed) =
            thrift_window(conns, spec.depth, cfg.ops, &request).await?;
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
async fn thrift_window(
    conns: Vec<ThriftConn>,
    depth: usize,
    total_ops: usize,
    request: &Arc<Vec<u8>>,
) -> Result<(Vec<ThriftConn>, Vec<Duration>, Duration), String> {
    let per_conn = (total_ops / conns.len().max(1)).max(depth).max(1);
    let started = Instant::now();
    let mut handles = Vec::with_capacity(conns.len());
    for conn in conns {
        let request = Arc::clone(request);
        handles.push(tokio::spawn(thrift_conn_window(
            conn, depth, per_conn, request,
        )));
    }
    let mut returned = Vec::with_capacity(handles.len());
    let mut all = Vec::with_capacity(per_conn * handles.len());
    for handle in handles {
        let (conn, lats) = handle
            .await
            .map_err(|e| format!("thrift worker panicked: {e}"))??;
        returned.push(conn);
        all.extend(lats);
    }
    Ok((returned, all, started.elapsed()))
}

/// FIFO pipeline window on one connection (BEN-003), same shape as the RESP3,
/// Memcached, MongoDB, PostgreSQL and MessagePack-RPC drivers.
async fn thrift_conn_window(
    mut conn: ThriftConn,
    depth: usize,
    ops: usize,
    request: Arc<Vec<u8>>,
) -> Result<(ThriftConn, Vec<Duration>), String> {
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
                    .map_err(|e| format!("thrift write failed: {e}"))?;
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
                    .ok_or_else(|| "thrift reply without a pending request".to_owned())?;
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
    handle: &ThriftHandle,
    storms: usize,
    cfg: &RunConfig,
) -> Result<Measured, String> {
    let addr = handle.local_addr();
    let request = build_thrift_request("PING", &[])?;
    for _ in 0..cfg.warmup.min(storms) {
        thrift_storm_once(addr, &request).await?;
    }
    let before = handle.snapshot();
    let mut reps = Vec::with_capacity(cfg.repetitions);
    for _ in 0..cfg.repetitions {
        let mut lats = Vec::with_capacity(storms);
        let started = Instant::now();
        for _ in 0..storms {
            lats.push(thrift_storm_once(addr, &request).await?);
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

async fn thrift_storm_once(addr: SocketAddr, request: &[u8]) -> Result<Duration, String> {
    let started = Instant::now();
    let mut conn = ThriftConn::connect(addr)
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

    /// Decode a reply body into its `success` value.
    fn decode_reply(bytes: &[u8]) -> String {
        let mut protocol = TCompactInputProtocol::new(Cursor::new(bytes));
        let header = protocol.read_message_begin().unwrap();
        assert_eq!(header.message_type, TMessageType::Reply);
        protocol.read_struct_begin().unwrap();
        let field = protocol.read_field_begin().unwrap();
        assert_eq!(field.id, Some(0), "success is field 0 by convention");
        let value = protocol.read_string().unwrap();
        protocol.read_field_end().unwrap();
        value
    }

    #[test]
    fn request_round_trips_through_the_real_compact_protocol() {
        let body = encode_request("ECHO", "hello world", 7).unwrap();
        let (command, payload, sequence) = decode_request(&body).unwrap();
        assert_eq!(command, "ECHO");
        assert_eq!(payload, "hello world");
        assert_eq!(sequence, 7);
    }

    #[test]
    fn the_frame_prefix_is_a_big_endian_u32_of_the_body() {
        let framed = build_thrift_request("ECHO", &[Value::Str("xyz".to_owned())]).unwrap();
        let declared = u32::from_be_bytes([framed[0], framed[1], framed[2], framed[3]]) as usize;
        assert_eq!(declared, framed.len() - FRAME_PREFIX);
    }

    #[test]
    fn echo_reply_carries_the_payload() {
        let body = encode_request("ECHO", &"x".repeat(64), 1).unwrap();
        let (command, payload, sequence) = decode_request(&body).unwrap();
        let backend = NoopBackend::new();
        let value = backend
            .respond(&command, command_args(&command, payload))
            .unwrap();
        let reply = encode_reply(&value_to_string(value), sequence).unwrap();
        assert_eq!(decode_reply(&reply), "x".repeat(64));
    }

    #[test]
    fn static_reply_is_4kib() {
        let body = encode_request("STATIC", "", 1).unwrap();
        let (command, payload, sequence) = decode_request(&body).unwrap();
        let backend = NoopBackend::new();
        let value = backend
            .respond(&command, command_args(&command, payload))
            .unwrap();
        let reply = encode_reply(&value_to_string(value), sequence).unwrap();
        assert_eq!(decode_reply(&reply).len(), STATIC_REPLY_BYTES);
    }

    /// The point of TCompactProtocol: a bare call should be genuinely small.
    /// Field ids are written as deltas and lengths as varints, so a
    /// two-field message with one empty string costs only a handful of bytes
    /// beyond the method name.
    #[test]
    fn a_bare_call_is_compact() {
        let body = encode_request("PING", "", 1).unwrap();
        assert!(
            body.len() < 24,
            "compact encoding of a bare call grew to {} bytes",
            body.len()
        );
    }

    #[test]
    fn non_utf8_payloads_are_refused_not_mangled() {
        let err = build_thrift_request("ECHO", &[Value::Bytes(vec![0xff, 0xfe])]).unwrap_err();
        assert!(err.contains("UTF-8"), "unexpected error: {err}");
    }
}

//! RESP3 lane (BEN-001) — a RESP3 listener over the shared
//! [`NoopBackend`](crate::backend::NoopBackend), plus a parity driver, so
//! Thunder is measured against the Redis/Synap convention on the same
//! host, process, runtime and allocator.
//!
//! The contract mirrors [`crate::http`] exactly — same handle shape, same
//! server-side byte counters recorded after successful writes, same
//! continuously-full pipeline window — because parity is the whole point
//! (BEN-003).
//!
//! # Scope — a benchmark peer, not a Redis product (BEN-002)
//!
//! This module implements exactly the RESP3 surface the BEN-010 matrix
//! needs, and nothing else:
//!
//! | Direction | Supported | Out of scope |
//! |---|---|---|
//! | request | arrays of bulk strings (`*N\r\n$len\r\n<bytes>\r\n…`) | inline commands (`PING\r\n`), `HELLO`, `AUTH`, `SUBSCRIBE`/push, `MULTI`, RESP2 negotiation |
//! | reply | simple string (`+`), error (`-`), bulk string (`$`), null (`_`), number (`:`), boolean (`#`), double (`,`), array (`*`), map (`%`) | verbatim (`=`), big number (`(`), set (`~`), attribute (`\|`), push (`>`), streamed aggregates |
//!
//! There is no keyspace, no `SET`/`GET`, no config, no RESP2 fallback: the
//! commands are exactly the shared backend's (`PING` / `ECHO` / `STATIC` /
//! `SINK`, matched case-insensitively as Redis does).
//!
//! # Command and value mapping
//!
//! The RESP3 command name is upper-cased and handed straight to
//! [`NoopBackend::respond`], so every lane answers identically. Two
//! deliberate, documented conversions:
//!
//! - **args in**: a bulk string is [`Value::Str`] when it is valid UTF-8,
//!   [`Value::Bytes`] otherwise. This is lossy in one direction — a
//!   caller's `Bytes` payload that happens to be valid UTF-8 arrives as
//!   `Str`. RESP3 has one bulk-string type and carries no such tag, so no
//!   wire format could preserve it; the matrix's commands (`ECHO` echoes,
//!   `SINK` drops) are indifferent to which variant they get, and the
//!   bytes on the wire — the measured quantity — are identical either way.
//! - **reply out**: `Value::Str` and `Value::Bytes` both encode as bulk
//!   strings, which is the honest RESP3 answer (again: one type). The one
//!   exception is a bare `PING`, whose `PONG` is encoded as a simple
//!   string (`+PONG\r\n`) to match real Redis byte-for-byte — this is the
//!   reply a calibration client checks.
//!
//! # Calibration (BEN-003) — **UNRUN**
//!
//! SPEC-007 requires this driver be validated against `redis-benchmark` on
//! this listener before its numbers are trusted. **That calibration has
//! not been run**, and the lane's numbers must not be trusted at G5 until
//! it is. What a calibration run would exercise, and what it would hit:
//!
//! - `redis-benchmark -p <port> -t ping_mbulk -P <depth> -c <conns> -n <ops>`
//!   — **works today**. It sends `*1\r\n$4\r\nPING\r\n` and reads
//!   `+PONG\r\n`, byte-identical to Redis. This is the calibration
//!   command: its qps should land within noise of this module's
//!   `point-echo-64B` / `pipelined-1k` cells at the same `-P` and `-c`.
//! - `redis-benchmark -t ping_inline` — **would fail**: inline commands
//!   are out of scope (see the table above); the listener answers
//!   `-ERR …` and closes.
//! - `redis-benchmark -t set` (and `get`, `incr`, `lpush`, …) — **would
//!   fail**: the shared no-op backend has no keyspace, so `SET` returns
//!   `-ERR unknown command 'SET'`. Adding a `SET` alias would put a fake
//!   command in the measurement (BEN-001: the engine must never be in the
//!   measurement), so it is deliberately absent. Calibration is
//!   `ping_mbulk` only.
//! - `redis-benchmark -3` (RESP3 mode) — **would fail**: it opens with
//!   `HELLO 3`, which is out of scope. Calibration runs in the default
//!   RESP2 mode; every reply type this listener actually emits for the
//!   matrix (`+`, `-`, `$`) is identical in RESP2 and RESP3, so the
//!   protocol version does not move the measured bytes. `_\r\n` (null) is
//!   RESP3-only and is never produced by the calibration command.
//! - `redis-benchmark`'s startup `CONFIG GET` probe gets `-ERR unknown
//!   command 'CONFIG'`; the tool prints a warning and proceeds.

use std::collections::VecDeque;
use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use thunder::wire::Value;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, watch, OwnedSemaphorePermit, Semaphore};

use crate::backend::NoopBackend;
use crate::driver::{CellSpec, Measured, RunConfig};
use crate::stats::compute;

/// Cap on one bulk string — mirrors the Thunder frame cap (WIRE-020).
const MAX_RESP3_BULK: usize = thunder::wire::DEFAULT_MAX_FRAME_BYTES;

/// Cap on one protocol line (`*N`, `$N`, `+…`, `-…`).
const MAX_RESP3_LINE: usize = 16 * 1024;

/// Cap on the element count of one request array (command + args).
const MAX_RESP3_ARGS: i64 = 1024;

/// Cap on the element count of one decoded aggregate reply — the driver
/// side's mirror of [`MAX_RESP3_ARGS`].
const MAX_RESP3_ELEMENTS: i64 = 1 << 20;

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

// ── Metrics ──────────────────────────────────────────────────────────────────

/// Server-side counters for the RESP3 lane — the same measurement point as
/// the Thunder listener's SRV-030 metrics (bytes counted at the socket,
/// recorded after the successful write).
#[derive(Debug, Default)]
struct Resp3Metrics {
    bytes_in: AtomicU64,
    bytes_out: AtomicU64,
    requests: AtomicU64,
}

impl Resp3Metrics {
    fn record_in(&self, bytes: usize) {
        self.bytes_in.fetch_add(bytes as u64, Ordering::Relaxed);
    }

    fn record_response(&self, bytes: usize) {
        self.bytes_out.fetch_add(bytes as u64, Ordering::Relaxed);
        self.requests.fetch_add(1, Ordering::Relaxed);
    }

    fn snapshot(&self) -> Resp3MetricsSnapshot {
        Resp3MetricsSnapshot {
            requests: self.requests.load(Ordering::Relaxed),
            bytes_in: self.bytes_in.load(Ordering::Relaxed),
            bytes_out: self.bytes_out.load(Ordering::Relaxed),
        }
    }
}

/// Server-side counters, sampled around a measured window.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Resp3MetricsSnapshot {
    /// Requests answered.
    pub requests: u64,
    /// Request bytes read off the wire.
    pub bytes_in: u64,
    /// Response bytes written to the wire (counted after the write).
    pub bytes_out: u64,
}

// ── Handle / listener ────────────────────────────────────────────────────────

/// Handle to the running RESP3 listener.
#[derive(Debug)]
pub struct Resp3Handle {
    addr: SocketAddr,
    shutdown: watch::Sender<bool>,
    metrics: Arc<Resp3Metrics>,
    done: Option<mpsc::Receiver<()>>,
}

impl Resp3Handle {
    /// The bound address.
    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    /// Current server-side counters.
    pub fn snapshot(&self) -> Resp3MetricsSnapshot {
        self.metrics.snapshot()
    }

    /// Graceful shutdown: stop accepting, let connections drain, resolve
    /// once every connection task is gone.
    pub async fn stop(mut self) {
        let _ = self.shutdown.send(true);
        if let Some(mut done) = self.done.take() {
            let _ = done.recv().await;
        }
    }
}

impl Drop for Resp3Handle {
    fn drop(&mut self) {
        let _ = self.shutdown.send(true);
    }
}

/// Spawn the RESP3 listener over the shared no-op backend.
pub async fn spawn_resp3_listener(
    backend: Arc<NoopBackend>,
    addr: SocketAddr,
) -> io::Result<Resp3Handle> {
    let listener = TcpListener::bind(addr).await?;
    let local_addr = listener.local_addr()?;
    let metrics = Arc::new(Resp3Metrics::default());
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let (done_tx, done_rx) = mpsc::channel::<()>(1);
    tokio::spawn(accept_loop(
        listener,
        backend,
        Arc::clone(&metrics),
        shutdown_rx,
        done_tx,
    ));
    Ok(Resp3Handle {
        addr: local_addr,
        shutdown: shutdown_tx,
        metrics,
        done: Some(done_rx),
    })
}

/// Accept until shutdown; one task per connection, accept errors are
/// transient (parity with the Thunder listener's SRV-004 posture).
async fn accept_loop(
    listener: TcpListener,
    backend: Arc<NoopBackend>,
    metrics: Arc<Resp3Metrics>,
    shutdown: watch::Receiver<bool>,
    done: mpsc::Sender<()>,
) {
    let mut accept_shutdown = shutdown.clone();
    loop {
        let accepted = tokio::select! {
            _ = accept_shutdown.wait_for(|stop| *stop) => break,
            accepted = listener.accept() => accepted,
        };
        let Ok((stream, _peer)) = accepted else {
            continue;
        };
        let backend = Arc::clone(&backend);
        let metrics = Arc::clone(&metrics);
        let conn_shutdown = shutdown.clone();
        let done_guard = done.clone();
        tokio::spawn(async move {
            handle_connection(stream, backend, metrics, conn_shutdown).await;
            drop(done_guard);
        });
    }
}

/// One connection: sequential command → reply loop (RESP3 is strictly
/// ordered request/response), with the flush deferred while another
/// pipelined command is already buffered (SRV-006 analog).
async fn handle_connection(
    stream: TcpStream,
    backend: Arc<NoopBackend>,
    metrics: Arc<Resp3Metrics>,
    mut shutdown: watch::Receiver<bool>,
) {
    // Parity with the Thunder listener: Nagle off (SRV-008).
    let _ = stream.set_nodelay(true);
    let (read_half, write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);
    let mut writer = BufWriter::new(write_half);
    let mut scratch = Vec::with_capacity(8 * 1024);
    loop {
        let read = tokio::select! {
            _ = shutdown.wait_for(|stop| *stop) => break,
            read = read_command(&mut reader) => read,
        };
        let (command, args, bytes) = match read {
            Ok(Some(request)) => request,
            // Clean EOF between commands: the client is done.
            Ok(None) => break,
            Err(error) => {
                // Protocol error: best-effort `-ERR`, then close — Redis
                // closes on a framing error too.
                let reply = Reply::Error(format!("ERR protocol error: {error}"));
                let _ = write_reply(&mut writer, &reply, &metrics, &mut scratch).await;
                let _ = writer.flush().await;
                break;
            }
        };
        metrics.record_in(bytes);
        let reply = respond(&backend, &command, args);
        if write_reply(&mut writer, &reply, &metrics, &mut scratch)
            .await
            .is_err()
        {
            break;
        }
        // Drain-then-flush analog: skip the flush while another pipelined
        // command is already sitting in the read buffer.
        if reader.buffer().is_empty() && writer.flush().await.is_err() {
            break;
        }
    }
    let _ = writer.flush().await;
}

/// Route one command through the shared backend.
///
/// Dispatch errors travel in-band as RESP3 errors, like Thunder's
/// `Response::err` — the connection stays usable.
fn respond(backend: &NoopBackend, command: &str, args: Vec<Vec<u8>>) -> Reply {
    let bare_ping = command == "PING" && args.is_empty();
    let args = args.into_iter().map(bulk_to_value).collect();
    match backend.respond(command, args) {
        // Real-Redis fidelity for the one reply a calibration client
        // checks: bare PING answers `+PONG\r\n`, not the bulk form.
        Ok(Value::Str(text)) if bare_ping => Reply::Simple(text),
        Ok(value) => value_to_reply(&value),
        Err(message) => Reply::Error(message),
    }
}

/// Write one reply; bytes recorded after the successful write (SRV-030
/// parity). The flush policy belongs to the caller.
async fn write_reply(
    writer: &mut BufWriter<OwnedWriteHalf>,
    reply: &Reply,
    metrics: &Resp3Metrics,
    scratch: &mut Vec<u8>,
) -> io::Result<()> {
    scratch.clear();
    reply.encode(scratch);
    writer.write_all(scratch).await?;
    metrics.record_response(scratch.len());
    Ok(())
}

// ── RESP3 value model ────────────────────────────────────────────────────────

/// The RESP3 reply types this bench peer speaks (see the module scope
/// table). One type per encoder branch — nothing is inferred.
#[derive(Debug, Clone, PartialEq)]
enum Reply {
    /// `+<text>\r\n`
    Simple(String),
    /// `-<message>\r\n`
    Error(String),
    /// `:<n>\r\n`
    Int(i64),
    /// `#t\r\n` / `#f\r\n`
    Bool(bool),
    /// `,<double>\r\n`
    Double(f64),
    /// `$<len>\r\n<bytes>\r\n`
    Bulk(Vec<u8>),
    /// `_\r\n` (RESP3 null; `$-1\r\n` and `*-1\r\n` decode to this too)
    Null,
    /// `*<n>\r\n<element>…`
    Array(Vec<Reply>),
    /// `%<n>\r\n<key><value>…`
    Map(Vec<(Reply, Reply)>),
}

impl Reply {
    /// Encode into RESP3 wire bytes, appending to `out`.
    fn encode(&self, out: &mut Vec<u8>) {
        match self {
            Self::Simple(text) => encode_line(b'+', text.as_bytes(), out),
            Self::Error(message) => encode_line(b'-', message.as_bytes(), out),
            Self::Int(n) => encode_line(b':', n.to_string().as_bytes(), out),
            Self::Bool(flag) => out.extend_from_slice(if *flag { b"#t\r\n" } else { b"#f\r\n" }),
            Self::Double(d) => encode_line(b',', double_text(*d).as_bytes(), out),
            Self::Bulk(bytes) => {
                encode_line(b'$', bytes.len().to_string().as_bytes(), out);
                out.extend_from_slice(bytes);
                out.extend_from_slice(b"\r\n");
            }
            Self::Null => out.extend_from_slice(b"_\r\n"),
            Self::Array(items) => {
                encode_line(b'*', items.len().to_string().as_bytes(), out);
                for item in items {
                    item.encode(out);
                }
            }
            Self::Map(pairs) => {
                encode_line(b'%', pairs.len().to_string().as_bytes(), out);
                for (key, value) in pairs {
                    key.encode(out);
                    value.encode(out);
                }
            }
        }
    }
}

fn encode_line(tag: u8, body: &[u8], out: &mut Vec<u8>) {
    out.push(tag);
    out.extend_from_slice(body);
    out.extend_from_slice(b"\r\n");
}

/// RESP3 double text: `inf` / `-inf` / `nan` are spelled out, everything
/// else uses Rust's shortest round-trip form.
fn double_text(value: f64) -> String {
    if value.is_nan() {
        "nan".to_owned()
    } else if value.is_infinite() {
        if value.is_sign_negative() {
            "-inf".to_owned()
        } else {
            "inf".to_owned()
        }
    } else {
        value.to_string()
    }
}

/// Map a backend [`Value`] onto a RESP3 reply. `Str` and `Bytes` both
/// become bulk strings — RESP3 has exactly one bulk-string type, so the
/// distinction is not representable on this wire (module docs).
fn value_to_reply(value: &Value) -> Reply {
    match value {
        Value::Null => Reply::Null,
        Value::Bool(flag) => Reply::Bool(*flag),
        Value::Int(n) => Reply::Int(*n),
        Value::Float(d) => Reply::Double(*d),
        Value::Bytes(bytes) => Reply::Bulk(bytes.clone()),
        Value::Str(text) => Reply::Bulk(text.as_bytes().to_vec()),
        Value::Array(items) => Reply::Array(items.iter().map(value_to_reply).collect()),
        Value::Map(pairs) => Reply::Map(
            pairs
                .iter()
                .map(|(k, v)| (value_to_reply(k), value_to_reply(v)))
                .collect(),
        ),
    }
}

/// Map one inbound bulk string onto the backend's value model: `Str` when
/// it is valid UTF-8, `Bytes` otherwise (module docs — RESP3 carries no
/// tag to distinguish them).
fn bulk_to_value(bulk: Vec<u8>) -> Value {
    match String::from_utf8(bulk) {
        Ok(text) => Value::Str(text),
        Err(error) => Value::Bytes(error.into_bytes()),
    }
}

/// Serialize one call as a RESP3 command: an array of bulk strings.
///
/// Aggregate arguments are rejected rather than silently flattened — a
/// RESP3 command is a flat array of bulk strings, and no matrix scenario
/// sends one (`request_for` yields `Str` only).
fn build_resp3_request(command: &str, args: &[Value]) -> Result<Vec<u8>, String> {
    let mut parts = Vec::with_capacity(args.len() + 1);
    parts.push(command.as_bytes().to_vec());
    for arg in args {
        parts.push(arg_bytes(arg)?);
    }
    let mut out = Vec::with_capacity(16 + parts.iter().map(|p| p.len() + 16).sum::<usize>());
    encode_line(b'*', parts.len().to_string().as_bytes(), &mut out);
    for part in &parts {
        Reply::Bulk(part.clone()).encode(&mut out);
    }
    Ok(out)
}

fn arg_bytes(arg: &Value) -> Result<Vec<u8>, String> {
    match arg {
        Value::Str(text) => Ok(text.as_bytes().to_vec()),
        Value::Bytes(bytes) => Ok(bytes.clone()),
        Value::Null => Ok(Vec::new()),
        Value::Bool(flag) => Ok(if *flag { b"1".to_vec() } else { b"0".to_vec() }),
        Value::Int(n) => Ok(n.to_string().into_bytes()),
        Value::Float(d) => Ok(double_text(*d).into_bytes()),
        Value::Array(_) | Value::Map(_) => Err(
            "resp3 lane sends flat bulk-string arguments; aggregate arguments are out of scope"
                .to_owned(),
        ),
    }
}

// ── Decoding ─────────────────────────────────────────────────────────────────

/// Read one CRLF-terminated protocol line. `Ok(None)` is a clean EOF
/// before the first byte. Returns the line without its CRLF, plus the
/// bytes consumed from the socket.
async fn read_line<R: AsyncBufRead + Unpin>(
    reader: &mut R,
) -> io::Result<Option<(Vec<u8>, usize)>> {
    let mut line = Vec::new();
    let read = reader.read_until(b'\n', &mut line).await?;
    if read == 0 {
        return Ok(None);
    }
    if read > MAX_RESP3_LINE {
        return Err(invalid("resp3 line too long"));
    }
    if line.last() != Some(&b'\n') {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "eof inside resp3 line",
        ));
    }
    line.pop();
    if line.last() == Some(&b'\r') {
        line.pop();
    }
    Ok(Some((line, read)))
}

fn parse_count(body: &[u8]) -> io::Result<i64> {
    std::str::from_utf8(body)
        .ok()
        .and_then(|text| text.parse::<i64>().ok())
        .ok_or_else(|| invalid("resp3 length is not an integer"))
}

/// Read one bulk string body (`$<len>\r\n<bytes>\r\n`), given its already
/// parsed length. Returns the bytes and the bytes consumed.
async fn read_bulk_body<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    len: i64,
) -> io::Result<(Vec<u8>, usize)> {
    let len = usize::try_from(len).map_err(|_| invalid("negative bulk length"))?;
    if len > MAX_RESP3_BULK {
        return Err(invalid("resp3 bulk string too large"));
    }
    let mut body = vec![0u8; len + 2];
    reader.read_exact(&mut body).await?;
    if &body[len..] != b"\r\n" {
        return Err(invalid("resp3 bulk string is not CRLF terminated"));
    }
    body.truncate(len);
    Ok((body, len + 2))
}

/// Read one command: an array of bulk strings. Returns the upper-cased
/// command name, its arguments, and the total bytes consumed (the
/// `bytes_in` metric). `Ok(None)` is a clean EOF between commands.
async fn read_command<R: AsyncBufRead + Unpin>(
    reader: &mut R,
) -> io::Result<Option<(String, Vec<Vec<u8>>, usize)>> {
    let Some((line, head_bytes)) = read_line(reader).await? else {
        return Ok(None);
    };
    let mut bytes = head_bytes;
    let Some((b'*', count)) = line.split_first() else {
        return Err(invalid(
            "resp3 bench peer accepts only arrays of bulk strings (inline commands are out of scope)",
        ));
    };
    let count = parse_count(count)?;
    if !(1..=MAX_RESP3_ARGS).contains(&count) {
        return Err(invalid("resp3 command array has an unsupported length"));
    }
    let mut parts = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let Some((line, line_bytes)) = read_line(reader).await? else {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "eof inside resp3 command",
            ));
        };
        bytes += line_bytes;
        let Some((b'$', len)) = line.split_first() else {
            return Err(invalid("resp3 command elements must be bulk strings"));
        };
        let (body, body_bytes) = read_bulk_body(reader, parse_count(len)?).await?;
        bytes += body_bytes;
        parts.push(body);
    }
    let name = String::from_utf8_lossy(&parts.remove(0)).to_ascii_uppercase();
    Ok(Some((name, parts, bytes)))
}

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Read one reply — the driver side and the tests. Boxed because RESP3
/// aggregates make this recursive.
fn read_reply<'a, R: AsyncBufRead + Unpin + Send>(
    reader: &'a mut R,
) -> BoxFuture<'a, io::Result<Reply>> {
    Box::pin(async move {
        let Some((line, _bytes)) = read_line(reader).await? else {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "eof before resp3 reply",
            ));
        };
        let Some((tag, body)) = line.split_first() else {
            return Err(invalid("empty resp3 reply line"));
        };
        match tag {
            b'+' => Ok(Reply::Simple(String::from_utf8_lossy(body).into_owned())),
            b'-' => Ok(Reply::Error(String::from_utf8_lossy(body).into_owned())),
            b':' => Ok(Reply::Int(parse_count(body)?)),
            b'_' => Ok(Reply::Null),
            b'#' => match body {
                b"t" => Ok(Reply::Bool(true)),
                b"f" => Ok(Reply::Bool(false)),
                _ => Err(invalid("resp3 boolean must be #t or #f")),
            },
            b',' => match body {
                b"inf" => Ok(Reply::Double(f64::INFINITY)),
                b"-inf" => Ok(Reply::Double(f64::NEG_INFINITY)),
                b"nan" => Ok(Reply::Double(f64::NAN)),
                _ => std::str::from_utf8(body)
                    .ok()
                    .and_then(|text| text.parse::<f64>().ok())
                    .map(Reply::Double)
                    .ok_or_else(|| invalid("resp3 double is not a number")),
            },
            b'$' => {
                let len = parse_count(body)?;
                // RESP2 null bulk, accepted so a RESP2 client's own
                // encoding round-trips through this decoder.
                if len < 0 {
                    return Ok(Reply::Null);
                }
                let (bytes, _) = read_bulk_body(reader, len).await?;
                Ok(Reply::Bulk(bytes))
            }
            b'*' => {
                let count = parse_count(body)?;
                if count < 0 {
                    return Ok(Reply::Null);
                }
                if count > MAX_RESP3_ELEMENTS {
                    return Err(invalid("resp3 array too large"));
                }
                let mut items = Vec::with_capacity(count as usize);
                for _ in 0..count {
                    items.push(read_reply(reader).await?);
                }
                Ok(Reply::Array(items))
            }
            b'%' => {
                let count = parse_count(body)?;
                if !(0..=MAX_RESP3_ELEMENTS).contains(&count) {
                    return Err(invalid("resp3 map has an unsupported length"));
                }
                let mut pairs = Vec::with_capacity(count as usize);
                for _ in 0..count {
                    let key = read_reply(reader).await?;
                    let value = read_reply(reader).await?;
                    pairs.push((key, value));
                }
                Ok(Reply::Map(pairs))
            }
            _ => Err(invalid("unsupported resp3 reply type")),
        }
    })
}

// ── Driver (mirrors the HTTP lane exactly — BEN-003) ─────────────────────────

/// Ride through std-mutex poisoning (a panicked worker must not wedge the
/// harness; the guarded state stays consistent).
fn lock<T>(mutex: &StdMutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// One raw RESP3 connection — split so the pipeline window can send and
/// receive concurrently.
struct Resp3Conn {
    reader: BufReader<OwnedReadHalf>,
    writer: OwnedWriteHalf,
}

impl Resp3Conn {
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

/// Read + fully decode one reply — the RESP3 lane's measurement point
/// mirrors the Thunder client's full MessagePack decode and the HTTP
/// lane's JSON parse + `ok` check (BEN-003 parity).
async fn check_resp3_reply(reader: &mut BufReader<OwnedReadHalf>) -> Result<(), String> {
    match read_reply(reader)
        .await
        .map_err(|e| format!("resp3 read failed: {e}"))?
    {
        Reply::Error(message) => Err(format!("resp3 lane returned an error: {message}")),
        _ => Ok(()),
    }
}

/// Measure one matrix cell on the RESP3 lane.
pub async fn cell(
    handle: &Resp3Handle,
    spec: &CellSpec,
    cfg: &RunConfig,
) -> Result<Measured, String> {
    let addr = handle.local_addr();
    let request = Arc::new(build_resp3_request(spec.command, &spec.args)?);
    let mut conns = Vec::with_capacity(spec.connections);
    for _ in 0..spec.connections {
        conns.push(
            Resp3Conn::connect(addr)
                .await
                .map_err(|e| format!("resp3 connect failed: {e}"))?,
        );
    }

    if cfg.warmup > 0 {
        let (warmed, _lats, _elapsed) =
            resp3_window(conns, spec.depth, cfg.warmup, &request).await?;
        conns = warmed;
    }
    let before = handle.snapshot();
    let mut reps = Vec::with_capacity(cfg.repetitions);
    for _ in 0..cfg.repetitions {
        let (returned, mut lats, elapsed) =
            resp3_window(conns, spec.depth, cfg.ops, &request).await?;
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

/// One continuously-full RESP3 window across all connections.
///
/// Each connection issues at least `depth` requests so the pipeline
/// window actually fills — the same floor the Thunder lane's
/// worker-per-slot model implies (parity, BEN-003).
async fn resp3_window(
    conns: Vec<Resp3Conn>,
    depth: usize,
    total_ops: usize,
    request: &Arc<Vec<u8>>,
) -> Result<(Vec<Resp3Conn>, Vec<Duration>, Duration), String> {
    let per_conn = (total_ops / conns.len().max(1)).max(depth).max(1);
    let started = Instant::now();
    let mut handles = Vec::with_capacity(conns.len());
    for conn in conns {
        let request = Arc::clone(request);
        handles.push(tokio::spawn(resp3_conn_window(
            conn, depth, per_conn, request,
        )));
    }
    let mut returned = Vec::with_capacity(handles.len());
    let mut all = Vec::with_capacity(per_conn * handles.len());
    for handle in handles {
        let (conn, lats) = handle
            .await
            .map_err(|e| format!("resp3 worker panicked: {e}"))??;
        returned.push(conn);
        all.extend(lats);
    }
    Ok((returned, all, started.elapsed()))
}

/// FIFO pipeline window on one connection: the sender keeps up to `depth`
/// commands on the wire (a semaphore slot per in-flight command), the
/// receiver reads replies in order and frees slots — continuous
/// pipelining, no inter-batch gaps (BEN-003). RESP3 is a strictly ordered
/// request/response protocol, so this is the same shape as
/// `http_conn_window`.
async fn resp3_conn_window(
    mut conn: Resp3Conn,
    depth: usize,
    ops: usize,
    request: Arc<Vec<u8>>,
) -> Result<(Resp3Conn, Vec<Duration>), String> {
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
                    .map_err(|e| format!("resp3 write failed: {e}"))?;
            }
            Ok::<(), String>(())
        }
    };
    let receiver = {
        let pending = Arc::clone(&pending);
        async move {
            let mut lats = Vec::with_capacity(ops);
            for _ in 0..ops {
                check_resp3_reply(reader).await?;
                let (sent, permit) = lock(&pending)
                    .pop_front()
                    .ok_or_else(|| "resp3 reply without a pending command".to_owned())?;
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

/// Measure the connection-storm scenario on the RESP3 lane: sequential
/// connect + one command + fully decoded reply.
pub async fn storm(
    handle: &Resp3Handle,
    storms: usize,
    cfg: &RunConfig,
) -> Result<Measured, String> {
    let addr = handle.local_addr();
    let request = build_resp3_request("PING", &[])?;
    for _ in 0..cfg.warmup.min(storms) {
        resp3_storm_once(addr, &request).await?;
    }
    let before = handle.snapshot();
    let mut reps = Vec::with_capacity(cfg.repetitions);
    for _ in 0..cfg.repetitions {
        let mut lats = Vec::with_capacity(storms);
        let started = Instant::now();
        for _ in 0..storms {
            lats.push(resp3_storm_once(addr, &request).await?);
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

async fn resp3_storm_once(addr: SocketAddr, request: &[u8]) -> Result<Duration, String> {
    let started = Instant::now();
    let mut conn = Resp3Conn::connect(addr)
        .await
        .map_err(|e| format!("storm connect failed: {e}"))?;
    conn.writer
        .write_all(request)
        .await
        .map_err(|e| format!("storm write failed: {e}"))?;
    check_resp3_reply(&mut conn.reader).await?;
    Ok(started.elapsed())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    use crate::backend::STATIC_REPLY_BYTES;

    fn encoded(reply: &Reply) -> Vec<u8> {
        let mut out = Vec::new();
        reply.encode(&mut out);
        out
    }

    async fn decoded(bytes: &[u8]) -> Reply {
        let mut reader = BufReader::new(bytes);
        read_reply(&mut reader).await.unwrap()
    }

    // ── Encoding: byte-exact ───────────────────────────────────────────────

    #[test]
    fn reply_kinds_encode_byte_exactly() {
        assert_eq!(encoded(&Reply::Simple("PONG".to_owned())), b"+PONG\r\n");
        assert_eq!(
            encoded(&Reply::Error("ERR unknown command 'NOPE'".to_owned())),
            b"-ERR unknown command 'NOPE'\r\n"
        );
        assert_eq!(encoded(&Reply::Int(-7)), b":-7\r\n");
        assert_eq!(encoded(&Reply::Bool(true)), b"#t\r\n");
        assert_eq!(encoded(&Reply::Bool(false)), b"#f\r\n");
        assert_eq!(encoded(&Reply::Double(1.5)), b",1.5\r\n");
        assert_eq!(encoded(&Reply::Double(f64::INFINITY)), b",inf\r\n");
        assert_eq!(encoded(&Reply::Bulk(b"hi".to_vec())), b"$2\r\nhi\r\n");
        assert_eq!(encoded(&Reply::Bulk(Vec::new())), b"$0\r\n\r\n");
        assert_eq!(encoded(&Reply::Null), b"_\r\n");
        assert_eq!(
            encoded(&Reply::Array(vec![Reply::Int(1), Reply::Null])),
            b"*2\r\n:1\r\n_\r\n"
        );
        assert_eq!(
            encoded(&Reply::Map(vec![(
                Reply::Bulk(b"k".to_vec()),
                Reply::Bool(false)
            )])),
            b"%1\r\n$1\r\nk\r\n#f\r\n"
        );
    }

    #[tokio::test]
    async fn reply_kinds_round_trip_through_the_decoder() {
        let cases = vec![
            Reply::Simple("PONG".to_owned()),
            Reply::Error("ERR nope".to_owned()),
            Reply::Int(0),
            Reply::Int(i64::MIN),
            Reply::Bool(true),
            Reply::Bool(false),
            Reply::Double(-2.25),
            Reply::Bulk(b"payload".to_vec()),
            Reply::Bulk(Vec::new()),
            Reply::Bulk(vec![0xff, 0x00, 0x0d, 0x0a]),
            Reply::Null,
            Reply::Array(vec![Reply::Bulk(b"a".to_vec()), Reply::Null]),
            Reply::Array(Vec::new()),
            Reply::Map(vec![(Reply::Bulk(b"k".to_vec()), Reply::Int(1))]),
        ];
        for case in cases {
            assert_eq!(decoded(&encoded(&case)).await, case, "round trip: {case:?}");
        }
    }

    #[tokio::test]
    async fn resp2_null_forms_decode_to_null() {
        assert_eq!(decoded(b"$-1\r\n").await, Reply::Null);
        assert_eq!(decoded(b"*-1\r\n").await, Reply::Null);
    }

    #[tokio::test]
    async fn non_finite_doubles_round_trip() {
        assert_eq!(
            decoded(b",-inf\r\n").await,
            Reply::Double(f64::NEG_INFINITY)
        );
        match decoded(&encoded(&Reply::Double(f64::NAN))).await {
            Reply::Double(d) => assert!(d.is_nan()),
            other => panic!("expected a double, got {other:?}"),
        }
    }

    // ── Command decoding ───────────────────────────────────────────────────

    #[tokio::test]
    async fn commands_decode_upper_cased_with_their_args() {
        let mut reader = BufReader::new(&b"*2\r\n$4\r\necho\r\n$2\r\nhi\r\n"[..]);
        let (command, args, bytes) = read_command(&mut reader).await.unwrap().unwrap();
        assert_eq!(command, "ECHO");
        assert_eq!(args, vec![b"hi".to_vec()]);
        assert_eq!(bytes, b"*2\r\n$4\r\necho\r\n$2\r\nhi\r\n".len());
    }

    #[tokio::test]
    async fn clean_eof_between_commands_is_not_an_error() {
        let mut reader = BufReader::new(&b""[..]);
        assert!(read_command(&mut reader).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn inline_commands_and_malformed_arrays_are_rejected() {
        // Inline form is documented as out of scope.
        let mut reader = BufReader::new(&b"PING\r\n"[..]);
        assert!(read_command(&mut reader).await.is_err());
        // Non-bulk element.
        let mut reader = BufReader::new(&b"*1\r\n+PING\r\n"[..]);
        assert!(read_command(&mut reader).await.is_err());
        // Empty array.
        let mut reader = BufReader::new(&b"*0\r\n"[..]);
        assert!(read_command(&mut reader).await.is_err());
        // Bulk body without its CRLF terminator.
        let mut reader = BufReader::new(&b"*1\r\n$4\r\nPINGxx"[..]);
        assert!(read_command(&mut reader).await.is_err());
    }

    // ── Value mapping ──────────────────────────────────────────────────────

    #[test]
    fn bulk_args_map_to_str_when_utf8_and_bytes_otherwise() {
        assert_eq!(bulk_to_value(b"hi".to_vec()), Value::Str("hi".to_owned()));
        assert_eq!(
            bulk_to_value(vec![0xff, 0xfe]),
            Value::Bytes(vec![0xff, 0xfe])
        );
    }

    #[test]
    fn str_and_bytes_both_encode_as_bulk_strings() {
        assert_eq!(
            value_to_reply(&Value::Str("hi".to_owned())),
            Reply::Bulk(b"hi".to_vec())
        );
        assert_eq!(
            value_to_reply(&Value::Bytes(b"hi".to_vec())),
            Reply::Bulk(b"hi".to_vec())
        );
    }

    #[test]
    fn requests_encode_as_arrays_of_bulk_strings_and_reject_aggregates() {
        assert_eq!(
            build_resp3_request("ECHO", &[Value::Str("hi".to_owned())]).unwrap(),
            b"*2\r\n$4\r\nECHO\r\n$2\r\nhi\r\n"
        );
        assert_eq!(
            build_resp3_request("PING", &[]).unwrap(),
            b"*1\r\n$4\r\nPING\r\n"
        );
        assert!(build_resp3_request("ECHO", &[Value::Array(vec![])]).is_err());
    }

    // ── Live listener tests ────────────────────────────────────────────────

    async fn start() -> Resp3Handle {
        spawn_resp3_listener(
            Arc::new(NoopBackend::new()),
            SocketAddr::from(([127, 0, 0, 1], 0)),
        )
        .await
        .unwrap()
    }

    async fn call(conn: &mut Resp3Conn, command: &str, args: &[Value]) -> Reply {
        let request = build_resp3_request(command, args).unwrap();
        conn.writer.write_all(&request).await.unwrap();
        read_reply(&mut conn.reader).await.unwrap()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn ping_answers_the_exact_redis_pong_bytes() {
        let handle = start().await;
        let mut conn = Resp3Conn::connect(handle.local_addr()).await.unwrap();

        // Byte-exact: this is what a redis-benchmark ping_mbulk run reads.
        conn.writer
            .write_all(b"*1\r\n$4\r\nPING\r\n")
            .await
            .unwrap();
        let mut got = [0u8; 7];
        conn.reader.read_exact(&mut got).await.unwrap();
        assert_eq!(&got, b"+PONG\r\n");

        // Lower-cased command names work too (Redis is case-insensitive).
        assert_eq!(
            call(&mut conn, "ping", &[]).await,
            Reply::Simple("PONG".to_owned())
        );

        let snapshot = handle.snapshot();
        assert_eq!(snapshot.requests, 2);
        assert_eq!(snapshot.bytes_in, 2 * b"*1\r\n$4\r\nPING\r\n".len() as u64);
        assert_eq!(snapshot.bytes_out, 2 * b"+PONG\r\n".len() as u64);
        handle.stop().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn backend_commands_answer_identically_to_the_other_lanes() {
        let handle = start().await;
        let mut conn = Resp3Conn::connect(handle.local_addr()).await.unwrap();

        // ECHO returns its argument, as a bulk string.
        assert_eq!(
            call(&mut conn, "ECHO", &[Value::Str("hi".to_owned())]).await,
            Reply::Bulk(b"hi".to_vec())
        );
        // ECHO with a payload: exact bytes back.
        let payload = "x".repeat(64);
        assert_eq!(
            call(&mut conn, "ECHO", &[Value::Str(payload.clone())]).await,
            Reply::Bulk(payload.into_bytes())
        );
        // STATIC returns exactly 4096 payload bytes.
        match call(&mut conn, "STATIC", &[]).await {
            Reply::Bulk(bytes) => assert_eq!(bytes.len(), STATIC_REPLY_BYTES),
            other => panic!("expected a bulk string, got {other:?}"),
        }
        // SINK drops its args and returns the RESP3 null.
        assert_eq!(
            call(&mut conn, "SINK", &[Value::Bytes(vec![0u8; 128])]).await,
            Reply::Null
        );
        // Dispatch errors travel in-band and the connection stays usable.
        assert_eq!(
            call(&mut conn, "NOPE", &[]).await,
            Reply::Error("ERR unknown command 'NOPE'".to_owned())
        );
        assert_eq!(
            call(&mut conn, "PING", &[]).await,
            Reply::Simple("PONG".to_owned())
        );

        let snapshot = handle.snapshot();
        assert_eq!(snapshot.requests, 6);
        assert!(snapshot.bytes_in > 0);
        assert!(snapshot.bytes_out > 0);
        handle.stop().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn pipelined_commands_get_ordered_replies_on_one_connection() {
        let handle = start().await;
        let mut conn = Resp3Conn::connect(handle.local_addr()).await.unwrap();

        const N: usize = 64;
        let mut batch = Vec::new();
        for i in 0..N {
            batch.extend_from_slice(
                &build_resp3_request("ECHO", &[Value::Str(format!("req-{i}"))]).unwrap(),
            );
        }
        conn.writer.write_all(&batch).await.unwrap();

        for i in 0..N {
            assert_eq!(
                read_reply(&mut conn.reader).await.unwrap(),
                Reply::Bulk(format!("req-{i}").into_bytes()),
                "reply {i} out of order"
            );
        }
        assert_eq!(handle.snapshot().requests, N as u64);
        handle.stop().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_protocol_error_gets_an_error_reply_then_the_socket_closes() {
        let handle = start().await;
        let mut conn = Resp3Conn::connect(handle.local_addr()).await.unwrap();
        conn.writer.write_all(b"PING\r\n").await.unwrap();
        match read_reply(&mut conn.reader).await.unwrap() {
            Reply::Error(message) => {
                assert!(message.starts_with("ERR protocol error"), "{message}")
            }
            other => panic!("expected an error, got {other:?}"),
        }
        assert!(read_reply(&mut conn.reader).await.is_err());
        handle.stop().await;
    }

    // ── Driver ─────────────────────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread")]
    async fn the_driver_measures_a_pipelined_cell_and_reports_server_side_bytes() {
        let handle = start().await;
        let cfg = RunConfig {
            ops: 200,
            warmup: 20,
            repetitions: 2,
        };
        let spec = CellSpec {
            command: "ECHO",
            args: vec![Value::Str("x".repeat(64))],
            depth: 8,
            connections: 2,
        };
        let (reps, bytes_in_per_op, bytes_out_per_op) = cell(&handle, &spec, &cfg).await.unwrap();
        assert_eq!(reps.len(), 2);
        assert!(reps.iter().all(|r| r.qps > 0.0));
        // `*2\r\n$4\r\nECHO\r\n$64\r\n<64B>\r\n` = 4 + 10 + 71 = 85 B in;
        // `$64\r\n<64B>\r\n` = 71 B out.
        assert!((bytes_in_per_op - 85.0).abs() < 0.001, "{bytes_in_per_op}");
        assert!(
            (bytes_out_per_op - 71.0).abs() < 0.001,
            "{bytes_out_per_op}"
        );
        handle.stop().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_driver_measures_a_connection_storm() {
        let handle = start().await;
        let cfg = RunConfig {
            ops: 20,
            warmup: 2,
            repetitions: 2,
        };
        let (reps, bytes_in_per_op, bytes_out_per_op) = storm(&handle, 10, &cfg).await.unwrap();
        assert_eq!(reps.len(), 2);
        assert!(reps.iter().all(|r| r.qps > 0.0));
        assert!((bytes_in_per_op - 14.0).abs() < 0.001, "{bytes_in_per_op}");
        assert!((bytes_out_per_op - 7.0).abs() < 0.001, "{bytes_out_per_op}");
        handle.stop().await;
    }
}

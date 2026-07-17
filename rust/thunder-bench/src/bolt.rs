//! Minimal Bolt v5 listener over the same no-op backend (BEN-001, BEN-002)
//! — same process, host, runtime and allocator as the Thunder listener.
//!
//! Scope is a **benchmark peer, not a Bolt server product** (BEN-002). The
//! artifact records this scope alongside the numbers.
//!
//! # Protocol subset supported
//!
//! - **Handshake**: the 20-byte opener — magic preamble `60 60 B0 17` plus
//!   four 4-byte version proposals (`00 <range> <minor> <major>`); the
//!   server replies with the 4-byte agreed version. Only **v5.0** is
//!   agreed (a proposal whose major is 5 and whose `[minor-range, minor]`
//!   span covers minor 0); anything else — including a wrong magic — gets
//!   the `00 00 00 00` refusal and the connection closes.
//! - **Chunked framing**: `u16` BE length + payload, repeated, terminated
//!   by a zero-length chunk. Encode splits at [`MAX_CHUNK`]; decode
//!   reassembles any split. A message whose payload is empty is the Bolt
//!   4.1+ NOOP keep-alive and is ignored.
//! - **PackStream v2**, exactly the shapes the matrix needs: `Null`,
//!   `Boolean`, `Integer` (every width tier — tiny / `INT_8` / `INT_16` /
//!   `INT_32` / `INT_64`, always packed at the narrowest tier that fits),
//!   `Float` (float64), `String`, `Bytes`, `List`, `Dictionary` and
//!   `Structure` (`0xB<len>` + signature byte).
//! - **Messages**: `HELLO` (0x01) → `SUCCESS` (0x70); `RUN` (0x10) →
//!   `SUCCESS`; `PULL` (0x3F) → `RECORD` (0x71) then `SUCCESS`; `GOODBYE`
//!   (0x02) → close. `FAILURE` is 0x7F.
//!
//! # Deliberately omitted
//!
//! Structures nested inside values (nodes, relationships, paths, temporal
//! and spatial types) — the matrix carries no graph values, so
//! [`unpack_value`] refuses a structure marker outside the message
//! envelope rather than half-supporting it. Also omitted: `BEGIN`/
//! `COMMIT`/`ROLLBACK`/`DISCARD`/`RESET`/`ROUTE`/`LOGON`/`LOGOFF`,
//! `IGNORED` (0x7E) and the FAILED connection state, streaming/`has_more`
//! (`PULL {"n": -1}` always drains the single record), authentication,
//! multi-database routing, TLS, and any version other than 5.0. A `RUN`
//! the backend rejects answers `FAILURE` and leaves the connection usable
//! instead of entering the FAILED state a real server would demand a
//! `RESET` to clear — the matrix never issues one, so the simplification
//! cannot flatter the measured path.
//!
//! # Command mapping
//!
//! The backend command rides `RUN` as the query string and its arguments
//! ride the parameters dictionary under the key `args`:
//! `RUN "ECHO" {"args": ["x"]} {}`. The server calls
//! [`NoopBackend::respond`] with exactly those, replies
//! `SUCCESS {"fields": ["result"]}`, and `PULL` then returns the resulting
//! [`Value`] as the single field of one `RECORD`, followed by
//! `SUCCESS {"type": "r"}`. Every lane therefore answers from the same
//! dispatch surface and can never diverge on semantics.
//!
//! # Honesty note: RUN+PULL is two messages per logical operation
//!
//! Bolt has no single-message request/response for a query: one logical
//! operation is `RUN` + `PULL` out and `SUCCESS` + `RECORD` + `SUCCESS`
//! back — 2 messages up, 3 down, where the Thunder, RESP3 and HTTP lanes
//! each spend exactly 1 and 1. This lane counts **one RUN+PULL pair as one
//! op**, writes the pair in a single `write_all` (what a real Neo4j driver
//! does for an auto-commit query), and stamps latency from submitting the
//! pair to decoding the trailing `SUCCESS`. The consequences, stated
//! plainly:
//!
//! - **bytes/op penalises Bolt honestly**: the server-side counters bill
//!   all five messages to the one op, which is Bolt's real per-query cost
//!   on the wire, not an artifact of this peer.
//! - **latency/qps neither flatter nor penalise**: the pair leaves in one
//!   write and the three replies come back in one server-side flush, so
//!   the pair costs one round trip — the same round-trip count the other
//!   lanes pay. Splitting the pair into two round trips would have
//!   doubled Bolt's p50 for a reason no real driver suffers.
//! - **where this peer flatters Bolt vs. a real Neo4j server**: no Cypher
//!   parse/plan, no transaction, no store, no auth, no routing, no TLS —
//!   the same no-op posture BEN-001 imposes on every lane, which is the
//!   point. The peer is a *ceiling* for Bolt-the-transport, so a Thunder
//!   win here is a win against Bolt's best case.
//! - **where it penalises Bolt**: none identified — this peer omits work a
//!   real server does; it adds none.
//!
//! The contract otherwise mirrors [`crate::http`] exactly — same handle
//! shape, same server-side byte counters recorded after successful writes,
//! same continuously-full FIFO pipeline window (BEN-003).

use std::collections::VecDeque;
use std::io;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use thunder::wire::Value;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, watch, OwnedSemaphorePermit, Semaphore};

use crate::backend::NoopBackend;
use crate::driver::{CellSpec, Measured, RunConfig};
use crate::stats::compute;

/// The Bolt magic preamble.
const MAGIC: [u8; 4] = [0x60, 0x60, 0xB0, 0x17];

/// The one version this peer agrees: 5.0, wire-encoded `00 00 <minor> <major>`.
const VERSION_5_0: [u8; 4] = [0x00, 0x00, 0x00, 0x05];

/// The refusal reply: no proposed version is supported.
const VERSION_NONE: [u8; 4] = [0x00, 0x00, 0x00, 0x00];

/// Largest payload one chunk can carry (`u16` length header).
pub const MAX_CHUNK: usize = u16::MAX as usize;

/// Cap on one reassembled message — mirrors the Thunder frame cap (WIRE-020).
const MAX_MESSAGE_BYTES: usize = thunder::wire::DEFAULT_MAX_FRAME_BYTES;

// ── Message signatures ───────────────────────────────────────────────────────

/// `HELLO` — client → server, opens the session.
const MSG_HELLO: u8 = 0x01;
/// `GOODBYE` — client → server, closes the connection.
const MSG_GOODBYE: u8 = 0x02;
/// `RUN` — client → server, carries the backend command + args.
const MSG_RUN: u8 = 0x10;
/// `PULL` — client → server, drains the result.
const MSG_PULL: u8 = 0x3F;
/// `SUCCESS` — server → client.
const MSG_SUCCESS: u8 = 0x70;
/// `RECORD` — server → client, one result row.
const MSG_RECORD: u8 = 0x71;
/// `FAILURE` — server → client.
const MSG_FAILURE: u8 = 0x7F;

// ── PackStream v2 markers ────────────────────────────────────────────────────

const PS_NULL: u8 = 0xC0;
const PS_FLOAT_64: u8 = 0xC1;
const PS_FALSE: u8 = 0xC2;
const PS_TRUE: u8 = 0xC3;
const PS_INT_8: u8 = 0xC8;
const PS_INT_16: u8 = 0xC9;
const PS_INT_32: u8 = 0xCA;
const PS_INT_64: u8 = 0xCB;
const PS_BYTES_8: u8 = 0xCC;
const PS_BYTES_16: u8 = 0xCD;
const PS_BYTES_32: u8 = 0xCE;
const PS_STRING_8: u8 = 0xD0;
const PS_STRING_16: u8 = 0xD1;
const PS_STRING_32: u8 = 0xD2;
const PS_LIST_8: u8 = 0xD4;
const PS_LIST_16: u8 = 0xD5;
const PS_LIST_32: u8 = 0xD6;
const PS_DICT_8: u8 = 0xD8;
const PS_DICT_16: u8 = 0xD9;
const PS_DICT_32: u8 = 0xDA;

/// Server-side counters for the Bolt lane — the same measurement point as
/// the Thunder listener's SRV-030 metrics (bytes counted at the socket,
/// recorded after the successful write).
#[derive(Debug, Default)]
pub struct BoltMetrics {
    bytes_in: AtomicU64,
    bytes_out: AtomicU64,
    requests: AtomicU64,
}

impl BoltMetrics {
    fn record_in(&self, bytes: usize) {
        self.bytes_in.fetch_add(bytes as u64, Ordering::Relaxed);
    }

    /// Bytes written to the socket — called only after the write succeeds.
    fn record_out(&self, bytes: usize) {
        self.bytes_out.fetch_add(bytes as u64, Ordering::Relaxed);
    }

    /// One logical operation served (one `RUN`+`PULL` pair — see the
    /// module docs' honesty note).
    fn record_request(&self) {
        self.requests.fetch_add(1, Ordering::Relaxed);
    }

    fn snapshot(&self) -> BoltMetricsSnapshot {
        BoltMetricsSnapshot {
            requests: self.requests.load(Ordering::Relaxed),
            bytes_in: self.bytes_in.load(Ordering::Relaxed),
            bytes_out: self.bytes_out.load(Ordering::Relaxed),
        }
    }
}

/// Server-side counters, sampled around a measured window.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BoltMetricsSnapshot {
    /// Requests answered — one per `RUN`+`PULL` pair.
    pub requests: u64,
    /// Request bytes read off the wire.
    pub bytes_in: u64,
    /// Response bytes written to the wire (counted after the write).
    pub bytes_out: u64,
}

/// Handle to the running Bolt listener — same shape as
/// [`thunder::server::ListenerHandle`].
#[derive(Debug)]
pub struct BoltHandle {
    addr: SocketAddr,
    shutdown: watch::Sender<bool>,
    metrics: Arc<BoltMetrics>,
    done: Option<mpsc::Receiver<()>>,
}

impl BoltHandle {
    /// The bound address (resolves port `0` binds).
    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    /// Current server-side counters.
    pub fn snapshot(&self) -> BoltMetricsSnapshot {
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

impl Drop for BoltHandle {
    fn drop(&mut self) {
        let _ = self.shutdown.send(true);
    }
}

/// Spawn the Bolt v5 listener over the shared no-op backend.
pub async fn spawn_bolt_listener(
    backend: Arc<NoopBackend>,
    addr: SocketAddr,
) -> io::Result<BoltHandle> {
    let listener = TcpListener::bind(addr).await?;
    let local_addr = listener.local_addr()?;
    let metrics = Arc::new(BoltMetrics::default());
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let (done_tx, done_rx) = mpsc::channel::<()>(1);
    tokio::spawn(accept_loop(
        listener,
        backend,
        Arc::clone(&metrics),
        shutdown_rx,
        done_tx,
    ));
    Ok(BoltHandle {
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
    metrics: Arc<BoltMetrics>,
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

/// One connection: handshake, then a sequential message → reply loop, with
/// the flush deferred while another pipelined message is already buffered
/// (the drain-then-flush analog of the Thunder listener, SRV-006 — this is
/// what lets a pipelined `RUN`+`PULL` pair leave in one flush).
async fn handle_connection(
    stream: TcpStream,
    backend: Arc<NoopBackend>,
    metrics: Arc<BoltMetrics>,
    mut shutdown: watch::Receiver<bool>,
) {
    // Parity with the Thunder listener: Nagle off (SRV-008).
    let _ = stream.set_nodelay(true);
    let (read_half, write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);
    let mut writer = BufWriter::new(write_half);

    if server_handshake(&mut reader, &mut writer, &metrics)
        .await
        .is_err()
    {
        return;
    }

    // The result `RUN` produced, waiting for its `PULL`.
    let mut pending: Option<Value> = None;
    loop {
        let read = tokio::select! {
            _ = shutdown.wait_for(|stop| *stop) => break,
            read = read_chunked_message(&mut reader) => read,
        };
        let message = match read {
            Ok(Some(message)) => message,
            // Clean EOF between messages: the client is done.
            Ok(None) => break,
            Err(_) => break,
        };
        metrics.record_in(message.bytes);
        // Bolt 4.1+ NOOP keep-alive: an empty chunk sequence.
        if message.payload.is_empty() {
            continue;
        }
        let (signature, fields) = match unpack_message(&message.payload) {
            Ok(decoded) => decoded,
            Err(reason) => {
                let _ =
                    write_message(&mut writer, MSG_FAILURE, &[failure(&reason)], &metrics).await;
                break;
            }
        };
        if !serve(
            signature,
            fields,
            &backend,
            &mut pending,
            &mut writer,
            &metrics,
        )
        .await
        {
            break;
        }
        // Drain-then-flush analog: skip the flush while another pipelined
        // message is already sitting in the read buffer.
        if reader.buffer().is_empty() && writer.flush().await.is_err() {
            break;
        }
    }
    let _ = writer.flush().await;
}

/// Serve one decoded client message. Returns `false` when the connection
/// must close (`GOODBYE`, an unsupported message, or a write failure).
async fn serve(
    signature: u8,
    fields: Vec<Value>,
    backend: &NoopBackend,
    pending: &mut Option<Value>,
    writer: &mut BufWriter<OwnedWriteHalf>,
    metrics: &BoltMetrics,
) -> bool {
    match signature {
        MSG_HELLO => {
            let meta = Value::Map(vec![
                (
                    Value::Str("server".to_owned()),
                    Value::Str("thunder-bench/bolt-5.0".to_owned()),
                ),
                (
                    Value::Str("connection_id".to_owned()),
                    Value::Str("bench-1".to_owned()),
                ),
            ]);
            write_message(writer, MSG_SUCCESS, &[meta], metrics)
                .await
                .is_ok()
        }
        MSG_RUN => match run_request(&fields) {
            Ok((command, args)) => match backend.respond(&command, args) {
                Ok(value) => {
                    *pending = Some(value);
                    let meta = Value::Map(vec![(
                        Value::Str("fields".to_owned()),
                        Value::Array(vec![Value::Str("result".to_owned())]),
                    )]);
                    write_message(writer, MSG_SUCCESS, &[meta], metrics)
                        .await
                        .is_ok()
                }
                Err(message) => {
                    *pending = None;
                    write_message(writer, MSG_FAILURE, &[failure(&message)], metrics)
                        .await
                        .is_ok()
                }
            },
            Err(reason) => {
                *pending = None;
                write_message(writer, MSG_FAILURE, &[failure(&reason)], metrics)
                    .await
                    .is_ok()
            }
        },
        MSG_PULL => {
            let Some(value) = pending.take() else {
                return write_message(
                    writer,
                    MSG_FAILURE,
                    &[failure("PULL without a streaming result")],
                    metrics,
                )
                .await
                .is_ok();
            };
            if write_message(writer, MSG_RECORD, &[Value::Array(vec![value])], metrics)
                .await
                .is_err()
            {
                return false;
            }
            let meta = Value::Map(vec![(
                Value::Str("type".to_owned()),
                Value::Str("r".to_owned()),
            )]);
            if write_message(writer, MSG_SUCCESS, &[meta], metrics)
                .await
                .is_err()
            {
                return false;
            }
            // One RUN+PULL pair = one logical op (module docs' honesty note).
            metrics.record_request();
            true
        }
        MSG_GOODBYE => false,
        other => {
            let _ = write_message(
                writer,
                MSG_FAILURE,
                &[failure(&format!("unsupported message 0x{other:02X}"))],
                metrics,
            )
            .await;
            false
        }
    }
}

/// The standard Bolt failure metadata dictionary.
fn failure(message: &str) -> Value {
    Value::Map(vec![
        (
            Value::Str("code".to_owned()),
            Value::Str("Neo.ClientError.Statement.StatementError".to_owned()),
        ),
        (
            Value::Str("message".to_owned()),
            Value::Str(message.to_owned()),
        ),
    ])
}

/// Pull the backend command + args out of `RUN "<command>" {"args": [...]} {}`.
fn run_request(fields: &[Value]) -> Result<(String, Vec<Value>), String> {
    let query = fields
        .first()
        .and_then(Value::as_str)
        .ok_or_else(|| "RUN needs a string query".to_owned())?
        .to_owned();
    let args = match fields.get(1) {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Map(pairs)) => match pairs
            .iter()
            .find(|(k, _)| k.as_str() == Some("args"))
            .map(|(_, v)| v)
        {
            None => Vec::new(),
            Some(Value::Array(items)) => items.clone(),
            Some(_) => return Err("RUN parameter 'args' must be a list".to_owned()),
        },
        Some(_) => return Err("RUN parameters must be a dictionary".to_owned()),
    };
    Ok((query, args))
}

/// Write one message, chunked; bytes recorded after the successful write
/// (SRV-030 parity). The flush policy belongs to the caller.
async fn write_message(
    writer: &mut BufWriter<OwnedWriteHalf>,
    signature: u8,
    fields: &[Value],
    metrics: &BoltMetrics,
) -> io::Result<()> {
    let bytes = encode_message(signature, fields)
        .map_err(|reason| io::Error::new(io::ErrorKind::InvalidData, reason))?;
    writer.write_all(&bytes).await?;
    metrics.record_out(bytes.len());
    Ok(())
}

// ── Handshake ────────────────────────────────────────────────────────────────

/// Pick the reply for a 16-byte proposal block: v5.0 if any proposal's
/// major is 5 and its `[minor-range, minor]` span covers minor 0.
pub fn negotiate(proposals: &[u8; 16]) -> [u8; 4] {
    for proposal in proposals.chunks_exact(4) {
        let (range, minor, major) = (proposal[1], proposal[2], proposal[3]);
        if major == 5 && minor.saturating_sub(range) == 0 {
            return VERSION_5_0;
        }
    }
    VERSION_NONE
}

/// Server side of the handshake: read the 20-byte opener, reply with the
/// agreed version. Errors (and closes) on a wrong magic or no agreement.
async fn server_handshake(
    reader: &mut BufReader<OwnedReadHalf>,
    writer: &mut BufWriter<OwnedWriteHalf>,
    metrics: &BoltMetrics,
) -> io::Result<()> {
    let mut opener = [0u8; 20];
    reader.read_exact(&mut opener).await?;
    metrics.record_in(opener.len());
    if opener[..4] != MAGIC {
        // A wrong magic is refused without a reply: the peer is not Bolt.
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "bad bolt magic preamble",
        ));
    }
    let mut proposals = [0u8; 16];
    proposals.copy_from_slice(&opener[4..]);
    let agreed = negotiate(&proposals);
    writer.write_all(&agreed).await?;
    writer.flush().await?;
    metrics.record_out(agreed.len());
    if agreed == VERSION_NONE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "no supported bolt version proposed",
        ));
    }
    Ok(())
}

/// The 20-byte client opener: magic + v5.0 proposal, the rest zeroed.
pub fn client_opener() -> [u8; 20] {
    let mut opener = [0u8; 20];
    opener[..4].copy_from_slice(&MAGIC);
    opener[4..8].copy_from_slice(&VERSION_5_0);
    opener
}

// ── Chunk framing ────────────────────────────────────────────────────────────

/// Split `payload` into chunks and append them plus the zero-length
/// terminator to `out`.
pub fn chunk(payload: &[u8], out: &mut Vec<u8>) {
    for part in payload.chunks(MAX_CHUNK) {
        out.extend_from_slice(&(part.len() as u16).to_be_bytes());
        out.extend_from_slice(part);
    }
    out.extend_from_slice(&[0, 0]);
}

/// One reassembled message plus the bytes it consumed from the socket.
struct ChunkedMessage {
    payload: Vec<u8>,
    bytes: usize,
}

/// Read chunks until the zero-length terminator, reassembling the payload.
/// `Ok(None)` is a clean EOF before the first byte of a message.
async fn read_chunked_message<R: AsyncRead + Unpin>(
    reader: &mut R,
) -> io::Result<Option<ChunkedMessage>> {
    let mut payload = Vec::new();
    let mut bytes = 0usize;
    let mut first = true;
    loop {
        let mut header = [0u8; 2];
        if first {
            // Only here may EOF be clean: between messages.
            let n = reader.read(&mut header[..1]).await?;
            if n == 0 {
                return Ok(None);
            }
            reader.read_exact(&mut header[1..]).await?;
            first = false;
        } else {
            reader.read_exact(&mut header).await?;
        }
        bytes += 2;
        let length = u16::from_be_bytes(header) as usize;
        if length == 0 {
            break;
        }
        if payload.len() + length > MAX_MESSAGE_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "bolt message too large",
            ));
        }
        let at = payload.len();
        payload.resize(at + length, 0);
        reader.read_exact(&mut payload[at..]).await?;
        bytes += length;
    }
    Ok(Some(ChunkedMessage { payload, bytes }))
}

// ── PackStream v2 ────────────────────────────────────────────────────────────

/// Pack one value. Integers always take the narrowest tier that fits.
pub fn pack(value: &Value, out: &mut Vec<u8>) {
    match value {
        Value::Null => out.push(PS_NULL),
        Value::Bool(b) => out.push(if *b { PS_TRUE } else { PS_FALSE }),
        Value::Int(i) => pack_int(*i, out),
        Value::Float(f) => {
            out.push(PS_FLOAT_64);
            out.extend_from_slice(&f.to_be_bytes());
        }
        Value::Bytes(bytes) => {
            pack_header(
                bytes.len(),
                None,
                (PS_BYTES_8, PS_BYTES_16, PS_BYTES_32),
                out,
            );
            out.extend_from_slice(bytes);
        }
        Value::Str(s) => {
            pack_header(
                s.len(),
                Some(0x80),
                (PS_STRING_8, PS_STRING_16, PS_STRING_32),
                out,
            );
            out.extend_from_slice(s.as_bytes());
        }
        Value::Array(items) => {
            pack_header(
                items.len(),
                Some(0x90),
                (PS_LIST_8, PS_LIST_16, PS_LIST_32),
                out,
            );
            for item in items {
                pack(item, out);
            }
        }
        Value::Map(pairs) => {
            pack_header(
                pairs.len(),
                Some(0xA0),
                (PS_DICT_8, PS_DICT_16, PS_DICT_32),
                out,
            );
            for (key, value) in pairs {
                // PackStream dictionary keys are strings; non-string keys
                // are stringified, mirroring the HTTP lane's JSON mapping.
                let key = key
                    .as_str()
                    .map_or_else(|| format!("{key:?}"), str::to_owned);
                pack(&Value::Str(key), out);
                pack(value, out);
            }
        }
    }
}

/// Pack an integer at the narrowest width tier that represents it.
fn pack_int(value: i64, out: &mut Vec<u8>) {
    if (-16..=127).contains(&value) {
        out.push(value as i8 as u8);
    } else if let Ok(small) = i8::try_from(value) {
        out.push(PS_INT_8);
        out.push(small as u8);
    } else if let Ok(small) = i16::try_from(value) {
        out.push(PS_INT_16);
        out.extend_from_slice(&small.to_be_bytes());
    } else if let Ok(small) = i32::try_from(value) {
        out.push(PS_INT_32);
        out.extend_from_slice(&small.to_be_bytes());
    } else {
        out.push(PS_INT_64);
        out.extend_from_slice(&value.to_be_bytes());
    }
}

/// Write a length header: the tiny marker when `tiny_base` is set and the
/// length fits in 4 bits, else the narrowest sized marker.
fn pack_header(length: usize, tiny_base: Option<u8>, sized: (u8, u8, u8), out: &mut Vec<u8>) {
    let (m8, m16, m32) = sized;
    match tiny_base {
        Some(base) if length < 16 => out.push(base | length as u8),
        _ => {
            if let Ok(small) = u8::try_from(length) {
                out.push(m8);
                out.push(small);
            } else if let Ok(small) = u16::try_from(length) {
                out.push(m16);
                out.extend_from_slice(&small.to_be_bytes());
            } else {
                out.push(m32);
                out.extend_from_slice(&(length as u32).to_be_bytes());
            }
        }
    }
}

/// Encode one message: a structure with `signature` and `fields`, chunked.
pub fn encode_message(signature: u8, fields: &[Value]) -> Result<Vec<u8>, String> {
    if fields.len() > 15 {
        return Err(format!(
            "bolt structures carry at most 15 fields, got {}",
            fields.len()
        ));
    }
    let mut payload = Vec::with_capacity(64);
    payload.push(0xB0 | fields.len() as u8);
    payload.push(signature);
    for field in fields {
        pack(field, &mut payload);
    }
    let mut out = Vec::with_capacity(payload.len() + 4);
    chunk(&payload, &mut out);
    Ok(out)
}

/// Cursor over a packed payload.
struct Unpacker<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Unpacker<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self
            .pos
            .checked_add(n)
            .ok_or_else(|| "packstream length overflow".to_owned())?;
        let slice = self
            .bytes
            .get(self.pos..end)
            .ok_or_else(|| "packstream payload truncated".to_owned())?;
        self.pos = end;
        Ok(slice)
    }

    fn byte(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }

    fn length(&mut self, width: usize) -> Result<usize, String> {
        let bytes = self.take(width)?;
        Ok(bytes.iter().fold(0usize, |acc, b| (acc << 8) | *b as usize))
    }

    fn int(&mut self, width: usize) -> Result<i64, String> {
        let bytes = self.take(width)?;
        // Sign-extend from the tier's width.
        let mut value = i64::from(bytes[0] as i8);
        for b in &bytes[1..] {
            value = (value << 8) | i64::from(*b);
        }
        Ok(value)
    }
}

/// Unpack one value. Structure markers are refused — this peer carries no
/// graph values (see the module docs' omissions).
fn unpack_value(u: &mut Unpacker<'_>) -> Result<Value, String> {
    let marker = u.byte()?;
    match marker {
        PS_NULL => Ok(Value::Null),
        PS_TRUE => Ok(Value::Bool(true)),
        PS_FALSE => Ok(Value::Bool(false)),
        PS_FLOAT_64 => {
            let bytes = u.take(8)?;
            let mut buf = [0u8; 8];
            buf.copy_from_slice(bytes);
            Ok(Value::Float(f64::from_be_bytes(buf)))
        }
        0x00..=0x7F => Ok(Value::Int(i64::from(marker))),
        0xF0..=0xFF => Ok(Value::Int(i64::from(marker as i8))),
        PS_INT_8 => Ok(Value::Int(u.int(1)?)),
        PS_INT_16 => Ok(Value::Int(u.int(2)?)),
        PS_INT_32 => Ok(Value::Int(u.int(4)?)),
        PS_INT_64 => Ok(Value::Int(u.int(8)?)),
        PS_BYTES_8 | PS_BYTES_16 | PS_BYTES_32 => {
            let width = 1 << (marker - PS_BYTES_8);
            let length = u.length(width)?;
            Ok(Value::Bytes(u.take(length)?.to_vec()))
        }
        0x80..=0x8F => unpack_string(u, usize::from(marker & 0x0F)),
        PS_STRING_8 | PS_STRING_16 | PS_STRING_32 => {
            let width = 1 << (marker - PS_STRING_8);
            let length = u.length(width)?;
            unpack_string(u, length)
        }
        0x90..=0x9F => unpack_list(u, usize::from(marker & 0x0F)),
        PS_LIST_8 | PS_LIST_16 | PS_LIST_32 => {
            let width = 1 << (marker - PS_LIST_8);
            let length = u.length(width)?;
            unpack_list(u, length)
        }
        0xA0..=0xAF => unpack_dict(u, usize::from(marker & 0x0F)),
        PS_DICT_8 | PS_DICT_16 | PS_DICT_32 => {
            let width = 1 << (marker - PS_DICT_8);
            let length = u.length(width)?;
            unpack_dict(u, length)
        }
        0xB0..=0xBF => {
            Err("packstream structures appear only at the message envelope in this peer".to_owned())
        }
        other => Err(format!("unsupported packstream marker 0x{other:02X}")),
    }
}

fn unpack_string(u: &mut Unpacker<'_>, length: usize) -> Result<Value, String> {
    let bytes = u.take(length)?;
    String::from_utf8(bytes.to_vec())
        .map(Value::Str)
        .map_err(|e| format!("packstream string is not utf-8: {e}"))
}

fn unpack_list(u: &mut Unpacker<'_>, length: usize) -> Result<Value, String> {
    let mut items = Vec::with_capacity(length.min(64));
    for _ in 0..length {
        items.push(unpack_value(u)?);
    }
    Ok(Value::Array(items))
}

fn unpack_dict(u: &mut Unpacker<'_>, length: usize) -> Result<Value, String> {
    let mut pairs = Vec::with_capacity(length.min(64));
    for _ in 0..length {
        let key = unpack_value(u)?;
        if key.as_str().is_none() {
            return Err("packstream dictionary keys must be strings".to_owned());
        }
        pairs.push((key, unpack_value(u)?));
    }
    Ok(Value::Map(pairs))
}

/// Unpack one message envelope: `(signature, fields)`.
pub fn unpack_message(payload: &[u8]) -> Result<(u8, Vec<Value>), String> {
    let mut u = Unpacker::new(payload);
    let marker = u.byte()?;
    if !(0xB0..=0xBF).contains(&marker) {
        return Err(format!(
            "bolt messages are structures, got marker 0x{marker:02X}"
        ));
    }
    let arity = usize::from(marker & 0x0F);
    let signature = u.byte()?;
    let mut fields = Vec::with_capacity(arity);
    for _ in 0..arity {
        fields.push(unpack_value(&mut u)?);
    }
    Ok((signature, fields))
}

// ── Driver lane ──────────────────────────────────────────────────────────────

/// Ride through std-mutex poisoning (a panicked worker must not wedge the
/// harness; the guarded state stays consistent).
fn lock<T>(mutex: &StdMutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// One Bolt connection, handshaken and HELLO'd — ready for RUN/PULL.
struct BoltConn {
    reader: BufReader<OwnedReadHalf>,
    writer: OwnedWriteHalf,
}

impl BoltConn {
    /// Dial, negotiate v5.0 and complete `HELLO`. Everything here is
    /// session setup: the cell driver calls it *before* any measurement so
    /// the handshake never lands inside a measured window (the storm
    /// scenario is the exception — there setup is the thing measured).
    async fn connect(addr: SocketAddr) -> Result<Self, String> {
        let stream = TcpStream::connect(addr)
            .await
            .map_err(|e| format!("bolt connect failed: {e}"))?;
        stream
            .set_nodelay(true)
            .map_err(|e| format!("bolt nodelay failed: {e}"))?;
        let (read_half, write_half) = stream.into_split();
        let mut conn = Self {
            reader: BufReader::new(read_half),
            writer: write_half,
        };
        conn.handshake().await?;
        conn.hello().await?;
        Ok(conn)
    }

    async fn handshake(&mut self) -> Result<(), String> {
        self.writer
            .write_all(&client_opener())
            .await
            .map_err(|e| format!("bolt handshake write failed: {e}"))?;
        let mut agreed = [0u8; 4];
        self.reader
            .read_exact(&mut agreed)
            .await
            .map_err(|e| format!("bolt handshake read failed: {e}"))?;
        if agreed != VERSION_5_0 {
            return Err(format!("bolt peer refused v5.0, replied {agreed:?}"));
        }
        Ok(())
    }

    async fn hello(&mut self) -> Result<(), String> {
        let extra = Value::Map(vec![
            (
                Value::Str("user_agent".to_owned()),
                Value::Str("thunder-bench/1.0".to_owned()),
            ),
            (
                Value::Str("scheme".to_owned()),
                Value::Str("none".to_owned()),
            ),
        ]);
        let bytes = encode_message(MSG_HELLO, &[extra])?;
        self.writer
            .write_all(&bytes)
            .await
            .map_err(|e| format!("bolt HELLO write failed: {e}"))?;
        let (signature, _fields) = read_message(&mut self.reader).await?;
        if signature != MSG_SUCCESS {
            return Err(format!("bolt HELLO refused: signature 0x{signature:02X}"));
        }
        Ok(())
    }
}

/// Read + fully decode one message from a Bolt peer.
async fn read_message(reader: &mut BufReader<OwnedReadHalf>) -> Result<(u8, Vec<Value>), String> {
    loop {
        let message = read_chunked_message(reader)
            .await
            .map_err(|e| format!("bolt read failed: {e}"))?
            .ok_or_else(|| "bolt peer closed mid-stream".to_owned())?;
        // Skip NOOP keep-alives.
        if message.payload.is_empty() {
            continue;
        }
        return unpack_message(&message.payload);
    }
}

/// Serialize one logical op: a `RUN`+`PULL` pair, written as one buffer —
/// what a real Neo4j driver does for an auto-commit query (module docs).
fn build_bolt_request(command: &str, args: &[Value]) -> Result<Vec<u8>, String> {
    let run = encode_message(
        MSG_RUN,
        &[
            Value::Str(command.to_owned()),
            Value::Map(vec![(
                Value::Str("args".to_owned()),
                Value::Array(args.to_vec()),
            )]),
            Value::Map(vec![]),
        ],
    )?;
    let pull = encode_message(
        MSG_PULL,
        &[Value::Map(vec![(
            Value::Str("n".to_owned()),
            Value::Int(-1),
        )])],
    )?;
    let mut out = run;
    out.extend_from_slice(&pull);
    Ok(out)
}

/// Read + fully decode one op's replies — `SUCCESS` (run), `RECORD`,
/// `SUCCESS` (pull). This is the Bolt lane's measurement point, mirroring
/// the Thunder client's full MessagePack decode (BEN-003 parity).
async fn check_bolt_response(reader: &mut BufReader<OwnedReadHalf>) -> Result<(), String> {
    for expected in [MSG_SUCCESS, MSG_RECORD, MSG_SUCCESS] {
        let (signature, fields) = read_message(reader).await?;
        if signature != expected {
            return Err(format!(
                "bolt lane expected 0x{expected:02X}, got 0x{signature:02X}: {fields:?}"
            ));
        }
    }
    Ok(())
}

/// One matrix cell on the Bolt v5 lane.
pub async fn cell(
    handle: &BoltHandle,
    spec: &CellSpec,
    cfg: &RunConfig,
) -> Result<Measured, String> {
    let addr = handle.local_addr();
    let request = Arc::new(build_bolt_request(spec.command, &spec.args)?);
    let mut conns = Vec::with_capacity(spec.connections);
    for _ in 0..spec.connections {
        conns.push(BoltConn::connect(addr).await?);
    }

    if cfg.warmup > 0 {
        let (warmed, _lats, _elapsed) =
            bolt_window(conns, spec.depth, cfg.warmup, &request).await?;
        conns = warmed;
    }
    let before = handle.snapshot();
    let mut reps = Vec::with_capacity(cfg.repetitions);
    for _ in 0..cfg.repetitions {
        let (returned, mut lats, elapsed) =
            bolt_window(conns, spec.depth, cfg.ops, &request).await?;
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

/// One continuously-full Bolt window across all connections.
///
/// Each connection issues at least `depth` ops so the pipeline window
/// actually fills — the same floor the Thunder lane's worker-per-slot
/// model implies (parity, BEN-003).
async fn bolt_window(
    conns: Vec<BoltConn>,
    depth: usize,
    total_ops: usize,
    request: &Arc<Vec<u8>>,
) -> Result<(Vec<BoltConn>, Vec<Duration>, Duration), String> {
    let per_conn = (total_ops / conns.len().max(1)).max(depth).max(1);
    let started = Instant::now();
    let mut handles = Vec::with_capacity(conns.len());
    for conn in conns {
        let request = Arc::clone(request);
        handles.push(tokio::spawn(bolt_conn_window(
            conn, depth, per_conn, request,
        )));
    }
    let mut returned = Vec::with_capacity(handles.len());
    let mut all = Vec::with_capacity(per_conn * handles.len());
    for handle in handles {
        let (conn, lats) = handle
            .await
            .map_err(|e| format!("bolt worker panicked: {e}"))??;
        returned.push(conn);
        all.extend(lats);
    }
    Ok((returned, all, started.elapsed()))
}

/// FIFO pipeline window on one connection: the sender keeps up to `depth`
/// ops on the wire (a semaphore slot per in-flight op), the receiver reads
/// replies in order and frees slots — continuous pipelining, no
/// inter-batch gaps (BEN-003). Bolt is an ordered request/response
/// protocol, so the HTTP lane's window shape transfers exactly.
async fn bolt_conn_window(
    mut conn: BoltConn,
    depth: usize,
    ops: usize,
    request: Arc<Vec<u8>>,
) -> Result<(BoltConn, Vec<Duration>), String> {
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
                    .map_err(|e| format!("bolt write failed: {e}"))?;
            }
            Ok::<(), String>(())
        }
    };
    let receiver = {
        let pending = Arc::clone(&pending);
        async move {
            let mut lats = Vec::with_capacity(ops);
            for _ in 0..ops {
                check_bolt_response(reader).await?;
                let (sent, permit) = lock(&pending)
                    .pop_front()
                    .ok_or_else(|| "bolt response without a pending request".to_owned())?;
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

/// Connection storm on the Bolt v5 lane: sequential connect + handshake +
/// `HELLO` + one `RUN`/`PULL` pair + fully decoded replies. Setup *is* the
/// thing measured here, so — unlike [`cell`] — the handshake and `HELLO`
/// land inside the stamp, mirroring how the Thunder and HTTP lanes bill
/// their own connection setup.
pub async fn storm(
    handle: &BoltHandle,
    storms: usize,
    cfg: &RunConfig,
) -> Result<Measured, String> {
    let addr = handle.local_addr();
    let request = build_bolt_request("PING", &[])?;
    for _ in 0..cfg.warmup.min(storms) {
        bolt_storm_once(addr, &request).await?;
    }
    let before = handle.snapshot();
    let mut reps = Vec::with_capacity(cfg.repetitions);
    for _ in 0..cfg.repetitions {
        let mut lats = Vec::with_capacity(storms);
        let started = Instant::now();
        for _ in 0..storms {
            lats.push(bolt_storm_once(addr, &request).await?);
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

async fn bolt_storm_once(addr: SocketAddr, request: &[u8]) -> Result<Duration, String> {
    let started = Instant::now();
    let mut conn = BoltConn::connect(addr).await?;
    conn.writer
        .write_all(request)
        .await
        .map_err(|e| format!("storm write failed: {e}"))?;
    check_bolt_response(&mut conn.reader).await?;
    Ok(started.elapsed())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn packed(value: &Value) -> Vec<u8> {
        let mut out = Vec::new();
        pack(value, &mut out);
        out
    }

    fn unpacked(bytes: &[u8]) -> Value {
        let mut u = Unpacker::new(bytes);
        let value = unpack_value(&mut u).unwrap();
        assert_eq!(u.pos, bytes.len(), "unpack must consume the whole payload");
        value
    }

    fn round_trips(value: Value, expected: &[u8]) {
        let bytes = packed(&value);
        assert_eq!(bytes, expected, "byte-exact packing of {value:?}");
        assert_eq!(unpacked(&bytes), value);
    }

    // ── PackStream ─────────────────────────────────────────────────────────

    #[test]
    fn packstream_scalars_are_byte_exact() {
        round_trips(Value::Null, &[0xC0]);
        round_trips(Value::Bool(true), &[0xC3]);
        round_trips(Value::Bool(false), &[0xC2]);
        round_trips(
            Value::Float(1.1),
            &[0xC1, 0x3F, 0xF1, 0x99, 0x99, 0x99, 0x99, 0x99, 0x9A],
        );
        round_trips(Value::Str(String::new()), &[0x80]);
        round_trips(Value::Str("A".to_owned()), &[0x81, 0x41]);
    }

    #[test]
    fn packstream_ints_pack_at_the_narrowest_tier() {
        // Tiny: -16..=127, one byte.
        round_trips(Value::Int(0), &[0x00]);
        round_trips(Value::Int(42), &[0x2A]);
        round_trips(Value::Int(127), &[0x7F]);
        round_trips(Value::Int(-1), &[0xFF]);
        round_trips(Value::Int(-16), &[0xF0]);
        // INT_8: -128..=-17.
        round_trips(Value::Int(-17), &[PS_INT_8, 0xEF]);
        round_trips(Value::Int(-128), &[PS_INT_8, 0x80]);
        // INT_16.
        round_trips(Value::Int(128), &[PS_INT_16, 0x00, 0x80]);
        round_trips(Value::Int(-129), &[PS_INT_16, 0xFF, 0x7F]);
        round_trips(Value::Int(32767), &[PS_INT_16, 0x7F, 0xFF]);
        round_trips(Value::Int(-32768), &[PS_INT_16, 0x80, 0x00]);
        // INT_32.
        round_trips(Value::Int(32768), &[PS_INT_32, 0x00, 0x00, 0x80, 0x00]);
        round_trips(Value::Int(-32769), &[PS_INT_32, 0xFF, 0xFF, 0x7F, 0xFF]);
        round_trips(
            Value::Int(2_147_483_647),
            &[PS_INT_32, 0x7F, 0xFF, 0xFF, 0xFF],
        );
        // INT_64.
        round_trips(
            Value::Int(2_147_483_648),
            &[PS_INT_64, 0, 0, 0, 0, 0x80, 0, 0, 0],
        );
        round_trips(
            Value::Int(i64::MIN),
            &[PS_INT_64, 0x80, 0, 0, 0, 0, 0, 0, 0],
        );
        round_trips(
            Value::Int(i64::MAX),
            &[PS_INT_64, 0x7F, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF],
        );
    }

    #[test]
    fn packstream_strings_cross_every_width_tier() {
        round_trips(Value::Str("x".repeat(15)), &{
            let mut v = vec![0x8F];
            v.extend(std::iter::repeat_n(b'x', 15));
            v
        });
        // 16 bytes leaves the tiny tier for STRING_8.
        let s16 = packed(&Value::Str("x".repeat(16)));
        assert_eq!(&s16[..2], &[PS_STRING_8, 16]);
        assert_eq!(unpacked(&s16), Value::Str("x".repeat(16)));
        // 256 bytes leaves STRING_8 for STRING_16.
        let s256 = packed(&Value::Str("x".repeat(256)));
        assert_eq!(&s256[..3], &[PS_STRING_16, 0x01, 0x00]);
        assert_eq!(unpacked(&s256), Value::Str("x".repeat(256)));
        // 64 KiB leaves STRING_16 for STRING_32.
        let big = "x".repeat(65_536);
        let s64k = packed(&Value::Str(big.clone()));
        assert_eq!(&s64k[..5], &[PS_STRING_32, 0x00, 0x01, 0x00, 0x00]);
        assert_eq!(unpacked(&s64k), Value::Str(big));
    }

    #[test]
    fn packstream_bytes_have_no_tiny_tier() {
        round_trips(Value::Bytes(vec![1, 2, 3]), &[PS_BYTES_8, 3, 1, 2, 3]);
        let big = packed(&Value::Bytes(vec![7u8; 300]));
        assert_eq!(&big[..3], &[PS_BYTES_16, 0x01, 0x2C]);
        assert_eq!(unpacked(&big), Value::Bytes(vec![7u8; 300]));
    }

    #[test]
    fn packstream_lists_and_dicts_round_trip() {
        round_trips(Value::Array(vec![]), &[0x90]);
        round_trips(
            Value::Array(vec![Value::Int(1), Value::Str("two".to_owned())]),
            &[0x92, 0x01, 0x83, b't', b'w', b'o'],
        );
        round_trips(Value::Map(vec![]), &[0xA0]);
        round_trips(
            Value::Map(vec![(Value::Str("a".to_owned()), Value::Int(1))]),
            &[0xA1, 0x81, b'a', 0x01],
        );
        // Nested, and the sized list tier.
        let nested = Value::Array(vec![
            Value::Null,
            Value::Map(vec![(
                Value::Str("k".to_owned()),
                Value::Array(vec![Value::Bool(true), Value::Float(-2.5)]),
            )]),
        ]);
        assert_eq!(unpacked(&packed(&nested)), nested);
        let long = Value::Array((0..20).map(Value::Int).collect());
        assert_eq!(packed(&long)[0], PS_LIST_8);
        assert_eq!(unpacked(&packed(&long)), long);
    }

    #[test]
    fn packstream_refuses_structures_inside_values_and_truncation() {
        let mut u = Unpacker::new(&[0xB1, 0x70, 0xC0]);
        assert!(unpack_value(&mut u).unwrap_err().contains("structures"));
        let mut u = Unpacker::new(&[0x83, b'a']);
        assert!(unpack_value(&mut u).unwrap_err().contains("truncated"));
        let mut u = Unpacker::new(&[0xA1, 0x01, 0xC0]);
        assert!(unpack_value(&mut u)
            .unwrap_err()
            .contains("keys must be strings"));
    }

    // ── Chunk framing ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn chunking_round_trips_and_honors_the_terminator() {
        let mut framed = Vec::new();
        chunk(&[1, 2, 3], &mut framed);
        assert_eq!(framed, vec![0x00, 0x03, 1, 2, 3, 0x00, 0x00]);

        let mut cursor = std::io::Cursor::new(framed);
        let message = read_chunked_message(&mut cursor).await.unwrap().unwrap();
        assert_eq!(message.payload, vec![1, 2, 3]);
        assert_eq!(message.bytes, 7);
    }

    #[tokio::test]
    async fn a_message_split_across_chunks_reassembles() {
        // Hand-built: two chunks then the terminator, plus a trailing
        // second message that must not be consumed.
        let wire: Vec<u8> = vec![
            0x00, 0x02, 0xAA, 0xBB, // chunk 1
            0x00, 0x03, 0xCC, 0xDD, 0xEE, // chunk 2
            0x00, 0x00, // terminator
            0x00, 0x01, 0x11, 0x00, 0x00, // a second message
        ];
        let mut cursor = std::io::Cursor::new(wire);
        let first = read_chunked_message(&mut cursor).await.unwrap().unwrap();
        assert_eq!(first.payload, vec![0xAA, 0xBB, 0xCC, 0xDD, 0xEE]);
        assert_eq!(first.bytes, 11);
        let second = read_chunked_message(&mut cursor).await.unwrap().unwrap();
        assert_eq!(second.payload, vec![0x11]);
        // Clean EOF between messages.
        assert!(read_chunked_message(&mut cursor).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn a_bare_terminator_is_the_noop_keepalive() {
        let mut cursor = std::io::Cursor::new(vec![0x00, 0x00]);
        let message = read_chunked_message(&mut cursor).await.unwrap().unwrap();
        assert!(message.payload.is_empty());
        assert_eq!(message.bytes, 2);
    }

    #[test]
    fn encode_message_frames_a_structure() {
        let bytes = encode_message(MSG_PULL, &[Value::Map(vec![])]).unwrap();
        // 3-byte payload: B1 3F A0, chunked.
        assert_eq!(bytes, vec![0x00, 0x03, 0xB1, 0x3F, 0xA0, 0x00, 0x00]);
        let (signature, fields) = unpack_message(&bytes[2..5]).unwrap();
        assert_eq!(signature, MSG_PULL);
        assert_eq!(fields, vec![Value::Map(vec![])]);
    }

    #[test]
    fn unpack_message_refuses_a_non_structure() {
        assert!(unpack_message(&[0xC0]).unwrap_err().contains("structures"));
    }

    // ── Handshake ──────────────────────────────────────────────────────────

    #[test]
    fn negotiation_agrees_v5_and_refuses_the_rest() {
        let mut proposals = [0u8; 16];
        proposals[..4].copy_from_slice(&VERSION_5_0);
        assert_eq!(negotiate(&proposals), VERSION_5_0);

        // v5.0 in a later slot still wins.
        let mut later = [0u8; 16];
        later[4..8].copy_from_slice(&[0, 0, 0, 4]); // 4.0
        later[8..12].copy_from_slice(&VERSION_5_0);
        assert_eq!(negotiate(&later), VERSION_5_0);

        // A range 5.3..5.0 covers 5.0.
        let mut ranged = [0u8; 16];
        ranged[..4].copy_from_slice(&[0, 3, 3, 5]);
        assert_eq!(negotiate(&ranged), VERSION_5_0);

        // 5.4 with no range does not reach 5.0.
        let mut too_new = [0u8; 16];
        too_new[..4].copy_from_slice(&[0, 0, 4, 5]);
        assert_eq!(negotiate(&too_new), VERSION_NONE);

        // Bolt 4.4 only.
        let mut old = [0u8; 16];
        old[..4].copy_from_slice(&[0, 0, 4, 4]);
        assert_eq!(negotiate(&old), VERSION_NONE);

        assert_eq!(negotiate(&[0u8; 16]), VERSION_NONE);
    }

    #[test]
    fn the_client_opener_carries_the_magic_and_v5() {
        let opener = client_opener();
        assert_eq!(&opener[..4], &MAGIC);
        assert_eq!(&opener[4..8], &VERSION_5_0);
        assert_eq!(&opener[8..], &[0u8; 12]);
    }

    // ── Live listener tests ────────────────────────────────────────────────

    async fn start() -> BoltHandle {
        spawn_bolt_listener(
            Arc::new(NoopBackend::new()),
            SocketAddr::from(([127, 0, 0, 1], 0)),
        )
        .await
        .unwrap()
    }

    /// One logical op over a live connection, returning the record's value.
    async fn call(conn: &mut BoltConn, command: &str, args: &[Value]) -> Value {
        let request = build_bolt_request(command, args).unwrap();
        conn.writer.write_all(&request).await.unwrap();
        let (run_sig, _run_meta) = read_message(&mut conn.reader).await.unwrap();
        assert_eq!(run_sig, MSG_SUCCESS);
        let (record_sig, record) = read_message(&mut conn.reader).await.unwrap();
        assert_eq!(record_sig, MSG_RECORD);
        let (pull_sig, _pull_meta) = read_message(&mut conn.reader).await.unwrap();
        assert_eq!(pull_sig, MSG_SUCCESS);
        match record.into_iter().next() {
            Some(Value::Array(mut fields)) if fields.len() == 1 => fields.swap_remove(0),
            other => panic!("expected a 1-field RECORD, got {other:?}"),
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn handshake_hello_run_pull_round_trips_end_to_end() {
        let handle = start().await;
        // `connect` performs the handshake and HELLO — both must succeed.
        let mut conn = BoltConn::connect(handle.local_addr()).await.unwrap();

        assert_eq!(
            call(&mut conn, "ECHO", &[Value::Str("hi".to_owned())]).await,
            Value::Str("hi".to_owned())
        );

        let snapshot = handle.snapshot();
        assert_eq!(snapshot.requests, 1);
        assert!(snapshot.bytes_in > 0);
        assert!(snapshot.bytes_out > 0, "bytes_out must move after a reply");
        handle.stop().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_backend_commands_answer_over_bolt() {
        let handle = start().await;
        let mut conn = BoltConn::connect(handle.local_addr()).await.unwrap();

        // ECHO returns its argument; bare PING is PONG.
        assert_eq!(
            call(&mut conn, "ECHO", &[Value::Int(7)]).await,
            Value::Int(7)
        );
        assert_eq!(
            call(&mut conn, "PING", &[]).await,
            Value::Str("PONG".to_owned())
        );
        // STATIC is exactly 4096 payload bytes.
        match call(&mut conn, "STATIC", &[]).await {
            Value::Str(s) => assert_eq!(s.len(), crate::backend::STATIC_REPLY_BYTES),
            other => panic!("expected Str, got {other:?}"),
        }
        // SINK drops its args and returns null.
        assert_eq!(
            call(&mut conn, "SINK", &[Value::Bytes(vec![0u8; 64])]).await,
            Value::Null
        );

        assert_eq!(handle.snapshot().requests, 4);
        handle.stop().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_pipelined_pair_of_ops_stays_in_order() {
        let handle = start().await;
        let mut conn = BoltConn::connect(handle.local_addr()).await.unwrap();
        let mut request = build_bolt_request("ECHO", &[Value::Str("one".to_owned())]).unwrap();
        request.extend(build_bolt_request("ECHO", &[Value::Str("two".to_owned())]).unwrap());
        conn.writer.write_all(&request).await.unwrap();
        // Both ops' replies arrive in submission order.
        check_bolt_response(&mut conn.reader).await.unwrap();
        check_bolt_response(&mut conn.reader).await.unwrap();
        assert_eq!(handle.snapshot().requests, 2);
        handle.stop().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_unknown_command_is_a_bolt_failure() {
        let handle = start().await;
        let mut conn = BoltConn::connect(handle.local_addr()).await.unwrap();
        let request = build_bolt_request("NOPE", &[]).unwrap();
        conn.writer.write_all(&request).await.unwrap();
        let (signature, fields) = read_message(&mut conn.reader).await.unwrap();
        assert_eq!(signature, MSG_FAILURE);
        match &fields[0] {
            Value::Map(pairs) => {
                let message = pairs
                    .iter()
                    .find(|(k, _)| k.as_str() == Some("message"))
                    .map(|(_, v)| v)
                    .unwrap();
                assert_eq!(
                    message,
                    &Value::Str("ERR unknown command 'NOPE'".to_owned())
                );
            }
            other => panic!("expected failure metadata, got {other:?}"),
        }
        handle.stop().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_wrong_magic_is_refused() {
        let handle = start().await;
        let mut stream = TcpStream::connect(handle.local_addr()).await.unwrap();
        let mut opener = client_opener();
        opener[0] = 0x00; // corrupt the magic
        stream.write_all(&opener).await.unwrap();
        // No version reply: the server closes on a non-Bolt peer.
        let mut reply = Vec::new();
        stream.read_to_end(&mut reply).await.unwrap();
        assert!(reply.is_empty(), "expected no reply, got {reply:?}");
        handle.stop().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_unsupported_version_gets_the_zero_reply() {
        let handle = start().await;
        let mut stream = TcpStream::connect(handle.local_addr()).await.unwrap();
        let mut opener = [0u8; 20];
        opener[..4].copy_from_slice(&MAGIC);
        opener[4..8].copy_from_slice(&[0, 0, 4, 4]); // Bolt 4.4 only
        stream.write_all(&opener).await.unwrap();
        let mut agreed = [0u8; 4];
        stream.read_exact(&mut agreed).await.unwrap();
        assert_eq!(agreed, VERSION_NONE);
        handle.stop().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn goodbye_closes_the_connection() {
        let handle = start().await;
        let mut conn = BoltConn::connect(handle.local_addr()).await.unwrap();
        let bye = encode_message(MSG_GOODBYE, &[]).unwrap();
        conn.writer.write_all(&bye).await.unwrap();
        let mut rest = Vec::new();
        conn.reader.read_to_end(&mut rest).await.unwrap();
        assert!(rest.is_empty());
        handle.stop().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_cell_driver_measures_a_window() {
        let handle = start().await;
        let spec = CellSpec {
            command: "ECHO",
            args: vec![Value::Str("x".repeat(64))],
            depth: 4,
            connections: 2,
        };
        let cfg = RunConfig {
            ops: 40,
            warmup: 4,
            repetitions: 2,
        };
        let (reps, bytes_in, bytes_out) = cell(&handle, &spec, &cfg).await.unwrap();
        assert_eq!(reps.len(), 2);
        assert!(reps.iter().all(|r| r.qps > 0.0));
        // Both RUN and PULL are billed to the one op.
        assert!(bytes_in > 0.0, "{bytes_in}");
        assert!(bytes_out > 0.0, "{bytes_out}");
        handle.stop().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_storm_driver_measures_connection_setup() {
        let handle = start().await;
        let cfg = RunConfig {
            ops: 8,
            warmup: 2,
            repetitions: 2,
        };
        let (reps, bytes_in, bytes_out) = storm(&handle, 4, &cfg).await.unwrap();
        assert_eq!(reps.len(), 2);
        // The storm bills handshake + HELLO to its one op, by design.
        assert!(bytes_in > 0.0);
        assert!(bytes_out > 0.0);
        handle.stop().await;
    }
}

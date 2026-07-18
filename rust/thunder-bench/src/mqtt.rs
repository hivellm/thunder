//! **MQTT 5 request/response** lane — a minimal broker and client of ours,
//! serving the same no-op backend (BEN-001, BEN-002) in the same process,
//! host, runtime and allocator as the Thunder listener.
//!
//! # ⚠ Same shape caveat as the NATS lane: four traversals, not two
//!
//! Like [`crate::nats`], this is a **broker** architecture, so a round trip
//! crosses four sockets rather than two:
//!
//! ```text
//!   requester ──▶ broker ──▶ responder
//!   requester ◀── broker ◀── responder
//! ```
//!
//! Comparing its latency against a point-to-point transport is a category
//! error. What it is legitimately compared against is **the NATS lane**: same
//! topology, same traversal count, different wire. That is the comparison
//! worth reading here — a compact binary pub/sub wire against a line-oriented
//! text one, with the architecture held constant.
//!
//! MQTT 3.1.1 has no request/response affordance at all. **MQTT 5 does**: the
//! `Response Topic` (0x08) and `Correlation Data` (0x09) PUBLISH properties,
//! which is what this lane uses — the responder replies to the topic the
//! requester named, echoing the correlation bytes so the requester can match
//! the reply to its request.
//!
//! # Both sides are ours, and why
//!
//! This is the one lane in the expansion with no real implementation on
//! either side, and the reasons are specific rather than convenient:
//!
//! - **`rumqttd`** (the mature Rust broker) cannot be used: `Broker::start()`
//!   blocks and internally spawns one OS thread per component, each building
//!   its *own* current-thread runtime. That is several extra runtimes on
//!   threads outside the harness's control, competing for cores and skewing
//!   percentiles — a worse BEN-001 violation than the Cap'n Proto lane's
//!   single extra runtime, and with no offsetting benefit.
//! - **The codec crates** are each disqualified: `mqttbytes` has been dead
//!   since 2021, `mqtt-bytes-v5` has essentially no users, `mqtt-protocol`'s
//!   v5 support is incomplete, and `mqtt5-protocol`, while actively developed
//!   and the best of them, is young enough that adopting it would trade one
//!   unproven implementation for another.
//!
//! So the encoder here is a deliberate, documented exception to the
//! real-crates policy. It is also the lane where that matters least: MQTT's
//! packet format is a fixed header plus a variable-byte-integer length, and
//! the QoS 0 subset is small enough to verify exhaustively against the spec —
//! which the tests do, byte by byte.
//!
//! # Scope (honesty note, BEN-002)
//!
//! A **benchmark broker, not an MQTT broker**. QoS 0 only, which is the right
//! choice for a latency measurement: QoS 1 doubles the packet count with
//! PUBACKs and puts the broker's inflight-window bookkeeping in the
//! measurement, and its window silently caps concurrency. Packets modelled:
//! `CONNECT`/`CONNACK`, `SUBSCRIBE`/`SUBACK`, `PUBLISH` (both directions),
//! `PINGREQ`/`PINGRESP`, `DISCONNECT`. Properties: only Response Topic and
//! Correlation Data. No retained messages, no wills, no sessions, no auth,
//! no TLS, no shared subscriptions, no topic aliases.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use thunder::wire::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::tcp::OwnedReadHalf;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot, watch};

use crate::backend::NoopBackend;
use crate::driver::{CellSpec, Measured, RunConfig};
use crate::stats::compute;

/// The topic the responder serves.
const REQUEST_TOPIC: &str = "bench/call";
/// Packet cap — mirrors the Thunder frame cap (WIRE-020).
const MAX_PACKET: usize = thunder::wire::DEFAULT_MAX_FRAME_BYTES;

// Packet types (the high nibble of byte 0).
const CONNECT: u8 = 1;
const CONNACK: u8 = 2;
const PUBLISH: u8 = 3;
const SUBSCRIBE: u8 = 8;
const SUBACK: u8 = 9;
const PINGREQ: u8 = 12;
const PINGRESP: u8 = 13;
const DISCONNECT: u8 = 14;

// The two PUBLISH properties this lane needs.
const PROP_RESPONSE_TOPIC: u8 = 0x08;
const PROP_CORRELATION_DATA: u8 = 0x09;

/// Ride through a poisoned lock: the guarded state stays consistent.
fn lock<T>(mutex: &StdMutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

// ── Encoding primitives ─────────────────────────────────────────────────────

/// Append a Variable Byte Integer (MQTT's 7-bits-plus-continuation length).
pub fn put_varint(out: &mut Vec<u8>, mut value: usize) {
    loop {
        let mut byte = (value % 128) as u8;
        value /= 128;
        if value > 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if value == 0 {
            break;
        }
    }
}

/// Read a Variable Byte Integer, returning `(value, bytes_consumed)`.
pub fn take_varint(bytes: &[u8]) -> Option<(usize, usize)> {
    let mut value = 0usize;
    let mut multiplier = 1usize;
    for (index, byte) in bytes.iter().enumerate() {
        if index == 4 {
            return None; // a VBI is at most 4 bytes
        }
        value += (*byte & 0x7f) as usize * multiplier;
        if *byte & 0x80 == 0 {
            return Some((value, index + 1));
        }
        multiplier *= 128;
    }
    None
}

/// Append a UTF-8 string with its 2-byte big-endian length.
fn put_string(out: &mut Vec<u8>, value: &str) {
    out.extend_from_slice(&(value.len() as u16).to_be_bytes());
    out.extend_from_slice(value.as_bytes());
}

/// Append binary data with its 2-byte big-endian length.
fn put_binary(out: &mut Vec<u8>, value: &[u8]) {
    out.extend_from_slice(&(value.len() as u16).to_be_bytes());
    out.extend_from_slice(value);
}

/// Read a length-prefixed string, returning `(value, bytes_consumed)`.
fn take_string(bytes: &[u8]) -> Option<(String, usize)> {
    let (value, used) = take_binary(bytes)?;
    Some((String::from_utf8(value).ok()?, used))
}

/// Read length-prefixed binary data, returning `(value, bytes_consumed)`.
fn take_binary(bytes: &[u8]) -> Option<(Vec<u8>, usize)> {
    if bytes.len() < 2 {
        return None;
    }
    let len = u16::from_be_bytes([bytes[0], bytes[1]]) as usize;
    if bytes.len() < 2 + len {
        return None;
    }
    Some((bytes[2..2 + len].to_vec(), 2 + len))
}

/// Wrap a variable header + payload in the fixed header.
fn packet(kind: u8, flags: u8, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len() + 5);
    out.push((kind << 4) | flags);
    put_varint(&mut out, body.len());
    out.extend_from_slice(body);
    out
}

// ── Packets ─────────────────────────────────────────────────────────────────

/// A QoS 0 `PUBLISH`, optionally carrying the request/response properties.
pub fn encode_publish(
    topic: &str,
    response_topic: Option<&str>,
    correlation: Option<&[u8]>,
    payload: &[u8],
) -> Vec<u8> {
    let mut body = Vec::with_capacity(payload.len() + topic.len() + 32);
    put_string(&mut body, topic);
    // QoS 0 carries no packet identifier.
    let mut properties = Vec::with_capacity(32);
    if let Some(response) = response_topic {
        properties.push(PROP_RESPONSE_TOPIC);
        put_string(&mut properties, response);
    }
    if let Some(data) = correlation {
        properties.push(PROP_CORRELATION_DATA);
        put_binary(&mut properties, data);
    }
    put_varint(&mut body, properties.len());
    body.extend_from_slice(&properties);
    body.extend_from_slice(payload);
    packet(PUBLISH, 0, &body)
}

/// What a decoded `PUBLISH` carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Publish {
    /// Destination topic.
    pub topic: String,
    /// Where the responder should reply (property 0x08).
    pub response_topic: Option<String>,
    /// Bytes the requester uses to match reply to request (property 0x09).
    pub correlation: Option<Vec<u8>>,
    /// The message body.
    pub payload: Vec<u8>,
}

/// Decode a `PUBLISH` body (everything after the fixed header).
pub fn decode_publish(body: &[u8]) -> Option<Publish> {
    let (topic, mut at) = take_string(body)?;
    let (properties_len, used) = take_varint(&body[at..])?;
    at += used;
    let properties_end = at.checked_add(properties_len)?;
    if properties_end > body.len() {
        return None;
    }
    let mut response_topic = None;
    let mut correlation = None;
    let mut cursor = at;
    while cursor < properties_end {
        let identifier = body[cursor];
        cursor += 1;
        match identifier {
            PROP_RESPONSE_TOPIC => {
                let (value, used) = take_string(&body[cursor..])?;
                response_topic = Some(value);
                cursor += used;
            }
            PROP_CORRELATION_DATA => {
                let (value, used) = take_binary(&body[cursor..])?;
                correlation = Some(value);
                cursor += used;
            }
            // This peer emits no other properties, so anything else is a
            // packet it did not produce — refuse rather than guess at lengths.
            _ => return None,
        }
    }
    Some(Publish {
        topic,
        response_topic,
        correlation,
        payload: body[properties_end..].to_vec(),
    })
}

/// A minimal MQTT 5 `CONNECT`.
fn encode_connect(client_id: &str) -> Vec<u8> {
    let mut body = Vec::with_capacity(client_id.len() + 16);
    put_string(&mut body, "MQTT");
    body.push(5); // protocol version
    body.push(0x02); // connect flags: clean start
    body.extend_from_slice(&60u16.to_be_bytes()); // keep-alive seconds
    put_varint(&mut body, 0); // no properties
    put_string(&mut body, client_id);
    packet(CONNECT, 0, &body)
}

/// `CONNACK` with reason code 0 (success).
fn encode_connack() -> Vec<u8> {
    packet(CONNACK, 0, &[0x00, 0x00, 0x00])
}

/// A QoS 0 `SUBSCRIBE` for one topic filter.
fn encode_subscribe(packet_id: u16, filter: &str) -> Vec<u8> {
    let mut body = Vec::with_capacity(filter.len() + 8);
    body.extend_from_slice(&packet_id.to_be_bytes());
    put_varint(&mut body, 0); // no properties
    put_string(&mut body, filter);
    body.push(0x00); // options: QoS 0
    packet(SUBSCRIBE, 0x02, &body) // SUBSCRIBE requires flags 0b0010
}

/// `SUBACK` granting QoS 0.
fn encode_suback(packet_id: u16) -> Vec<u8> {
    let mut body = Vec::with_capacity(4);
    body.extend_from_slice(&packet_id.to_be_bytes());
    put_varint(&mut body, 0); // no properties
    body.push(0x00); // granted QoS 0
    packet(SUBACK, 0, &body)
}

/// Does a subscription filter match a topic? Supports `+` (one level) and `#`
/// (the rest).
pub fn topic_matches(filter: &str, topic: &str) -> bool {
    let mut filter_levels = filter.split('/');
    let mut topic_levels = topic.split('/');
    loop {
        match (filter_levels.next(), topic_levels.next()) {
            (Some("#"), Some(_)) => return true,
            (Some("+"), Some(_)) => continue,
            (Some(f), Some(t)) if f == t => continue,
            (None, None) => return true,
            _ => return false,
        }
    }
}

// ── Broker ──────────────────────────────────────────────────────────────────

#[derive(Debug, Default)]
struct MqttMetrics {
    bytes_in: AtomicU64,
    bytes_out: AtomicU64,
}

#[derive(Debug, Default)]
struct Broker {
    /// filter → connections subscribed to it.
    subscriptions: Vec<(String, u64)>,
    outboxes: HashMap<u64, mpsc::UnboundedSender<Vec<u8>>>,
}

impl Broker {
    fn deliver(&self, publish: &Publish) {
        let frame = encode_publish(
            &publish.topic,
            publish.response_topic.as_deref(),
            publish.correlation.as_deref(),
            &publish.payload,
        );
        for (filter, connection) in &self.subscriptions {
            if !topic_matches(filter, &publish.topic) {
                continue;
            }
            if let Some(outbox) = self.outboxes.get(connection) {
                let _ = outbox.send(frame.clone());
            }
        }
    }
}

/// Handle to the running broker plus its responder.
#[derive(Debug)]
pub struct MqttHandle {
    addr: SocketAddr,
    metrics: Arc<MqttMetrics>,
    shutdown: watch::Sender<bool>,
}

impl MqttHandle {
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

impl Drop for MqttHandle {
    fn drop(&mut self) {
        let _ = self.shutdown.send(true);
    }
}

/// Spawn the broker and the responder that serves the shared no-op backend.
pub async fn spawn_mqtt_broker(
    backend: Arc<NoopBackend>,
    addr: SocketAddr,
) -> std::io::Result<MqttHandle> {
    let listener = TcpListener::bind(addr).await?;
    let addr = listener.local_addr()?;
    let metrics = Arc::new(MqttMetrics::default());
    let broker = Arc::new(StdMutex::new(Broker::default()));
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    let accept_broker = Arc::clone(&broker);
    let accept_metrics = Arc::clone(&metrics);
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
                Arc::clone(&accept_broker),
                Arc::clone(&accept_metrics),
                accept_shutdown.clone(),
            ));
        }
    });

    // The responder: a separate connection, as in the NATS lane, so the
    // four-traversal shape stays faithful.
    let (ready_tx, ready_rx) = oneshot::channel();
    let mut responder_shutdown = shutdown_rx;
    tokio::spawn(async move {
        let Ok(mut client) = MqttClient::connect(addr, "responder").await else {
            let _ = ready_tx.send(false);
            return;
        };
        if client.subscribe(REQUEST_TOPIC).await.is_err() {
            let _ = ready_tx.send(false);
            return;
        }
        let _ = ready_tx.send(true);
        loop {
            let publish = tokio::select! {
                _ = responder_shutdown.wait_for(|stop| *stop) => break,
                publish = client.next_publish() => publish,
            };
            let Ok(publish) = publish else { break };
            let (Some(response_topic), correlation) =
                (publish.response_topic.clone(), publish.correlation.clone())
            else {
                continue;
            };
            let (command, payload) = split_request(&publish.payload);
            let value = match backend.respond(&command, command_args(&command, payload)) {
                Ok(value) => value_to_bytes(value),
                Err(error) => error.into_bytes(),
            };
            if client
                .publish(&response_topic, None, correlation.as_deref(), &value)
                .await
                .is_err()
            {
                break;
            }
        }
    });
    let _ = ready_rx.await;

    Ok(MqttHandle {
        addr,
        metrics,
        shutdown: shutdown_tx,
    })
}

/// A request payload is `<COMMAND> <payload>`.
fn split_request(bytes: &[u8]) -> (String, Vec<u8>) {
    match bytes.iter().position(|byte| *byte == b' ') {
        Some(at) => (
            String::from_utf8_lossy(&bytes[..at]).into_owned(),
            bytes[at + 1..].to_vec(),
        ),
        None => (String::from_utf8_lossy(bytes).into_owned(), Vec::new()),
    }
}

fn value_to_bytes(value: Value) -> Vec<u8> {
    match value {
        Value::Str(s) => s.into_bytes(),
        Value::Bytes(b) => b.to_vec(),
        _ => Vec::new(),
    }
}

fn command_args(command: &str, payload: Vec<u8>) -> Vec<Value> {
    match command {
        "ECHO" if !payload.is_empty() => vec![Value::bytes(payload)],
        _ => vec![],
    }
}

/// One broker connection.
async fn serve_conn(
    stream: TcpStream,
    connection: u64,
    broker: Arc<StdMutex<Broker>>,
    metrics: Arc<MqttMetrics>,
    mut shutdown: watch::Receiver<bool>,
) {
    let _ = stream.set_nodelay(true);
    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);
    let (outbox_tx, mut outbox_rx) = mpsc::unbounded_channel::<Vec<u8>>();
    lock(&broker).outboxes.insert(connection, outbox_tx);

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

    loop {
        let read = tokio::select! {
            _ = shutdown.wait_for(|stop| *stop) => break,
            read = read_packet(&mut reader) => read,
        };
        let Ok((kind, _flags, body)) = read else {
            break;
        };
        metrics
            .bytes_in
            .fetch_add((body.len() + 2) as u64, Ordering::Relaxed);
        let queued = match kind {
            CONNECT => send(&broker, connection, encode_connack()),
            SUBSCRIBE => {
                if body.len() < 2 {
                    break;
                }
                let packet_id = u16::from_be_bytes([body[0], body[1]]);
                let Some((properties_len, used)) = take_varint(&body[2..]) else {
                    break;
                };
                let at = 2 + used + properties_len;
                let Some((filter, _)) = take_string(&body[at..]) else {
                    break;
                };
                lock(&broker).subscriptions.push((filter, connection));
                send(&broker, connection, encode_suback(packet_id))
            }
            PUBLISH => {
                let Some(publish) = decode_publish(&body) else {
                    break;
                };
                lock(&broker).deliver(&publish);
                Ok(())
            }
            PINGREQ => send(&broker, connection, packet(PINGRESP, 0, &[])),
            DISCONNECT => break,
            _ => break,
        };
        if queued.is_err() {
            break;
        }
    }

    {
        let mut guard = lock(&broker);
        guard.outboxes.remove(&connection);
        guard.subscriptions.retain(|(_, c)| *c != connection);
    }
    writer.abort();
}

fn send(broker: &Arc<StdMutex<Broker>>, connection: u64, frame: Vec<u8>) -> Result<(), ()> {
    let guard = lock(broker);
    let Some(outbox) = guard.outboxes.get(&connection) else {
        return Err(());
    };
    outbox.send(frame).map_err(|_| ())
}

/// Read one packet: fixed header, then the declared remaining length.
async fn read_packet(
    reader: &mut BufReader<OwnedReadHalf>,
) -> Result<(u8, u8, Vec<u8>), std::io::Error> {
    let mut first = [0u8; 1];
    reader.read_exact(&mut first).await?;
    let kind = first[0] >> 4;
    let flags = first[0] & 0x0f;
    // Remaining length is a Variable Byte Integer read one byte at a time.
    let mut length = 0usize;
    let mut multiplier = 1usize;
    for _ in 0..4 {
        let mut byte = [0u8; 1];
        reader.read_exact(&mut byte).await?;
        length += (byte[0] & 0x7f) as usize * multiplier;
        if byte[0] & 0x80 == 0 {
            break;
        }
        multiplier *= 128;
    }
    if length > MAX_PACKET {
        return Err(std::io::Error::other("mqtt packet too large"));
    }
    let mut body = vec![0u8; length];
    reader.read_exact(&mut body).await?;
    Ok((kind, flags, body))
}

// ── Client ──────────────────────────────────────────────────────────────────

/// A minimal MQTT client — used by both the responder and the driver.
struct MqttClient {
    reader: BufReader<OwnedReadHalf>,
    writer: tokio::net::tcp::OwnedWriteHalf,
    next_packet_id: u16,
}

impl MqttClient {
    async fn connect(addr: SocketAddr, client_id: &str) -> Result<Self, String> {
        let stream = TcpStream::connect(addr)
            .await
            .map_err(|e| format!("mqtt connect failed: {e}"))?;
        stream
            .set_nodelay(true)
            .map_err(|e| format!("mqtt nodelay failed: {e}"))?;
        let (read_half, write_half) = stream.into_split();
        let mut client = Self {
            reader: BufReader::new(read_half),
            writer: write_half,
            next_packet_id: 1,
        };
        client
            .writer
            .write_all(&encode_connect(client_id))
            .await
            .map_err(|e| format!("mqtt CONNECT write failed: {e}"))?;
        let (kind, _, _) = read_packet(&mut client.reader)
            .await
            .map_err(|e| format!("mqtt CONNACK read failed: {e}"))?;
        if kind != CONNACK {
            return Err(format!("mqtt expected CONNACK, got packet type {kind}"));
        }
        Ok(client)
    }

    async fn subscribe(&mut self, filter: &str) -> Result<(), String> {
        let packet_id = self.next_packet_id;
        self.next_packet_id = self.next_packet_id.wrapping_add(1).max(1);
        self.writer
            .write_all(&encode_subscribe(packet_id, filter))
            .await
            .map_err(|e| format!("mqtt SUBSCRIBE write failed: {e}"))?;
        let (kind, _, _) = read_packet(&mut self.reader)
            .await
            .map_err(|e| format!("mqtt SUBACK read failed: {e}"))?;
        if kind != SUBACK {
            return Err(format!("mqtt expected SUBACK, got packet type {kind}"));
        }
        Ok(())
    }

    async fn publish(
        &mut self,
        topic: &str,
        response_topic: Option<&str>,
        correlation: Option<&[u8]>,
        payload: &[u8],
    ) -> Result<(), String> {
        self.writer
            .write_all(&encode_publish(topic, response_topic, correlation, payload))
            .await
            .map_err(|e| format!("mqtt PUBLISH write failed: {e}"))
    }

    async fn next_publish(&mut self) -> Result<Publish, String> {
        loop {
            let (kind, _, body) = read_packet(&mut self.reader)
                .await
                .map_err(|e| format!("mqtt read failed: {e}"))?;
            if kind != PUBLISH {
                continue;
            }
            return decode_publish(&body).ok_or_else(|| "mqtt malformed PUBLISH".to_owned());
        }
    }
}

// ── Driver ──────────────────────────────────────────────────────────────────

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
        Some(other) => return Err(format!("mqtt lane: unsupported arg {other:?}")),
        None => {}
    }
    Ok(out)
}

/// One driver connection: a client subscribed to its own reply topic, with a
/// window of outstanding requests correlated by Correlation Data.
struct Requester {
    client: MqttClient,
    reply_topic: String,
}

impl Requester {
    async fn connect(addr: SocketAddr, index: usize) -> Result<Self, String> {
        let reply_topic = format!("bench/reply/{index}");
        let mut client = MqttClient::connect(addr, &format!("requester-{index}")).await?;
        client.subscribe(&reply_topic).await?;
        Ok(Self {
            client,
            reply_topic,
        })
    }

    /// One continuously-full window of `depth` outstanding requests.
    ///
    /// QoS 0 PUBLISH carries no packet identifier, so requests are matched to
    /// replies by Correlation Data — the MQTT 5 mechanism this lane exists to
    /// exercise.
    async fn window(
        &mut self,
        depth: usize,
        ops: usize,
        payload: &[u8],
    ) -> Result<Vec<Duration>, String> {
        let mut latencies = Vec::with_capacity(ops);
        let mut sent = HashMap::with_capacity(depth.max(1));
        let mut issued = 0u64;
        let mut completed = 0usize;

        while completed < ops {
            while sent.len() < depth.max(1) && (issued as usize) < ops {
                let correlation = issued.to_be_bytes().to_vec();
                self.client
                    .publish(
                        REQUEST_TOPIC,
                        Some(&self.reply_topic),
                        Some(&correlation),
                        payload,
                    )
                    .await?;
                sent.insert(correlation, Instant::now());
                issued += 1;
            }
            let reply = self.client.next_publish().await?;
            let Some(correlation) = reply.correlation else {
                return Err("mqtt reply without correlation data".to_owned());
            };
            let Some(started) = sent.remove(&correlation) else {
                return Err("mqtt reply for an unknown request".to_owned());
            };
            latencies.push(started.elapsed());
            completed += 1;
        }
        Ok(latencies)
    }
}

/// Measure one matrix cell on the MQTT lane.
pub async fn cell(
    handle: &MqttHandle,
    spec: &CellSpec,
    cfg: &RunConfig,
) -> Result<Measured, String> {
    let addr = handle.local_addr();
    let payload = build_payload(spec.command, &spec.args)?;
    let mut requesters = Vec::with_capacity(spec.connections);
    for index in 0..spec.connections {
        requesters.push(Requester::connect(addr, index).await?);
    }

    if cfg.warmup > 0 {
        for requester in &mut requesters {
            requester.window(spec.depth, cfg.warmup, &payload).await?;
        }
    }

    let before_in = handle.bytes_in();
    let before_out = handle.bytes_out();
    let mut reps = Vec::with_capacity(cfg.repetitions);
    let mut ops = 0u64;
    for _ in 0..cfg.repetitions {
        let per_conn = (cfg.ops / requesters.len().max(1)).max(spec.depth).max(1);
        let started = Instant::now();
        let mut tasks = Vec::with_capacity(requesters.len());
        for mut requester in requesters.drain(..) {
            let payload = payload.clone();
            let depth = spec.depth;
            tasks.push(tokio::spawn(async move {
                let latencies = requester.window(depth, per_conn, &payload).await;
                (requester, latencies)
            }));
        }
        let mut all = Vec::with_capacity(per_conn * tasks.len());
        for task in tasks {
            let (requester, latencies) = task
                .await
                .map_err(|e| format!("mqtt worker panicked: {e}"))?;
            requesters.push(requester);
            all.extend(latencies?);
        }
        ops += all.len() as u64;
        reps.push(compute(&mut all, started.elapsed()));
    }
    let after_in = handle.bytes_in();
    let after_out = handle.bytes_out();
    drop(requesters);

    // As in the NATS lane, bytes are counted at the BROKER across all four
    // traversals of a round trip, so they are not comparable with a
    // point-to-point lane's.
    let ops = ops.max(1) as f64;
    Ok((
        reps,
        (after_in - before_in) as f64 / ops,
        (after_out - before_out) as f64 / ops,
    ))
}

/// The connection-storm cell: connect + subscribe + one round trip, repeated.
pub async fn storm(
    handle: &MqttHandle,
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
    let mut requester = Requester::connect(addr, 0).await?;
    requester.window(1, 1, payload).await?;
    Ok(started.elapsed())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::backend::STATIC_REPLY_BYTES;

    /// The Variable Byte Integer is MQTT's whole framing story — verify it at
    /// every boundary the spec calls out.
    #[test]
    fn varints_round_trip_at_every_boundary() {
        for value in [0usize, 1, 127, 128, 16_383, 16_384, 2_097_151, 2_097_152] {
            let mut encoded = Vec::new();
            put_varint(&mut encoded, value);
            let (decoded, used) = take_varint(&encoded).unwrap();
            assert_eq!(decoded, value, "round trip failed for {value}");
            assert_eq!(used, encoded.len());
        }
    }

    #[test]
    fn varint_widths_match_the_spec() {
        let widths = [
            (0usize, 1usize),
            (127, 1),
            (128, 2),
            (16_383, 2),
            (16_384, 3),
            (2_097_152, 4),
        ];
        for (value, expected) in widths {
            let mut encoded = Vec::new();
            put_varint(&mut encoded, value);
            assert_eq!(encoded.len(), expected, "width wrong for {value}");
        }
    }

    #[test]
    fn a_varint_never_exceeds_four_bytes() {
        assert_eq!(take_varint(&[0x80, 0x80, 0x80, 0x80, 0x01]), None);
    }

    #[test]
    fn publish_round_trips_with_request_response_properties() {
        let framed = encode_publish(
            "bench/call",
            Some("bench/reply/7"),
            Some(&[1, 2, 3, 4]),
            b"ECHO hi",
        );
        // Skip the fixed header to get the body.
        let (length, used) = take_varint(&framed[1..]).unwrap();
        let body = &framed[1 + used..];
        assert_eq!(body.len(), length, "remaining length must cover the body");

        let publish = decode_publish(body).unwrap();
        assert_eq!(publish.topic, "bench/call");
        assert_eq!(publish.response_topic.as_deref(), Some("bench/reply/7"));
        assert_eq!(publish.correlation.as_deref(), Some(&[1u8, 2, 3, 4][..]));
        assert_eq!(publish.payload, b"ECHO hi");
    }

    #[test]
    fn publish_round_trips_without_properties() {
        let framed = encode_publish("bench/reply/0", None, None, b"pong");
        let (_, used) = take_varint(&framed[1..]).unwrap();
        let publish = decode_publish(&framed[1 + used..]).unwrap();
        assert_eq!(publish.topic, "bench/reply/0");
        assert!(publish.response_topic.is_none());
        assert!(publish.correlation.is_none());
        assert_eq!(publish.payload, b"pong");
    }

    #[test]
    fn a_qos0_publish_carries_no_packet_identifier() {
        // Fixed header flags must be 0: QoS 0, not retained, not duplicate.
        let framed = encode_publish("t", None, None, b"x");
        assert_eq!(framed[0], PUBLISH << 4, "QoS 0 PUBLISH has zero flags");
    }

    #[test]
    fn subscribe_uses_the_reserved_flag_bits() {
        // The spec fixes SUBSCRIBE's flags at 0b0010; brokers reject anything else.
        let framed = encode_subscribe(1, "bench/call");
        assert_eq!(framed[0], (SUBSCRIBE << 4) | 0x02);
    }

    #[test]
    fn connect_declares_mqtt_5() {
        let framed = encode_connect("bench");
        let (_, used) = take_varint(&framed[1..]).unwrap();
        let body = &framed[1 + used..];
        let (protocol, at) = take_string(body).unwrap();
        assert_eq!(protocol, "MQTT");
        assert_eq!(body[at], 5, "protocol version must be 5 for properties");
    }

    #[test]
    fn topic_wildcards_match_per_spec() {
        assert!(topic_matches("bench/call", "bench/call"));
        assert!(!topic_matches("bench/call", "bench/other"));
        assert!(topic_matches("bench/+", "bench/call"));
        assert!(!topic_matches("bench/+", "bench/call/deep"));
        assert!(topic_matches("bench/#", "bench/call/deep"));
        assert!(!topic_matches("other/#", "bench/call"));
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
}

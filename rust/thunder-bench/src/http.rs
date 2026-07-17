//! Minimal hand-rolled HTTP/1.1 + JSON listener over the same no-op
//! backend (BEN-001, FR-70) — same process, host, runtime and allocator as
//! the Thunder listener.
//!
//! Protocol surface (exactly what the matrix needs, in BEN-002's honesty
//! spirit): `POST` with a JSON body `{"command": "...", "args": [...]}`,
//! answered `200` with `{"ok": true, "result": ...}` or
//! `{"ok": false, "error": "..."}` (dispatch errors travel in-band, like
//! Thunder's `Response::err`). Keep-alive per HTTP/1.1 defaults;
//! pipelined requests are read back-to-back from the buffered reader, and
//! the writer only flushes when no further pipelined request is already
//! buffered — the drain-then-flush analog of the Thunder listener
//! (SRV-006).
//!
//! **T4.2 note:** this is a no-op harness lane, deliberately dependency
//! free. If the real-shootout parity review (BEN-003) demands a
//! production-grade HTTP stack, an axum + serde_json lane may replace this
//! module at T4.2 — the driver only depends on the wire shape above.
//!
//! `Bytes` values are mapped to JSON integer arrays (the family's legacy
//! form); the base64-vs-raw payload comparison is exactly the
//! embedding-768 scenario's business and lands with it at T4.3.

use std::io;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use thunder_wire::Value;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, watch};

use crate::backend::NoopBackend;

/// Cap on one request body — mirrors the Thunder frame cap (WIRE-020).
const MAX_HTTP_BODY: usize = thunder_wire::DEFAULT_MAX_FRAME_BYTES;

/// Cap on one request/response head.
const MAX_HTTP_HEAD: usize = 16 * 1024;

/// Server-side counters for the HTTP lane — the same measurement point as
/// the Thunder listener's SRV-030 metrics (bytes counted at the socket,
/// recorded after the successful write).
#[derive(Debug, Default)]
pub struct HttpMetrics {
    bytes_in: AtomicU64,
    bytes_out: AtomicU64,
    requests: AtomicU64,
}

impl HttpMetrics {
    fn record_in(&self, bytes: usize) {
        self.bytes_in.fetch_add(bytes as u64, Ordering::Relaxed);
    }

    fn record_response(&self, bytes: usize) {
        self.bytes_out.fetch_add(bytes as u64, Ordering::Relaxed);
        self.requests.fetch_add(1, Ordering::Relaxed);
    }

    fn snapshot(&self) -> HttpMetricsSnapshot {
        HttpMetricsSnapshot {
            bytes_in: self.bytes_in.load(Ordering::Relaxed),
            bytes_out: self.bytes_out.load(Ordering::Relaxed),
            requests: self.requests.load(Ordering::Relaxed),
        }
    }
}

/// Point-in-time copy of the HTTP lane's counters.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HttpMetricsSnapshot {
    /// Request bytes read from sockets (head + body).
    pub bytes_in: u64,
    /// Response bytes written to sockets (head + body).
    pub bytes_out: u64,
    /// Responses written.
    pub requests: u64,
}

/// Handle to the running HTTP listener — same shape as
/// [`thunder_server::ListenerHandle`].
#[derive(Debug)]
pub struct HttpHandle {
    local_addr: SocketAddr,
    shutdown: watch::Sender<bool>,
    metrics: Arc<HttpMetrics>,
    done: Option<mpsc::Receiver<()>>,
}

impl HttpHandle {
    /// The bound address (resolves port `0` binds).
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Point-in-time lane counters.
    pub fn snapshot(&self) -> HttpMetricsSnapshot {
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

impl Drop for HttpHandle {
    fn drop(&mut self) {
        let _ = self.shutdown.send(true);
    }
}

/// Bind `addr` and serve the no-op backend over minimal HTTP/1.1 + JSON.
pub async fn spawn_http_listener(
    backend: Arc<NoopBackend>,
    addr: SocketAddr,
) -> io::Result<HttpHandle> {
    let listener = TcpListener::bind(addr).await?;
    let local_addr = listener.local_addr()?;
    let metrics = Arc::new(HttpMetrics::default());
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let (done_tx, done_rx) = mpsc::channel::<()>(1);
    tokio::spawn(accept_loop(
        listener,
        backend,
        Arc::clone(&metrics),
        shutdown_rx,
        done_tx,
    ));
    Ok(HttpHandle {
        local_addr,
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
    metrics: Arc<HttpMetrics>,
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

/// One connection: sequential request → response loop (HTTP/1.1 semantics),
/// with the flush deferred while another pipelined request is already
/// buffered (SRV-006 analog).
async fn handle_connection(
    stream: TcpStream,
    backend: Arc<NoopBackend>,
    metrics: Arc<HttpMetrics>,
    mut shutdown: watch::Receiver<bool>,
) {
    // Parity with the Thunder listener: Nagle off (SRV-008).
    let _ = stream.set_nodelay(true);
    let (read_half, write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);
    let mut writer = BufWriter::new(write_half);
    loop {
        let read = tokio::select! {
            _ = shutdown.wait_for(|stop| *stop) => break,
            read = read_http_request(&mut reader) => read,
        };
        let request = match read {
            Ok(Some(request)) => request,
            // Clean EOF between requests: the client is done.
            Ok(None) => break,
            Err(_) => {
                // Malformed head/body: best-effort 400, then close.
                let _ =
                    write_response(&mut writer, 400, &json_error("bad request"), &metrics).await;
                break;
            }
        };
        metrics.record_in(request.bytes);
        let (status, body, keep_alive) = respond(&backend, &request);
        if write_response(&mut writer, status, &body, &metrics)
            .await
            .is_err()
        {
            break;
        }
        // Drain-then-flush analog: skip the flush while another pipelined
        // request is already sitting in the read buffer.
        if reader.buffer().is_empty() && writer.flush().await.is_err() {
            break;
        }
        if !keep_alive {
            break;
        }
    }
    let _ = writer.flush().await;
}

/// One parsed request: just enough HTTP for the harness.
struct HttpRequest {
    method: String,
    keep_alive: bool,
    body: Vec<u8>,
    /// Total bytes consumed from the socket (head + body) — the in-bytes
    /// metric.
    bytes: usize,
}

/// Route one request through the shared backend. Returns
/// `(status, body, keep_alive)`.
fn respond(backend: &NoopBackend, request: &HttpRequest) -> (u16, String, bool) {
    if request.method != "POST" {
        return (405, json_error("only POST is supported"), false);
    }
    match parse_rpc_body(&request.body) {
        Ok((command, args)) => {
            let body = match backend.respond(&command, args) {
                Ok(value) => {
                    serde_json::json!({ "ok": true, "result": wire_to_json(&value) }).to_string()
                }
                // Dispatch errors travel in-band, like Thunder's
                // Response::err — the connection stays usable.
                Err(message) => serde_json::json!({ "ok": false, "error": message }).to_string(),
            };
            (200, body, request.keep_alive)
        }
        Err(message) => (400, json_error(&message), false),
    }
}

/// Parse `{"command": "...", "args": [...]}`.
fn parse_rpc_body(body: &[u8]) -> Result<(String, Vec<Value>), String> {
    let parsed: serde_json::Value =
        serde_json::from_slice(body).map_err(|e| format!("body is not JSON: {e}"))?;
    let command = parsed
        .get("command")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "missing string field 'command'".to_owned())?
        .to_owned();
    let args = match parsed.get("args") {
        None | Some(serde_json::Value::Null) => Vec::new(),
        Some(serde_json::Value::Array(items)) => items.iter().map(json_to_wire).collect(),
        Some(_) => return Err("'args' must be an array".to_owned()),
    };
    Ok((command, args))
}

fn json_error(message: &str) -> String {
    serde_json::json!({ "ok": false, "error": message }).to_string()
}

/// Write one response; bytes recorded after the successful write
/// (SRV-030 parity). The flush policy belongs to the caller.
async fn write_response(
    writer: &mut BufWriter<OwnedWriteHalf>,
    status: u16,
    body: &str,
    metrics: &HttpMetrics,
) -> io::Result<()> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        405 => "Method Not Allowed",
        _ => "Error",
    };
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
        body.len()
    );
    writer.write_all(head.as_bytes()).await?;
    writer.write_all(body.as_bytes()).await?;
    metrics.record_response(head.len() + body.len());
    Ok(())
}

// ── Head/body reading (shared shape for server requests and driver-side
//    responses) ──────────────────────────────────────────────────────────────

/// Read one head: first line + lowercased headers + bytes consumed.
/// `Ok(None)` is a clean EOF before the first byte.
async fn read_head<R: AsyncBufRead + Unpin>(
    reader: &mut R,
) -> io::Result<Option<(String, Vec<(String, String)>, usize)>> {
    let mut line = Vec::new();
    let n = reader.read_until(b'\n', &mut line).await?;
    if n == 0 {
        return Ok(None);
    }
    let mut bytes = n;
    let first = String::from_utf8_lossy(&line).trim().to_owned();
    let mut headers = Vec::new();
    loop {
        line.clear();
        let n = reader.read_until(b'\n', &mut line).await?;
        if n == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "eof inside http head",
            ));
        }
        bytes += n;
        if bytes > MAX_HTTP_HEAD {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "http head too large",
            ));
        }
        let text = String::from_utf8_lossy(&line);
        let text = text.trim();
        if text.is_empty() {
            break;
        }
        if let Some((name, value)) = text.split_once(':') {
            headers.push((name.trim().to_ascii_lowercase(), value.trim().to_owned()));
        }
    }
    Ok(Some((first, headers, bytes)))
}

fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, v)| v.as_str())
}

fn content_length(headers: &[(String, String)]) -> io::Result<usize> {
    match header(headers, "content-length") {
        None => Ok(0),
        Some(value) => value
            .parse::<usize>()
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid content-length")),
    }
}

/// Read one request (head + body). `Ok(None)` is a clean EOF between
/// requests (keep-alive close).
async fn read_http_request(
    reader: &mut BufReader<OwnedReadHalf>,
) -> io::Result<Option<HttpRequest>> {
    let Some((request_line, headers, head_bytes)) = read_head(reader).await? else {
        return Ok(None);
    };
    let method = request_line
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_owned();
    let http_11 = request_line.ends_with("HTTP/1.1");
    let keep_alive = match header(&headers, "connection") {
        Some(v) if v.eq_ignore_ascii_case("close") => false,
        Some(v) if v.eq_ignore_ascii_case("keep-alive") => true,
        _ => http_11,
    };
    let length = content_length(&headers)?;
    if length > MAX_HTTP_BODY {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "http body too large",
        ));
    }
    let mut body = vec![0u8; length];
    reader.read_exact(&mut body).await?;
    Ok(Some(HttpRequest {
        method,
        keep_alive,
        body,
        bytes: head_bytes + length,
    }))
}

/// Read one response from a buffered stream: `(status, body, total bytes)`.
/// Used by the HTTP driver lane and tests.
pub async fn read_http_response<R: AsyncBufRead + Unpin>(
    reader: &mut R,
) -> io::Result<(u16, Vec<u8>, usize)> {
    let Some((status_line, headers, head_bytes)) = read_head(reader).await? else {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "eof before http response",
        ));
    };
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("malformed status line: {status_line}"),
            )
        })?;
    let length = content_length(&headers)?;
    if length > MAX_HTTP_BODY {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "http body too large",
        ));
    }
    let mut body = vec![0u8; length];
    reader.read_exact(&mut body).await?;
    Ok((status, body, head_bytes + length))
}

// ── JSON ⇄ wire value mapping ────────────────────────────────────────────────

/// Map a JSON value onto the wire [`Value`] model.
pub fn json_to_wire(value: &serde_json::Value) -> Value {
    match value {
        serde_json::Value::Null => Value::Null,
        serde_json::Value::Bool(b) => Value::Bool(*b),
        serde_json::Value::Number(n) => match n.as_i64() {
            Some(i) => Value::Int(i),
            None => Value::Float(n.as_f64().unwrap_or(f64::NAN)),
        },
        serde_json::Value::String(s) => Value::Str(s.clone()),
        serde_json::Value::Array(items) => Value::Array(items.iter().map(json_to_wire).collect()),
        serde_json::Value::Object(map) => Value::Map(
            map.iter()
                .map(|(k, v)| (Value::Str(k.clone()), json_to_wire(v)))
                .collect(),
        ),
    }
}

/// Map a wire [`Value`] onto JSON. `Bytes` become an integer array (the
/// family's legacy JSON form); non-finite floats become `null`; non-string
/// map keys are stringified.
pub fn wire_to_json(value: &Value) -> serde_json::Value {
    match value {
        Value::Null => serde_json::Value::Null,
        Value::Bool(b) => (*b).into(),
        Value::Int(i) => (*i).into(),
        Value::Float(f) => serde_json::Number::from_f64(*f)
            .map_or(serde_json::Value::Null, serde_json::Value::Number),
        Value::Bytes(bytes) => {
            serde_json::Value::Array(bytes.iter().map(|b| (*b).into()).collect())
        }
        Value::Str(s) => s.clone().into(),
        Value::Array(items) => serde_json::Value::Array(items.iter().map(wire_to_json).collect()),
        Value::Map(pairs) => {
            let mut map = serde_json::Map::with_capacity(pairs.len());
            for (k, v) in pairs {
                let key = k
                    .as_str()
                    .map_or_else(|| wire_to_json(k).to_string(), str::to_owned);
                map.insert(key, wire_to_json(v));
            }
            serde_json::Value::Object(map)
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn round_trips(json: serde_json::Value) {
        assert_eq!(wire_to_json(&json_to_wire(&json)), json);
    }

    #[test]
    fn json_wire_round_trip_covers_the_scalar_model() {
        round_trips(serde_json::json!(null));
        round_trips(serde_json::json!(true));
        round_trips(serde_json::json!(42));
        round_trips(serde_json::json!(-7.5));
        round_trips(serde_json::json!("hello"));
        round_trips(serde_json::json!([1, "two", [3], {"four": 4}]));
        round_trips(serde_json::json!({"a": 1, "b": [true, null]}));
    }

    #[test]
    fn bytes_map_to_integer_arrays() {
        assert_eq!(
            wire_to_json(&Value::Bytes(vec![1, 2, 255])),
            serde_json::json!([1, 2, 255])
        );
    }

    #[test]
    fn parse_rpc_body_extracts_command_and_args() {
        let (command, args) = parse_rpc_body(br#"{"command": "ECHO", "args": ["x"]}"#).unwrap();
        assert_eq!(command, "ECHO");
        assert_eq!(args, vec![Value::Str("x".to_owned())]);
    }

    #[test]
    fn parse_rpc_body_defaults_args_and_rejects_garbage() {
        let (command, args) = parse_rpc_body(br#"{"command": "STATIC"}"#).unwrap();
        assert_eq!(command, "STATIC");
        assert!(args.is_empty());
        assert!(parse_rpc_body(b"not json").is_err());
        assert!(parse_rpc_body(br#"{"args": []}"#).is_err());
        assert!(parse_rpc_body(br#"{"command": "X", "args": 1}"#).is_err());
    }

    // ── Live listener tests ────────────────────────────────────────────────

    use std::sync::Arc;

    use tokio::io::{AsyncWriteExt, BufReader};
    use tokio::net::TcpStream;

    async fn start() -> HttpHandle {
        spawn_http_listener(
            Arc::new(NoopBackend::new()),
            std::net::SocketAddr::from(([127, 0, 0, 1], 0)),
        )
        .await
        .unwrap()
    }

    fn request_bytes(body: &str) -> Vec<u8> {
        format!(
            "POST /rpc HTTP/1.1\r\nHost: bench\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        )
        .into_bytes()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn echo_over_http_round_trips() {
        let handle = start().await;
        let stream = TcpStream::connect(handle.local_addr()).await.unwrap();
        let (read_half, mut write_half) = stream.into_split();
        let mut reader = BufReader::new(read_half);

        write_half
            .write_all(&request_bytes(r#"{"command":"ECHO","args":["hi"]}"#))
            .await
            .unwrap();
        let (status, body, _bytes) = read_http_response(&mut reader).await.unwrap();
        assert_eq!(status, 200);
        let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed, serde_json::json!({"ok": true, "result": "hi"}));

        // Keep-alive: a second request on the same connection works, and a
        // dispatch error stays in-band with status 200.
        write_half
            .write_all(&request_bytes(r#"{"command":"NOPE"}"#))
            .await
            .unwrap();
        let (status, body, _bytes) = read_http_response(&mut reader).await.unwrap();
        assert_eq!(status, 200);
        let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed["ok"], serde_json::json!(false));

        let snapshot = handle.snapshot();
        assert_eq!(snapshot.requests, 2);
        assert!(snapshot.bytes_in > 0);
        assert!(snapshot.bytes_out > 0);
        handle.stop().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn non_post_gets_405_and_bad_json_gets_400() {
        let handle = start().await;

        let stream = TcpStream::connect(handle.local_addr()).await.unwrap();
        let (read_half, mut write_half) = stream.into_split();
        let mut reader = BufReader::new(read_half);
        write_half
            .write_all(b"GET / HTTP/1.1\r\nHost: bench\r\n\r\n")
            .await
            .unwrap();
        let (status, _body, _bytes) = read_http_response(&mut reader).await.unwrap();
        assert_eq!(status, 405);

        let stream = TcpStream::connect(handle.local_addr()).await.unwrap();
        let (read_half, mut write_half) = stream.into_split();
        let mut reader = BufReader::new(read_half);
        write_half
            .write_all(&request_bytes("not json"))
            .await
            .unwrap();
        let (status, _body, _bytes) = read_http_response(&mut reader).await.unwrap();
        assert_eq!(status, 400);

        handle.stop().await;
    }
}

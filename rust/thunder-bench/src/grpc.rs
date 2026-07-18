//! **gRPC** lane — the real `tonic` server and client (HTTP/2 + protobuf) over
//! the same no-op backend (BEN-001, BEN-002), in the same process, host,
//! runtime and allocator as the Thunder listener.
//!
//! # Why this lane — the one that answers the d1/c4 question
//!
//! Every other peer in the shootout is **FIFO**: one connection, replies in
//! request order, no demultiplexing. Thunder is not — it multiplexes, matching
//! replies to requests by id, which is what wins it the pipelined cells and
//! what the d1/c4 analysis suspected of costing it on sync-tiny payloads.
//! Against FIFO peers that cost is unfalsifiable: any gap could be Thunder's
//! implementation rather than the price of multiplexing itself.
//!
//! gRPC breaks the tie. It is **multiplexed too** — concurrent HTTP/2 streams
//! over one connection, replies interleaved — so it must pay the same kind of
//! per-call demux cost. If gRPC shows the same sync-tiny shape, that cost is
//! the price of the design and not a Thunder defect; if it does not, the gap
//! is ours to fix. This lane exists to make that falsifiable.
//!
//! # Real crate, both sides (and why the driver differs here)
//!
//! Server *and* client are `tonic`. Every other lane drives a hand-written
//! parity driver, because BEN-003 wants one concurrency model everywhere. Here
//! that is impossible and would also be wrong:
//!
//! - **Impossible**: a faithful client means HTTP/2 framing, HPACK, flow
//!   control and stream state. Hand-rolling that would measure the quality of
//!   the hand-roll, which is exactly the bias the real-crate policy exists to
//!   remove.
//! - **Wrong**: "pipeline depth" on a FIFO wire means N requests written
//!   before reading N replies. On a multiplexed wire it means N *concurrent
//!   streams*. Forcing gRPC into the FIFO shape would serialize the very
//!   property under test.
//!
//! So the driver keeps the same *contract* — a continuously-full window of
//! `depth` in-flight requests per connection, latency measured per request
//! from just-before-send to reply-received — and implements it with
//! `buffer_unordered`, which is the multiplexed spelling of the same thing.
//!
//! **One TCP connection per driver connection** is verified, not assumed: a
//! `tonic` `Channel` holds exactly one HTTP/2 connection with no pool, clones
//! share it, and the listener counts accepts so a run that silently opened a
//! second connection fails loudly ([`GrpcHandle::connections`]).
//!
//! # Determinism knobs
//!
//! HTTP/2 flow-control windows are set explicitly on both ends rather than
//! left adaptive: `http2_adaptive_window` tunes itself from observed
//! bandwidth-delay product, which makes runs differ from each other. Fixed
//! windows trade a little peak throughput for a reproducible number, which is
//! the right trade for a benchmark (BEN-011).
//!
//! # Scope (honesty note, BEN-002)
//!
//! A **benchmark peer, not a service**. One unary method, `/bench.Bench/Call`,
//! carrying `{command, payload}` and returning `{value}` — `command` selects
//! the backend mode (`ECHO`/`STATIC`/`SINK`/`PING`). Messages are declared
//! with `prost` derives rather than a `.proto` file, so the build needs no
//! `protoc`; the wire encoding is identical either way. No TLS, no
//! interceptors, no compression, no streaming methods, no reflection.

use std::io::IoSlice;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use futures::stream::{self, StreamExt};
use http::uri::PathAndQuery;
use thunder::wire::Value;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use tokio_stream::wrappers::TcpListenerStream;
use tonic::body::Body;
use tonic::server::{Grpc as ServerGrpc, NamedService, UnaryService};
use tonic::transport::server::{Connected, TcpConnectInfo};
use tonic::transport::{Channel, Server};
use tonic::{Request, Response, Status};
use tonic_prost::ProstCodec;

use crate::backend::NoopBackend;
use crate::driver::{CellSpec, Measured, RunConfig};
use crate::stats::compute;

/// The one method this peer serves.
const METHOD_PATH: &str = "/bench.Bench/Call";
/// Per-stream HTTP/2 flow-control window. Fixed (not adaptive) for
/// reproducibility; comfortably above the largest matrix payload.
const STREAM_WINDOW: u32 = 4 * 1024 * 1024;
/// Whole-connection HTTP/2 flow-control window — must exceed the per-stream
/// window enough to keep a deep in-flight window from stalling on credit.
const CONNECTION_WINDOW: u32 = 64 * 1024 * 1024;
/// Concurrent streams the server admits. Above the deepest matrix cell
/// (pipelined-1k) so the protocol's own limit is never what is measured.
const MAX_CONCURRENT_STREAMS: u32 = 8192;
/// `tower::Buffer` capacity on the client channel. Above the deepest cell so
/// client-side backpressure is never what is measured.
const CHANNEL_BUFFER: usize = 8192;

// ── Messages (prost derives — no .proto, no protoc) ─────────────────────────

/// `{command, payload}` — the request the matrix issues.
#[derive(Clone, PartialEq, prost::Message)]
pub struct BenchRequest {
    /// Backend mode: `ECHO` / `STATIC` / `SINK` / `PING`.
    #[prost(string, tag = "1")]
    pub command: String,
    /// The payload ECHO carries; empty for the sentinels.
    #[prost(string, tag = "2")]
    pub payload: String,
}

/// `{value}` — the backend's reply.
#[derive(Clone, PartialEq, prost::Message)]
pub struct BenchReply {
    /// The reply value.
    #[prost(string, tag = "1")]
    pub value: String,
}

/// A backend reply value as the reply's `value` field.
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

// ── Byte-counting transport ─────────────────────────────────────────────────

/// Socket-level counters for the listener — the same measurement point every
/// other lane uses (bytes at the socket, not at the application).
#[derive(Debug, Default)]
pub struct ByteCounters {
    read: AtomicU64,
    written: AtomicU64,
    accepts: AtomicU64,
}

/// A `TcpStream` that counts the bytes crossing it.
///
/// Vectored writes are forwarded deliberately: HTTP/2 leans on them, and a
/// wrapper that silently drops `poll_write_vectored` would make this lane look
/// slow for a reason that has nothing to do with gRPC.
struct CountingStream {
    inner: TcpStream,
    counters: Arc<ByteCounters>,
}

impl Connected for CountingStream {
    type ConnectInfo = TcpConnectInfo;

    fn connect_info(&self) -> Self::ConnectInfo {
        TcpConnectInfo {
            local_addr: self.inner.local_addr().ok(),
            remote_addr: self.inner.peer_addr().ok(),
        }
    }
}

impl AsyncRead for CountingStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let before = buf.filled().len();
        let polled = Pin::new(&mut self.inner).poll_read(cx, buf);
        if let Poll::Ready(Ok(())) = &polled {
            let read = (buf.filled().len() - before) as u64;
            self.counters.read.fetch_add(read, Ordering::Relaxed);
        }
        polled
    }
}

impl AsyncWrite for CountingStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let polled = Pin::new(&mut self.inner).poll_write(cx, buf);
        if let Poll::Ready(Ok(written)) = &polled {
            self.counters
                .written
                .fetch_add(*written as u64, Ordering::Relaxed);
        }
        polled
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }

    fn is_write_vectored(&self) -> bool {
        self.inner.is_write_vectored()
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[IoSlice<'_>],
    ) -> Poll<std::io::Result<usize>> {
        let polled = Pin::new(&mut self.inner).poll_write_vectored(cx, bufs);
        if let Poll::Ready(Ok(written)) = &polled {
            self.counters
                .written
                .fetch_add(*written as u64, Ordering::Relaxed);
        }
        polled
    }
}

// ── Server ──────────────────────────────────────────────────────────────────

/// Serves the shared no-op backend as the one unary method.
#[derive(Clone)]
struct BenchHandler {
    backend: Arc<NoopBackend>,
}

impl BenchHandler {
    fn call(&self, request: BenchRequest) -> BenchReply {
        let BenchRequest { command, payload } = request;
        let value = match self
            .backend
            .respond(&command, command_args(&command, payload))
        {
            Ok(value) => value_to_string(value),
            Err(message) => message,
        };
        BenchReply { value }
    }
}

/// The per-method adapter tonic's codegen would emit.
struct CallSvc(BenchHandler);

impl UnaryService<BenchRequest> for CallSvc {
    type Response = BenchReply;
    type Future =
        Pin<Box<dyn std::future::Future<Output = Result<Response<BenchReply>, Status>> + Send>>;

    fn call(&mut self, request: Request<BenchRequest>) -> Self::Future {
        let handler = self.0.clone();
        Box::pin(async move { Ok(Response::new(handler.call(request.into_inner()))) })
    }
}

/// The `tower` service tonic's codegen would emit — one route, everything else
/// `Unimplemented`.
#[derive(Clone)]
struct BenchServer {
    handler: BenchHandler,
}

impl NamedService for BenchServer {
    const NAME: &'static str = "bench.Bench";
}

impl<B> tower_service::Service<http::Request<B>> for BenchServer
where
    B: http_body::Body + Send + 'static,
    // `tonic::codegen::StdError` lives behind the `codegen` feature, which
    // this lane does not enable (it exists only for generated code); this is
    // the same type spelled out.
    B::Error: Into<Box<dyn std::error::Error + Send + Sync>> + Send + 'static,
{
    type Response = http::Response<Body>;
    type Error = std::convert::Infallible;
    type Future =
        Pin<Box<dyn std::future::Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, request: http::Request<B>) -> Self::Future {
        if request.uri().path() != METHOD_PATH {
            return Box::pin(async move {
                let mut response = http::Response::new(Body::default());
                let headers = response.headers_mut();
                headers.insert(
                    Status::GRPC_STATUS,
                    (tonic::Code::Unimplemented as i32).into(),
                );
                headers.insert(
                    http::header::CONTENT_TYPE,
                    tonic::metadata::GRPC_CONTENT_TYPE,
                );
                Ok(response)
            });
        }
        let handler = self.handler.clone();
        Box::pin(async move {
            let mut grpc = ServerGrpc::new(ProstCodec::<BenchReply, BenchRequest>::default());
            Ok(grpc.unary(CallSvc(handler), request).await)
        })
    }
}

/// Handle to the running gRPC listener — same shape as the other lanes.
#[derive(Debug)]
pub struct GrpcHandle {
    addr: SocketAddr,
    shutdown: watch::Sender<bool>,
    counters: Arc<ByteCounters>,
    done: Option<tokio::sync::mpsc::Receiver<()>>,
}

impl GrpcHandle {
    /// The bound address.
    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    /// Connections accepted so far. The driver asserts this grows by exactly
    /// one per intended connection — proof that no hidden pool opened more.
    pub fn connections(&self) -> u64 {
        self.counters.accepts.load(Ordering::Relaxed)
    }

    /// Bytes read off the wire so far.
    pub fn bytes_in(&self) -> u64 {
        self.counters.read.load(Ordering::Relaxed)
    }

    /// Bytes written to the wire so far.
    pub fn bytes_out(&self) -> u64 {
        self.counters.written.load(Ordering::Relaxed)
    }

    /// Graceful shutdown.
    pub async fn stop(mut self) {
        let _ = self.shutdown.send(true);
        if let Some(mut done) = self.done.take() {
            let _ = done.recv().await;
        }
    }
}

impl Drop for GrpcHandle {
    fn drop(&mut self) {
        let _ = self.shutdown.send(true);
    }
}

/// Spawn the tonic listener over the shared no-op backend.
pub async fn spawn_grpc_listener(
    backend: Arc<NoopBackend>,
    addr: SocketAddr,
) -> std::io::Result<GrpcHandle> {
    let listener = TcpListener::bind(addr).await?;
    let addr = listener.local_addr()?;
    let counters = Arc::new(ByteCounters::default());
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let (done_tx, done_rx) = tokio::sync::mpsc::channel::<()>(1);

    let service = BenchServer {
        handler: BenchHandler { backend },
    };

    // Own the accept path so every stream is wrapped in the byte counter.
    // NOTE: `Server::builder().tcp_nodelay(..)` is ignored by
    // `serve_with_incoming`, so nodelay is set here — without it Nagle would
    // distort every latency in this lane.
    let incoming = {
        let counters = Arc::clone(&counters);
        TcpListenerStream::new(listener).map(move |accepted| {
            accepted.map(|stream| {
                let _ = stream.set_nodelay(true);
                counters.accepts.fetch_add(1, Ordering::Relaxed);
                CountingStream {
                    inner: stream,
                    counters: Arc::clone(&counters),
                }
            })
        })
    };

    let mut stop = shutdown_rx;
    tokio::spawn(async move {
        let _done = done_tx;
        let _ = Server::builder()
            .max_concurrent_streams(Some(MAX_CONCURRENT_STREAMS))
            .initial_stream_window_size(Some(STREAM_WINDOW))
            .initial_connection_window_size(Some(CONNECTION_WINDOW))
            .serve_with_incoming_shutdown(service, incoming, async move {
                let _ = stop.wait_for(|halt| *halt).await;
            })
            .await;
    });

    Ok(GrpcHandle {
        addr,
        shutdown: shutdown_tx,
        counters,
        done: Some(done_rx),
    })
}

// ── Driver ──────────────────────────────────────────────────────────────────

/// One driver connection: a `tonic` `Channel`, which holds exactly one HTTP/2
/// connection. Clones of it multiplex over that same connection rather than
/// opening more — the property this lane depends on, asserted per cell.
async fn connect(addr: SocketAddr) -> Result<Channel, String> {
    Channel::from_shared(format!("http://{addr}"))
        .map_err(|e| format!("grpc bad endpoint: {e}"))?
        .tcp_nodelay(true)
        .initial_stream_window_size(Some(STREAM_WINDOW))
        .initial_connection_window_size(Some(CONNECTION_WINDOW))
        .buffer_size(Some(CHANNEL_BUFFER))
        .connect()
        .await
        .map_err(|e| format!("grpc connect failed: {e}"))
}

/// Issue one unary call and consume its reply — the measurement point
/// (BEN-003).
async fn call(channel: Channel, request: BenchRequest) -> Result<(), String> {
    let mut client = tonic::client::Grpc::new(channel);
    client
        .ready()
        .await
        .map_err(|e| format!("grpc channel not ready: {e}"))?;
    let codec = ProstCodec::<BenchRequest, BenchReply>::default();
    let path = PathAndQuery::from_static(METHOD_PATH);
    client
        .unary(Request::new(request), path, codec)
        .await
        .map(|_reply| ())
        .map_err(|e| format!("grpc call failed: {e}"))
}

/// Build the request the matrix asked for.
fn build_request(command: &str, args: &[Value]) -> Result<BenchRequest, String> {
    let payload = match args.first() {
        Some(Value::Str(s)) => s.clone(),
        Some(Value::Bytes(b)) => String::from_utf8(b.to_vec())
            .map_err(|_| "grpc lane: protobuf string payloads must be UTF-8".to_owned())?,
        Some(other) => return Err(format!("grpc lane: unsupported arg {other:?}")),
        None => String::new(),
    };
    Ok(BenchRequest {
        command: command.to_owned(),
        payload,
    })
}

/// One continuously-full window of `depth` in-flight requests on one channel.
///
/// `buffer_unordered` is the multiplexed spelling of the FIFO drivers'
/// send-ahead window: it keeps `depth` futures resolving at once and starts a
/// replacement the moment one finishes.
async fn grpc_window(
    channel: &Channel,
    depth: usize,
    ops: usize,
    request: &BenchRequest,
) -> Result<(Vec<Duration>, Duration), String> {
    let started = Instant::now();
    let latencies: Vec<Result<Duration, String>> = stream::iter(0..ops)
        .map(|_| {
            let channel = channel.clone();
            let request = request.clone();
            async move {
                let sent = Instant::now();
                call(channel, request).await?;
                Ok(sent.elapsed())
            }
        })
        .buffer_unordered(depth.max(1))
        .collect()
        .await;
    let elapsed = started.elapsed();
    latencies
        .into_iter()
        .collect::<Result<Vec<_>, String>>()
        .map(|lats| (lats, elapsed))
}

/// Measure one matrix cell on the gRPC lane.
pub async fn cell(
    handle: &GrpcHandle,
    spec: &CellSpec,
    cfg: &RunConfig,
) -> Result<Measured, String> {
    let addr = handle.local_addr();
    let request = build_request(spec.command, &spec.args)?;
    let before_connections = handle.connections();
    let mut channels = Vec::with_capacity(spec.connections);
    for _ in 0..spec.connections {
        channels.push(connect(addr).await?);
    }

    if cfg.warmup > 0 {
        for channel in &channels {
            grpc_window(channel, spec.depth, cfg.warmup, &request).await?;
        }
    }

    let before_in = handle.bytes_in();
    let before_out = handle.bytes_out();
    let mut reps = Vec::with_capacity(cfg.repetitions);
    let mut ops = 0u64;
    for _ in 0..cfg.repetitions {
        let per_conn = (cfg.ops / channels.len().max(1)).max(spec.depth).max(1);
        let started = Instant::now();
        let mut handles = Vec::with_capacity(channels.len());
        for channel in &channels {
            let channel = channel.clone();
            let request = request.clone();
            let depth = spec.depth;
            handles.push(tokio::spawn(async move {
                grpc_window(&channel, depth, per_conn, &request).await
            }));
        }
        let mut all = Vec::with_capacity(per_conn * channels.len());
        for handle in handles {
            let (lats, _elapsed) = handle
                .await
                .map_err(|e| format!("grpc worker panicked: {e}"))??;
            all.extend(lats);
        }
        let elapsed = started.elapsed();
        ops += all.len() as u64;
        reps.push(compute(&mut all, elapsed));
    }
    let after_in = handle.bytes_in();
    let after_out = handle.bytes_out();

    // The property this lane rests on: one TCP connection per driver
    // connection. A hidden pool would make every per-connection number a lie.
    let opened = handle.connections() - before_connections;
    if opened != spec.connections as u64 {
        return Err(format!(
            "grpc lane: expected {} connection(s), the listener accepted {opened}",
            spec.connections
        ));
    }
    drop(channels);

    let ops = ops.max(1) as f64;
    Ok((
        reps,
        (after_in - before_in) as f64 / ops,
        (after_out - before_out) as f64 / ops,
    ))
}

/// The connection-storm cell: connect (HTTP/2 handshake included) + one call,
/// repeated.
pub async fn storm(
    handle: &GrpcHandle,
    storms: usize,
    cfg: &RunConfig,
) -> Result<Measured, String> {
    let addr = handle.local_addr();
    let request = build_request("PING", &[])?;
    for _ in 0..cfg.warmup.min(storms) {
        storm_once(addr, &request).await?;
    }
    let before_in = handle.bytes_in();
    let before_out = handle.bytes_out();
    let mut reps = Vec::with_capacity(cfg.repetitions);
    let mut ops = 0u64;
    for _ in 0..cfg.repetitions {
        let mut lats = Vec::with_capacity(storms);
        let started = Instant::now();
        for _ in 0..storms {
            lats.push(storm_once(addr, &request).await?);
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

async fn storm_once(addr: SocketAddr, request: &BenchRequest) -> Result<Duration, String> {
    let started = Instant::now();
    let channel = connect(addr).await?;
    call(channel, request.clone()).await?;
    Ok(started.elapsed())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::backend::STATIC_REPLY_BYTES;
    use prost::Message;

    #[test]
    fn messages_round_trip_through_protobuf() {
        let request = BenchRequest {
            command: "ECHO".to_owned(),
            payload: "hello world".to_owned(),
        };
        let encoded = request.encode_to_vec();
        let decoded = BenchRequest::decode(encoded.as_slice()).unwrap();
        assert_eq!(decoded, request);
    }

    #[test]
    fn an_empty_payload_encodes_to_nothing() {
        // protobuf omits default-valued fields: a bare PING carries only its
        // command, which is what makes the sync-tiny cells tiny.
        let request = BenchRequest {
            command: "PING".to_owned(),
            payload: String::new(),
        };
        let encoded = request.encode_to_vec();
        assert_eq!(encoded.len(), 2 + "PING".len(), "tag + len + 'PING'");
    }

    #[test]
    fn the_handler_echoes_through_the_shared_backend() {
        let handler = BenchHandler {
            backend: Arc::new(NoopBackend::new()),
        };
        let reply = handler.call(BenchRequest {
            command: "ECHO".to_owned(),
            payload: "x".repeat(64),
        });
        assert_eq!(reply.value, "x".repeat(64));
    }

    #[test]
    fn the_handler_serves_the_4kib_static_reply() {
        let handler = BenchHandler {
            backend: Arc::new(NoopBackend::new()),
        };
        let reply = handler.call(BenchRequest {
            command: "STATIC".to_owned(),
            payload: String::new(),
        });
        assert_eq!(reply.value.len(), STATIC_REPLY_BYTES);
    }

    #[test]
    fn unknown_commands_surface_the_backend_error() {
        let handler = BenchHandler {
            backend: Arc::new(NoopBackend::new()),
        };
        let reply = handler.call(BenchRequest {
            command: "NOPE".to_owned(),
            payload: String::new(),
        });
        assert!(reply.value.contains("unknown command"), "{}", reply.value);
    }

    #[test]
    fn non_utf8_payloads_are_refused_not_mangled() {
        let err = build_request("ECHO", &[Value::bytes(vec![0xff, 0xfe])]).unwrap_err();
        assert!(err.contains("UTF-8"), "unexpected error: {err}");
    }
}

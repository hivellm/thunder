//! **Cap'n Proto RPC** lane — the real `capnp-rpc` on both sides, serving the
//! same no-op backend (BEN-001, BEN-002).
//!
//! # Why this lane
//!
//! Cap'n Proto's claim is **no parse step**: field accessors are pointer
//! arithmetic into the received buffer, so there is no decode pass building a
//! Rust value the way MessagePack, BSON, protobuf and TCompactProtocol all
//! do. Every other lane in the shootout pays that pass. This is the one that
//! does not, which makes it the natural upper bound on "what if decoding were
//! free?" It is also multiplexed (question ids, answers may return out of
//! order), like Thunder and gRPC.
//!
//! # ⚠ This lane is NOT throughput-comparable with the others. Read this.
//!
//! `capnp_rpc::RpcSystem` and `twoparty::VatNetwork` are **`!Send`** — they are
//! built on `Rc` capability tables. They cannot be `tokio::spawn`ed; they
//! require a `LocalSet`, and a `LocalSet` cannot be pinned onto a worker of an
//! already-running multi-thread runtime. The only way to run this lane is a
//! dedicated OS thread with its own current-thread runtime.
//!
//! That is a **second runtime instance in the process**, which is the same
//! class of problem that disqualified the `rmp-rpc` crate from the
//! MessagePack-RPC lane. The two are not equally bad, and the distinction is
//! the whole justification for building this lane anyway:
//!
//! - `rmp-rpc` would have pulled **tokio 0.1.22** — a different major version,
//!   a different and incompatible reactor, unmaintained since 2019. Two
//!   *reactors of different vintages*.
//! - `capnp-rpc` uses the **same tokio 1.x, same allocator, same process** —
//!   a second *instance* of the identical scheduler, running current-thread.
//!
//! So the honest reading of this lane's numbers:
//!
//! - **Per-request latency at depth=1, connections=1 is comparable.** Nothing
//!   is being parallelized in that cell on any lane, so single-threaded
//!   execution costs it nothing.
//! - **Aggregate throughput at higher depth/connections is NOT comparable.**
//!   Every other lane spreads across N worker threads; this one has one
//!   thread for server *and* client. A deficit there is this lane's
//!   deployment shape, not Cap'n Proto's protocol.
//!
//! Sharding connections across per-thread runtimes would restore multi-core
//! throughput, and is how you would really deploy capnp-rpc — but it would
//! multiply the runtime-instance problem rather than fix it, so it is not
//! done here. The limitation is reported instead of engineered around.
//!
//! # Schema and generated code
//!
//! [`crate::bench_capnp`] is checked in, `@generated`, exactly as `capnp-rpc`
//! does with its own `rpc_twoparty_capnp.rs`. It was produced once by
//! `capnpc-embedded` (the schema compiler as WebAssembly), so **building this
//! harness needs no `capnp` binary and no C++ toolchain** — and no `build.rs`
//! either. Hand-writing it was never an option: generated Cap'n Proto code
//! embeds the compiled schema as raw words, which the `Introspect`/`Owned`
//! machinery requires.
//!
//! # Scope (honesty note, BEN-002)
//!
//! A **benchmark peer, not a service**. One interface, one method:
//! `call(command :Text, payload :Text) -> (value :Text)`, where `command`
//! selects the backend mode (`ECHO`/`STATIC`/`SINK`/`PING`). No promise
//! pipelining is exercised (the matrix has no call chains to pipeline), no
//! capability passing, no levels beyond level 1.

use std::cell::RefCell;
use std::net::SocketAddr;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use capnp::capability::Promise;
use capnp_rpc::{rpc_twoparty_capnp, twoparty, RpcSystem};
use futures::AsyncReadExt;
use thunder::wire::Value;
use tokio::net::{TcpListener, TcpStream};
use tokio_util::compat::TokioAsyncReadCompatExt;

use crate::backend::NoopBackend;
use crate::bench_capnp::bench;
use crate::driver::{CellSpec, Measured, RunConfig};
use crate::stats::compute;

/// Shared byte counters — the lane runs on its own thread, so these cross the
/// boundary as an `Arc` rather than living in the handle directly.
#[derive(Debug, Default)]
struct CapnpCounters {
    bytes_in: AtomicU64,
    bytes_out: AtomicU64,
}

// ── Server ──────────────────────────────────────────────────────────────────

/// Serves the shared no-op backend as the one `call` method.
struct BenchImpl {
    backend: Arc<NoopBackend>,
}

impl bench::Server for BenchImpl {
    fn call(
        self: capnp::capability::Rc<Self>,
        params: bench::CallParams,
        mut results: bench::CallResults,
    ) -> impl core::future::Future<Output = Result<(), capnp::Error>> + 'static {
        // Read the request out of the received buffer — pointer arithmetic,
        // no decode pass. That is the property this lane exists to measure.
        let outcome = (|| -> Result<String, capnp::Error> {
            let reader = params.get()?;
            let command = reader.get_command()?.to_str()?;
            let payload = reader.get_payload()?.to_str()?;
            let args = command_args(command, payload);
            Ok(match self.backend.respond(command, args) {
                Ok(value) => value_to_string(value),
                Err(message) => message,
            })
        })();
        core::future::ready(outcome.map(|value| {
            results.get().set_value(&value[..]);
        }))
    }
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
fn command_args(command: &str, payload: &str) -> Vec<Value> {
    match command {
        "ECHO" if !payload.is_empty() => vec![Value::Str(payload.to_owned())],
        _ => vec![],
    }
}

/// Handle to the running Cap'n Proto listener.
///
/// The listener lives on a dedicated OS thread with its own current-thread
/// runtime — see the module docs for why, and for what that costs the
/// comparison.
#[derive(Debug)]
pub struct CapnpHandle {
    addr: SocketAddr,
    counters: Arc<CapnpCounters>,
    shutdown: Arc<AtomicU64>,
}

impl CapnpHandle {
    /// The bound address.
    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    /// Bytes read off the wire so far.
    pub fn bytes_in(&self) -> u64 {
        self.counters.bytes_in.load(Ordering::Relaxed)
    }

    /// Bytes written to the wire so far.
    pub fn bytes_out(&self) -> u64 {
        self.counters.bytes_out.load(Ordering::Relaxed)
    }

    /// Signal the listener thread to stop accepting.
    pub async fn stop(self) {
        self.shutdown.store(1, Ordering::Relaxed);
    }
}

impl Drop for CapnpHandle {
    fn drop(&mut self) {
        self.shutdown.store(1, Ordering::Relaxed);
    }
}

/// Spawn the Cap'n Proto listener on its own thread.
///
/// The socket is bound here, on the caller's runtime, so the ephemeral port is
/// known before the thread starts; only the `RpcSystem` driving needs the
/// current-thread runtime.
pub async fn spawn_capnp_listener(
    backend: Arc<NoopBackend>,
    addr: SocketAddr,
) -> std::io::Result<CapnpHandle> {
    let listener = std::net::TcpListener::bind(addr)?;
    listener.set_nonblocking(true)?;
    let addr = listener.local_addr()?;
    let counters = Arc::new(CapnpCounters::default());
    let shutdown = Arc::new(AtomicU64::new(0));

    let thread_counters = Arc::clone(&counters);
    let thread_shutdown = Arc::clone(&shutdown);
    std::thread::Builder::new()
        .name("capnp-lane".to_owned())
        .spawn(move || {
            let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            else {
                return;
            };
            let local = tokio::task::LocalSet::new();
            local.block_on(&runtime, async move {
                let Ok(listener) = TcpListener::from_std(listener) else {
                    return;
                };
                loop {
                    if thread_shutdown.load(Ordering::Relaxed) != 0 {
                        break;
                    }
                    let accepted = tokio::select! {
                        accepted = listener.accept() => accepted,
                        _ = tokio::time::sleep(Duration::from_millis(50)) => continue,
                    };
                    let Ok((stream, _)) = accepted else { break };
                    let _ = stream.set_nodelay(true);
                    let backend = Arc::clone(&backend);
                    let counters = Arc::clone(&thread_counters);
                    tokio::task::spawn_local(async move {
                        let client: bench::Client = capnp_rpc::new_client(BenchImpl { backend });
                        let stream = CountingStream::new(stream, counters);
                        let (reader, writer) = stream.compat().split();
                        let network = twoparty::VatNetwork::new(
                            futures::io::BufReader::new(reader),
                            futures::io::BufWriter::new(writer),
                            rpc_twoparty_capnp::Side::Server,
                            Default::default(),
                        );
                        let rpc = RpcSystem::new(Box::new(network), Some(client.client));
                        let _ = rpc.await;
                    });
                }
            });
        })?;

    Ok(CapnpHandle {
        addr,
        counters,
        shutdown,
    })
}

// ── Byte-counting stream ────────────────────────────────────────────────────

/// A `TcpStream` that counts bytes, so this lane reports the same
/// bytes-at-the-socket measure as every other lane.
struct CountingStream {
    inner: TcpStream,
    counters: Arc<CapnpCounters>,
}

impl CountingStream {
    fn new(inner: TcpStream, counters: Arc<CapnpCounters>) -> Self {
        Self { inner, counters }
    }
}

impl tokio::io::AsyncRead for CountingStream {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let before = buf.filled().len();
        let polled = std::pin::Pin::new(&mut self.inner).poll_read(cx, buf);
        if let std::task::Poll::Ready(Ok(())) = &polled {
            let read = (buf.filled().len() - before) as u64;
            self.counters.bytes_in.fetch_add(read, Ordering::Relaxed);
        }
        polled
    }
}

impl tokio::io::AsyncWrite for CountingStream {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        let polled = std::pin::Pin::new(&mut self.inner).poll_write(cx, buf);
        if let std::task::Poll::Ready(Ok(written)) = &polled {
            self.counters
                .bytes_out
                .fetch_add(*written as u64, Ordering::Relaxed);
        }
        polled
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

// ── Driver ──────────────────────────────────────────────────────────────────

/// Everything one cell needs on the far side of the thread boundary.
struct CellJob {
    addr: SocketAddr,
    command: String,
    payload: String,
    depth: usize,
    connections: usize,
    ops: usize,
    warmup: usize,
    repetitions: usize,
}

/// Run a whole cell on a dedicated thread — the client capabilities are
/// `!Send` too, so the driver cannot live on the harness runtime either.
fn run_cell_on_thread(job: CellJob) -> Result<Vec<(Vec<Duration>, Duration)>, String> {
    let CellJob {
        addr,
        command,
        payload,
        depth,
        connections,
        ops,
        warmup,
        repetitions,
    } = job;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("capnp-driver".to_owned())
        .spawn(move || {
            let result = (|| -> Result<Vec<(Vec<Duration>, Duration)>, String> {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|e| format!("capnp driver runtime failed: {e}"))?;
                let local = tokio::task::LocalSet::new();
                local.block_on(&runtime, async move {
                    let mut clients = Vec::with_capacity(connections);
                    for _ in 0..connections {
                        clients.push(connect(addr).await?);
                    }
                    if warmup > 0 {
                        for client in &clients {
                            window(client, depth, warmup, &command, &payload).await?;
                        }
                    }
                    let mut reps = Vec::with_capacity(repetitions);
                    for _ in 0..repetitions {
                        let per_conn = (ops / clients.len().max(1)).max(depth).max(1);
                        let started = Instant::now();
                        let mut all = Vec::with_capacity(per_conn * clients.len());
                        // One LocalSet, so connections interleave cooperatively
                        // rather than in parallel — the caveat in the module docs.
                        let mut pending = Vec::with_capacity(clients.len());
                        for client in &clients {
                            pending.push(window(client, depth, per_conn, &command, &payload));
                        }
                        for result in futures::future::join_all(pending).await {
                            all.extend(result?);
                        }
                        reps.push((all, started.elapsed()));
                    }
                    Ok(reps)
                })
            })();
            let _ = tx.send(result);
        })
        .map_err(|e| format!("capnp driver thread failed: {e}"))?;
    rx.recv()
        .map_err(|e| format!("capnp driver vanished: {e}"))?
}

/// Dial and take the bootstrap capability.
async fn connect(addr: SocketAddr) -> Result<bench::Client, String> {
    let stream = TcpStream::connect(addr)
        .await
        .map_err(|e| format!("capnp connect failed: {e}"))?;
    stream
        .set_nodelay(true)
        .map_err(|e| format!("capnp nodelay failed: {e}"))?;
    let (reader, writer) = stream.compat().split();
    let network = Box::new(twoparty::VatNetwork::new(
        futures::io::BufReader::new(reader),
        futures::io::BufWriter::new(writer),
        rpc_twoparty_capnp::Side::Client,
        Default::default(),
    ));
    let mut rpc = RpcSystem::new(network, None);
    let client: bench::Client = rpc.bootstrap(rpc_twoparty_capnp::Side::Server);
    tokio::task::spawn_local(async move {
        let _ = rpc.await;
    });
    Ok(client)
}

/// One continuously-full window of `depth` in-flight calls on one connection.
///
/// Cap'n Proto RPC is multiplexed — many outstanding questions on one
/// connection — so, like the gRPC lane, the window is expressed as concurrent
/// calls rather than as writes-before-reads.
async fn window(
    client: &bench::Client,
    depth: usize,
    ops: usize,
    command: &str,
    payload: &str,
) -> Result<Vec<Duration>, String> {
    let latencies = Rc::new(RefCell::new(Vec::with_capacity(ops)));
    let mut issued = 0usize;
    let mut inflight = futures::stream::FuturesUnordered::new();

    let spawn_one = |client: &bench::Client| {
        let mut request = client.call_request();
        {
            let mut builder = request.get();
            builder.set_command(command);
            builder.set_payload(payload);
        }
        let started = Instant::now();
        let latencies = Rc::clone(&latencies);
        async move {
            let response = request.send().promise.await.map_err(|e| e.to_string())?;
            // Touch the reply so the read path is genuinely exercised.
            let _ = response
                .get()
                .map_err(|e| e.to_string())?
                .get_value()
                .map_err(|e| e.to_string())?;
            latencies.borrow_mut().push(started.elapsed());
            Ok::<(), String>(())
        }
    };

    while issued < ops && inflight.len() < depth.max(1) {
        inflight.push(spawn_one(client));
        issued += 1;
    }
    while let Some(done) = futures::StreamExt::next(&mut inflight).await {
        done?;
        if issued < ops {
            inflight.push(spawn_one(client));
            issued += 1;
        }
    }
    let taken = latencies.borrow().clone();
    Ok(taken)
}

/// Measure one matrix cell on the Cap'n Proto lane.
pub async fn cell(
    handle: &CapnpHandle,
    spec: &CellSpec,
    cfg: &RunConfig,
) -> Result<Measured, String> {
    let payload = match spec.args.first() {
        Some(Value::Str(s)) => s.clone(),
        Some(Value::Bytes(b)) => String::from_utf8(b.to_vec())
            .map_err(|_| "capnp lane: Text payloads must be UTF-8".to_owned())?,
        Some(other) => return Err(format!("capnp lane: unsupported arg {other:?}")),
        None => String::new(),
    };
    let addr = handle.local_addr();
    let command = spec.command.to_owned();
    let (depth, connections) = (spec.depth, spec.connections);
    let (ops, warmup, repetitions) = (cfg.ops, cfg.warmup, cfg.repetitions);

    let before_in = handle.bytes_in();
    let before_out = handle.bytes_out();
    let windows = tokio::task::spawn_blocking(move || {
        run_cell_on_thread(CellJob {
            addr,
            command,
            payload,
            depth,
            connections,
            ops,
            warmup,
            repetitions,
        })
    })
    .await
    .map_err(|e| format!("capnp cell join failed: {e}"))??;
    let after_in = handle.bytes_in();
    let after_out = handle.bytes_out();

    let mut total_ops = 0u64;
    let mut reps = Vec::with_capacity(windows.len());
    for (mut latencies, elapsed) in windows {
        total_ops += latencies.len() as u64;
        reps.push(compute(&mut latencies, elapsed));
    }
    let total_ops = total_ops.max(1) as f64;
    Ok((
        reps,
        (after_in - before_in) as f64 / total_ops,
        (after_out - before_out) as f64 / total_ops,
    ))
}

/// The connection-storm cell: connect (bootstrap included) + one call,
/// repeated.
pub async fn storm(
    handle: &CapnpHandle,
    storms: usize,
    cfg: &RunConfig,
) -> Result<Measured, String> {
    let addr = handle.local_addr();
    let repetitions = cfg.repetitions;
    let warmup = cfg.warmup.min(storms);

    let before_in = handle.bytes_in();
    let before_out = handle.bytes_out();
    let windows = tokio::task::spawn_blocking(move || {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("capnp-storm".to_owned())
            .spawn(move || {
                let result = (|| -> Result<Vec<(Vec<Duration>, Duration)>, String> {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|e| format!("capnp storm runtime failed: {e}"))?;
                    let local = tokio::task::LocalSet::new();
                    local.block_on(&runtime, async move {
                        for _ in 0..warmup {
                            storm_once(addr).await?;
                        }
                        let mut reps = Vec::with_capacity(repetitions);
                        for _ in 0..repetitions {
                            let mut lats = Vec::with_capacity(storms);
                            let started = Instant::now();
                            for _ in 0..storms {
                                lats.push(storm_once(addr).await?);
                            }
                            reps.push((lats, started.elapsed()));
                        }
                        Ok(reps)
                    })
                })();
                let _ = tx.send(result);
            })
            .map_err(|e| format!("capnp storm thread failed: {e}"))?;
        rx.recv()
            .map_err(|e| format!("capnp storm vanished: {e}"))?
    })
    .await
    .map_err(|e| format!("capnp storm join failed: {e}"))??;
    let after_in = handle.bytes_in();
    let after_out = handle.bytes_out();

    let mut total_ops = 0u64;
    let mut reps = Vec::with_capacity(windows.len());
    for (mut latencies, elapsed) in windows {
        total_ops += latencies.len() as u64;
        reps.push(compute(&mut latencies, elapsed));
    }
    let total_ops = total_ops.max(1) as f64;
    Ok((
        reps,
        (after_in - before_in) as f64 / total_ops,
        (after_out - before_out) as f64 / total_ops,
    ))
}

async fn storm_once(addr: SocketAddr) -> Result<Duration, String> {
    let started = Instant::now();
    let client = connect(addr).await?;
    let mut request = client.call_request();
    {
        let mut builder = request.get();
        builder.set_command("PING");
        builder.set_payload("");
    }
    let response = request.send().promise.await.map_err(|e| e.to_string())?;
    let _ = response
        .get()
        .map_err(|e| e.to_string())?
        .get_value()
        .map_err(|e| e.to_string())?;
    Ok(started.elapsed())
}

/// Silence the unused-import warning for `Promise`, which the generated code
/// references in its trait default bodies.
#[allow(dead_code)]
fn _promise_is_used(_: Option<Promise<(), capnp::Error>>) {}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::backend::STATIC_REPLY_BYTES;

    #[test]
    fn echo_routes_the_payload_through_the_shared_backend() {
        let backend = NoopBackend::new();
        let value = backend
            .respond("ECHO", command_args("ECHO", &"x".repeat(64)))
            .unwrap();
        assert_eq!(value_to_string(value), "x".repeat(64));
    }

    #[test]
    fn static_serves_the_4kib_reply() {
        let backend = NoopBackend::new();
        let value = backend
            .respond("STATIC", command_args("STATIC", ""))
            .unwrap();
        assert_eq!(value_to_string(value).len(), STATIC_REPLY_BYTES);
    }

    #[test]
    fn the_sentinels_carry_no_args() {
        assert!(command_args("PING", "").is_empty());
        assert!(command_args("STATIC", "ignored").is_empty());
        assert_eq!(command_args("ECHO", "hi").len(), 1);
    }

    /// The message round-trips through the real capnp builder/reader — the
    /// zero-copy read path this lane exists to measure.
    #[test]
    fn a_message_round_trips_through_the_capnp_reader() {
        let mut message = capnp::message::Builder::new_default();
        {
            let mut builder = message.init_root::<bench::call_params::Builder>();
            builder.set_command("ECHO");
            builder.set_payload("hello");
        }
        let reader = message
            .get_root_as_reader::<bench::call_params::Reader>()
            .unwrap();
        assert_eq!(reader.get_command().unwrap().to_str().unwrap(), "ECHO");
        assert_eq!(reader.get_payload().unwrap().to_str().unwrap(), "hello");
    }
}

//! Parity driver harness (BEN-003, BEN-011).
//!
//! One driver shape per lane, with **provable parity**:
//!
//! - continuous pipelining, no inter-batch gaps (the Synap `-P 16`
//!   lesson): the Thunder lane keeps `depth` concurrent callers per
//!   connection multiplexed over [`thunder::client::Client`]; the HTTP lane
//!   keeps a FIFO pipeline window of `depth` requests on each keep-alive
//!   connection — in both lanes the next request is issued the moment a
//!   slot frees, never after a batch barrier;
//! - identical measurement points: latency is client-observed, from
//!   request submission to the response fully decoded (MessagePack decode
//!   on the Thunder lane, JSON parse + `ok` check on the HTTP lane). One
//!   caveat is recorded in every artifact's honesty notes: at
//!   depth = burst the Thunder stamp additionally includes in-client
//!   write-lock queueing, so qps is the comparable number for that row
//!   until the T4.2 harness unifies the stamp point;
//! - identical byte accounting: bytes-on-wire per op come from
//!   server-side counters recorded after successful socket writes on both
//!   lanes (SRV-030 and [`crate::http::HttpMetrics`]), never re-encoded;
//! - run discipline (BEN-011): warmup ops discarded before any
//!   measurement, then N repetitions, each reported with its own
//!   p50/p99/qps plus min/mean/max dispersion across repetitions.

use std::collections::VecDeque;
use std::io;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex as StdMutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use thunder::client::Client;
use thunder::server::{spawn_listener, ListenerConfig, ListenerHandle, ServerInfo};
use thunder::wire::profile::{
    ErrorConvention, Handshake, HelloStyle, Profile, PushPolicy, TlsPolicy,
};
use thunder::wire::Value;
use tokio::io::{AsyncWriteExt, BufReader};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::TcpStream;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::backend::NoopBackend;
use crate::bolt::{spawn_bolt_listener, BoltHandle};
use crate::http::{read_http_response, spawn_http_listener, wire_to_json, HttpHandle};
use crate::resp3::{spawn_resp3_listener, Resp3Handle};
use crate::scenarios::{Scenario, Workload};
use crate::stats::{compute, dispersion, CellStats, Dispersion};

/// The profile both bench peers run: no handshake (nothing but transport
/// in the measurement), push reserved, in-flight bound sized above the
/// deepest matrix window (pipelined-1k).
pub const fn bench_profile() -> Profile {
    Profile {
        name: "thunder-bench",
        scheme: "thunder-bench",
        default_port: 0,
        handshake: Handshake::None,
        hello_style: HelloStyle::NotUsed,
        push: PushPolicy::Reserved,
        max_frame_bytes: thunder::wire::DEFAULT_MAX_FRAME_BYTES,
        max_in_flight: 4096,
        error_codes: ErrorConvention::None,
        tls: TlsPolicy::Off,
    }
}

/// The four shootout lanes (BEN-001) — all served by the one no-op backend
/// in the same process, host, runtime and allocator, so the transport is the
/// only thing measured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lane {
    /// Thunder RPC via `thunder::client` against `thunder::server`.
    Thunder,
    /// RESP3 (the Redis/Synap convention) against [`crate::resp3`].
    Resp3,
    /// Bolt v5 (the Neo4j/Nexus competitor) against [`crate::bolt`].
    Bolt,
    /// Raw HTTP/1.1 + JSON against the hand-rolled listener.
    Http,
}

impl Lane {
    /// Stable lane key used in artifacts.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Thunder => "thunder",
            Self::Resp3 => "resp3",
            Self::Bolt => "bolt",
            Self::Http => "http",
        }
    }

    /// Every lane, artifact order — Thunder first, then the peers it must
    /// beat in every cell (BEN-020 / gate G5).
    pub const ALL: [Lane; 4] = [Lane::Thunder, Lane::Resp3, Lane::Bolt, Lane::Http];
}

/// Run discipline knobs (BEN-011).
#[derive(Debug, Clone, Copy)]
pub struct RunConfig {
    /// Measured operations per repetition per cell. Also clamps the
    /// connection-storm size (`min(storm, ops)`).
    pub ops: usize,
    /// Warmup operations per cell, discarded before measurement.
    pub warmup: usize,
    /// Repetitions per cell — dispersion is reported across these.
    pub repetitions: usize,
}

impl Default for RunConfig {
    fn default() -> Self {
        Self {
            ops: 2000,
            warmup: 200,
            repetitions: 3,
        }
    }
}

/// One measured (or pending) cell of the matrix.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CellResult {
    /// Scenario name (stable artifact key).
    pub scenario: String,
    /// Lane key (`thunder` / `http`), or `-` for pending rows.
    pub lane: String,
    /// Pipeline depth (in-flight window per connection).
    pub depth: usize,
    /// Concurrent connections.
    pub connections: usize,
    /// `"ok"`, or `"pending — lands at T4.x"` for declared stubs.
    pub status: String,
    /// Per-repetition stats (BEN-011).
    pub reps: Vec<CellStats>,
    /// p50 dispersion across repetitions, microseconds.
    pub p50_us: Option<Dispersion>,
    /// p99 dispersion across repetitions, microseconds.
    pub p99_us: Option<Dispersion>,
    /// Throughput dispersion across repetitions.
    pub qps: Option<Dispersion>,
    /// Mean request bytes on the wire per op (server-side counter).
    pub bytes_in_per_op: Option<f64>,
    /// Mean response bytes on the wire per op (server-side counter).
    pub bytes_out_per_op: Option<f64>,
}

impl CellResult {
    /// One human line for progress output.
    pub fn one_line(&self) -> String {
        match (&self.p50_us, &self.p99_us, &self.qps) {
            (Some(p50), Some(p99), Some(qps)) => format!(
                "{:<16} {:<7} depth={:<5} conns={:<2} p50={:.1}us p99={:.1}us qps={:.0} B/op in={:.0} out={:.0}",
                self.scenario,
                self.lane,
                self.depth,
                self.connections,
                p50.mean,
                p99.mean,
                qps.mean,
                self.bytes_in_per_op.unwrap_or(0.0),
                self.bytes_out_per_op.unwrap_or(0.0),
            ),
            _ => format!("{:<16} {}", self.scenario, self.status),
        }
    }
}

/// The in-process shootout targets: every listener over the one shared
/// no-op backend (BEN-001 — same process, host, runtime, allocator).
#[derive(Debug)]
pub struct Targets {
    /// The Thunder RPC listener handle.
    pub thunder: ListenerHandle,
    /// The RESP3 listener handle.
    pub resp3: Resp3Handle,
    /// The Bolt v5 listener handle.
    pub bolt: BoltHandle,
    /// The HTTP/1.1 listener handle.
    pub http: HttpHandle,
}

impl Targets {
    /// Graceful shutdown of every listener.
    pub async fn stop(self) {
        self.thunder.stop().await;
        self.resp3.stop().await;
        self.bolt.stop().await;
        self.http.stop().await;
    }
}

/// Spawn every listener on loopback ephemeral ports, sharing one
/// [`NoopBackend`] (BEN-001).
pub async fn spawn_targets() -> io::Result<Targets> {
    let backend = Arc::new(NoopBackend::new());
    let loopback = SocketAddr::from(([127, 0, 0, 1], 0));
    let thunder = spawn_listener(
        Arc::clone(&backend),
        bench_profile(),
        ServerInfo {
            name: "thunder-bench".to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
        },
        ListenerConfig::default(),
    )
    .await?;
    let resp3 = spawn_resp3_listener(Arc::clone(&backend), loopback).await?;
    let bolt = spawn_bolt_listener(Arc::clone(&backend), loopback).await?;
    let http = spawn_http_listener(backend, loopback).await?;
    Ok(Targets {
        thunder,
        resp3,
        bolt,
        http,
    })
}

/// Run one scenario on one lane across its matrix cells.
///
/// Pending scenarios return their single declaration row (lane `-`) —
/// callers iterating lanes should emit it once via
/// [`Scenario::is_pending`].
pub async fn run_scenario(
    targets: &Targets,
    scenario: &Scenario,
    lane: Lane,
    cfg: &RunConfig,
) -> Result<Vec<CellResult>, String> {
    match scenario.workload {
        Workload::Pending { .. } => Ok(vec![pending_cell(scenario)]),
        Workload::ConnectionStorm { connections } => {
            let storms = connections.min(cfg.ops.max(1));
            let measured = match lane {
                Lane::Thunder => thunder_storm(targets, storms, cfg).await?,
                Lane::Resp3 => crate::resp3::storm(&targets.resp3, storms, cfg).await?,
                Lane::Bolt => crate::bolt::storm(&targets.bolt, storms, cfg).await?,
                Lane::Http => http_storm(targets, storms, cfg).await?,
            };
            Ok(vec![finish_cell(scenario, lane, 1, storms, measured)])
        }
        _ => {
            let (command, args) = request_for(scenario.workload)
                .ok_or_else(|| format!("scenario '{}' has no request shape", scenario.name))?;
            let mut cells = Vec::new();
            for (depth, connections) in scenario.cells() {
                let spec = CellSpec {
                    command,
                    args: args.clone(),
                    depth,
                    connections,
                };
                let measured = match lane {
                    Lane::Thunder => thunder_cell(targets, &spec, cfg).await?,
                    Lane::Resp3 => crate::resp3::cell(&targets.resp3, &spec, cfg).await?,
                    Lane::Bolt => crate::bolt::cell(&targets.bolt, &spec, cfg).await?,
                    Lane::Http => http_cell(targets, &spec, cfg).await?,
                };
                cells.push(finish_cell(scenario, lane, depth, connections, measured));
            }
            Ok(cells)
        }
    }
}

/// The declaration row for a scenario the skeleton cannot measure yet.
pub fn pending_cell(scenario: &Scenario) -> CellResult {
    let lands_at = match scenario.workload {
        Workload::Pending { lands_at } => lands_at,
        _ => "T4.x",
    };
    CellResult {
        scenario: scenario.name.to_owned(),
        lane: "-".to_owned(),
        depth: 0,
        connections: 0,
        status: format!("pending — lands at {lands_at}"),
        reps: Vec::new(),
        p50_us: None,
        p99_us: None,
        qps: None,
        bytes_in_per_op: None,
        bytes_out_per_op: None,
    }
}

// ── Internals ────────────────────────────────────────────────────────────────

/// Repetition stats plus server-side per-op byte costs for one cell.
/// What a lane's cell measurement returns: per-repetition stats, mean
/// request bytes/op, mean response bytes/op (server-side counters).
pub type Measured = (Vec<CellStats>, f64, f64);

/// One cell's request shape and concurrency.
pub struct CellSpec {
    /// Backend command this cell issues (`ECHO` / `STATIC` / `SINK`).
    pub command: &'static str,
    /// Arguments carried with it.
    pub args: Vec<Value>,
    /// In-flight window per connection.
    pub depth: usize,
    /// Concurrent connections.
    pub connections: usize,
}

/// The wire request each workload issues.
fn request_for(workload: Workload) -> Option<(&'static str, Vec<Value>)> {
    match workload {
        Workload::Echo { payload_bytes } => {
            Some(("ECHO", vec![Value::Str("x".repeat(payload_bytes))]))
        }
        Workload::StaticReply => Some(("STATIC", vec![])),
        // The burst pipelines the 64 B echo — pipelining on merit.
        Workload::PipelinedBurst { .. } => Some(("ECHO", vec![Value::Str("x".repeat(64))])),
        Workload::ConnectionStorm { .. } | Workload::Pending { .. } => None,
    }
}

fn finish_cell(
    scenario: &Scenario,
    lane: Lane,
    depth: usize,
    connections: usize,
    (reps, bytes_in_per_op, bytes_out_per_op): Measured,
) -> CellResult {
    CellResult {
        scenario: scenario.name.to_owned(),
        lane: lane.as_str().to_owned(),
        depth,
        connections,
        status: "ok".to_owned(),
        p50_us: dispersion(reps.iter().map(|r| r.p50_us)),
        p99_us: dispersion(reps.iter().map(|r| r.p99_us)),
        qps: dispersion(reps.iter().map(|r| r.qps)),
        bytes_in_per_op: Some(bytes_in_per_op),
        bytes_out_per_op: Some(bytes_out_per_op),
        reps,
    }
}

/// Ride through std-mutex poisoning (a panicked worker must not wedge the
/// harness; the guarded state stays consistent).
fn lock<T>(mutex: &StdMutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

// ── Thunder lane ─────────────────────────────────────────────────────────────

/// One matrix cell on the Thunder lane: `connections` clients ×
/// `depth` concurrent callers each, window kept continuously full.
async fn thunder_cell(
    targets: &Targets,
    spec: &CellSpec,
    cfg: &RunConfig,
) -> Result<Measured, String> {
    let addr = targets.thunder.local_addr().to_string();
    let mut clients = Vec::with_capacity(spec.connections);
    for _ in 0..spec.connections {
        let client = Client::connect(&addr, bench_profile())
            .await
            .map_err(|e| format!("thunder connect failed: {e}"))?;
        clients.push(Arc::new(client));
    }

    if cfg.warmup > 0 {
        let _ = thunder_window(&clients, spec, cfg.warmup).await?;
    }
    let before = targets.thunder.snapshot();
    let mut reps = Vec::with_capacity(cfg.repetitions);
    for _ in 0..cfg.repetitions {
        let (mut lats, elapsed) = thunder_window(&clients, spec, cfg.ops).await?;
        reps.push(compute(&mut lats, elapsed));
    }
    let after = targets.thunder.snapshot();
    for client in &clients {
        client.close().await;
    }

    let ops = (after.commands_total - before.commands_total).max(1) as f64;
    Ok((
        reps,
        (after.frame_bytes_in_total - before.frame_bytes_in_total) as f64 / ops,
        (after.frame_bytes_out_total - before.frame_bytes_out_total) as f64 / ops,
    ))
}

/// One continuously-full window: `depth` workers per client, each looping
/// sequential calls — a slot re-fills the moment its response lands.
async fn thunder_window(
    clients: &[Arc<Client>],
    spec: &CellSpec,
    total_ops: usize,
) -> Result<(Vec<Duration>, Duration), String> {
    let workers = clients.len() * spec.depth;
    let ops_per_worker = (total_ops / workers.max(1)).max(1);
    let started = Instant::now();
    let mut handles = Vec::with_capacity(workers);
    for client in clients {
        for _ in 0..spec.depth {
            let client = Arc::clone(client);
            let command = spec.command;
            let args = spec.args.clone();
            handles.push(tokio::spawn(async move {
                let mut lats = Vec::with_capacity(ops_per_worker);
                for _ in 0..ops_per_worker {
                    let sent = Instant::now();
                    client
                        .call(command, args.clone())
                        .await
                        .map_err(|e| format!("thunder call failed: {e}"))?;
                    lats.push(sent.elapsed());
                }
                Ok::<Vec<Duration>, String>(lats)
            }));
        }
    }
    let mut all = Vec::with_capacity(ops_per_worker * workers);
    for handle in handles {
        let lats = handle
            .await
            .map_err(|e| format!("thunder worker panicked: {e}"))??;
        all.extend(lats);
    }
    Ok((all, started.elapsed()))
}

/// Connection storm on the Thunder lane: sequential connect + first byte
/// (`PING`), per-storm latency covers dial through decoded reply.
async fn thunder_storm(
    targets: &Targets,
    storms: usize,
    cfg: &RunConfig,
) -> Result<Measured, String> {
    let addr = targets.thunder.local_addr().to_string();
    for _ in 0..cfg.warmup.min(storms) {
        thunder_storm_once(&addr).await?;
    }
    let before = targets.thunder.snapshot();
    let mut reps = Vec::with_capacity(cfg.repetitions);
    for _ in 0..cfg.repetitions {
        let mut lats = Vec::with_capacity(storms);
        let started = Instant::now();
        for _ in 0..storms {
            lats.push(thunder_storm_once(&addr).await?);
        }
        reps.push(compute(&mut lats, started.elapsed()));
    }
    let after = targets.thunder.snapshot();
    let ops = (after.commands_total - before.commands_total).max(1) as f64;
    Ok((
        reps,
        (after.frame_bytes_in_total - before.frame_bytes_in_total) as f64 / ops,
        (after.frame_bytes_out_total - before.frame_bytes_out_total) as f64 / ops,
    ))
}

async fn thunder_storm_once(addr: &str) -> Result<Duration, String> {
    let started = Instant::now();
    let client = Client::connect(addr, bench_profile())
        .await
        .map_err(|e| format!("storm connect failed: {e}"))?;
    client
        .call("PING", vec![])
        .await
        .map_err(|e| format!("storm call failed: {e}"))?;
    let latency = started.elapsed();
    client.close().await;
    Ok(latency)
}

// ── HTTP lane ────────────────────────────────────────────────────────────────

/// One raw keep-alive HTTP/1.1 connection.
struct HttpConn {
    reader: BufReader<OwnedReadHalf>,
    writer: OwnedWriteHalf,
}

impl HttpConn {
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

/// Serialize one RPC as raw HTTP/1.1 request bytes.
fn build_http_request(command: &str, args: &[Value]) -> Vec<u8> {
    let body = serde_json::json!({
        "command": command,
        "args": args.iter().map(wire_to_json).collect::<Vec<_>>(),
    })
    .to_string();
    format!(
        "POST /rpc HTTP/1.1\r\nHost: bench\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

/// Read + fully decode one response — the HTTP lane's measurement point
/// mirrors the Thunder client's full MessagePack decode (BEN-003 parity).
async fn check_http_response(reader: &mut BufReader<OwnedReadHalf>) -> Result<(), String> {
    let (status, body, _bytes) = read_http_response(reader)
        .await
        .map_err(|e| format!("http read failed: {e}"))?;
    if status != 200 {
        return Err(format!(
            "http status {status}: {}",
            String::from_utf8_lossy(&body)
        ));
    }
    let parsed: serde_json::Value =
        serde_json::from_slice(&body).map_err(|e| format!("http body is not JSON: {e}"))?;
    if parsed.get("ok").and_then(serde_json::Value::as_bool) != Some(true) {
        return Err(format!("http lane returned an error: {parsed}"));
    }
    Ok(())
}

/// One matrix cell on the HTTP lane.
async fn http_cell(
    targets: &Targets,
    spec: &CellSpec,
    cfg: &RunConfig,
) -> Result<Measured, String> {
    let addr = targets.http.local_addr();
    let request = Arc::new(build_http_request(spec.command, &spec.args));
    let mut conns = Vec::with_capacity(spec.connections);
    for _ in 0..spec.connections {
        conns.push(
            HttpConn::connect(addr)
                .await
                .map_err(|e| format!("http connect failed: {e}"))?,
        );
    }

    if cfg.warmup > 0 {
        let (warmed, _lats, _elapsed) =
            http_window(conns, spec.depth, cfg.warmup, &request).await?;
        conns = warmed;
    }
    let before = targets.http.snapshot();
    let mut reps = Vec::with_capacity(cfg.repetitions);
    for _ in 0..cfg.repetitions {
        let (returned, mut lats, elapsed) =
            http_window(conns, spec.depth, cfg.ops, &request).await?;
        conns = returned;
        reps.push(compute(&mut lats, elapsed));
    }
    let after = targets.http.snapshot();
    drop(conns);

    let ops = (after.requests - before.requests).max(1) as f64;
    Ok((
        reps,
        (after.bytes_in - before.bytes_in) as f64 / ops,
        (after.bytes_out - before.bytes_out) as f64 / ops,
    ))
}

/// One continuously-full HTTP window across all connections.
///
/// Each connection issues at least `depth` requests so the pipeline
/// window actually fills — the same floor the Thunder lane's
/// worker-per-slot model implies (parity, BEN-003).
async fn http_window(
    conns: Vec<HttpConn>,
    depth: usize,
    total_ops: usize,
    request: &Arc<Vec<u8>>,
) -> Result<(Vec<HttpConn>, Vec<Duration>, Duration), String> {
    let per_conn = (total_ops / conns.len().max(1)).max(depth).max(1);
    let started = Instant::now();
    let mut handles = Vec::with_capacity(conns.len());
    for conn in conns {
        let request = Arc::clone(request);
        handles.push(tokio::spawn(http_conn_window(
            conn, depth, per_conn, request,
        )));
    }
    let mut returned = Vec::with_capacity(handles.len());
    let mut all = Vec::with_capacity(per_conn * handles.len());
    for handle in handles {
        let (conn, lats) = handle
            .await
            .map_err(|e| format!("http worker panicked: {e}"))??;
        returned.push(conn);
        all.extend(lats);
    }
    Ok((returned, all, started.elapsed()))
}

/// FIFO pipeline window on one connection: the sender keeps up to `depth`
/// requests on the wire (a semaphore slot per in-flight request), the
/// receiver reads responses in order and frees slots — continuous
/// pipelining, no inter-batch gaps (BEN-003).
async fn http_conn_window(
    mut conn: HttpConn,
    depth: usize,
    ops: usize,
    request: Arc<Vec<u8>>,
) -> Result<(HttpConn, Vec<Duration>), String> {
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
                    .map_err(|e| format!("http write failed: {e}"))?;
            }
            Ok::<(), String>(())
        }
    };
    let receiver = {
        let pending = Arc::clone(&pending);
        async move {
            let mut lats = Vec::with_capacity(ops);
            for _ in 0..ops {
                check_http_response(reader).await?;
                let (sent, permit) = lock(&pending)
                    .pop_front()
                    .ok_or_else(|| "http response without a pending request".to_owned())?;
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

/// Connection storm on the HTTP lane: sequential connect + one request +
/// fully decoded response.
async fn http_storm(targets: &Targets, storms: usize, cfg: &RunConfig) -> Result<Measured, String> {
    let addr = targets.http.local_addr();
    let request = build_http_request("PING", &[]);
    for _ in 0..cfg.warmup.min(storms) {
        http_storm_once(addr, &request).await?;
    }
    let before = targets.http.snapshot();
    let mut reps = Vec::with_capacity(cfg.repetitions);
    for _ in 0..cfg.repetitions {
        let mut lats = Vec::with_capacity(storms);
        let started = Instant::now();
        for _ in 0..storms {
            lats.push(http_storm_once(addr, &request).await?);
        }
        reps.push(compute(&mut lats, started.elapsed()));
    }
    let after = targets.http.snapshot();
    let ops = (after.requests - before.requests).max(1) as f64;
    Ok((
        reps,
        (after.bytes_in - before.bytes_in) as f64 / ops,
        (after.bytes_out - before.bytes_out) as f64 / ops,
    ))
}

async fn http_storm_once(addr: SocketAddr, request: &[u8]) -> Result<Duration, String> {
    let started = Instant::now();
    let mut conn = HttpConn::connect(addr)
        .await
        .map_err(|e| format!("storm connect failed: {e}"))?;
    conn.writer
        .write_all(request)
        .await
        .map_err(|e| format!("storm write failed: {e}"))?;
    check_http_response(&mut conn.reader).await?;
    Ok(started.elapsed())
}

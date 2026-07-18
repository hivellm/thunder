//! Product-level RPC-vs-HTTP harness (BEN-040, FR-73, DAG T4.4).
//!
//! The transport shootout (the rest of this crate) isolates the *transport*
//! over a no-op backend. This harness answers a different question, one level
//! up: on a product's **real engine**, how much does the RPC transport win
//! over HTTP when the engine is held identical? It runs three scenarios the
//! family mandated — bulk ingest, small high-QPS call, pipelined polling — and
//! drives each over **both** a Thunder RPC listener and an HTTP/1.1 listener
//! that call the **same handler** ([`ProductHandler::respond`]), so at product
//! level the transport is the only variable (BEN-040).
//!
//! ## Same handler, both transports
//!
//! A product implements [`ProductHandler`] once, routing the harness commands
//! to its engine. [`DispatchAdapter`] wraps it as a [`thunder::server::Dispatch`]
//! for the RPC lane; the HTTP lane calls the very same `respond`. Neither lane
//! can diverge on semantics because there is one implementation. This mirrors
//! how [`crate::backend::NoopBackend`] backs both shootout lanes, generalized so
//! a real engine can sit behind it instead of a no-op.
//!
//! ## Seed floors are SEEDS, not results
//!
//! The acceptance floors below are seeded from Nexus's table (point read
//! 320 µs baseline -> ≤120 µs target; bulk 780 ms -> ≤220 ms). BEN-040 is
//! explicit that these are **starting points**: each product recalibrates its
//! own floors from its first measured run on its real engine. Every floor this
//! harness reports carries that caveat, and per BEN-031 no number here may be
//! cited as a product result while the shootout measurement substrate is
//! unsettled (see `phase4_hotpath-optimization`). The harness's job is to be
//! runnable and correct; the numbers are provisional until a product owns them.
//!
//! ## What lives where
//!
//! This module is the Thunder-repo **template**: the scenarios, the dual-lane
//! driver, the seeded floors, and a demonstration engine ([`DemoEngine`]) that
//! proves it runs end to end. Pointing it at a real engine, and committing the
//! per-product artifact, is each product's own wiring in its own repository
//! (Nexus / Vectorizer / Synap) — BEN-040's per-product half.

use std::collections::VecDeque;
use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex as StdMutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use thunder::client::Client;
use thunder::server::{
    spawn_listener, AuthError, Credentials, Dispatch, ListenerConfig, Principal, ServerInfo,
    Session,
};
use thunder::wire::Value;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, watch, OwnedSemaphorePermit, Semaphore};

use crate::artifact::Environment;
use crate::driver::{bench_profile, RunConfig};
use crate::http::{json_to_wire, read_http_response, wire_to_json};
use crate::pinning::PinReport;
use crate::stats::{compute, dispersion, CellStats, Dispersion};

/// Artifact schema tag for a product-harness run (distinct from the shootout's).
pub const SCHEMA: &str = "thunder-bench/product-harness-v1";

// ── Seed acceptance floors (BEN-040), from Nexus's table. SEEDS, NOT RESULTS. ──

/// Point-read latency the seed is measured against — Nexus's pre-Thunder p50.
pub const POINT_READ_SEED_BASELINE_US: f64 = 320.0;
/// Point-read target the seed aims for. A product recalibrates its own.
pub const POINT_READ_SEED_TARGET_US: f64 = 120.0;
/// Bulk-operation latency the seed is measured against — Nexus's pre-Thunder.
pub const BULK_SEED_BASELINE_MS: f64 = 780.0;
/// Bulk-operation target the seed aims for. A product recalibrates its own.
pub const BULK_SEED_TARGET_MS: f64 = 220.0;

// Compile-time invariant: a seed *target* must sit below its *baseline*, or the
// floor would be asking for a regression. Checked at build, not at test time.
const _: () = assert!(POINT_READ_SEED_TARGET_US < POINT_READ_SEED_BASELINE_US);
const _: () = assert!(BULK_SEED_TARGET_MS < BULK_SEED_BASELINE_MS);

/// The mandated bulk-ingest batch: 1000 items of 256 bytes each.
const BULK_ITEMS: usize = 1000;
const BULK_ITEM_BYTES: usize = 256;

/// The caveat stamped on every floor this harness reports.
const SEED_CAVEAT: &str = "SEED from Nexus's acceptance table — provisional, not a \
    product-calibrated result. Each product recalibrates its own floor from its first measured \
    run on its real engine (BEN-040), and no number here may be cited while the shootout \
    substrate is unsettled (BEN-031).";

/// A product's engine behind **both** transports (BEN-040 same-handler
/// discipline). One implementation, invoked identically whichever transport
/// delivered the request — the same shape [`thunder::server::Dispatch`] uses,
/// minus the session the transport owns.
pub trait ProductHandler: Send + Sync + 'static {
    /// Answer one command. A real product routes `command`/`args` to its
    /// engine; the demonstration [`DemoEngine`] keeps a tiny in-memory store.
    ///
    /// The future must be `Send`: both the RPC listener's [`Dispatch`] and the
    /// HTTP lane's per-connection task spawn it onto the runtime.
    fn respond(
        &self,
        command: &str,
        args: Vec<Value>,
    ) -> impl Future<Output = Result<Value, String>> + Send;
}

/// Adapts a [`ProductHandler`] into the RPC lane's [`Dispatch`], so the Thunder
/// listener and the HTTP lane call one and the same `respond`.
struct DispatchAdapter<H> {
    inner: Arc<H>,
}

impl<H: ProductHandler> Dispatch for DispatchAdapter<H> {
    type Identity = ();

    async fn dispatch(
        &self,
        _session: &Session,
        command: &str,
        args: Vec<Value>,
    ) -> Result<Value, String> {
        self.inner.respond(command, args).await
    }

    /// The harness profile uses `Handshake::None`, so this never runs on the
    /// measured path; it accepts everything for completeness.
    async fn authenticate(&self, _creds: Credentials) -> Result<Principal, AuthError> {
        Ok(Principal::new("product-harness".to_owned()))
    }
}

// ── Scenarios (BEN-040) ────────────────────────────────────────────────────────

/// One product-harness scenario. Distinct from the frozen BEN-010 shootout
/// matrix: these three are the product-level trio, each with its own request
/// shape, concurrency, and seed floor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProductScenario {
    /// One large batch per call — the ingest hot path (bulk seed floor, ms).
    BulkIngest,
    /// A tiny point read at concurrency — the high-QPS path (point seed, µs).
    PointReadHighQps,
    /// Tiny reads kept pipelined at depth — the polling path (no seed floor).
    PipelinedPolling,
}

impl ProductScenario {
    /// The three scenarios, artifact order.
    pub const ALL: [ProductScenario; 3] = [
        ProductScenario::BulkIngest,
        ProductScenario::PointReadHighQps,
        ProductScenario::PipelinedPolling,
    ];

    /// Stable scenario name (artifact key).
    pub fn name(self) -> &'static str {
        match self {
            Self::BulkIngest => "bulk-ingest",
            Self::PointReadHighQps => "point-read-high-qps",
            Self::PipelinedPolling => "pipelined-polling",
        }
    }

    /// What this scenario probes.
    pub fn probe(self) -> &'static str {
        match self {
            Self::BulkIngest => "one large batch per call — ingest hot path",
            Self::PointReadHighQps => "tiny point read at concurrency — high-QPS path",
            Self::PipelinedPolling => "tiny reads kept pipelined at depth — polling path",
        }
    }

    /// The wire request each scenario issues on both lanes.
    pub fn request(self) -> (&'static str, Vec<Value>) {
        match self {
            Self::BulkIngest => {
                let item = Value::bytes(vec![b'x'; BULK_ITEM_BYTES]);
                let batch = std::iter::repeat_n(item, BULK_ITEMS).collect();
                ("INGEST", vec![Value::Array(batch)])
            }
            Self::PointReadHighQps => ("GET", vec![Value::Str("k".to_owned())]),
            Self::PipelinedPolling => ("POLL", vec![]),
        }
    }

    /// In-flight window per connection.
    pub fn depth(self) -> usize {
        match self {
            Self::BulkIngest | Self::PointReadHighQps => 1,
            Self::PipelinedPolling => 16,
        }
    }

    /// Concurrent connections.
    pub fn connections(self) -> usize {
        match self {
            Self::BulkIngest => 1,
            Self::PointReadHighQps => 4,
            Self::PipelinedPolling => 1,
        }
    }

    /// Measured ops per repetition, scaled from the run config. Bulk ops are
    /// individually huge, so far fewer of them make a repetition.
    pub fn ops(self, cfg: &RunConfig) -> usize {
        match self {
            Self::BulkIngest => (cfg.ops / 40).max(10),
            Self::PointReadHighQps | Self::PipelinedPolling => cfg.ops,
        }
    }

    /// The seed floor this scenario is judged against (a seed, never a result).
    pub fn floor(self) -> FloorSeed {
        match self {
            Self::BulkIngest => FloorSeed::BulkMs {
                baseline: BULK_SEED_BASELINE_MS,
                target: BULK_SEED_TARGET_MS,
            },
            Self::PointReadHighQps => FloorSeed::PointReadUs {
                baseline: POINT_READ_SEED_BASELINE_US,
                target: POINT_READ_SEED_TARGET_US,
            },
            Self::PipelinedPolling => FloorSeed::Calibrate,
        }
    }
}

/// A seeded acceptance floor. The target is a **seed**; a product recalibrates.
#[derive(Debug, Clone, Copy)]
pub enum FloorSeed {
    /// Point-read latency floor, microseconds.
    PointReadUs {
        /// Nexus's pre-Thunder p50.
        baseline: f64,
        /// Seed target.
        target: f64,
    },
    /// Bulk-operation latency floor, milliseconds.
    BulkMs {
        /// Nexus's pre-Thunder p50.
        baseline: f64,
        /// Seed target.
        target: f64,
    },
    /// No seed — the product calibrates this floor from its first measured run.
    Calibrate,
}

impl FloorSeed {
    /// Judge the RPC lane's p50 (in µs) against the seed target. The verdict is
    /// explicitly provisional — clearing a *seed* is not clearing a product's
    /// own calibrated floor.
    fn evaluate(self, rpc_p50_us: f64) -> FloorReport {
        match self {
            Self::PointReadUs { baseline, target } => FloorReport {
                kind: "point-read".to_owned(),
                unit: "us".to_owned(),
                baseline: Some(baseline),
                target: Some(target),
                measured_rpc: rpc_p50_us,
                clears_seed: Some(rpc_p50_us <= target),
                note: SEED_CAVEAT.to_owned(),
            },
            Self::BulkMs { baseline, target } => {
                let measured_ms = rpc_p50_us / 1000.0;
                FloorReport {
                    kind: "bulk".to_owned(),
                    unit: "ms".to_owned(),
                    baseline: Some(baseline),
                    target: Some(target),
                    measured_rpc: measured_ms,
                    clears_seed: Some(measured_ms <= target),
                    note: SEED_CAVEAT.to_owned(),
                }
            }
            Self::Calibrate => FloorReport {
                kind: "calibrate".to_owned(),
                unit: "us".to_owned(),
                baseline: None,
                target: None,
                measured_rpc: rpc_p50_us,
                clears_seed: None,
                note: "no seed floor — the product calibrates this one from its first measured \
                       run (BEN-040)."
                    .to_owned(),
            },
        }
    }
}

// ── Results ────────────────────────────────────────────────────────────────────

/// One transport lane's measurement of one scenario.
#[derive(Debug, Clone, serde::Serialize)]
pub struct LaneMeasure {
    /// p50 latency dispersion across repetitions, microseconds.
    pub p50_us: Dispersion,
    /// p99 latency dispersion across repetitions, microseconds.
    pub p99_us: Dispersion,
    /// Throughput dispersion across repetitions.
    pub qps: Dispersion,
}

impl LaneMeasure {
    fn from_reps(reps: &[CellStats]) -> Result<Self, String> {
        let missing = || "no repetitions measured".to_owned();
        Ok(Self {
            p50_us: dispersion(reps.iter().map(|r| r.p50_us)).ok_or_else(missing)?,
            p99_us: dispersion(reps.iter().map(|r| r.p99_us)).ok_or_else(missing)?,
            qps: dispersion(reps.iter().map(|r| r.qps)).ok_or_else(missing)?,
        })
    }
}

/// A seed floor's verdict, always provisional.
#[derive(Debug, Clone, serde::Serialize)]
pub struct FloorReport {
    /// Floor family (`point-read` / `bulk` / `calibrate`).
    pub kind: String,
    /// Unit the baseline/target/measured are in (`us` / `ms`).
    pub unit: String,
    /// Nexus's seed baseline, if any.
    pub baseline: Option<f64>,
    /// Seed target, if any.
    pub target: Option<f64>,
    /// The RPC lane's measured p50 on this scenario, in `unit`.
    pub measured_rpc: f64,
    /// Whether the RPC lane cleared the **seed** target — `None` when there is
    /// no seed (the product calibrates). Never read as a product verdict.
    pub clears_seed: Option<bool>,
    /// The provisional-seed caveat.
    pub note: String,
}

/// One scenario's full RPC-vs-HTTP report.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ScenarioReport {
    /// Scenario name.
    pub scenario: String,
    /// What it probes.
    pub probe: String,
    /// Pipeline depth used.
    pub depth: usize,
    /// Connections used.
    pub connections: usize,
    /// The Thunder RPC lane.
    pub rpc: LaneMeasure,
    /// The HTTP/1.1 lane.
    pub http: LaneMeasure,
    /// How much faster the RPC lane's throughput is, percent
    /// (`(rpc_qps - http_qps) / http_qps * 100`).
    pub rpc_vs_http_qps_pct: f64,
    /// The seeded floor verdict (provisional).
    pub floor: FloorReport,
}

/// A complete product-harness run.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ProductArtifact {
    /// [`SCHEMA`].
    pub schema: String,
    /// Machine/environment header (BEN-011).
    pub environment: Environment,
    /// Measured ops per rep (nominal — scenarios scale it, see per-scenario).
    pub ops: usize,
    /// Warmup ops per cell (discarded).
    pub warmup: usize,
    /// Repetitions per cell.
    pub repetitions: usize,
    /// Every scenario's RPC-vs-HTTP report.
    pub scenarios: Vec<ScenarioReport>,
    /// Run-wide honesty notes.
    pub notes: Vec<String>,
}

// ── Runner ─────────────────────────────────────────────────────────────────────

/// Ride through std-mutex poisoning (a panicked worker must not wedge the run).
fn lock<T>(mutex: &StdMutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Run all three scenarios against `handler`, over both transports, and return
/// the assembled report per scenario. Spawns one Thunder listener and one HTTP
/// listener over the same handler, in this process (the engine is identical
/// across lanes by construction).
pub async fn run<H: ProductHandler>(
    handler: Arc<H>,
    cfg: &RunConfig,
) -> Result<Vec<ScenarioReport>, String> {
    let adapter = Arc::new(DispatchAdapter {
        inner: Arc::clone(&handler),
    });
    let thunder = spawn_listener(
        adapter,
        bench_profile(),
        ServerInfo {
            name: "product-harness".to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
        },
        ListenerConfig::default(),
    )
    .await
    .map_err(|e| format!("thunder listener failed: {e}"))?;
    let http = spawn_product_http(Arc::clone(&handler))
        .await
        .map_err(|e| format!("http listener failed: {e}"))?;

    let mut reports = Vec::with_capacity(ProductScenario::ALL.len());
    for scenario in ProductScenario::ALL {
        let rpc = measure_thunder(thunder.local_addr(), scenario, cfg).await?;
        let http_measure = measure_http(http.local_addr(), scenario, cfg).await?;
        let rpc_vs_http_qps_pct = if http_measure.qps.mean > 0.0 {
            (rpc.qps.mean - http_measure.qps.mean) / http_measure.qps.mean * 100.0
        } else {
            0.0
        };
        reports.push(ScenarioReport {
            scenario: scenario.name().to_owned(),
            probe: scenario.probe().to_owned(),
            depth: scenario.depth(),
            connections: scenario.connections(),
            floor: scenario.floor().evaluate(rpc.p50_us.mean),
            rpc,
            http: http_measure,
            rpc_vs_http_qps_pct,
        });
    }

    thunder.stop().await;
    http.stop().await;
    Ok(reports)
}

/// Assemble a full artifact from a run against `handler`.
pub async fn run_artifact<H: ProductHandler>(
    handler: Arc<H>,
    cfg: &RunConfig,
) -> Result<ProductArtifact, String> {
    let scenarios = run(handler, cfg).await?;
    let cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    Ok(ProductArtifact {
        schema: SCHEMA.to_owned(),
        environment: Environment::capture(PinReport::unpinned(cores)),
        ops: cfg.ops,
        warmup: cfg.warmup,
        repetitions: cfg.repetitions,
        scenarios,
        notes: vec![
            "Product level: the Thunder RPC lane and the HTTP lane call the same handler, so \
             the engine cancels out and the transport is the only variable (BEN-040)."
                .to_owned(),
            "Floors are SEEDS from Nexus's table, not results; a product recalibrates its own \
             from its first measured run (BEN-040), and numbers must not be cited while the \
             shootout substrate is unsettled (BEN-031)."
                .to_owned(),
        ],
    })
}

// ── Thunder lane ─────────────────────────────────────────────────────────────

async fn measure_thunder(
    addr: SocketAddr,
    scenario: ProductScenario,
    cfg: &RunConfig,
) -> Result<LaneMeasure, String> {
    let addr = addr.to_string();
    let (command, args) = scenario.request();
    let depth = scenario.depth();
    let ops = scenario.ops(cfg);
    let warmup = cfg.warmup.min(ops);

    let mut clients = Vec::with_capacity(scenario.connections());
    for _ in 0..scenario.connections() {
        let client = Client::connect(&addr, bench_profile())
            .await
            .map_err(|e| format!("rpc connect failed: {e}"))?;
        clients.push(Arc::new(client));
    }
    if warmup > 0 {
        thunder_window(&clients, command, &args, depth, warmup).await?;
    }
    let mut reps = Vec::with_capacity(cfg.repetitions);
    for _ in 0..cfg.repetitions {
        let (mut lats, elapsed) = thunder_window(&clients, command, &args, depth, ops).await?;
        reps.push(compute(&mut lats, elapsed));
    }
    for client in &clients {
        client.close().await;
    }
    LaneMeasure::from_reps(&reps)
}

/// One continuously-full window: `depth` workers per client, each looping
/// sequential calls — a slot re-fills the moment its response lands.
async fn thunder_window(
    clients: &[Arc<Client>],
    command: &'static str,
    args: &[Value],
    depth: usize,
    total_ops: usize,
) -> Result<(Vec<Duration>, Duration), String> {
    let workers = clients.len() * depth;
    let ops_per_worker = (total_ops / workers.max(1)).max(1);
    let started = Instant::now();
    let mut handles = Vec::with_capacity(workers);
    for client in clients {
        for _ in 0..depth {
            let client = Arc::clone(client);
            let args = args.to_vec();
            handles.push(tokio::spawn(async move {
                let mut lats = Vec::with_capacity(ops_per_worker);
                for _ in 0..ops_per_worker {
                    let sent = Instant::now();
                    client
                        .call(command, args.clone())
                        .await
                        .map_err(|e| format!("rpc call failed: {e}"))?;
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
            .map_err(|e| format!("rpc worker panicked: {e}"))??;
        all.extend(lats);
    }
    Ok((all, started.elapsed()))
}

// ── HTTP lane ────────────────────────────────────────────────────────────────

/// Handle to the harness's minimal HTTP listener.
pub struct ProductHttpHandle {
    local_addr: SocketAddr,
    shutdown: watch::Sender<bool>,
    done: Option<mpsc::Receiver<()>>,
}

impl ProductHttpHandle {
    /// The bound loopback address.
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Signal shutdown and wait for the accept loop to stop.
    pub async fn stop(mut self) {
        let _ = self.shutdown.send(true);
        if let Some(mut done) = self.done.take() {
            let _ = done.recv().await;
        }
    }
}

/// Bind loopback and serve `handler` over minimal HTTP/1.1 + JSON. Unlike the
/// shootout's HTTP lane, this one is generic over the handler and calls
/// [`ProductHandler::respond`] — the identical surface the RPC lane calls.
async fn spawn_product_http<H: ProductHandler>(handler: Arc<H>) -> io::Result<ProductHttpHandle> {
    let listener = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0))).await?;
    let local_addr = listener.local_addr()?;
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let (done_tx, done_rx) = mpsc::channel::<()>(1);
    tokio::spawn(http_accept_loop(listener, handler, shutdown_rx, done_tx));
    Ok(ProductHttpHandle {
        local_addr,
        shutdown: shutdown_tx,
        done: Some(done_rx),
    })
}

async fn http_accept_loop<H: ProductHandler>(
    listener: TcpListener,
    handler: Arc<H>,
    mut shutdown: watch::Receiver<bool>,
    done: mpsc::Sender<()>,
) {
    loop {
        let accepted = tokio::select! {
            _ = shutdown.wait_for(|stop| *stop) => break,
            accepted = listener.accept() => accepted,
        };
        let Ok((stream, _peer)) = accepted else {
            continue;
        };
        let handler = Arc::clone(&handler);
        tokio::spawn(async move {
            let _ = serve_http_conn(stream, handler).await;
        });
    }
    drop(done);
}

async fn serve_http_conn<H: ProductHandler>(stream: TcpStream, handler: Arc<H>) -> io::Result<()> {
    stream.set_nodelay(true)?;
    let (read_half, mut writer) = stream.into_split();
    let mut reader = BufReader::new(read_half);
    while let Some((command, args)) = read_http_request(&mut reader).await? {
        let body = match handler.respond(&command, args).await {
            Ok(value) => serde_json::json!({ "ok": true, "result": wire_to_json(&value) }),
            Err(message) => serde_json::json!({ "ok": false, "error": message }),
        }
        .to_string();
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        writer.write_all(response.as_bytes()).await?;
    }
    Ok(())
}

/// Read one HTTP request; `Ok(None)` on a clean connection close.
async fn read_http_request(
    reader: &mut BufReader<OwnedReadHalf>,
) -> io::Result<Option<(String, Vec<Value>)>> {
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).await? == 0 {
        return Ok(None);
    }
    let mut content_length = 0usize;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).await? == 0 {
            return Ok(None);
        }
        let trimmed = header.trim_end();
        if trimmed.is_empty() {
            break;
        }
        let lower = trimmed.to_ascii_lowercase();
        if let Some(rest) = lower.strip_prefix("content-length:") {
            content_length = rest.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0u8; content_length];
    reader.read_exact(&mut body).await?;
    let json: serde_json::Value =
        serde_json::from_slice(&body).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let command = json
        .get("command")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .to_owned();
    let args = json
        .get("args")
        .and_then(serde_json::Value::as_array)
        .map(|a| a.iter().map(json_to_wire).collect())
        .unwrap_or_default();
    Ok(Some((command, args)))
}

/// One raw keep-alive HTTP/1.1 connection (client side).
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

fn build_http_request(command: &str, args: &[Value]) -> Vec<u8> {
    let body = serde_json::json!({
        "command": command,
        "args": args.iter().map(wire_to_json).collect::<Vec<_>>(),
    })
    .to_string();
    format!(
        "POST /rpc HTTP/1.1\r\nHost: harness\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

async fn measure_http(
    addr: SocketAddr,
    scenario: ProductScenario,
    cfg: &RunConfig,
) -> Result<LaneMeasure, String> {
    let (command, args) = scenario.request();
    let request = Arc::new(build_http_request(command, &args));
    let depth = scenario.depth();
    let ops = scenario.ops(cfg);
    let warmup = cfg.warmup.min(ops);

    let mut conns = Vec::with_capacity(scenario.connections());
    for _ in 0..scenario.connections() {
        conns.push(
            HttpConn::connect(addr)
                .await
                .map_err(|e| format!("http connect failed: {e}"))?,
        );
    }
    if warmup > 0 {
        conns = http_window(conns, depth, warmup, &request).await?.0;
    }
    let mut reps = Vec::with_capacity(cfg.repetitions);
    for _ in 0..cfg.repetitions {
        let (returned, mut lats, elapsed) = http_window(conns, depth, ops, &request).await?;
        conns = returned;
        reps.push(compute(&mut lats, elapsed));
    }
    drop(conns);
    LaneMeasure::from_reps(&reps)
}

/// One continuously-full HTTP window across all connections.
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
    let mut all = Vec::with_capacity(per_conn * handles.len().max(1));
    for handle in handles {
        let (conn, lats) = handle
            .await
            .map_err(|e| format!("http worker panicked: {e}"))??;
        returned.push(conn);
        all.extend(lats);
    }
    Ok((returned, all, started.elapsed()))
}

/// FIFO pipeline window on one connection: up to `depth` requests on the wire,
/// responses read in order freeing slots — continuous pipelining, no gaps.
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
                let (status, body, _bytes) = read_http_response(reader)
                    .await
                    .map_err(|e| format!("http read failed: {e}"))?;
                if status != 200 {
                    return Err(format!("http status {status}"));
                }
                let parsed: serde_json::Value = serde_json::from_slice(&body)
                    .map_err(|e| format!("http body is not JSON: {e}"))?;
                if parsed.get("ok").and_then(serde_json::Value::as_bool) != Some(true) {
                    return Err(format!("http lane returned an error: {parsed}"));
                }
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

// ── Demonstration engine ───────────────────────────────────────────────────────

/// A stand-in "product engine" so the template runs end to end without a real
/// product. A tiny in-memory store: `INGEST` counts a batch, `GET`/`POLL`
/// return a small fixed value. Enough work to be a plausible handler, little
/// enough to stay out of the way — a real product replaces this with its engine.
#[derive(Debug)]
pub struct DemoEngine {
    value: Vec<u8>,
    ingested: StdMutex<usize>,
}

impl DemoEngine {
    /// Build the demo engine with a small fixed read value.
    pub fn new() -> Self {
        Self {
            value: vec![b'x'; 256],
            ingested: StdMutex::new(0),
        }
    }

    /// How many items have been ingested (test/inspection).
    pub fn ingested(&self) -> usize {
        *lock(&self.ingested)
    }
}

impl Default for DemoEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl ProductHandler for DemoEngine {
    async fn respond(&self, command: &str, args: Vec<Value>) -> Result<Value, String> {
        match command {
            "INGEST" => {
                let count = match args.first() {
                    Some(Value::Array(items)) => items.len(),
                    _ => 0,
                };
                *lock(&self.ingested) += count;
                Ok(Value::Int(count as i64))
            }
            "GET" | "POLL" | "PING" => Ok(Value::bytes(self.value.clone())),
            other => Err(format!("ERR unknown command '{other}'")),
        }
    }
}

// ── Artifact rendering ─────────────────────────────────────────────────────────

/// Serialize the artifact as pretty JSON.
pub fn render_json(artifact: &ProductArtifact) -> Result<String, String> {
    serde_json::to_string_pretty(artifact)
        .map(|mut s| {
            s.push('\n');
            s
        })
        .map_err(|e| format!("artifact serialization failed: {e}"))
}

/// Render the human summary. Floors are stamped SEED throughout.
pub fn render_markdown(artifact: &ProductArtifact) -> String {
    let mut md = String::with_capacity(2048);
    md.push_str("# thunder-bench — product harness (RPC vs HTTP, BEN-040)\n\n");
    md.push_str(
        "The RPC and HTTP lanes call the **same handler**, so at product level the transport is \
         the only variable. Floors below are **SEEDS** from Nexus's acceptance table, not \
         results — each product recalibrates its own from its first measured run (BEN-040), and \
         no number here may be cited while the shootout substrate is unsettled (BEN-031).\n\n",
    );
    md.push_str(&format!(
        "Run: {} reps, {} warmup ops (scenarios scale ops from a {} nominal).\n\n",
        artifact.repetitions, artifact.warmup, artifact.ops
    ));
    md.push_str("| Scenario | depth | conns | RPC p50 µs | HTTP p50 µs | RPC qps | HTTP qps | RPC vs HTTP | Seed floor |\n");
    md.push_str("|---|---:|---:|---:|---:|---:|---:|---:|---|\n");
    for s in &artifact.scenarios {
        let floor = match (&s.floor.target, s.floor.clears_seed) {
            (Some(target), Some(clears)) => format!(
                "{} {:.0}{} seed → {} (SEED)",
                s.floor.kind,
                target,
                s.floor.unit,
                if clears { "clears" } else { "above" }
            ),
            _ => format!("{} — calibrate", s.floor.kind),
        };
        md.push_str(&format!(
            "| {} | {} | {} | {:.1} | {:.1} | {:.0} | {:.0} | {:+.1}% | {} |\n",
            s.scenario,
            s.depth,
            s.connections,
            s.rpc.p50_us.mean,
            s.http.p50_us.mean,
            s.rpc.qps.mean,
            s.http.qps.mean,
            s.rpc_vs_http_qps_pct,
            floor,
        ));
    }
    md.push_str("\n## Notes\n\n");
    for note in &artifact.notes {
        md.push_str(&format!("- {note}\n"));
    }
    md
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn bulk_ingest_builds_the_mandated_batch() {
        let (command, args) = ProductScenario::BulkIngest.request();
        assert_eq!(command, "INGEST");
        match args.first() {
            Some(Value::Array(items)) => {
                assert_eq!(items.len(), BULK_ITEMS);
                assert!(
                    matches!(items.first(), Some(Value::Bytes(b)) if b.len() == BULK_ITEM_BYTES)
                );
            }
            other => panic!("expected one batch array, got {other:?}"),
        }
    }

    #[test]
    fn point_and_poll_are_tiny() {
        assert_eq!(ProductScenario::PointReadHighQps.request().0, "GET");
        assert_eq!(ProductScenario::PipelinedPolling.request().0, "POLL");
        assert!(ProductScenario::PipelinedPolling.request().1.is_empty());
    }

    #[test]
    fn point_floor_clears_below_target_and_fails_above() {
        let floor = ProductScenario::PointReadHighQps.floor();
        assert_eq!(floor.evaluate(100.0).clears_seed, Some(true));
        assert_eq!(floor.evaluate(200.0).clears_seed, Some(false));
    }

    #[test]
    fn bulk_floor_judges_in_milliseconds() {
        // 150_000 µs = 150 ms, under the 220 ms seed target.
        let report = ProductScenario::BulkIngest.floor().evaluate(150_000.0);
        assert_eq!(report.unit, "ms");
        assert!((report.measured_rpc - 150.0).abs() < 1e-9);
        assert_eq!(report.clears_seed, Some(true));
    }

    #[test]
    fn polling_has_no_seed_floor() {
        let report = ProductScenario::PipelinedPolling.floor().evaluate(50.0);
        assert_eq!(report.clears_seed, None);
        assert!(report.target.is_none());
    }

    #[test]
    fn every_floor_report_carries_a_caveat() {
        for scenario in ProductScenario::ALL {
            let note = scenario.floor().evaluate(100.0).note;
            assert!(!note.is_empty(), "{} floor has no caveat", scenario.name());
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn harness_runs_end_to_end_over_both_transports() {
        let handler = Arc::new(DemoEngine::new());
        let cfg = RunConfig {
            ops: 40,
            warmup: 4,
            repetitions: 2,
        };
        let artifact = run_artifact(Arc::clone(&handler), &cfg).await.unwrap();

        // Three scenarios, each with both lanes measured.
        assert_eq!(artifact.scenarios.len(), 3);
        for s in &artifact.scenarios {
            assert!(s.rpc.qps.mean > 0.0, "{}: rpc produced no qps", s.scenario);
            assert!(
                s.http.qps.mean > 0.0,
                "{}: http produced no qps",
                s.scenario
            );
        }
        // The demo engine actually ran the ingest batches over both lanes.
        assert!(handler.ingested() > 0);
        // Both renderers work and the SEED caveat survives to the summary.
        assert!(render_json(&artifact)
            .unwrap()
            .contains("product-harness-v1"));
        assert!(render_markdown(&artifact).contains("SEED"));
    }
}

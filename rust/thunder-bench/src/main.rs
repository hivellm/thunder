//! The shootout CLI — hand-rolled arg parsing, no clap.
//!
//! ```text
//! cargo run -p thunder-bench --release -- --scenario all --out bench-out/
//! ```

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use thunder_bench::artifact::{write_artifact, Artifact, Environment};
use thunder_bench::driver::{run_scenario, spawn_targets, Lane, RunConfig};
use thunder_bench::pinning::{self, PinReport};
use thunder_bench::scenarios::{self, Scenario};
use thunder_bench::stats::{noise_check, NoisyCell, DEFAULT_NOISE_FLOOR_PCT};

const USAGE: &str = "\
thunder-bench — transport shootout skeleton (SPEC-007, DAG T1.6)

USAGE:
    cargo run -p thunder-bench --release -- [OPTIONS]

OPTIONS:
    --scenario <names|all>   comma-separated scenario names, or 'all' (default: all)
    --out <dir>              artifact output directory (default: bench-out)
    --ops <n>                measured ops per repetition per cell (default: 2000)
    --warmup <n>             discarded warmup ops per cell (default: 200)
    --reps <n>               repetitions per cell, >= 1 (default: 3)
    --label <name>           artifact file stem (default: run-<unix-timestamp>)
    --cores <n>              pin the runtime's worker threads to <n> cores
                             (default: 4, or all cores if fewer). BEN-011 asks
                             for pinned runs; unpinned, the scheduler may move
                             the driver and listener between repetitions, so
                             consecutive repetitions do not measure the same
                             machine.
    --no-pin                 do not pin. The artifact records the run as
                             unpinned and its numbers may not settle a BEN-020
                             (>=10%) question.
    --noise-floor <pct>      max qps spread across repetitions before the run
                             is REFUSED (default: 5). A cell whose own
                             repetitions disagree by more than the margin under
                             test cannot answer the question either way.
    --allow-noisy            write the artifact and exit 0 even when the floor
                             is busted. For diagnostics on a machine you know
                             is not quiet — never for a G5 verdict.
    --diagnostic             include the thunder-stripped lane (bare-wire
                             listener, same client) — isolates what the
                             server's features cost from what the wire costs.
                             Never a G5 lane.
    --product-harness        BEN-040 product-level RPC-vs-HTTP harness against
                             the demonstration engine: three scenarios (bulk
                             ingest, small high-QPS call, pipelined polling),
                             the same handler behind both transports, floors
                             SEEDED from Nexus's table (never results). Writes
                             <out>/<label>.json + .md (label default
                             'product-harness-demo'). A real product swaps in
                             its own engine in its own repo.
    --serve-resp3 <port>     BEN-003 calibration: serve the RESP3 lane on
                             0.0.0.0:<port> until killed, so an external
                             client (redis-benchmark) can drive the very same
                             listener the matrix measures. Runs no matrix.
    --help                   print this help

SCENARIOS:
    point-echo-64B, medium-4KiB, pipelined-1k, connection-storm,
    bulk-10k (pending T4.3), embedding-768 (pending T4.3)";

/// How many cores a run pins by default. Four is enough to hold the driver,
/// the listener and their runtime threads without spilling across a hybrid
/// CPU's P/E boundary on a typical desktop, and leaves the rest of the machine
/// for everything else.
const DEFAULT_PIN_CORES: usize = 4;

struct Args {
    scenario: String,
    out: PathBuf,
    cfg: RunConfig,
    label: Option<String>,
    serve_resp3: Option<u16>,
    product_harness: bool,
    diagnostic: bool,
    cores: usize,
    pin: bool,
    noise_floor_pct: f64,
    allow_noisy: bool,
    help: bool,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            scenario: "all".to_owned(),
            out: PathBuf::from("bench-out"),
            cfg: RunConfig::default(),
            label: None,
            serve_resp3: None,
            product_harness: false,
            diagnostic: false,
            cores: DEFAULT_PIN_CORES,
            pin: true,
            noise_floor_pct: DEFAULT_NOISE_FLOOR_PCT,
            allow_noisy: false,
            help: false,
        }
    }
}

fn parse_args(mut argv: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut args = Args::default();
    while let Some(flag) = argv.next() {
        match flag.as_str() {
            "--scenario" => args.scenario = need_value(&mut argv, "--scenario")?,
            "--out" => args.out = PathBuf::from(need_value(&mut argv, "--out")?),
            "--ops" => args.cfg.ops = need_number(&mut argv, "--ops", 1)?,
            "--warmup" => args.cfg.warmup = need_number(&mut argv, "--warmup", 0)?,
            "--reps" => args.cfg.repetitions = need_number(&mut argv, "--reps", 1)?,
            "--label" => args.label = Some(need_value(&mut argv, "--label")?),
            "--cores" => args.cores = need_number(&mut argv, "--cores", 1)?,
            "--no-pin" => args.pin = false,
            "--noise-floor" => args.noise_floor_pct = need_float(&mut argv, "--noise-floor")?,
            "--allow-noisy" => args.allow_noisy = true,
            "--diagnostic" => args.diagnostic = true,
            "--product-harness" => args.product_harness = true,
            "--serve-resp3" => {
                args.serve_resp3 = Some(need_number(&mut argv, "--serve-resp3", 1)? as u16)
            }
            "--help" | "-h" => args.help = true,
            other => return Err(format!("unknown flag '{other}'\n\n{USAGE}")),
        }
    }
    Ok(args)
}

fn need_value(argv: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, String> {
    argv.next()
        .filter(|v| !v.is_empty())
        .ok_or_else(|| format!("{flag} needs a value\n\n{USAGE}"))
}

fn need_number(
    argv: &mut impl Iterator<Item = String>,
    flag: &str,
    min: usize,
) -> Result<usize, String> {
    let raw = need_value(argv, flag)?;
    let value: usize = raw
        .parse()
        .map_err(|_| format!("{flag} needs a number, got '{raw}'"))?;
    if value < min {
        return Err(format!("{flag} must be >= {min}"));
    }
    Ok(value)
}

fn need_float(argv: &mut impl Iterator<Item = String>, flag: &str) -> Result<f64, String> {
    let raw = need_value(argv, flag)?;
    let value: f64 = raw
        .parse()
        .map_err(|_| format!("{flag} needs a number, got '{raw}'"))?;
    if !value.is_finite() || value < 0.0 {
        return Err(format!("{flag} must be a finite number >= 0, got '{raw}'"));
    }
    Ok(value)
}

fn main() -> ExitCode {
    let args = match parse_args(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::from(2);
        }
    };
    if args.help {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    if let Some(port) = args.serve_resp3 {
        return serve_resp3(port);
    }
    if args.product_harness {
        return run_product_harness(&args);
    }
    let selected = match scenarios::select(&args.scenario) {
        Ok(selected) => selected,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::from(2);
        }
    };
    if cfg!(debug_assertions) {
        eprintln!(
            "warning: debug build — BEN-011 requires release runs; \
             the artifact will record build_profile=debug"
        );
    }
    if args.cfg.repetitions < 2 && !args.allow_noisy {
        eprintln!(
            "error: --reps {} cannot be judged — a single repetition has no spread, so the \
             noise floor would pass by vacuum rather than by quiet. Use --reps 2 or more, or \
             --allow-noisy if you are deliberately taking an unjudgeable sample.",
            args.cfg.repetitions
        );
        return ExitCode::from(2);
    }
    let (runtime, pin_report) = match build_runtime(&args) {
        Ok(pair) => pair,
        Err(e) => {
            eprintln!("error: failed to start the tokio runtime: {e}");
            return ExitCode::FAILURE;
        }
    };
    eprintln!("{pin_report}");
    match runtime.block_on(run(&args, &selected, pin_report)) {
        Ok(outcome) => {
            println!("wrote {}", outcome.json_path.display());
            println!("wrote {}", outcome.md_path.display());
            report_noise(&outcome, &args)
        }
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}

/// Build the runtime, pinning each worker thread to one of the chosen cores.
///
/// Pinning happens in `on_thread_start`, which tokio runs **on the worker
/// thread itself** — the only place `set_for_current` can affect it. Cores are
/// handed out round-robin by an atomic counter, so with `--cores 4` and more
/// workers than cores the workers still stay inside the chosen set.
///
/// Every lane shares this runtime, so the pinning applies identically to
/// Thunder and to every peer (BEN-001): it cannot favour a lane, it only
/// removes migration noise from all of them.
fn build_runtime(args: &Args) -> Result<(tokio::runtime::Runtime, PinReport), std::io::Error> {
    let available = pinning::available_cores();
    if !args.pin {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;
        return Ok((rt, PinReport::unpinned(available.len())));
    }
    if available.is_empty() {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;
        return Ok((
            rt,
            PinReport {
                requested: true,
                cores: Vec::new(),
                available: 0,
                failure: Some("the platform reported no core ids".to_owned()),
            },
        ));
    }

    let cores = pinning::choose_cores(&available, args.cores);
    let workers = cores.len();
    // Set by the worker threads; read after the runtime is up. Any worker that
    // fails to pin makes the whole run unpinned — a partially pinned run is
    // exactly the confound pinning exists to remove, so it must not be
    // reported as pinned.
    let failures = Arc::new(AtomicUsize::new(0));
    let next = Arc::new(AtomicUsize::new(0));

    let rt = {
        let cores = cores.clone();
        let failures = Arc::clone(&failures);
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(workers)
            .enable_all()
            .on_thread_start(move || {
                let idx = next.fetch_add(1, Ordering::Relaxed) % cores.len();
                if !pinning::pin_current_thread(cores[idx]) {
                    failures.fetch_add(1, Ordering::Relaxed);
                }
            })
            .build()?
    };
    // Force the workers to start so their pinning result is known before the
    // header is captured — tokio spawns them lazily otherwise.
    rt.block_on(async {
        tokio::task::yield_now().await;
    });

    let failed = failures.load(Ordering::Relaxed);
    let report = PinReport {
        requested: true,
        cores,
        available: available.len(),
        failure: (failed > 0).then(|| {
            format!("{failed} worker thread(s) refused the affinity request; the run is not pinned")
        }),
    };
    Ok((rt, report))
}

/// Judge the run's stability and decide its exit code.
///
/// The artifact is written either way — a noisy run is still evidence, and
/// deleting it would hide *why* the run was refused. What the floor changes is
/// the exit code: a busted floor is a failure, not a footnote.
fn report_noise(outcome: &RunOutcome, args: &Args) -> ExitCode {
    let verdict = noise_check(outcome.noisy_cells.clone(), args.noise_floor_pct);
    if verdict.is_quiet() {
        eprintln!(
            "noise floor: PASS — every cell's qps spread is within {:.1}% across {} repetitions",
            args.noise_floor_pct, args.cfg.repetitions
        );
        return ExitCode::SUCCESS;
    }
    eprintln!(
        "\nnoise floor: FAIL — {} cell(s) exceed {:.1}% qps spread across repetitions:",
        verdict.offenders.len(),
        args.noise_floor_pct
    );
    for cell in verdict.offenders.iter().take(10) {
        eprintln!(
            "  {:>6.1}%  {} / {} d{} c{}",
            cell.spread_pct, cell.scenario, cell.lane, cell.depth, cell.connections
        );
    }
    if verdict.offenders.len() > 10 {
        eprintln!("  … and {} more", verdict.offenders.len() - 10);
    }
    eprintln!(
        "\nThis machine was not quiet enough to answer a BEN-020 (>=10%) question: a cell whose \
         own repetitions disagree by more than the margin under test cannot support a verdict \
         for OR against Thunder. Close other work and re-run, or pass --allow-noisy to keep the \
         artifact as a diagnostic (it may not be cited — BEN-031)."
    );
    if args.allow_noisy {
        eprintln!("--allow-noisy: exiting 0 anyway; this artifact is diagnostic, not a verdict.");
        return ExitCode::SUCCESS;
    }
    ExitCode::FAILURE
}

/// BEN-003 calibration mode: serve the RESP3 lane on `0.0.0.0:<port>` until
/// killed, over the very same [`NoopBackend`] the matrix uses.
///
/// This exists so `redis-benchmark` can drive **the exact listener the matrix
/// measures**, on the same host and allocator — the only way its qps is
/// comparable to this harness's own driver at matching `-P`/`-c`. Binding
/// `0.0.0.0` (not loopback) is deliberate: the calibration client runs in a
/// container and reaches the host from outside.
/// Run the product-harness template (BEN-040) against the demonstration engine
/// and commit the artifact. Numbers are provisional SEED comparisons, not a
/// product verdict — a real product swaps [`DemoEngine`] for its own engine in
/// its own repository (BEN-040's per-product half).
fn run_product_harness(args: &Args) -> ExitCode {
    use thunder_bench::product_harness::{render_json, render_markdown, run_artifact, DemoEngine};

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(e) => {
            eprintln!("error: failed to start the tokio runtime: {e}");
            return ExitCode::FAILURE;
        }
    };
    let artifact = match runtime.block_on(run_artifact(Arc::new(DemoEngine::new()), &args.cfg)) {
        Ok(artifact) => artifact,
        Err(message) => {
            eprintln!("error: {message}");
            return ExitCode::FAILURE;
        }
    };

    let label = args.label.as_deref().unwrap_or("product-harness-demo");
    let json = match render_json(&artifact) {
        Ok(json) => json,
        Err(message) => {
            eprintln!("error: {message}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(e) = std::fs::create_dir_all(&args.out) {
        eprintln!("error: cannot create {}: {e}", args.out.display());
        return ExitCode::FAILURE;
    }
    let json_path = args.out.join(format!("{label}.json"));
    let md_path = args.out.join(format!("{label}.md"));
    if let Err(e) = std::fs::write(&json_path, json) {
        eprintln!("error: cannot write {}: {e}", json_path.display());
        return ExitCode::FAILURE;
    }
    if let Err(e) = std::fs::write(&md_path, render_markdown(&artifact)) {
        eprintln!("error: cannot write {}: {e}", md_path.display());
        return ExitCode::FAILURE;
    }
    println!("wrote {}", json_path.display());
    println!("wrote {}", md_path.display());
    println!(
        "note: floors are SEEDS from Nexus's table, not results — a product recalibrates its own \
         and must not cite these numbers while the shootout substrate is unsettled (BEN-040/031)."
    );
    ExitCode::SUCCESS
}

fn serve_resp3(port: u16) -> ExitCode {
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(e) => {
            eprintln!("error: failed to start the tokio runtime: {e}");
            return ExitCode::FAILURE;
        }
    };
    runtime.block_on(async move {
        let backend = std::sync::Arc::new(thunder_bench::backend::NoopBackend::new());
        let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
        let handle = match thunder_bench::resp3::spawn_resp3_listener(backend, addr).await {
            Ok(handle) => handle,
            Err(e) => {
                eprintln!("error: cannot bind {addr}: {e}");
                return ExitCode::FAILURE;
            }
        };
        println!("RESP3 calibration listener ready on {}", handle.local_addr());
        println!("drive it, e.g.: redis-benchmark -h <host> -p {port} -t ping_mbulk -P 16 -c 4 -n 100000");
        println!("press Ctrl-C to stop and print the server-side counters");
        if tokio::signal::ctrl_c().await.is_err() {
            eprintln!("warning: could not listen for Ctrl-C; serving until killed");
            std::future::pending::<()>().await;
        }
        let snap = handle.snapshot();
        println!(
            "server-side counters: requests={} bytes_in={} bytes_out={}",
            snap.requests, snap.bytes_in, snap.bytes_out
        );
        handle.stop().await;
        ExitCode::SUCCESS
    })
}

/// The G5 lanes, plus the bare-wire diagnostic when `--diagnostic` asks for
/// it. The diagnostic is never in the default set: a lane Thunder "beats" by
/// dropping its own guarantees would be a meaningless win.
fn lane_set(args: &Args) -> &'static [Lane] {
    if args.diagnostic {
        &Lane::ALL_WITH_DIAGNOSTIC
    } else {
        &Lane::ALL
    }
}

/// What a completed run produced: the artifact paths, plus the per-cell qps
/// spreads the noise floor judges.
struct RunOutcome {
    json_path: PathBuf,
    md_path: PathBuf,
    noisy_cells: Vec<NoisyCell>,
}

async fn run(
    args: &Args,
    selected: &[&'static Scenario],
    pin_report: PinReport,
) -> Result<RunOutcome, String> {
    let environment = Environment::capture(pin_report);
    let label = args
        .label
        .clone()
        .unwrap_or_else(|| format!("run-{}", environment.timestamp_unix));
    let targets = spawn_targets()
        .await
        .map_err(|e| format!("failed to spawn listeners: {e}"))?;
    eprintln!(
        "listeners up: thunder={} http={} — {} scenario(s), ops/rep={} warmup={} reps={}",
        targets.thunder.local_addr(),
        targets.http.local_addr(),
        selected.len(),
        args.cfg.ops,
        args.cfg.warmup,
        args.cfg.repetitions,
    );

    let mut cells = Vec::new();
    for scenario in selected {
        if scenario.is_pending() {
            // One declaration row, not one per lane.
            let results = run_scenario(&targets, scenario, Lane::Thunder, &args.cfg).await?;
            for cell in &results {
                eprintln!("{}", cell.one_line());
            }
            cells.extend(results);
            continue;
        }
        for &lane in lane_set(args) {
            let results = run_scenario(&targets, scenario, lane, &args.cfg).await?;
            for cell in &results {
                eprintln!("{}", cell.one_line());
            }
            cells.extend(results);
        }
    }
    targets.stop().await;

    let lanes = lane_set(args)
        .iter()
        .map(|l| l.as_str().to_owned())
        .collect();

    // Judge on qps: it is the metric BEN-020 compares, and unlike p99 it is not
    // dominated by a single outlier repetition. Pending cells carry no
    // measurement, so they have nothing to be noisy about.
    let noisy_cells: Vec<NoisyCell> = cells
        .iter()
        .filter_map(|c| {
            c.qps.map(|d| NoisyCell {
                scenario: c.scenario.clone(),
                lane: c.lane.clone(),
                depth: c.depth,
                connections: c.connections,
                spread_pct: d.spread_pct,
            })
        })
        .collect();

    let artifact = Artifact::new(environment, &args.cfg, selected, lanes, cells);
    let (json_path, md_path) = write_artifact(&artifact, &args.out, &label)
        .map_err(|e| format!("artifact write failed: {e}"))?;
    Ok(RunOutcome {
        json_path,
        md_path,
        noisy_cells,
    })
}

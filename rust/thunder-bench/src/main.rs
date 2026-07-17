//! The shootout CLI — hand-rolled arg parsing, no clap.
//!
//! ```text
//! cargo run -p thunder-bench --release -- --scenario all --out bench-out/
//! ```

use std::path::PathBuf;
use std::process::ExitCode;

use thunder_bench::artifact::{write_artifact, Artifact, Environment};
use thunder_bench::driver::{run_scenario, spawn_targets, Lane, RunConfig};
use thunder_bench::scenarios::{self, Scenario};

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
    --diagnostic             include the thunder-stripped lane (bare-wire
                             listener, same client) — isolates what the
                             server's features cost from what the wire costs.
                             Never a G5 lane.
    --serve-resp3 <port>     BEN-003 calibration: serve the RESP3 lane on
                             0.0.0.0:<port> until killed, so an external
                             client (redis-benchmark) can drive the very same
                             listener the matrix measures. Runs no matrix.
    --help                   print this help

SCENARIOS:
    point-echo-64B, medium-4KiB, pipelined-1k, connection-storm,
    bulk-10k (pending T4.3), embedding-768 (pending T4.3)";

struct Args {
    scenario: String,
    out: PathBuf,
    cfg: RunConfig,
    label: Option<String>,
    serve_resp3: Option<u16>,
    diagnostic: bool,
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
            diagnostic: false,
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
            "--diagnostic" => args.diagnostic = true,
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
    match runtime.block_on(run(&args, &selected)) {
        Ok((json_path, md_path)) => {
            println!("wrote {}", json_path.display());
            println!("wrote {}", md_path.display());
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}

/// BEN-003 calibration mode: serve the RESP3 lane on `0.0.0.0:<port>` until
/// killed, over the very same [`NoopBackend`] the matrix uses.
///
/// This exists so `redis-benchmark` can drive **the exact listener the matrix
/// measures**, on the same host and allocator — the only way its qps is
/// comparable to this harness's own driver at matching `-P`/`-c`. Binding
/// `0.0.0.0` (not loopback) is deliberate: the calibration client runs in a
/// container and reaches the host from outside.
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

async fn run(args: &Args, selected: &[&'static Scenario]) -> Result<(PathBuf, PathBuf), String> {
    let environment = Environment::capture();
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
    let artifact = Artifact::new(environment, &args.cfg, selected, lanes, cells);
    write_artifact(&artifact, &args.out, &label).map_err(|e| format!("artifact write failed: {e}"))
}

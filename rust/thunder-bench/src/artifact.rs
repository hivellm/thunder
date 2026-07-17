//! Committed artifacts (BEN-030): JSON + markdown summary with a
//! machine/environment header (BEN-011), written under `bench-out/` — never
//! left in git-ignored build dirs.
//!
//! The environment header carries OS/arch, logical CPU count, hostname, the
//! exact rustc that built the binary, build profile, a runtime timestamp, and
//! the kernel/governor probes plus the pinning report (see [`crate::pinning`]).
//!
//! Kernel and governor were promised "at T4.3" and shipped as the literal
//! string `"unknown (platform probe lands at T4.3)"` — through T4.3, which
//! closed without them. They are real probes now, and where a probe genuinely
//! cannot answer (a VM with no cpufreq sysfs) the field says *why* rather than
//! deferring to a milestone that has already passed.

use std::io;
use std::path::{Path, PathBuf};

use crate::driver::{CellResult, RunConfig};
use crate::pinning::{self, PinReport};
use crate::scenarios::Scenario;

/// Artifact schema tag — bump when the JSON shape changes.
pub const SCHEMA: &str = "thunder-bench/skeleton-v1";

/// The machine/environment header (BEN-011).
#[derive(Debug, Clone, serde::Serialize)]
pub struct Environment {
    /// Operating system (`std::env::consts::OS`).
    pub os: String,
    /// CPU architecture (`std::env::consts::ARCH`).
    pub arch: String,
    /// Logical CPU count (`std::thread::available_parallelism`).
    pub cpus: usize,
    /// Machine hostname (env `COMPUTERNAME`/`HOSTNAME`, else `unknown`).
    pub hostname: String,
    /// The rustc that compiled this binary (captured at build time).
    pub rustc: String,
    /// `release` or `debug` — BEN-011 requires release runs.
    pub build_profile: String,
    /// Run timestamp, seconds since the Unix epoch (taken at runtime).
    pub timestamp_unix: u64,
    /// The same instant, ISO-8601 UTC.
    pub timestamp_utc: String,
    /// Kernel/OS version ([`crate::pinning::kernel_version`]).
    pub kernel: String,
    /// CPU frequency governor / power policy ([`crate::pinning::governor`]).
    /// A ramping governor makes early repetitions slower for reasons unrelated
    /// to the code, so this belongs next to every number.
    pub governor: String,
    /// What this run actually pinned (BEN-011). Distinguishes a pinned run
    /// from one that asked and failed — the flag alone cannot.
    pub pinning: PinReport,
}

impl Environment {
    /// Capture the header at runtime, recording `pinning` as it happened.
    pub fn capture(pinning: PinReport) -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        Self {
            os: std::env::consts::OS.to_owned(),
            arch: std::env::consts::ARCH.to_owned(),
            cpus: std::thread::available_parallelism().map_or(1, usize::from),
            hostname: std::env::var("COMPUTERNAME")
                .or_else(|_| std::env::var("HOSTNAME"))
                .unwrap_or_else(|_| "unknown".to_owned()),
            rustc: env!("THUNDER_BENCH_RUSTC").to_owned(),
            build_profile: if cfg!(debug_assertions) {
                "debug"
            } else {
                "release"
            }
            .to_owned(),
            timestamp_unix: now,
            timestamp_utc: utc_string(now),
            kernel: pinning::kernel_version(),
            governor: pinning::governor(),
            pinning,
        }
    }
}

/// The run knobs, recorded so every artifact is reproducible.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ArtifactConfig {
    /// Measured ops per repetition per cell.
    pub ops: usize,
    /// Warmup ops per cell (discarded).
    pub warmup: usize,
    /// Repetitions per cell.
    pub repetitions: usize,
    /// Lanes this run measured.
    pub lanes: Vec<String>,
    /// Scenarios this run selected.
    pub scenarios: Vec<String>,
}

/// One complete run: header + config + every cell.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Artifact {
    /// [`SCHEMA`].
    pub schema: String,
    /// Machine/environment header (BEN-011).
    pub environment: Environment,
    /// Run knobs.
    pub config: ArtifactConfig,
    /// Every measured or pending cell.
    pub cells: Vec<CellResult>,
}

impl Artifact {
    /// Assemble one run's artifact.
    pub fn new(
        environment: Environment,
        cfg: &RunConfig,
        scenarios: &[&Scenario],
        lanes: Vec<String>,
        cells: Vec<CellResult>,
    ) -> Self {
        Self {
            schema: SCHEMA.to_owned(),
            environment,
            config: ArtifactConfig {
                ops: cfg.ops,
                warmup: cfg.warmup,
                repetitions: cfg.repetitions,
                lanes,
                scenarios: scenarios.iter().map(|s| s.name.to_owned()).collect(),
            },
            cells,
        }
    }
}

/// Serialize the artifact as pretty JSON.
pub fn render_json(artifact: &Artifact) -> Result<String, String> {
    serde_json::to_string_pretty(artifact)
        .map(|mut s| {
            s.push('\n');
            s
        })
        .map_err(|e| format!("artifact serialization failed: {e}"))
}

/// Render the human summary (BEN-030's md half).
pub fn render_markdown(artifact: &Artifact) -> String {
    let env = &artifact.environment;
    let cfg = &artifact.config;
    let mut md = String::with_capacity(4096);
    md.push_str("# thunder-bench — transport shootout (skeleton, T1.6)\n\n");
    md.push_str(
        "Transport-isolated: one no-op dispatch backend (echo / static-reply / sink), \
         every lane in the same process, host, runtime and allocator (BEN-001).\n\n",
    );
    md.push_str("## Environment\n\n");
    md.push_str("| Field | Value |\n|---|---|\n");
    md.push_str(&format!("| os / arch | {} / {} |\n", env.os, env.arch));
    md.push_str(&format!("| cpus (logical) | {} |\n", env.cpus));
    md.push_str(&format!("| hostname | {} |\n", env.hostname));
    md.push_str(&format!("| rustc | {} |\n", env.rustc));
    md.push_str(&format!("| build profile | {} |\n", env.build_profile));
    md.push_str(&format!(
        "| timestamp | {} (unix {}) |\n",
        env.timestamp_utc, env.timestamp_unix
    ));
    md.push_str(&format!("| kernel | {} |\n", env.kernel));
    md.push_str(&format!("| governor | {} |\n", env.governor));
    md.push_str(&format!("| pinning | {} |\n\n", env.pinning));
    if !env.pinning.is_pinned() {
        md.push_str(
            "> **This run is not pinned.** BEN-011 asks for pinned runs; without pinning the \
             scheduler may migrate the driver and the listener between repetitions, so \
             consecutive repetitions are not measuring the same machine. Read the numbers \
             below as indicative only — they must not settle a BEN-020 (≥10%) question.\n\n",
        );
    }

    md.push_str("## Run config\n\n");
    md.push_str(&format!(
        "ops/rep = {}, warmup = {} (discarded), repetitions = {} — dispersion below is \
         min…max across repetitions (BEN-011). Lanes: {}. Scenarios: {}.\n\n",
        cfg.ops,
        cfg.warmup,
        cfg.repetitions,
        cfg.lanes.join(", "),
        cfg.scenarios.join(", "),
    ));

    md.push_str("## Cells\n\n");
    md.push_str(
        "| Scenario | Lane | Depth | Conns | Ops/rep | p50 µs | p99 µs | QPS | B/op in | B/op out | Status |\n",
    );
    md.push_str("|---|---|---:|---:|---:|---|---|---|---:|---:|---|\n");
    for cell in &artifact.cells {
        let ops = cell.reps.first().map_or(0, |r| r.ops);
        md.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |\n",
            cell.scenario,
            cell.lane,
            cell.depth,
            cell.connections,
            ops,
            fmt_dispersion(cell.p50_us.as_ref(), 1),
            fmt_dispersion(cell.p99_us.as_ref(), 1),
            fmt_dispersion(cell.qps.as_ref(), 0),
            fmt_bytes(cell.bytes_in_per_op),
            fmt_bytes(cell.bytes_out_per_op),
            cell.status,
        ));
    }

    md.push_str(
        "\n## Honesty notes\n\n\
         - **Skeleton scope (T1.6):** lanes = Thunder RPC + HTTP/1.1+JSON only; RESP3 and \
           Bolt peers land at T4.2 (BEN-001). No G5 claim is made from this artifact \
           (BEN-031).\n\
         - **Sweep:** connections {1, 4} of the frozen {1, 4, 16, 64}; the full sweep and \
           the bulk-10k / embedding-768 scenarios land at T4.3 (BEN-010).\n\
         - **Parity (BEN-003):** both lanes keep a continuously full in-flight window per \
           connection (no inter-batch gaps); latency is client-observed, request \
           submission → response fully decoded; bytes/op come from server-side counters \
           on both lanes.\n\
         - **Deep-burst latency semantics:** at depth = burst (pipelined-1k) the lanes \
           stamp differently — the Thunder client's stamp includes waiting behind other \
           slots' frame writes inside `call`, the HTTP sender stamps as its own bytes \
           enter the socket. Compare qps on that row; the stamp point is unified in the \
           T4.2 harness.\n\
         - **HTTP lane:** hand-rolled minimal HTTP/1.1 (`src/http.rs`); a production-grade \
           axum lane may replace it at T4.2 if the parity review demands.\n",
    );
    md
}

/// Write `<label>.json` + `<label>.md` under `out_dir` (BEN-030).
pub fn write_artifact(
    artifact: &Artifact,
    out_dir: &Path,
    label: &str,
) -> io::Result<(PathBuf, PathBuf)> {
    std::fs::create_dir_all(out_dir)?;
    let json = render_json(artifact).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let md = render_markdown(artifact);
    let json_path = out_dir.join(format!("{label}.json"));
    let md_path = out_dir.join(format!("{label}.md"));
    std::fs::write(&json_path, json)?;
    std::fs::write(&md_path, md)?;
    Ok((json_path, md_path))
}

fn fmt_dispersion(d: Option<&crate::stats::Dispersion>, decimals: usize) -> String {
    match d {
        Some(d) => format!(
            "{:.dec$} ({:.dec$}…{:.dec$})",
            d.mean,
            d.min,
            d.max,
            dec = decimals
        ),
        None => "-".to_owned(),
    }
}

fn fmt_bytes(b: Option<f64>) -> String {
    match b {
        Some(b) => format!("{b:.0}"),
        None => "-".to_owned(),
    }
}

/// Seconds since the Unix epoch → ISO-8601 UTC, no external deps
/// (civil-from-days, Hinnant's algorithm).
fn utc_string(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// Days since 1970-01-01 → (year, month, day) in the proleptic Gregorian
/// calendar.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::driver::{pending_cell, CellResult};
    use crate::scenarios;
    use crate::stats::{CellStats, Dispersion};

    fn sample_artifact() -> Artifact {
        let stats = CellStats {
            ops: 100,
            elapsed_secs: 0.5,
            p50_us: 12.0,
            p99_us: 40.0,
            qps: 200.0,
        };
        let cell = CellResult {
            scenario: "point-echo-64B".to_owned(),
            lane: "thunder".to_owned(),
            depth: 1,
            connections: 1,
            status: "ok".to_owned(),
            reps: vec![stats, stats],
            p50_us: Some(Dispersion {
                min: 12.0,
                mean: 12.0,
                max: 12.0,
                spread_pct: 0.0,
            }),
            p99_us: Some(Dispersion {
                min: 40.0,
                mean: 40.0,
                max: 40.0,
                spread_pct: 0.0,
            }),
            qps: Some(Dispersion {
                min: 200.0,
                mean: 200.0,
                max: 200.0,
                spread_pct: 0.0,
            }),
            bytes_in_per_op: Some(90.0),
            bytes_out_per_op: Some(88.0),
        };
        let pending = pending_cell(scenarios::find("bulk-10k").unwrap());
        let selected = scenarios::select("point-echo-64B,bulk-10k").unwrap();
        Artifact::new(
            Environment::capture(PinReport::unpinned(8)),
            &crate::driver::RunConfig {
                ops: 100,
                warmup: 10,
                repetitions: 2,
            },
            &selected,
            vec!["thunder".to_owned(), "http".to_owned()],
            vec![cell, pending],
        )
    }

    #[test]
    fn environment_capture_is_sane() {
        let env = Environment::capture(PinReport::unpinned(8));
        assert!(env.cpus >= 1);
        assert!(!env.os.is_empty());
        assert!(!env.rustc.is_empty());
        assert!(env.timestamp_unix > 1_700_000_000, "clock looks wrong");
        assert!(env.timestamp_utc.ends_with('Z'));
    }

    #[test]
    fn json_round_trips_the_schema() {
        let artifact = sample_artifact();
        let json = render_json(&artifact).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["schema"], SCHEMA);
        assert_eq!(parsed["config"]["ops"], 100);
        assert_eq!(parsed["cells"][0]["scenario"], "point-echo-64B");
        assert_eq!(parsed["cells"][0]["reps"][0]["p50_us"], 12.0);
        assert_eq!(parsed["cells"][0]["bytes_in_per_op"], 90.0);
        assert!(parsed["cells"][1]["status"]
            .as_str()
            .unwrap()
            .starts_with("pending"));
        assert!(parsed["environment"]["cpus"].as_u64().unwrap() >= 1);
    }

    #[test]
    fn markdown_carries_header_table_and_honesty_notes() {
        let artifact = sample_artifact();
        let md = render_markdown(&artifact);
        assert!(md.contains("## Environment"));
        assert!(md.contains("| rustc |"));
        assert!(md.contains("| governor |"));
        assert!(md.contains("| point-echo-64B | thunder | 1 | 1 |"));
        assert!(md.contains("12.0 (12.0…12.0)"));
        assert!(md.contains("pending — lands at T4.3"));
        assert!(md.contains("Honesty notes"));
        // The note must name the lanes that are still missing (BEN-031).
        assert!(md.contains("RESP3 and Bolt peers land at T4.2"));
    }

    #[test]
    fn write_artifact_emits_both_files() {
        let artifact = sample_artifact();
        let dir = std::env::temp_dir().join(format!(
            "thunder-bench-test-{}-{}",
            std::process::id(),
            artifact.environment.timestamp_unix
        ));
        let (json_path, md_path) = write_artifact(&artifact, &dir, "unit").unwrap();
        assert!(json_path.exists());
        assert!(md_path.exists());
        assert_eq!(json_path.file_name().unwrap(), "unit.json");
        assert_eq!(md_path.file_name().unwrap(), "unit.md");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn civil_from_days_matches_known_dates() {
        assert_eq!(utc_string(0), "1970-01-01T00:00:00Z");
        assert_eq!(utc_string(86_399), "1970-01-01T23:59:59Z");
        // The 1.7 billion second landmark.
        assert_eq!(utc_string(1_700_000_000), "2023-11-14T22:13:20Z");
        // Leap-year day.
        assert_eq!(utc_string(1_709_164_800), "2024-02-29T00:00:00Z");
    }
}

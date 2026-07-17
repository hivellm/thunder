//! In-process smoke test: the echo scenario at tiny N against both
//! listeners, end to end through the parity driver (BEN-001/003 skeleton
//! proof).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use thunder_bench::artifact::{render_json, render_markdown, Artifact, Environment};
use thunder_bench::driver::{run_scenario, spawn_targets, Lane, RunConfig};
use thunder_bench::pinning::PinReport;
use thunder_bench::scenarios;

fn tiny() -> RunConfig {
    RunConfig {
        ops: 32,
        warmup: 8,
        repetitions: 2,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn echo_scenario_runs_on_every_lane() {
    let targets = spawn_targets().await.unwrap();
    let scenario = scenarios::find("point-echo-64B").unwrap();
    let cfg = tiny();

    let mut all = Vec::new();
    for lane in Lane::ALL {
        let cells = run_scenario(&targets, scenario, lane, &cfg).await.unwrap();
        // Skeleton sweep: depths {1, 16} x connections {1, 4} = 4 cells.
        assert_eq!(cells.len(), 4, "lane {}", lane.as_str());
        for cell in &cells {
            assert_eq!(cell.status, "ok");
            assert_eq!(cell.lane, lane.as_str());
            assert_eq!(cell.reps.len(), cfg.repetitions);
            for rep in &cell.reps {
                assert!(rep.ops > 0, "{cell:?}");
                assert!(rep.p50_us > 0.0, "{cell:?}");
                assert!(rep.p99_us >= rep.p50_us, "{cell:?}");
                assert!(rep.qps > 0.0, "{cell:?}");
            }
            let p50 = cell.p50_us.unwrap();
            assert!(p50.min <= p50.mean && p50.mean <= p50.max);
            // Server-side byte accounting: a 64 B echo costs more than its
            // payload on the wire in both lanes, and both directions flow.
            assert!(cell.bytes_in_per_op.unwrap() > 64.0, "{cell:?}");
            assert!(cell.bytes_out_per_op.unwrap() > 0.0, "{cell:?}");
        }
        all.extend(cells);
    }

    // The artifact renders end to end from a real run.
    let selected = scenarios::select("point-echo-64B").unwrap();
    let artifact = Artifact::new(
        Environment::capture(PinReport::unpinned(8)),
        &cfg,
        &selected,
        Lane::ALL.iter().map(|l| l.as_str().to_owned()).collect(),
        all,
    );
    let json = render_json(&artifact).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(
        parsed["cells"].as_array().unwrap().len(),
        4 * Lane::ALL.len(),
        "every lane reports all four point-echo cells"
    );
    let md = render_markdown(&artifact);
    for lane in Lane::ALL {
        assert!(
            md.contains(&format!("| point-echo-64B | {} |", lane.as_str())),
            "markdown must carry the {} lane",
            lane.as_str()
        );
    }

    targets.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn pending_scenarios_report_without_measuring() {
    let targets = spawn_targets().await.unwrap();
    let scenario = scenarios::find("bulk-10k").unwrap();
    let cells = run_scenario(&targets, scenario, Lane::Thunder, &tiny())
        .await
        .unwrap();
    assert_eq!(cells.len(), 1);
    assert!(cells[0].status.contains("pending"));
    assert!(cells[0].reps.is_empty());
    assert!(cells[0].p50_us.is_none());
    targets.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn connection_storm_runs_tiny_on_every_lane() {
    let targets = spawn_targets().await.unwrap();
    let scenario = scenarios::find("connection-storm").unwrap();
    let cfg = RunConfig {
        ops: 4, // clamps the storm to 4 connects per repetition
        warmup: 2,
        repetitions: 1,
    };
    for lane in Lane::ALL {
        let cells = run_scenario(&targets, scenario, lane, &cfg).await.unwrap();
        assert_eq!(cells.len(), 1, "lane {}", lane.as_str());
        let cell = &cells[0];
        assert_eq!(cell.status, "ok");
        assert_eq!(cell.connections, 4);
        assert_eq!(cell.reps[0].ops, 4);
        assert!(cell.p50_us.unwrap().mean > 0.0);
    }
    targets.stop().await;
}

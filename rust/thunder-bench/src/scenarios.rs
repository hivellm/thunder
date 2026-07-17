//! The scenario matrix **as data** (BEN-010, FR-71).
//!
//! Every BEN-010 row is declared here, including the two the skeleton
//! cannot measure yet (bulk-10k and embedding-768 — marked
//! [`Workload::Pending`], they land with the full harness at T4.3). The
//! frozen full sweep is depths [`DEPTHS`] × connections
//! [`FULL_CONNECTIONS`]; skeleton runs sweep the
//! [`SKELETON_CONNECTIONS`] subset (T1.6 scope — the full sweep is a
//! T4.3 deliverable).

/// Pipeline depths of the frozen matrix (BEN-010): {1, 16}.
pub const DEPTHS: &[usize] = &[1, 16];

/// Connection counts of the frozen matrix (BEN-010): {1, 4, 16, 64}.
/// The full sweep runs at T4.3.
pub const FULL_CONNECTIONS: &[usize] = &[1, 4, 16, 64];

/// Connection counts the T1.6 skeleton sweeps: {1, 4}.
pub const SKELETON_CONNECTIONS: &[usize] = &[1, 4];

/// What one scenario asks of the transport.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Workload {
    /// `ECHO` with a payload of exactly `payload_bytes`.
    Echo {
        /// Request payload size in bytes.
        payload_bytes: usize,
    },
    /// `STATIC` — the fixed 4 KiB reply
    /// ([`crate::backend::STATIC_REPLY_BYTES`]).
    StaticReply,
    /// `requests` echo calls kept continuously in flight per connection
    /// (the pipeline window *is* the burst size — depth is pinned).
    PipelinedBurst {
        /// In-flight window per connection.
        requests: usize,
    },
    /// connect + first-byte, repeated `connections` times sequentially.
    /// Skeleton runs clamp the count to `--ops` so tiny runs stay tiny.
    ConnectionStorm {
        /// Storm size of the frozen matrix definition.
        connections: usize,
    },
    /// Declared but not measurable by the skeleton; lands at `lands_at`.
    Pending {
        /// DAG task that delivers this scenario.
        lands_at: &'static str,
    },
}

/// One row of the BEN-010 matrix.
#[derive(Debug, Clone, Copy)]
pub struct Scenario {
    /// Stable scenario name — artifact key, `--scenario` selector.
    pub name: &'static str,
    /// What the scenario probes (BEN-010 table).
    pub probe: &'static str,
    /// The workload definition.
    pub workload: Workload,
}

impl Scenario {
    /// `(pipeline depth, connections)` cells this scenario runs in the
    /// skeleton sweep. Pending scenarios have no cells.
    pub fn cells(&self) -> Vec<(usize, usize)> {
        match self.workload {
            Workload::Echo { .. } | Workload::StaticReply => DEPTHS
                .iter()
                .flat_map(|&d| SKELETON_CONNECTIONS.iter().map(move |&c| (d, c)))
                .collect(),
            Workload::PipelinedBurst { requests } => SKELETON_CONNECTIONS
                .iter()
                .map(|&c| (requests, c))
                .collect(),
            // The storm is its own cell: sequential connects, depth 1.
            Workload::ConnectionStorm { .. } => vec![(1, 1)],
            Workload::Pending { .. } => vec![],
        }
    }

    /// `true` for scenarios the skeleton declares but cannot measure.
    pub fn is_pending(&self) -> bool {
        matches!(self.workload, Workload::Pending { .. })
    }
}

/// The frozen BEN-010 rows. Names are stable artifact keys — changing one
/// invalidates release-over-release comparison (BEN-022).
pub const SCENARIOS: &[Scenario] = &[
    Scenario {
        name: "point-echo-64B",
        probe: "per-frame overhead floor",
        workload: Workload::Echo { payload_bytes: 64 },
    },
    Scenario {
        name: "medium-4KiB",
        probe: "typical document/search response",
        workload: Workload::StaticReply,
    },
    Scenario {
        name: "bulk-10k",
        probe: "10k-item bulk reply — the family's strongest mandated row",
        workload: Workload::Pending { lands_at: "T4.3" },
    },
    Scenario {
        name: "embedding-768",
        probe: "768xf32 embedding — payload encodings, measured",
        workload: Workload::Pending { lands_at: "T4.3" },
    },
    Scenario {
        name: "pipelined-1k",
        probe: "pipelining on merit",
        workload: Workload::PipelinedBurst { requests: 1000 },
    },
    Scenario {
        name: "connection-storm",
        probe: "connect + first-byte handshake cost",
        workload: Workload::ConnectionStorm { connections: 1000 },
    },
];

/// Look one scenario up by its stable name.
pub fn find(name: &str) -> Option<&'static Scenario> {
    SCENARIOS.iter().find(|s| s.name == name)
}

/// Every stable scenario name, matrix order.
pub fn names() -> Vec<&'static str> {
    SCENARIOS.iter().map(|s| s.name).collect()
}

/// Parse a `--scenario` selection: `all`, or a comma-separated list of
/// names. Unknown names fail with the full catalog in the message.
pub fn select(arg: &str) -> Result<Vec<&'static Scenario>, String> {
    if arg.eq_ignore_ascii_case("all") {
        return Ok(SCENARIOS.iter().collect());
    }
    let mut selected = Vec::new();
    for name in arg.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let scenario = find(name).ok_or_else(|| {
            format!(
                "unknown scenario '{name}' — known scenarios: {} (or 'all')",
                names().join(", ")
            )
        })?;
        selected.push(scenario);
    }
    if selected.is_empty() {
        return Err("--scenario needs at least one scenario name, or 'all'".to_owned());
    }
    Ok(selected)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn matrix_declares_every_ben_010_row() {
        assert_eq!(
            names(),
            vec![
                "point-echo-64B",
                "medium-4KiB",
                "bulk-10k",
                "embedding-768",
                "pipelined-1k",
                "connection-storm",
            ]
        );
    }

    #[test]
    fn select_all_returns_the_whole_matrix() {
        assert_eq!(select("all").unwrap().len(), SCENARIOS.len());
        assert_eq!(select("ALL").unwrap().len(), SCENARIOS.len());
    }

    #[test]
    fn select_parses_a_comma_list() {
        let picked = select("point-echo-64B, medium-4KiB").unwrap();
        assert_eq!(picked.len(), 2);
        assert_eq!(picked[0].name, "point-echo-64B");
        assert_eq!(picked[1].name, "medium-4KiB");
    }

    #[test]
    fn select_rejects_unknown_names_listing_the_catalog() {
        let err = select("nope").unwrap_err();
        assert!(err.contains("unknown scenario 'nope'"), "{err}");
        assert!(err.contains("point-echo-64B"), "{err}");
    }

    #[test]
    fn select_rejects_empty_selection() {
        assert!(select("").is_err());
        assert!(select(" , ").is_err());
    }

    #[test]
    fn echo_cells_cross_depths_and_skeleton_connections() {
        let echo = find("point-echo-64B").unwrap();
        assert_eq!(echo.cells(), vec![(1, 1), (1, 4), (16, 1), (16, 4)]);
    }

    #[test]
    fn pipelined_burst_pins_depth_to_the_burst() {
        let pipelined = find("pipelined-1k").unwrap();
        assert_eq!(pipelined.cells(), vec![(1000, 1), (1000, 4)]);
    }

    #[test]
    fn storm_is_a_single_cell_and_pending_rows_have_none() {
        assert_eq!(find("connection-storm").unwrap().cells(), vec![(1, 1)]);
        assert!(find("bulk-10k").unwrap().cells().is_empty());
        assert!(find("bulk-10k").unwrap().is_pending());
        assert!(find("embedding-768").unwrap().is_pending());
    }

    #[test]
    fn frozen_sweep_constants_match_ben_010() {
        assert_eq!(DEPTHS, &[1, 16]);
        assert_eq!(FULL_CONNECTIONS, &[1, 4, 16, 64]);
        assert_eq!(SKELETON_CONNECTIONS, &[1, 4]);
    }
}

//! Thunder RPC server — accept loop, writer task, dispatch trait.
//!
//! Skeleton crate: the server contract is specified in
//! `docs/specs/SPEC-004-server.md` (hot path from the Synap listener per
//! the §7 baseline analysis) and lands at DAG task T1.5
//! (`phase1_thunder-server`).

pub use thunder_wire as wire;

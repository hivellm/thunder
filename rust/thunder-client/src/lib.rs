//! Thunder RPC client — multiplexed, profile-driven.
//!
//! Skeleton crate: the client contract is specified in
//! `docs/specs/SPEC-003-client.md` and lands at DAG task T1.4
//! (`phase1_thunder-client`). The wire layer it will consume is
//! [`thunder_wire`].

pub use thunder_wire as wire;

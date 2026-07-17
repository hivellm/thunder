//! Transport shootout harness (SPEC-007, `BEN-xxx`) — **skeleton scope,
//! DAG T1.6**.
//!
//! Two principles govern everything here (SPEC-007 preamble):
//!
//! - **isolate the transport** — one no-op dispatch backend
//!   ([`backend::NoopBackend`]: echo / static-reply / sink, zero storage,
//!   zero business logic) is served by every listener in the same process,
//!   on the same host, runtime and allocator, so the transport is the only
//!   thing measured (BEN-001);
//! - **harness parity** — one driver shape per protocol with identical
//!   concurrency model and measurement points: continuous pipelining with
//!   no inter-batch gaps (the Synap `-P 16` lesson, BEN-003), warmup
//!   discarded, N repetitions with dispersion reported (BEN-011).
//!
//! # Skeleton scope
//!
//! The skeleton hosts **two** of the four BEN-001 lanes:
//!
//! | Lane | Listener |
//! |---|---|
//! | `thunder` | [`thunder::server::spawn_listener`] over the no-op backend |
//! | `http` | hand-rolled minimal HTTP/1.1 + JSON ([`http`]) over the same backend |
//!
//! RESP3 and Bolt lanes land at T4.2, together with the full connection
//! sweep {1, 4, 16, 64} and the bulk-10k / embedding-768 scenarios
//! (declared as data today, marked pending — [`scenarios`]).
//!
//! Results are written as committed artifacts under `bench-out/` — JSON +
//! markdown summary with a machine/environment header (BEN-030,
//! [`artifact`]).
//!
//! Run it:
//!
//! ```text
//! cargo run -p thunder-bench --release -- --scenario all --out bench-out/
//! ```

pub mod artifact;
pub mod backend;
pub mod driver;
pub mod http;
pub mod scenarios;
pub mod stats;

pub use driver::bench_profile;
pub use thunder::wire;

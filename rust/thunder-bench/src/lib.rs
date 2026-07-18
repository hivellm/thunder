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
//! # Lanes
//!
//! All four BEN-001 lanes are live, every one served by the same no-op
//! backend in the same process:
//!
//! | Lane | Listener |
//! |---|---|
//! | `thunder` | [`thunder::server::spawn_listener`] over the no-op backend |
//! | `resp3` | RESP3 peer ([`resp3`]) — the Redis/Synap convention |
//! | `bolt` | minimal Bolt v5 peer ([`bolt`]) — the Neo4j competitor |
//! | `http` | hand-rolled minimal HTTP/1.1 + JSON ([`http`]) over the same backend |
//!
//! Each peer implements exactly the subset the BEN-010 matrix needs and
//! documents that scope in its module docs — a benchmark peer, not a
//! product (BEN-002).
//!
//! **The RESP3 lane's `redis-benchmark` calibration (BEN-003) is UNRUN**:
//! its numbers must not be trusted at G5 until it is run — see [`resp3`].
//!
//! Still pending: the full connection sweep {1, 4, 16, 64} and the
//! bulk-10k / embedding-768 scenarios (declared as data, marked pending —
//! [`scenarios`]).
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
pub mod bolt;
pub mod driver;
pub mod grpc;
pub mod http;
pub mod memcached;
pub mod mongodb;
pub mod msgpack_rpc;
pub mod pinning;
pub mod postgres;
pub mod product_harness;
pub mod resp3;
pub mod scenarios;
pub mod stats;
pub mod stripped;
pub mod thrift_lane;

pub use driver::bench_profile;
pub use thunder::wire;

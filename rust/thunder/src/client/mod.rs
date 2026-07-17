//! Thunder RPC client — multiplexed, config-driven (SPEC-003, `CLT-xxx`).
//!
//! One [`Client`] owns one TCP connection and multiplexes concurrent
//! calls over it via a background reader task (CLT-001/010). Behavior is
//! driven entirely by a [`Config`] (SPEC-002): handshake style, push
//! policy, frame cap, in-flight bound, and error-string convention.
//!
//! The application supplies that config — Thunder has no product registry
//! to look one up in. Start from [`Config::standard`] and set your
//! identity; override a dimension only where you actually diverge.
//!
//! ```no_run
//! use thunder::{Client, ClientConfig, Config, Value};
//!
//! # async fn demo() -> Result<(), thunder::ClientError> {
//! let app = Config::standard().scheme("myapp").port(9000);
//! let config = ClientConfig::new().api_key("secret").client_name("demo");
//! let client = Client::connect_with("myapp://localhost", app, config).await?;
//! let pong = client.call("PING", vec![]).await?;
//! assert_eq!(pong.as_str(), Some("PONG"));
//! client.close().await;
//! # Ok(())
//! # }
//! ```
//!
//! The semantics here are the family's "uniform floor" (PRD NFR-07): the
//! same contract ships in TypeScript, Python, and C#. Error classes on
//! [`ClientError`] are stable public API (CLT-052) — branch on the class
//! and `code`, never on message text.

mod conn;
mod endpoint;
mod error;

pub use conn::{Client, ClientConfig, Credentials, HandshakeInfo};
pub use endpoint::{parse_endpoint, Endpoint};
pub use error::ClientError;

// The wire layer lives at `thunder::wire`; re-export the two types that
// surface in this module's public API for ergonomics.
pub use crate::wire::{Config, Value};

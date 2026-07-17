//! The product integration surface (SRV-020..022): one trait, three hooks.
//!
//! Command routing, argument extraction and business logic are product-side
//! (SRV-020); credential validation is product code — Thunder owns the
//! handshake state machine, never the credential store (SRV-012). Command
//! name matching is byte-exact pass-through: case policy lives inside the
//! product's `dispatch` (SRV-022).

use std::future::Future;

use thunder_wire::Value;

use crate::session::Session;

/// Credentials parsed by Thunder from `HELLO`/`AUTH` payloads (SRV-012).
///
/// - `AUTH <api_key>` → [`Credentials::ApiKey`] (Nexus single-arg form)
/// - `AUTH <user> <pass>` → [`Credentials::UserPass`]
/// - `HELLO {token: …}` → [`Credentials::Token`] (Vectorizer/Lexum map)
/// - `HELLO {api_key: …}` → [`Credentials::ApiKey`]
/// - `HELLO {}` / missing map → [`Credentials::None`] — products with auth
///   disabled accept it; everyone else rejects it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Credentials {
    /// A bare API key.
    ApiKey(String),
    /// Username + password.
    UserPass(String, String),
    /// A bearer token from a `MapPayload` HELLO.
    Token(String),
    /// No credentials supplied.
    None,
}

/// The identity a successful [`Dispatch::authenticate`] resolves to. Stored
/// on the [`Session`] and fed to [`Dispatch::capabilities`] for the HELLO
/// reply (SRV-014).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Principal {
    /// Product-defined principal name (user, key id, …).
    pub name: String,
}

/// Authentication failure from the product hook (SRV-012). Thunder maps it
/// to the profile's error convention before it reaches the wire (SRV-021).
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    /// Credentials failed validation. Rendered as the family's
    /// `WRONGPASS …` string under `Resp3Prefixes`, `[unauthorized] …`
    /// under `BracketCode`/`Both`.
    #[error("invalid credentials")]
    InvalidCredentials,
    /// Product-specific failure; the message travels verbatim (WIRE-040).
    #[error("{0}")]
    Message(String),
}

/// Product integration is exactly this trait (SRV-020).
///
/// Declared with return-position `impl Future + Send` so implementors can
/// write plain `async fn` (no `async-trait` dependency) while the listener
/// can still spawn dispatch futures onto the runtime. The listener is
/// generic over `D: Dispatch`, so object safety is not required.
pub trait Dispatch: Send + Sync + 'static {
    /// Run one command. The error `String` travels verbatim on the wire
    /// (SRV-021, WIRE-040); a returned `Err` never closes the connection
    /// (SRV-005).
    fn dispatch(
        &self,
        session: &Session,
        command: &str,
        args: Vec<Value>,
    ) -> impl Future<Output = Result<Value, String>> + Send;

    /// Validate credentials parsed from `HELLO`/`AUTH` (SRV-012). Thunder
    /// flips the session's auth flag on `Ok` — product code never touches
    /// the state machine.
    fn authenticate(
        &self,
        creds: Credentials,
    ) -> impl Future<Output = Result<Principal, AuthError>> + Send;

    /// Capability names advertised in `MapPayload` HELLO replies
    /// (SRV-014). Defaults to none.
    fn capabilities(&self, principal: &Principal) -> Vec<String> {
        let _ = principal;
        vec![]
    }
}

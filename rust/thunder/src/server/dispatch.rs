//! The product integration surface (SRV-020..022): one trait, three hooks.
//!
//! Command routing, argument extraction and business logic are product-side
//! (SRV-020); credential validation is product code — Thunder owns the
//! handshake state machine, never the credential store (SRV-012). Command
//! name matching is byte-exact pass-through: case policy lives inside the
//! product's `dispatch` (SRV-022).

use std::future::Future;

use crate::wire::Value;

use crate::server::session::Session;

/// Credentials parsed by Thunder from `HELLO`/`AUTH` payloads (SRV-012).
///
/// - `AUTH <api_key>` → [`Credentials::ApiKey`] (single-arg form)
/// - `AUTH <user> <pass>` → [`Credentials::UserPass`]
/// - `HELLO {token: …}` → [`Credentials::Token`] (map payload)
/// - `HELLO {api_key: …}` → [`Credentials::ApiKey`]
/// - `HELLO {}` / missing map → [`Credentials::None`] — a deployment with
///   `auth_required = false` accepts it; everyone else rejects it.
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
pub struct Principal<I = ()> {
    /// Product-defined principal name (user, key id, …).
    pub name: String,
    /// The product's own resolved identity — roles, permissions, quotas,
    /// tenant, whatever authorization actually needs.
    ///
    /// Before this existed a product could only carry the *name*, so every
    /// privileged command had to re-resolve the user from its credential
    /// store. That was not merely a cost: the second lookup reads live state,
    /// so a user edited or deleted mid-session was evaluated against the new
    /// record. Carrying the identity here restores the other semantics —
    /// **captured at `AUTH`, stable for the session** — and makes the choice
    /// the product's rather than an accident of the transport.
    ///
    /// Defaults to `()` for products that only need the name.
    pub identity: I,
}

impl Principal {
    /// A principal carrying only a name — the `Identity = ()` case.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            identity: (),
        }
    }
}

impl<I> Principal<I> {
    /// A principal carrying the product's resolved identity.
    pub fn with_identity(name: impl Into<String>, identity: I) -> Self {
        Self {
            name: name.into(),
            identity,
        }
    }
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
/// Declared with return-position `impl Future + Send` so implementers can
/// write plain `async fn` (no `async-trait` dependency) while the listener
/// can still spawn dispatch futures onto the runtime. The listener is
/// generic over `D: Dispatch`, so object safety is not required.
pub trait Dispatch: Send + Sync + 'static {
    /// The product's own identity payload, resolved once at `AUTH` and
    /// carried on the session (SRV-012).
    ///
    /// Write `type Identity = ();` when the principal's name is all the
    /// product needs. Rust has no stable associated-type defaults, so the
    /// line is required even in that case — the ergonomics live on
    /// [`Principal`] and [`Session`], which both default their parameter
    /// to `()`.
    type Identity: Send + Sync + 'static;

    /// Run one command. The error `String` travels verbatim on the wire
    /// (SRV-021, WIRE-040); a returned `Err` never closes the connection
    /// (SRV-005).
    fn dispatch(
        &self,
        session: &Session<Self::Identity>,
        command: &str,
        args: Vec<Value>,
    ) -> impl Future<Output = Result<Value, String>> + Send;

    /// Validate credentials parsed from `HELLO`/`AUTH` (SRV-012). Thunder
    /// flips the session's auth flag on `Ok` — product code never touches
    /// the state machine.
    fn authenticate(
        &self,
        creds: Credentials,
    ) -> impl Future<Output = Result<Principal<Self::Identity>, AuthError>> + Send;

    /// Capability names advertised in `MapPayload` HELLO replies
    /// (SRV-014). Defaults to none.
    fn capabilities(&self, principal: &Principal<Self::Identity>) -> Vec<String> {
        let _ = principal;
        vec![]
    }
}

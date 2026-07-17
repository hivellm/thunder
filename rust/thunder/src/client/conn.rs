//! The multiplexed Thunder client (SPEC-003).
//!
//! One [`Client`] owns one TCP connection (CLT-001; pooling is a layer
//! above, CLT-080) and demultiplexes concurrent in-flight calls over it:
//!
//! - ids are monotonically increasing `u32`s skipping [`PUSH_ID`]
//!   (CLT-010);
//! - a background tokio reader task routes each response to its caller's
//!   `oneshot` channel by id (CLT-010), drops unknown ids (CLT-013), and
//!   poisons the connection on malformed / oversized frames — every
//!   pending call fails with the same typed error (CLT-014);
//! - writes are serialized behind an async mutex so frames never
//!   interleave (CLT-011);
//! - in-flight calls are bounded by the profile's `max_in_flight` via a
//!   semaphore — excess calls wait, they are not refused (CLT-012);
//! - per-call timeouts remove the pending entry so a late response falls
//!   under the unknown-id drop (CLT-020);
//! - when a call finds the connection dead, the client lazily re-dials
//!   and re-handshakes up to 2 attempts with capped backoff; calls that
//!   were pending when the connection died fail typed and are never
//!   replayed (CLT-030/031);
//! - frames with `id == PUSH_ID` go to the registered push handler under
//!   `PushPolicy::Enabled` and poison the connection under `Reserved`
//!   (CLT-060).
//!
//! The demux architecture follows the family's best client (the
//! Vectorizer Rust SDK reader-task + oneshot-map pattern).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex, MutexGuard, PoisonError};
use std::time::Duration;

use tokio::io::{AsyncWriteExt, BufReader};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::TcpStream;
use tokio::sync::{oneshot, Mutex as TokioMutex, Semaphore};
use tokio::task::JoinHandle;

use crate::wire::profile::{Handshake, HelloStyle, PushPolicy};
use crate::wire::{
    read_response_with_limit, write_request, Profile, Request, Response, Value, PUSH_ID,
};

use crate::client::endpoint::{parse_endpoint, Endpoint};
use crate::client::error::ClientError;

/// Reconnect backoff: first re-dial retries after `BACKOFF_BASE`, doubling
/// up to `BACKOFF_CAP` (CLT-030 "capped backoff").
const BACKOFF_BASE: Duration = Duration::from_millis(50);
const BACKOFF_CAP: Duration = Duration::from_millis(500);

/// Re-dial budget when a call finds the connection dead (CLT-030).
const RECONNECT_ATTEMPTS: u32 = 2;

/// Credentials for the profile handshake (CLT-002). Auth state is
/// per-connection and sticky — there are no per-call credentials
/// (CLT-003).
#[derive(Debug, Clone)]
pub enum Credentials {
    /// Bearer token (`token` key under `HelloMandatory`).
    Token(String),
    /// API key (`api_key` key under `HelloMandatory`, single-arg `AUTH`
    /// under `AuthCommand`).
    ApiKey(String),
    /// User + password (`AUTH [user, pass]` under `AuthCommand`).
    UserPass {
        /// User name.
        user: String,
        /// Password.
        pass: String,
    },
}

/// Client configuration: connect timeout default **10 s** (CLT-001),
/// per-call timeout default **30 s** (CLT-020), optional credentials and
/// client name for the handshake (CLT-002).
#[derive(Debug, Clone)]
pub struct ClientConfig {
    /// TCP connect timeout (CLT-001). Default 10 s.
    pub connect_timeout: Duration,
    /// Default per-call timeout (CLT-020); override per call with
    /// [`Client::call_with_timeout`]. Default 30 s.
    pub call_timeout: Duration,
    /// Handshake credentials, when the profile wants them.
    pub credentials: Option<Credentials>,
    /// Client identifier sent in the `HELLO` map (`HelloMandatory`).
    pub client_name: Option<String>,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_secs(10),
            call_timeout: Duration::from_secs(30),
            credentials: None,
            client_name: None,
        }
    }
}

impl ClientConfig {
    /// Defaults: 10 s connect, 30 s per call, no credentials.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the connect timeout (CLT-001).
    #[must_use]
    pub fn connect_timeout(mut self, timeout: Duration) -> Self {
        self.connect_timeout = timeout;
        self
    }

    /// Set the default per-call timeout (CLT-020).
    #[must_use]
    pub fn call_timeout(mut self, timeout: Duration) -> Self {
        self.call_timeout = timeout;
        self
    }

    /// Authenticate with a bearer token.
    #[must_use]
    pub fn token(mut self, token: impl Into<String>) -> Self {
        self.credentials = Some(Credentials::Token(token.into()));
        self
    }

    /// Authenticate with an API key.
    #[must_use]
    pub fn api_key(mut self, api_key: impl Into<String>) -> Self {
        self.credentials = Some(Credentials::ApiKey(api_key.into()));
        self
    }

    /// Authenticate with user + password (`AuthCommand` profiles).
    #[must_use]
    pub fn user_pass(mut self, user: impl Into<String>, pass: impl Into<String>) -> Self {
        self.credentials = Some(Credentials::UserPass {
            user: user.into(),
            pass: pass.into(),
        });
        self
    }

    /// Set the client name announced in the `HELLO` map.
    #[must_use]
    pub fn client_name(mut self, name: impl Into<String>) -> Self {
        self.client_name = Some(name.into());
        self
    }
}

/// What the handshake learned about this connection (CLT-002).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HandshakeInfo {
    /// `true` once the server accepted the credentials (`AUTH` succeeded
    /// or the `HELLO` reply said so).
    pub authenticated: bool,
    /// Capability names from the `HELLO` reply (`HelloMandatory`).
    pub capabilities: Vec<String>,
}

/// Registered push handler (CLT-060). Runs on the reader task — keep it
/// fast and offload real work to a channel.
type PushHandler = Arc<dyn Fn(Value) + Send + Sync>;

type PendingTx = oneshot::Sender<Result<Response, ClientError>>;

/// State shared between one connection's caller side and its reader task.
struct ConnShared {
    /// id → oneshot sender demux map (CLT-010).
    pending: StdMutex<HashMap<u32, PendingTx>>,
    /// Cleared when the connection is poisoned or closed.
    alive: AtomicBool,
}

impl ConnShared {
    /// Poison: mark dead and fail every pending call with the same typed
    /// error (CLT-014). Idempotent.
    fn poison(&self, err: &ClientError) {
        self.alive.store(false, Ordering::SeqCst);
        let drained: Vec<PendingTx> = {
            let mut pending = lock(&self.pending);
            pending.drain().map(|(_, tx)| tx).collect()
        };
        for tx in drained {
            let _ = tx.send(Err(err.clone()));
        }
    }
}

/// One live connection: write half + demux state + the reader task.
struct Conn {
    shared: Arc<ConnShared>,
    /// Writes serialize behind this mutex so frames never interleave
    /// (CLT-011); reads belong to the reader task alone.
    writer: TokioMutex<OwnedWriteHalf>,
    reader_task: JoinHandle<()>,
}

impl Conn {
    fn is_alive(&self) -> bool {
        self.shared.alive.load(Ordering::SeqCst)
    }

    /// Tear down: stop the reader and fail all pending calls typed.
    fn kill(&self, err: &ClientError) {
        self.reader_task.abort();
        self.shared.poison(err);
    }
}

impl Drop for Conn {
    fn drop(&mut self) {
        self.kill(&ClientError::Connection {
            message: "connection dropped".to_owned(),
        });
    }
}

/// Outcome of one dispatch attempt on one connection.
enum DispatchError {
    /// The request never reached the wire — safe to resend on a fresh
    /// connection (not a replay; CLT-031 concerns frames that were sent).
    WriteFailed(ClientError),
    /// Final for this call: the frame may have reached the server, or the
    /// outcome is a server / timeout / poison error. Never retried.
    Fatal(ClientError),
}

impl DispatchError {
    fn into_error(self) -> ClientError {
        match self {
            Self::WriteFailed(e) | Self::Fatal(e) => e,
        }
    }
}

/// A multiplexed, profile-driven Thunder RPC client (SPEC-003).
///
/// Cheap to share behind an `Arc`; every method takes `&self` and calls
/// may run concurrently (CLT-010).
pub struct Client {
    profile: Profile,
    config: ClientConfig,
    endpoint: Endpoint,
    /// Monotonic id allocator, skipping `PUSH_ID` (CLT-010).
    next_id: AtomicU32,
    /// In-flight bound sized `profile.max_in_flight` (CLT-012).
    in_flight: Semaphore,
    /// Current connection; `None` after close.
    conn: StdMutex<Option<Arc<Conn>>>,
    /// Serializes re-dial attempts so one caller reconnects at a time.
    reconnect: TokioMutex<()>,
    closed: AtomicBool,
    /// Push hook shared with every connection's reader task (CLT-060).
    push_handler: Arc<StdMutex<Option<PushHandler>>>,
    /// Responses whose id matched no pending call (CLT-013).
    unknown_drops: Arc<AtomicU64>,
    handshake_info: StdMutex<HandshakeInfo>,
}

impl Client {
    /// Connect with default [`ClientConfig`] and run the profile
    /// handshake (CLT-001/002).
    ///
    /// `endpoint` accepts every form of [`parse_endpoint`] (CLT-070):
    /// `scheme://host[:port]` or bare `host:port`.
    pub async fn connect(endpoint: &str, profile: Profile) -> Result<Self, ClientError> {
        Self::connect_with(endpoint, profile, ClientConfig::default()).await
    }

    /// Connect with explicit configuration.
    pub async fn connect_with(
        endpoint: &str,
        profile: Profile,
        config: ClientConfig,
    ) -> Result<Self, ClientError> {
        let endpoint = parse_endpoint(endpoint)?;
        let client = Self {
            next_id: AtomicU32::new(1),
            in_flight: Semaphore::new(profile.max_in_flight),
            conn: StdMutex::new(None),
            reconnect: TokioMutex::new(()),
            closed: AtomicBool::new(false),
            push_handler: Arc::new(StdMutex::new(None)),
            unknown_drops: Arc::new(AtomicU64::new(0)),
            handshake_info: StdMutex::new(HandshakeInfo::default()),
            endpoint,
            profile,
            config,
        };
        let conn = client.establish().await?;
        *lock(&client.conn) = Some(conn);
        Ok(client)
    }

    /// Issue one call with the client's default timeout (CLT-020).
    ///
    /// Concurrent callers multiplex over the one connection; completion
    /// order follows the server, not submission order (CLT-010).
    pub async fn call(
        &self,
        command: impl Into<String>,
        args: Vec<Value>,
    ) -> Result<Value, ClientError> {
        let command = command.into();
        self.call_with_timeout(&command, args, self.config.call_timeout)
            .await
    }

    /// Issue one call with a per-call timeout override (CLT-020).
    pub async fn call_with_timeout(
        &self,
        command: &str,
        args: Vec<Value>,
        timeout: Duration,
    ) -> Result<Value, ClientError> {
        // CLT-012: bounded in-flight — excess calls wait here, never refused.
        let _permit = self
            .in_flight
            .acquire()
            .await
            .map_err(|_| Self::closed_error())?;
        let mut redials_left = RECONNECT_ATTEMPTS;
        loop {
            let conn = self.live_conn(&mut redials_left).await?;
            match self.dispatch(&conn, command, args.clone(), timeout).await {
                Ok(value) => return Ok(value),
                Err(DispatchError::Fatal(err)) => return Err(err),
                Err(DispatchError::WriteFailed(err)) => {
                    if redials_left == 0 {
                        return Err(err);
                    }
                    // The frame never hit the wire: reconnect and resend.
                }
            }
        }
    }

    /// Register the push hook (CLT-060). Frames with `id == PUSH_ID` are
    /// routed here under `PushPolicy::Enabled` and never matched against
    /// pending calls. The handler runs on the reader task.
    pub fn on_push<F>(&self, handler: F)
    where
        F: Fn(Value) + Send + Sync + 'static,
    {
        *lock(&self.push_handler) = Some(Arc::new(handler));
    }

    /// Explicit, idempotent close (CLT-004): fails all in-flight calls
    /// with a typed connection-closed error and shuts the socket down.
    pub async fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
        self.in_flight.close();
        let conn = lock(&self.conn).take();
        if let Some(conn) = conn {
            conn.kill(&Self::closed_error());
            let mut writer = conn.writer.lock().await;
            let _ = writer.shutdown().await;
        }
    }

    /// `true` once the current connection's handshake authenticated
    /// (CLT-003 — auth is sticky per connection).
    pub fn is_authenticated(&self) -> bool {
        lock(&self.handshake_info).authenticated
    }

    /// Capabilities the server advertised in the `HELLO` reply.
    pub fn capabilities(&self) -> Vec<String> {
        lock(&self.handshake_info).capabilities.clone()
    }

    /// Snapshot of what the handshake learned (CLT-002).
    pub fn handshake_info(&self) -> HandshakeInfo {
        lock(&self.handshake_info).clone()
    }

    /// How many responses matched no pending call and were dropped
    /// (CLT-013 — client stats, never fatal).
    pub fn unknown_response_drops(&self) -> u64 {
        self.unknown_drops.load(Ordering::Relaxed)
    }

    /// The profile this client drives its behavior from.
    pub fn profile(&self) -> &Profile {
        &self.profile
    }

    // ── internals ──────────────────────────────────────────────────────

    fn closed_error() -> ClientError {
        ClientError::Connection {
            message: "client is closed".to_owned(),
        }
    }

    /// Allocate the next request id, skipping `PUSH_ID` (CLT-010).
    fn alloc_id(&self) -> u32 {
        loop {
            let id = self.next_id.fetch_add(1, Ordering::Relaxed);
            if id != PUSH_ID {
                return id;
            }
        }
    }

    /// Return the current live connection, lazily reconnecting when it is
    /// dead or absent: up to `redials_left` re-dial + re-handshake
    /// attempts with capped backoff (CLT-030). Never replays in-flight
    /// calls — those already failed typed when the connection died
    /// (CLT-031).
    async fn live_conn(&self, redials_left: &mut u32) -> Result<Arc<Conn>, ClientError> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(Self::closed_error());
        }
        let current = { lock(&self.conn).clone() };
        if let Some(conn) = current {
            if conn.is_alive() {
                return Ok(conn);
            }
        }
        let _guard = self.reconnect.lock().await;
        if self.closed.load(Ordering::SeqCst) {
            return Err(Self::closed_error());
        }
        // Another caller may have reconnected while we waited.
        let current = { lock(&self.conn).clone() };
        if let Some(conn) = current {
            if conn.is_alive() {
                return Ok(conn);
            }
        }
        let mut last_err = ClientError::Connection {
            message: "connection is dead".to_owned(),
        };
        let mut backoff = BACKOFF_BASE;
        while *redials_left > 0 {
            *redials_left -= 1;
            match self.establish().await {
                Ok(conn) => {
                    *lock(&self.conn) = Some(Arc::clone(&conn));
                    return Ok(conn);
                }
                // An auth rejection is deterministic — retrying cannot fix it.
                Err(err @ ClientError::Auth { .. }) => return Err(err),
                Err(err) => {
                    last_err = err;
                    if *redials_left > 0 {
                        tokio::time::sleep(backoff).await;
                        backoff = (backoff * 2).min(BACKOFF_CAP);
                    }
                }
            }
        }
        Err(last_err)
    }

    /// Dial (with the connect timeout, TCP_NODELAY on — CLT-001), spawn
    /// the reader task, and run the profile handshake (CLT-002).
    async fn establish(&self) -> Result<Arc<Conn>, ClientError> {
        let addr = (self.endpoint.host.as_str(), self.endpoint.port);
        let stream = tokio::time::timeout(self.config.connect_timeout, TcpStream::connect(addr))
            .await
            .map_err(|_| ClientError::Timeout)?
            .map_err(|e| ClientError::Connection {
                message: format!(
                    "connect to {}:{} failed: {e}",
                    self.endpoint.host, self.endpoint.port
                ),
            })?;
        stream
            .set_nodelay(true)
            .map_err(|e| ClientError::Connection {
                message: format!("TCP_NODELAY failed: {e}"),
            })?;
        let (read_half, write_half) = stream.into_split();
        let shared = Arc::new(ConnShared {
            pending: StdMutex::new(HashMap::new()),
            alive: AtomicBool::new(true),
        });
        let reader_task = tokio::spawn(reader_loop(
            BufReader::new(read_half),
            Arc::clone(&shared),
            self.profile.max_frame_bytes,
            self.profile.push,
            Arc::clone(&self.push_handler),
            Arc::clone(&self.unknown_drops),
        ));
        let conn = Arc::new(Conn {
            shared,
            writer: TokioMutex::new(write_half),
            reader_task,
        });
        // On handshake failure the `Err` return drops `conn`, whose Drop
        // aborts the reader and closes the socket.
        let info = self.handshake(&conn).await?;
        *lock(&self.handshake_info) = info;
        Ok(conn)
    }

    /// Run the profile handshake before user calls proceed (CLT-002):
    /// `None` sends nothing; `AuthCommand` sends optional `HELLO` then
    /// `AUTH` when credentials are configured; `HelloMandatory` sends the
    /// `HELLO` map as the first frame and parses the reply.
    async fn handshake(&self, conn: &Arc<Conn>) -> Result<HandshakeInfo, ClientError> {
        match self.profile.handshake {
            Handshake::None => Ok(HandshakeInfo::default()),
            Handshake::AuthCommand => {
                let Some(credentials) = self.config.credentials.clone() else {
                    return Ok(HandshakeInfo::default());
                };
                if self.profile.hello_style == HelloStyle::PositionalVersion {
                    // Optional HELLO announcing the protocol version.
                    self.handshake_call(conn, "HELLO", vec![Value::Int(1)])
                        .await?;
                }
                let args = match credentials {
                    Credentials::Token(token) => vec![Value::Str(token)],
                    Credentials::ApiKey(api_key) => vec![Value::Str(api_key)],
                    Credentials::UserPass { user, pass } => {
                        vec![Value::Str(user), Value::Str(pass)]
                    }
                };
                self.handshake_call(conn, "AUTH", args).await?;
                Ok(HandshakeInfo {
                    authenticated: true,
                    capabilities: Vec::new(),
                })
            }
            Handshake::HelloMandatory => {
                let mut pairs = vec![(Value::Str("version".to_owned()), Value::Int(1))];
                match &self.config.credentials {
                    Some(Credentials::Token(token)) => {
                        pairs.push((Value::Str("token".to_owned()), Value::Str(token.clone())));
                    }
                    Some(Credentials::ApiKey(api_key)) => {
                        pairs.push((
                            Value::Str("api_key".to_owned()),
                            Value::Str(api_key.clone()),
                        ));
                    }
                    Some(Credentials::UserPass { .. }) => {
                        return Err(ClientError::Auth {
                            message: "user/password credentials are not supported by \
                                      HelloMandatory profiles — use a token or api_key (PRO-001)"
                                .to_owned(),
                        });
                    }
                    None => {}
                }
                let name = self
                    .config
                    .client_name
                    .clone()
                    .unwrap_or_else(|| "thunder-client".to_owned());
                pairs.push((Value::Str("client_name".to_owned()), Value::Str(name)));
                let reply = self
                    .handshake_call(conn, "HELLO", vec![Value::Map(pairs)])
                    .await?;
                Ok(HandshakeInfo {
                    authenticated: reply
                        .map_get("authenticated")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    capabilities: reply
                        .map_get("capabilities")
                        .and_then(Value::as_array)
                        .map(|caps| {
                            caps.iter()
                                .filter_map(|v| v.as_str().map(str::to_owned))
                                .collect()
                        })
                        .unwrap_or_default(),
                })
            }
        }
    }

    /// One handshake round-trip. Server rejections surface as the typed
    /// auth class, never a generic error (CLT-003); transport failures
    /// keep their own class.
    async fn handshake_call(
        &self,
        conn: &Arc<Conn>,
        command: &str,
        args: Vec<Value>,
    ) -> Result<Value, ClientError> {
        self.dispatch(conn, command, args, self.config.call_timeout)
            .await
            .map_err(|e| match e.into_error() {
                ClientError::Server { message, .. } | ClientError::Auth { message } => {
                    ClientError::Auth { message }
                }
                other => other,
            })
    }

    /// One request/response attempt on one connection: register the
    /// pending entry, write the frame (serialized, CLT-011), await the
    /// demuxed response under the timeout (CLT-020).
    async fn dispatch(
        &self,
        conn: &Arc<Conn>,
        command: &str,
        args: Vec<Value>,
        timeout: Duration,
    ) -> Result<Value, DispatchError> {
        let id = self.alloc_id();
        let (tx, rx) = oneshot::channel();
        {
            // Register under the pending lock, checking liveness inside
            // the same critical section the poisoner drains under — a
            // dying connection either fails this entry or is seen dead.
            let mut pending = lock(&conn.shared.pending);
            if !conn.shared.alive.load(Ordering::SeqCst) {
                return Err(DispatchError::WriteFailed(ClientError::Connection {
                    message: "connection is dead".to_owned(),
                }));
            }
            pending.insert(id, tx);
        }
        let request = Request {
            id,
            command: command.to_owned(),
            args,
        };
        {
            let mut writer = conn.writer.lock().await;
            if let Err(e) = write_request(&mut *writer, &request).await {
                lock(&conn.shared.pending).remove(&id);
                let err = ClientError::Connection {
                    message: format!("write failed: {e}"),
                };
                conn.kill(&err);
                return Err(DispatchError::WriteFailed(err));
            }
        }
        match tokio::time::timeout(timeout, rx).await {
            // CLT-020: remove the pending entry on timeout; a late
            // response to this id is dropped per CLT-013.
            Err(_elapsed) => {
                lock(&conn.shared.pending).remove(&id);
                Err(DispatchError::Fatal(ClientError::Timeout))
            }
            // Poison always sends before dropping senders; a bare drop
            // still means the connection went away.
            Ok(Err(_recv)) => Err(DispatchError::Fatal(ClientError::Connection {
                message: "connection closed before response".to_owned(),
            })),
            Ok(Ok(Err(poison))) => Err(DispatchError::Fatal(poison)),
            Ok(Ok(Ok(response))) => match response.result {
                Ok(value) => Ok(value),
                Err(message) => Err(DispatchError::Fatal(ClientError::from_server_message(
                    message,
                    self.profile.error_codes,
                ))),
            },
        }
    }
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client")
            .field("profile", &self.profile.name)
            .field("endpoint", &self.endpoint)
            .field("closed", &self.closed.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        // CLT-004: dropping the client closes the socket and fails all
        // in-flight calls with a typed connection-closed error.
        if let Ok(mut guard) = self.conn.lock() {
            if let Some(conn) = guard.take() {
                conn.kill(&Self::closed_error());
            }
        }
    }
}

/// The background reader (CLT-010): reads frames with the profile cap,
/// demuxes by id, routes push frames (CLT-060), drops unknown ids
/// (CLT-013), and poisons the connection on any read failure (CLT-014).
async fn reader_loop(
    mut reader: BufReader<OwnedReadHalf>,
    shared: Arc<ConnShared>,
    max_frame_bytes: usize,
    push: PushPolicy,
    push_handler: Arc<StdMutex<Option<PushHandler>>>,
    unknown_drops: Arc<AtomicU64>,
) {
    let err = loop {
        match read_response_with_limit(&mut reader, max_frame_bytes).await {
            Ok((response, _frame_bytes)) => {
                if response.id == PUSH_ID {
                    match push {
                        PushPolicy::Enabled => {
                            let handler = { lock(&push_handler).clone() };
                            if let (Some(handler), Ok(value)) = (handler, response.result) {
                                handler(value);
                            }
                        }
                        PushPolicy::Reserved => {
                            // Protocol error: poison per CLT-014.
                            break ClientError::Decode {
                                message: "server sent a push frame but the profile reserves \
                                          PUSH_ID (CLT-060)"
                                    .to_owned(),
                            };
                        }
                    }
                    continue;
                }
                let tx = lock(&shared.pending).remove(&response.id);
                match tx {
                    Some(tx) => {
                        let _ = tx.send(Ok(response));
                    }
                    // CLT-013: unknown id — count and drop, never fatal.
                    None => {
                        unknown_drops.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
            Err(e) => break classify_read_error(&e),
        }
    };
    // CLT-014: fail all pending calls typed; dropping the read half on
    // return closes our side of the socket.
    shared.poison(&err);
}

/// Map a reader I/O failure onto the stable error classes. The wire layer
/// reports both cap violations and malformed MessagePack as
/// `InvalidData`; the cap message is pinned by thunder-wire
/// ("… exceeds limit …", WIRE-020/021).
fn classify_read_error(e: &std::io::Error) -> ClientError {
    if e.kind() == std::io::ErrorKind::InvalidData {
        let message = e.to_string();
        if message.contains("exceeds limit") {
            ClientError::FrameTooLarge { message }
        } else {
            ClientError::Decode { message }
        }
    } else {
        ClientError::Connection {
            message: format!("connection lost: {e}"),
        }
    }
}

/// Lock a std mutex, riding through poisoning (a panicked holder must not
/// take the whole client down — the guarded state stays consistent).
fn lock<T>(mutex: &StdMutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

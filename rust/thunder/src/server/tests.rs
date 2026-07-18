//! In-crate integration suite (SRV-050), un-gated: a real TCP listener, a
//! test dispatch, and the raw `crate::wire` codec as the client.

use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::wire::config::{Config, ErrorConvention, Handshake, HelloStyle, PushPolicy, TlsPolicy};
use crate::wire::{encode_frame, read_response, Request, Response, Value, PUSH_ID};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;

use crate::server::{
    spawn_listener, AuthError, Credentials, Dispatch, ListenerConfig, ListenerHandle, Principal,
    PushSender, ServerInfo, Session, NOAUTH, WRONGPASS,
};

// ── Test dispatch ────────────────────────────────────────────────────────────

/// Product stand-in: echoes, sleeps, subscribes, knows one key, one user
/// and one token.
#[derive(Default)]
struct EchoDispatch {
    /// PushSender captured by `SUBSCRIBE` — proves the sender outlives the
    /// registering request (SRV-013).
    push: Mutex<Option<PushSender>>,
}

impl Dispatch for EchoDispatch {
    async fn dispatch(
        &self,
        session: &Session,
        command: &str,
        args: Vec<Value>,
    ) -> Result<Value, String> {
        match command {
            "PING" => Ok(Value::Str("PONG".to_owned())),
            "ECHO" => Ok(args.into_iter().next().unwrap_or(Value::Null)),
            "SLEEP" => {
                let ms = args.first().and_then(Value::as_int).unwrap_or(0);
                tokio::time::sleep(Duration::from_millis(ms as u64)).await;
                Ok(Value::Int(ms))
            }
            "SUBSCRIBE" => {
                let sender = session
                    .push_sender()
                    .cloned()
                    .ok_or_else(|| "ERR push is not enabled on this profile".to_owned())?;
                *self.push.lock().unwrap() = Some(sender);
                Ok(Value::Str("OK".to_owned()))
            }
            "WHOAMI" => Ok(session
                .principal()
                .map_or(Value::Null, |principal| Value::Str(principal.name))),
            other => Err(format!("ERR unknown command '{other}'")),
        }
    }

    async fn authenticate(&self, creds: Credentials) -> Result<Principal, AuthError> {
        match creds {
            Credentials::ApiKey(key) if key == "key-1" => Ok(Principal {
                name: "api-key".to_owned(),
            }),
            Credentials::UserPass(user, pass) if user == "root" && pass == "hunter2" => {
                Ok(Principal { name: user })
            }
            Credentials::Token(token) if token == "tok-1" => Ok(Principal {
                name: "token-user".to_owned(),
            }),
            _ => Err(AuthError::InvalidCredentials),
        }
    }

    fn capabilities(&self, _principal: &Principal) -> Vec<String> {
        vec!["search".to_owned(), "insert".to_owned()]
    }
}

// ── Harness helpers ──────────────────────────────────────────────────────────

fn info() -> ServerInfo {
    ServerInfo {
        name: "thunder-test".to_owned(),
        version: "0.0.0".to_owned(),
    }
}

fn config() -> ListenerConfig {
    ListenerConfig::default()
}

async fn start(profile: Config) -> (ListenerHandle, Arc<EchoDispatch>) {
    start_with(profile, config()).await
}

/// A listener for a deployment that does not require credentials —
/// `auth_required = false` (Synap's `require_auth` / Nexus's `auth_required`
/// turned off). The handshake *shape* still comes from the profile; only
/// enforcement is off (SRV-011).
async fn start_open(profile: Config) -> (ListenerHandle, Arc<EchoDispatch>) {
    start_with(profile, config().open()).await
}

async fn start_with(
    profile: Config,
    config: ListenerConfig,
) -> (ListenerHandle, Arc<EchoDispatch>) {
    let dispatch = Arc::new(EchoDispatch::default());
    let handle = spawn_listener(Arc::clone(&dispatch), profile, info(), config)
        .await
        .unwrap();
    (handle, dispatch)
}

async fn connect(handle: &ListenerHandle) -> TcpStream {
    TcpStream::connect(handle.local_addr()).await.unwrap()
}

/// Write one request frame; returns the frame size (the server's in-bytes).
async fn send(stream: &mut TcpStream, id: u32, command: &str, args: Vec<Value>) -> usize {
    let frame = encode_frame(&Request {
        id,
        command: command.to_owned(),
        args,
    })
    .unwrap();
    stream.write_all(&frame).await.unwrap();
    frame.len()
}

async fn recv(stream: &mut TcpStream) -> Response {
    let (response, _) = read_response(stream).await.unwrap();
    response
}

async fn call(stream: &mut TcpStream, id: u32, command: &str, args: Vec<Value>) -> Response {
    send(stream, id, command, args).await;
    recv(stream).await
}

/// A config with the `AuthCommand` shape and no HELLO — and a
/// push-producing command. Named for the SHAPE it exercises: Thunder ships
/// no product configs, so tests bring their own, exactly like an
/// application does.
fn auth_command_config() -> Config {
    Config::standard()
        .scheme("authcmd")
        .port(0)
        .handshake(Handshake::AuthCommand)
        .hello_style(HelloStyle::NotUsed)
        .push(PushPolicy::Enabled)
        .error_codes(ErrorConvention::Resp3Prefixes)
}

/// A config with the `AuthCommand` shape plus an optional arg-less HELLO.
fn argless_hello_config() -> Config {
    Config::standard()
        .scheme("argless")
        .port(0)
        .handshake(Handshake::AuthCommand)
        .hello_style(HelloStyle::ArgLess)
        .error_codes(ErrorConvention::Resp3Prefixes)
}

/// A config with the standard `HelloMandatory` + map-payload shape and
/// bracket-code errors.
fn hello_mandatory_config() -> Config {
    Config::standard()
        .scheme("hellomap")
        .port(0)
        .error_codes(ErrorConvention::BracketCode)
}

/// A custom config (PRO-020) with a tiny frame cap for WIRE-020 tests.
fn tiny_profile() -> Config {
    Config {
        scheme: "tiny",
        default_port: 0,
        handshake: Handshake::None,
        hello_style: HelloStyle::NotUsed,
        push: PushPolicy::Reserved,
        max_frame_bytes: 64,
        max_in_flight: 4,
        error_codes: ErrorConvention::Resp3Prefixes,
        tls: TlsPolicy::Off,
    }
}

/// A custom config (PRO-020) with no handshake at all — the `Handshake::None`
/// variant, for a deployment that gates nothing.
fn no_handshake_profile() -> Config {
    Config {
        scheme: "open",
        default_port: 0,
        handshake: Handshake::None,
        hello_style: HelloStyle::NotUsed,
        push: PushPolicy::Reserved,
        max_frame_bytes: crate::wire::DEFAULT_MAX_FRAME_BYTES,
        max_in_flight: 16,
        error_codes: ErrorConvention::Resp3Prefixes,
        tls: TlsPolicy::Off,
    }
}

// ── SRV-050: ping round-trip over real TCP ──────────────────────────────────

#[tokio::test]
async fn ping_round_trips_pre_auth() {
    let (handle, _dispatch) = start(argless_hello_config()).await;
    let mut client = connect(&handle).await;
    let response = call(&mut client, 1, "PING", vec![]).await;
    assert_eq!(response.id, 1);
    assert_eq!(response.result, Ok(Value::Str("PONG".to_owned())));
    // Family echo form, still pre-auth (SRV-011 allowlist).
    let response = call(&mut client, 2, "PING", vec![Value::Str("hi".into())]).await;
    assert_eq!(response.result, Ok(Value::Str("hi".to_owned())));
}

// ── SRV-050: 5-way multiplexing, out-of-order completion ────────────────────

#[tokio::test]
async fn five_way_multiplexing_completes_out_of_order() {
    let (handle, _dispatch) = start_open(auth_command_config()).await;
    let mut client = connect(&handle).await;
    // ids 1..=5 sleep 400,300,200,100,0 ms — completion reverses the
    // request order (SRV-002/003).
    for (id, ms) in [(1u32, 400i64), (2, 300), (3, 200), (4, 100), (5, 0)] {
        send(&mut client, id, "SLEEP", vec![Value::Int(ms)]).await;
    }
    let mut order = Vec::new();
    for _ in 0..5 {
        order.push(recv(&mut client).await.id);
    }
    let mut sorted = order.clone();
    sorted.sort_unstable();
    assert_eq!(sorted, vec![1, 2, 3, 4, 5], "every request answered once");
    assert_ne!(
        order,
        vec![1, 2, 3, 4, 5],
        "completion must not follow request order"
    );
    assert_eq!(order.first(), Some(&5), "shortest sleep completes first");
    assert_eq!(order.last(), Some(&1), "longest sleep completes last");
}

// ── SRV-050 / SRV-013: PUSH_ID refusal ──────────────────────────────────────

#[tokio::test]
async fn push_id_client_frame_is_refused_and_connection_stays_usable() {
    let (handle, _dispatch) = start_open(auth_command_config()).await;
    let mut client = connect(&handle).await;
    let response = call(&mut client, PUSH_ID, "ECHO", vec![Value::Int(1)]).await;
    assert_eq!(response.id, PUSH_ID);
    let err = response.result.unwrap_err();
    assert!(
        err.contains("reserved for server push"),
        "dedicated refusal expected, got: {err}"
    );
    // The refusal must not poison the connection (SRV-013).
    let response = call(&mut client, 7, "ECHO", vec![Value::Int(2)]).await;
    assert_eq!(response.result, Ok(Value::Int(2)));
}

// ── SRV-050 / SRV-005: unknown-command survival ─────────────────────────────

#[tokio::test]
async fn unknown_command_error_leaves_connection_usable() {
    let (handle, _dispatch) = start_open(auth_command_config()).await;
    let mut client = connect(&handle).await;
    let response = call(&mut client, 1, "NOPE", vec![]).await;
    assert_eq!(
        response.result,
        Err("ERR unknown command 'NOPE'".to_owned())
    );
    let response = call(
        &mut client,
        2,
        "ECHO",
        vec![Value::Str("still alive".into())],
    )
    .await;
    assert_eq!(response.result, Ok(Value::Str("still alive".to_owned())));
}

// ── SRV-050 / SRV-011: auth gating per profile ──────────────────────────────

#[tokio::test]
async fn auth_command_profile_gates_until_auth_succeeds() {
    let (handle, _dispatch) = start(argless_hello_config()).await;
    let mut client = connect(&handle).await;
    // Non-allowlisted command pre-auth → NOAUTH (SRV-011).
    let response = call(&mut client, 1, "ECHO", vec![Value::Int(1)]).await;
    assert_eq!(response.result, Err(NOAUTH.to_owned()));
    // Wrong credentials → WRONGPASS, still gated.
    let response = call(
        &mut client,
        2,
        "AUTH",
        vec![Value::Str("root".into()), Value::Str("wrong".into())],
    )
    .await;
    assert_eq!(response.result, Err(WRONGPASS.to_owned()));
    let response = call(&mut client, 3, "ECHO", vec![Value::Int(1)]).await;
    assert_eq!(response.result, Err(NOAUTH.to_owned()));
    // Correct credentials → OK, then commands dispatch (SRV-012).
    let response = call(
        &mut client,
        4,
        "AUTH",
        vec![Value::Str("root".into()), Value::Str("hunter2".into())],
    )
    .await;
    assert_eq!(response.result, Ok(Value::Str("OK".to_owned())));
    let response = call(&mut client, 5, "ECHO", vec![Value::Int(42)]).await;
    assert_eq!(response.result, Ok(Value::Int(42)));
    // The principal landed on the session (SRV-010).
    let response = call(&mut client, 6, "WHOAMI", vec![]).await;
    assert_eq!(response.result, Ok(Value::Str("root".to_owned())));
}

// ── SRV-014: HELLO reply shapes ─────────────────────────────────────────────

#[tokio::test]
async fn hello_reply_matches_the_metadata_shape() {
    let (handle, _dispatch) = start(argless_hello_config()).await;
    let mut client = connect(&handle).await;
    let response = call(&mut client, 1, "HELLO", vec![Value::Int(1)]).await;
    let value = response.result.unwrap();
    assert_eq!(
        value.map_get("server"),
        Some(&Value::Str("thunder-test".to_owned()))
    );
    assert_eq!(
        value.map_get("version"),
        Some(&Value::Str("0.0.0".to_owned()))
    );
    assert_eq!(value.map_get("proto"), Some(&Value::Int(1)));
    assert!(matches!(value.map_get("id"), Some(Value::Int(_))));
    assert_eq!(value.map_get("authenticated"), Some(&Value::Bool(false)));
    // After AUTH the same reply reports authenticated = true.
    call(&mut client, 2, "AUTH", vec![Value::Str("key-1".into())]).await;
    let response = call(&mut client, 3, "HELLO", vec![Value::Int(1)]).await;
    assert_eq!(
        response.result.unwrap().map_get("authenticated"),
        Some(&Value::Bool(true))
    );
}

#[tokio::test]
async fn hello_mandatory_rejects_non_hello_first_frame_and_closes() {
    let (handle, _dispatch) = start(hello_mandatory_config()).await;
    let mut client = connect(&handle).await;
    let response = call(&mut client, 1, "PING", vec![]).await;
    assert!(
        response.result.is_err(),
        "non-HELLO first frame must be rejected (SRV-011)"
    );
    // The connection is closed after the rejection (PRO-030).
    assert!(read_response(&mut client).await.is_err());
}

#[tokio::test]
async fn hello_mandatory_handshake_grants_access_and_reports_capabilities() {
    let (handle, _dispatch) = start(hello_mandatory_config()).await;
    let mut client = connect(&handle).await;
    let hello_arg = Value::Map(vec![
        (Value::Str("version".into()), Value::Int(1)),
        (Value::Str("token".into()), Value::Str("tok-1".into())),
        (Value::Str("client_name".into()), Value::Str("suite".into())),
    ]);
    let response = call(&mut client, 1, "HELLO", vec![hello_arg]).await;
    let value = response.result.unwrap();
    // Capabilities reply shape (SRV-014), capabilities from the hook.
    assert_eq!(value.map_get("protocol_version"), Some(&Value::Int(1)));
    assert_eq!(
        value.map_get("capabilities"),
        Some(&Value::Array(vec![
            Value::Str("search".into()),
            Value::Str("insert".into())
        ]))
    );
    let response = call(&mut client, 2, "ECHO", vec![Value::Int(9)]).await;
    assert_eq!(response.result, Ok(Value::Int(9)));
    let response = call(&mut client, 3, "WHOAMI", vec![]).await;
    assert_eq!(response.result, Ok(Value::Str("token-user".to_owned())));
}

#[tokio::test]
async fn non_hello_first_frames_are_counted_for_handshake_adoption() {
    // Leading with HELLO does not bump the adoption counter (SPEC-008).
    let (handle, _d) = start(hello_mandatory_config()).await;
    let mut client = connect(&handle).await;
    let hello = Value::Map(vec![
        (Value::Str("version".into()), Value::Int(1)),
        (Value::Str("token".into()), Value::Str("tok-1".into())),
    ]);
    let _ = call(&mut client, 1, "HELLO", vec![hello]).await;
    assert_eq!(handle.snapshot().non_hello_first_frames_total, 0);
    drop(client);
    handle.stop().await;

    // Under an optional-HELLO profile a client may go straight to a call;
    // each such connection is a non-HELLO first frame, counted cumulatively.
    let (handle, _d) = start(argless_hello_config()).await;
    let mut c1 = connect(&handle).await;
    let _ = call(&mut c1, 1, "PING", vec![]).await;
    let mut c2 = connect(&handle).await;
    let _ = call(&mut c2, 1, "PING", vec![]).await;
    assert_eq!(
        handle.snapshot().non_hello_first_frames_total,
        2,
        "two connections led with a non-HELLO frame"
    );
    drop(c1);
    drop(c2);
    handle.stop().await;
}

#[tokio::test]
async fn hello_mandatory_bad_credentials_error_allows_retry() {
    let (handle, _dispatch) = start(hello_mandatory_config()).await;
    let mut client = connect(&handle).await;
    let bad = Value::Map(vec![(
        Value::Str("token".into()),
        Value::Str("nope".into()),
    )]);
    let response = call(&mut client, 1, "HELLO", vec![bad]).await;
    assert_eq!(
        response.result,
        Err("[unauthorized] invalid credentials".to_owned())
    );
    // Still gated with the profile's convention, but the connection
    // survives a failed handshake.
    let response = call(&mut client, 2, "ECHO", vec![Value::Int(1)]).await;
    assert_eq!(
        response.result,
        Err("[unauthorized] authentication required: send HELLO first".to_owned())
    );
    let good = Value::Map(vec![(
        Value::Str("token".into()),
        Value::Str("tok-1".into()),
    )]);
    let response = call(&mut client, 3, "HELLO", vec![good]).await;
    assert!(response.result.is_ok());
    let response = call(&mut client, 4, "ECHO", vec![Value::Int(1)]).await;
    assert_eq!(response.result, Ok(Value::Int(1)));
}

// ── SRV-011 / BN-023: handshake shape vs auth policy ────────────────────────

/// The `AuthCommand` *shape* on a deployment that requires credentials gates
/// everything until `AUTH` lands. Enforcement is the deployment's
/// (`auth_required`), never the shape's (PRO-001a).
#[tokio::test]
async fn auth_command_shape_gates_until_auth_when_the_deployment_requires_it() {
    let (handle, _dispatch) = start(auth_command_config()).await;
    let mut client = connect(&handle).await;
    // Pre-auth: gated.
    let response = call(&mut client, 1, "ECHO", vec![Value::Int(1)]).await;
    assert_eq!(response.result, Err(NOAUTH.to_owned()));
    // The AUTH forms: `AUTH <password>` and `AUTH <user> <password>`.
    let response = call(
        &mut client,
        2,
        "AUTH",
        vec![Value::Str("root".into()), Value::Str("hunter2".into())],
    )
    .await;
    assert!(response.result.is_ok(), "AUTH must succeed: {response:?}");
    // Post-auth: dispatched, and the principal reached the session.
    let response = call(&mut client, 3, "ECHO", vec![Value::Int(1)]).await;
    assert_eq!(response.result, Ok(Value::Int(1)));
    let response = call(&mut client, 4, "WHOAMI", vec![]).await;
    assert_eq!(response.result, Ok(Value::Str("root".to_owned())));
}

/// The same profile against an **open** deployment (`require_auth = false`)
/// serves un-credentialed sessions. Shape is the profile's, enforcement is the
/// deployment's — conflating them is the bug BN-023 fixed.
#[tokio::test]
async fn auth_command_profile_serves_uncredentialed_sessions_when_open() {
    let (handle, _dispatch) = start_open(auth_command_config()).await;
    let mut client = connect(&handle).await;
    let response = call(&mut client, 1, "ECHO", vec![Value::Int(7)]).await;
    assert_eq!(
        response.result,
        Ok(Value::Int(7)),
        "an open deployment must not demand AUTH"
    );
}

/// Bad credentials still fail on an open deployment: `auth_required = false`
/// removes the *requirement*, it does not make `AUTH` succeed unconditionally.
#[tokio::test]
async fn open_deployment_still_rejects_bad_credentials() {
    let (handle, _dispatch) = start_open(auth_command_config()).await;
    let mut client = connect(&handle).await;
    let response = call(&mut client, 1, "AUTH", vec![Value::Str("nope".into())]).await;
    assert_eq!(response.result, Err(WRONGPASS.to_owned()));
}

#[tokio::test]
async fn handshake_none_dispatches_the_first_frame() {
    // `Handshake::None` needs no deployment opt-out: there is no gate at all,
    // so the default (auth_required = true) config still serves immediately.
    let (handle, _dispatch) = start(no_handshake_profile()).await;
    let mut client = connect(&handle).await;
    let response = call(&mut client, 1, "ECHO", vec![Value::Str("first".into())]).await;
    assert_eq!(response.result, Ok(Value::Str("first".to_owned())));
}

// ── SRV-050 / WIRE-020: oversized frame ─────────────────────────────────────

#[tokio::test]
async fn oversized_frame_closes_the_connection_without_killing_the_listener() {
    let (handle, _dispatch) = start(tiny_profile()).await;
    let mut client = connect(&handle).await;
    // A length prefix over the cap — no body needed: the check fires
    // before any body allocation (WIRE-020/021).
    client.write_all(&1024u32.to_le_bytes()).await.unwrap();
    assert!(
        read_response(&mut client).await.is_err(),
        "connection must be closed"
    );
    // The listener survives (SRV-004): a fresh connection works.
    let mut second = connect(&handle).await;
    let response = call(&mut second, 1, "ECHO", vec![Value::Int(5)]).await;
    assert_eq!(response.result, Ok(Value::Int(5)));
}

// ── SRV-050 / SRV-004: malformed body isolation ─────────────────────────────

#[tokio::test]
async fn malformed_body_closes_only_that_connection() {
    let (handle, _dispatch) = start_open(auth_command_config()).await;
    let mut bad = connect(&handle).await;
    let mut good = connect(&handle).await;
    // Well-formed prefix, garbage MessagePack body (0xc1 is never valid).
    bad.write_all(&4u32.to_le_bytes()).await.unwrap();
    bad.write_all(&[0xc1, 0xc1, 0xc1, 0xc1]).await.unwrap();
    assert!(
        read_response(&mut bad).await.is_err(),
        "malformed body closes the connection"
    );
    // The sibling connection is untouched (SRV-004).
    let response = call(&mut good, 1, "ECHO", vec![Value::Int(3)]).await;
    assert_eq!(response.result, Ok(Value::Int(3)));
}

// ── SRV-013: push emission when push is enabled ─────────────────────────────

#[tokio::test]
async fn push_frames_flow_when_push_is_enabled() {
    let (handle, dispatch) = start_open(auth_command_config()).await;
    let mut client = connect(&handle).await;
    let response = call(&mut client, 1, "SUBSCRIBE", vec![]).await;
    assert_eq!(response.result, Ok(Value::Str("OK".to_owned())));
    // The registering request has completed; the captured PushSender is
    // still valid for the connection lifetime (SRV-013).
    let sender = dispatch.push.lock().unwrap().clone().unwrap();
    sender.push(Value::Str("event-1".to_owned())).await.unwrap();
    let push = recv(&mut client).await;
    assert_eq!(push.id, PUSH_ID);
    assert_eq!(push.result, Ok(Value::Str("event-1".to_owned())));
    // The same connection still serves requests after a push.
    let response = call(&mut client, 2, "ECHO", vec![Value::Int(1)]).await;
    assert_eq!(response.result, Ok(Value::Int(1)));
    // Once the connection is gone, pushes fail instead of hanging.
    drop(client);
    handle.stop().await;
    assert!(sender.push(Value::Null).await.is_err());
}

#[tokio::test]
async fn push_reserved_profile_exposes_no_push_sender() {
    let (handle, _dispatch) = start(argless_hello_config()).await;
    let mut client = connect(&handle).await;
    call(&mut client, 1, "AUTH", vec![Value::Str("key-1".into())]).await;
    let response = call(&mut client, 2, "SUBSCRIBE", vec![]).await;
    // PRO-031: under `Reserved` no push emission is possible.
    assert_eq!(
        response.result,
        Err("ERR push is not enabled on this profile".to_owned())
    );
}

// ── SRV-030 / SRV-007: metrics ──────────────────────────────────────────────

#[tokio::test]
async fn metrics_snapshot_counts_after_successful_writes() {
    let mut cfg = config();
    cfg.slow_threshold = Duration::from_millis(5);
    let (handle, _dispatch) = start_with(auth_command_config(), cfg.open()).await;
    let mut client = connect(&handle).await;

    let request_frame_len = send(&mut client, 1, "ECHO", vec![Value::Str("hi".into())]).await;
    let response = recv(&mut client).await;
    let response_frame_len = encode_frame(&response).unwrap().len();

    let snap = handle.snapshot();
    assert_eq!(snap.connections, 1);
    assert_eq!(snap.commands_total, 1);
    assert_eq!(snap.commands_error_total, 0);
    // SRV-007: in-bytes from the decoder's frame size, out-bytes from the
    // single encoded buffer — both match the client's own encoding.
    assert_eq!(snap.frame_bytes_in_total, request_frame_len as u64);
    assert_eq!(snap.frame_bytes_out_total, response_frame_len as u64);

    // An error response bumps the error counter…
    call(&mut client, 2, "NOPE", vec![]).await;
    // …and a command over the 5 ms threshold bumps the slow counter.
    call(&mut client, 3, "SLEEP", vec![Value::Int(50)]).await;

    let snap = handle.snapshot();
    assert_eq!(snap.commands_total, 3);
    assert_eq!(snap.commands_error_total, 1);
    assert_eq!(snap.slow_commands_total, 1);
    assert!(snap.command_duration_microseconds_total >= 45_000);
}

// ── SRV-009: idle timeout ───────────────────────────────────────────────────

#[tokio::test]
async fn idle_timeout_closes_a_silent_connection() {
    let mut cfg = config();
    cfg.idle_timeout = Duration::from_millis(100);
    let (handle, _dispatch) = start_with(auth_command_config(), cfg.open()).await;
    let mut client = connect(&handle).await;
    // A request inside the window works…
    let response = call(&mut client, 1, "ECHO", vec![Value::Int(1)]).await;
    assert!(response.result.is_ok());
    // …then silence: the per-read timeout closes the connection.
    let read = tokio::time::timeout(Duration::from_secs(5), read_response(&mut client)).await;
    assert!(
        matches!(read, Ok(Err(_))),
        "expected EOF after idle timeout, got {read:?}"
    );
}

// ── SRV-001: graceful shutdown drains in-flight work ────────────────────────

#[tokio::test]
async fn stop_drains_in_flight_requests_before_closing() {
    let (handle, _dispatch) = start_open(auth_command_config()).await;
    let mut client = connect(&handle).await;
    send(&mut client, 1, "SLEEP", vec![Value::Int(150)]).await;
    // Let the server read the frame before the shutdown signal lands.
    tokio::time::sleep(Duration::from_millis(50)).await;
    let addr = handle.local_addr();
    handle.stop().await;
    // The in-flight response was drained before the close (SRV-001).
    let response = recv(&mut client).await;
    assert_eq!(response.id, 1);
    assert_eq!(response.result, Ok(Value::Int(150)));
    assert!(
        read_response(&mut client).await.is_err(),
        "connection closed after drain"
    );
    assert!(
        TcpStream::connect(addr).await.is_err(),
        "listener no longer accepts"
    );
}

// ── SRV-021: error-format helpers ───────────────────────────────────────────

#[test]
fn error_helpers_format_the_family_conventions() {
    assert_eq!(
        crate::server::format_bracket_code("not_found", "no such collection"),
        "[not_found] no such collection"
    );
    assert_eq!(crate::server::format_err("boom"), "ERR boom");
    assert!(NOAUTH.starts_with("NOAUTH "));
    assert!(WRONGPASS.starts_with("WRONGPASS "));
}

// ── Adoption fixes from the Synap swap (0.1.2) ──────────────────────────────

/// GH #2: at the ceiling a further accept is **refused**, not queued — the
/// client fails fast instead of hanging on a connection nobody will read.
#[tokio::test]
async fn max_connections_refuses_past_the_ceiling_and_counts_it() {
    let (handle, _d) = start_with(no_handshake_profile(), config().with_max_connections(2)).await;

    let _a = connect(&handle).await;
    let _b = connect(&handle).await;
    // Both slots are taken; let the accept loop settle before the third dial.
    tokio::time::sleep(Duration::from_millis(50)).await;

    let mut refused = TcpStream::connect(handle.local_addr()).await.unwrap();
    // The listener drops the socket immediately, so the far end reads EOF
    // rather than blocking forever.
    let mut buf = [0u8; 1];
    let read = tokio::time::timeout(
        Duration::from_secs(2),
        tokio::io::AsyncReadExt::read(&mut refused, &mut buf),
    )
    .await
    .expect("a refused connection must not hang");
    assert_eq!(read.unwrap(), 0, "refused connection is closed, not queued");

    assert_eq!(
        handle.snapshot().connections_refused_total,
        1,
        "the ceiling must be observable, not silent"
    );
}

/// GH #2: a closed connection frees its slot, so the ceiling bounds
/// *concurrent* connections rather than total accepts over time.
#[tokio::test]
async fn max_connections_frees_a_slot_when_a_connection_closes() {
    let (handle, _d) = start_with(no_handshake_profile(), config().with_max_connections(1)).await;

    let first = connect(&handle).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    drop(first);
    tokio::time::sleep(Duration::from_millis(150)).await;

    // The slot came back: this connection is served, not refused.
    let mut second = connect(&handle).await;
    let response = call(&mut second, 1, "PING", vec![]).await;
    assert_eq!(response.result.unwrap(), Value::Str("PONG".to_owned()));
}

/// GH #2: the default stays unbounded, so existing deployments are untouched.
#[tokio::test]
async fn max_connections_defaults_to_unbounded() {
    assert_eq!(ListenerConfig::default().max_connections, 0);
    let (handle, _d) = start(no_handshake_profile()).await;
    let mut conns = Vec::new();
    for _ in 0..8 {
        conns.push(connect(&handle).await);
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(handle.snapshot().connections_refused_total, 0);
}

/// GH #5: an `Arc<ListenerHandle>` can both observe and drain. Before this,
/// sharing the handle made `stop()` unreachable and shutdown silently
/// downgraded to the fire-and-forget `Drop` path.
#[tokio::test]
async fn a_shared_handle_can_both_observe_and_stop() {
    let (handle, _d) = start(no_handshake_profile()).await;
    let handle = Arc::new(handle);

    // The "exporter task" holds a metrics reader; the handle stays shared.
    let reader = handle.metrics();
    let observer = Arc::clone(&handle);
    assert_eq!(reader.snapshot().connections, 0);
    assert_eq!(observer.snapshot().connections, 0);

    let mut conn = connect(&handle).await;
    let _ = call(&mut conn, 1, "PING", vec![]).await;
    assert_eq!(reader.snapshot().commands_total, 1);

    // Graceful drain through the shared handle — the point of the fix.
    handle.stop().await;
    assert!(TcpStream::connect(handle.local_addr()).await.is_err());
}

/// GH #5: stopping twice is safe — the second call observes the shutdown the
/// first one completed rather than hanging.
#[tokio::test]
async fn stopping_twice_is_safe() {
    let (handle, _d) = start(no_handshake_profile()).await;
    handle.stop().await;
    tokio::time::timeout(Duration::from_secs(2), handle.stop())
        .await
        .expect("a second stop must return, not hang");
}

/// GH #3: the observer sees every command with the label and both byte
/// counts — the dimensions cumulative totals cannot reconstruct.
#[tokio::test]
async fn the_metrics_observer_sees_each_command_with_its_label() {
    #[derive(Default)]
    struct Recorder {
        seen: Mutex<Vec<(String, usize, usize, bool)>>,
        opened: AtomicUsize,
        closed: AtomicUsize,
    }
    impl crate::server::MetricsObserver for Recorder {
        fn command_completed(
            &self,
            command: &str,
            in_bytes: usize,
            out_bytes: usize,
            _duration: Duration,
            is_error: bool,
        ) {
            self.seen
                .lock()
                .unwrap()
                .push((command.to_owned(), in_bytes, out_bytes, is_error));
        }
        fn connection_opened(&self) {
            self.opened.fetch_add(1, AtomicOrdering::Relaxed);
        }
        fn connection_closed(&self) {
            self.closed.fetch_add(1, AtomicOrdering::Relaxed);
        }
    }

    let recorder = Arc::new(Recorder::default());
    let observer: Arc<dyn crate::server::MetricsObserver> = Arc::clone(&recorder) as _;
    let (handle, _d) = start_with(no_handshake_profile(), config().with_observer(observer)).await;

    let mut conn = connect(&handle).await;
    let ping_in = send(&mut conn, 1, "PING", vec![]).await;
    let _ = recv(&mut conn).await;
    let bad_in = send(&mut conn, 2, "NOPE", vec![]).await;
    let _ = recv(&mut conn).await;
    drop(conn);
    tokio::time::sleep(Duration::from_millis(150)).await;

    let seen = recorder.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 2, "one callback per completed command");

    let (command, in_bytes, out_bytes, is_error) = &seen[0];
    assert_eq!(command, "PING", "the label is what totals cannot give");
    assert_eq!(
        *in_bytes, ping_in,
        "in-bytes match the decoder's frame size"
    );
    assert!(*out_bytes > 0);
    assert!(!*is_error);

    let (command, in_bytes, _, is_error) = &seen[1];
    assert_eq!(command, "NOPE");
    assert_eq!(*in_bytes, bad_in);
    assert!(*is_error, "an Err response is flagged as an error");

    assert_eq!(recorder.opened.load(AtomicOrdering::Relaxed), 1);
    assert_eq!(recorder.closed.load(AtomicOrdering::Relaxed), 1);
}

/// GH #3: the observer's byte counts agree with the built-in totals, because
/// both record at the same point from the same values — the disagreement the
/// issue reported when a product timed inside `dispatch` instead.
#[tokio::test]
async fn observer_and_builtin_metrics_agree() {
    #[derive(Default)]
    struct Sum {
        in_bytes: AtomicUsize,
        out_bytes: AtomicUsize,
    }
    impl crate::server::MetricsObserver for Sum {
        fn command_completed(
            &self,
            _command: &str,
            in_bytes: usize,
            out_bytes: usize,
            _duration: Duration,
            _is_error: bool,
        ) {
            self.in_bytes.fetch_add(in_bytes, AtomicOrdering::Relaxed);
            self.out_bytes.fetch_add(out_bytes, AtomicOrdering::Relaxed);
        }
    }

    let sum = Arc::new(Sum::default());
    let observer: Arc<dyn crate::server::MetricsObserver> = Arc::clone(&sum) as _;
    let (handle, _d) = start_with(no_handshake_profile(), config().with_observer(observer)).await;

    let mut conn = connect(&handle).await;
    for id in 1..=5 {
        let _ = call(&mut conn, id, "PING", vec![]).await;
    }
    tokio::time::sleep(Duration::from_millis(150)).await;

    let snapshot = handle.snapshot();
    assert_eq!(
        sum.in_bytes.load(AtomicOrdering::Relaxed) as u64,
        snapshot.frame_bytes_in_total
    );
    assert_eq!(
        sum.out_bytes.load(AtomicOrdering::Relaxed) as u64,
        snapshot.frame_bytes_out_total
    );
}

/// GH #3: with no observer installed nothing runs and nothing is allocated for
/// the label — the default path is exactly what it was.
#[tokio::test]
async fn no_observer_means_no_observer_work() {
    let (handle, _d) = start(no_handshake_profile()).await;
    assert!(ListenerConfig::default().observer.is_none());
    let mut conn = connect(&handle).await;
    let _ = call(&mut conn, 1, "PING", vec![]).await;
    // The built-in metrics still record, unchanged.
    assert_eq!(handle.snapshot().commands_total, 1);
}

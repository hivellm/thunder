//! Behavioral floor tests for the Thunder client (SPEC-003, feeds the
//! CLT-090 suite): loopback tokio responders built on the thunder::wire
//! codec stand in for `thunder::server` (DAG T1.5) — the client contract
//! is exercised end-to-end over real sockets.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncWriteExt, BufReader};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::TcpListener;

use thunder::wire::config::{ErrorConvention, Handshake, HelloStyle, PushPolicy, TlsPolicy};
use thunder::wire::{read_request_with_limit, write_response, Request, Response, PUSH_ID};
use thunder::{Client, ClientConfig, ClientError, Config, Value};

/// Frame cap the loopback responders read with.
const SRV_CAP: usize = 1024 * 1024;

/// A custom profile (PRO-020): no handshake, push reserved, no error
/// parsing — the neutral baseline the behavioral tests mutate.
fn plain_profile() -> Config {
    Config {
        scheme: "test",
        default_port: 0,
        handshake: Handshake::None,
        hello_style: HelloStyle::NotUsed,
        push: PushPolicy::Reserved,
        max_frame_bytes: SRV_CAP,
        max_in_flight: 64,
        error_codes: ErrorConvention::None,
        tls: TlsPolicy::Off,
    }
}

/// A config with the `AuthCommand` shape and **no** HELLO — the shape a
/// deployment whose RPC path authenticates via `AUTH` uses. Named for the
/// shape: Thunder ships no product configs (PRO-020), so tests build their
/// own exactly as an application does.
fn auth_command_config() -> Config {
    plain_profile()
        .handshake(Handshake::AuthCommand)
        .hello_style(HelloStyle::NotUsed)
        .error_codes(ErrorConvention::Resp3Prefixes)
}

/// The `AuthCommand` shape plus an optional arg-less HELLO.
fn argless_hello_config() -> Config {
    plain_profile()
        .handshake(Handshake::AuthCommand)
        .hello_style(HelloStyle::ArgLess)
        .error_codes(ErrorConvention::Resp3Prefixes)
}

/// The standard `HelloMandatory` + map-payload shape.
fn hello_mandatory_config() -> Config {
    plain_profile()
        .handshake(Handshake::HelloMandatory)
        .hello_style(HelloStyle::MapPayload)
        .error_codes(ErrorConvention::BracketCode)
}

async fn listener() -> (TcpListener, String) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = format!("127.0.0.1:{}", listener.local_addr().unwrap().port());
    (listener, addr)
}

async fn accept_split(listener: &TcpListener) -> (BufReader<OwnedReadHalf>, OwnedWriteHalf) {
    let (stream, _) = listener.accept().await.unwrap();
    let (read_half, write_half) = stream.into_split();
    (BufReader::new(read_half), write_half)
}

async fn read_req(reader: &mut BufReader<OwnedReadHalf>) -> Request {
    read_request_with_limit(reader, SRV_CAP).await.unwrap().0
}

async fn send_ok(writer: &mut OwnedWriteHalf, id: u32, value: Value) {
    write_response(writer, &Response::ok(id, value))
        .await
        .unwrap();
}

async fn send_err(writer: &mut OwnedWriteHalf, id: u32, message: &str) {
    write_response(writer, &Response::err(id, message))
        .await
        .unwrap();
}

fn hello_ok_reply() -> Value {
    Value::Map(vec![(
        Value::Str("authenticated".to_owned()),
        Value::Bool(true),
    )])
}

// ── Multiplexing (CLT-010/011) ──────────────────────────────────────────

#[tokio::test]
async fn pipelined_calls_complete_out_of_order() {
    let (listener, addr) = listener().await;
    let server = tokio::spawn(async move {
        let (mut r, mut w) = accept_split(&listener).await;
        // Read BOTH requests before answering, then answer in reverse:
        // completion order follows the server, not submission order.
        let first = read_req(&mut r).await;
        let second = read_req(&mut r).await;
        assert_ne!(first.id, second.id, "ids must be distinct (CLT-010)");
        send_ok(&mut w, second.id, Value::Str(second.command)).await;
        send_ok(&mut w, first.id, Value::Str(first.command)).await;
    });

    let client = Client::connect(&addr, plain_profile()).await.unwrap();
    let (one, two) = tokio::join!(client.call("ONE", vec![]), client.call("TWO", vec![]));
    assert_eq!(one.unwrap().as_str(), Some("ONE"));
    assert_eq!(two.unwrap().as_str(), Some("TWO"));
    server.await.unwrap();
}

#[tokio::test]
async fn in_flight_bound_backpressures_instead_of_refusing() {
    let (listener, addr) = listener().await;
    let server = tokio::spawn(async move {
        let (mut r, mut w) = accept_split(&listener).await;
        // Strictly serial: with max_in_flight = 1 the second call must
        // wait for the first permit, never be refused (CLT-012).
        for _ in 0..2 {
            let req = read_req(&mut r).await;
            send_ok(&mut w, req.id, Value::Str(req.command)).await;
        }
    });

    let profile = Config {
        max_in_flight: 1,
        ..plain_profile()
    };
    let client = Client::connect(&addr, profile).await.unwrap();
    let (a, b) = tokio::join!(client.call("A", vec![]), client.call("B", vec![]));
    assert_eq!(a.unwrap().as_str(), Some("A"));
    assert_eq!(b.unwrap().as_str(), Some("B"));
    server.await.unwrap();
}

#[tokio::test]
async fn stray_response_id_is_dropped_never_fatal() {
    let (listener, addr) = listener().await;
    let server = tokio::spawn(async move {
        let (mut r, mut w) = accept_split(&listener).await;
        let req = read_req(&mut r).await;
        // A response nobody asked for, then the real one (CLT-013).
        send_ok(&mut w, 9_999, Value::Null).await;
        send_ok(&mut w, req.id, Value::Str("real".to_owned())).await;
    });

    let client = Client::connect(&addr, plain_profile()).await.unwrap();
    let value = client.call("GET", vec![]).await.unwrap();
    assert_eq!(value.as_str(), Some("real"));
    assert_eq!(client.unknown_response_drops(), 1);
    server.await.unwrap();
}

// ── Handshakes (CLT-002/003) ────────────────────────────────────────────

#[tokio::test]
async fn none_handshake_sends_nothing_before_user_calls() {
    let (listener, addr) = listener().await;
    let server = tokio::spawn(async move {
        let (mut r, mut w) = accept_split(&listener).await;
        // The very first frame must be the user's command — no HELLO,
        // no AUTH (Handshake::None).
        let req = read_req(&mut r).await;
        assert_eq!(req.command, "PING");
        send_ok(&mut w, req.id, Value::Str("PONG".to_owned())).await;
    });

    // `plain_profile()` is the genuine Handshake::None case. (This test used
    // to ride on auth_command_config(), which is `AuthCommand` since BN-023.)
    let client = Client::connect(&addr, plain_profile()).await.unwrap();
    assert!(!client.is_authenticated());
    let pong = client.call("PING", vec![]).await.unwrap();
    assert_eq!(pong.as_str(), Some("PONG"));
    server.await.unwrap();
}

/// The client half of the shape/policy split, on the profile BN-023 changed:
/// `synap` is `AuthCommand` now, but with no credentials configured it sends
/// no `AUTH` at all — exactly right against an open deployment
/// (`require_auth` off). It must also never send `HELLO` (`HelloStyle::NotUsed`).
#[tokio::test]
async fn synap_profile_without_credentials_sends_nothing() {
    let (listener, addr) = listener().await;
    let server = tokio::spawn(async move {
        let (mut r, mut w) = accept_split(&listener).await;
        let req = read_req(&mut r).await;
        assert_eq!(
            req.command, "PING",
            "no AUTH/HELLO frame without credentials"
        );
        send_ok(&mut w, req.id, Value::Str("PONG".to_owned())).await;
    });

    let client = Client::connect(&addr, auth_command_config()).await.unwrap();
    assert!(!client.is_authenticated());
    let pong = client.call("PING", vec![]).await.unwrap();
    assert_eq!(pong.as_str(), Some("PONG"));
    server.await.unwrap();
}

#[tokio::test]
async fn auth_command_handshake_sends_hello_then_auth_api_key() {
    let (listener, addr) = listener().await;
    let server = tokio::spawn(async move {
        let (mut r, mut w) = accept_split(&listener).await;
        let hello = read_req(&mut r).await;
        assert_eq!(hello.command, "HELLO");
        assert_eq!(
            hello.args,
            Vec::<Value>::new(),
            "Nexus RPC HELLO takes no arguments — the positional [Int(1)] is \
             the RESP3 HELLO, a different surface (BN-023 errata)"
        );
        send_ok(&mut w, hello.id, Value::Null).await;
        let auth = read_req(&mut r).await;
        assert_eq!(auth.command, "AUTH");
        assert_eq!(auth.args, vec![Value::Str("k-123".to_owned())]);
        send_ok(&mut w, auth.id, Value::Str("OK".to_owned())).await;
        let ping = read_req(&mut r).await;
        assert_eq!(ping.command, "PING");
        send_ok(&mut w, ping.id, Value::Str("PONG".to_owned())).await;
    });

    let config = ClientConfig::new().api_key("k-123");
    let client = Client::connect_with(&addr, argless_hello_config(), config)
        .await
        .unwrap();
    assert!(client.is_authenticated());
    let pong = client.call("PING", vec![]).await.unwrap();
    assert_eq!(pong.as_str(), Some("PONG"));
    server.await.unwrap();
}

/// BN-023 regression: the `synap` profile must be able to authenticate.
///
/// It used to be `Handshake::None`, so a credentialed client sent **nothing**
/// and could never reach a `require_auth` Synap. Synap's RPC path has an `AUTH`
/// handler (and no `HELLO` handler), so the profile is `AuthCommand` +
/// `HelloStyle::NotUsed`: `AUTH` goes out, `HELLO` never does.
#[tokio::test]
async fn synap_profile_sends_auth_and_never_hello() {
    let (listener, addr) = listener().await;
    let server = tokio::spawn(async move {
        let (mut r, mut w) = accept_split(&listener).await;
        // First frame must be AUTH — Synap has no HELLO handler at all.
        let auth = read_req(&mut r).await;
        assert_eq!(auth.command, "AUTH", "first frame must be AUTH, not HELLO");
        assert_eq!(
            auth.args,
            vec![
                Value::Str("root".to_owned()),
                Value::Str("hunter2".to_owned())
            ],
            "Synap's AUTH <user> <password> form"
        );
        send_ok(&mut w, auth.id, Value::Str("OK".to_owned())).await;
        let ping = read_req(&mut r).await;
        assert_eq!(ping.command, "PING");
        send_ok(&mut w, ping.id, Value::Str("PONG".to_owned())).await;
    });

    let config = ClientConfig::new().user_pass("root", "hunter2");
    let client = Client::connect_with(&addr, auth_command_config(), config)
        .await
        .unwrap();
    assert!(client.is_authenticated());
    let pong = client.call("PING", vec![]).await.unwrap();
    assert_eq!(pong.as_str(), Some("PONG"));
    server.await.unwrap();
}

#[tokio::test]
async fn auth_command_handshake_sends_user_pass() {
    let (listener, addr) = listener().await;
    let server = tokio::spawn(async move {
        let (mut r, mut w) = accept_split(&listener).await;
        let hello = read_req(&mut r).await;
        assert_eq!(hello.command, "HELLO");
        send_ok(&mut w, hello.id, Value::Null).await;
        let auth = read_req(&mut r).await;
        assert_eq!(auth.command, "AUTH");
        assert_eq!(
            auth.args,
            vec![
                Value::Str("admin".to_owned()),
                Value::Str("hunter2".to_owned())
            ]
        );
        send_ok(&mut w, auth.id, Value::Str("OK".to_owned())).await;
    });

    let config = ClientConfig::new().user_pass("admin", "hunter2");
    let client = Client::connect_with(&addr, argless_hello_config(), config)
        .await
        .unwrap();
    assert!(client.is_authenticated());
    server.await.unwrap();
}

#[tokio::test]
async fn auth_command_without_credentials_sends_nothing() {
    let (listener, addr) = listener().await;
    let server = tokio::spawn(async move {
        let (mut r, mut w) = accept_split(&listener).await;
        let req = read_req(&mut r).await;
        assert_eq!(req.command, "PING", "no HELLO/AUTH without credentials");
        send_ok(&mut w, req.id, Value::Str("PONG".to_owned())).await;
    });

    let client = Client::connect(&addr, argless_hello_config())
        .await
        .unwrap();
    client.call("PING", vec![]).await.unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn hello_mandatory_sends_hello_map_first_and_exposes_capabilities() {
    let (listener, addr) = listener().await;
    let server = tokio::spawn(async move {
        let (mut r, mut w) = accept_split(&listener).await;
        let hello = read_req(&mut r).await;
        assert_eq!(hello.command, "HELLO", "HELLO must be the first frame");
        let map = &hello.args[0];
        assert_eq!(map.map_get("version").and_then(Value::as_int), Some(1));
        assert_eq!(
            map.map_get("token").and_then(Value::as_str),
            Some("tok-1"),
            "token credential goes in the HELLO map"
        );
        assert_eq!(
            map.map_get("client_name").and_then(Value::as_str),
            Some("itest")
        );
        send_ok(
            &mut w,
            hello.id,
            Value::Map(vec![
                (Value::Str("authenticated".to_owned()), Value::Bool(true)),
                (
                    Value::Str("capabilities".to_owned()),
                    Value::Array(vec![
                        Value::Str("search".to_owned()),
                        Value::Str("insert".to_owned()),
                    ]),
                ),
            ]),
        )
        .await;
    });

    let config = ClientConfig::new().token("tok-1").client_name("itest");
    let client = Client::connect_with(&addr, hello_mandatory_config(), config)
        .await
        .unwrap();
    assert!(client.is_authenticated());
    assert_eq!(client.capabilities(), ["search", "insert"]);
    server.await.unwrap();
}

#[tokio::test]
async fn handshake_rejection_is_a_typed_auth_error() {
    let (listener, addr) = listener().await;
    let server = tokio::spawn(async move {
        let (mut r, mut w) = accept_split(&listener).await;
        let hello = read_req(&mut r).await;
        send_err(&mut w, hello.id, "[unauthorized] invalid api key").await;
    });

    let config = ClientConfig::new().api_key("wrong");
    let err = Client::connect_with(&addr, hello_mandatory_config(), config)
        .await
        .unwrap_err();
    // CLT-003: an auth failure is the auth class, not a generic error.
    let ClientError::Auth { message } = err else {
        panic!("expected the auth class, got {err:?}");
    };
    assert!(message.contains("unauthorized"), "{message}");
    server.await.unwrap();
}

// ── Timeouts (CLT-020) ──────────────────────────────────────────────────

#[tokio::test]
async fn per_call_timeout_fires_and_late_response_is_dropped() {
    let (listener, addr) = listener().await;
    let server = tokio::spawn(async move {
        let (mut r, mut w) = accept_split(&listener).await;
        let slow = read_req(&mut r).await;
        // Answer nothing until the *next* request proves the timeout
        // fired client-side; then deliver the late response first.
        let next = read_req(&mut r).await;
        send_ok(&mut w, slow.id, Value::Str("late".to_owned())).await;
        send_ok(&mut w, next.id, Value::Str("fresh".to_owned())).await;
    });

    let client = Client::connect(&addr, plain_profile()).await.unwrap();
    let err = client
        .call_with_timeout("SLOW", vec![], Duration::from_millis(100))
        .await
        .unwrap_err();
    assert_eq!(err, ClientError::Timeout);
    // The pending entry was removed (CLT-020); the late response falls
    // under the unknown-id drop (CLT-013) and the connection lives on.
    let fresh = client.call("NEXT", vec![]).await.unwrap();
    assert_eq!(fresh.as_str(), Some("fresh"));
    assert_eq!(client.unknown_response_drops(), 1);
    server.await.unwrap();
}

// ── Reconnection (CLT-030/031) ──────────────────────────────────────────

#[tokio::test]
async fn reconnect_after_server_drop_succeeds() {
    let (listener, addr) = listener().await;
    let server = tokio::spawn(async move {
        {
            let (mut r, mut w) = accept_split(&listener).await;
            let req = read_req(&mut r).await;
            send_ok(&mut w, req.id, Value::Str("first".to_owned())).await;
        } // connection dropped
        let (mut r, mut w) = accept_split(&listener).await;
        let req = read_req(&mut r).await;
        send_ok(&mut w, req.id, Value::Str("second".to_owned())).await;
    });

    let client = Client::connect(&addr, plain_profile()).await.unwrap();
    assert_eq!(
        client.call("A", vec![]).await.unwrap().as_str(),
        Some("first")
    );
    // Let the reader observe the EOF and mark the connection dead.
    tokio::time::sleep(Duration::from_millis(200)).await;
    // CLT-030: the call finds the connection dead and lazily re-dials.
    assert_eq!(
        client.call("B", vec![]).await.unwrap().as_str(),
        Some("second")
    );
    server.await.unwrap();
}

#[tokio::test]
async fn reconnect_gives_up_after_two_attempts_with_typed_connection_error() {
    let (listener, addr) = listener().await;
    let accepts = Arc::new(AtomicUsize::new(0));
    let server = tokio::spawn({
        let accepts = Arc::clone(&accepts);
        async move {
            {
                // Connection 1: serve the handshake and one call, then drop.
                let (mut r, mut w) = accept_split(&listener).await;
                accepts.fetch_add(1, Ordering::SeqCst);
                let hello = read_req(&mut r).await;
                send_ok(&mut w, hello.id, hello_ok_reply()).await;
                let req = read_req(&mut r).await;
                send_ok(&mut w, req.id, Value::Str("ok".to_owned())).await;
            }
            // Re-dial attempts: accept and slam shut before the
            // HelloMandatory handshake can complete.
            for _ in 0..2 {
                let (stream, _) = listener.accept().await.unwrap();
                accepts.fetch_add(1, Ordering::SeqCst);
                drop(stream);
            }
        }
    });

    let config = ClientConfig::new().api_key("k");
    let client = Client::connect_with(&addr, hello_mandatory_config(), config)
        .await
        .unwrap();
    client.call("PING", vec![]).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;

    let err = client.call("PING", vec![]).await.unwrap_err();
    assert!(
        matches!(err, ClientError::Connection { .. }),
        "expected the connection class after exhausted re-dials, got {err:?}"
    );
    assert_eq!(
        accepts.load(Ordering::SeqCst),
        3,
        "initial connect + exactly 2 re-dial attempts (CLT-030)"
    );
    server.await.unwrap();
}

// ── Error mapping (CLT-050..052) ────────────────────────────────────────

#[tokio::test]
async fn resp3_error_mapping_over_the_wire() {
    let (listener, addr) = listener().await;
    let server = tokio::spawn(async move {
        let (mut r, mut w) = accept_split(&listener).await;
        let req = read_req(&mut r).await;
        send_err(&mut w, req.id, "NOAUTH Authentication required.").await;
        let req = read_req(&mut r).await;
        send_err(&mut w, req.id, "ERR unknown command 'FOO'").await;
    });

    let client = Client::connect(&addr, argless_hello_config())
        .await
        .unwrap();
    let err = client.call("GET", vec![]).await.unwrap_err();
    assert_eq!(
        err,
        ClientError::Auth {
            message: "NOAUTH Authentication required.".to_owned()
        }
    );
    let err = client.call("FOO", vec![]).await.unwrap_err();
    assert_eq!(
        err,
        ClientError::Server {
            message: "ERR unknown command 'FOO'".to_owned(),
            code: None,
        }
    );
    server.await.unwrap();
}

#[tokio::test]
async fn bracket_error_mapping_over_the_wire() {
    let (listener, addr) = listener().await;
    let server = tokio::spawn(async move {
        let (mut r, mut w) = accept_split(&listener).await;
        let hello = read_req(&mut r).await;
        send_ok(&mut w, hello.id, hello_ok_reply()).await;
        let req = read_req(&mut r).await;
        send_err(
            &mut w,
            req.id,
            "[collection_not_found] no such collection: docs",
        )
        .await;
    });

    let client = Client::connect(&addr, hello_mandatory_config())
        .await
        .unwrap();
    let err = client.call("SEARCH", vec![]).await.unwrap_err();
    assert_eq!(
        err,
        ClientError::Server {
            message: "[collection_not_found] no such collection: docs".to_owned(),
            code: Some("collection_not_found".to_owned()),
        }
    );
    server.await.unwrap();
}

// ── Push frames (CLT-060) ───────────────────────────────────────────────

#[tokio::test]
async fn push_frames_route_to_handler_under_enabled() {
    let (listener, addr) = listener().await;
    let server = tokio::spawn(async move {
        let (mut r, mut w) = accept_split(&listener).await;
        let req = read_req(&mut r).await;
        // A push frame in front of the response: it must reach the
        // handler and never be matched against the pending call.
        write_response(&mut w, &Response::ok(PUSH_ID, Value::Str("evt".to_owned())))
            .await
            .unwrap();
        send_ok(&mut w, req.id, Value::Str("PONG".to_owned())).await;
    });

    let profile = Config {
        push: PushPolicy::Enabled,
        ..plain_profile()
    };
    let client = Client::connect(&addr, profile).await.unwrap();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    client.on_push(move |value| {
        let _ = tx.send(value);
    });
    let pong = client.call("SUBSCRIBE", vec![]).await.unwrap();
    assert_eq!(pong.as_str(), Some("PONG"));
    let pushed = rx.recv().await.unwrap();
    assert_eq!(pushed.as_str(), Some("evt"));
    assert_eq!(client.unknown_response_drops(), 0);
    server.await.unwrap();
}

#[tokio::test]
async fn push_frame_under_reserved_profile_poisons_connection() {
    let (listener, addr) = listener().await;
    let server = tokio::spawn(async move {
        {
            let (mut r, mut w) = accept_split(&listener).await;
            let _req = read_req(&mut r).await;
            write_response(&mut w, &Response::ok(PUSH_ID, Value::Null))
                .await
                .unwrap();
            // Keep writing nothing; the client poisons on its own.
        }
        // The next call may reconnect (CLT-014/030): serve it.
        let (mut r, mut w) = accept_split(&listener).await;
        let req = read_req(&mut r).await;
        send_ok(&mut w, req.id, Value::Str("recovered".to_owned())).await;
    });

    let client = Client::connect(&addr, plain_profile()).await.unwrap();
    let err = client.call("GET", vec![]).await.unwrap_err();
    assert!(
        matches!(err, ClientError::Decode { .. }),
        "push under Reserved is a protocol error (CLT-060), got {err:?}"
    );
    // Poisoned connection, lazy reconnect on the next call.
    let value = client.call("GET", vec![]).await.unwrap();
    assert_eq!(value.as_str(), Some("recovered"));
    server.await.unwrap();
}

// ── Poisoning (CLT-014) ─────────────────────────────────────────────────

#[tokio::test]
async fn oversized_inbound_frame_fails_typed_and_poisons() {
    let (listener, addr) = listener().await;
    let server = tokio::spawn(async move {
        {
            let (mut r, mut w) = accept_split(&listener).await;
            let _req = read_req(&mut r).await;
            // A length prefix past the profile cap — the client must
            // refuse on the prefix alone, before any body exists.
            w.write_all(&1_000u32.to_le_bytes()).await.unwrap();
        }
        let (mut r, mut w) = accept_split(&listener).await;
        let req = read_req(&mut r).await;
        send_ok(&mut w, req.id, Value::Str("recovered".to_owned())).await;
    });

    let profile = Config {
        max_frame_bytes: 64,
        ..plain_profile()
    };
    let client = Client::connect(&addr, profile).await.unwrap();
    let err = client.call("GET", vec![]).await.unwrap_err();
    assert!(
        matches!(err, ClientError::FrameTooLarge { .. }),
        "expected the frame-too-large class, got {err:?}"
    );
    let value = client.call("GET", vec![]).await.unwrap();
    assert_eq!(value.as_str(), Some("recovered"));
    server.await.unwrap();
}

#[tokio::test]
async fn malformed_frame_poisons_with_decode_error() {
    let (listener, addr) = listener().await;
    let server = tokio::spawn(async move {
        let (mut r, mut w) = accept_split(&listener).await;
        let _req = read_req(&mut r).await;
        // Valid length prefix, garbage body (0xc1 is never valid
        // MessagePack).
        w.write_all(&4u32.to_le_bytes()).await.unwrap();
        w.write_all(&[0xc1, 0xc1, 0xc1, 0xc1]).await.unwrap();
    });

    let client = Client::connect(&addr, plain_profile()).await.unwrap();
    let err = client.call("GET", vec![]).await.unwrap_err();
    assert!(
        matches!(err, ClientError::Decode { .. }),
        "expected the decode class, got {err:?}"
    );
    server.await.unwrap();
}

// ── Lifecycle (CLT-004) ─────────────────────────────────────────────────

#[tokio::test]
async fn close_is_idempotent_and_fails_in_flight_calls() {
    let (listener, addr) = listener().await;
    let server = tokio::spawn(async move {
        let (mut r, _w) = accept_split(&listener).await;
        // Swallow the request, never answer; wait out the client close.
        let _ = read_request_with_limit(&mut r, SRV_CAP).await;
        let _ = read_request_with_limit(&mut r, SRV_CAP).await;
    });

    let client = Arc::new(Client::connect(&addr, plain_profile()).await.unwrap());
    let pending = tokio::spawn({
        let client = Arc::clone(&client);
        async move { client.call("HANG", vec![]).await }
    });
    tokio::time::sleep(Duration::from_millis(100)).await;

    client.close().await;
    client.close().await; // idempotent (CLT-004)

    let err = pending.await.unwrap().unwrap_err();
    assert!(
        matches!(err, ClientError::Connection { .. }),
        "in-flight calls fail with the typed connection-closed error, got {err:?}"
    );
    let err = client.call("AFTER", vec![]).await.unwrap_err();
    assert!(matches!(err, ClientError::Connection { .. }));
    server.await.unwrap();
}

// ── Endpoints (CLT-070) ─────────────────────────────────────────────────

#[tokio::test]
async fn http_url_is_rejected_at_connect() {
    let err = Client::connect("http://localhost:8080", plain_profile())
        .await
        .unwrap_err();
    let ClientError::Connection { message } = err else {
        panic!("expected the connection class");
    };
    assert!(
        message.contains("RPC-only") && message.contains("HTTP client"),
        "{message}"
    );
}

//! Optional-TLS transport tests (SPEC-008 CAN-020, SRV-040, FR-29). Only
//! compiled with `--features tls`; the plaintext build never sees rustls.
//!
//! Three properties: an encrypted round-trip actually works end to end; the
//! plaintext path is unchanged when TLS is unused; and a cert the client does
//! not trust fails as a `Connection` error, not a hang or a panic.
#![cfg(feature = "tls")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use thunder::server::{
    spawn_listener, AuthError, Credentials, Dispatch, ListenerConfig, Principal, ServerInfo,
    Session,
};
use thunder::wire::config::{ErrorConvention, Handshake, HelloStyle, PushPolicy, TlsPolicy};
use thunder::{Client, ClientConfig, ClientError, ClientTls, Config, ServerTls, Value};

/// A trivial echo engine — PING → PONG, ECHO → its first arg.
struct Echo;

impl Dispatch for Echo {
    type Identity = ();

    async fn dispatch(
        &self,
        _session: &Session,
        command: &str,
        mut args: Vec<Value>,
    ) -> Result<Value, String> {
        match command {
            "PING" => Ok(Value::Str("PONG".to_owned())),
            "ECHO" => Ok(if args.is_empty() {
                Value::Null
            } else {
                args.swap_remove(0)
            }),
            other => Err(format!("ERR unknown command '{other}'")),
        }
    }

    async fn authenticate(&self, _creds: Credentials) -> Result<Principal, AuthError> {
        Ok(Principal::new("tls-test".to_owned()))
    }
}

/// A no-handshake profile — the test exercises the transport, not auth.
fn profile() -> Config {
    Config {
        scheme: "test",
        default_port: 0,
        handshake: Handshake::None,
        hello_style: HelloStyle::NotUsed,
        push: PushPolicy::Reserved,
        max_frame_bytes: 1024 * 1024,
        max_in_flight: 64,
        error_codes: ErrorConvention::None,
        // The wire-config TLS *policy* signal; the actual transport TLS is
        // driven by the listener/client configs below (PRO-003: config is data).
        tls: TlsPolicy::Optional,
    }
}

fn info() -> ServerInfo {
    ServerInfo {
        name: "tls-test".to_owned(),
        version: "0".to_owned(),
    }
}

fn loopback() -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], 0))
}

/// A fresh self-signed cert/key for `localhost`, PEM-encoded.
fn self_signed() -> (String, String) {
    let signed = rcgen::generate_simple_self_signed(vec!["localhost".to_owned()]).unwrap();
    (signed.cert.pem(), signed.key_pair.serialize_pem())
}

/// Write PEM to a uniquely-named temp file (no `Date`/`rand` in tests here).
fn write_temp(contents: &str, suffix: &str) -> PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let path =
        std::env::temp_dir().join(format!("thunder-tls-{}-{n}-{suffix}", std::process::id()));
    std::fs::write(&path, contents).unwrap();
    path
}

#[tokio::test]
async fn tls_round_trip_encrypts_request_and_response() {
    let (cert_pem, key_pem) = self_signed();
    let cert_path = write_temp(&cert_pem, "cert.pem");
    let key_path = write_temp(&key_pem, "key.pem");

    let handle = spawn_listener(
        Arc::new(Echo),
        profile(),
        info(),
        ListenerConfig::new(loopback()).with_tls(ServerTls {
            cert_path: cert_path.clone(),
            key_path: key_path.clone(),
        }),
    )
    .await
    .unwrap();
    let addr = handle.local_addr().to_string();

    // The client trusts exactly this self-signed cert and verifies the SAN
    // `localhost`.
    let client = Client::connect_with(
        &addr,
        profile(),
        ClientConfig::new().with_tls(ClientTls {
            server_name: Some("localhost".to_owned()),
            ca_path: Some(cert_path.clone()),
        }),
    )
    .await
    .unwrap();

    assert_eq!(
        client.call("PING", vec![]).await.unwrap().as_str(),
        Some("PONG")
    );
    assert_eq!(
        client
            .call("ECHO", vec![Value::Str("secret-over-tls".to_owned())])
            .await
            .unwrap()
            .as_str(),
        Some("secret-over-tls")
    );

    client.close().await;
    handle.stop().await;
    let _ = std::fs::remove_file(cert_path);
    let _ = std::fs::remove_file(key_path);
}

#[tokio::test]
async fn plaintext_still_works_when_tls_is_unused() {
    // Same real server/client stack, no TLS configured on either end — proves
    // the default path is unchanged with the feature compiled in.
    let handle = spawn_listener(
        Arc::new(Echo),
        profile(),
        info(),
        ListenerConfig::new(loopback()),
    )
    .await
    .unwrap();
    let addr = handle.local_addr().to_string();

    let client = Client::connect(&addr, profile()).await.unwrap();
    assert_eq!(
        client.call("PING", vec![]).await.unwrap().as_str(),
        Some("PONG")
    );

    client.close().await;
    handle.stop().await;
}

#[tokio::test]
async fn cert_mismatch_is_a_connection_error() {
    let (server_cert, server_key) = self_signed();
    // A DIFFERENT self-signed cert the client will trust instead of the
    // server's — verification must fail.
    let (other_cert, _other_key) = self_signed();
    let cert_path = write_temp(&server_cert, "cert.pem");
    let key_path = write_temp(&server_key, "key.pem");
    let wrong_ca = write_temp(&other_cert, "wrongca.pem");

    let handle = spawn_listener(
        Arc::new(Echo),
        profile(),
        info(),
        ListenerConfig::new(loopback()).with_tls(ServerTls {
            cert_path: cert_path.clone(),
            key_path: key_path.clone(),
        }),
    )
    .await
    .unwrap();
    let addr = handle.local_addr().to_string();

    let err = Client::connect_with(
        &addr,
        profile(),
        ClientConfig::new().with_tls(ClientTls {
            server_name: Some("localhost".to_owned()),
            ca_path: Some(wrong_ca.clone()),
        }),
    )
    .await
    .unwrap_err();
    // FR-29: a TLS/verification failure is the Connection class.
    assert!(
        matches!(err, ClientError::Connection { .. }),
        "expected a Connection error from an untrusted cert, got {err:?}"
    );

    handle.stop().await;
    let _ = std::fs::remove_file(cert_path);
    let _ = std::fs::remove_file(key_path);
    let _ = std::fs::remove_file(wrong_ca);
}

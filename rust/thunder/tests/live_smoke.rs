//! Live interop smoke (TST-050) — the release-path check that the four-language
//! floor cannot cover: a Thunder client against a **real** product instance.
//!
//! Env-gated and skipped by default. Set any of
//! `THUNDER_LIVE_URL_SYNAP` / `_NEXUS` / `_VECTORIZER` to a reachable RPC
//! endpoint (e.g. `synap://host:port`) and this connects with that product's
//! deployment shape, makes a PING-class call, makes one typed-error call, and
//! closes cleanly. With none set it prints what it skipped and passes — it is
//! not part of the always-on floor.
//!
//! The per-product shapes are the BN-023 errata facts (product SOURCE is the
//! truth): Synap authenticates via bare `AUTH` (no HELLO), Nexus offers an
//! arg-less `HELLO`, Vectorizer leads with the standard `HELLO` map; none run
//! RPC TLS. Thunder ships no product configs (PRO-010), so — exactly as a
//! deployment does — this test builds each shape from `Config::standard()`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use thunder::wire::config::{ErrorConvention, Handshake, HelloStyle};
use thunder::{Client, ClientConfig, Config};

fn synap_shape() -> Config {
    Config::standard()
        .scheme("synap")
        .handshake(Handshake::AuthCommand)
        .hello_style(HelloStyle::NotUsed)
        .error_codes(ErrorConvention::Resp3Prefixes)
}

fn nexus_shape() -> Config {
    Config::standard()
        .scheme("nexus")
        .handshake(Handshake::AuthCommand)
        .hello_style(HelloStyle::ArgLess)
        .error_codes(ErrorConvention::Resp3Prefixes)
}

fn vectorizer_shape() -> Config {
    // The standard shape (HelloMandatory + map payload); `Both` error codes
    // parse Vectorizer's bracket form.
    Config::standard().scheme("vectorizer")
}

/// Connect, one PING-class call, one typed-error call, clean close.
async fn smoke(url: &str, config: Config) -> Result<(), String> {
    let client = Client::connect_with(
        url,
        config,
        ClientConfig::new().client_name("thunder-live-smoke"),
    )
    .await
    .map_err(|e| format!("connect/handshake failed: {e}"))?;

    // A PING-class call must succeed (pre-auth allowlisted, SRV-011).
    client
        .call("PING", vec![])
        .await
        .map_err(|e| format!("PING failed: {e}"))?;

    // A command no product implements must come back a TYPED error — an auth
    // refusal under a require-auth deployment, or a server error — never a hang
    // or a panic.
    if let Ok(value) = client.call("__thunder_live_smoke_unknown__", vec![]).await {
        return Err(format!(
            "the bogus command returned ok {value:?}, expected a typed error"
        ));
    }

    client.close().await;
    Ok(())
}

/// One product's env var and the deployment shape a client dials it with.
type Product = (&'static str, fn() -> Config);

#[tokio::test]
async fn live_interop_smoke() {
    let products: [Product; 3] = [
        ("THUNDER_LIVE_URL_SYNAP", synap_shape),
        ("THUNDER_LIVE_URL_NEXUS", nexus_shape),
        ("THUNDER_LIVE_URL_VECTORIZER", vectorizer_shape),
    ];

    let mut ran = 0;
    for (env, shape) in products {
        match std::env::var(env) {
            Ok(url) if !url.is_empty() => {
                if let Err(why) = smoke(&url, shape()).await {
                    panic!("{env} ({url}): {why}");
                }
                eprintln!("live smoke: {env} ({url}) — OK");
                ran += 1;
            }
            _ => eprintln!("live smoke: {env} unset — skipped (release-path only, TST-050)"),
        }
    }
    if ran == 0 {
        eprintln!(
            "live smoke: no THUNDER_LIVE_URL_* set — nothing to run \
             (expected off the release path)"
        );
    }
}

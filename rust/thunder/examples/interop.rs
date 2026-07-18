//! Cross-language live-interop probe (Rust). Dual mode:
//!
//! ```text
//! cargo run -q -p thunder-rpc --example interop -- server <port>
//! cargo run -q -p thunder-rpc --example interop -- client <port>
//! ```
//!
//! The `interop/run.py` driver pairs every language's server with every
//! language's client over a real socket. The server prints `READY` once bound;
//! the client prints `OK` and exits 0 on success, or `FAIL: <why>` and exits 1.
//! Every probe speaks the family standard config (mandatory `HELLO` map +
//! capabilities reply), so the handshake itself is part of what interops.

use std::io::Write;
use std::sync::Arc;

use thunder::server::{
    spawn_listener, AuthError, Credentials, Dispatch, ListenerConfig, Principal, ServerInfo,
    Session,
};
use thunder::{Client, ClientConfig, Config, Value};

const SCHEME: &str = "interop";

fn app_config() -> Config {
    Config::standard().scheme(SCHEME).port(0)
}

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
            "ECHO" if !args.is_empty() => Ok(args.swap_remove(0)),
            other => Err(format!("ERR unknown command '{other}'")),
        }
    }

    async fn authenticate(&self, _creds: Credentials) -> Result<Principal, AuthError> {
        Ok(Principal::new("interop".to_owned()))
    }
}

async fn run_server(port: u16) -> Result<(), Box<dyn std::error::Error>> {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let handle = spawn_listener(
        Arc::new(Echo),
        app_config(),
        ServerInfo {
            name: "rust-interop".to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
        },
        ListenerConfig::new(addr).open(),
    )
    .await?;
    // Signal the driver that the port is bound, then serve until killed.
    println!("READY");
    std::io::stdout().flush().ok();
    std::future::pending::<()>().await;
    drop(handle);
    Ok(())
}

async fn run_client(port: u16) -> Result<(), String> {
    let addr = format!("127.0.0.1:{port}");
    let client = Client::connect_with(&addr, app_config(), ClientConfig::new().client_name("rust"))
        .await
        .map_err(|e| format!("connect/handshake failed: {e}"))?;

    let pong = client
        .call("PING", vec![])
        .await
        .map_err(|e| format!("PING failed: {e}"))?;
    if pong.as_str() != Some("PONG") {
        return Err(format!("PING returned {pong:?}, want PONG"));
    }

    let payload = "cross-language-🌩";
    let echo = client
        .call("ECHO", vec![Value::Str(payload.to_owned())])
        .await
        .map_err(|e| format!("ECHO failed: {e}"))?;
    if echo.as_str() != Some(payload) {
        return Err(format!("ECHO returned {echo:?}, want {payload:?}"));
    }

    // An Err is exactly right: NOPE must come back as a typed error.
    if let Ok(v) = client.call("NOPE", vec![]).await {
        return Err(format!("NOPE returned ok {v:?}, want a typed error"));
    }

    client.close().await;
    Ok(())
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let role = args.get(1).map(String::as_str);
    let port: u16 = args
        .get(2)
        .and_then(|p| p.parse().ok())
        .unwrap_or_else(|| fail("usage: interop <server|client> <port>"));

    match role {
        Some("server") => {
            if let Err(e) = run_server(port).await {
                fail(&format!("server error: {e}"));
            }
        }
        Some("client") => match run_client(port).await {
            Ok(()) => {
                println!("OK");
                std::process::exit(0);
            }
            Err(why) => fail(&format!("FAIL: {why}")),
        },
        _ => fail("usage: interop <server|client> <port>"),
    }
}

fn fail(msg: &str) -> ! {
    eprintln!("{msg}");
    std::process::exit(1);
}

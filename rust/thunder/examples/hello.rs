//! The smallest end-to-end Thunder: start a server, connect a client, make a
//! few calls. Run it with:
//!
//! ```text
//! cargo run -p thunder-rpc --example hello
//! ```
//!
//! It uses the family standard config (a mandatory `HELLO` handshake with a
//! capabilities reply), an open deployment (no credentials required), on a
//! loopback ephemeral port — the same stack the products speak, in ~40 lines.

use std::sync::Arc;

use thunder::server::{
    spawn_listener, AuthError, Credentials, Dispatch, ListenerConfig, Principal, ServerInfo,
    Session,
};
use thunder::{Client, ClientConfig, Config, Value};

/// A trivial echo engine: `PING` → `PONG`, `ECHO` → its argument, anything
/// else → a typed server error. A real product implements this one trait over
/// its own engine; Thunder owns everything below it.
struct Echo;

impl Dispatch for Echo {
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
        Ok(Principal {
            name: "hello".to_owned(),
        })
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // The standard config plus this app's identity; open (no auth required).
    let app = Config::standard().scheme("hello").port(0);

    let server = spawn_listener(
        Arc::new(Echo),
        app.clone(),
        ServerInfo {
            name: "hello-server".to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
        },
        ListenerConfig::default().open(),
    )
    .await?;
    let addr = server.local_addr().to_string();
    println!("server listening on {addr}");

    let client =
        Client::connect_with(&addr, app, ClientConfig::new().client_name("hello-client")).await?;
    println!(
        "client connected (handshake done, authenticated = {})",
        client.is_authenticated()
    );

    let pong = client.call("PING", vec![]).await?;
    println!("PING  -> {:?}", pong.as_str());

    let echo = client
        .call("ECHO", vec![Value::Str("hello, thunder".to_owned())])
        .await?;
    println!("ECHO  -> {:?}", echo.as_str());

    match client.call("NOPE", vec![]).await {
        Ok(v) => println!("NOPE  -> unexpected ok: {v:?}"),
        Err(e) => println!("NOPE  -> typed error (as designed): {e}"),
    }

    client.close().await;
    server.stop().await;
    println!("done.");
    Ok(())
}

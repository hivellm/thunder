//! Connection-pool behavior (CLT-080). The pool is a layer above the
//! single-connection client; these tests exercise it end to end over a real
//! `thunder::server` — checkout/return, the capacity bound, the poison drop,
//! and the property the whole layer exists for: `N` operations pay **one**
//! connection and **one** handshake, not `N`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use thunder::client::Pool;
use thunder::server::{
    spawn_listener, AuthError, Credentials, Dispatch, ListenerConfig, Principal, ServerInfo,
    Session,
};
use thunder::{ClientConfig, Config, Value};

/// Echo backend that records the distinct connection ids it served — so a test
/// can prove how many connections (hence handshakes) `N` operations really used.
#[derive(Default)]
struct Counting {
    connection_ids: StdMutex<HashSet<u64>>,
    calls: AtomicU64,
}

impl Counting {
    fn distinct_connections(&self) -> usize {
        self.connection_ids.lock().unwrap().len()
    }
}

impl Dispatch for Counting {
    type Identity = ();

    async fn dispatch(
        &self,
        session: &Session,
        command: &str,
        _args: Vec<Value>,
    ) -> Result<Value, String> {
        self.connection_ids
            .lock()
            .unwrap()
            .insert(session.connection_id());
        self.calls.fetch_add(1, Ordering::Relaxed);
        match command {
            "PING" => Ok(Value::Str("PONG".to_owned())),
            other => Err(format!("ERR unknown command '{other}'")),
        }
    }

    async fn authenticate(&self, _creds: Credentials) -> Result<Principal, AuthError> {
        Ok(Principal::new("pool-test".to_owned()))
    }
}

/// The standard profile (mandatory HELLO map + capabilities reply), so a real
/// handshake happens on every new connection. `.open()` on the listener means
/// no credentials are required — the test measures connection reuse, not auth.
fn profile() -> Config {
    Config::standard().scheme("test").port(0)
}

fn info() -> ServerInfo {
    ServerInfo {
        name: "pool-test".to_owned(),
        version: "0".to_owned(),
    }
}

async fn serve(backend: Arc<Counting>) -> thunder::server::ListenerHandle {
    spawn_listener(backend, profile(), info(), ListenerConfig::default().open())
        .await
        .unwrap()
}

#[tokio::test]
async fn checkout_returns_the_connection_for_reuse() {
    let backend = Arc::new(Counting::default());
    let handle = serve(Arc::clone(&backend)).await;
    let pool = Pool::new(
        handle.local_addr().to_string(),
        profile(),
        ClientConfig::new().client_name("pool-test"),
        4,
    );

    assert_eq!(pool.idle_count(), 0, "construction dials nothing");
    {
        let conn = pool.acquire().await.unwrap();
        assert_eq!(
            conn.call("PING", vec![]).await.unwrap().as_str(),
            Some("PONG")
        );
        assert_eq!(pool.idle_count(), 0, "checked out, so not idle");
    }
    // The guard dropped: the connection returned to the pool.
    assert_eq!(pool.idle_count(), 1, "returned on drop");

    handle.stop().await;
}

#[tokio::test]
async fn n_operations_use_one_connection_and_handshake() {
    let backend = Arc::new(Counting::default());
    let handle = serve(Arc::clone(&backend)).await;
    let pool = Pool::new(
        handle.local_addr().to_string(),
        profile(),
        ClientConfig::new().client_name("pool-test"),
        4,
    );

    for _ in 0..10 {
        let conn = pool.acquire().await.unwrap();
        assert_eq!(
            conn.call("PING", vec![]).await.unwrap().as_str(),
            Some("PONG")
        );
    }

    // The whole point of the layer: ten sequential operations reused one
    // connection, so the server saw one handshake, not ten.
    assert_eq!(backend.calls.load(Ordering::Relaxed), 10);
    assert_eq!(
        backend.distinct_connections(),
        1,
        "ten operations must ride one connection (one handshake), not ten"
    );

    handle.stop().await;
}

#[tokio::test]
async fn pool_never_exceeds_max_connections() {
    let backend = Arc::new(Counting::default());
    let handle = serve(Arc::clone(&backend)).await;
    let pool = Pool::new(
        handle.local_addr().to_string(),
        profile(),
        ClientConfig::new().client_name("pool-test"),
        2,
    );

    let a = pool.acquire().await.unwrap();
    let b = pool.acquire().await.unwrap();

    // With both permits held, a third checkout must wait, not open a third
    // connection (CLT-080 fixed N).
    let mut third = Box::pin(pool.acquire());
    assert!(
        tokio::time::timeout(Duration::from_millis(150), &mut third)
            .await
            .is_err(),
        "third checkout must block while max_connections are held"
    );

    // Release one; the waiter now completes.
    drop(a);
    let c = tokio::time::timeout(Duration::from_millis(1000), &mut third)
        .await
        .expect("third checkout should proceed once a slot frees")
        .unwrap();
    assert_eq!(c.call("PING", vec![]).await.unwrap().as_str(), Some("PONG"));

    // At most two connections ever existed.
    assert!(backend.distinct_connections() <= 2);

    drop(b);
    drop(c);
    handle.stop().await;
}

#[tokio::test]
async fn a_poisoned_connection_is_not_handed_to_the_next_caller() {
    let backend = Arc::new(Counting::default());
    let handle = serve(Arc::clone(&backend)).await;
    let pool = Pool::new(
        handle.local_addr().to_string(),
        profile(),
        ClientConfig::new().client_name("pool-test"),
        4,
    );

    {
        let conn = pool.acquire().await.unwrap();
        assert_eq!(
            conn.call("PING", vec![]).await.unwrap().as_str(),
            Some("PONG")
        );
        // Kill this connection, then let the guard drop.
        conn.close().await;
        assert!(!conn.is_alive());
    }
    // CLT-014: the dead connection was dropped, not parked for reuse.
    assert_eq!(
        pool.idle_count(),
        0,
        "a poisoned connection must not return to the pool"
    );

    // The next checkout dials a fresh, working connection.
    let fresh = pool.acquire().await.unwrap();
    assert_eq!(
        fresh.call("PING", vec![]).await.unwrap().as_str(),
        Some("PONG")
    );

    handle.stop().await;
}

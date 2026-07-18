//! Optional connection pool (CLT-080) — a layer **above** the
//! single-connection [`Client`] (CLT-001: "pooling is a layer above").
//!
//! Under a mandatory-`HELLO` profile with `auth_required`, a fresh connection
//! costs a handshake round trip before the first request. A caller that opens a
//! connection per operation therefore pays that round trip every time — the
//! failure Nexus's per-request REST client and Vectorizer's per-RPC Raft
//! channel each hit as TIME_WAIT port exhaustion. The pool amortizes it: `N`
//! operations over a checked-out connection pay **one** connect and **one**
//! handshake, not `N`.
//!
//! The shape is deliberately minimal — a fixed number of connections bounded by
//! a semaphore, an idle list, lazy connect on first checkout, and an RAII guard
//! that returns the connection on drop. It is **not** `bb8`/`deadpool`/`r2d2`:
//! those bring async traits and heavyweight reconnect logic this layer does not
//! need. Health checks, background reaping and min-idle warmup are out of scope;
//! a poisoned connection (CLT-014) is dropped on return and the next checkout
//! connects fresh, leaving reconnect to CLT-030 rather than the pool.
//!
//! The pool adds **no wire behavior**: it builds the same [`Client`] as
//! [`Client::connect_with`] from a [`Config`] and [`ClientConfig`], and the
//! single-connection client's API is unchanged (CLT-001). `max_in_flight`
//! (CLT-012) stays a per-connection bound; the pool bounds connections, not
//! in-flight calls.
//!
//! ```no_run
//! use thunder::{ClientConfig, Config};
//! use thunder::client::Pool;
//!
//! # async fn demo() -> Result<(), thunder::ClientError> {
//! let app = Config::standard().scheme("myapp").port(9000);
//! let pool = Pool::new("myapp://localhost", app, ClientConfig::new(), 8);
//! let conn = pool.acquire().await?; // reuses an idle connection, or dials one
//! let pong = conn.call("PING", vec![]).await?;
//! assert_eq!(pong.as_str(), Some("PONG"));
//! // `conn` returns the connection to the pool when it drops.
//! # Ok(())
//! # }
//! ```

use std::sync::{Arc, Mutex as StdMutex, MutexGuard, PoisonError};

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::client::{Client, ClientConfig, ClientError};
use crate::wire::Config;

/// Ride through std-mutex poisoning: a panicked holder must not wedge the pool.
fn lock<T>(mutex: &StdMutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A bounded pool of [`Client`]s over one endpoint (CLT-080).
///
/// At most `max_connections` connections are live at once; a checkout beyond
/// that awaits a return. Connections are dialed lazily — construction opens
/// none — and reused across checkouts so the handshake is paid once per
/// connection, not once per operation.
pub struct Pool {
    endpoint: String,
    config: Config,
    client_config: ClientConfig,
    /// Bounds live + checked-out connections to `max_connections`.
    permits: Arc<Semaphore>,
    /// Idle connections available for reuse.
    idle: Arc<StdMutex<Vec<Client>>>,
}

impl Pool {
    /// Build a pool for `endpoint`. Opens no connections — the first
    /// [`acquire`](Self::acquire) dials the first one. `max_connections` is
    /// clamped to at least 1.
    pub fn new(
        endpoint: impl Into<String>,
        config: Config,
        client_config: ClientConfig,
        max_connections: usize,
    ) -> Self {
        let max = max_connections.max(1);
        Self {
            endpoint: endpoint.into(),
            config,
            client_config,
            permits: Arc::new(Semaphore::new(max)),
            idle: Arc::new(StdMutex::new(Vec::with_capacity(max))),
        }
    }

    /// Check out a connection. Reuses an idle, **live** connection when one is
    /// available; otherwise dials and handshakes a fresh one (CLT-002). Awaits a
    /// return when `max_connections` are already checked out. The returned
    /// [`PooledConn`] returns the connection to the pool on drop.
    pub async fn acquire(&self) -> Result<PooledConn, ClientError> {
        let permit = Arc::clone(&self.permits)
            .acquire_owned()
            .await
            .map_err(|_| ClientError::Connection {
                message: "connection pool is closed".to_owned(),
            })?;

        // Reuse the newest idle connection that is still live; discard any that
        // were poisoned (CLT-014) while sitting idle.
        let reused = {
            let mut idle = lock(&self.idle);
            loop {
                match idle.pop() {
                    Some(client) if client.is_alive() => break Some(client),
                    Some(_dead) => continue,
                    None => break None,
                }
            }
        };
        let client = match reused {
            Some(client) => client,
            None => {
                Client::connect_with(
                    &self.endpoint,
                    self.config.clone(),
                    self.client_config.clone(),
                )
                .await?
            }
        };

        Ok(PooledConn {
            inner: Some(client),
            idle: Arc::clone(&self.idle),
            _permit: permit,
        })
    }

    /// Idle connections currently parked in the pool. For diagnostics and tests
    /// — production code should not branch on it.
    pub fn idle_count(&self) -> usize {
        lock(&self.idle).len()
    }
}

/// RAII guard from [`Pool::acquire`]. Derefs to the [`Client`], and returns the
/// connection to the pool on drop so the next checkout reuses it — unless the
/// connection was poisoned, in which case it is dropped and the next checkout
/// connects fresh (CLT-014/030).
pub struct PooledConn {
    /// `Some` for the guard's whole life; taken only in [`Drop`].
    inner: Option<Client>,
    idle: Arc<StdMutex<Vec<Client>>>,
    /// Held for the checkout's duration; releasing it lets a waiter proceed.
    _permit: OwnedSemaphorePermit,
}

impl PooledConn {
    /// Borrow the checked-out client. (Also available via [`Deref`].)
    pub fn client(&self) -> &Client {
        match &self.inner {
            Some(client) => client,
            // Unreachable: `inner` is only taken in `Drop`, after which no
            // method can be called on the guard.
            None => unreachable!("PooledConn::client after drop"),
        }
    }
}

impl std::ops::Deref for PooledConn {
    type Target = Client;

    fn deref(&self) -> &Client {
        self.client()
    }
}

impl Drop for PooledConn {
    fn drop(&mut self) {
        if let Some(client) = self.inner.take() {
            // CLT-014: only a live connection returns to the pool. A poisoned or
            // closed one is dropped here; the next checkout dials fresh, leaving
            // reconnect to CLT-030 rather than the pool.
            if client.is_alive() {
                lock(&self.idle).push(client);
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn new_does_not_dial_and_clamps_capacity() {
        let pool = Pool::new(
            "test://127.0.0.1:0",
            Config::standard(),
            ClientConfig::new(),
            0,
        );
        // No connection opened at construction, and max clamped to >= 1.
        assert_eq!(pool.idle_count(), 0);
        assert_eq!(pool.permits.available_permits(), 1);
    }
}

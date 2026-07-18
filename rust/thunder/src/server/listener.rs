//! The accept loop, per-connection shape and hot path (SPEC-004 §1/§1b).
//!
//! Hot path transplanted from the Synap listener (§7 baseline analysis,
//! T-027): `set_nodelay(true)` on accept (SRV-008), a dedicated writer task
//! owning a `BufWriter` over the write half behind an mpsc channel
//! (SRV-002), and the drain-then-flush pattern — write one response, drain
//! every already-queued response via `try_recv`, then flush once, so a
//! pipelined burst coalesces into one syscall (SRV-006, +23% committed
//! in-family evidence). Exactly one serialization per response: the frame
//! is encoded once, written, and its length is the out-bytes metric;
//! request in-bytes come from the decoder's frame size (SRV-007).

use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex as StdMutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use crate::wire::config::{Config, ErrorConvention, Handshake, HelloStyle, PushPolicy};
use crate::wire::{encode_frame, read_request_with_limit, Request, Response, Value, PUSH_ID};
use tokio::io::{AsyncWriteExt, BufReader, BufWriter};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, watch, Semaphore};

use crate::server::dispatch::{AuthError, Credentials, Dispatch};
use crate::server::errors::{format_bracket_code, format_err, NOAUTH, WRONGPASS};
use crate::server::metrics::{Metrics, MetricsSnapshot};
use crate::server::observer::MetricsObserver;
use crate::server::session::{PushSender, Session, WriteJob};

/// Ride through a poisoned lock: the guarded state stays consistent.
fn lock<T>(mutex: &StdMutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Wire protocol version advertised in HELLO replies (WIRE-004: v1,
/// frozen — adding commands or profiles never bumps it).
const PROTO_VERSION: i64 = 1;

/// Pre-auth allowlist under `Handshake::AuthCommand` (SRV-011, PRO-001).
const PRE_AUTH_COMMANDS: &[&str] = &["PING", "HELLO", "AUTH", "QUIT"];

/// Depth of the per-connection writer queue (the family's proven value).
const WRITER_QUEUE_DEPTH: usize = 64;

/// Server identity used by Thunder-built HELLO replies (SRV-014).
#[derive(Debug, Clone)]
pub struct ServerInfo {
    /// `server` field of the metadata-shape HELLO reply (`HelloStyle::ArgLess`).
    pub name: String,
    /// `version` field of the metadata-shape HELLO reply.
    pub version: String,
}

/// Listener configuration. Family posture keeps binds loopback/private by
/// default (SRV-040 guidance).
#[derive(Clone)]
pub struct ListenerConfig {
    /// Address to bind. Port `0` picks an ephemeral port — read it back
    /// via [`ListenerHandle::local_addr`].
    pub addr: SocketAddr,
    /// Per-read idle timeout (slow-loris resistance, SRV-009). Zero
    /// disables, matching each product's current posture.
    pub idle_timeout: Duration,
    /// Commands slower than this bump `slow_commands_total` (SRV-030).
    /// Zero disables the counter.
    pub slow_threshold: Duration,
    /// Whether this **deployment** enforces credentials (SRV-011).
    ///
    /// This is policy, not protocol: the profile fixes the handshake
    /// *shape* (does the client lead with `HELLO`? does it authenticate via
    /// `AUTH`?), while this flag decides whether the server actually refuses
    /// un-credentialed sessions. Both family products that authenticate on
    /// the RPC path expose exactly this toggle — Nexus's `auth_required` and
    /// Synap's `require_auth` — and an open Synap deployment is the reason
    /// it must live here rather than in the profile.
    ///
    /// Ignored under [`Handshake::None`], which has no gate at all. Defaults
    /// to `true`: a deployment opens up only by saying so.
    pub auth_required: bool,
    /// Optional TLS (SRV-040 / SPEC-008 CAN-020). `Some` turns TLS on for this
    /// deployment; `None` (the default) keeps it plaintext. Requires the crate's
    /// `tls` feature — a listener configured with TLS but built without it fails
    /// to start rather than silently serving plaintext.
    pub tls: Option<crate::tls::ServerTls>,
    /// Maximum concurrently open connections; further accepts are **refused**.
    /// `0` (the default) means unbounded.
    ///
    /// This is a different resource from [`Config::max_in_flight`], which
    /// bounds in-flight *requests per connection*: ten thousand idle
    /// connections each hold a reader task, a writer task and a `BufWriter`
    /// no matter what `max_in_flight` says. Operators bound memory and
    /// slow-loris exposure with *this* knob — Synap's `network.max_connections`
    /// is exactly it.
    ///
    /// **Refusal, not queueing**: at capacity the socket is dropped
    /// immediately, so a client fails fast instead of hanging on a connection
    /// nobody will ever read. Each refusal increments
    /// [`MetricsSnapshot::connections_refused_total`], so a ceiling that is
    /// engaging is visible rather than silent.
    pub max_connections: usize,
    /// Optional per-command observer (SRV-030 extension).
    ///
    /// `None` (the default) keeps today's behavior exactly: the built-in
    /// counters still record, nothing else runs, and the command label is not
    /// even materialized. Install one when an exporter needs per-command
    /// dimensions or frame-size distributions that cumulative totals cannot
    /// reconstruct — see [`MetricsObserver`].
    pub observer: Option<Arc<dyn MetricsObserver>>,
}

impl ListenerConfig {
    /// Config for `addr` with the defaults: no idle timeout, 1000 ms slow
    /// threshold, credentials enforced, plaintext.
    pub fn new(addr: SocketAddr) -> Self {
        Self {
            addr,
            idle_timeout: Duration::ZERO,
            slow_threshold: Duration::from_millis(1000),
            auth_required: true,
            tls: None,
            max_connections: 0,
            observer: None,
        }
    }

    /// Install a per-command metrics observer (SRV-030 extension).
    pub fn with_observer(mut self, observer: Arc<dyn MetricsObserver>) -> Self {
        self.observer = Some(observer);
        self
    }

    /// Refuse accepts beyond `max` concurrently open connections (SRV-009
    /// analog for connection count). `0` restores the unbounded default.
    pub fn with_max_connections(mut self, max: usize) -> Self {
        self.max_connections = max;
        self
    }

    /// Turn TLS on for this deployment (SRV-040): the listener wraps every
    /// accepted stream in `tokio-rustls` before the first Thunder frame.
    pub fn with_tls(mut self, tls: crate::tls::ServerTls) -> Self {
        self.tls = Some(tls);
        self
    }

    /// Serve un-credentialed sessions — the `auth_required = false` /
    /// `require_auth = false` posture (e.g. an open Synap deployment).
    ///
    /// The handshake shape is unchanged: a client may still send `AUTH`, and
    /// it still succeeds or fails on its own merits; nothing is *required*.
    pub fn open(mut self) -> Self {
        self.auth_required = false;
        self
    }
}

impl std::fmt::Debug for ListenerConfig {
    /// Hand-written because `observer` is a trait object: it reports whether
    /// one is installed rather than requiring `Debug` from every implementer.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ListenerConfig")
            .field("addr", &self.addr)
            .field("idle_timeout", &self.idle_timeout)
            .field("slow_threshold", &self.slow_threshold)
            .field("auth_required", &self.auth_required)
            .field("tls", &self.tls)
            .field("max_connections", &self.max_connections)
            .field("observer", &self.observer.is_some())
            .finish()
    }
}

impl Default for ListenerConfig {
    /// Loopback on an ephemeral port with the standard defaults.
    fn default() -> Self {
        Self::new(SocketAddr::from(([127, 0, 0, 1], 0)))
    }
}

/// Everything a connection task needs, shared once per listener.
struct ConnShared<D> {
    dispatch: Arc<D>,
    profile: Config,
    info: ServerInfo,
    idle_timeout: Duration,
    slow_threshold: Duration,
    auth_required: bool,
    metrics: Arc<Metrics>,
    /// Connection ceiling from `ListenerConfig.max_connections`; 0 = unbounded.
    max_connections: usize,
    /// Optional per-command observer; `None` on the default path.
    observer: Option<Arc<dyn MetricsObserver>>,
    /// Built once from `ListenerConfig.tls`; every accepted stream is wrapped
    /// through it before the first frame (SRV-040).
    #[cfg(feature = "tls")]
    acceptor: Option<tokio_rustls::TlsAcceptor>,
}

/// Handle to a running listener (SRV-001).
///
/// [`stop`](Self::stop) performs the graceful shutdown: the accept loop
/// ends, every connection finishes its in-flight requests, drains its
/// writer and closes; `stop` resolves when the last connection is gone.
/// Dropping the handle signals the same shutdown without waiting.
#[derive(Debug)]
pub struct ListenerHandle {
    local_addr: SocketAddr,
    shutdown: watch::Sender<bool>,
    metrics: Arc<Metrics>,
    /// Behind a mutex so [`ListenerHandle::stop`] can take `&self`: the
    /// receiver is consumed by whichever caller stops first, and any later
    /// caller finds `None` and simply observes the completed shutdown.
    done: StdMutex<Option<mpsc::Receiver<()>>>,
}

/// A cheap, clonable reader of a listener's metrics (SRV-030).
///
/// Observation is split from lifecycle on purpose: a metrics exporter can hold
/// one of these while the [`ListenerHandle`] stays wherever shutdown lives.
/// Before this existed, a product that wanted both had to wrap the handle in an
/// `Arc` — which made graceful `stop()` unreachable and silently downgraded
/// shutdown to the fire-and-forget `Drop` path.
#[derive(Debug, Clone)]
pub struct MetricsRef {
    metrics: Arc<Metrics>,
}

impl MetricsRef {
    /// Point-in-time metrics (SRV-030).
    pub fn snapshot(&self) -> MetricsSnapshot {
        self.metrics.snapshot()
    }
}

impl ListenerHandle {
    /// The bound address (resolves port `0` binds).
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Point-in-time metrics (SRV-030).
    pub fn snapshot(&self) -> MetricsSnapshot {
        self.metrics.snapshot()
    }

    /// A clonable metrics reader that does **not** carry lifecycle.
    ///
    /// Hand this to an exporter task instead of sharing the handle itself, so
    /// the handle keeps single ownership and graceful [`stop`](Self::stop)
    /// stays available.
    pub fn metrics(&self) -> MetricsRef {
        MetricsRef {
            metrics: Arc::clone(&self.metrics),
        }
    }

    /// Graceful shutdown (SRV-001): stop accepting, let every connection
    /// drain its in-flight responses, and resolve once all of them closed.
    ///
    /// Takes `&self` so an `Arc<ListenerHandle>` can still drain — sharing the
    /// handle must not cost the graceful path. Calling it twice is safe: the
    /// second call observes the shutdown the first one completed.
    pub async fn stop(&self) {
        let _ = self.shutdown.send(true);
        // Take the receiver out under the lock, then await outside it — a
        // std mutex must never be held across an await point.
        let receiver = lock(&self.done).take();
        if let Some(mut done) = receiver {
            // `recv` yields `None` once the accept loop and every
            // connection task dropped their guard senders.
            let _ = done.recv().await;
        }
    }
}

impl Drop for ListenerHandle {
    fn drop(&mut self) {
        // Fire-and-forget shutdown; `stop()` is the waiting variant.
        let _ = self.shutdown.send(true);
    }
}

/// Bind `config.addr` and run the accept loop: one task per connection,
/// graceful shutdown through the returned handle (SRV-001).
pub async fn spawn_listener<D: Dispatch>(
    dispatch: Arc<D>,
    profile: Config,
    info: ServerInfo,
    config: ListenerConfig,
) -> io::Result<ListenerHandle> {
    // SRV-040: build the TLS acceptor before binding so a misconfigured cert
    // fails fast. A TLS config without the `tls` feature is a hard error, not a
    // silent downgrade to plaintext.
    #[cfg(feature = "tls")]
    let acceptor = match &config.tls {
        Some(tls) => Some(
            crate::tls::build_acceptor(tls)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?,
        ),
        None => None,
    };
    #[cfg(not(feature = "tls"))]
    if config.tls.is_some() {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "TLS is configured but the crate was built without the `tls` feature",
        ));
    }

    let listener = TcpListener::bind(config.addr).await?;
    let local_addr = listener.local_addr()?;
    let metrics = Arc::new(Metrics::default());
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let (done_tx, done_rx) = mpsc::channel::<()>(1);

    let shared = Arc::new(ConnShared {
        dispatch,
        profile,
        info,
        idle_timeout: config.idle_timeout,
        slow_threshold: config.slow_threshold,
        auth_required: config.auth_required,
        metrics: Arc::clone(&metrics),
        max_connections: config.max_connections,
        observer: config.observer.clone(),
        #[cfg(feature = "tls")]
        acceptor,
    });

    tokio::spawn(accept_loop(listener, shared, shutdown_rx, done_tx));

    Ok(ListenerHandle {
        local_addr,
        shutdown: shutdown_tx,
        metrics,
        done: StdMutex::new(Some(done_rx)),
    })
}

/// Accept until shutdown; each connection runs in its own task (SRV-001).
/// Accept errors are transient — they never end the loop (SRV-004 spirit:
/// nothing a single socket does may kill the listener).
async fn accept_loop<D: Dispatch>(
    listener: TcpListener,
    shared: Arc<ConnShared<D>>,
    shutdown: watch::Receiver<bool>,
    done: mpsc::Sender<()>,
) {
    let mut accept_shutdown = shutdown.clone();
    let mut next_conn_id: u64 = 1;
    // `None` when unbounded, so the common case costs nothing at all.
    let limiter =
        (shared.max_connections > 0).then(|| Arc::new(Semaphore::new(shared.max_connections)));
    loop {
        let accepted = tokio::select! {
            _ = accept_shutdown.wait_for(|stop| *stop) => break,
            accepted = listener.accept() => accepted,
        };
        let Ok((stream, _peer)) = accepted else {
            continue;
        };
        // At capacity, drop the socket now. Queueing here would leave the
        // client waiting on a connection nobody is going to read; refusing
        // lets it fail fast and try elsewhere.
        let permit = match &limiter {
            Some(limiter) => match Arc::clone(limiter).try_acquire_owned() {
                Ok(permit) => Some(permit),
                Err(_) => {
                    drop(stream);
                    shared.metrics.connection_refused();
                    if let Some(observer) = &shared.observer {
                        observer.connection_refused();
                    }
                    continue;
                }
            },
            None => None,
        };
        let conn_id = next_conn_id;
        next_conn_id = next_conn_id.wrapping_add(1);
        let ctx = Arc::clone(&shared);
        let conn_shutdown = shutdown.clone();
        let done_guard = done.clone();
        ctx.metrics.connection_opened();
        if let Some(observer) = &ctx.observer {
            observer.connection_opened();
        }
        tokio::spawn(async move {
            handle_connection(stream, &ctx, conn_id, conn_shutdown).await;
            ctx.metrics.connection_closed();
            if let Some(observer) = &ctx.observer {
                observer.connection_closed();
            }
            // Held for the whole connection: the slot frees here, on every
            // path out of `handle_connection` including errors.
            drop(permit);
            drop(done_guard);
        });
    }
    // Dropping `listener` stops new connections; dropping `done` lets
    // `stop()` resolve once every connection guard is gone.
}

/// One connection: split socket, writer task behind an mpsc channel
/// (SRV-002), sequential read loop spawning one dispatch task per request
/// bounded by the profile's `max_in_flight` semaphore (SRV-003).
async fn handle_connection<D: Dispatch>(
    stream: TcpStream,
    ctx: &ConnShared<D>,
    conn_id: u64,
    shutdown: watch::Receiver<bool>,
) {
    // SRV-008: disable Nagle so length-prefixed replies are not held ~40 ms
    // by the delayed-ACK interaction documented in the Synap listener.
    let _ = stream.set_nodelay(true);

    // SRV-040: when this deployment configured TLS, complete the TLS handshake
    // before any Thunder frame. A TLS failure ends this connection only, never
    // the listener (SRV-004). The plaintext path keeps the lock-free
    // `into_split`; only the encrypted path pays `tokio::io::split`.
    #[cfg(feature = "tls")]
    if let Some(acceptor) = &ctx.acceptor {
        // A TLS handshake failure ends this connection only (SRV-004).
        if let Ok(tls) = acceptor.accept(stream).await {
            let (read_half, write_half) = tokio::io::split(tls);
            run_connection(
                BufReader::new(read_half),
                write_half,
                ctx,
                conn_id,
                shutdown,
            )
            .await;
        }
        return;
    }

    let (read_half, write_half) = stream.into_split();
    run_connection(
        BufReader::new(read_half),
        write_half,
        ctx,
        conn_id,
        shutdown,
    )
    .await;
}

/// The connection loop over already-split, transport-agnostic halves — one
/// monomorphization for plaintext (`OwnedReadHalf`/`OwnedWriteHalf`) and one for
/// TLS. Splitting here keeps the hot plaintext path byte-identical and
/// lock-free (SRV-002/003).
async fn run_connection<D, R, W>(
    mut reader: BufReader<R>,
    write_half: W,
    ctx: &ConnShared<D>,
    conn_id: u64,
    mut shutdown: watch::Receiver<bool>,
) where
    D: Dispatch,
    R: tokio::io::AsyncRead + Unpin + Send,
    W: tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let (tx, rx) = mpsc::channel::<WriteJob>(WRITER_QUEUE_DEPTH);
    let write_task = tokio::spawn(writer_task(
        BufWriter::new(write_half),
        rx,
        Arc::clone(&ctx.metrics),
        ctx.slow_threshold,
        ctx.observer.clone(),
    ));

    // SRV-013 / PRO-031: the typed push channel exists only under
    // `push = Enabled`; `Reserved` profiles can never emit.
    let push = match ctx.profile.push {
        PushPolicy::Enabled => Some(PushSender::new(tx.clone())),
        PushPolicy::Reserved => None,
    };
    // SRV-011: a session starts ungated when the profile has no handshake
    // at all, or when this deployment does not require credentials
    // (`auth_required = false` — Nexus's `auth_required`, Synap's
    // `require_auth`). Shape is the profile's; enforcement is the
    // deployment's, and conflating them is what left the `synap` profile
    // unable to authenticate (BN-023).
    let starts_authenticated =
        matches!(ctx.profile.handshake, Handshake::None) || !ctx.auth_required;
    // Typed with the product's own identity payload (SRV-012): the session
    // carries whatever `Dispatch::authenticate` resolved, so authorization
    // reads memory instead of re-querying a credential store.
    let session: Arc<Session<D::Identity>> =
        Arc::new(Session::new(conn_id, starts_authenticated, push));

    let permits = ctx.profile.max_in_flight.clamp(1, u32::MAX as usize) as u32;
    let in_flight = Arc::new(Semaphore::new(permits as usize));

    let mut first_frame = true;
    loop {
        let read = tokio::select! {
            _ = shutdown.wait_for(|stop| *stop) => break,
            read = read_next(&mut reader, ctx.profile.max_frame_bytes, ctx.idle_timeout) => read,
        };
        // SRV-004: EOF, a decode error, an oversized length prefix
        // (WIRE-020, rejected before any body allocation) or the idle
        // timeout (SRV-009) ends this read loop — this connection only,
        // never the listener.
        let Ok((req, in_bytes)) = read else { break };

        // SRV-013 / WIRE-005: client frames carrying PUSH_ID get a
        // dedicated refusal; the connection stays usable.
        if req.id == PUSH_ID {
            let response = Response::err(req.id, push_refusal_error(&ctx.profile));
            if !send_inline(&tx, response, in_bytes).await {
                break;
            }
            continue;
        }

        // SRV-011 / PRO-030: `HelloMandatory` rejects a non-HELLO first
        // frame with the profile's error convention and closes.
        if first_frame {
            first_frame = false;
            // SPEC-008 handshake adoption signal: did this connection lead
            // with a canonical HELLO, or a legacy first frame?
            if req.command != "HELLO" {
                ctx.metrics.record_non_hello_first_frame();
            }
            if matches!(ctx.profile.handshake, Handshake::HelloMandatory) && req.command != "HELLO"
            {
                let response = Response::err(req.id, hello_required_error(&ctx.profile));
                let _ = send_inline(&tx, response, in_bytes).await;
                break;
            }
        }

        // Built-ins Thunder owns, handled inline so the auth flag is set
        // before the next frame's gate check (the donor listeners
        // serialize AUTH ahead of request tasks for the same reason).
        match req.command.as_str() {
            // SRV-014: HELLO replies are constructed by Thunder.
            "HELLO" if !matches!(ctx.profile.hello_style, HelloStyle::NotUsed) => {
                let response = handle_hello(ctx, &session, req.id, &req.args).await;
                if !send_inline(&tx, response, in_bytes).await {
                    break;
                }
                continue;
            }
            // SRV-012: Thunder parses, the product validates.
            "AUTH" if matches!(ctx.profile.handshake, Handshake::AuthCommand) => {
                let response = handle_auth(ctx, &session, req.id, &req.args).await;
                if !send_inline(&tx, response, in_bytes).await {
                    break;
                }
                continue;
            }
            // SRV-011 allowlist: PING answers pre-auth without product
            // involvement; post-auth PING belongs to the product dispatch.
            "PING" if !session.is_authenticated() => {
                let response = builtin_ping(req.id, &req.args);
                if !send_inline(&tx, response, in_bytes).await {
                    break;
                }
                continue;
            }
            // AuthCommand semantics: acknowledge, then close after the write.
            "QUIT" if matches!(ctx.profile.handshake, Handshake::AuthCommand) => {
                let response = Response::ok(req.id, Value::Str("OK".to_owned()));
                let _ = send_inline(&tx, response, in_bytes).await;
                break;
            }
            _ => {}
        }

        // SRV-011: pre-auth gate per profile.
        if !session.is_authenticated() {
            match ctx.profile.handshake {
                Handshake::None => {}
                Handshake::AuthCommand => {
                    if !PRE_AUTH_COMMANDS.contains(&req.command.as_str()) {
                        let response = Response::err(req.id, NOAUTH);
                        if !send_inline(&tx, response, in_bytes).await {
                            break;
                        }
                        continue;
                    }
                    // Allowlisted command with no built-in under this
                    // profile combination — falls through to dispatch.
                }
                Handshake::HelloMandatory => {
                    let response = Response::err(req.id, hello_required_error(&ctx.profile));
                    if !send_inline(&tx, response, in_bytes).await {
                        break;
                    }
                    continue;
                }
            }
        }

        // SRV-003: every request takes an in-flight permit — excess requests
        // wait right here (backpressure on the read loop), never refused. The
        // permit bounds the *spawned* path below; on the fast path it is
        // released as soon as the response is in hand.
        let Ok(permit) = Arc::clone(&in_flight).acquire_owned().await else {
            break;
        };
        let dispatch = Arc::clone(&ctx.dispatch);
        let session = Arc::clone(&session);
        let started = Instant::now();
        let Request { id, command, args } = req;
        // Only materialized when an observer is installed: on the default path
        // this is `None` and the command name is never copied.
        let label: Option<Box<str>> = ctx.observer.as_ref().map(|_| command.as_str().into());
        // One dispatch future, owned so it can either finish inline or move to
        // a task — `Box::pin` gives a single 'static Send future for both.
        let mut fut = Box::pin(async move {
            match dispatch.dispatch(&session, &command, args).await {
                Ok(value) => Response::ok(id, value),
                // SRV-005/021: the error string travels verbatim and the
                // connection stays usable.
                Err(message) => Response::err(id, message),
            }
        });
        // SRV-006 fast path: poll the dispatch once. A handler that resolves
        // without suspending — a cache hit, an in-memory read, the peers'
        // inline model — is answered on the read loop with no task, erasing
        // the spawn + scheduler cost that dominates small-payload throughput
        // under concurrency (measured: +76% on point-echo depth16/4conns).
        // A handler that suspends on real I/O returns Pending and is moved to
        // a task, so it still never head-of-line blocks its connection
        // (SRV-004). A synchronous panic is isolated exactly as the runtime
        // isolates a task panic (SRV-005): the request is dropped, the
        // connection lives.
        // Scope the poll so the non-`Send` `Context` is dropped before any
        // `.await` below — otherwise this connection future would not be
        // `Send` and could not be spawned. `Poll<Response>` is `Send`.
        let polled = {
            let waker = std::task::Waker::noop();
            let mut cx = std::task::Context::from_waker(waker);
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| fut.as_mut().poll(&mut cx)))
        };
        match polled {
            Ok(std::task::Poll::Ready(response)) => {
                drop(fut);
                drop(permit);
                if !send_response(&tx, response, in_bytes, started.elapsed(), label).await {
                    break;
                }
            }
            Ok(std::task::Poll::Pending) => {
                let tx = tx.clone();
                tokio::spawn(async move {
                    let response = fut.await;
                    let _ = tx
                        .send(WriteJob::Response {
                            response,
                            in_bytes,
                            duration: started.elapsed(),
                            command: label,
                        })
                        .await;
                    // Released after the send: the drain below can rely on the
                    // queue holding every response once all permits are back.
                    drop(permit);
                });
            }
            // Synchronous panic in the handler: drop the request, keep the
            // connection — the spawned path's runtime-swallowed panic, mirrored.
            Err(_panic) => {
                drop(fut);
                drop(permit);
            }
        }
    }

    // Graceful drain (SRV-001/004): every dispatch task enqueues its
    // response before releasing its permit, so once all permits return the
    // writer's queue holds every outstanding response. The Shutdown job
    // then stops the writer even while product-held PushSender clones (and
    // the session's own) keep the channel open.
    let _ = in_flight.acquire_many(permits).await;
    let _ = tx.send(WriteJob::Shutdown).await;
    drop(tx);
    let _ = write_task.await;
}

/// Read one request bounded by the profile's cap (WIRE-020) and the
/// optional per-read idle timeout (SRV-009; zero disables). The returned
/// frame size comes from the decoder's length prefix (SRV-007).
async fn read_next<R: tokio::io::AsyncRead + Unpin>(
    reader: &mut BufReader<R>,
    max_frame_bytes: usize,
    idle_timeout: Duration,
) -> io::Result<(Request, usize)> {
    if idle_timeout.is_zero() {
        read_request_with_limit(reader, max_frame_bytes).await
    } else {
        match tokio::time::timeout(
            idle_timeout,
            read_request_with_limit(reader, max_frame_bytes),
        )
        .await
        {
            Ok(read) => read,
            Err(_) => Err(io::Error::new(io::ErrorKind::TimedOut, "idle timeout")),
        }
    }
}

/// Enqueue a read-loop response (built-ins and gate errors carry no
/// dispatch duration). Returns `false` when the writer is gone and the
/// connection should close.
async fn send_inline(tx: &mpsc::Sender<WriteJob>, response: Response, in_bytes: usize) -> bool {
    // Built-ins carry no label: they are answered by the listener itself, and
    // an observer that wants them can count `command_completed` for HELLO/AUTH
    // through the normal path.
    send_response(tx, response, in_bytes, Duration::ZERO, None).await
}

/// Enqueue a dispatched response with its measured duration (SRV-030). Used
/// by the fast path, where the response is produced inline on the read loop
/// rather than in a task. Returns `false` when the writer is gone.
async fn send_response(
    tx: &mpsc::Sender<WriteJob>,
    response: Response,
    in_bytes: usize,
    duration: Duration,
    command: Option<Box<str>>,
) -> bool {
    tx.send(WriteJob::Response {
        response,
        in_bytes,
        duration,
        command,
    })
    .await
    .is_ok()
}

/// The connection's writer (SRV-002/006): owns the buffered write half.
/// After writing one job it drains every already-queued job via `try_recv`
/// before a single `flush()` — the Synap drain-then-flush pattern that
/// coalesces a pipelined burst into one syscall (SRV-006).
async fn writer_task<W: tokio::io::AsyncWrite + Unpin>(
    mut writer: BufWriter<W>,
    mut rx: mpsc::Receiver<WriteJob>,
    metrics: Arc<Metrics>,
    slow_threshold: Duration,
    observer: Option<Arc<dyn MetricsObserver>>,
) {
    'outer: while let Some(job) = rx.recv().await {
        if !write_job(
            &mut writer,
            job,
            &metrics,
            slow_threshold,
            observer.as_ref(),
        )
        .await
        {
            break;
        }
        while let Ok(job) = rx.try_recv() {
            if !write_job(
                &mut writer,
                job,
                &metrics,
                slow_threshold,
                observer.as_ref(),
            )
            .await
            {
                break 'outer;
            }
        }
        if writer.flush().await.is_err() {
            break;
        }
    }
    // Cover the Shutdown exit paths with frames still buffered.
    let _ = writer.flush().await;
}

/// Encode exactly once, write, record after the successful write
/// (SRV-007/030). Returns `false` when the writer must stop (write error
/// or shutdown).
async fn write_job<W: tokio::io::AsyncWrite + Unpin>(
    writer: &mut BufWriter<W>,
    job: WriteJob,
    metrics: &Metrics,
    slow_threshold: Duration,
    observer: Option<&Arc<dyn MetricsObserver>>,
) -> bool {
    let (response, in_bytes, duration, command) = match job {
        WriteJob::Shutdown => return false,
        WriteJob::Push(response) => {
            let Ok(frame) = encode_frame(&response) else {
                return true;
            };
            if writer.write_all(&frame).await.is_err() {
                return false;
            }
            metrics.record_push(frame.len());
            if let Some(observer) = observer {
                observer.push_emitted(frame.len());
            }
            return true;
        }
        WriteJob::Response {
            response,
            in_bytes,
            duration,
            command,
        } => (response, in_bytes, duration, command),
    };
    let is_error = response.result.is_err();
    // SRV-007: the one serialization — this buffer is written and its
    // length is the out-bytes metric. Re-encoding for metrics is banned.
    let Ok(frame) = encode_frame(&response) else {
        // Unencodable response: skip the frame, keep the connection (the
        // donor listeners do the same).
        return true;
    };
    if writer.write_all(&frame).await.is_err() {
        return false;
    }
    // SRV-030: metrics record after the successful socket write. The observer
    // is called at the same point and with the same values, so the two can
    // never disagree about what a command cost.
    metrics.record_command(in_bytes, frame.len(), duration, is_error, slow_threshold);
    if let Some(observer) = observer {
        observer.command_completed(
            command.as_deref().unwrap_or(""),
            in_bytes,
            frame.len(),
            duration,
            is_error,
        );
    }
    true
}

// ── Built-ins (SRV-011/012/014) ──────────────────────────────────────────────

/// Build the HELLO reply from `ServerInfo` + config + the
/// `authenticate`/`capabilities` hooks — Thunder's job, never application
/// code (SRV-014). Covers both shapes pinned by the corpus handshake
/// group: the metadata shape `{server, version, proto, id, authenticated}`
/// (`HelloStyle::ArgLess`) and the capabilities shape
/// `{protocol_version, capabilities}` (`HelloStyle::MapPayload`).
async fn handle_hello<D: Dispatch>(
    ctx: &ConnShared<D>,
    session: &Session<D::Identity>,
    req_id: u32,
    args: &[Value],
) -> Response {
    match ctx.profile.hello_style {
        // Guarded by the caller; kept total for safety.
        HelloStyle::NotUsed => {
            Response::err(req_id, format_err("HELLO is not part of this profile"))
        }
        // Metadata shape: arg-less request, metadata-only reply —
        // credentials travel via AUTH.
        HelloStyle::ArgLess => Response::ok(
            req_id,
            Value::Map(vec![
                (
                    Value::Str("server".to_owned()),
                    Value::Str(ctx.info.name.clone()),
                ),
                (
                    Value::Str("version".to_owned()),
                    Value::Str(ctx.info.version.clone()),
                ),
                (Value::Str("proto".to_owned()), Value::Int(PROTO_VERSION)),
                (
                    Value::Str("id".to_owned()),
                    Value::Int(session.connection_id() as i64),
                ),
                (
                    Value::Str("authenticated".to_owned()),
                    Value::Bool(session.is_authenticated()),
                ),
            ]),
        ),
        // Capabilities shape: credentials ride in the map (SRV-012).
        HelloStyle::MapPayload => {
            let creds = match parse_hello_credentials(args) {
                Ok(creds) => creds,
                Err(message) => return Response::err(req_id, message),
            };
            match ctx.dispatch.authenticate(creds).await {
                Ok(principal) => {
                    let capabilities = ctx.dispatch.capabilities(&principal);
                    session.set_principal(principal);
                    Response::ok(
                        req_id,
                        Value::Map(vec![
                            (
                                Value::Str("protocol_version".to_owned()),
                                Value::Int(PROTO_VERSION),
                            ),
                            (
                                Value::Str("capabilities".to_owned()),
                                Value::Array(capabilities.into_iter().map(Value::Str).collect()),
                            ),
                        ]),
                    )
                }
                // A failed HELLO leaves the connection open and gated —
                // the client may retry with better credentials.
                Err(err) => Response::err(req_id, auth_error_string(&ctx.profile, err)),
            }
        }
    }
}

/// `AUTH <api_key>` / `AUTH <user> <pass>` under `AuthCommand` (SRV-012):
/// Thunder parses, the product validates, the session flips (SRV-010).
async fn handle_auth<D: Dispatch>(
    ctx: &ConnShared<D>,
    session: &Session<D::Identity>,
    req_id: u32,
    args: &[Value],
) -> Response {
    let creds = match args {
        [key] => value_str(key).map(Credentials::ApiKey),
        [user, pass] => value_str(user)
            .zip(value_str(pass))
            .map(|(user, pass)| Credentials::UserPass(user, pass)),
        _ => None,
    };
    let Some(creds) = creds else {
        return Response::err(req_id, format_err("invalid arguments for 'AUTH'"));
    };
    match ctx.dispatch.authenticate(creds).await {
        Ok(principal) => {
            session.set_principal(principal);
            Response::ok(req_id, Value::Str("OK".to_owned()))
        }
        Err(err) => Response::err(req_id, auth_error_string(&ctx.profile, err)),
    }
}

/// Built-in pre-auth `PING` (SRV-011 allowlist), family-pinned echo shape:
/// bare `PING` → `"PONG"`, one string/bytes argument echoes back.
fn builtin_ping(req_id: u32, args: &[Value]) -> Response {
    match args {
        [] => Response::ok(req_id, Value::Str("PONG".to_owned())),
        [Value::Str(payload)] => Response::ok(req_id, Value::Str(payload.clone())),
        [Value::Bytes(payload)] => Response::ok(req_id, Value::Bytes(Arc::clone(payload))),
        [_] => Response::err(
            req_id,
            format_err("PING argument must be a string or bytes"),
        ),
        args => Response::err(
            req_id,
            format_err(&format!(
                "wrong number of arguments for 'PING' ({})",
                args.len()
            )),
        ),
    }
}

/// Parse the `MapPayload` HELLO argument — a map with `version`,
/// `token` | `api_key`, `client_name` (PRO-001). Missing credentials
/// become [`Credentials::None`]: products with auth disabled accept them.
fn parse_hello_credentials(args: &[Value]) -> Result<Credentials, String> {
    let map = match args.first() {
        None => return Ok(Credentials::None),
        Some(map @ Value::Map(_)) => map,
        Some(_) => return Err(format_err("HELLO expects a Map argument")),
    };
    if let Some(token) = map.map_get("token").and_then(value_str) {
        Ok(Credentials::Token(token))
    } else if let Some(key) = map.map_get("api_key").and_then(value_str) {
        Ok(Credentials::ApiKey(key))
    } else {
        Ok(Credentials::None)
    }
}

/// Extract a UTF-8 string from a credential argument (`Str`, or `Bytes`
/// holding UTF-8 — the family's tolerant form).
fn value_str(value: &Value) -> Option<String> {
    match value {
        Value::Str(text) => Some(text.clone()),
        Value::Bytes(bytes) => String::from_utf8(bytes.to_vec()).ok(),
        _ => None,
    }
}

// ── Config-convention error strings (SRV-021, PRO-014) ─────────────────────

/// Map an [`AuthError`] to the profile's convention; product-supplied
/// messages travel verbatim (WIRE-040).
fn auth_error_string(profile: &Config, err: AuthError) -> String {
    match err {
        AuthError::Message(message) => message,
        AuthError::InvalidCredentials => match profile.error_codes {
            ErrorConvention::BracketCode | ErrorConvention::Both => {
                format_bracket_code("unauthorized", "invalid credentials")
            }
            _ => WRONGPASS.to_owned(),
        },
    }
}

/// The gate error for `HelloMandatory` profiles (SRV-011).
fn hello_required_error(profile: &Config) -> String {
    match profile.error_codes {
        ErrorConvention::BracketCode | ErrorConvention::Both => {
            format_bracket_code("unauthorized", "authentication required: send HELLO first")
        }
        _ => NOAUTH.to_owned(),
    }
}

/// The dedicated PUSH_ID refusal (SRV-013, WIRE-005).
fn push_refusal_error(profile: &Config) -> String {
    const MESSAGE: &str = "request id u32::MAX is reserved for server push frames";
    match profile.error_codes {
        ErrorConvention::BracketCode | ErrorConvention::Both => {
            format_bracket_code("reserved_frame_id", MESSAGE)
        }
        _ => format_err(MESSAGE),
    }
}

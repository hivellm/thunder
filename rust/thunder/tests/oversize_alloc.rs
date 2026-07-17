//! WIRE-020 has an allocation claim in it: an inbound frame whose declared
//! length is over the cap must be refused **without allocating the body**.
//! The behavioral suite asserts the typed error and the poisoning, but a
//! client that allocated the body and *then* errored would pass those — the
//! observable outcome is identical. Only a counting allocator separates the
//! two, so this lives in its own test binary: `#[global_allocator]` is
//! process-wide, and cargo gives each `tests/*.rs` its own process, which
//! keeps the counter free of other tests' concurrent allocations.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;

use thunder::wire::config::{ErrorConvention, Handshake, HelloStyle, PushPolicy, TlsPolicy};
use thunder::wire::{read_request_with_limit, Request};
use thunder::{Client, ClientError, Config};

/// Counts bytes handed out, never subtracting: the question is "was a
/// body-sized buffer ever created", which a high-water mark answers and a
/// live-bytes gauge would hide (the buffer could be freed on the error
/// path before the test looks).
struct Counting;

static ALLOCATED: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATED.fetch_add(layout.size(), Ordering::Relaxed);
        System.alloc(layout)
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // `vec![0u8; len]` lands here, not in `alloc` — the exact shape the
        // frame reader would use to pre-size a body.
        ALLOCATED.fetch_add(layout.size(), Ordering::Relaxed);
        System.alloc_zeroed(layout)
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCATED.fetch_add(new_size.saturating_sub(layout.size()), Ordering::Relaxed);
        System.realloc(ptr, layout, new_size)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout);
    }
}

#[global_allocator]
static ALLOC: Counting = Counting;

/// The cap the client is configured with — small, so the declared length
/// below is unambiguously over it.
const CAP: usize = 64;

/// Declared body length of the hostile frame: 512 MiB. Never sent, only
/// claimed. Large enough that allocating it is impossible to miss against
/// the noise floor of a tokio runtime, and small enough that a client that
/// *does* allocate still completes the test rather than dying on OOM (which
/// would fail the run without telling us why).
const DECLARED_BODY: u32 = 512 * 1024 * 1024;

fn probe_profile() -> Config {
    Config {
        scheme: "test",
        default_port: 0,
        handshake: Handshake::None,
        hello_style: HelloStyle::NotUsed,
        push: PushPolicy::Reserved,
        max_frame_bytes: CAP,
        max_in_flight: 64,
        error_codes: ErrorConvention::None,
        tls: TlsPolicy::Off,
    }
}

#[tokio::test]
async fn oversized_frame_is_refused_without_allocating_the_body() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = format!("127.0.0.1:{}", listener.local_addr().unwrap().port());
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (read_half, mut write_half) = stream.into_split();
        let mut reader = tokio::io::BufReader::new(read_half);
        let _req: Request = read_request_with_limit(&mut reader, 1024 * 1024)
            .await
            .unwrap()
            .0;
        // The prefix alone, and nothing behind it. A client that pre-sizes
        // a buffer from this number allocates 512 MiB for bytes that will
        // never arrive.
        write_half
            .write_all(&DECLARED_BODY.to_le_bytes())
            .await
            .unwrap();
        // Hold the connection open so the client's failure is the cap
        // check, not an EOF racing it.
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    });

    let client = Client::connect(&addr, probe_profile()).await.unwrap();

    let before = ALLOCATED.load(Ordering::Relaxed);
    let err = client.call("GET", vec![]).await.unwrap_err();
    let grew_by = ALLOCATED.load(Ordering::Relaxed) - before;

    assert!(
        matches!(err, ClientError::FrameTooLarge { .. }),
        "expected the frame-too-large class, got {err:?}"
    );
    // The margin is deliberately loose: the call path legitimately
    // allocates (the pending entry, the error, the request buffer). What it
    // must never do is allocate anything on the order of the declared body.
    const CEILING: usize = 8 * 1024 * 1024;
    assert!(
        grew_by < CEILING,
        "refusing a frame declaring {DECLARED_BODY} bytes allocated {grew_by} bytes \
         (ceiling {CEILING}) — the body was sized from the prefix before the cap was checked \
         (WIRE-020)"
    );

    server.abort();
}

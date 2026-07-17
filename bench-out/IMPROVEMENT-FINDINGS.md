# Where Thunder can still improve — evidence, not guesses (2026-07-17)

Running record for `phase4_hotpath-optimization`. Every claim here cites a measurement or a file;
where a measurement failed, that is said rather than rounded off.

---

## 0. Landed: the client's write path (6.5×)

The client held a mutex per call and wrote straight to the socket with no buffering — N concurrent
calls cost N syscalls and N waiters on one lock. The server has had drain-then-flush since T1.5
(SRV-006, the measured +23% from Synap); the client never got it.

Fixed in `1bdbcb6` (writer task + bounded mpsc + `BufWriter` + drain-then-flush). Controlled A/B,
back-to-back: `pipelined-1k` 1 conn **85,842 → 561,127 qps** (6.5×), p50 11,645 → 1,716 µs. Untouched
peers moved 4–24% across the same A/B, so the effect is far outside the noise.

**Lesson worth keeping:** the fix was found by a diagnostic that *refuted* the stated hypothesis. The
T4.3 verdict named spawn-per-request the prime suspect; the `thunder-stripped` lane showed
thunder 103,022 vs stripped 102,508 at pipelined-1k — identical — so the server was never the
bottleneck. Guessing would have optimized the wrong layer.

---

## 1. Top remaining lever: the server's per-request overhead

Now that the client no longer masks it, `thunder` vs `thunder-stripped` (same wire, same client, bare
listener) shows what the server's features cost:

| Scenario | depth | conns | thunder | stripped | server cost |
|---|---:|---:|---:|---:|---:|
| pipelined-1k | 1000 | 1 | 627,218 | 957,252 | **34.5%** |
| pipelined-1k | 1000 | 4 | 780,056 | 974,652 | 20.0% |
| medium-4KiB | 1 | 4 | 41,544 | 51,492 | **19.3%** |
| point-echo-64B | 16 | 4 | 318,675 | 372,018 | 14.3% |
| medium-4KiB | 16 | 1 | 81,219 | 94,427 | **14.0%** |
| point-echo-64B | 16 | 1 | 166,018 | 186,263 | 10.9% |
| medium-4KiB | 16 | 4 | 159,935 | 174,243 | 8.2% |
| point-echo-64B | 1 | 4 | 42,688 | 45,570 | 6.3% |
| point-echo-64B | 1 | 1 | 33,635 | 34,694 | 3.1% |
| medium-4KiB | 1 | 1 | 32,935 | 33,462 | 1.6% |

**The bolded cells are exactly the ones Thunder does not win** (`medium-4KiB` d1/c4 and d16/c1;
see the 3-run stability table). The correlation is the lead.

Per request `thunder::server` pays: a `tokio::spawn`, a `Semaphore::acquire_owned` (which allocates),
three `Arc::clone`s, an mpsc send, plus the session/PUSH_ID/first-frame checks and metric atomics.
At pipelined-1k/1conn the delta works out to **~550 ns per request**.

### Which of those dominates is still OPEN — the ablation I tried was invalid

An attempt to isolate the spawn by making the stripped lane `tokio::spawn(...).await` per request
produced numbers that contradict the table above (it claimed 32.7% at point-echo d1/c1 where the
whole-server cost is 3.1%). The reason is a modelling error: `spawn().await` serializes and blocks the
read loop, whereas `thunder::server` spawns **fire-and-forget** and keeps reading. It measured
spawn+await, not spawn+continue. Reverted rather than reported.

A faithful decomposition needs a third variant — writer task + mpsc + fire-and-forget spawn, without
session/metrics — so `stripped` vs `stripped+spawn` isolates the spawn model alone. **Not yet built.**

### The candidate fix (keeps SRV-002, does not need to drop spawn)

The naive answer, "stop spawning", is wrong: spawn is what buys no head-of-line blocking (SRV-002/003)
— a slow command must not stall its connection, which is precisely the guarantee `thunder-stripped`
drops. The known technique that keeps both: **poll the dispatch future once inline; spawn only if it
returns `Pending`.** Fast commands (the overwhelmingly common case) then cost zero spawn, while a slow
one still moves off the read loop. Cost: the future must be built `'static + Send` before the first
poll, which the current `Arc`-clone-into-task shape already nearly does.

---

## 2. A promised feature that was never built: zero-copy reply values

`docs/ARCHITECTURE.md:86` commits Thunder to taking Synap's zero-copy replies:

> | Zero-copy reply values (`Arc`-friendly `Bytes`) | Synap (`Arc<[u8]>`, phase11) | value type design (T1.1) |

The implementation did not take it — `wire/value.rs:24-25`:

```rust
Bytes(#[serde(with = "serde_bytes")] Vec<u8>),
Str(String),
```

Both are **owned**. A server replying with a payload it already holds must **clone it in full**, every
request. For the family's actual hot workload — embeddings — that is a copy of the whole vector per
reply, on top of the encode copy. The architecture's own donor table says this was supposed to land in
T1.1 and it did not; it is a real, traceable gap, not a nice-to-have.

This also explains the harness noise (§3): the bench's `STATIC` reply does
`self.static_reply.clone()` — a 4 KiB allocation + memcpy per request — which is why every unstable
cell is a `medium-4KiB` one.

Changing `Value` is a public-API break, so it is a spec decision (PKG-012 makes it a **major**), not a
silent optimization. Worth raising deliberately.

---

## 3. The instrument is still the blocker for G5

Across three independent full-matrix runs, cells split three ways:

- **6 cells won in all three runs** (pipelined-1k ×2, point-echo d1/c1, d16/c1, d16/c4, medium-4KiB d1/c1);
- **1 cell is a stable tie** — `connection-storm` (−0.3% / −1.9% / +0.1%). Not noise: it measures
  opening 1000 TCP connections, where the cost is the kernel's, not the protocol's. No wire wins that;
- **4 cells are unstable**, swinging up to **43 points** between runs (`medium-4KiB` d1/c4:
  +33.0% / −7.6% / −9.7%).

Every unstable cell is a `conns=4` or `medium-4KiB` one — i.e. the cells dominated by allocation and
scheduling, not by the transport. Between the earlier two matrices, lanes nobody touched moved +95%
(resp3), +75% (bolt), +45% (http).

**A ≥10% gate cannot be judged with an instrument that swings 40+ points.** BEN-011 asks for pinned
runs; the harness does not deliver them. Until it does, no per-cell G5 verdict — including the
"3 of 11" in `T4.3-G5-VERDICT.md`, which was measured on a machine that was not quiet — should be
trusted for or against Thunder.

Fixing the instrument (CPU pinning, more samples, an explicit noise-floor check that fails the run
rather than reporting it) is therefore a **prerequisite** for the remaining optimization work, not a
follow-up to it.

---

## Ranked next steps

| # | Work | Evidence | Blocking? |
|---|---|---|---|
| 1 | Fix the instrument (pinning, samples, noise floor) | 4 cells swing ≤43 pts; untouched lanes moved 45–95% | **Yes** — nothing else can be judged |
| 2 | Faithful spawn ablation (3rd variant) | the decomposition is open; my first attempt was invalid | Yes, for #3 |
| 3 | Poll-once fast path for dispatch | server costs 34.5% at pipelined, 14–19% at the cells we lose | — |
| 4 | Zero-copy `Arc` reply values | promised at ARCHITECTURE.md:86, never built; a full payload copy per reply | Needs a spec decision (major) |

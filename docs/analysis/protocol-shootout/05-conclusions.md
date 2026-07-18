# §5 — Conclusions

> What fourteen lanes established, what they did not, and what should happen
> next. Numbers `--allow-noisy`; see §5.5 on what that permits claiming.

## 5.1 The headline

**Thunder is the fastest transport in the field on latency, and 3.5x the
fastest on pipelined throughput, and it does not win by being simplest.**

The Memcached lane exists to be the floor nobody beats — a 24-byte fixed
header, one opcode, nothing to parse. Thunder beats it:

| point-echo-64B | thunder | memcached (the "ceiling") |
|---|---|---|
| d1/c1 | **35.9 µs / 27 370 qps** | 42.5 µs / 21 043 qps |
| d16/c1 | **148 640 qps** | 100 089 qps |
| d1000/c1 | **448 171 qps** | 103 244 qps |

A protocol carrying request ids, out-of-order demux, a typed value model and a
length prefix outruns one that carries essentially nothing. That result was
not expected when the ceiling lane was built, and it is the strongest single
finding in the expansion.

## 5.2 The question the expansion was built to answer

**Is Thunder's sync-tiny deficit the price of multiplexing, or a defect?**

**Answer: the price of multiplexing — and Thunder pays it far more cheaply
than the only other multiplexed peer.** (§2)

- At d1/c4, both multiplexed protocols trail the FIFO leaders. Thunder by
  **6%**; gRPC by **57%**.
- At d1/c1 Thunder pays nothing at all — it leads the entire field.
- In every cell with concurrency, the trade pays back: **+49%** over the best
  FIFO at d16/c1, **+253%** at d1000/c1.

The 6% is not a bug to chase. It is the visible half of a trade whose other
half is worth 2.5–3.5x.

## 5.3 What each isolated comparison established

| Question | Comparison | Answer |
|---|---|---|
| What does the length prefix cost? | msgpack-rpc (same codec, no prefix) | **4–5 bytes/msg**, and it buys 4.0x at depth 1000 (§3.1) |
| Does codec aggressiveness matter? | thrift (same framing, TCompactProtocol) | **No** — statistically identical to MessagePack at this payload shape (§3.2) |
| What does response structure cost? | postgres (4-message cycle) | Leanest request in the field, last place under pipelining (§3.3) |
| Is parse cost the bottleneck? | capnp (no parse step at all) | **No** — heaviest wire in the field; removing the parse bought nothing (§3.4) |
| What does a broker cost? | nats / mqtt (4 traversals) | **2.4–3.9x bytes, up to 3.9x latency** (§3.5) |
| Is the mux cost universal? | grpc (the other mux) | **Yes** (§2) |

Two of these are negative results, and they are the more useful ones:
**codec choice and parse elimination did not move this workload.** Effort
spent there would have been wasted. Framing and design did move it.

## 5.4 The methodological finding, which may outlast the numbers

**Hand-written benchmark peers flatter the protocol they model — they do not
sabotage it.** (§1.3)

When the PostgreSQL lane was swapped from our hand-written listener to the
real `pgwire`, at **byte-for-byte identical traffic**, throughput fell 2–4x.
Our peer had implemented the optimal drain-then-flush that the production
crate does not.

This inverts the usual suspicion about benchmark authorship. The risk is not
that the author cripples the competition; it is that the author writes an
idealized competitor that no real deployment matches, then reports it as
what users would get. Any shootout publishing hand-rolled peers is publishing
**upper bounds on protocols**, not measurements of implementations — and the
two answer different questions.

Where lanes remain hand-rolled (Memcached, MQTT, the NATS broker) their
numbers carry that reading explicitly.

## 5.5 What these numbers are not

- **Not citable at G5.** Produced with `--allow-noisy` on a developer
  workstation. BEN-031 requires a quiet host; that gate is open and shared
  with `phase4_hotpath`.
- **Not wire-vs-wire for gRPC and Cap'n Proto.** Those are full-stack
  comparisons (real client + real server both sides). Part of their deficit is
  the protocol and part is the implementation; these lanes cannot separate
  them.
- **Not throughput-comparable for Cap'n Proto.** It runs single-threaded by
  necessity (`RpcSystem` is `!Send`). Only its d1/c1 latency is comparable.
- **Not a verdict on protocols outside this workload shape.** Everything here
  is small request/response. §3.2 and §3.4 in particular would likely change
  with many-small-fields records or large repeatedly-read structures.

What survives noise, and what this analysis therefore claims, are **large
repeatable orderings and ratios** — 3.5x, 8.2x, 2–4x — not single-digit
percentage differences. The one single-digit claim made (Thunder's 6% d1/c4
deficit) is stated as small precisely because it is near the noise floor, and
the argument does not rest on its exact value.

## 5.6 What should happen next

1. **Re-run on a quiet host** and promote the artifact (BEN-031). Nothing here
   is quotable until then.
2. **Add a many-small-fields scenario** if codec comparison is wanted for
   real. The current matrix cannot separate codecs (§3.2), and saying so is
   more useful than a table that looks like it can.
3. **Leave the 6% alone.** §2 establishes it as a design cost, not a defect.
   Optimizing it would trade against the pipelined win that is 40x larger.
4. **Consider retiring the hand-rolled Memcached lane** or re-labelling it.
   Now that Thunder beats it, "the ceiling nobody beats" is the wrong frame,
   and it is one of the lanes still carrying the §5.4 bias.

## 5.7 One-line summary

Fourteen protocols, one no-op backend, one process: **Thunder leads on
latency and by 3.5x on pipelined throughput; its one deficit is 6% in a single
cell and is the measured, universal price of multiplexing that gRPC pays nine
times over; and the codec and zero-copy optimizations the field argues about
did not move this workload at all.**

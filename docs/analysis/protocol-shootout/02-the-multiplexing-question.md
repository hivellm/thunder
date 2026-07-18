# §2 — The multiplexing question, answered

> The question this whole expansion existed to settle. Governing spec:
> SPEC-007 (BEN-001, BEN-020). Numbers from the 14-lane matrix,
> `--allow-noisy`, 2000 ops/rep, 3 reps, dispersion in parentheses.

## 2.1 The hypothesis under test

Thunder multiplexes: it stamps every request with an id and matches replies
out of order. That machinery is what wins it the pipelined cells. The d1/c4
analysis in `phase4_hotpath` suspected it also *costs* something on
synchronous tiny payloads — one request in flight, nothing to reorder, but the
demux bookkeeping is paid anyway.

Against FIFO peers this was **unfalsifiable**. RESP3, Memcached, Bolt,
MongoDB, Thrift and PostgreSQL are all strictly ordered per connection; any
gap could have been Thunder's implementation quality rather than the price of
the design.

gRPC breaks the tie. It is the only other **multiplexed** peer in the field —
concurrent HTTP/2 streams, interleaved replies. If out-of-order demux carries
an intrinsic sync-tiny cost, gRPC must pay it too.

## 2.2 What the data says

**point-echo-64B, depth=1, connections=1** — one request at a time, nothing to
multiplex, the cell most hostile to Thunder's design:

| lane | model | p50 µs | qps |
|---|---|---|---|
| thunder-stripped | mux (diagnostic) | 30.1 | 31 454 |
| **thunder** | **mux** | **35.9** | **27 370** |
| memcached | FIFO | 42.5 | 21 043 |
| msgpack-rpc | FIFO | 43.3 | 20 754 |
| thrift | FIFO | 44.2 | 20 701 |
| mongodb | FIFO | 45.3 | 19 585 |
| resp3 | FIFO | 47.4 | 20 004 |
| bolt | FIFO | 54.9 | 17 196 |
| http | FIFO | 58.8 | 15 806 |
| postgres | FIFO | 82.6 | 11 797 |
| capnp | mux | 84.6 | 10 378 |
| **grpc** | **mux** | **137.8** | **7 241** |

**Thunder is the fastest lane in the field here** — ahead of Memcached, whose
entire reason for inclusion was to be the FIFO performance ceiling (24-byte
fixed header, one opcode, nothing to parse). The original worry that Thunder
trails FIFO peers on sync-tiny **does not reproduce at c1**.

**point-echo-64B, depth=1, connections=4** — where the deficit actually lives:

| lane | model | p50 µs | qps |
|---|---|---|---|
| resp3 | FIFO | 65.5 | 55 997 |
| msgpack-rpc | FIFO | 64.1 | 55 589 |
| **thunder** | **mux** | **73.8** | **52 476** |
| mongodb | FIFO | 70.3 | 51 463 |
| http | FIFO | 71.8 | 49 421 |
| memcached | FIFO | 75.6 | 48 406 |
| thrift | FIFO | 77.5 | 46 620 |
| bolt | FIFO | 78.1 | 45 830 |
| capnp | mux | 81.3 | 44 235 |
| postgres | FIFO | 118.1 | 30 725 |
| **grpc** | **mux** | **152.2** | **24 332** |

Here Thunder does trail the leaders — 52.5k against RESP3's 56.0k, a **6%
deficit**. And gRPC, the other multiplexed protocol, is at **24.3k: 57% below
RESP3**, and below every FIFO lane in the table.

## 2.3 The verdict

**The sync-tiny cost is real, it belongs to the multiplexed design, and
Thunder pays it roughly an order of magnitude more cheaply than gRPC does.**

- Both multiplexed protocols sit below the FIFO leaders at d1/c4. That is the
  falsifiable prediction the lane was built to test, and it held.
- Thunder's deficit is 6%. gRPC's is 57%. If the cost were a Thunder defect,
  the protocol that shares its design would not be nine times worse off.
- At d1/c1 Thunder does not pay it at all — it leads the field. Whatever the
  demux costs, it is smaller than what Thunder's other choices save.

The gap is therefore **not a defect to fix**. It is a design trade whose other
half shows up two cells over.

## 2.4 The other half of the trade

**point-echo-64B, depth=16, connections=1**:

| lane | qps |
|---|---|
| thunder-stripped | 182 527 |
| **thunder** | **148 640** |
| memcached | 100 089 |
| msgpack-rpc | 97 301 |
| thrift | 96 882 |
| mongodb | 93 994 |
| resp3 | 87 926 |
| capnp | 64 326 |
| **grpc** | **27 092** |

**pipelined-1k, depth=1000, connections=1**:

| lane | qps |
|---|---|
| thunder-stripped | 587 104 |
| **thunder** | **448 171** |
| mongodb | 126 802 |
| msgpack-rpc | 113 184 |
| thrift | 110 229 |
| bolt | 108 944 |
| resp3 | 108 307 |
| http | 105 433 |
| memcached | 103 244 |
| capnp | 56 433 |
| mqtt | 56 140 |
| **grpc** | **54 751** |
| nats | 49 292 |
| postgres | 19 325 |

At depth 1000 Thunder does **448k qps — 3.5x the best FIFO peer** (MongoDB at
127k) and **1.5x above the Memcached ceiling even at depth 16**. Against gRPC,
the protocol that shares its design, it is **8.2x faster**.

So the complete picture of the trade:

| cell | Thunder vs best FIFO | Thunder vs gRPC (same design) |
|---|---|---|
| d1/c1 | **+30%** (fastest in field) | **+278%** |
| d1/c4 | −6% | **+116%** |
| d16/c1 | **+49%** | **+449%** |
| d1000/c1 | **+253%** | **+719%** |

Thunder gives up 6% in exactly one cell and takes back 49–253% in the cells
where concurrency exists. That is the trade working as designed, and gRPC —
paying the same architectural cost without the same implementation care —
shows what it looks like when the trade is made badly.

## 2.5 Caveat that must travel with these numbers

The gRPC lane is measured **full-stack against full-stack**: tonic's real
client and server against Thunder's real client and server. That is a fair
comparison of *stacks*, and it is the only one available — nobody can
hand-roll an HTTP/2 client without measuring the hand-roll. But it is not a
comparison of *wire formats in isolation*, and it should not be quoted as one.

Part of gRPC's deficit is HTTP/2 itself (125/102 bytes per op against
Thunder's 85/83, plus HPACK state and flow-control accounting), and part is
tonic's implementation. This lane cannot separate them, and does not claim to.

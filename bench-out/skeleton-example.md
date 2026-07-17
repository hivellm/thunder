# thunder-bench — transport shootout (skeleton, T1.6)

Transport-isolated: one no-op dispatch backend (echo / static-reply / sink), every lane in the same process, host, runtime and allocator (BEN-001).

## Environment

| Field | Value |
|---|---|
| os / arch | windows / x86_64 |
| cpus (logical) | 32 |
| hostname | BOLADO |
| rustc | rustc 1.96.0 (ac68faa20 2026-05-25) |
| build profile | release |
| timestamp | 2026-07-17T06:42:34Z (unix 1784270554) |
| kernel | unknown (platform probe lands at T4.3) |
| governor | unknown (platform probe lands at T4.3) |

## Run config

ops/rep = 256, warmup = 64 (discarded), repetitions = 2 — dispersion below is min…max across repetitions (BEN-011). Lanes: thunder, http. Scenarios: point-echo-64B, medium-4KiB, bulk-10k, embedding-768, pipelined-1k, connection-storm.

## Cells

| Scenario | Lane | Depth | Conns | Ops/rep | p50 µs | p99 µs | QPS | B/op in | B/op out | Status |
|---|---|---:|---:|---:|---|---|---|---:|---:|---|
| point-echo-64B | thunder | 1 | 1 | 256 | 30.7 (30.3…31.1) | 90.7 (61.8…119.5) | 28502 (26173…30830) | 85 | 83 | ok |
| point-echo-64B | thunder | 1 | 4 | 256 | 111.3 (110.9…111.8) | 185.2 (184.9…185.4) | 34501 (33838…35164) | 83 | 81 | ok |
| point-echo-64B | thunder | 16 | 1 | 256 | 214.2 (213.4…215.0) | 438.3 (385.3…491.3) | 67560 (64892…70227) | 85 | 83 | ok |
| point-echo-64B | thunder | 16 | 4 | 256 | 396.5 (377.3…415.7) | 691.5 (650.8…732.3) | 104793 (71192…138393) | 83 | 81 | ok |
| point-echo-64B | http | 1 | 1 | 256 | 77.3 (77.3…77.4) | 161.6 (108.7…214.5) | 12070 (11553…12588) | 181 | 158 | ok |
| point-echo-64B | http | 1 | 4 | 256 | 113.8 (112.0…115.5) | 264.6 (185.0…344.3) | 31104 (29293…32915) | 181 | 158 | ok |
| point-echo-64B | http | 16 | 1 | 256 | 121.6 (117.8…125.5) | 293.0 (284.2…301.7) | 69803 (68029…71576) | 181 | 158 | ok |
| point-echo-64B | http | 16 | 4 | 256 | 328.9 (319.1…338.6) | 542.1 (509.8…574.4) | 134185 (133313…135057) | 181 | 158 | ok |
| medium-4KiB | thunder | 1 | 1 | 256 | 32.2 (31.8…32.6) | 94.7 (81.1…108.2) | 26613 (26064…27163) | 16 | 4116 | ok |
| medium-4KiB | thunder | 1 | 4 | 256 | 118.9 (118.8…119.1) | 212.1 (210.4…213.9) | 31479 (31101…31857) | 14 | 4114 | ok |
| medium-4KiB | thunder | 16 | 1 | 256 | 281.8 (237.4…326.2) | 552.1 (498.4…605.8) | 25815 (5507…46124) | 16 | 4116 | ok |
| medium-4KiB | thunder | 16 | 4 | 256 | 480.4 (424.9…535.8) | 1000.7 (992.5…1009.0) | 73873 (36373…111372) | 14 | 4114 | ok |
| medium-4KiB | http | 1 | 1 | 256 | 93.6 (90.7…96.5) | 155.6 (132.3…178.8) | 10389 (9983…10794) | 117 | 4192 | ok |
| medium-4KiB | http | 1 | 4 | 256 | 136.0 (134.5…137.5) | 233.6 (212.5…254.7) | 25364 (24552…26176) | 117 | 4192 | ok |
| medium-4KiB | http | 16 | 1 | 256 | 248.9 (179.7…318.0) | 403.1 (369.6…436.6) | 50216 (45893…54540) | 117 | 4192 | ok |
| medium-4KiB | http | 16 | 4 | 256 | 478.6 (454.2…503.1) | 5974.8 (851.3…11098.2) | 22333 (18819…25846) | 117 | 4192 | ok |
| bulk-10k | - | 0 | 0 | 0 | - | - | - | - | - | pending — lands at T4.3 |
| embedding-768 | - | 0 | 0 | 0 | - | - | - | - | - | pending — lands at T4.3 |
| pipelined-1k | thunder | 1000 | 1 | 1000 | 6122.7 (5489.5…6755.9) | 11693.5 (10793.1…12593.8) | 76642 (72365…80919) | 85 | 83 | ok |
| pipelined-1k | thunder | 1000 | 4 | 4000 | 10616.1 (9712.3…11520.0) | 22935.2 (22743.7…23126.7) | 150749 (145270…156229) | 85 | 83 | ok |
| pipelined-1k | http | 1000 | 1 | 1000 | 1082.5 (764.8…1400.2) | 1687.7 (1359.1…2016.2) | 112708 (108119…117298) | 181 | 158 | ok |
| pipelined-1k | http | 1000 | 4 | 4000 | 2346.8 (2246.0…2447.6) | 5487.0 (4603.5…6370.6) | 189688 (182714…196661) | 181 | 158 | ok |
| connection-storm | thunder | 1 | 256 | 256 | 386.2 (382.6…389.8) | 694.2 (606.4…782.0) | 2140 (2057…2223) | 12 | 20 | ok |
| connection-storm | http | 1 | 256 | 256 | 366.6 (366.1…367.2) | 562.8 (549.3…576.4) | 2354 (2348…2360) | 115 | 98 | ok |

## Honesty notes

- **Skeleton scope (T1.6):** lanes = Thunder RPC + HTTP/1.1+JSON only; RESP3 and Bolt peers land at T4.2 (BEN-001). No G5 claim is made from this artifact (BEN-031).
- **Sweep:** connections {1, 4} of the frozen {1, 4, 16, 64}; the full sweep and the bulk-10k / embedding-768 scenarios land at T4.3 (BEN-010).
- **Parity (BEN-003):** both lanes keep a continuously full in-flight window per connection (no inter-batch gaps); latency is client-observed, request submission → response fully decoded; bytes/op come from server-side counters on both lanes.
- **Deep-burst latency semantics:** at depth = burst (pipelined-1k) the lanes stamp differently — the Thunder client's stamp includes waiting behind other slots' frame writes inside `call`, the HTTP sender stamps as its own bytes enter the socket. Compare qps on that row; the stamp point is unified in the T4.2 harness.
- **HTTP lane:** hand-rolled minimal HTTP/1.1 (`src/http.rs`); a production-grade axum lane may replace it at T4.2 if the parity review demands.

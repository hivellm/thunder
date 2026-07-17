# §3 — Conformance, Versioning and Governance

> The module's value is not the code — 18 teams have proven the code is writable — it is the *guarantee* that all copies emit and accept the same bytes forever. That guarantee is manufactured here.

## 3.1 The golden-vector corpus

### T-015 — One language-neutral corpus, consumed by all four test suites, replaces per-repo hex pasting

Format: data files under `conformance/vectors/`, each pairing exact frame bytes with the expected decoded structure, e.g.:

```yaml
# conformance/vectors/request-ping.yaml
name: request-ping
frame_hex: "08 00 00 00 93 01 a4 50 49 4e 47 90"      # from VECTORIZER_RPC.md §11
decoded:
  kind: request
  id: 1
  command: PING
  args: []
```

Corpus contents (start set — every item below is an edge that bit at least one existing SDK):

| Group | Vectors |
|---|---|
| Canonical pair | PING request, nested `{"Ok":{"Str":"PONG"}}` response (the two Vectorizer already pins) |
| Value matrix | every variant alone and nested; empty `Bytes`/`Str`/`Array`/`Map`; `Map` with non-string keys |
| Integer edges | `i64::MIN`/`MAX`, compact-int forms (Go needed `UseCompactInts`, C# needed the low-level writer to match) |
| Float edges | NaN bit pattern (Nexus tests it), ±inf, -0.0 |
| Result edges | `Err` string, `Err` with `[code] ` prefix, `NOAUTH`/`WRONGPASS` prefixes |
| Framing | two frames in one buffer, partial header, partial body, zero-length body, frame at exactly the cap, frame one byte over the cap (must reject **without allocating**) |
| Legacy tolerance | `Bytes` as int-array (Synap form — must decode, must not be emitted), `Request` as map (must decode server-side, must not be emitted) (T-005) |
| Push | frame with id `u32::MAX` (client must route to push hook / refuse per profile) |
| Handshake | Nexus `HELLO [Int(1)]` reply shape; Vectorizer HELLO map with `version`/`token`/`api_key`/`client_name` and capabilities reply |

Each language test suite has one loader (~50 LOC) that walks the corpus and asserts: decode(frame) == expected, encode(expected) == frame for canonical vectors, decode-only for tolerance vectors.

- **Impact**: today's guarantee is comments saying "byte-for-byte" plus one product's pasted hex (T-006). After this, byte-compatibility is a CI property. The corpus is also the free gift to the languages *not* yet ported (Go/PHP/Java keep their per-product transports initially but can adopt the corpus immediately).
- **Confidence**: high.

## 3.2 Cross-implementation checks in CI

1. **Corpus matrix** — 4 languages × full corpus on every PR.
2. **Reference cross-decode** — the Rust suite additionally decodes every canonical frame with `nexus-protocol` (dev-dependency) and re-encodes, asserting equality both ways: Thunder is pinned to the family's existing reference, not to itself (the same method the Lexum plan mandates, P1.3).
3. **Cross-language pairwise fuzz** — a generator emits random `Value` trees as JSON seeds; each language encodes its tree and every other language must decode it to the same tree. Catches encoding-choice drift (compact ints, bin vs seq) that fixed vectors miss.
4. **Live interop (env-gated)** — a smoke client per language against real Synap/Nexus/Vectorizer instances (`THUNDER_LIVE_URL_*`), following the family's existing `s2s`/`NEXUS_SDK_LIVE_TEST` gating convention. Not on the PR path; on the release path.

## 3.3 Spec home and versioning

### T-016 — Transplant the spec here; version the module by semver, the wire by the frozen v1

- **Spec**: copy `Nexus/docs/specs/rpc-wire-format.md` v1 into `Thunder/docs/spec/` verbatim with a provenance header; Nexus's copy gains a pointer. One normative home ends the "Nexus spec + Vectorizer golden vectors + Synap behavior" triangulation every adopter currently performs (the Lexum study needed all three sources — its §2 F-008 documents the split). The profile dimensions (T-010) get specified alongside, absorbing what would have been six per-product decisions per adopter.
- **Wire version**: stays `proto: 1`, frozen; the module's packages use independent semver. Adding a command, a profile field with a default, or a new language port = minor. Any byte change = it doesn't happen (that is what "frozen" means; a hypothetical v2 is a new negotiated proto integer, per the existing HELLO mechanism).
- **Tolerances are one-way**: the module *accepts* legacy forms (T-005) but *emits* only canonical bytes. Tolerance removal (dropping int-array `Bytes` decode once Synap ≥2 ships) = major.
- **Confidence**: high.

### T-017 — Governance: the module needs one owner and a compatibility gate, or it becomes a fourth divergent copy

- **Evidence for the risk**: the family already grew three copies with "byte-for-byte" comments and still drifted at the edges (T-005); Fluxum forked the envelope rather than coordinate (T-008).
- **Mechanics**: wire-affecting PRs require the corpus + cross-decode + pairwise-fuzz gates green (they cannot be feature-gated or `#[ignore]`d — the anti-pattern Lexum's F-022 documents); product repos consume released versions, never git paths, so a product cannot silently vendor-patch; a `CODEOWNERS` entry makes one team the wire authority.
- **Performance-claim discipline** (inherited from F-014/F-016/F-017): the module's README may state *mechanisms* freely, but quantitative claims ("3–10×", "beats Bolt/RESP3") must cite committed artifacts. The full benchmark program — the transport shootout Thunder vs Bolt vs RESP3 vs HTTP over a shared no-op engine, and the always-win gate G5 — is specified in [§6](06-benchmark-mandate.md); it closes the family's remaining gaps (Bolt conflated with engine, RESP3 never raced with a parity client) in one place.
- **Confidence**: high.

---

Next: [§4 — Adoption plan](04-adoption-plan.md).

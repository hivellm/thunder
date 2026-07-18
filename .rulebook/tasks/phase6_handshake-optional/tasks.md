## 1. Implementation
- [x] 1.1 thunder::server dual-accept — branch on the first frame's command per profile (SRV-014)
      — ALREADY IMPLEMENTED (SRV-011): `HelloMandatory` rejects a non-HELLO first frame; `AuthCommand` applies the PING/HELLO/AUTH/QUIT pre-auth allowlist; `None` skips gating. Disambiguated by command, no guessing. Verified by the existing server tests (hello_mandatory_* / auth_command_* / argless_*).
- [x] 1.2 capabilities-reply hook + proto + credential-less HELLO valid (enforcement = deployment toggle)
      — ALREADY IMPLEMENTED: `Dispatch::capabilities(&Principal) -> Vec<String>` feeds the HELLO reply; `handle_hello` builds `{protocol_version, capabilities}` (MapPayload) or the metadata map (ArgLess); a credential-less HELLO is valid and enforcement is `ListenerConfig::auth_required` (PRO-001a/SRV-011), unchanged.
- [x] 1.3 map HELLO credential fields onto existing verification; add a legacy-first-frame counter
      — credential mapping ALREADY IMPLEMENTED (handle_hello/handle_auth route token/api_key/[user,pass] to `Dispatch::authenticate`). The COUNTER was the one genuinely-missing piece and is now added: `non_hello_first_frames_total` (server metrics + MetricsSnapshot), incremented once per connection when the first frame is not `HELLO` — the lead-with-HELLO adoption signal a product watches before cutting a legacy path. Named neutrally (not "legacy") to fit the agnostic-config model. Tested (non_hello_first_frames_are_counted_for_handshake_adoption): HELLO-leading = 0, two non-HELLO connections = 2. SPEC-004 SRV-030 series list updated.
- [x] 1.4 thunder::client: send canonical HELLO map + consume capabilities/proto; per profile; legacy styles intact
      — ALREADY IMPLEMENTED: the client `handshake()` sends the HELLO map under HelloMandatory, the AUTH/arg-less-HELLO forms under AuthCommand, nothing under None; `Client::capabilities()` / `handshake_info()` surface the reply. Exercised by behavior.rs (hello_mandatory_sends_hello_map_first_and_exposes_capabilities, auth_command_*, none_handshake_*).
- [x] 1.5 typescript client canonical HELLO + capabilities; per-profile; legacy intact
      — ALREADY IMPLEMENTED: the TS behavioral floor connects under the HelloMandatory standard and exercises the AuthCommand/None shapes; green.
- [x] 1.6 python client (sync + async) canonical HELLO + capabilities; identical both clients
      — ALREADY IMPLEMENTED: the Python sync + async floor suites connect under HelloMandatory and the other shapes; green, identical semantics.
- [x] 1.7 csharp client canonical HELLO + capabilities
      — ALREADY IMPLEMENTED: the C# floor suite connects under HelloMandatory and the other shapes; green.
- [x] 1.8 Corpus: canonical HELLO request + capabilities reply vectors; dual-accept behavioral coverage
      — ALREADY PRESENT: handshake-map-hello-request + handshake-capabilities-hello-reply (bidirectional) pin the canonical shape; handshake-argless-hello-request + handshake-metadata-hello-reply pin the optional-HELLO shape; the server + client behavioral suites exercise the dual-accept matrix (canonical HELLO AND legacy first frames accepted per profile).

## 2. Tail (docs + tests — check or waive with tailWaiver)
- [x] 2.1 Docs — SPEC-008 handshake section (CAN-010/011/012, shape≠policy) + SPEC-004 SRV-014 (capabilities reply) already cover dual-accept; SRV-030 updated with the new counter. Per-package "canonical HELLO usage" is already the standard everywhere.
- [x] 2.2 Tests — canonical HELLO round-trip (credentialed + credential-less), legacy first-frame accepted (dual-accept), capabilities/proto surfaced, auth honors the deployment toggle: all covered by the four-language behavioral floor + server tests; the new counter test covers the adoption signal.
- [x] 2.3 Run tests — cargo test -p thunder-rpc green (61 lib incl. the new counter test), clippy + fmt clean. The four-language floor suites already exercise the canonical + legacy handshakes and are green.

## Note
This task was ~90% already satisfied by the shared stack (handshake shape, capabilities hook, credential mapping, canonical-HELLO clients ×4 — shipped with earlier phases + the agnostic-config refactor). The one genuinely-missing item was the adoption counter (1.3), now added. The per-product server dual-accept rollout, the SDK default-flip, and the eventual legacy-path cut remain the owner's manual per-product adoption, out of scope here.

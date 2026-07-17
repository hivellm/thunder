## 1. Implementation
- [x] 1.1 Rust (the reference): `wire/profile.rs` → `wire/config.rs`; `Profile` → `Config`; delete `synap()`/`nexus()`/`vectorizer()`/`lexum()`/`registry()` — no product name survives anywhere in the library
- [x] 1.2 Rust: `Config::standard()` + `impl Default` carrying the canonical behavior (HelloMandatory + MapPayload + proto/capabilities, `[CODE]` error superset, 64 MiB, 256 in-flight, push reserved, TLS off); `scheme`/`port` have NO default — identity is the application's
- [x] 1.3 Rust: a builder for every dimension (`.scheme()`, `.port()`, `.handshake()`, `.hello_style()`, `.push()`, `.max_frame_bytes()`, `.max_in_flight()`, `.error_codes()`, `.tls()`), so a diverging application expresses that in its own repo; keep direct struct construction working (a config is plain data)
- [x] 1.4 Conformance: delete `profiles/{synap,nexus,vectorizer,lexum}.yaml`; add `conformance/standard.yaml` pinning the standard's defaults — the cross-language agreement guarantee survives without any product name
- [x] 1.5 Rust: rewrite the pinning test to pin `Config::standard()` to `standard.yaml`; add a test proving a custom config is constructible without touching Thunder (the "future implementation" case) and one proving overrides compose
- [x] 1.6 `thunder-bench`: its `bench_profile()` becomes the worked example of an application defining its own config via the builder
- [x] 1.7 Mirror to TypeScript, Python, C#: same rename, same deletions, same `standard()` + builder, same pinning against `standard.yaml`
- [x] 1.8 Rewrite SPEC-002 (no longer a product registry — delete the PRO-010/PRO-011 registry requirements; keep PRO-003 "config is data, never behavior" and PRO-001a shape≠policy); update SPEC-003/004/005 references
- [ ] 1.9 DEFERRED: the corpus vectors are still named for products (`handshake-nexus-hello-request`, `handshake-vectorizer-hello-*`). They pin wire SHAPES, which stay valid; renaming files touches the golden corpus, so it is its own change with its own review — filed as follow-up, not silently skipped
- [x] 1.10 Docs: README, ARCHITECTURE, PRD/DAG/ROADMAP, analysis cross-links — Thunder is a protocol library, not a product catalogue; the verified per-product facts stay in `docs/analysis/` as configuration reference

## 2. Tail (docs + tests — check or waive with tailWaiver)
- [x] 2.1 Update or create documentation covering the implementation — SPEC-002 rewritten, READMEs/ARCHITECTURE reframed, the standard documented once
- [x] 2.2 Write tests covering the new behavior — standard pinned to `standard.yaml` in all four languages; custom-config construction proven; override composition proven
- [x] 2.3 Run tests and confirm they pass — full gate green in all four languages, verified independently

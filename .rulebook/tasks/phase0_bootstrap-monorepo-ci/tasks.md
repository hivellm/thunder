## 1. Implementation
- [x] 1.1 Repo layout: `rust/` workspace (thunder-wire, thunder-client, thunder-server, thunder-bench — empty lib skeletons), `typescript/`, `python/`, `csharp/`, `conformance/{vectors,profiles}/` (PKG-001) — *note: the three library crates were later consolidated into one `thunder` crate with feature-gated layers; the workspace is now `thunder` + `thunder-bench`*
- [x] 1.2 Rust workspace lints (clippy `-D warnings`, `unwrap_used`/`expect_used` denied) + rustfmt config, matching the family posture
- [x] 1.3 TypeScript skeleton: tsup ESM+CJS build, tsc strict, eslint, vitest wired
- [x] 1.4 Python skeleton: hatchling `hivellm-thunder` (import `thunder_rpc`), ruff, pytest wired
- [x] 1.5 C# skeleton: `HiveLLM.Thunder` net8.0 project + test project, `-warnaserror`
- [x] 1.6 CI matrix (PKG-002): Rust fmt+clippy+test ×3 OS; TS/Python/C# lanes; corpus lanes. **A phase-0 review audited this against the workflows on disk and found two PKG-002 requirements missing, both now closed:** (a) *corpus lanes per TST-020/021 did not exist* — the only trace was two step **names** asserting coverage, and the TS/Python corpus tests were not in `ci.yml` at all; each lane now ends with a real corpus step, verified locally (Rust 8, TS 39, Python 39, C# 39); (b) *the "type-check → lint → tests" gate order was not enforced* — lint and test lived in separate workflows running concurrently, so tests ran even when lint failed; each lane is now ordered steps in one job. Also fixed while open: `npm install` → `npm ci` (the lockfile was being ignored), the `python` lane was an import check masquerading as the Python lane (now ruff + pytest), and the Python lint was running unconfigured at repo root (now scoped to `python/`, so its own line-length 100 applies)
- [x] 1.7 Root README status refresh once CI is green

## 2. Tail (docs + tests — check or waive with tailWaiver)
- [x] 2.1 Update or create documentation covering the implementation
- [x] 2.2 Write tests covering the new behavior
- [x] 2.3 Run tests and confirm they pass

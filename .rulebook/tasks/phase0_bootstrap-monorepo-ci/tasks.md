## 1. Implementation
- [x] 1.1 Repo layout: `rust/` workspace (thunder-wire, thunder-client, thunder-server, thunder-bench — empty lib skeletons), `typescript/`, `python/`, `csharp/`, `conformance/{vectors,profiles}/` (PKG-001)
- [x] 1.2 Rust workspace lints (clippy `-D warnings`, `unwrap_used`/`expect_used` denied) + rustfmt config, matching the family posture
- [x] 1.3 TypeScript skeleton: tsup ESM+CJS build, tsc strict, eslint, vitest wired
- [x] 1.4 Python skeleton: hatchling `hivellm-thunder` (import `thunder_rpc`), ruff, pytest wired
- [x] 1.5 C# skeleton: `HiveLLM.Thunder` net8.0 project + test project, `-warnaserror`
- [ ] 1.6 CI matrix (PKG-002): Rust fmt+clippy+test ×3 OS; TS/Python/C# lanes; corpus lane placeholders
- [x] 1.7 Root README status refresh once CI is green

## 2. Tail (docs + tests — check or waive with tailWaiver)
- [x] 2.1 Update or create documentation covering the implementation
- [x] 2.2 Write tests covering the new behavior
- [x] 2.3 Run tests and confirm they pass

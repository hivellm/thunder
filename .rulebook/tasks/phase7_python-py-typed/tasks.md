## 1. Implementation
- [ ] 1.1 Add the empty marker file `python/thunder_rpc/py.typed`
- [ ] 1.2 Confirm hatchling includes it in **both** the wheel and the sdist — build each and list the contents rather than trusting the default
- [ ] 1.3 If it is not picked up automatically, add the explicit package-data entry to `pyproject.toml`

## 2. Tail (docs + tests — check or waive with tailWaiver)
- [ ] 2.1 Update or create documentation covering the implementation — note in `python/README.md` that the package is typed (PEP 561), so consumers know they can drop any `ignore_missing_imports` override
- [ ] 2.2 Write tests covering the new behavior — a check that the built artifacts contain `py.typed`, so a future packaging change cannot silently drop it again. Ideally also verify a consumer outside the repo resolves the types (install the wheel into a temp env and run a type checker against a two-line consumer)
- [ ] 2.3 Run tests and confirm they pass — full Python gate green

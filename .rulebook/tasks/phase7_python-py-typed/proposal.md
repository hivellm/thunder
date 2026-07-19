# Proposal: phase7_python-py-typed (GH #7, filed by the Synap SDK swap)

## Why
`thunder_rpc` is thoroughly annotated — frozen dataclasses, `-> Value | None`
accessors, typed `ClientConfig` fields — but the distribution ships no
`py.typed` marker. Under PEP 561 that means **every downstream type checker
treats the package as untyped**, so the annotations that already exist never
reach a single consumer:

```
synap_sdk/client.py:11: error: Skipping analyzing "thunder_rpc": module is
    installed, but missing library stubs or py.typed marker  [import-untyped]
```

The consequences are worse than a missing convenience:

- Every `Value`, `Client` and `Config` a consumer touches degrades to `Any`, so
  mistakes Thunder's own types would have caught pass silently **at the wire
  boundary** — the place a protocol library most wants to be strict.
- A consumer on `strict` gets an error it cannot fix from its side except by
  writing stubs or suppressing the import.
- The annotations can drift from reality unnoticed, because nothing downstream
  exercises them.

Synap's SDK is currently in the third position: a `[[tool.mypy.overrides]]`
entry that suppresses the import error **and every real type mismatch with it**.

## What Changes
An empty `python/thunder_rpc/py.typed`, included in the wheel and the sdist.
That is the whole fix — the annotations are already written.

Worth verifying rather than assuming: that hatchling actually ships the file in
both artifacts, and that a consumer outside the repo sees the types.

## Impact
- Governing spec: SPEC-006 (packaging)
- Affected code: python/thunder_rpc/py.typed (new), python/pyproject.toml if
  hatchling needs telling
- Breaking change: **NO** — purely additive; consumers who suppressed the
  import can drop the suppression at their own pace
- User benefit: the type annotations Thunder already has start protecting the
  people using it

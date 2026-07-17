# Thunder Conformance Assets

Language-neutral fixtures consumed by every Thunder implementation's default test run
(SPEC-005). One PR changes wire behavior in all languages at once, or fails CI.

## `vectors/` — golden byte vectors (TST-001)

One YAML file per vector:

```yaml
name: request-ping            # unique, kebab-case
group: canonical              # canonical | value | framing | tolerance | push | handshake
mode: bidirectional           # see below
frame_hex: "08 00 00 00 ..."  # space-separated hex of the COMPLETE frame (or partial input)
decoded: { ... }              # expected structure (bidirectional / decode-only)
error: frame_too_large        # expected error class (reject): frame_too_large | decode
notes: "..."                  # provenance + the WIRE-xxx requirements this vector covers
```

`mode` semantics:

| mode | assertion |
|---|---|
| `bidirectional` | `decode(frame) == decoded` **and** `encode(decoded) == frame`, byte-exact |
| `decode-only` | decode succeeds and equals `decoded`; encoding this form is forbidden (legacy tolerances WIRE-011/013) |
| `reject` | decode fails with the named `error` class; `frame_too_large` vectors must reject **without allocating** the body |
| `incomplete` | decoder reports "need more bytes" (no value, no error) |

`decoded` value nodes: `{type: null|bool|int|float|str|bytes|array|map, value: ...}`;
`bytes` values are space-separated hex; `map` values are lists of `[key, value]` pairs.
Requests: `{kind: request, id, command, args: [<value>...]}`. Responses:
`{kind: response, id, ok: <value>}` or `{kind: response, id, err: "<string>"}`.

Canonical bytes never change (wire v1 is frozen — NFR-01). Adding vectors is a minor
change; the full 1.0 corpus lands at DAG T1.2 (`phase1_conformance-harness`).

## `profiles/` — the family profile registry (SPEC-002, PRO-010)

One YAML per registered product profile (synap, nexus, vectorizer, lexum). These files
are the single source the per-language `Profile` constants are generated from; a
product's server and SDKs can therefore never disagree. Custom profiles remain
constructible in code — the registry never gates a new product (PRO-020).

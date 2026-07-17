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
frames: [ { ... }, ... ]      # expected structures, in order (stream mode only)
error: frame_too_large        # expected error class (reject): frame_too_large | decode
max_frame_bytes: 8            # optional cap override (default 64 MiB) for framing vectors
notes: "..."                  # provenance + the WIRE-xxx requirements this vector covers
```

`mode` semantics:

| mode | assertion |
|---|---|
| `bidirectional` | `decode(frame) == decoded` (structural, floats by bit pattern) **and** `encode(decoded) == frame`, byte-exact |
| `decode-only` | decode succeeds and equals `decoded`; encoding this form is forbidden (legacy tolerances WIRE-011/013) — loaders also assert `encode(decoded) != frame` |
| `stream` | the buffer holds several frames back-to-back; sequential decodes yield `frames` in order, one frame per decode, consuming the buffer exactly |
| `reject` | decode fails with the named `error` class; `frame_too_large` vectors must reject **without allocating** the body |
| `incomplete` | decoder reports "need more bytes" (no value, no error) |

`decoded` value nodes: `{type: null|bool|int|float|str|bytes|array|map, value: ...}`;
`bytes` values are space-separated hex; `array` values are lists of nodes; `map` values
are lists of `[key, value]` node pairs. Float nodes MAY carry `bits` instead of `value` —
the u64 IEEE-754 bit pattern as a hex string — and loaders MUST compare floats by bit
pattern: NaN never compares equal numerically and `-0.0 == 0.0` would hide the sign bit.
Requests: `{kind: request, id, command, args: [<value>...]}`. Responses:
`{kind: response, id, ok: <value>}` or `{kind: response, id, err: "<string>"}`.

All `frame_hex` bytes are generated programmatically from a reference encoder and
pasted verified — never hand-computed. Legacy tolerance frames come from the legacy
encoders themselves (`nexus-protocol` for int-array Bytes, `rmp_serde::to_vec_named`
for map-shaped requests).

Canonical bytes never change (wire v1 is frozen — NFR-01). Adding vectors is a minor
change; the full 1.0 corpus lands at DAG T1.2 (`phase1_conformance-harness`).

## `standard.yaml` — THE standard configuration (SPEC-002, PRO-011)

One file, because Thunder ships **one** configuration and **zero** product knowledge. There is no
per-product registry: a protocol library that must serve implementations which do not exist yet
cannot ship a hardcoded list of the ones that did.

Every language pins its `Config::standard()` to this file in its default test run (PRO-013), so the
four implementations can never disagree about what "standard" means — the one guarantee the deleted
per-product registry legitimately provided.

`scheme` and `default_port` are deliberately absent: identity is the application's. An application
starts from the standard and overrides only what it diverges on, in its own repository (PRO-020/021).

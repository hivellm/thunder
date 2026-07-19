# Proposal: phase8_php-port — Thunder as a sixth language lane

## Why

Thunder exists so no product hand-writes the protocol again. A language without
a Thunder SDK is a language where that is still happening, and PHP is one of
them.

This task ships the **wire layer and the conformance loader** — the part that
decides whether PHP can be in the family at all. The client (SPEC-003) is a
separate task built on top of this, and is worthless without it.

## Feasibility, checked before committing to the design

The port turns on one question: can a PHP MessagePack library reproduce the
canonical bytes `rmp-serde` emits? A hand-rolled codec is forbidden (WIRE-031,
PRD NFR-02), so if no library reproduces them, the lane does not exist.

`rybakit/msgpack` v0.9 was probed against the corpus's pinned bytes *before* any
SDK code was written:

| Requirement | Expected | Got |
|---|---|---|
| `request-ping` body (WIRE-001/012) | `93 01 a4 50 49 4e 47 90` | identical |
| Shortest-form ints (WIRE-014) | `127`→`7f`, `128`→`cc 80`, `-33`→`d0 df` | identical |
| `Bytes` as msgpack `bin` (WIRE-010) | `c4 03 01 02 03` | identical |
| f64 always, bits preserved (WIRE-014) | `cb 3f f8 …` for 1.5 | identical |
| `Null` as bare string (WIRE-003) | `a4 4e 75 6c 6c` | identical |
| Payload variant as fixmap-of-1 (WIRE-003) | `81 a3 49 6e 74 2a` | identical |

The lane is viable and the library choice is settled by evidence rather than
reputation.

## What Changes

A new top-level `php/` directory (PKG-001), laid out like every other lane:

- `src/Wire/` — the pure wire layer (WIRE-030: no sockets, no timers, no product
  knowledge): the 8-variant `Value`, `Request`/`Response`, the externally-tagged
  codec, and length-prefixed framing with the cap checked before allocation.
- `tests/` — unit tests plus the corpus loader (TST-020), running in the
  **default** test command, never gated.

## Two problems this port has that no other lane had

**PHP does not distinguish bytes from text** — both are `string`. WIRE-015
forbids smuggling one as the other, so `Value` carries an explicit kind and is
built only through factories, the same shape Go uses and for the same reason.
Inferring the variant from the PHP value would make `Str` and `Bytes`
indistinguishable on encode, which the corpus would catch as a byte diff.

**PHP integers are signed 64-bit with no unsigned type.** Frame ids are `u32`
and `PUSH_ID` is `0xFFFFFFFF`; that fits a PHP int on a 64-bit build but must
never be sign-extended or compared as negative. Ids are validated on the way in
rather than trusted.

## Impact
- Affected specs: SPEC-001 (wire) in full; SPEC-005 TST-020 (loader). Amendments
  needed: a PHP row in WIRE-031 naming `rybakit/msgpack`, and a `PKG-051`-shaped
  lane entry in SPEC-006 with its registry (Packagist)
- Affected code: new `php/` tree only — no existing lane is touched
- Breaking change: NO. A new lane cannot break an existing one; per PKG-012 a new
  language port is a **minor** bump
- User benefit: PHP consumers get the family wire instead of a private
  reimplementation — the thing Thunder exists to end
- Not in this task: the client (CLT-001..090, including the mandatory pool), the
  interop probe, and the CI lane. Each depends on this landing first

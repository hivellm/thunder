# Proposal: phase7_principal-identity (issue #5 of the Synap adoption set, GH #4)

## Why
`Dispatch::authenticate` returns `Principal { name: String }`, and
`Session::principal()` hands back a clone. A product's own resolved identity —
roles, permissions, quotas, tenant — has nowhere to live.

Synap therefore re-resolves the user from its credential store on **every**
admin-gated command, where before it read a field off a struct already in
memory.

That is not only a cost. It is a **semantic** change Thunder imposed silently:
the per-command lookup re-reads live state, so a user edited or deleted
mid-session is now evaluated against the new record, where the pre-Thunder
server evaluated the identity captured at `AUTH`. Either semantics is
defensible; a transport library should not pick for its consumers by accident.

## What Changes
Let the product attach its own payload at authentication time, carried on the
session and returned by reference.

**The issue's sketch uses an associated type with a default
(`type Identity = ()`), which is unstable Rust** (`associated_type_defaults`).
Task 1.1 exists to pick a shape that works on stable and does not force every
existing `Dispatch` impl to name a type it does not need.

## Impact
- Governing spec: SPEC-004 (SRV-012 authentication, Session)
- Affected code: rust/thunder/src/server/ (Dispatch, Principal, Session)
- Breaking change: **likely YES** — depends on the shape chosen in 1.1; adding
  a field to the public `Principal` struct breaks literal construction, and a
  generic parameter on `Dispatch` touches every impl
- Ships in: **0.2.0** alongside phase7_bytes-zero-copy, unless 1.1 finds a
  fully additive shape
- User benefit: authorization reads memory instead of the credential store, and
  session identity semantics become the product's explicit choice again

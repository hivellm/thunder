# Proposal: phase2_lexum-onboarding

## Why
Lexum's gated RPC plan has it building a fourth wire-crate copy at its P1 ("create crates/lexum-protocol", Lexum/docs/analysis/hivellm-rpc/05-execution-plan.md). With Thunder landed first, Lexum adopts a dependency instead - negative effort relative to the plan it replaces (analysis T-019). As the first green-field consumer with no legacy, Lexum is the proof that "a new family project onboards by picking profile values" (T-010) actually holds.

## What Changes
Lexum depends on thunder-wire + thunder-server with Profile::lexum(): HelloMandatory handshake, MapPayload hello, error_codes Both (BracketCode + Resp3Prefixes), scheme lexum:// with default port 17001 (PRO-011). The RPC listener is thunder-server plus a dispatch-trait adapter over Lexum's command handlers; profile enforcement (non-HELLO first frame rejected with the Both error convention) comes from Thunder, not product code (PRO-030). Lexum's SPEC-015 is written referencing Thunder's spec instead of respecifying bytes. The planned lexum-protocol crate is never created - this task REPLACES Lexum's P1 outright.

## Impact
- Governing spec: SPEC-002 (PRO-011 lexum row, PRO-013, PRO-030) - docs/specs/SPEC-002-profiles.md
- PRD requirements: FR-11
- DAG: T2.5 (gate G2); depends on G1
- Affected code: e:\HiveLLM\Lexum - new RPC listener on thunder-server, SPEC-015, execution plan supersession; Thunder - conformance/profiles/lexum.yaml exercised under the lexum profile
- Breaking change: NO (green-field; Lexum ships no RPC surface today)
- User benefit: Lexum's RPC lands with the family floor (caps, handshake enforcement, typed errors) built in, and a fourth wire-crate copy never exists

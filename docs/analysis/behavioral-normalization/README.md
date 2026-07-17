# Behavioral Normalization of the Thunder Protocol — Feasibility, Blockers, Migration

> **Question**: Thunder's feasibility study already proved the family can share one codec and one
> client contract while expressing per-product differences as a declarative **Profile** — its stated
> goal is *"Profiles, not forks."* This analysis asks the sharper, next question: can those
> behavioral differences (handshake, errors, push, caps, TLS) be **eliminated** so Nexus, Vectorizer,
> Synap and Lexum all speak *exactly* the same way — collapsing the four-row profile table to one?
>
> **Answer**: **Yes — and four of the five dimensions get there at near-zero cost.** The entire
> residual difficulty concentrates in a single dimension, the handshake, and even that becomes
> tractable once you separate handshake *shape* (which normalizes) from auth *policy* (which is
> legitimately per-deployment config, not a protocol dialect). The end state is one canonical
> behavior with four address labels on it: the profile registry keeps `scheme`, `port` and command
> catalog — a product's *identity* — and sheds every *behavioral* field. Two preconditions make it
> cheap: run it as Thunder's **sequel** (after products are on Thunder, so each convergence is a
> shared-server change plus a profile flip, not 18 transport edits), and pin the canonical behavior
> **before Lexum ships its RPC listener** so a green-field product never implements a divergent one.
>
> **Research date**: 2026-07-17. Built on Thunder's feasibility analysis (`docs/analysis/`, T-001..T-030),
> the four committed profiles (`conformance/profiles/*.yaml`), SPEC-001..004, the canonical wire spec
> (`docs/spec/rpc-wire-format.md`), and the Lexum adoption study (`Lexum/docs/analysis/hivellm-rpc/`,
> F-011/F-012/F-019..F-024) — then **verified against the product sources themselves** (Nexus, Synap,
> Vectorizer RPC listeners, file:line-anchored). That sweep *corrected* three inherited claims and
> exposed three profile-registry errata (BN-023). Findings are numbered **BN-001..BN-023** (a prefix
> distinct from the sibling `T-` and `F-` series so cross-references never collide).

## Section index

| § | File | Contents |
|---|---|---|
| §1 | [01-divergence-inventory.md](01-divergence-inventory.md) | What "speak the same way" means (identity vs behavior); the wire is already normalized; the five behavioral dimensions today, per product; the registry errata the source sweep exposed (BN-001..BN-007, BN-023) |
| §2 | [02-feasibility-by-dimension.md](02-feasibility-by-dimension.md) | The single canonical behavior per dimension, cheapest → hardest; the shape-vs-policy split that unlocks the handshake; consolidated verdict (BN-008..BN-013) |
| §3 | [03-blockers-by-product.md](03-blockers-by-product.md) | Where the handshake migration bites, product by product — Lexum (none) → Vectorizer (reference) → Nexus (fold AUTH) → Synap (the load-bearing blocker) (BN-014..BN-017) |
| §4 | [04-migration-path.md](04-migration-path.md) | Converging without a flag day: HELLO/`proto` negotiation, server-first dual-accept, the profile-as-convergence-ledger, backward-compat guarantees (BN-018..BN-021) |
| §5 | [05-execution-plan.md](05-execution-plan.md) | Phased N0–N4 plan layered onto Thunder's milestones; why normalization is cheap only *after* consolidation (BN-022) |

## Executive summary

**The wire is already the same; the divergence is entirely in the connection's behavior.** Framing,
the value model, the externally-tagged encoding and the `PUSH_ID` reservation are byte-identical
across all four products by construction (BN-002). What differs sits above the bytes and is captured,
today, by four different rows of Thunder's profile registry: three incompatible handshakes, two error
grammars, a push producer vs three reservers, a 512-vs-64 MiB cap, and RPC TLS running in **zero**
products (one specs it, unwired) (BN-001..BN-007). Thunder chose to *parameterize* these as data; the
question here is whether they can instead be *eliminated*. (The source sweep behind this inventory
also caught the registry itself misdescribing three cells — see BN-023.)

**Four of the five dimensions collapse at near-zero cost.** Caps converge to a 64 MiB configurable
default — and Synap, the lone 512 MiB outlier, already caps at 64 in its own SDK, so nothing
observable changes (BN-008). TLS is a *missing capability* in **all four** products (Vectorizer
specs it but never wired it — BN-023), not a conflicting behavior, so shipping Thunder's one
optional rustls layer — the family's first running RPC TLS — to all four is purely additive and
breaks no deployed client (BN-009). Push is already wire-uniform — every client already reserves
and routes `PUSH_ID` — so the `enabled`/`reserved` flag degrades to a per-product *capability*
(Synap has a `SUBSCRIBE` command; others don't), identical in kind to "Nexus has `CYPHER`" (BN-010).
Error grammars differ on the wire but **no deployed client parses either today**, so converging every
server onto one superset grammar (`[CODE] message` with the auth tokens as codes — already Lexum's
target) has essentially no client blast radius (BN-011).

**The entire residual difficulty is one dimension — the handshake — and one product — Synap.** Three
mutually incompatible handshake models exist (Synap: bare `AUTH`, no HELLO; Nexus: optional arg-less
HELLO + separate `AUTH`; Vectorizer/Lexum: mandatory-HELLO map), and unlike the other four
dimensions, converging them changes *the first frame a deployed client sends* (BN-003, BN-012). The
unlock is a distinction the current profile model blurs: **"mandatory HELLO" is a statement about
frame ordering and negotiation, not about whether authentication is required.** Separate the
handshake *shape* (normalize to a mandatory HELLO map with `proto` negotiation and a capabilities
reply — already what Vectorizer/Lexum do) from auth *enforcement* (stays per-deployment config,
exactly like TLS on/off) — a split both credentialed products already practice via their own config
toggles. The source sweep made Synap **more tractable than the registry suggests**: its RPC path
*already* authenticates (`AUTH` + `NOAUTH` gate on the shared `UserManager`, behind `require_auth` —
the registry's `handshake: none` is an errata, BN-017/BN-023), so its migration is "put a HELLO in
front of existing auth" and flip six SDKs — no auth subsystem to build. What remains is genuinely a
migration, not a defaulting exercise: it needs a server-first, dual-accept transition per product,
riding the `HELLO`/`proto` channel the wire already carries and the exact template the family
already ran for Synap's `Bytes` canonicalization (BN-018, BN-019).

**The profile does not vanish — it converges.** Column by column, each product's behavioral row moves
to the single canonical value; when a column is identical everywhere it is promoted out of the
per-product profile into a family constant and the field is retired (BN-020). The finish line is a
CI property: the conformance suite goes from asserting four handshake/error behaviors to asserting
one, with the legacy forms demoted to decode-only tolerance vectors. What stays per-product is
exactly what *should* — scheme, port, command catalog (identity) and auth-required/TLS-on/cap-override
(deployment policy) — neither of which is a "they speak differently" problem (BN-013, BN-021).

**Sequencing is the strategic lever.** Every convergence is cheap *after* a product is on Thunder (a
shared-server change plus a four-line profile flip) and expensive *before* it (18 transport edits);
so normalization is Thunder's natural sequel, gated on M2/M3, not a parallel effort (BN-022). And the
cheapest possible moment to normalize Lexum is *before it starts* — pin the canonical behavior first
and Lexum's green-field RPC listener implements the one family behavior from day zero, becoming the
proof that a new product onboards onto a single canonical protocol with no transition (BN-014).

### Top findings by impact

| # | Impact | Finding |
|---|---|---|
| BN-001 | Scope | Two profile fields are *identity* (scheme, port), not divergence; normalization targets the five *behavioral* dimensions only — this is what makes "the same way" achievable without making four products one product |
| BN-002 | Sizing | The wire layer is already byte-identical; every remaining divergence is connection *behavior*, so this is a behavioral project layered on Thunder's structural one, not a second wire fork |
| BN-012 | The unlock | Separating handshake *shape* (normalize) from auth *policy* (deployment config) turns the hardest dimension from "every product rebuilds auth" into "every product leads with a HELLO" |
| BN-013 | Verdict | Feasible; 4 of 5 dimensions converge at near-zero cost; the four-row profile collapses to one canonical behavior + four address labels; auth-enforcement policy stays per-deployment by design |
| BN-017 | The blocker, resized | Synap needs a HELLO where none exists (six SDK flips) — but its RPC path *already* authenticates behind `require_auth`, so no auth build is on the critical path (the registry's `none` is an errata) |
| BN-023 | Errata | The source sweep caught the registry misdescribing three cells (`synap.handshake`, `nexus.hello_style`, `vectorizer.tls`) plus one corpus vector and an unmodeled `NOPERM` token — one coordinated fix owed regardless of normalization |
| BN-014 | The lever | Lexum is the forcing function — pin the canonical behavior before its RPC listener exists and it never builds a divergent handshake to migrate later |
| BN-019 | How | Server-first dual-accept per product, riding the `HELLO`/`proto` channel — the exact template the family already ran for Synap's `Bytes` change; no flag day |
| BN-020 | Finish line | The profile becomes a convergence ledger: behavioral columns unify, then retire into family constants; "same way" ends up a CI property, not a convention |
| BN-022 | Sequencing | Normalization is cheap only after consolidation (shared-server change + profile flip vs 18 transport edits) — Thunder's sequel, gated on M2/M3 |
| BN-010 | Cheap win | Push is already wire-uniform; the divergence is a capability, not a dialect — and normalizing it lets a future family push feature land once, not per-product |

### Recommendation

**Proceed, as Thunder's sequel, in the order the costs dictate.** (0) First and unconditionally:
fix the BN-023 registry errata in one coordinated Thunder commit, once the in-flight T3 language
packages land — that is correctness owed today, not normalization. (1) During Thunder M1, ratify the
canonical behavior in a normative spec — pinning the mandatory-HELLO-map shape, the `[CODE]` error
superset (recognizing `NOPERM`), the 64 MiB default, the uniform optional TLS, the uniform push
hook, and above all the **shape ≠ auth-policy** principle (N0). (2) As each product completes its
Thunder swap, take the four cheap dimensions — caps, TLS, push, errors — to a single family value;
this delivers "four of five dimensions identical" within roughly two weeks of engineering and no
cross-product coordination (N1, N2). (3) Run the handshake convergence as a per-product,
server-first, dual-accept migration on its own evidence-gated calendar, with Vectorizer as the
reference, Nexus folding `AUTH` into HELLO, and Synap putting a HELLO in front of its existing
`AUTH` (N3). (4) Retire each behavioral profile field as its column reaches parity, until the
conformance suite asserts one behavior and the profile carries only identity (N4). And **pin the
canonical behavior before Lexum's RPC listener is written** — that is a free normalization that
would otherwise become a fifth thing to reconcile.

## How this analysis relates to Thunder's existing plan

Thunder's feasibility analysis and PRD deliberately stop at *"Profiles, not forks"* — one codec, one
client contract, per-product behavior parameterized as data. That is the right first step and a
prerequisite for this one. This analysis argues the profile is not the terminal state but a
**convergence ledger**: the same registry that today encodes the four products' differences can drive
them to a single canonical behavior and end up encoding only their names. Nothing here contradicts
the Thunder plan; it extends it one milestone further — from "one implementation, four behaviors" to
"one implementation, one behavior, four addresses."

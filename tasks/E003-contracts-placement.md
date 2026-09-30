## Task
Resolve where `crpg-contracts` can live so it can actually implement traits for
`crpg-net` (and other consumers) — or drop the "contracts provides cross-crate
implementations" spec promise. Human-decision task.

## Why this is deferred / blocked

Spec "Contracts" describes shared cross-crate implementations. But three
constraints conflict:

1. AGENTS.md one-task-one-crate rule.
2. AGENTS.md direction: "edit crates may depend on contracts; contracts may NOT
   depend on rules/sim/net".
3. The enforced dependency graph in `tools/lint/deps.py` ALLOWED table.

The reviewer flagged: `crpg-contracts` cannot implement traits for `crpg-net`
under the current graph, because to do so it would need to depend on
`crpg-net`, which the direction forbids. The spec promise is therefore not
fulfillable as written without either (a) changing the ALLOWED table / rule, or
(b) moving contract implementations into a crate that can see both sides.

## Decision to make
- Option A: Relax the direction so `crpg-contracts` may dev-depend on consumer
  crates for the impls, and update `tools/lint/deps.py` ALLOWED.
- Option B: Treat "contracts" as only trait *definitions* (no cross-crate impls)
  and soften the spec wording.
- Option C: Rehome cross-crate impls into the consumer side (each crate
  implements the contracts traits for its own types).

## Deliverable
- A decision recorded in an ADR or `docs/architecture/`.
- If the graph changes, the AGENTS.md dependency-direction line and
  `tools/lint/deps.py` ALLOWED table updated together (they must stay in sync).

## Constraints
- Out-of-scope for the 2026-09-06 autonomous session; required human sign-off.
- Any `deps.py` ALLOWED change must also update its self-tests.

## Resolution — 2026-09-30 (T032, POST-T018 D01)

**Placement resolved: Option B.** `crpg-contracts` contains trait
*definitions* only, never cross-crate implementations. `Transport` stays local
to `crpg-net` permanently: it is defined in `crates/crpg-net/src/transport.rs`
and was built and conformance-tested there by T018 (PR #16) without touching
`crpg-contracts` or `crpg-testkit`. Higher consumers may define narrow
adapters. No net→contracts edge, no `crpg-contracts` edit and no `ALLOWED`
change is made or implied; `tools/lint/deps.py` already permits exactly this.

Spec wording reconciled in `docs/CRPG_ENGINE_SPEC.md` §14 (the "why"
paragraph), §15.3 (conformance ownership; `assert_transport` is an
illustrative sketch, not a live API) and §24 T18 (as-built affected crates),
each with a dated note. A roadmap sketch stays a sketch; it does not become a
live API by being in the spec.

Still open: the E017 residual items (the post-T018 evolution queue), which
this resolution does not close.

- 2026-09-30 (UTC) · claude-code + T032 E003 resolution · Recorded the D01 definitions-only placement with the live Transport location and the reconciled spec sections, leaving E017 residuals explicitly open.

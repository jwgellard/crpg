# Decision D32 — draft

Status: **Answered 2026-10-23: every question (a), the recommendation (the user, walking through each item; D32e–h: "Do whatever is best").** Non-blocking questions collected
by manager sessions under `docs/AGENT_TEAM_PLAN.md` §6, each with a
recommendation. Nothing here is decided until the user answers.

## D32a — approve the T136b exact contract?

**Answered 2026-10-23: (a).**

From `tasks/T136b.md` ("Status", "Exact contract"): the contract proposes ruleset/ability/effect tag-only bumps with encounter staying `/1`; typed and derived stats with health and attributes as `Int`; `bonus_stats`; defence `Constant(i32)`/`ActorStat`/`Opposed{dice, bonus_stats}`; damage dice added to a required `amount`; a ruleset-level initiative rule; effect `Stat` target and `grants` tags; and one new warning code (about 1,400 hand-written lines, two Sonnet sessions). Status: exact contract proposed — awaits the user's approval.

- **Recommend:** (a) approve as written, with D32b–D32h as recommended.
- Alternative: (b) do not approve; revise the contract.

## D32b — approve companion T136c in the same batch as T136b?

**Answered 2026-10-23: (a).**

From `tasks/T136b.md`: T136b cannot merge alone (exhaustive sim matches at `crpg-sim/src/combat.rs:1259` and `:1298`), so the contract names companion T136c (`crpg-sim`, fail-closed `UnsupportedAuthored` for the new enum variants, about 40 source and 220 test lines) plus two or three mechanical test lines in `crpg-net` and `crpg-cli` (T017b/T017d precedent), in the same batch as T136b.

- **Recommend:** (a) yes, approve companion T136c plus the mechanical test lines in the same batch as T136b.
- Alternative: (b) do not batch them; merge T136b without the companion.

## D32c — drop the three version bumps and extend the no-bump rule to additive enum variants?

**Answered 2026-10-23: (a).**

From `tasks/T136b.md`: all steps are tag-only, and the bumps cost about 25 test-pin edits, a golden rewrite, a CLI pin and stale-tagged srd-lite content; every bump-only item in the file is marked for deletion.

- **Recommend:** (a) yes, drop them and extend the approved no-bump rule to additive enum variants.
- Alternative: (b) keep the three version bumps.

## D32d — `crpg.encounter` stays `/1`?

**Answered 2026-10-23: (a).**

From `tasks/T136b.md`: the contract keeps `crpg.encounter` at `/1`, departing from the T136 frame table.

- **Recommend:** (a) yes, encounter stays `/1`.
- Alternative: (b) bump encounter as the T136 frame table said.

## D32e — defer effect sides to T137's contract as an optional field?

**Answered 2026-10-23: (a).**

From `tasks/T136b.md`: effect sides are deferred to T137's contract as an optional field.

- **Recommend:** (a) yes, defer effect sides to T137's contract as an optional field.
- Alternative: (b) decide effect sides in T136b.

## D32f — defer effect lifetime kinds to T139?

**Answered 2026-10-23: (a).**

From `tasks/T136b.md`: effect lifetime kinds are deferred to T139.

- **Recommend:** (a) yes, defer effect lifetime kinds to T139.
- Alternative: (b) decide effect lifetime kinds in T136b.

## D32g — defer tiered thresholds to T109 as an optional field mapped onto the kernel's outcome evaluation?

**Answered 2026-10-23: (a).**

From `tasks/T136b.md`: tiered thresholds are deferred to T109 as an optional field mapped onto the kernel's outcome evaluation.

- **Recommend:** (a) yes, defer tiered thresholds to T109 as an optional field mapped onto the kernel's outcome evaluation.
- Alternative: (b) decide tiered thresholds in T136b.

## D32h — defer modifier conditions to their first consumer?

**Answered 2026-10-23: (a).**

From `tasks/T136b.md`: modifier conditions are deferred to their first consumer.

- **Recommend:** (a) yes, defer modifier conditions to their first consumer.
- Alternative: (b) decide modifier conditions in T136b.

## D32i — approve the T138 exact contract?

**Answered 2026-10-23: (a).**

From `tasks/T138.md` ("Status", "Exact contract"): the contract proposes per-encounter mode on `CombatState`, one `Option<EncounterClock{round_ticks, elapsed}>`, `start_encounter_in_mode`, a free `request_mode`, `Stop` as a `CombatAction` variant, and history v2 lazy and sticky, with no existing golden moving; it splits into T138a (World/tick/RT policy) then T138b (history v2, tick facts, golden), one Sonnet session each. Status: exact contract proposed — awaits the user's approval.

- **Recommend:** (a) approve as written, with D32j–D32q as recommended.
- Alternative: (b) do not approve; revise the contract.

## D32j — approve the three companion packages in the same batch?

**Answered 2026-10-23: (a).**

From `tasks/T138.md`: the three companion packages go in the same batch — T138c (`crpg-script`, one or-pattern arm), T138d (`crpg-server`, a `Stop` arm; `candidate` returns `Option`, withholding the three new events until T140/T141), T138e (`crpg-net` tests only, three `panic!` arms) — three single-crate packages rather than one multi-crate one.

- **Recommend:** (a) yes, approve the three companion packages in the same batch.
- Alternative: (b) combine them into one multi-crate package instead.

## D32k — approve the new goldens and the two mechanical test edits?

**Answered 2026-10-23: (a).**

From `tasks/T138.md`: new per-target `crates/crpg-sim/tests/goldens/history_v2.<target>.txt`; `tests/legal_actions.rs:566–573` gains a `Stop` arm; `tests/history.rs:829–834` uses version 3 as the unknown version since 2 is now valid (no existing golden moves; neither edit weakens a test).

- **Recommend:** (a) yes (no existing golden moves; neither edit weakens a test).
- Alternative: (b) do not approve the new goldens or the test edits.

## D32l — `OrderEnded` covers only an invalid order cleared when due?

**Answered 2026-10-23: (a).**

From `tasks/T138.md`: `OrderEnded` covers only an invalid order cleared when due (the ADR's literal text).

- **Recommend:** (a) yes, `OrderEnded` covers only an invalid order cleared when due.
- Alternative: (b) give `OrderEnded` a broader reading than the ADR's literal text.

## D32m — an order unaffordable when due is cleared?

**Answered 2026-10-23: (a).**

From `tasks/T138.md`: an order unaffordable when due is cleared, as the ADR says.

- **Recommend:** (a) yes, an order unaffordable when due is cleared.
- Alternative: (b) keep an order that is unaffordable when due.

## D32n — setting an order checks affordability?

**Answered 2026-10-23: (a).**

From `tasks/T138.md`: setting an order checks affordability, so AI does not churn orders.

- **Recommend:** (a) yes, setting an order checks affordability.
- Alternative: (b) do not check affordability when setting an order.

## D32o — defer the RT replay golden in `crpg-testkit`?

**Answered 2026-10-23: (a).**

From `tasks/T138.md`: an RT replay golden in `crpg-testkit` is deferred.

- **Recommend:** (a) defer the RT replay golden in `crpg-testkit`.
- Alternative: (b) add the RT replay golden in `crpg-testkit` now.

## D32p — the RT terminal error reuses `OutOfTurn`?

**Answered 2026-10-23: (a).**

From `tasks/T138.md`: the RT terminal error reuses `OutOfTurn`.

- **Recommend:** (a) yes, the RT terminal error reuses `OutOfTurn`.
- Alternative: (b) add a new terminal error variant instead.

## D32q — the `serialize_struct` length stays unpinned by a test?

**Answered 2026-10-23: (a).**

From `tasks/T135.md` ("Open questions", Q1), carried from session 14: the `serialize_struct` length (4 without refill, 5 with) stays unpinned by a test, because a recording `Serializer` needs float method names in a `crpg-rules` test.

- **Recommend:** (a) accept as unobservable (exact bytes pin H1/H2); revisit if a length-sensitive format is adopted.
- Alternative: (b) pin the length with a test.

## Agent log

- 2026-10-23 (UTC) · opencode-go/muse-spark-1.3-contributor (grunt) + S15 bookkeeping · Started the D32 draft with the T136b/T138 contract approvals and their fifteen open questions, so the user can answer them in one batch.
- 2026-10-23 (UTC) · claude-code (manager, Opus 5.5) + D32 answered · Recorded the user's answers: every recommendation (a), so the T136b and T138 contracts are approved.

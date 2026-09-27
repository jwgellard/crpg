# ADR-0015: `crpg-sim` compatibility support boundary for B1 shapes

Date: 2026-09-27
Status: **Accepted** (final approval granted in-session 2026-09-27 for the
revised proposal: single variant, per-field Display with canonical ULIDs,
pinned validation and field order, first-only support; implementation
authorized in `tasks/T017b.md`, B3 generalization remains separate).

## Context

B1 (`tasks/T017a.md`) replaces authored `Ruleset.action_pool` with
`Ruleset.pools` and extends `Ability` with `extra_costs`, `ends_turn`,
`effect`, `defense`, and `natural_die`, with `/1 → /2` migration edges.
T017b (`tasks/T017b.md`) adapts `crpg-sim` to read `pools.first()` for the
existing single-pool runtime, preserving legacy behavior without
generalizing. The old runtime cannot honor plural pools, extra costs,
non-ending turns, effects, target-stat defense, or natural selection:
using only `pools[0]` for a 2-pool ruleset, spending only primary cost,
always ending the turn, ignoring effects, resolving against the actor
attribute instead of the target stat, or passing `natural_die: None` would
execute srd-lite content incorrectly and silently. No existing
`CombatError` accurately covers these shapes (see below); repurposing an
unrelated variant would mislead consumers. The crate contract requires an
ADR for new public API; this is the narrow draft, separate from B3's full
generalization ADR.

## Decision (revised proposal, awaiting final approval)

1. **Single new variant (retained).** Add
   `CombatError::UnsupportedAuthored { what: String }` where `what` is a
   stable `"<kind> <ULID> <field>"` triple with canonical uppercase ULID
   text (via `Ulid::to_string()`), e.g., `"ruleset <ULID> pools"`,
   `"ability <ULID> extra_costs"`, `"ability <ULID> ends_turn"`,
   `"ability <ULID> effect"`, `"ability <ULID> defense"`, `"ability <ULID>
   natural_die"`. One variant keeps the temporary boundary easy to narrow
   field-by-field in B3; a ruleset/ability pair would double the surface
   for no lasting benefit.
2. **Display identifies owner and field.** `Display` is
   `"UnsupportedAuthored at spec/<field>: <what>"` where `<field>` is the
   stable per-site location (`pools`, `extra_costs`, `ends_turn`,
   `effect`, `defense`, `natural_die`, matching the existing `spec/*`
   lowercase conventions) and `<what>` carries the canonical ULIDs above.
   CLI users thus see both the meaningful location and the owner/field
   (the prior constant `"at spec/support"` hid `what` and is withdrawn).
   Existing variants keep their spellings, locations, and precedence
   byte-identical.
3. **Validation order (pinned, no mutation before failure).**
   `start_encounter` runs, in order: existing encounter/collection checks
   (`EncounterActive`, `TooManyParticipants`, `TooManyStats`,
   placement/area/creature resolution with `MissingPlacement`/`MixedArea`/
   `MissingCreature`); per-participant stat presence/wholeness/health
   (`MissingStat`/`InvalidStatValue`/`ValueOverflow`); pool cardinality
   (`pools.len() != 1` → new variant, before any `pools.first()`
   dereference, so empty never panics); the complete existing
   per-ability loop over **all** listed abilities in authored order
   (attribute `MissingStat`, dice `InvalidDice`, table
   `InvalidOutcomeTable`, primary-cost `InvalidCost` per ability); then
   the first-ability (encounter-ability) support checks in pinned
   field order — `extra_costs`, `ends_turn`, `effect`, `defense`,
   `natural_die` — before publication. Existing errors thus win over
   unsupported in every multifault combination, including an unsupported
   first ability versus invalid dice/table/cost in a later listed
   ability (the later `InvalidDice`/`InvalidOutcomeTable`/`InvalidCost`
   reports, since the existing loop completes first). Non-first listed
   abilities retain existing validation only and are never support-checked;
   they cannot be executed (using a non-first id in `perform_action`
   fails the existing `UnknownAbility` without mutation).
   - All checks run before any entity spawn, RNG stream creation, or event
     enqueue; a rejected spec leaves World/RNG/events unchanged (existing
     all-or-nothing guarantee, extended with new regressions).
4. **Temporary scope with per-field retirement.** The check covers exactly
   the six sites above. Each restriction is removed or replaced
   individually only after B3 lands corresponding execution and validation
   coverage for that shape (e.g., plural-pool spending lifts `pools`;
   effect execution lifts `effect`); the `UnsupportedAuthored` variant
   itself is retired only by B3's reviewed API decision when no
   restriction remains (or replaced with broader support). No blanket
   deletion is granted here.

## Alternatives considered

- Reusing `InvalidCost`/`MissingStat`/`InvalidOutcomeTable` for new
  shapes: rejected as inaccurate and misleading per the crate's error
  contract; the maintainer explicitly forbids repurposing.
- `UnsupportedRuleset`/`UnsupportedAbility` pair with per-site locations:
  rejected as double surface for a temporary gate; single variant with
  `what` naming kind+ULID+field preserves debuggability with one spelling.
- Requiring all listed abilities (not just first) to be legacy-shaped:
  rejected per maintainer direction; only the first-listed executable
  ability is support-checked, non-first abilities keep existing validation
  and stay non-executable via `UnknownAbility`. Boundary regressions pin
  both: new-field non-first abilities do not trigger unsupported, and
  using them fails `UnknownAbility` without mutation.

## Consequences

- `CombatError` gains one variant (public API change, hence this ADR);
  `Display` gains six `spec/<field>` spellings carrying canonical ULIDs;
  precedence gains one pinned block (pool cardinality, then complete
  existing ability loop, then first-ability support checks in
  `extra_costs, ends_turn, effect, defense, natural_die` order). No other
  public names, shapes, locations, or behaviors change.
- New regressions pin pre-mutation failure for each of the six sites plus
  empty pools, legacy-subset success, the pinned field order, and the
  multifault cases (unsupported first vs invalid later-ability dice/table/
  cost → existing error wins; non-first new fields → no unsupported,
  execution → `UnknownAbility`); existing goldens stay byte-identical.
- Each restriction lifts individually with B3 execution+validation
  coverage; variant retirement belongs to B3's reviewed API decision.
  This ADR does not grant B3's architecture.

## Approval requested (revised)

Maintainer: grant final approval of variant name/fields, per-field
`spec/<field>: <what>` Display with canonical ULIDs, the pinned
encounter→stats→pools→all-abilities→first-ability-support→publish order
with the `extra_costs, ends_turn, effect, defense, natural_die` field
order, and the first-only support rule with non-first `UnknownAbility`
non-executability. Primary-pool compat (no new API) is already implemented
and green; boundary regressions wait on final approval. Do not mark
Accepted here.

Approval granted in-session 2026-09-27 ("Approve ADR + implement
boundary"): implement exactly the revised proposal above in `tasks/T017b.md`
scope; do not expand into B3 generalization; variant retirement stays with
B3's reviewed API decision.

## Agent log

- 2026-09-27 (UTC) · opencode/muse-spark + T017b ADR draft · Proposed the single-variant support boundary with constant location, pending approval; no implementation of the boundary claimed.
- 2026-09-27 (UTC) · opencode/muse-spark + ADR-0015 revision · Pinned the full validation order with pool cardinality before dereference and the new-field order, made Display carry per-field location plus canonical ULIDs, replaced blanket B3 deletion with per-field retirement owned by B3's API decision, and pinned first-only support with non-first UnknownAbility coverage; still Proposed, awaiting final approval with no boundary implementation.
- 2026-09-27 (UTC) · opencode/muse-spark + ADR-0015 acceptance · Recorded the maintainer's in-session final approval of the revised proposal (single variant, per-field Display, pinned orders, first-only rule) and authorized boundary implementation in T017b scope; no B3 architecture granted.

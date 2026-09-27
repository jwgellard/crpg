# ADR-0016: `crpg-sim` multi-ability/pool/effect/turn generalization

Date: 2026-09-27
Status: **Accepted** (maintainer approval granted 2026-09-27 — “Accept” —
for the readiness revision: exact shapes, `Display` strings, Phase A–E
precedence, `EndTurn`/effect semantics, persistence/single-source, and
deletion verification. Implementation may proceed in `tasks/T017d.md`
scope; B4/B5 remain separate).

## Context

B1 (`tasks/T017a.md`) landed the data vocabulary (plural pools,
per-pool costs, defense selector, natural selection, effect family) with
migrations; T017b (`tasks/T017b.md`, ADR-0015) keeps the old
single-pool/single-ability runtime working by rejecting new execution
requirements before mutation. Work package B3 (`tasks/T017d.md`) must
execute those shapes through the production sim path with no
package-name branches while preserving minimal-d6 behavior
byte-identically (same RNG draws in the same order, same hashes;
`combat_basic` artifacts, goldens, and suites are read-only). The crate
contract requires an ADR for public-API changes; this is that record,
separate from B4's replay/goldens and B5's CLI proof.

## Decision (accepted — readiness revision approved)

All public items have rustdoc. Existing names keep their meanings unless
re-specified below. This revision pins exact shapes, `Display` strings,
validation order, persistence predicates, and retirement verification so
an implementer following this ADR alone (plus `tasks/T017d.md` tests)
cannot diverge. Where `tasks/T017d.md` uses shorthand, this ADR is the
authoritative pin; a one-line `T017d` alignment is noted in Approval.

1. **Per-ability definitions addressed by ULID (exact shapes).**

```rust
pub struct AbilityDefinition {
    pub ability: Ulid,
    pub dice: DiceExpr,
    pub attribute: String,
    pub defense: AbilityDefense,
    pub outcome_table: OutcomeTable,
    pub damage: Vec<(Outcome, u32)>,
    pub costs: Vec<(ResourcePoolId, u32)>, // template order; primary first
    pub ends_turn: bool,
    pub effect: Option<Ulid>,
    pub natural_die: Option<u32>,
    pub requires_target: bool,
    pub allow_self_target: bool,
}

pub enum AbilityDefense { ActorAttribute, TargetStat { stat: String } }

pub enum EffectAim { Slf, Target }

pub enum EffectTarget { Roll, Dc }

pub enum EffectOp { Add, Set }

pub struct EffectModifier {
    pub id: Ulid,
    pub target: EffectTarget,
    pub op: EffectOp,
    pub value: i32,
    pub priority: i16,
    pub name: Option<String>,
}

pub struct EffectDefinition {
    pub effect: Ulid,
    pub aim: EffectAim,
    pub mod_type: String,
    pub policy: StackingPolicy,
    pub modifiers: Vec<EffectModifier>,
    pub duration_rounds: u32,
}

pub struct PoolTemplate {
    pub id: Ulid,
    pub max: u32,
    pub refresh: RefreshTrigger,
}

pub struct CombatDefinition {
    // ... encounter/ruleset/health_stat/attribute_names as today ...
    pub abilities: Vec<AbilityDefinition>, // ascending ability ULID
    pub pools: Vec<PoolTemplate>,          // authored template order
    pub effects: Vec<EffectDefinition>,    // ascending effect ULID
    // ... legacy single-ability/pool fields retained populated from the
    // authored first-listed entry (`ruleset.abilities[0]`, `pools[0]`)
    // for hash-compat; execution uses the new vectors when present
    // (see §5 single-source rule) ...
}

pub struct Combatant {
    // ... placement/initiative/attributes/health/max_health/dead ...
    pub action_pool: ResourcePool,       // primary pool (unchanged name/bytes)
    pub extra_pools: Vec<ResourcePool>, // non-primary, template order; skipped when empty
    pub attached: Vec<AttachedEffect>,  // ascending effect ULID; skipped when empty
}

pub struct AttachedEffect { pub effect: Ulid, pub expires_round: u32 }

pub enum CombatAction {
    UseAbility { actor: EntityId, ability: Ulid, target: EntityId },
    EndTurn { actor: EntityId },
}
```

Ordering (determinism, no hash-order anywhere): `abilities` ascending
ability ULID, `pools` authored template order, `effects` ascending effect
ULID, `extra_pools` template order excluding primary, `attached`
ascending effect ULID (re-application updates `expires_round` in place,
order stays sorted). At most one `attached` entry per effect per
combatant. Lookups scan these vectors; sim adds no new `MAX_*` and no
`HashMap`/`HashSet` (counts are small). No new dependencies; existing
`sim → data` / `sim → rules` edges cover all new consumption. Data enums
are never runtime types beyond the existing authored-document borrows;
`PolicyWire`→`StackingPolicy`, `EffectTargetWire`→`Roll`/`Dc`,
`EffectOpWire`→`Add(Int)`/`Set(Int)` map explicitly.

`EncounterSpec` gains `effects: &BTreeMap<Ulid, &AuthoredEffect>`
(caller supplies every referenced effect). Missing spec-map entries
return typed errors, never panic: missing ability → `UnknownAbility`
(sole `start_encounter` error with a `combat/*` location, naming the
ability-set domain shared by adapter and controller), missing table →
`InvalidOutcomeTable`, missing effect → `MissingEffect`. Empty
participants / empty ruleset abilities remain panics (data guarantees
nonempty, as today). `UnknownAbility` otherwise means an action names an
ability ULID not in `definition.abilities` (action-time set-membership).

2. **Plural runtime pools, unchanged primary name.** Costs are atomic:
check every pool before spending any; first insufficient pool in
template order reports `InsufficientAction { pool, cost, current }`
where `cost`/`current` are that pool's cost/balance. `Display` stays
`"InsufficientAction at combat/pool"` byte-identical per `tasks/T017.md`
(parent constraint); the pool identity is programmatic (tests assert
struct fields, not `Display`, for pool identity). Accepted-but-failed
attacks spend their costs (success or failure, if accepted).
Malformed/rejected actions spend neither resources nor randomness
(pre-draw validation; staged-clone transactional RNG as in T015).
`ResourcePool::{new,spend,refresh,can_afford}` semantics unchanged; pools
own no timeline.

3. **Defense, naturals, turns, effects (exact semantics).**

- `AbilityDefense`/`EffectAim`/modifier mirrors map explicitly per §1.
  `SourceRef` for rebuilt modifiers is `{kind: "effect", id: effect ULID}`;
  `modifier.id` is the wire modifier ULID (unique per effect; cross-effect
  uniqueness enforced as `DuplicateModifier`, so each side's fold keeps
  unique ids with at most one entry per effect).
- Aim selects the holder: `Slf` attaches to the actor, `Target` attaches
  to the target (when actor equals target, one entry). Resolution reads
  the actor's unexpired attached entries for the Roll fold and the
  target's unexpired entries for the Dc fold (`round < expires_round`),
  filtering by `modifier.target` (`Roll` vs `Dc`). Base blocks stay empty
  as today, extended with these two sets under the same
  `COMBAT_ROLL_TAG`. Target guard uses a transient `StatBlock` for
  `TargetStat`; `Against::Stat{entity: target, stat, modifier_target}` vs
  `Against::Dc`. Margin `i64(actor) - i64(against)`; table decision;
  per-outcome damage (unmapped → 0).
- `natural_die` indexes raw draws including dropped dice (per
  `DiceExpr::count()` / `RollRequest` “even when dropped”); `None` means
  no rule; `Some(i)` with `i >= count` → `InvalidNaturalDie`.
- `ends_turn: false` accepted attacks (success or failure — clarifying
  `tasks/T017d.md` “success” shorthand) skip the pop/refresh/rollover:
  active stays, no refresh, no RNG/event beyond
  resolution+spend+effect+damage/`Died`. Dead participants are still
  removed from scheduling; `<2` alive still goes terminal (`active=None`)
  regardless of `ends_turn`. Otherwise `ends_turn: true` advances per the
  existing pop/`TurnStart`/rollover/terminal rule.
- `EndTurn{actor}` validates turn-holding in existing order
  (`NoEncounter`, `NotParticipant(actor)`, `AbsentActor`, `DeadActor`,
  `OutOfTurn`; no ability/target/cost checks) and advances through the
  shared pop/`TurnStart`/rollover/terminal rule with no spend, no draw,
  no damage, no effect (RNG streams identical pre/post; no events).
- Effects attach only on success with `effect: Some`: attach
  `(effect, round + duration_rounds)`, refreshing any existing entry for
  that effect. Failure attaches nothing even with `effect: Some`. Round
  rollover additionally drops attached with `expires_round <= new_round`
  (silent: state and hash show it; no event). Op/policy compatibility is
  adapter-checked as `InvalidEffect`: `Set` under
  `HighestBonusWorstPenalty` and nameless modifiers under
  `HighestPriorityPerName` fail at adapter (kernel would fail them at
  query as `InvalidOperation`; adapter pre-empts to avoid runtime
  `expect`).
- `perform_action` returns `Result<Option<ActionOutcome>>` (`Some` attack
  / `None` `EndTurn`; existing tests unwrap mechanically).

4. **New error variants (exact `Display` strings and pinned order).**

```text
MissingEffect at spec/effects        (MissingEffect { effect } — fields hidden)
MissingPool at spec/pools            (MissingPool { pool })
InvalidEffect at spec/effects        (InvalidEffect { effect })
DuplicateModifier at spec/effects    (DuplicateModifier { id })
InvalidNaturalDie at spec/ability    (InvalidNaturalDie { index })
PolicyConflict at spec/effects       (PolicyConflict { mod_type })
InsufficientAction at combat/pool    (gains pool; Display unchanged)
UnknownAbility at combat/ability     (unchanged; now also adapter missing-map case)
```

All new variants hide fields in `Display` (Code + location only),
matching existing `MissingStat`/`InvalidCost` style; `UnsupportedAuthored`
remains the only field-interpolating `Display` until retirement.
Existing variants keep spellings, locations, and meanings except as
re-specified; no repurposed errors (unknown pool is never `InvalidCost`
or `MissingStat`; unknown policy strings remain data `Malformed`, never
sim errors).

Pinned `start_encounter` order (all-or-nothing, no mutation before
failure, including before RNG stream creation):

- Phase A (existing, preserved order): `EncounterActive`,
  `TooManyParticipants`, `TooManyStats`, per-participant `MissingPlacement`
  (authored order), `MixedArea`, per-participant/per-stat
  `MissingCreature` then presence/wholeness/health (`MissingStat`,
  `InvalidStatValue`, `ValueOverflow`) in authored stat order.
- Phase B (pool cardinality): empty pools → panic (data guarantees
  nonempty, like empty participants; replaces the retiring
  `UnsupportedAuthored` empty-pools typed case); plural valid.
- Phase C (per-ability loop, authored `ruleset.abilities` order; first
  failure wins across abilities): for each ability, in field order —
  spec-map contains ability else `UnknownAbility`; attribute ∈ attributes
  else `MissingStat` (Display `spec/stats`, unchanged); dice parse else
  `InvalidDice`; table present-and-buildable else `InvalidOutcomeTable`;
  each cost in template order — pool declared else `MissingPool`, amount
  within template max else `InvalidCost` (empty `costs` → `InvalidCost` at
  `spec/cost`); `TargetStat` names a declared non-health stat else
  `MissingStat` (same `spec/stats` Display; attribute checked first so
  multifault reports attribute); effect `Some` resolves into
  `spec.effects` else `MissingEffect`; `natural_die` bound else
  `InvalidNaturalDie`. Existing checks thus win over new checks in the
  same ability when earlier in this field order.
- Phase D (global effect checks, ascending effect ULID; per-ability
  errors win over global): each effect `InvalidEffect` (duration ≥ 1,
  modifiers nonempty, `mod_type` nonempty, op/policy compatibility);
  then cross-effect `DuplicateModifier` (ascending duplicate id wins);
  then `PolicyConflict` (lexicographically smallest conflicting
  `mod_type` wins).
- Phase E: publication (entities authored order, every pool at max,
  timeline at initiatives, `round = 0`, active = head, roll tag interned,
  incoming-actor `TurnStart` refresh). ADR-0015 `UnsupportedAuthored`
  gate is removed field-by-field as each shape gains execution here.

`EndTurn` validation order is the existing-action prefix without
ability/target/cost: `NoEncounter`, `NotParticipant`, `AbsentActor`,
`DeadActor`, `OutOfTurn`.

5. **Absence-skip hash preservation (exact predicates).** Legacy
single-ability/pool fields (`ability`, `dice`, `attribute`, `damage`,
`cost`, `requires_target`, `allow_self_target`, `outcome_table`,
`pool_id`, `pool_max`, `pool_refresh`, plus `encounter`/`ruleset`/
`health_stat`/`attribute_names`) are always persisted, populated from
the authored first-listed entry (`ruleset.abilities[0]`, `pools[0]`).
New definition vectors (`abilities`, `pools`, `effects`) share one skip
predicate: skipped when legacy-equivalent (single ability AND single
pool AND no effects), persisted together otherwise. Minimal-d6 worlds
(`1,1,0`) therefore serialize byte-identically to T016-era bytes; any
non-legacy shape persists all three vectors. `Combatant.extra_pools`
skipped when empty, `attached` skipped when empty (independent
per-combatant predicates). Execution uses new vectors when present,
else the legacy-derived single-entry view; when new vectors are present,
legacy copies are agreement-checked at load but never read for behavior
(single source per world kind, no double truth).

World load coherence extends `T016e`-style (string messages, reviewed at
implementation; invalid saves fail before publishing, no partial
`World`): per-pool id/max/refresh agreement by index; costs drawn from
declared pools; effect refs resolving with sane durations; attached
pairs resolving with future expiries (`expires_round > round`);
multi-ability coherence (legacy copies equal first-listed entries;
vectors ordered ascending-ULID/template as in §1 — out-of-order
rejects, never sorts; combatant `extra_pools` length `pools.len()-1`
with id/max/refresh agreement by index; `attached` at most one per
effect); combatant balances/attributes/health/dead/placement and
active/timeline as before. Load-time empty pools rejects (corrupt save);
adapter-time empty pools panics (caller bug, data-guaranteed).

6. **Retirement (deletion, not deprecation).** Each ADR-0015 restriction
lifts individually as its shape gains execution+validation here;
`UnsupportedAuthored` is deleted (breaking removal, not dead variant)
when no restriction remains. Deletion is verified by: no
`UnsupportedAuthored` construction in `src/` (grep), all six sites
execution-covered by `combat_multi` tests, existing suites green via
mechanical updates only, legacy `combat_basic` goldens byte-identical
per target, recorded in the B3 completion record. This ADR authorizes
that deletion as part of the change.

7. **Determinism, neutrality, bounds (pinned).** Same
seed/content/inputs repeat every hash (exact-build scope per ADR-0012;
never cross-target equality); mutating faces/ward/damage/pools/effect
numbers changes behavior through content; no package-name branches —
neutral scan extends to every new/changed file with T017 vocabulary
added (`srd-lite`, `ward`, `focus`, `heavy`, stacking-policy words).
No new `MAX_*`; effect modifiers ≤ 4096 enforced by the kernel at query;
scan over ascending-ULID/template vectors only.

## Alternatives considered

- Versioned persistence envelope with migration + re-baselined goldens:
  rejected; goldens are read-only and live-hash bytes must not move for
  legacy content.
- Separate per-effect pipelines instead of one merged pipeline:
  rejected; phase/priority/ID folds must stay global per T014.
- `EndTurn` as a separate function instead of a `CombatAction` variant:
  rejected; one authoritative action funnel keeps precedence and
  transactional guarantees in one place. `CombatAction::EndTurn{actor}`
  with `perform_action` returning `Result<Option<ActionOutcome>>`
  (`Some` attack / `None` `EndTurn`) is the pinned shape.

## Consequences

- Public API changes as in §1–§4 (exact shapes above, `EndTurn`,
  `Option` return, `effects` map, six new variants, one extended
  variant); `Display` strings pinned in §4; Phase A–E precedence pinned
  in §4; neutrality extends to every new/changed file with T017
  vocabulary added to the banned list; `combat_multi` suite added per
  `tasks/T017d.md` tests (modified roll vs target defense with
  breakdowns, two abilities/pools with triggers, turn-staying +
  `EndTurn` + spend-nothing rejections, effect lifetime across a midway
  save, natural invocation, generic path, rollback, hash
  repeat/sensitivity, exactly-one-dead, boundaries); existing suites
  pass via mechanical updates only (`InsufficientAction` pool identity,
  `Option` unwraps, no weakened assertions, no changed goldens). B4 owns
  replay/goldens; B5 owns CLI proof. No new dependencies, no new `MAX_*`,
  no `HashMap`/`HashSet`, no `f64`, no `unsafe`, no OS branches.

## Approval requested

Maintainer: approve exact shapes/serde (§1, §5 single-source +
skip-when-legacy-equivalent), error set with exact `Display` strings
and Phase A–E precedence including `EndTurn` order (§4),
absence-skip persistence with load-coherence checks (§5),
`UnsupportedAuthored` deletion with grep + six-site + goldens
verification (§6), determinism/neutrality/bounds/lookup (§7), and the
`combat_multi` test plan per Consequences (or request changes) before
`tasks/T017d.md` implementation. `tasks/T017d.md` “success skips” reads
as accepted-attack skips per §3, and missing spec-map `UnknownAbility`
is the documented sole `combat/*`-located adapter error per §1/§4; align
`T017d` one line on implementation if required. Do not mark Accepted here.

Approval granted 2026-09-27 (maintainer “Accept”): the exact shapes/serde,
error set with `Display` strings and Phase A–E precedence, persistence
with coherence checks and single-source, deletion verification, and test
plan above are approved for `tasks/T017d.md` implementation. The “Do not
mark Accepted here” constraint above is lifted by this approval.

## Agent log

- 2026-09-27 (UTC) · opencode/muse-spark + ADR-0016 draft · Proposed the multi-ability/pool/effect/turn API with absence-skip hash preservation and per-field retirement of the ADR-0015 gate; pending approval with no implementation claimed.
- 2026-09-27 (UTC) · opencode/muse-spark + ADR-0016 readiness revision · Pinned exact shapes, Display strings, Phase A–E precedence, EndTurn and effect/attach semantics, persistence predicates with single-source, and deletion verification to make the draft implementable without guessing; still Proposed, awaiting maintainer approval with no implementation claimed.
- 2026-09-27 (UTC) · opencode/muse-spark + ADR-0016 acceptance · Recorded the maintainer's “Accept” approval of the readiness revision with Status Accepted and the prior do-not-mark constraint lifted; no implementation started and no commit/push/PR created.

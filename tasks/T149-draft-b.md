# T149 draft b — range check (blind draft, unapproved)

## Task
T149 (split: T149a data, then T149b sim) implements ADR-0036 §X (X1–X3) on top
of the approved T036 contract: an ability may declare an optional `range`
(X1); `validate_action` rejects an out-of-range target after target liveness
using the movement `Measure` and T036 positions (X2); a due real-time order
whose target is out of range neither executes nor ends and stays due (X3).
Nothing in this draft is approved; a Claude architect adjudicates it against
an independent draft and the user approves the final contract.

## Crate(s)
Split: one task = one crate. T149a owns the data shape; T149b owns the sim
checks. T149b depends on T149a and on the approved T036 contract.

### T149a (`crpg-data`) owns
- `crates/crpg-data/src/combat.rs` — `Ability.range` field + read-time check.
- `crates/crpg-data/src/validation.rs` — `walk_combat` semantic check.
- `crates/crpg-data/src/schema.rs` — regenerated schema entry.
- `schemas/ability.schema.json` — regenerated (no `"range"` property today).
- `crates/crpg-data/tests/ability_range.rs` (new) — contract tests.
- `crates/crpg-data/AGENTS.md`, `docs/architecture/crpg-data.md` — doc touch-ups.

### T149b (`crpg-sim`) owns
- `crates/crpg-sim/src/combat.rs` — `AbilityDefinition.range`, legacy
  decision, `CombatError::OutOfRange`, `validate_action` check, X3 branch.
- `crates/crpg-sim/tests/action_range.rs` (new) — contract tests.
- `crates/crpg-sim/AGENTS.md`, `docs/architecture/crpg-sim.md` — doc touch-ups.

### goldens:
No existing golden moves. `crates/crpg-sim/tests/goldens/` keeps
`history_v1`, `history_v2`, `surface_v1` byte-identical and
`crates/crpg-testkit/goldens/` keeps `combat_basic`, `replay_basic`,
`srd_lite` byte-identical, because: no shipped ruleset or fixture declares
`range`, `reach` or `movement` (recursive grep over `rulesets/` and
`campaigns/fixtures/` is clean); X1 absent-means-unlimited keeps existing
data unchanged (`docs/adr/0036-movement-and-navigation.md:468-469`); and the
new fields skip serialization when absent/`None`, as the movement fields do
(`tasks/T036.md:326-328`). T036's expectation that an absent field keeps
existing goldens byte-identical (`tasks/T036.md:109-115`) applies unchanged.

## Readings of the ADR
1. **R1 — shape: `Option<Fx16_16>`, raw-`i32` wire, additive, no version
   bump.** X1's "`range: Fx16_16`" with absent-means-unlimited
   (`docs/adr/0036-movement-and-navigation.md:468-469`) is an
   optional-missing field decoded by the `Ability` deserializer
   (`crates/crpg-data/src/combat.rs:583-634`), serialized
   `skip_serializing_if = "None"`, schematized as an `i32` surrogate like
   the other `Fx16_16` fields (`crates/crpg-data/src/combat.rs:100-104`,
   `crates/crpg-data/src/combat.rs:526-528`), and added to the
   optional-missing list under the current-tag additive rule
   (`crates/crpg-data/AGENTS.md:33-46`). Rejected: a `LengthWire`
   constant-or-stat form — X1 names `Fx16_16`, not a stat reference; and a
   `crpg.ability/2` → `/3` bump with migration — T148's approved "no version
   bump" reading (`tasks/T149-facts.md:181`) and the additive rule cover an
   optional-missing field, and X1 promises existing data is unchanged.
2. **R2 — validity: raw value `>= 1`; `0` and negatives are malformed.**
   Rejected: allowing raw `0` — a zero range admits only a zero-length
   pairing, but `seg_len` of two distinct points is never `0` and a
   self-pairing is already governed by `SelfTarget`
   (`crates/crpg-sim/src/combat.rs:1006`), so `0` would be a silently
   unusable ability; the bound mirrors the turn-step `raw(step) >= 1` rule
   checked by `validate_ruleset_movement`
   (`crates/crpg-data/src/combat.rs:1268-1320`).
3. **R3 — check position: last in the `UseAbility` path, after every
   existing check.** X2's "after target liveness"
   (`docs/adr/0036-movement-and-navigation.md:471-474`) fixes the earliest
   point (after `DeadTarget` at `crates/crpg-sim/src/combat.rs:3422-3424`);
   this draft pins the exact point at the end, after derived-stat evaluation
   (`crates/crpg-sim/src/combat.rs:3395-3500`), so `SelfTarget`, `OutOfTurn`,
   `UnknownAbility`, affordability and every existing variant keep their
   precedence (`CombatError` "check in declaration order",
   `crates/crpg-sim/src/combat.rs:956-960`; precedence pinned by
   `crates/crpg-sim/tests/legal_actions.rs:264` and `:337`). Rejected:
   immediately after `DeadTarget` — it would let `OutOfRange` beat
   `UnknownAbility`/`OutOfTurn`/affordability for ranged abilities and
   contradict the brief's "runs after every existing check" requirement.
4. **R4 — either side without a mover is `OutOfRange` (literal X2).**
   Mover-free worlds therefore reject every ranged `UseAbility`, including
   adjacent targets. Rejected: treating no-mover as unlimited — it
   contradicts X2's explicit sentence
   (`docs/adr/0036-movement-and-navigation.md:471-474`).
5. **R5 — distance is `route_measure(measure, [actor, target])` over
   authoritative mover positions; horizontal; `None` is `OutOfRange`.**
   The measure is the encounter combat definition's movement measure (S2:
   `route_measure(measure, points)` is the only rule-visible seam,
   `docs/adr/0036-movement-and-navigation.md:317-325`); positions are
   `Mover::position()` (`tasks/T036.md:245-260`), never the `f32` mirror
   (`tasks/T036.md:188-191`); `y` is ignored because `seg_len` is horizontal
   (`tasks/T036.md:296-297`); a `None` measure (fewer than two points or an
   out-of-bounds component, `tasks/T036.md:449-453`) cannot show
   at-most-`range`, so it is `OutOfRange`. Rejected: calling `seg_len`
   directly (S2 forbids rule-visible callers,
   `docs/adr/0036-movement-and-navigation.md:324-325`) and 3D length
   (rejected tabletop-reasons alternative,
   `docs/adr/0036-movement-and-navigation.md:526`).
6. **R6 — X3 changes only the `OutOfRange` branch of `execution_stage`.**
   Today any validation failure clears the order and reports `OrderEnded`
   (`crates/crpg-sim/src/combat.rs:4410-4417`); after X3, an `OutOfRange`
   failure keeps `order` unchanged, pushes no `OrderEnded`, and leaves the
   timeline entry due so the next stage retries it. Every other failure
   still clears and ends. Rejected: re-keying the entry or emitting a new
   event — X3 says "neither executes nor ends; it stays due"
   (`docs/adr/0036-movement-and-navigation.md:476-477`) and amends ADR-0029
   T7 for this case only.
7. **R7 — `legal_actions` needs no new logic.** Candidates are already
   validated one by one (`crates/crpg-sim/src/combat.rs:3604`), so omission
   of out-of-range targets
   (`docs/adr/0036-movement-and-navigation.md:471-474`) falls out of the
   `validate_action` check. Rejected: a separate pre-filter — two seams
   could disagree; the candidate loop is the single seam.

## Interface
### T149a — `crates/crpg-data/src/combat.rs`
```rust
// On `Ability` (deny_unknown_fields struct, `crates/crpg-data/src/combat.rs:583-586`),
// beside the other optional-missing selection fields (`crates/crpg-data/src/combat.rs:613-633`):
#[serde(default, skip_serializing_if = "Option::is_none", schemars(with = "Option<i32>"))]
pub range: Option<Fx16_16>,
```
- Wire: raw signed 32-bit integer metres×65536 (`Fx16_16` transparent serde,
  `crates/crpg-core/src/fixed.rs:22-36`); `None` absent on the wire.
- Schema: regenerated `Ability` entry (`crates/crpg-data/src/schema.rs:117`);
  `crpg.ability` stays at version 2, `crpg.ruleset` stays at 2
  (`crates/crpg-data/src/migrations/mod.rs:78`,
  `crates/crpg-data/src/migrations/mod.rs:94`); no migration edge.
- Read-time (`validate_combat_local` Ability arm,
  `crates/crpg-data/src/combat.rs:1563-1636`): `Some(r)` with
  `raw(r) < 1` is malformed (see Errors).
- Semantic (`walk_combat` per-ability checks,
  `crates/crpg-data/src/validation.rs:625`): same bound re-checked on the
  decoded value.

### T149b — `crates/crpg-sim/src/combat.rs`
```rust
// On `AbilityDefinition` (persisted sim form, `crates/crpg-sim/src/combat.rs:412-455`),
// beside the other optional serde fields (`crates/crpg-sim/src/combat.rs:439-454`):
#[serde(default, skip_serializing_if = "Option::is_none")]
pub range: Option<Fx16_16>,
```
- Conversion copies `Ability.range` verbatim; `ability_is_legacy`
  (`crates/crpg-sim/src/combat.rs:510-521`) gains a range condition:
  `range.is_some()` is non-legacy (legacy shape keeps the 15 fields,
  `crates/crpg-sim/src/combat.rs:617-660`).
- Check placement: end of the `UseAbility` path in
  `pub fn validate_action(world: &World, action: &CombatAction) -> Result<(), CombatError>`
  (`crates/crpg-sim/src/combat.rs:3332`), after derived-stat evaluation
  (`crates/crpg-sim/src/combat.rs:3395-3500`), still side-effect-free
  (`crates/crpg-sim/src/combat.rs:3322-3324`):
```rust
// After every existing UseAbility check; ability is the resolved definition:
if let Some(range) = ability.range {
    let ok = match (world.movers().get(actor), world.movers().get(target)) {
        (Some(a), Some(t)) => route_measure(measure, &[a.position(), t.position()])
            .is_some_and(|d| d <= range.raw() as u64),
        _ => false,
    };
    if !ok { return Err(CombatError::OutOfRange); }
}
```
  where `measure` is the encounter combat definition's movement measure
  (default `Measure::Euclidean` when the definition carries none), `actor`
  and `target` are the validated participants, and `range.raw()` uses the
  accessor (`crates/crpg-core/src/fixed.rs:52-59`). Exact names
  (`movers()`, `route_measure`, `Measure`) per `tasks/T036.md:302-303`,
  `tasks/T036.md:326-328`, `tasks/T036.md:236-243`.
- X3 (`execution_stage`, `crates/crpg-sim/src/combat.rs:4375-4458`): the
  failure branch (`crates/crpg-sim/src/combat.rs:4410-4417`) becomes:
  `Err(CombatError::OutOfRange)` → keep `order`, push no event, `continue`
  (entry stays due); any other `Err` → today's clear-and-`OrderEnded`.

## Behaviour
Shared: `m = 65536` raw units per metre; `p(x, z)` and MOVE/MOVE_RT fixture
as in `tasks/T036.md:411-435` (A at `p(0,0)`, B at `p(10,0)`); `seg_len`
ignores `y` (`tasks/T036.md:296-297`).
1. **B-D1 (X1 decode).** An ability document with `"range": <i32 raw>`
   decodes to `Some` with that raw value; without the property it decodes
   to `None`; unknown properties still fail via `deny_unknown_fields`
   (`crates/crpg-data/src/combat.rs:583-586`).
2. **B-D2 (X1 bound).** `Some` range with raw `< 1` fails read-time and
   semantic validation; raw `== 1` passes validation (value checks only).
3. **B-D3 (X1 compat).** All shipped abilities (no `range` on the wire)
   decode to `None` and re-emit byte-identical canonical JSON
   (`crates/crpg-data/src/canonical.rs:71-80`); schema version tags
   (`crates/crpg-data/src/document.rs:434-436`) do not move.
4. **B-S1 (X2 admission).** A `UseAbility` whose ability has `None` range
   is unaffected by distance: any live-participant target admitted on range
   grounds at any distance.
5. **B-S2 (X2 measure).** With `Some(range)`, admission is
   `route_measure(measure, [actor_pos, target_pos]) <= raw(range)`; equality
   admits. E.g. range raw `655360` (10 m), A→B (`dx = 655360`) measures
   `655360` → admit; range raw `327680` (5 m) → reject.
6. **B-S3 (X2 boundary ±1).** Range raw `655360`, B at exactly `p(10,0)`
   → admit (`655360 <= 655360`); B one raw unit farther (`x` raw `655361`)
   measures `655361` → reject. Both computed by `ceil_sqrt(dx²+dz²)` with
   `dz = 0` (`tasks/T036.md:440-444`).
7. **B-S4 (X2 no-mover).** With `Some(range)`, actor without mover or
   target without mover (including any mover-free world) → reject, even
   adjacent (distance `0`). `None` range in a mover-free world → unaffected.
8. **B-S5 (X2 order).** The range check runs after every existing
   `UseAbility` check: `SelfTarget`, `OutOfTurn`, `UnknownAbility`,
   per-pool affordability and derived-stat evaluation all win over
   `OutOfRange` (`crates/crpg-sim/src/combat.rs:3395-3500` order). In
   particular a self-targeted ranged ability reports `SelfTarget`, not
   `OutOfRange`.
9. **B-S6 (X2 vertical).** Positions differing only in `y` measure `0`:
   same `x,z`, `y` 5 m apart with any `Some(range)` raw `>= 1` → admit.
10. **B-S7 (X2 omission).** `legal_actions` omits exactly the
    `UseAbility` candidates `validate_action` rejects for range
    (`crates/crpg-sim/src/combat.rs:3604`); in-range candidates are
    unaffected and the closing-action mapping
    (`crates/crpg-sim/src/combat.rs:3572`) is unchanged.
11. **B-S8 (X3 stays due).** In RT, a due order whose revalidation fails
    only with `OutOfRange` keeps its `order` (`StandingOrder`,
    `crates/crpg-sim/src/combat.rs:94-103`), emits no `OrderEnded`, writes
    no execution, spends nothing, and is due again at the next
    `execution_stage` (`crates/crpg-sim/src/combat.rs:4375-4458`); when the
    target later moves into range the same order executes with the normal
    re-key (`crates/crpg-sim/src/combat.rs:4419-4458`).
12. **B-S9 (X3 narrowness).** Any other revalidation failure (e.g.
    `DeadTarget`) still clears the order and emits `OrderEnded`
    (`crates/crpg-sim/src/combat.rs:4410-4417`); TB `perform_action` still
    rejects out-of-range with no spend and no event (validation first,
    `crates/crpg-sim/src/combat.rs:3648`).
13. **B-S10 (hash stability).** Ranged vs unranged definitions hash through
    the existing `state_hash` path (`crates/crpg-sim/src/hash.rs:37-49`);
    worlds without ranged abilities hash as before (legacy shape,
    `crates/crpg-sim/src/combat.rs:510-521`).

## Errors
- T149a: read-time `malformed("ability range must be positive")` for
  `Some` range with raw `< 1`, in the `Ability` arm
  (`crates/crpg-data/src/combat.rs:1563-1636`), following the arm's
  existing style, e.g. `malformed("ability recovery_ticks must be nonzero")`
  (`crates/crpg-data/src/combat.rs:1607-1608`). Semantic `walk_combat`
  (`crates/crpg-data/src/validation.rs:625`) reports the same condition on
  the same pointer (`/range`).
- T149b: `CombatError` gains, last (declaration order is check order,
  `crates/crpg-sim/src/combat.rs:956-960`), after `EncounterTerminal`
  (`crates/crpg-sim/src/combat.rs:1143`):
```rust
OutOfRange,
```
  `Display`: `out of range` (unit variant mirroring `SelfTarget`,
  `crates/crpg-sim/src/combat.rs:1006`). No other variant's text changes;
  there is no `ActionError` type in `crpg-sim` and no existing `OutOfRange`.

## Tests
All sim tests extend the MOVE fixture (`tasks/T036.md:424-435`): A at
`p(0,0)`, B at `p(10,0)`; ranged strike `"range": 655360` (10 m) unless
stated. `1 m = 65536` raw; `seg_len = ceil_sqrt(dx²+dz²)`.
- **T-D1 decode/bound (covers B-D1, B-D2).** Control: `strike`-shaped JSON
  plus `"range": 65536` validates and reads back raw `65536`. Fails if:
  `"range": 0` or `"range": -1` validates; missing `range` reads back
  anything but `None`.
- **T-D2 compat (covers B-D3).** Control: every shipped ability JSON
  round-trips through `write_document`
  (`crates/crpg-data/src/document.rs:495-499`) byte-identical. Fails if any
  byte moves.
- **T-S1 unlimited (covers B-S1).** Control: unranged strike A→B at
  `655360` raw distance → `Ok(())`. Fails if distance affects unranged
  admission.
- **T-S2 admit/reject (covers B-S2).** Control: range `655360`, A→B →
  `Ok(())` (measured `655360`). Case: range `327680`, A→B →
  `Err(OutOfRange)` (measured `655360 > 327680`). Fails if either differs.
- **T-S3 boundary ±1 (covers B-S3).** At: range `655360`, B at `p(10,0)`
  → `Ok(())`. One past: B at raw `x = 655361` → `Err(OutOfRange)`
  (measured `655361`). Fails if the boundary moves either way.
- **T-S4 no-mover (covers B-S4).** Control: mover-free world, unranged
  strike adjacent → `Ok(())`. Cases: same world, ranged strike adjacent →
  `Err(OutOfRange)`; MOVE world with target's mover removed → 
  `Err(OutOfRange)`. Fails if a missing mover admits a ranged ability.
- **T-S5 order (covers B-S5).** Control: self-targeted ranged strike →
  `Err(SelfTarget)` (not `OutOfRange`); unaffordable ranged strike →
  `Err(InsufficientAction)`. Fails if `OutOfRange` wins over either.
- **T-S6 vertical (covers B-S6).** Control: target same `x,z`, `y` raw
  `327680` (5 m) above actor, range raw `1` → `Ok(())` (measured `0`).
  Fails if `y` enters the distance.
- **T-S7 omission (covers B-S7).** Control: `legal_actions` for A with
  ranged strike lists B when range `655360`. Case: range `327680` omits B.
  Fails if omission disagrees with `validate_action` either way.
- **T-S8 stays-due (covers B-S8).** Control (MOVE_RT): due order, target
  moved out of range → after `execution_stage`, `order` is `Some`,
  no `OrderEnded` event, and the entry is still due; then move the target
  back into range → the order executes with normal re-key. Fails if the
  order clears, ends, executes while out of range, or never executes.
- **T-S9 narrowness (covers B-S9).** Control: due order whose target died
  → cleared with `OrderEnded`. TB control: out-of-range `perform_action`
  → `Err(OutOfRange)` with pools unspent. Fails if any non-range failure
  stays due or a ranged TB failure spends.

## What must not change
- Existing `Ability` wire (`crates/crpg-data/src/combat.rs:586-634`),
  schema tags (`crates/crpg-data/src/document.rs:431-436`), migration
  registry versions (`crates/crpg-data/src/migrations/mod.rs:78`,
  `crates/crpg-data/src/migrations/mod.rs:94`), canonical form
  (`crates/crpg-data/src/canonical.rs:71-80`).
- `validate_action` signature (`crates/crpg-sim/src/combat.rs:3332`),
  existing check order and existing `CombatError` variants/texts
  (`crates/crpg-sim/src/combat.rs:962-1143`), side-effect-freedom
  (`crates/crpg-sim/src/combat.rs:3322-3324`).
- Movement surface (`Measure`, `route_measure`, `Mover`, `World.movers/nav`)
  from T036 (`tasks/T036.md:236-243`, `tasks/T036.md:245-260`,
  `tasks/T036.md:302-328`); TB move, history versions, lane-0 protocol.
- All existing goldens byte-identical (see `goldens:` line above).

## Out of scope
Any code in this draft task (draft only); lane-0 protocol (T026p); the
turn-based move itself (T036c); grids/hex layers (later S2–S5 work,
`docs/adr/0036-movement-and-navigation.md:544`); cover, LOS, facing,
difficult terrain (D46f,
`docs/adr/0036-movement-and-navigation.md:586-589`); client previews;
new goldens (T149 adds none).

## Open questions (each with a recommendation)
1. **Should `requires_target: false` abilities with `range` check the
   action's target anyway?** Recommend yes — X2 names "target" without
   qualification
   (`docs/adr/0036-movement-and-navigation.md:471-474`), and the
   `UseAbility` path always has a liveness-checked target
   (`crates/crpg-sim/src/combat.rs:3404-3424`); skipping by flag would add
   a second seam.
2. **Should `OutOfRange` carry `actor`/`target`/`distance`/`range`
   fields?** Recommend no (unit variant) — mirrors `SelfTarget`
   (`crates/crpg-sim/src/combat.rs:1006`), keeps the error surface minimal;
   distances are recomputable in tests via `route_measure`.
3. **Which measure when the encounter definition has no movement block but
   movers exist?** Recommend default `Measure::Euclidean` (the enum's
   `#[default]`, `tasks/T036.md:236-243`); can only arise from hand-built
   worlds, and Euclidean is the sole v1 variant.
4. **Does an out-of-range due order consume recovery/repeat scheduling?**
   Recommend no — nothing executes, so no re-key
   (`crates/crpg-sim/src/combat.rs:4419-4458` never runs); the entry stays
   due as X3 requires.

## Agent log
- 2026-11-09 (UTC) · muse-spark-1.3-contributor + T149 draft b · Skeleton commit to secure progress per attempt-2 notes; filling sections next.

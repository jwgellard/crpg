# T149 — ability range check (ADR-0036 X1–X3)

**Status: blind draft (draft a). Not approved.** The Claude architect
adjudicates it against another independent draft; the user approves the final
contract. It builds on the approved T036 contract (`tasks/T036.md`, D49), which
is the only statement of the movement surface on this branch
(`tasks/T149-facts.md:5-9`).

## Task

ADR-0036 X1–X3 (`docs/adr/0036-movement-and-navigation.md:466-477`), D46d
(`:583-584`) and the table row "new: range (D46d) | `crpg-data`, then
`crpg-sim` | X1-X3 | T036" (`:543`). An ability may declare an optional `range`
(X1); `validate_action` checks it after target liveness with the S2 measure and
the participants' positions, and `legal_actions` omits out-of-range targets
(X2); a due RT order with an out-of-range target neither executes nor ends, it
stays due (X3). X1 lands as **T149a** (data); X2 and X3 as **T149b** (sim),
after T036c.

## Crate(s)

One task = one crate (`AGENTS.md`), so the work is two tasks; T149a ships
first. T149b reads T036's `Mover`, `route_measure` and
`CombatDefinition.movement` (`tasks/T036.md:236-306`, `:369-382`).

**T149a — `crpg-data`.** Owns `crates/crpg-data/src/combat.rs` (the
`Ability.range` field, its doc, the read-time check in the `Document::Ability`
arm of `validate_local`); `schemas/ability.schema.json` (regenerated only, by
`cargo run -p crpg-data --example generate_schemas -- schemas`); new
`crates/crpg-data/tests/ability_range.rs` (implementer) and
`ability_range_contract.rs` (two-keys writer); the mechanical `range: None,`
additions (no assertion changes) at
`crates/crpg-data/tests/combat_content.rs:764`,
`crates/crpg-data/tests/kernel_surface_contract.rs:1341` and `:1586`,
`crates/crpg-data/tests/round_trip.rs:278`,
`crates/crpg-data/tests/srd_content.rs:1145` and
`crates/crpg-data/tests/support/migration_gate.rs:425`; and the docs
`docs/architecture/crpg-data.md` and `crates/crpg-data/AGENTS.md` (the
optional-missing list at `crates/crpg-data/AGENTS.md:33-40`).

**T149b — `crpg-sim`.** Owns `crates/crpg-sim/src/combat.rs`:
`AbilityDefinition.range`, `ability_is_legacy` (`:510`), the lowering at
`:1963-1985`, `CombatError::OutOfRange` and its `Display`, `validate_action`
(`:3332`), the `execution_stage` branch (`:4410-4417`), the doc comments at
`:3316-3320`, `:3547-3552`, `:4371-4374`, and the unit-test table at
`:4738-4778` (`[(&str, Edit); 12]` becomes `13`, with one `("range", |e|
e.range = Some(…))` row). Also new `crates/crpg-sim/tests/ability_range.rs`
(implementer) and `..._contract.rs` (two-keys writer),
`docs/architecture/crpg-sim.md` and `crates/crpg-sim/AGENTS.md`.

`goldens: none move.` The six
`crates/crpg-sim/tests/goldens/{history_v1,history_v2,surface_v1}.{x86_64-pc-windows-msvc,x86_64-unknown-linux-gnu}.txt`
and the six `crates/crpg-testkit/goldens/{combat_basic,replay_basic,srd_lite}_rust-1.98.0_{x86_64-pc-windows-msvc,x86_64-unknown-linux-gnu}_test-default.golden`
stay byte-identical: no shipped ruleset, fixture or existing test declares
`range` (`tasks/T149-facts.md:335-337`), and a range-absent ability keeps its
legacy shape (B-S1). Only `schemas/ability.schema.json` is regenerated (T149a,
not a hash golden, never by a test).

## Readings of the ADR

1. **Two tasks.** D46d and the table say "`crpg-data`, then `crpg-sim`"
   (`docs/adr/0036-movement-and-navigation.md:543`, `:583-584`), and one task =
   one crate. Alternative rejected: one combined task, because it would own two
   crates and break the repository rule.
2. **`range` joins `crpg.ability/2`; no version bump or migration edge.** X1
   says absent means existing data is unchanged, and an optional-missing field
   whose absence is exactly the old behaviour joins at the current tag
   (`crates/crpg-data/AGENTS.md:41-46`). Alternative rejected: `crpg.ability/3`
   with a `2 -> 3` step, which is the tag-only migration the current-tag rule
   replaces and would edit every version pin for no decode difference.
3. **Absent is unwritten; explicit `null` reads as absent and is not written.**
   The crate's optional-missing fields follow this rule, and X1's "absent" is
   the omission. Alternative rejected: writing `"range": null`, which would
   make every ranged-absent document's canonical bytes change and move hash
   goldens.
4. **The wire value is the raw `i32`.** `Fx16_16` is `#[serde(transparent)]`
   over a private `i32` (`crates/crpg-core/src/fixed.rs:22-36`), and every data
   fixed field uses an `i32` schema surrogate
   (`crates/crpg-data/src/combat.rs:100-104`, `:526-528`). Alternative
   rejected: a decimal string, which no other fixed field uses and which
   `canonical_json` would not preserve faithfully.
5. **The range is nonnegative, with no upper bound beyond `i32`.** A negative
   bound is unsatisfiable; the largest measured in-bounds pair is `1518500250`
   (`tasks/T036.md:206`), so a larger range is unlimited in practice.
   Alternatives rejected: accepting a negative (it only fails later at action
   time), and rejecting above that diameter (it forbids a spelling of unlimited
   X1 already covers by omission). See Q3.
6. **The range travels on `AbilityDefinition`, copied at lowering.** The sim
   validates its own lowered definition and holds no data documents.
   Alternative rejected: a data lookup at validation time.
7. **`ability_is_legacy` gains `range.is_none()`.** Otherwise a ranged ability
   loses its range with the legacy shape. Alternative rejected: leaving the
   legacy test unchanged.
8. **The check is last in `validate_action`, after every existing check.** X2's
   "after target liveness" is a precondition, not an immediate slot: the range
   is known only after `UnknownAbility` (`crates/crpg-sim/src/combat.rs:3443`),
   `SelfTarget` stays first (`:3446`), and affordability and derived-stat
   evaluation are pinned (`:3449-3498`,
   `crates/crpg-sim/tests/legal_actions.rs:337`). Alternative rejected:
   immediately after `DeadTarget`, which cannot read the ability and reorders
   pinned tests.
9. **The measure is `CombatDefinition.movement`'s, defaulting to `Euclidean`.**
   S2 holds it on the combat definition
   (`docs/adr/0036-movement-and-navigation.md:317-325`); T036c adds
   `movement: Option<TurnMovement>` (`tasks/T036.md:369-382`). Alternative
   rejected: skipping a check with no movement block, silently unlimited.
10. **Either side lacking a mover is `OutOfRange`, even at distance zero.** X2
    says so without qualification. Alternative rejected: treating a missing
    mover as unlimited.
11. **Compare in `i128`; `measured <= raw(range)` admits the target.** A
    negative raw bound (hand-edited save only; T149a rejects it) admits
    nothing, including distance zero. Alternative rejected: a `u64` cast.
12. **`OutOfRange` is fieldless and declared last.** X3 needs only to
    discriminate it; `SelfTarget` is the fieldless precedent and new variants
    hide fields (`crates/crpg-sim/src/combat.rs:956-960`). Alternative rejected:
    `{ actor, target, range }` fields no caller reads.
13. **X3 retains only `OutOfRange`.** Every other failure still clears the
    order and journals `OrderEnded`; X3 amends T7 "for this case only" and W6
    says only X3 changes order/route independence
    (`docs/adr/0036-movement-and-navigation.md:476-477`, `:269-270`).
    Alternative rejected: retaining every failing order.
14. **A retained order keeps its timeline key and is not re-keyed.** "Stays
    due" means the next tick sees it again; recovery runs only after success
    (`crates/crpg-sim/src/combat.rs:4419-4458`). Alternative rejected: a retry
    delay, which X3 does not state.
15. **`legal_actions` needs no new filter.** It keeps exactly what
    `validate_action` accepts (`crates/crpg-sim/src/combat.rs:3549-3552`,
    `:3604`). Alternative rejected: a second range filter and legality source.

## Interface

### T149a — `crpg-data`

`Ability` (`crates/crpg-data/src/combat.rs:586`) gains, last, after
`bonus_stats` (`:631-633`):

```rust
    /// Optional maximum use range in metres (ADR-0036 X1); absent means
    /// unlimited. The raw value is a nonnegative `Fx16_16`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Option<i32>")]
    pub range: Option<Fx16_16>,
```

The `Document::Ability` arm of `validate_local`
(`crates/crpg-data/src/combat.rs:1563-1636`) gains one check, after the damage
dice loop and before `Ok(())`:

```rust
if ability.range.is_some_and(|range| range.to_raw() < 0) {
    return Err(malformed("ability range must be nonnegative"));
}
```

Generated schema (`schemas/ability.schema.json`, from
`crates/crpg-data/src/schema.rs:117`): `range` becomes a property of the
`crpg.ability/2` object with `"type": ["integer", "null"]`, `"format":
"int32"`, `"minimum": -2147483648`, `"maximum": 2147483647`, and its doc as
the description; `range` is **not** added to `required`. No `schema.rs`,
migration registry (`crates/crpg-data/src/migrations/mod.rs:78`, `:94`) or
`Document` tag (`crates/crpg-data/src/document.rs:434-436`) change.

### T149b — `crpg-sim`

`AbilityDefinition` (`crates/crpg-sim/src/combat.rs:414`) gains, last, after
`damage_dice` (`:452-454`):

```rust
    /// Optional maximum use range in metres; absent means unlimited
    /// (ADR-0036 X1). Copied from the authored ability.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<Fx16_16>,
```

`ability_is_legacy` gains `&& entry.range.is_none()`, and the lowering loop
(`crates/crpg-sim/src/combat.rs:1963-1985`) gains `range: entry.authored.range,`
last. The legacy rebuild at `:706-723` sets `range: None`.

`CombatError` (`crates/crpg-sim/src/combat.rs:962`) gains, last, after
`EncounterTerminal` (`:1143`):

```rust
    /// The target lies beyond the ability's declared range (ADR-0036 X2).
    OutOfRange,
```

with `Display` `OutOfRange at combat/range`, matched beside the other
fieldless variants.

One private helper in `combat.rs`, called as the last statement of the
`UseAbility` arm (after `crates/crpg-sim/src/combat.rs:3497-3498`):

```rust
/// X2: `Ok` when `entry.range` is absent or the target is within it.
fn check_range(world: &World, definition: &CombatDefinition,
    entry: &AbilityDefinition, actor: EntityId, target: EntityId)
    -> Result<(), CombatError> {
    use CombatError::OutOfRange;
    let Some(range) = entry.range else { return Ok(()) };
    let Some(actor_mover) = world.movers().get(actor) else { return Err(OutOfRange) };
    let Some(target_mover) = world.movers().get(target) else { return Err(OutOfRange) };
    let measure = definition.movement.map_or(Measure::default(), |m| m.measure);
    let measured = route_measure(measure,
        &[actor_mover.position(), target_mover.position()]).ok_or(OutOfRange)?;
    if measured as i128 > i128::from(range.to_raw()) { return Err(OutOfRange); }
    Ok(())
}
```

`execution_stage` (`crates/crpg-sim/src/combat.rs:4410-4417`) replaces its
`is_err()` branch with:

```rust
match validate_action(world, &action) {
    Ok(()) => {}
    Err(CombatError::OutOfRange) => continue, // X3: stays due, no event.
    Err(_) => {
        world.combatants_mut().get_mut(actor)
            .expect("the actor is a live combatant").order = None;
        facts.events.push(TickEvent::OrderEnded { actor });
        continue;
    }
}
```

`combat.rs` imports `Measure` and `route_measure` from the crate root.
`Mover`, `ComponentStore::get` and `CombatDefinition.movement` are T036's
(`tasks/T036.md:239-260`, `:303`, `:382`); no new export is needed
(`CombatError` and `AbilityDefinition` are already re-exported at
`crates/crpg-sim/src/lib.rs:34-40`).

## Behaviour

### T149a — data

1. **B-A1 `range` round-trips as an optional raw `i32`.** Decoding an ability
   with no `range` gives `None` and `canonical_json` writes no `range` key;
   decoding `"range": 98304` gives `Some(Fx16_16::from_raw(98304))` and the
   canonical output contains `"range": 98304` at the sorted key position. The
   tag stays `crpg.ability/2`. `range: null` decodes to `None` and is omitted
   on write.
2. **B-A2 The existence and shape of the field are the whole X1 data change.**
   No other `Ability` field, `DamageEntry`, `AbilityCost`, document tag,
   migration edge or canonical writer changes. A range-absent document decodes
   to exactly the same value as before T149a and re-encodes byte-identically.
3. **B-A3 A negative raw range is read-time `Malformed`.** `validate_local`
   rejects `raw < 0` with the message in Errors; `raw == 0` and
   `raw == i32::MAX` are accepted. The check is last in the arm, after every
   existing ability check, so a document that fails an earlier check reports
   that earlier message.
4. **B-A4 The schema is regenerated, never hand-edited.** The
   `generated_schemas()` bytes equal `schemas/ability.schema.json`; `range` is
   present with the shape in Interface, absent from `required`. No other
   schema file changes.
5. **B-A5 Data adds no runtime meaning.** `crpg-sim` does not read
   `Ability.range` until T149b lowers it; T149a changes no sim code, event,
   hash or golden.
6. **B-A6 Shipped content is untouched.** Every ability under `rulesets/` and
   `campaigns/fixtures/` still decodes with `range == None` and unchanged
   canonical bytes (none declares a range today,
   `tasks/T149-facts.md:335-337`).

### T149b — simulation

7. **B-S1 Lowering and persistence.** `AbilityDefinition.range` is
   `entry.authored.range` for every listed ability, in every mode, serialized
   only when `Some`. A world whose abilities all have `range == None`
   serializes exactly as today, with the `abilities` vector still omitted for a
   legacy shape and no `range` key; any ranged ability writes the vector with
   `"range": <raw i32>` in its entry.
8. **B-S2 Error identity.** `CombatError::OutOfRange` is the new last variant,
   rendering exactly `OutOfRange at combat/range`; the declaration-order doc
   and the variant list at `:956-960` otherwise stand.
9. **B-S3 The check sits last and runs after every existing check.**
   `entry.range == None` never returns `OutOfRange`. With `Some`, after
   `NoEncounter`, liveness/nonparticipation, `AbsentActor`/`AbsentTarget`,
   `DeadActor`/`DeadTarget`, the mode stage, `UnknownAbility`, `SelfTarget`,
   per-pool affordability and derived-stat evaluation, `check_range` runs and
   admits when the measured distance is at most `raw(range)`. `measured` is
   `route_measure(measure, [actor position, target position])`, with
   `measure = definition.movement.measure` or `Euclidean` when it is `None`.
10. **B-S4 Missing and degenerate inputs.** No mover on the actor or the target
    is `OutOfRange`; so is a `None` from `route_measure` (impossible for two
    loaded mover positions, bounded by T036 L4). A raw negative range admits no
    target, including one at the actor's exact position.
11. **B-S5 The check is read-only.** `validate_action` still interns nothing,
    creates no RNG stream, enqueues no event, spends nothing and advances
    nothing; repeated calls agree and the world's bytes are unchanged
    (`crates/crpg-sim/src/combat.rs:3322-3324`).
12. **B-S6 `legal_actions` omits out-of-range targets.** It keeps exactly the
    accepted candidates, so a pair rejected with `OutOfRange` is absent,
    in-range pairs remain, and the closing action is unaffected.
13. **B-S7 `perform_action` revalidates and rejects cleanly.** An out-of-range
    `UseAbility` returns `Err(OutOfRange)` and leaves the world byte-identical
    (no spend, damage, event, RNG draw or order).
14. **B-S8 X3 retention.** In a running RT encounter, a due order whose
    validation returns `Err(OutOfRange)` is neither executed nor ended: its
    `order`, timeline key and the journal are unchanged (no
    `HistoryEvent::OrderEnded`, no `ActionResolved`), while the clock still
    advances. An out-of-range target at issue time never becomes an order
    (B-S7).
15. **B-S9 Retry and later execution.** A retained order is due every
    subsequent tick (one attempt per tick; the due snapshot is taken once at
    `crates/crpg-sim/src/combat.rs:4386-4391`). Once the target is back in
    range, the next tick resolves normally: one `ActionResolved`, the ordinary
    spend and the ordinary `min(elapsed + recovery, RT_CLOCK_MAX)` re-key
    (`:4419-4458`).
16. **B-S10 Every other due-order failure is unchanged.** A due order failing
    any other check (including a target beyond range that is also unaffordable,
    which reports `InsufficientAction`) is cleared and journals `OrderEnded`
    exactly as today (`crates/crpg-sim/src/combat.rs:4423-4427`). The doc
    comments at `:3316-3320` and `:4371-4374` and the two crates' `AGENTS.md`
    files are updated with the new rung and the X3 exception.

## Errors

| Value | Text |
|---|---|
| `DataError::Malformed { path: None, .. }` from a negative range | `malformed document: ability range must be nonnegative` |
| the same with `path: Some(p)` | `malformed document at {p}: ability range must be nonnegative` |
| a non-integer JSON number (`1.5`, `1.0`) | `malformed document: non-integer number` (`crates/crpg-data/src/migrations/mod.rs:121`) |
| a string or a number outside `i32` | the existing serde `Malformed` text for the wrong shape; not pinned |
| `CombatError::OutOfRange` | `OutOfRange at combat/range` |

No other variant or text changes. `LegalActionsError` is unchanged: an omitted
candidate is no error, and the closing action's errors still map to
`LegalActionsError::Actor` (`crates/crpg-sim/src/combat.rs:3505-3545`).

## Tests

Integration tests only: implementer and blind two-keys suites in
`crates/crpg-data/tests/ability_range{.rs,_contract.rs}` and
`crates/crpg-sim/tests/ability_range{.rs,_contract.rs}`. Each case names a
control and a "fails if". Sim cases use T036's MOVE/MOVE_RT fixture
(`tasks/T036.md:421-435`): A at `p(0,0)`, B at `rp(655360,0,0)` (10 m), C at
`rp(0,0,655360)` (10 m), A's timeline key 10, B's 9, `round_ticks` 4. Range is
set by `support::edit_ability` or by editing the decoded ability document;
tests never edit shipped files.

### Data

- **D-1 `range_round_trip` (B-A1, B-A2).** Take `rulesets/srd-lite/focus.json`
  (a `crpg.ability/2` document, `rulesets/srd-lite/focus.json:35`); decoding
  gives `range == None`, and `canonical_json` of the decoded value contains no
  `"range"` key. Insert `"range": 98304`: decode gives
  `Some(Fx16_16::from_raw(98304))`, canonical output contains `"range": 98304`,
  and re-decoding that output reproduces it. `"range": null` decodes to `None`
  and its canonical output has no `range` key (equal to the absent case).
  *Control:* the unedited document decodes and re-encodes unchanged.
  *Fails if:* `None` writes the key, `Some` loses it, or `null` is preserved.
- **D-2 `range_integer_domain` (B-A1).** `0`, `98304`, `1518500250` and
  `2147483647` decode to their exact raw values; `2147483648`, `-2147483649`
  and `"1"` fail with `Malformed` (text not pinned); `1.5` and `1.0` report
  `malformed document: non-integer number`. *Control:* `2147483647` succeeds.
  *Fails if:* a float is coerced, a string is accepted, or `i32` bounds move.
- **D-3 `range_negative_rejected` (B-A3).** `-1` and `-2147483648` fail with
  `malformed document: ability range must be nonnegative`; `0` and `1` pass.
  Multi-fault: empty `dice` plus `-1` reports the dice message, pinning the
  range check last. *Control:* removing only the range leaves the dice
  message. *Fails if:* a negative is accepted or wins an earlier fault.
- **D-4 `range_schema` (B-A4).** `generated_schemas()["ability.schema.json"]`
  equals the file's bytes. Parsed, the payload object's `range` property is
  `{"type": ["integer", "null"], "format": "int32", "minimum": -2147483648,
  "maximum": 2147483647, "description": <the field's doc>}`, `required`
  omits it, and the tag is `crpg.ability/2`. *Control:* the ruleset schema is
  unchanged. *Fails if:* the file is hand-edited, `range` is required, or the
  tag moves.
- **D-5 `shipped_abilities_have_no_range` (B-A6).** `rulesets/minimal-d6/strike.json`,
  `rulesets/srd-lite/heavy.json`, `rulesets/srd-lite/focus.json` and their
  `campaigns/fixtures/` copies decode with `range == None` and re-encode
  without a `range` key; the T149a diff touches no `crates/crpg-sim` file
  (B-A5). *Control:* D-1's ranged copy has the key. *Fails if:* a shipped file
  changes, the default becomes `Some`, or T149a edits sim code.

### Simulation

- **S-1 `range_persists_and_legacy_stays` (B-S1).** MOVE TB with ability range
  raw `655360`: `AbilityDefinition` serializes `"range": 655360`, the world
  round-trips `Some(655360)`, and `CombatDefinition` contains the `abilities`
  vector with that entry (the range makes `ability_is_legacy` false). With
  `range = None` the same ability writes no `range` key and `ability_is_legacy`
  returns what the unextended table returned. *Control:* the no-range world is
  serialized exactly as T149a left it. *Fails if:* a ranged single-ability
  world still serializes legacy, or a range-absent one becomes non-legacy.
- **S-2 `boundary_and_one_raw_past` (B-S3).** MOVE TB, range `655360`:
  `UseAbility { actor: A, ability, target: B }` (A→B = `655360`) is `Ok`.
  Range `655359`: `Err(OutOfRange)`, one raw unit past. For B→C the distance
  is `926820` (`ceil_sqrt(2 * 655360^2)`): range `926820` is `Ok`, `926819`
  is `OutOfRange`; `None` admits both. *Control:* each admitted case without
  the range change. *Fails if:* the comparison is `<`, or `seg_len` differs.
- **S-3 `missing_mover_is_out_of_range` (B-S4).** Standard fixture (no
  movement block, hence no movers): range `0`, `allow_self_target = true`,
  TB self-target (distance 0) → `OutOfRange`; `range = None` → `Ok`. MOVE
  world: a joined combatant has no mover (T036 Reading 5), so ranged actor A
  targeting the joiner → `OutOfRange`; no range → `Ok`. *Control:* the
  no-range action is admitted in both worlds. *Fails if:* a missing mover is
  unlimited, or distance 0 is admitted without a mover.
- **S-4 `raw_negative_range_admits_nothing` (B-S4).** A hand-edited limited
  definition (`range = Some(Fx16_16::from_raw(-1))`, bypassing the data
  check), self-target at distance 0 → `OutOfRange`; `Some(0)` → `Ok`.
  *Control:* the zero bound admits that position. *Fails if:* the bound is
  cast to `u64` or the sign is ignored.
- **S-5 `check_runs_after_every_existing_check` (B-S3).** MOVE TB, range
  `655359` combined one at a time with: dead target B → `DeadTarget`; target
  not a participant → `NotParticipant`; self target with
  `allow_self_target = false` → `SelfTarget`; actor's pool at 0 with cost 1 →
  `InsufficientAction`; a derived fixed `boom = divide(stat pace, literal
  fixed 0)` used as the action's `ActorStat` defense → `StatEvaluation`; an
  unlisted ability → `UnknownAbility`. *Control:* each case with B at 1 m and
  healthy/affordable reports `Ok`, or the named non-movement error in its own
  case. *Fails if:* `OutOfRange` wins any of these.
- **S-6 `validation_is_read_only` (B-S5).** MOVE TB, out-of-range action: two
  `validate_action` calls return `Err(OutOfRange)`; `bytes_of(world)`,
  `state_hash` and the event queue length are identical before and after.
  *Control:* an admitted in-range call also changes nothing. *Fails if:* the
  check mutates, interns, spends or draws.
- **S-7 `legal_actions_omits_out_of_range` (B-S6).** MOVE TB, actor B, range
  `926820`: B→A and B→C (`655360`, `926820`) are listed. Range `926819`:
  B→C is absent, B→A remains. Range `None`: both remain. Actor A,
  `allow_self_target = true`, range `0`: B and C are omitted and A→A
  (distance 0) is listed. *Control:* the unlimited list. *Fails if:* an
  out-of-range pair is listed or the closing action changes.
- **S-8 `perform_action_rejects_cleanly` (B-S7).** MOVE TB, range `655359`,
  A→B: `validate_action` and `perform_action` both return
  `Err(OutOfRange)`; `CombatError::OutOfRange.to_string()` is exactly
  `OutOfRange at combat/range`; the world bytes are unchanged; a
  `HistoryWorld` journal gains no `ActionResolved`
  (`crates/crpg-sim/src/history.rs:98-108`) and no `OrderEnded` (`:143-147`).
  *Control:* range `655360` resolves with one `ActionResolved` and the
  ordinary spend. *Fails if:* a failed action spends, damages, journals,
  draws, or the error text differs.
- **S-9 `rt_out_of_range_order_stays_due` (B-S8, B-S9, B-S10).** MOVE_RT
  (`round_ticks = 4`), range `655360`: A issues `UseAbility` on B at 10 m
  (accepted, order stored, nothing spent). `place(B, p(20,0))` puts B at raw
  x `1310720`. Ten ticks reach `elapsed = 10`; A's order is still
  `Some(StandingOrder { ability, target: B, repeat: false })`, its key is
  still `10`, the journal has no `OrderEnded` and no `ActionResolved`, A's
  movement pool (template 106) is still 6, and positions are unchanged.
  `place(B, p(1,0))` (raw x `65536`) and one tick (`elapsed = 11`) resolve
  once: one `ActionResolved`, the ordinary cost spent, A re-keyed at
  `min(11 + 4, RT_CLOCK_MAX) = 15`. *Control:* the same order never moved out
  of range resolves at `elapsed = 10`; an out-of-range order that is also
  unaffordable journals `OrderEnded` (B-S10). *Fails if:* the retained order
  clears, re-keys, journals an end, spends, or fails to resolve after B
  returns to 1 m.

## What must not change

- `validate_action`'s order and results for range-absent actions; the pinned
  precedence tests (`crates/crpg-sim/tests/legal_actions.rs:264`, `:337`) and
  every `rt_orders.rs` rung for non-range failures.
- `CombatError`'s existing variant order and texts; `HISTORY_VERSION`, every
  `HistoryEvent`, and the history/surface goldens.
- `AbilityDefinition`'s legacy bytes for range-absent abilities; world
  serialization order; `state_hash` for worlds with no range.
- `crpg.ability/2`, the migration registry, `canonical_json`, the read-time
  order of existing ability checks, and every shipped ruleset/fixture byte.
- S2: no `Measure` variant, no bypass of `route_measure`, and no direct
  `seg_len` in a rules path (`docs/adr/0036-movement-and-navigation.md:317-325`).
- T036's movement surface; `crpg-testkit`, `crpg-net`, `crpg-server`,
  `crpg-godot` and all goldens; the repository rules (no dependency, no
  `unsafe`, no `HashMap`/`HashSet`, no `f32`/`f64`, no `crpg-contracts` or
  `rust-toolchain.toml` edits).

## Out of scope

- Any code: this is a draft; the architect writes the final contract.
- A grid/hex measure (S4, S5), `snap`, and any new `Measure` variant.
- The turn-based move itself (T036c), routes, `perform_move`, history v3.
- Lane-0 protocol and network projection (T026p), server baking (T037), host
  preview and client range display (T049/T026/T050).
- Range for items, effects or any non-ability object; line of sight, cover,
  minimum range, range bands and per-target modifiers.
- `requires_target` changes (`crates/crpg-sim/src/combat.rs:3549-3552`),
  tutorial data, new shipped rulesets and any balance value.

## Open questions (each with a recommendation)

1. **Should a ranged ability in a ruleset with no `movement` block be a data
   warning?** X2 makes it unusable (no movers means `OutOfRange`).
   *Recommend:* no new diagnostic in T149a; it would add a `DiagnosticCode`
   beyond X1. Document the consequence instead, or add the warning later.
2. **Should `OutOfRange` carry `{ actor, target, range }`?** *Recommend:*
   fieldless. X3 needs only the variant and the stable `Display` is unchanged;
   the fields would be dead public surface (they change no behaviour).
3. **Is a huge declared range (above `1518500250`) legal?** *Recommend:* yes,
   no upper check; it is unlimited inside the coordinate box, and X1 already
   has omission for that intent.
4. **Does the range apply when `requires_target` is false?** *Recommend:* yes.
   Every `UseAbility` names a target today
   (`crates/crpg-sim/src/combat.rs:3549-3552`); a future untargeted action
   revisits this.
5. **Should a negative range in a hand-edited sim save fail world load?**
   *Recommend:* no new load rule; the `i128` comparison admits nothing (S-4)
   and data already rejects it at read time.
6. **Should `check_range` be a helper or inline?** *Recommend:* the helper in
   Interface, so the insertion point is one call and the check has one
   unit-testable home. Inlining changes no behaviour.
7. **Does T036c persist `CombatDefinition.movement`?** Its serializer is
   custom (`crates/crpg-sim/src/combat.rs:617-660`) and T036's interface does
   not show the edit. *Recommend:* T036c must serialize the field and decode
   it; otherwise a load silently loses the measure and defaults to Euclidean.
   T149b depends on that and should not land before it.

## Agent log

- 2026-11-09 (UTC) · opencode-go/deepseek-v4.1-flash + T149 draft a · Wrote the blind T149 contract draft: split T149a/T149b, pinned the raw `range` wire shape, the last check order, the `OutOfRange` retention for X3, and boundary tests at and one raw unit past range.

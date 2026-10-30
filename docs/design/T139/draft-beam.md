# T139 — persistent creature state; join and leave (`crpg-sim`): blind contract draft (beam)

Date: 2026-10-30 (UTC)
Status: **Proposed (draft by opencode-go/beam-free)**

## Task

ADR-0029 Follow-on 6 (`docs/adr/0029-kernel-range-and-time-model.md:485`):
persistent creature state outside encounters (stats, pools, effects, tags) and
the journaled join and leave operations (T13, T14). S-set: the `World`
struct (`docs/IMPLEMENTATION_PLAN.md` §1.4). This draft is written blind;
another lab writes the same draft independently and a Claude architect
adjudicates.

Inputs: `tasks/T139-facts.md`, ADR-0029 T12–T16 and Open question 7,
`tasks/T138.md` (B31, T139 hand-off at `:851-858`), `tasks/T137.md`
(hand-off at `:815-818`, `:898-901`), `docs/architecture/crpg-sim.md`,
`crates/crpg-sim/AGENTS.md`, and the sim source (`world.rs`, `combat.rs`,
`tick.rs`, `history.rs`).

## Crate(s)

**`crpg-sim` only.** Four packages (T139a–d), one crate each, serialized
(see §9). No companion crate: join/leave are free functions, not
`CombatAction` variants, so `crpg-script`, `crpg-server` and `crpg-net`
need no exhaustive-match edits.

## 1. Where persistent creature state lives and its exact shape

### Current fact
`World` has no creature-level state outside an encounter: `EntityMeta {}`
(`world.rs:58-59`), `Transform` (`world.rs:70`), and the combatant store
that only exists while an encounter is active (`world.rs:82`, `combat.rs:760`).
`Combatant` (`combat.rs:202-223`) is the only home of stats, pools, effects
and tags, and it is stripped at `end_encounter` (`combat.rs:3719`).

### Recommendation
Add a new component store to `World`, not fields on `EntityMeta`:

```rust
// crates/crpg-sim/src/world.rs
#[serde(default, skip_serializing_if = "ComponentStore::is_empty")]
creatures: ComponentStore<Creature>,
```

with `World::creatures()` / `World::creatures_mut()` (crate-visible mutator,
matching `combatants_mut`). Absence-skip keeps non-combat worlds byte-identical
to today. A creature is any live entity carrying this component; presence is
liveness, so no membership index is needed (ADR-0020 discipline).

### Exact shape
```rust
// crates/crpg-sim/src/combat.rs (or a new creature.rs module)
/// Persistent creature state, survives across encounters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Creature {
    /// Current and maximum health.
    pub health: u32,
    pub max_health: u32,
    /// Dead creatures retain their entity and state.
    pub dead: bool,
    /// All stats in authored order: Int (carried into encounters) and
    /// non-Int (persisted only, T137 D3). Stored as the rules kernel's
    /// value type; no float, no handle.
    pub stats: Vec<(String, crpg_rules::StatValue)>,
    /// Persistent pool balances, in authored pool-id order.
    pub pools: Vec<crpg_rules::ResourcePool>,
    /// Encounter-attached effects are NOT written back (see §2). This field
    /// is reserved for effects applied outside an encounter; empty today.
    pub attached: Vec<AttachedEffect>,
    /// Placement tags, ascending, distinct.
    pub tags: Vec<String>,
    /// Stored initiative key for joins; 0 if never rolled.
    pub initiative: i32,
}
```

### Options considered
- **Option A (recommended):** a separate `ComponentStore<Creature>` on
  `World`, mirroring `combatants`. Absence-skip keeps goldens; clean liveness.
- **Option B:** extend `EntityMeta {}` with creature fields. Rejected: it
  changes every entity's bytes (including non-creatures), breaks the
  `EntityMeta` braced-empty serde invariant (`world.rs:52-57`), and has no
  absence form.
- **Option C:** a `BTreeMap<EntityId, Creature>` field. Rejected: a second
  structure that can disagree with the arena; `ComponentStore` already gives
  insertion order and serde.

### Consequences
- `World` struct gains a field (S-set; named in §6).
- `state_hash`/`history_hash` code unchanged; new state hashes through
  ordinary serde (`hash.rs:37-49`).
- Non-Int stats use `crpg_rules::StatValue` (which is `Int`/`Fixed`/`Bool`/
  `Enum`/`Tags`/`Dice`), so no float and no new type. `Fixed` is `Fx16_16`,
  already rules-legal.
- Iteration order is `ComponentStore` insertion order (authored), matching
  `combatants` (`world.rs:970-973`); no `HashMap`/`HashSet`.
- D6 scratch-interner rule (T137) extends unchanged: T139 interns nothing
  into `World` beyond the existing `TargetStat` write.

## 2. Encounter start and end: seeding and write-back

### Current fact
`start_encounter`/`start_encounter_in_mode` publish `combat` and the
`combatants` store with health/max_health from the prefab, attributes from
declared `Int` stats, pools at max from templates, attached empty, tags per
T137 B19 (`facts §2`). `end_encounter` (`combat.rs:3719`) strips combat
components and keeps entities live; stats/pools/effects/tags existed only in
the encounter and are gone.

### Seeding at start
For each participant, in authored order:
1. Build the `Combatant` from the ruleset/prefab exactly as today
   (B21 order, T137).
2. If `world.creatures` contains the entity, overwrite the combatant's
   `health`, `max_health`, `dead`, `attributes` (Int stats present in the
   creature's `stats`), `action_pool`/`extra_pools` (current balance clamped
   to template max, or max if absent), and `tags` from the creature. Non-Int
   stats are not carried (T137 D3); they stay persistent only.
3. If no creature exists, create one from the resulting combatant (health,
   max_health, dead, Int stats, pools at max, tags, initiative) so the first
   encounter persists state.
4. Store the creature back before publishing.

This is a seeding step inside `start_encounter_tracked`, after the existing
checks and before the spawn loop, mutating only the creature store (which is
not yet hashed into the encounter's draw order; draws stay as today).

### Write-back at end
`end_encounter` gains a write-back pass before releasing:
1. For each combatant (authored order), find or create its `Creature`.
2. Set `health`, `max_health`, `dead` from the combatant.
3. Set Int attributes into `stats` (replace by name; create if missing).
   Non-Int stats are untouched.
4. For each pool in the combatant (primary then extras), find the matching
   persistent pool by id and set its `current` (and `max`/`refresh` from the
   template); append if missing.
5. Set `tags` to the combatant's tags with any tag that is a grant of an
   effect being released removed (granted tags from surviving persistent
   effects are retained; encounter-only grants are stripped).
6. `attached` is NOT written back from `Combatant.attached`: encounter-scoped
   effects (duration_rounds) are stripped at release. Persistent `attached`
   is reserved for effects applied outside an encounter (none today).
7. Store `initiative` from `Combatant.initiative`.

### What resets vs carries over
| Piece | At encounter start | At encounter end |
|---|---|---|
| Health / max_health | persistent overrides prefab | written back |
| Int attributes | persistent overrides | written back |
| Non-Int stats | not carried (T137 D3) | untouched (persist) |
| Pools | persistent current clamped, else max | written back |
| Tags | persistent ∪ grants (T137 B19) | persistent base (grants stripped) |
| Effects | persistent `attached` + encounter effects | encounter effects stripped |
| Initiative | persistent or rolled (T137 B17) | written back |

### Terminal states
A terminal encounter keeps `mode`/`pending_mode` (T138 B2) and its
combatants. Write-back happens only at `end_encounter` or `leave_encounter`,
not on becoming terminal. A creature that dies in an encounter has
`dead = true` persisted; a later encounter seeds `dead = true` and the
participant is rejected at start? No: `start_encounter` already rejects
dead? Today a dead combatant can be re-included only after healing; T139
does not change that. Persistent `dead` is informational for the host; the
encounter adapter does not auto-revive.

### Options considered
- **Option A (recommended):** write-back only at release/leave; terminal
  state is a scheduling condition, not a persistence trigger. Matches
  T138 (terminal keeps mode until release).
- **Option B:** write-back on every tick. Rejected: O(world clone) per tick
  and no need; persistent state is between-encounter.
- **Option C:** no write-back; persistent state is read-only seed. Rejected:
  defeats the purpose of persistence.

### OnTick outside encounters
There is no encounter clock outside an encounter (T138, `deliver_clock_advance`
is encounter-scoped, `facts §9`). Recommendation: **no automatic pool refresh
outside encounters.** Persistent pools keep their balance; `OnTick(n)` only
fires inside an encounter via `EncounterClock`. Rest/refresh outside
encounters is a host concern (T141). This settles ADR-0029 T14's open half.

## 3. Join (T13, T14)

### Operation
```rust
pub fn join_encounter(
    world: &mut World,
    entity: EntityId,
    placement: Ulid,
) -> Result<(), CombatError>;
```
History wrapper: `HistoryWorld::join_encounter(&mut self, entity, placement)`.

A journaled operation (ADR-0029 T16). Stages a clone, runs the combat
function, commits through `commit` (H5 pattern). Emits one `Joined { entity }`
event on success.

### Precondition checks (in order)
1. `NoEncounter` if `combat` is `None`.
2. `TerminalEncounter` if the encounter is terminal (recommended; see open
   question 3). [Option B allows revival; see §9.]
3. `TooManyCombatants` if `combatants.len() >= MAX_COMBATANTS`.
4. `MissingCreature` if the entity is not live or has no `Creature`.
5. `DeadParticipant` if `creature.dead`.
6. `AlreadyParticipant` if the entity is already in `combatants`.
7. `AreaMismatch` / `MixedArea` as in `start_encounter` (`world.rs` checks),
   if the world is area-bound.
8. `MissingPlacement` if `placement` is not a live authored placement (host
   validates; sim checks presence in the definition's placement set if any).
   Sim treats an unknown placement as `MissingPlacement`.

### Seeding the joiner
Build a `Combatant` from the encounter's definition and the creature's
persistent state (§2 seeding), with:
- `placement = placement`.
- Initiative: if the definition has an `initiative` rule, roll one
  `evaluate` on `combat.roll` in authored order (the joiner alone) and store
  the key (T137 B17); otherwise use the creature's stored `initiative`.
- Pools seeded from persistent (clamped), else max.
- Tags = persistent ∪ grants of unexpired attachments (none at join).
- `attached = creature.attached` filtered to effects present in the
  definition (encounter effects only; persistent attached reserved).
- `order = None`.

### Timeline insertion
- **TB, first round** (`round == 0` and the timeline still holds its initial
  entries, i.e. no rollover has occurred): insert the joiner at
  `InitiativeKey(initiative)`, but never ahead of the active combatant. If
  its key is greater than the active's key, insert at the active's key
  (clamped, tie-broken by `EntityId` ascending after the active). The joiner
  does not become active.
- **TB, later rounds** (`round > 0` or the timeline has rolled over): do not
  insert into the current round. Mark the joiner as pending and insert it at
  the next rollover (`tb_round_start`), at its initiative. Implementation:
  store the joiner in `combatants` and a `pending_joiners: Vec<EntityId>` on
  `CombatState`? That adds a field. Alternative: insert into timeline with
  a sentinel key `i32::MIN`? No. Recommendation: add
  `pending_joiners: Vec<EntityId>` to `CombatState` (skipped when empty) as
  a new last field, and have `tb_round_start` drain it (insert each at
  initiative, then clear). This is an S-set touch on `CombatState` (new
  field) — named in §6.
- **RT**: insert the joiner into the timeline at key `elapsed` (due now) with
  `order: None` (T138 B11). `active` stays `None`.

### Terminal RT encounter
Recommended: reject with `TerminalEncounter` (see open question 3). If the
alternative (allow revival) is chosen, a join that brings `rt_alive` to
`>= 2` makes the encounter non-terminal and the stages resume on the next
tick; the joiner is keyed at `elapsed`.

### History
Emits `Joined { entity }`. No `TurnStarted`/`RoundStarted` from a join
unless the insertion triggers a rollover (it does not: a TB first-round
join never becomes head; a later-round join waits).

## 4. Leave

### Operation
```rust
pub fn leave_encounter(
    world: &mut World,
    entity: EntityId,
) -> Result<(), CombatError>;
```
History wrapper: `HistoryWorld::leave_encounter(&mut self, entity)`.

Journaled; emits `Left { entity }`.

### Semantics
1. `NoEncounter` if `combat` is `None`.
2. `NotParticipant` if the entity is not in `combatants`.
3. Write back the combatant's persistent state (§2 write-back) for this
   entity.
4. Remove the combatant from `combatants`, its timeline entry, and its order.
   The entity and its `Creature` stay live.
5. If the leaving combatant was `active` (TB), call `advance_tracked` to
   repair scheduling (new head + `TurnStart`, rollover, or terminal).
6. If it was not active, leave the turn untouched.
7. If this was the last combatant, release the encounter (`combat = None`,
   as `despawn` of the last combatant, `world.rs:861-862`) and report
   `encounter_ended` in the transition facts (RT: when `rt_alive` drops
   from `>= 2` to `< 2`).

### How leave differs from death and despawn
| | Death | Leave | Despawn |
|---|---|---|---|
| Entity stays live | yes | yes | no |
| Combatant stays in store | yes (dead) | no | no |
| Scheduling entry removed | yes | yes | yes |
| Persistent state written back | at release only | immediately | n/a (gone) |
| Emits `Died` | yes | no | no |
| Releases encounter if last | yes (despawn path) | yes | yes |

Death keeps a dead combatant in the encounter (terminal when `< 2` alive);
leave removes the combatant entirely and may revive the encounter's
scheduling without it. Despawn additionally destroys the entity and its
transform.

### Options considered
- **Option A (recommended):** leave is a combatant removal with immediate
  write-back, reusing `advance_tracked` and the last-participant release.
- **Option B:** leave defers write-back to release. Rejected: a leaving
  creature that joins another encounter later (T13 merge) would lose state.
- **Option C:** leave requires the combatant to be dead. Rejected: that is
  death, not leave.

## 5. History and replay

### New payloads
`HistoryEvent` gains two variants, adjacent-tagged, after `OrderEnded`:
```rust
Joined { entity: EntityId },   // {"type":"joined","value":{"entity":{"index":..,"generation":..}}}
Left   { entity: EntityId },   // {"type":"left","value":{"entity":{"index":..,"generation":..}}}
```
Manual visitors, entity via `EntityRef` (as `TurnStarted`'s actor).

### HISTORY_VERSION
Bump `HISTORY_VERSION` from 2 to 3. Lazy and sticky (T138 D16): a wrapper
starts at 1, upgrades to 2 on the first RT-capable commit, and to 3 on the
first commit whose events include `Joined`/`Left`. Never downgrades. A
rejected commit changes nothing.

### Loader rule for older history
- Accept `version` 1, 2 or 3 (`unknown history version` otherwise).
- Payloads decode with all three vocabularies whatever the version.
- After all keys are read, a version-1 wrapper is rejected if any retained
  payload is a v2 or v3 variant, or if its world holds `combat.clock: Some`.
- A version-2 wrapper is rejected if any retained payload is a v3 variant.
- Every input valid today decodes, re-encodes and hashes unchanged.

### Existing goldens / .replay
No existing golden or `.replay` file changes: no shipped schedule uses
join/leave, TB-only histories stay at version 1, and RT-capable histories
without join/leave stay at version 2. A new `history_v3` golden is optional
(T139d) and needs H-golden approval. If a change becomes unavoidable, it
would be the two new golden files only, and only with the user's approval.

### Replay determinism
Two runs of one script (same seed, commands, ticks) give equal per-tick
`state_hash` sequences (T138 B35 pattern). A save/reload mid-script
continues to equal hashes (T137 B24).

## 6. Save shape (S-set touches)

S-set touches, named explicitly:
1. **`World` struct** (`world.rs:68-99`): new field `creatures:
   ComponentStore<Creature>` (absence-skip). S-set per `docs/AGENT_TEAM_PLAN.md:131-134`
   and `IMPLEMENTATION_PLAN.md` §1.4.
2. **`CombatState` struct** (`combat.rs:760-776`): new field
   `pending_joiners: Vec<EntityId>` (skipped when empty), only meaningful in
   TB; absent in RT and TB-only worlds that never join.
3. **`Creature` struct** (new, serialized).
4. **`WorldRepr`** (`world.rs:110-126`): new `creatures` field with
   `#[serde(default = "ComponentStore::new")]`.

Not touched: `crpg-contracts`, `crpg-data` schemas/migrations, GDExtension
boundary, workspace `Cargo.toml`, `deny.toml`, workflows. No new dependency
(`deps.py` `ALLOWED` untouched). No migration needed: older saves load with
an empty creature store and empty pending joiners.

## 7. Public API

### New types
```rust
pub struct Creature { /* §1 */ }
pub enum JoinError is folded into CombatError (below).
```

### New `CombatError` variants (after `WrongMode`)
```rust
/// The entity is already a participant in the active encounter.
AlreadyParticipant,
/// The entity has no persistent creature state (or is not live).
MissingCreature,
/// A dead creature cannot join an encounter.
DeadParticipant,
/// A join was attempted against a terminal encounter (recommended policy).
TerminalEncounter,
/// The entity is not a participant in the active encounter.
NotParticipant,
```
Display: `AlreadyParticipant at combat/join`, `MissingCreature at combat/join`,
`DeadParticipant at combat/join`, `TerminalEncounter at combat/join`,
`NotParticipant at combat/leave`.

### New functions
```rust
pub fn join_encounter(world: &mut World, entity: EntityId, placement: Ulid)
    -> Result<(), CombatError>;
pub fn leave_encounter(world: &mut World, entity: EntityId)
    -> Result<(), CombatError>;
```

### History wrapper
```rust
impl HistoryWorld {
    pub fn join_encounter(&mut self, entity: EntityId, placement: Ulid)
        -> Result<(), HistoryError>;
    pub fn leave_encounter(&mut self, entity: EntityId)
        -> Result<(), HistoryError>;
    pub fn version(&self) -> u32; // unchanged; now may return 3
}
```
Gameplay errors win over history errors (H5). `join_encounter`/`leave_encounter`
commit their events (`Joined`/`Left`); a zero-event operation is not needed
here.

### `lib.rs` re-exports
`Creature`, `join_encounter`, `leave_encounter` added to the combat re-export
block. Nothing removed.

### Unchanged signatures
`start_encounter`, `start_encounter_in_mode`, `perform_action`,
`validate_action`, `legal_actions`, `end_encounter`, `request_mode`,
`EncounterSpec`, `CombatAction`, `ActionOutcome`, `World::spawn`/`despawn`.

## 8. Test list

Each case names its Behaviour items and includes a valid control.

T139a (state + save):
- **A1 (§1, §6)** `tests/creature_state.rs`: `Creature` round-trips with
  empty and full fields; a world with no creatures omits the `creatures` key
  (byte-identical to pre-T139).
- **A2 (§2)** `tests/creature_state.rs`: first encounter creates a `Creature`
  for each participant with health/max/pools/tags/initiative.
- **A3 (§2)** `tests/creature_state.rs`: a second encounter seeds health,
  pools and tags from the persistent creature (control: prefab-only values
  would differ).
- **A4 (§2)** `tests/creature_state.rs`: `end_encounter` writes back health,
  pools, Int stats, tags (grants stripped) and initiative.
- **A5 (§2)** `tests/creature_state.rs`: non-Int stats persist across
  encounters and are not carried into combat (no margin/damage change).
- **A6 (§2)** `tests/creature_state.rs`: encounter effects are stripped at
  release (control: persistent `attached` unchanged).
- **A7 (§2)** `tests/creature_state.rs`: `OnTick` pools do not refresh
  outside an encounter (balance unchanged over 100 ticks with no encounter).
- **A8 (§6)** `tests/creature_state.rs`: load coherence rejects a creature
  with `health > max_health` or `dead != (health == 0)`.

T139b (join):
- **B1 (§3)** `tests/join.rs`: TB first round inserts the joiner at its
  initiative, never ahead of `active` (control: active's key unchanged).
- **B2 (§3)** `tests/join.rs`: TB later round does not insert into the
  current timeline; the joiner appears at the next rollover.
- **B3 (§3)** `tests/join.rs`: RT join is keyed at `elapsed` with `order: None`.
- **B4 (§3)** `tests/join.rs`: each rejection (NoEncounter, TooManyCombatants,
  MissingCreature, DeadParticipant, AlreadyParticipant, TerminalEncounter)
  leaves world bytes, RNG and events unchanged.
- **B5 (§3)** `tests/join.rs`: a TB join seeds pools/tags/health from the
  persistent creature.
- **B6 (§3)** `tests/join.rs`: a join into a terminal encounter is rejected
  (recommended policy; control documents the alternative).

T139c (leave):
- **C1 (§4)** `tests/leave.rs`: leave removes the combatant and timeline
  entry, writes back persistent state, keeps the entity live.
- **C2 (§4)** `tests/leave.rs`: leaving the active combatant advances the
  turn (new head + `TurnStart`).
- **C3 (§4)** `tests/leave.rs`: leaving the last combatant releases the
  encounter.
- **C4 (§4)** `tests/leave.rs`: leave of a non-participant returns
  `NotParticipant` with no mutation.
- **C5 (§4)** `tests/leave.rs`: leave differs from death (no `Died`) and from
  despawn (entity stays live with `Creature`).

T139d (history/replay):
- **D1 (§5)** `tests/history_v3.rs`: `Joined`/`Left` encodings, round trips,
  duplicate/unknown/missing keys.
- **D2 (§5)** `tests/history_v3.rs`: emission order for join/leave inside
  `tick`/`perform_action`/`despawn`.
- **D3 (§5)** `tests/history_v3.rs`: version upgrades 1→2→3 lazily and
  sticks; rejected commits do not upgrade.
- **D4 (§5)** `tests/history_v3.rs`: loader rejects v1/v2 wrappers holding
  v3 payloads.
- **D5 (§5)** `tests/history_v3.rs`: two runs give equal per-tick
  `state_hash` sequences; save/reload mid-script continues equal.
- **D6 (§5)** `tests/history_v3.rs`: TB-only histories stay at version 1;
  RT histories without join/leave stay at version 2.
- **D7 (§1, §2)** `tests/creature_state.rs`: reload mid-encounter preserves
  the creature store and persistent state (control: re-seeded combatant
  unchanged).

## 9. Split proposal (one crate each, landing order)

| # | Id | Crate | Owns | Depends on |
|---|---|---|---|---|
| 1 | T139a | `crpg-sim` | `Creature` store on `World`, `WorldRepr` field, save/load, start seeding, end write-back, non-Int persistence, OnTick-outside decision | T137, T138 |
| 2 | T139b | `crpg-sim` | `join_encounter`, `JoinError` variants, TB first-round/later-round/RT insertion, `pending_joiners`, seeding | T139a |
| 3 | T139c | `crpg-sim` | `leave_encounter`, `LeaveError`/`NotParticipant`, write-back, turn repair, last-participant release | T139a |
| 4 | T139d | `crpg-sim` | `Joined`/`Left` payloads, `HISTORY_VERSION` 3, lazy upgrade, loader rules, determinism + reload tests, optional golden | T139b, T139c |

Boundary list (left to later tasks):
- Non-combat effect application and rest/refresh outside encounters
  (T141/T142): T139 fixes no refresh (§2).
- Carrying non-Int stats into an encounter (T137 D3 defers; T139 keeps them
  persistent-only).
- Creature transfer between worlds (`transfer_entity`, T027a): creature state
  is not moved; a later task extends the move contract.
- Wire projection of persistent state and join/leave intents (T140/T141).
- AI decisions to join/leave (host, T141).
- Creature prefab loading and authoring schema (crpg-data, T136 follow-ups).
- Revival-on-join policy if the recommended rejection is overturned.

## Interface (consolidated)

See §1, §3, §4, §7. Summary of new public items: `Creature`,
`join_encounter`, `leave_encounter`, `CombatError::{AlreadyParticipant,
MissingCreature, DeadParticipant, TerminalEncounter, NotParticipant}`, and
the `HistoryWorld` methods. `World::creatures()`/`creatures_mut()` are
crate-visible mutator only. No new dependency, no float, no
`HashMap`/`HashSet`, no `unsafe`.

## Behaviour (consolidated)

B1. A non-combat world omits `creatures` and `pending_joiners`; bytes match
    pre-T139.
B2. Start seeds combatants from persistent state when present, else creates
    persistent state from the combatant.
B3. End writes back health, pools, Int stats, tags (grants stripped) and
    initiative.
B4. Non-Int stats persist and never affect combat.
B5. Join TB first round never becomes head; later-round join waits for
    rollover; RT join is due now.
B6. Leave writes back, repairs scheduling, releases on last.
B7. Terminal encounters reject join (recommended).
B8. No pool refresh outside encounters.
B9. Determinism and reload-mid-encounter hold.

## Errors

See §7. Closed enums: `CombatError` (extended) and `HistoryError`
(unchanged; join/leave reuse it through the wrapper). Every variant listed
in §7.

## Tests

See §8 (A1–A8, B1–B6, C1–C5, D1–D7).

## Out of scope and hand-off

- No host, protocol, data or script change (join/leave are free functions).
- No new `CombatAction` variant (so no companion crate).
- No `SimEvent` variant (persistence is state, not an event; `Joined`/`Left`
  are `HistoryEvent`s only).
- Movement, reactions, rest, sides, lifetimes stay with their owners
  (T034/T036, T136b, T141).
- `crpg-net`/`crpg-server` projection of persistent state and join/leave
  intents are T140/T141.

## Open questions for the user

1. **Persistent state home.** *Recommend* (a): new `ComponentStore<Creature>`
   on `World` (absence-skip). (b) extend `EntityMeta`. The latter changes
   every entity's bytes and has no absence form.
2. **Pool seeding at start.** *Recommend* (a): persistent current balance
   clamped to template max, defaulting to max when absent. (b) always max at
   start (persistent pools become display-only).
3. **Join into a terminal encounter.** *Recommend* (a): reject with
   `TerminalEncounter`, keeping terminal absorbing until release. (b) allow
   and revive when `rt_alive`/alive reaches `>= 2`.
4. **Effects across encounters.** *Recommend* (a): strip encounter effects at
   release; persistent `attached` is reserved for future non-combat effects.
   (b) carry unexpired encounter effects over as persistent (needs a
   tick-based expiry).
5. **Tags at end.** *Recommend* (a): strip tags that are grants of released
   effects. (b) carry all combatant tags forward.
6. **Join initiative.** *Recommend* (a): roll at join when the definition has
   an initiative rule (one draw on `combat.roll`), else use the stored key.
   (b) never roll on join (stored key or 0).
7. **`HISTORY_VERSION`.** *Recommend* (a): bump to 3 with lazy sticky
   upgrade; v1/v2 still read. (b) keep at 2 and emit no version bump
   (join/leave unjournaled at the version level — not recommended, breaks
   replay).
8. **Non-Int stat storage type.** *Recommend* (a): `crpg_rules::StatValue`.
   (b) a sim-owned enum mirroring it (more code, same bytes).
9. **Leave of the active combatant.** *Recommend* (a): repair via
   `advance_tracked` (new head/rollover/terminal). (b) defer repair to the
   next tick (risks a dangling active).
10. **Join/leave as free functions vs `CombatAction`.** *Recommend* (a): free
    functions + `HistoryWorld` methods, no companion crates. (b)
    `CombatAction::Join`/`Leave` variants (requires crpg-script/server/net
    edits).

## Definition of done

Per package:
```
cargo fmt --all
cargo clippy -p crpg-sim --all-targets -- -D warnings
cargo test -p crpg-sim
python tools/lint/deps.py
python tools/lint/determinism.py
python -m unittest discover -s tools/lint -p "test_*.py"
git diff --exit-code <base> -- rulesets campaigns crates/crpg-testkit/goldens crates/crpg-sim/tests/goldens/history_v1.x86_64-pc-windows-msvc.txt crates/crpg-sim/tests/goldens/history_v1.x86_64-unknown-linux-gnu.txt crates/crpg-sim/tests/goldens/history_v2.x86_64-pc-windows-msvc.txt crates/crpg-sim/tests/goldens/history_v2.x86_64-unknown-linux-gnu.txt crates/crpg-sim/tests/goldens/surface_v1.x86_64-pc-windows-msvc.txt crates/crpg-sim/tests/goldens/surface_v1.x86_64-unknown-linux-gnu.txt
```

## Agent log

- 2026-10-30 (UTC) · opencode-go/beam-free + T139 draft · Drafted the persistent-creature-state, join and leave contract: a new `ComponentStore<Creature>` on `World` (absence-skip), seeding/write-back at encounter boundaries, journaled join with TB-first-round/later-round/RT insertion and a rejection list, leave with scheduling repair and last-participant release, `HISTORY_VERSION` 3 with lazy upgrade, and a four-package split, with options, consequences and recommendations for each decision.
# T139 — persistent creature state; join and leave (`crpg-sim`): blind contract draft (muse)

## Task

ADR-0029 Follow-on 6 (`docs/adr/0029-kernel-range-and-time-model.md:485`):
persistent creature state outside encounters (stats, pools, effects, tags),
plus the journaled join and leave operations (T13, T14). Today every creature
stat, pool balance, effect attachment and creature tag exists only inside an
encounter (`tasks/T139-facts.md` §1, §9): `Combatant.attributes`
(`crates/crpg-sim/src/combat.rs:205`), `action_pool`/`extra_pools` (`:209`,
`:212`), `attached` (`:215`) and `tags` (`:222`) are created per participant
at start and dropped at release (`:3719`, `world.rs:861-862`), and `World`
has no creature store of any kind (`world.rs:68-99`). This draft pins the
persistent shape, the seed/write-back lifecycle, join, leave, history, save
bytes, API, tests and the split. `crpg-sim` only. S-set: the `World` struct
(and `CombatState`, which lives in it). No new dependency.

Base: the batch branch holding T137 (adapter generalization, `tasks/T137.md`)
and T138 (mode/RT/history v2, `tasks/T138.md`). T139 touches no history
versioning beyond two new payloads, no tick stage, no `Timeline` policy.

## Status

Draft, blind, for architect adjudication. Every code claim below was checked
against this branch; each cites `path:line`. Open questions end the file,
each with a recommendation.

## Decision

- **D1. One new `World`-owned store, `creatures: ComponentStore<CreatureState>`.**
  An out-of-encounter creature is an entity with a `CreatureState` entry; an
  in-encounter creature is an entity with a `Combatant` entry; never both at
  once. *Rejected:* fields on `EntityMeta` (`world.rs:58-59`, a braced
  placeholder whose emptiness keeps arena saves distinct from vacant slots);
  a side map outside `World` (un-hashed authority, against the
  world-owned-state trap in `crates/crpg-sim/AGENTS.md`).
- **D2. Persist everything the encounter reads, symbolically.** Base usable
  `Int` stats, typed non-`Int` stats (T137 D3/D13 hand-off:
  `tasks/T137.md:815-818`), pool balances with template identities, health
  and maximum, placement identity, and tags. Derived `Int` formulas stay on
  `CombatDefinition.derived` (`combat.rs:483-484`) and are re-queried, never
  stored per creature. *Rejected:* persisting only `Int` (leaves the T137
  non-`Int` hand-off unowned); persisting derived values (stale on the first
  `Stat` effect, the T137 D5 reason).
- **D3. Seed at start, write back at end; durations pause outside.**
  `start_encounter` seeds each `Combatant` from its entity's `CreatureState`;
  `end_encounter`, `despawn` of a combatant and `leave` write the closings
  back. Round-based effect expiry (`expires_round`, `combat.rs:195`) has no
  meaning where there is no round, so attachments suspend outside encounters
  and resume on the next seed. `OnTick` pools never refresh outside an
  encounter clock (the T138 hand-off question, `tasks/T138.md:856-858).
- **D4. Join and leave are free functions plus history methods, journaled per
  T16.** `join_encounter(world, join)`, `leave_encounter(world, entity)`,
  `HistoryWorld::join_encounter`, `HistoryWorld::leave_encounter`. They are
  not `CombatAction` variants: join has no actor-turn semantics and leave
  targets any combatant, and AI enumeration must not offer them (the T138 D7
  reason). *Rejected:* action variants (would need turn-stage surgery and
  would leak into `legal_actions`).
- **D5. Terminal encounters refuse join.** A join into a terminal encounter
  (TB `active == None`, `combat.rs:766`; RT `rt_alive < 2`, T138 D15) fails
  with the new `EncounterOver`. The derived RT terminal stays derived: no
  stored flag. *Rejected:* revival by join (re-opens T10 single-refresh
  accounting and T138 B18/B24, which pin terminal as absorbing).
- **D6. History stays at version 2.** `Joined`/`Left` are v2-vocabulary
  payloads beside `RoundStarted`/`ModeChanged`/`OrderEnded`
  (`history.rs:127-144`); the D16 lazy-sticky trigger
  (`history.rs:838-846`) already covers "events include a v2 variant".
  *Rejected:* a version 3 (moves every `history_v2` golden and hash for no
  loader-visible gain).
- **D7. Every existing golden stays byte-identical.** New `World`/`CombatState`
  fields are absence-skipped; new events fire only on new operations; TB-only
  histories stay at version 1. A moved golden is a stop, not a re-bless.

## Interface

Each new field is the **last** field of its struct and each new variant comes
after the existing ones, in the order shown. Every new public item has a doc
comment (`lib.rs:2`, `#![warn(missing_docs)]`). No dependency, no float, no
`HashMap`/`HashSet`, no `unsafe`. Iteration follows the store (insertion
order); tag lists are ascending and distinct (the T137 B19 shape).

### `crates/crpg-sim/src/combat.rs`

```rust
/// One symbolic extra stat value carried outside encounters (T139, D2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case", deny_unknown_fields)]
pub enum PersistentStat {
    /// Whole-number `Int` stat (base usable stats live in `attributes`).
    Int(i32),
    /// `Fixed` stat, canonical data notation, never read by combat.
    Fixed(String),
    /// `Bool` stat.
    Bool(bool),
    /// `Enum` stat, symbolic variant name.
    Enum(String),
    /// `Tags` stat, ascending distinct names.
    Tags(Vec<String>),
    /// `Dice` stat, canonical dice notation.
    Dice(String),
}

/// One creature's persistent state, keyed by entity in a `ComponentStore`
/// on `World` (T139, D1). Absent while the entity fights.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreatureState {
    /// The placed-creature identity this state was instantiated from.
    pub placement: Ulid,
    /// Base usable `Int` stat values in authored stat order (mirrors
    /// `Combatant.attributes`).
    pub attributes: Vec<(String, i32)>,
    /// Non-`Int` stats by name, in authored stat order; never read in combat.
    pub extra_stats: Vec<(String, PersistentStat)>,
    /// Current health; `dead` is `health == 0`, as for combatants.
    pub health: u32,
    /// Health maximum; always at least one.
    pub max_health: u32,
    /// Pool balances in definition template order (primary first).
    pub pools: Vec<ResourcePool>,
    /// Suspended effect attachments, ascending effect identity.
    pub attached: Vec<AttachedEffect>,
    /// Creature tags, ascending and distinct; always carried here.
    pub tags: Vec<String>,
}

/// One join request: an already-live outsider entering a live encounter.
#[derive(Debug)]
pub struct JoinRequest<'a> {
    /// Live, non-combatant entity holding a `CreatureState`.
    pub entity: EntityId,
    /// The placed-creature document for `entity`.
    pub placement: &'a AuthoredPlacement,
    /// The creature prefab for `entity`.
    pub creature: &'a AuthoredCreature,
}
```

`CombatState` (`combat.rs:760-776`) gains one last field:

```rust
    /// Entities that joined but wait for their entry round (T139, TB).
    /// Absent when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending_joins: Vec<EntityId>,
```

`CombatError` (`combat.rs:895-1052`) gains two last variants, after
`StatEvaluation`:

```rust
    /// A join or order targets a terminal encounter (T139, D5).
    EncounterOver,
    /// The join entity already fights in this encounter.
    AlreadyParticipant {
        /// The entity that already holds a `Combatant`.
        entity: EntityId,
    },
```

Display hides fields, matching existing style: `EncounterOver at
combat/encounter`, `AlreadyParticipant at combat/participants`.

```rust
/// Validates and publishes one joiner into a live encounter (ADR-0029 T13).
pub fn join_encounter(world: &mut World, join: &JoinRequest<'_>) -> Result<(), CombatError>;

/// Removes one combatant from a live encounter without despawning it
/// (ADR-0029 Follow-on 6). The entity stays live with written-back state.
pub fn leave_encounter(world: &mut World, entity: EntityId) -> Result<(), CombatError>;

/// Crate-visible tracked forms feeding the history wrapper.
pub(crate) fn join_encounter_tracked(world: &mut World, join: &JoinRequest<'_>) -> Result<JoinFacts, CombatError>;
pub(crate) fn leave_encounter_tracked(world: &mut World, entity: EntityId) -> Result<TransitionFacts, CombatError>;

/// One join observation: where the joiner landed, if anywhere yet.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct JoinFacts {
    /// The timeline key the joiner was inserted at, if inserted now.
    pub inserted_key: Option<InitiativeKey>,
    /// Whether the joiner waits in `pending_joins` for the next rollover.
    pub deferred: bool,
}
```

`impl CreatureState` gains readers mirroring `Combatant`
(`combat.rs:225-280`): `placement`, `attributes`, `health`, `max_health`,
`pools`, `attached`, `tags`.

### `crates/crpg-sim/src/world.rs`

`World` (`world.rs:68-99`) gains one last field, after `area`:

```rust
    /// Persistent out-of-encounter creature state (T139, D1, S-set).
    /// Absent (empty) in worlds that never register a creature, so those
    /// serialize byte-identically to today. Compatibility shape, not a hash
    /// exclusion: when present every field hashes.
    #[serde(default, skip_serializing_if = "ComponentStore::is_empty")]
    creatures: ComponentStore<CreatureState>,
```

`WorldRepr` (`world.rs:110-126`) gains `#[serde(default =
"ComponentStore::new")] creatures: ComponentStore<CreatureState>`. Accessors:
`pub fn creatures(&self) -> &ComponentStore<CreatureState>` and crate-visible
`pub(crate) fn creatures_mut(&mut self)`. Load checks L1–L4 (B22) go in
`Deserialize`, beside the combat checks (`world.rs:152-725`).

Registration helper (no new operation kind; it rides `spawn`):

```rust
/// Attaches persistent creature state to a live, stateless, non-combatant
/// entity. Fails `AlreadyParticipant` for combatants, `MissingStat` /
/// `InvalidStatValue` for bad prefab stats. Emits no event.
pub fn register_creature(world: &mut World, entity: EntityId, placement: &AuthoredPlacement, creature: &AuthoredCreature) -> Result<(), CombatError>;
```

### History (`crates/crpg-sim/src/history.rs`)

`HistoryEvent` (`history.rs:80-144`) gains two last variants:

```rust
    /// One outsider joined a live encounter (T139).
    Joined {
        /// The entity that joined.
        entity: EntityId,
    },
    /// One combatant left a live encounter without despawning (T139).
    Left {
        /// The entity that left.
        entity: EntityId,
    },
```

Both are v2 (`is_v2`, `:147-154`). Wire form follows the adjacent-tag
convention (`type` before `value`, field order as declared):

```json
{"type":"joined","value":{"entity":{"index":1,"generation":1}}}
{"type":"left","value":{"actor":{"index":1,"generation":1}}}
```

`Left` reuses the `actor` key spelling so its visitor shares the
`OrderEnded`-style single-entity shape; `Joined` uses `entity` like
`Spawned`/`Died`. (Either spelling round-trips; the split keeps
join-vs-leave readable in the journal.)

```rust
impl HistoryWorld {
    /// Journaled join (ADR-0029 T13, T16); journals one `Joined`.
    pub fn join_encounter(&mut self, join: &JoinRequest<'_>) -> Result<(), HistoryError>;
    /// Journaled leave; journals one `Left` plus any trailing transition.
    pub fn leave_encounter(&mut self, entity: EntityId) -> Result<(), HistoryError>;
}
```

### `crates/crpg-sim/src/lib.rs`

Re-export gains `CreatureState`, `PersistentStat`, `JoinRequest`,
`join_encounter`, `leave_encounter`, `register_creature`. Nothing removed.

## Behaviour

"Alive" and "terminal" keep their T138 meanings. "Persistent state" is the
entity's `CreatureState`. Items marked [a] belong to split part T139a, [b]
to T139b (see "Split proposal").

1. **B1 (store) [a].** `creatures` is the only persistent creature state in
   the crate. An entity holds a `CreatureState` or a `Combatant`, never both;
   `Transform`, timeline membership and area presence are orthogonal. Worlds
   that never register a creature write no `creatures` key and hash exactly
   as today.
2. **B2 (registration) [a].** `register_creature` checks, in order:
   `AbsentActor` (dead id) — reuse of the combat spelling keeps one
   live-entity check; then `AlreadyParticipant`; then per-stat presence /
   wholeness / positivity exactly as start's B2 (`tasks/T137.md:431-439`),
   with non-derived `Fixed` presence-checked and `Bool`/`Enum`/`Tags`/`Dice`
   carried symbolically into `extra_stats`. It mutates nothing on failure and
   journals nothing on success (registration rides the already-journaled
   `spawn`; history needs no new op for it).
3. **B3 (shape) [a].** `attributes` holds the base usable `Int` stats in
   authored order (the T137 B3 list); `extra_stats` holds every other
   declared stat in authored order. `max_health >= 1`, `health <=
   max_health`, `pools` mirrors the definition template order with current
   balances, `attached` is ascending, `tags` ascending distinct. The union
   bound `MAX_TAGS` applies at registration (`TooManyTags`).
4. **B4 (seed at start) [a].** After every existing start check passes
   (T137 B21 order, `combat.rs:1200-1214`; T138 `EncounterActive` then
   `ModeUnsupported`, `combat.rs:1270-1275`), each participant with a
   `CreatureState` seeds its `Combatant` from it: `attributes`, `health` /
   `max_health`, pool currents (clamped to the template maxima, never
   raised), suspended `attached` re-based to `round + duration` semantics of
   a fresh attachment, `tags` per the T137 B19 conditional carriage (only in
   encounters with a granting effect, so the `srd_lite` goldens do not
   move). Participants without one seed exactly as today (pools at max,
   empty attachments). Seeding removes the `CreatureState` entry.
5. **B5 (write-back) [a].** `end_encounter` (`combat.rs:3719-3743`) writes
   each combatant's closings back into `creatures` before stripping the
   component: current `health`, pool currents, `tags` (persistent copy kept
   whole: encounter-conditional carriage never deletes stored tags).
   Attachments are suspended back (absolute `expires_round` converted to
   remaining rounds). Entities stay live, no events emitted, summary
   unchanged. A combatant whose entity was despawned writes back nothing.
6. **B6 (suspend, no outside refresh) [a].** Outside an encounter nothing
   expires and nothing refreshes: no `RoundStart`/`TurnStart`/`OnTick`
   delivery runs without an encounter clock (`combat.rs:3460-3477` stays
   encounter-scoped). A suspended attachment resumes with its remaining
   duration on the next seed. Tick on a creature-only world advances only
   the world tick (`tick.rs:31-36`).
7. **B7 (join validation) [b].** `join_encounter` checks, in order:
   `NoEncounter` (no live `combat`, the `end_encounter` spelling at
   `combat.rs:3720`); `EncounterOver` (terminal in either mode, D5);
   `TooManyParticipants` (live combatants + waiting joins would exceed
   `MAX_COMBATANTS`); `AbsentActor` (dead entity id); `AlreadyParticipant`;
   placement/area/creature/stat checks in the T137 B21 positions (per
   participant, B2-equivalent); `TooManyTags` on the joiner's union. Failure
   mutates nothing: no entity, event, RNG stream, interner entry or hash
   change (the start-transactionality rule, `tasks/T137.md:614-615`).
8. **B8 (TB join placement) [b].** The joiner's key is its stored initiative
   (rolled once at join when `definition.initiative` is `Some`, on
   `combat.roll` after the last check — the T137 B17 draw position — else the
   creature's authored key, validated `MissingStat`-free). Round 0 with a
   live head: if the key orders strictly after the head, insert at once at
   the key. If it would land on or ahead of the head, the entity is pushed to
   `pending_joins` and inserted at its key the moment the active turn
   advances (same op that advances: `perform_action`/`despawn`/leave path
   through `advance_tracked`, `combat.rs:3312-3410`). Round ≥ 1: the joiner
   always waits in `pending_joins` and enters at the next `tb_round_start`
   (`combat.rs:3244-3297`), at its key. *This is ADR-0029 T13/T14 verbatim*
   (`docs/adr/0029-kernel-range-and-time-model.md:260-264`).
9. **B9 (RT join) [b].** The joiner is keyed at the current `elapsed` with
   `order: None` (the T138 hand-off, `tasks/T138.md:851-858`), drawn from its
   `CreatureState` pools at clamped currents. It acts on the first tick after
   it receives an order (T138 B17). Join never sets an order.
10. **B10 (join effects) [b].** The joiner's `attached` seeds from its
    suspended list re-based to the live `round`; its `TurnStart`/`RoundStart`
    pools are not refreshed on entry (T10 single-refresh: entry is not a
    round start). Its `CreatureState` entry is removed on publication.
11. **B11 (leave validation) [b].** `leave_encounter` checks, in order:
    `NoEncounter`; `NotParticipant { entity }` (live outsider, the existing
    spelling at `combat.rs:901-904`); `AbsentActor` (dead id);
    `DeadActor` (the dead cannot leave — they await despawn or release; this
    is the leave-vs-death boundary). Success removes the timeline entry and
    the `Combatant`, writes back persistent state per B5, and clears any
    standing order without an event.
12. **B12 (leave advance) [b].** If the leaver held `active`, the shared
    advance runs (new head + `TurnStart`, rollover, or terminal — the
    despawn-active rule in `world.rs:868-877`, reused not copied). Any other
    leaver leaves the turn. Orders other combatants hold that target it stay
    until due (T138 B34). Leaving the last combatant releases the encounter
    (`combat = None`, the `world.rs:861-862` shape) with interners, events,
    RNG and tick retained.
13. **B13 (leave vs death/removal) [b].** Leave emits no `Died` (health is
    retained, nonzero by B11), removes no entity (unlike `despawn`,
    `world.rs:838-891`), ends no encounter by itself unless it was the last
    combatant (unlike `end_encounter`, which releases unconditionally), and
    keeps the leaver re-joinable in a later round. Death still removes only
    scheduling and retains the zero-health `Combatant` (the death-vs-despawn
    trap in `crates/crpg-sim/AGENTS.md`).
14. **B14 (history join/leave) [b].** `HistoryWorld::join_encounter` journals
    exactly one `Joined { entity }`; `leave_encounter` journals one `Left {
    entity }` plus the trailing transition (`TurnStarted` or
    `EncounterEnded`) in the existing `push_transition` position
    (`history.rs:924-941`). Leave of the last combatant of a nonterminal
    encounter journals `EncounterEnded`; of an already-terminal one, none
    (the despawn rule at `history.rs:513-517`). Order setting, `Stop`,
    `request_mode` and release stay event-free as today.
15. **B15 (version stickiness) [b].** A wrapper stays at 1 until the first
    commit staging a `Joined`/`Left` (or any earlier v2 trigger,
    `history.rs:841-846`); then 2 for good. A rejected join/leave changes
    neither version nor journal (gameplay errors win, `history.rs:638-641`).
16. **B16 (determinism) [b].** Two runs of one script (same seed, same ops
    and ticks) give equal per-step `state_hash` sequences in one binary; a
    run that saves and reloads mid-encounter continues to the same hashes.
    Join draws (B8) and deferred inserts are journal-ordered, so replay
    through the journal reproduces keys exactly.
17. **B17 (despawn/transfer coherence) [a].** `despawn` strips a `CreatureState`
    with the rest and enqueues only `Despawned` (`world.rs:857-859` extended
    one line). `transfer_entity` moves the `CreatureState` with the entity
    (the T027a "extend the move contract" invariant); an in-encounter
    combatant still cannot transfer.

## Errors

Closed `CombatError` gains exactly `EncounterOver` and `AlreadyParticipant`
(Interface). Precedence additions: join checks run B7's order; leave runs
B11's order; registration runs B2's order. No existing path's precedence
moves. `HistoryError` is unchanged: gameplay errors arrive as
`HistoryError::Combat`, as today (`history.rs:633-654`).

## Tests

Each test names its Behaviour items. Fixture builders extend
`tests/support/mod.rs`; `rulesets/` and `campaigns/` are never edited.
`JoinRequest` fixtures use an already-spawned, registered outsider.

T139a (persistent state, no join/leave):

- **P1 (B1, B2, B3).** Register all stat kinds; assert exact `CreatureState`
  bytes. Controls: dead id gives `AbsentActor` with world unchanged;
  combatant gives `AlreadyParticipant`; missing `Fixed` gives `MissingStat`;
  fractional `Int` gives `InvalidStatValue`; 1025-tag union gives
  `TooManyTags`.
- **P2 (B4).** Start with registered creatures; assert seeded `Combatant`
  equals an unregistered start except pool currents clamp to maxima and tags
  follow B19. Control: unregistered participants seed exactly as today
  (bytes equal pre-T139 for the same fixture).
- **P3 (B5, B6).** Fight one round with spends, damage and an attachment;
  `end_encounter`; assert written-back health, pool currents, suspended
  attachment with remaining duration, and intact tags. Tick 50 times outside;
  assert nothing expires or refreshes. Control: immediate release without
  actions writes back entry values unchanged.
- **P4 (B5, B17).** Despawn a combatant mid-fight; assert no write-back and
  surviving combatants unaffected. Transfer a registered outsider between
  bound worlds; assert state moves whole. Controls: despawn of an outsider
  strips state with only `Despawned`; transfer of a combatant still fails.
- **P5 (B1, B5, save shape).** Round-trip a world with creatures and a
  suspended list; assert equal `World`. Mutate one raw-JSON field per new
  load check (B22 L1–L4); assert each message. Controls: worlds without
  `creatures` load and hash exactly as today; TB-only saves gain no new key.

T139b (join, leave, journal):

- **J1 (B7, B8).** TB round-0 joins: a key strictly after the head inserts at
  once at its key; a key on/ahead of the head waits in `pending_joins` and
  appears at its key when the active turn advances. Controls: each B7
  rejection with world bytes, RNG and events unchanged, ordered neighbours
  (`NoEncounter` before `EncounterOver` before `TooManyParticipants`).
- **J2 (B7, B8).** TB round ≥ 1 join waits for the next rollover and enters
  at its key with no mid-round refresh. Rolled-initiative definition: the
  join draw is the next `combat.roll` draw after the last check (assert via
  cloned-RNG evaluation, the T137 R9 technique). Control: authored-key
  definition draws nothing.
- **J3 (B9, B10).** RT join keys at `elapsed` with `order: None`, acts after
  receiving an order, sets no refresh on entry. Controls: join into a
  terminal RT encounter gives `EncounterOver` with nothing changed; join with
  fewer than two live combatants pre-join still publishes (terminal from
  B18 only when alive < 2 after publication — assert both sides).
- **J4 (B11, B12, B13).** Leave of a non-head removes scheduling only;
  leave of the head advances (head/`TurnStarted`, rollover, terminal as
  applicable); leave of the last combatant releases with state written back.
  No `Died` in any case; leaver re-joins next round. Controls: outsider
  gives `NotParticipant`; dead id `AbsentActor`; dead combatant `DeadActor`;
  no encounter `NoEncounter`; each leaves the world unchanged.
- **H1 (B14, B15).** Exact `joined`/`left` encodings, round trips, and every
  key fault (duplicate/unknown/missing, `value`-before-`type`). Emission
  order: `Joined` alone; `Left` then trailing `TurnStarted` or
  `EncounterEnded`. Version stays 1 through creature-only histories, becomes
  2 at the first join, stays 2 after release and acknowledgement. A rejected
  join/leave upgrades nothing. Controls: a v1 wrapper holding `joined`
  fails `history v1 journal holds a v2 event`; TB-only suites stay version 1.
- **R1 (B16).** Two runs of one script (start, joins round 0 and 2, leaves,
  mode switch, 200 ticks) give equal per-tick `state_hash` sequences; a
  save/load mid-encounter (after a deferred join is waiting) continues to
  equal hashes. Control: the same script with join/leave removed hashes as
  the pre-T139 build where comparable.
- **Two-keys:** `tests/persistent_contract.rs`, written blind from this
  contract by the OpenCode test writer.

## Hash and goldens

`state_hash` and `history_hash` code unchanged; nothing excluded
(`hash.rs:37-49` pattern). New values hash through ordinary serde, absent
when empty:

| Value | JSON | Present when |
|---|---|---|
| `World.creatures` | `"creatures":[[<id>,<CreatureState>],…]` (store pair-list shape) | nonempty |
| `CombatState.pending_joins` | `"pending_joins":[{"index":…,"generation":…},…]` | nonempty |
| `CreatureState.extra_stats` | `"extra_stats":[["lore",{"type":"tags","value":[…]}]]` | nonempty |
| `CreatureState.pools` | `"pools":[…]` in template order | nonempty |
| `CreatureState.attached` | `"attached":[{"effect":…,"expires_round":…}]` | nonempty |
| `CreatureState.tags` | `"tags":[…]` ascending distinct | nonempty |

No existing golden moves: `history_v1.*`, `history_v2.*`, `surface_v1.*`
per target, all six `crpg-testkit/goldens/*` files, every `crpg-cli` replay
fixture and hash assertion. No new golden file is proposed: R1 plus the
hand-written oracles cover the behaviour, and goldens for join/leave wait
for the RT host (the T138 OQ6 precedent).

## Save shape and S-set

S-set touches, each explicit:

1. `World` struct: `+ creatures: ComponentStore<CreatureState>` (world.rs).
2. `CombatState` (a `World` field's type): `+ pending_joins: Vec<EntityId>`.
3. History payload vocabulary: `+ Joined`, `+ Left` at `HISTORY_VERSION` 2
   (no version change, no migration).
4. Tick order: none — no stage added or reordered; join/leave run outside
   `tick`, and deferred inserts ride existing `advance_tracked` /
   `tb_round_start` branches.

Migration: none. Old saves omit `creatures`/`pending_joins` and load with
empty stores (serde `default`, the `WorldRepr` pattern at
`world.rs:118-125`); absence-compat extends to TB-only worlds per T138 B6.
`World::deserialize` gains load checks B22:

- **L1**: `creatures` entry for a non-live entity, or for a combatant:
  `creature state addresses no outsider`.
- **L2**: `max_health == 0`, `health > max_health`, pool identity/maximum/
  refresh disagreement with the running definition (in-encounter check;
  outside encounters only maxima positivity is checked):
  `creature state disagrees with its definition`.
- **L3**: `attributes` names/order disagree with `attribute_names` (+
  derived), or `extra_stats` names another kind's shape:
  `creature stats disagree with their declaration`.
- **L4**: `|tags ∪ every grant| > MAX_TAGS`:
  `creature tags exceed their bound`.

No dependency is added (`deps.py` `ALLOWED` untouched). No `f32`/`f64`, no
`HashMap`/`HashSet`, no `unsafe`, no OS branch, no clock/thread/I-O.

## Split proposal

One crate (`crpg-sim`), two packages in landing order:

- **T139a — persistent state and lifecycle.** `CreatureState`,
  `PersistentStat`, `creatures` + accessors + L1–L4, `register_creature`,
  B4 seeding, B5 write-back, B6 suspension, despawn/transfer coherence
  (B17). Tests P1–P5. No behaviour change for worlds without creatures;
  no new event; no join/leave. Docs: architecture + AGENTS deltas for the
  store and lifecycle, each signed.
- **T139b — join and leave with journal.** `JoinRequest`, `pending_joins`,
  `join/leave_encounter` (+tracked), `EncounterOver`,
  `AlreadyParticipant`, `Joined`/`Left` + `HistoryWorld` methods, B7–B16.
  Tests J1–J4, H1, R1 + two-keys. Needs T139a merged; T140/T141 consume it.

If T139a overruns, the exact fallback is: T139a1 = store, registration,
seed (B1–B4, P1–P2, P5-load half); T139a2 = write-back, suspension,
coherence (B5, B6, B17, P3–P4).

## Boundary list (left to later tasks)

Rest, long-term effects and condition reads of tags (first condition
consumer per `tasks/T137.md:819-820`); sides; movement (T034/T036, T12);
`OnRest` pools; stat-added damage; T140 wire codes for `Joined`/`Left`,
`EncounterOver` (action rejection) and `AlreadyParticipant` (host-side);
T141 pause/auto-pause/AI for joiners (RT AI fires when due with
`order() == None`, `tasks/T138.md:871-872`); T143 preview modelling of
join keys; `crpg.ui/1`; concurrent encounters (Open question 7, still a
separate S-set change to a `CombatState` collection).

## What must not change

`state_hash`/`history_hash` code; `World` field order; `Timeline` (no
policy, no new method — deferred inserts use `insert`/`remove`/`iter`);
`end_turn`; `World::advance_tick`; `SimEvent` (no new variant);
`start_encounter`/`perform_action` signatures; the pinned `CombatError`
precedence of every existing path; the `perform_action`/`resolve_use` draw
order for content without join draws; history caps and `commit`-only
journal writes; the T137 scratch-interner rule (join/start interning
follows D6, world interner gains only today's `TargetStat` write).

## Open questions for the user

1. **Join into a terminal RT encounter: reject or revive?**
   *Recommend* (a) reject with `EncounterOver` (D5): terminal stays
   absorbing per T138 B18/B24 and T10 accounting is untouched. (b) Revive:
   re-keys, re-rounds and re-journals terminal state, and needs a new golden.
2. **How do `OnTick` pools refresh outside encounters?**
   *Recommend* (a) they do not — no encounter clock exists (B6). (b) A
   world-level clock with its own refresh pass: new hashed state and a tick
   change for a question T138 explicitly left open.
3. **History version for join/leave.** *Recommend* (a) stay at 2 with
   `Joined`/`Left` as v2 vocabulary (D6). (b) Bump to 3, moving every
   `history_v2` golden and hash.
4. **Creature tags outside encounters.** *Recommend* (a) always persist in
   `CreatureState`, but seed the encounter copy only with a granting effect
   (B4): the store is complete and both `srd_lite` goldens stay put. (b)
   Always carry into encounters too, and re-bless both `srd_lite` goldens
   (needs approval). (c) Persist grants only.
5. **Leave of the last combatant: release with or without `EncounterEnded`?**
   *Recommend* (a) with it iff the encounter was nonterminal (B14), exactly
   the despawn rule. (b) Never: leave is voluntary, release is silent like
   `end_encounter`.
6. **Join identity: existing entity or fresh spawn?** *Recommend* (a)
   existing live entity with a `CreatureState` (Interface): the outside
   world already owns the creature and `spawn` is already journaled. (b) A
   spawn-taking join: couples identity minting to admission and complicates
   replay ordering.
7. **Split.** *Recommend* (a) T139a then T139b as above. (b) One package:
   about 900 lines of `src` plus 800 of tests in a single review.

## Agent log

- 2026-10-30 (UTC) · opencode-go/muse-spark-1.3-contributor + T139 draft · Drafted the blind T139 contract (persistent store, seed/write-back, join/leave, v2 journal payloads, S-set touches, tests, split) from the T139 fact sheet against the T137/T138 hand-offs with no code touched.

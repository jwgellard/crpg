# T139 facts — `crpg-sim` persistent creature state; join and leave (ADR-0029 Follow-on 6)

Fact sheet only. Every `path:line` below exists on this branch. No
recommendations, proposed types, or opinions.

## 1. Where creature state lives today

- `World` struct (`crates/crpg-sim/src/world.rs:68-99`): one loaded area's
  state. Fields: `entities: GenerationalArena<EntityMeta>` (`:69`),
  `transforms: ComponentStore<Transform>` (`:70`), `timeline: Timeline` (`:71`),
  `events: EventQueue<SimEvent>` (`:72`), `rng: DeterministicRng` (`:73`),
  `tick: Tick` (`:74`), `combatants: ComponentStore<Combatant>` (`:82`),
  `combat: Option<CombatState>` (`:85`), `interners: Interners` (`:91`),
  `area: Option<Ulid>` (`:98`). `combat` is `None` when no encounter is
  active (`:83-85`).
- `CombatState` (`crates/crpg-sim/src/combat.rs:760-776`): `definition:
  CombatDefinition` (`:762`), `round: u32` (`:764`), `active:
  Option<EntityId>` (`:766`), `mode: EncounterMode` (`:769`), `clock:
  Option<EncounterClock>` (`:772`), `pending_mode: Option<EncounterMode>`
  (`:775`).
- `Combatant` (`crates/crpg-sim/src/combat.rs:202-223`): `placement: Ulid`
  (`:203`), `initiative: i32` (`:204`), `attributes: Vec<(String, i32)>`
  (`:205`), `health: u32` (`:206`), `max_health: u32` (`:207`), `dead: bool`
  (`:208`), `action_pool: ResourcePool` (`:209`), `extra_pools:
  Vec<ResourcePool>` (`:212`), `attached: Vec<AttachedEffect>` (`:215`),
  `order: Option<StandingOrder>` (`:218`), `tags: Vec<String>` (`:222`).
- `CombatDefinition` (`crates/crpg-sim/src/combat.rs:446-487`): encounter and
  ruleset ULIDs (`:448-450`), `ability: Ulid` (`:452`), `dice: DiceExpr`
  (`:454`), `attribute: String` (`:456`), `health_stat: String` (`:458`),
  `attribute_names: Vec<String>` (`:460`), `damage: Vec<(Outcome, u32)>`
  (`:462`), `cost: u32` (`:464`), `requires_target`/`allow_self_target`
  (`:466-468`), `outcome_table: OutcomeTable` (`:470`), `pool_id: Ulid`
  (`:472`), `pool_max: u32` (`:474`), `pool_refresh: RefreshTrigger`
  (`:476`), `abilities: Vec<AbilityDefinition>` (`:478`), `pools:
  Vec<PoolTemplate>` (`:480`), `effects: Vec<EffectDefinition>` (`:482`),
  `derived: Vec<DerivedStat>` (`:484`), `initiative: Option<InitiativeRule>`
  (`:486`).
- `AbilityDefinition` (`crates/crpg-sim/src/combat.rs:394-435`): ability ULID
  (`:396`), `dice: DiceExpr` (`:398`), `attribute: String` (`:400`),
  `defense: AbilityDefense` (`:402`), `outcome_table: OutcomeTable` (`:404`),
  `damage: Vec<(Outcome, u32)>` (`:406`), `costs: Vec<(ResourcePoolId, u32)>`
  (`:408`), `ends_turn: bool` (`:410`), `effect: Option<Ulid>` (`:412`),
  `natural_die: Option<u32>` (`:414`), `requires_target`/`allow_self_target`
  (`:416-418`), `recovery_ticks: Option<u32>` (`:422`),
  `repeats_in_real_time: bool` (`:426`), `bonus_stats: Vec<String>` (`:430`),
  `damage_dice: Vec<(Outcome, DiceExpr)>` (`:434`).
- `EffectDefinition` (`crates/crpg-sim/src/combat.rs:363-379`): `effect:
  Ulid` (`:365`), `aim: EffectAim` (`:367`), `mod_type: String` (`:369`),
  `policy: StackingPolicy` (`:371`), `modifiers: Vec<EffectModifier>` (`:373`),
  `duration_rounds: u32` (`:375`), `grants: Vec<String>` (`:378`).
- `AttachedEffect` (`crates/crpg-sim/src/combat.rs:190-196`): `effect: Ulid`
  (`:191`), `expires_round: u32` (`:195`).
- Who creates and drops them: `start_encounter`/`start_encounter_in_mode`
  (`crates/crpg-sim/src/combat.rs:1218,1229`) publish `combat` and the
  `combatants` store; `end_encounter` (`:3719`) releases them; `despawn` of
  the last combatant sets `combat = None` (`crates/crpg-sim/src/world.rs:861-862`).
  `CombatState` is `World`-owned and serialized as `combat: Option<CombatState>`
  (`world.rs:83-85`).
- What of stats, pools, effects and creature tags exists only inside an
  encounter: `Combatant.attributes` (`Vec<(String, i32)>`) holds the
  non-health `Int` stat values at encounter start only (`combat.rs:205`);
  `Combatant.action_pool`/`extra_pools` (`:209,212`) hold the runtime pool
  balances; `Combatant.attached` (`:215`) holds effect attachments with
  expiry rounds; `Combatant.tags` (`:222`) holds placement tags carried only
  when a referenced effect grants tags (T137 B19). `CombatDefinition.derived`
  (`:484`) and `initiative` (`:486`) are encounter-scoped. There is no
  `World`-level (non-encounter) storage of any creature stat, pool, effect,
  or tag: `World` has no creature component store, no stat store, and no
  effect store outside `combatants`/`combat` (`world.rs:68-99`).
- `World` has no persistent creature entity concept beyond `EntityMeta {}`
  (`world.rs:58-59`) and `Transform` (`world.rs:70`); a creature outside an
  encounter is represented only as an entity with an optional `Transform`
  (`world.rs:69-71`).

## 2. Encounter lifecycle

- Start: `start_encounter(world, spec)` (`crates/crpg-sim/src/combat.rs:1218`)
  and `start_encounter_in_mode(world, spec, mode)` (`:1229`). The latter is
  `start_encounter` delegating with `EncounterMode::TurnBased`
  (`crates/crpg-sim/src/combat.rs:1218-1227` per contract). `start_encounter`
  keeps its signature and means turn-based (T138 D3).
- Terminal states: encounter goes terminal when `active = None`
  (`advance_tracked`, `crates/crpg-sim/src/combat.rs:3359-3361` and
  `:3401-3406`); RT terminal is `rt_alive < 2` (T138 B18). A terminal
  encounter keeps `mode` and `pending_mode` until release (T138 B2).
- Release: `end_encounter(world)` (`crates/crpg-sim/src/combat.rs:3719`)
  strips combat scheduling/components, keeps entities live, emits no events,
  returns `EncounterSummary`.
- Despawn of the last combatant releases the encounter
  (`world.rs:861-862`).
- Mode: `EncounterMode::{TurnBased, RealTime}` (`crates/crpg-sim/src/combat.rs:70-78`),
  stored as `CombatState.mode` (`:769`), omitted when `TurnBased`
  (`:768`); `clock: Option<EncounterClock>` (`:772`) is `None` for TB-only.
  `start_encounter_in_mode(.., RealTime)` on a TB-only ruleset returns
  `ModeUnsupported` (T138 B4). A TB-only world writes none of the keys
  `"mode"`, `"clock"`, `"pending_mode"`, `"order"`, `"recovery_ticks"`,
  `"repeats_in_real_time"` (T138 B6).
- At start: pools at max, timeline at authored initiative keys
  (`crates/crpg-sim/AGENTS.md`), `round = 0`, `active` = timeline head,
  `roll` tag interned, incoming actor refreshed once with `TurnStart`. RT
  start keys every combatant at `0` and sets `active = None`
  (`switch_to_real_time` not used; T138 B28, `start_encounter_in_mode`).
- At end (release): `combat = None`, combatants store cleared of combat
  components but entities stay live; interners, events, RNG, tick retained
  (`crates/crpg-sim/AGENTS.md` invariant 1; `end_encounter`).
- What happens to each piece of creature state at start: `Combatant`
  created per participant in authored order with health/max_health from the
  creature prefab, attributes from the declared `Int` stats, pool balances
  at max from templates, attached empty, order None, tags per B19. At end:
  all combatant components removed (despawn of combatants not entities); the
  stats/pools/effects/tags only existed inside the encounter and are gone.

## 3. Tick order and journaled World operations

- `tick(world)` (`crates/crpg-sim/src/tick.rs:22-24`) delegates to
  `tick_tracked(world)` (`:31-36`), which calls `world.advance_tick()`
  (`:32`), then `run_systems(world, &mut facts)` (`:34`).
- `run_systems` (`tick.rs:54-59`): stage 2 `encounter_clock_system` →
  `clock_stage` (`tick.rs:65-67`, `combat.rs:3572`); stage 3 `timeline_system`
  → no-op loop then `execution_stage` (`tick.rs:77-82`,
  `combat.rs:3616`).
- `end_turn(world)` (`tick.rs:45-48`): pops timeline head, never re-queues.
- Journaled World operations (ADR-0029 T16): `tick`, command
  (`perform_action`), mode request (`request_mode`), start
  (`start_encounter`/`start_encounter_in_mode`), end (`end_encounter`),
  `spawn`, `despawn`. History wrapper records them:
  `HistoryWorld::tick` (`crates/crpg-sim/src/history.rs:542-584`),
  `perform_action` (`:633-654`), `request_mode` (`:677-692`),
  `start_encounter`/`start_encounter_in_mode` (`:590-621`),
  `end_encounter` (`:660-675`), `spawn` (`:496-508`), `despawn`
  (`:517-530`).
- `HISTORY_VERSION = 2` (`crates/crpg-sim/src/history.rs:64`); v1 = 1
  (`:65`? no, `const HISTORY_VERSION_V1: u32 = 1` at `:332`); a new wrapper
  starts at 1 (`new` sets `version: HISTORY_VERSION_V1`, `:426`).
- `HistoryEvent` variants include `Spawned`, `Despawned`, `ActionResolved`,
  `Died`, `TurnStarted`, `EncounterEnded` (legacy) plus v2 `RoundStarted`,
  `ModeChanged`, `OrderEnded` (`history.rs` `HistoryEvent` around `:75-124`
  and the additions at `:334-351` of T138).
- `TransitionFacts` (`crates/crpg-sim/src/combat.rs:301-306`): `turn_started`,
  `encounter_ended`, `mode_changed`, `round_started`. `TickFacts`
  (`:315-320`): `transitions: TransitionFacts`, `events: Vec<TickEvent>`.
  `TickEvent::{Resolved(ActionOutcome), OrderEnded { actor }}`
  (`:307-314`).
- `advance_tracked(world)` (`crates/crpg-sim/src/combat.rs:3312-3410`)
  returns `TransitionFacts`; used by `perform_action` and despawn.
- `push_transition`/`push_lead` assemble emission order
  (`history.rs:548-581`).

## 4. Initiative, rounds, rollover and RT keying (`due`)

- `InitiativeKey(pub i32)` (`crates/crpg-sim/src/timeline.rs`); timeline is
  `BTreeMap<(key, id)>` with replace-on-insert and O(n) removal.
- `Combatant.initiative: i32` (`crates/crpg-sim/src/combat.rs:204`), accessor
  `initiative()` (`:232-234`).
- TB: timeline holds one entry per alive combatant keyed by
  `InitiativeKey(state.initiative)` (`advance_tracked` rollover branch
  `:3349` inserts them; `tb_round_start` `:3261-3269`).
- Round rollover: when timeline empty with >=2 alive, `tb_round_start`
  (`crates/crpg-sim/src/combat.rs:3244-3297`) does `round + 1`, `RoundStart`
  then `TurnStart` refresh of every alive combatant, attachment expiry
  (`expires_round > round`), re-insert alive at stored initiative, head with
  `TurnStart`, `active = Some(head)`.
- `advance_tracked` (`:3312-3410`): if timeline empty and alive>=2, TB
  rollover (with clock advance if RT-capable); if timeline nonempty and
  alive>=2, set head + `TurnStart`; else terminal.
- RT: `StandingOrder { ability: Ulid, target: EntityId, repeat: bool }`
  (`crates/crpg-sim/src/combat.rs:155-164`); `Combatant.order:
  Option<StandingOrder>` (`:218`).
- RT timeline: exactly one entry per alive combatant (`T138 B11`); key is
  `0` after RT start, current `elapsed` after TB→RT switch, and
  `min(elapsed + recovery, RT_CLOCK_MAX)` after execution, where
  `recovery = recovery_ticks.unwrap_or(round_ticks)` (T138 B11).
- `rt_round_start` (`crates/crpg-sim/src/combat.rs:3484-3511`): `round + 1`,
  per alive combatant in ascending `EntityId` a `RoundStart` then `TurnStart`
  refresh (primary first, then extras), attachment expiry, `active` stays
  `None`.
- `clock_stage` (`:3572`) advances `elapsed` by 1 in RT (bounded by
  `RT_CLOCK_MAX`), delivers `OnTick` refreshes via `deliver_clock_advance`
  (`:3460-3477`), and at a boundary (`elapsed % round_ticks == 0`) applies a
  pending RT→TB switch (`switch_to_turn_based`, `:3537-3552`) or runs
  `rt_round_start`.
- `execution_stage` (`:3616`) collects entries with key `<= elapsed` in
  ascending `(key, EntityId)` before acting; validates and calls
  `resolve_use`; re-keys alive actors to `clock + recovery_ticks`; clears a
  one-shot order, keeps a repeating one; invalid orders clear to `None` and
  journal `OrderEnded`.
- RT terminal: `rt_alive < 2` (T138 B18, D15).
- `request_mode(world, mode)` (`crates/crpg-sim/src/combat.rs:3443-3453`)
  sets/replaces/clears `pending_mode`; never changes `mode` immediately.
- `switch_to_real_time` (`:3517-3532`): mode flips to RT, every alive
  combatant keyed at `elapsed`, `active = None`, then `rt_round_start`.
- `switch_to_turn_based` (`:3537-3552`): clears all orders, mode flips to TB,
  then `tb_round_start`.
- `RT_CLOCK_MAX = 2_147_483_647` (`crates/crpg-sim/src/combat.rs:82`).
- `EncounterClock { round_ticks: u32, elapsed: u64 }`
  (`crates/crpg-sim/src/combat.rs:85-90`).
- `Combatant.order` accessor (`crates/crpg-sim/src/combat.rs:271-274`).

## 5. What T137 and T138 explicitly hand to T139

- `tasks/T137.md:815-818` (verbatim):
  > - **T139** (persistent state): owns non-`Int` stats and creature tags
  >   outside encounters. If it needs either inside an encounter, it adds the
  >   carriage with its consumer (D3, D13). D6's scratch-interner rule extends
  >   to its handles.
- `tasks/T137.md:898-901` (verbatim):
  >    until T139.
  > 4. **Creature tags** (D13). *Recommend* (a): carry them only in encounters
  >    that reference a granting effect. This keeps T136b's mapping and every
  >    golden. (b) Defer creature tags to T139 and project grants only. (c)
  >    Always carry them, and re-bless both `srd_lite` goldens.
- `tasks/T138.md:602-603` (verbatim):
  > 31. **B31 (T14).** No join operation is added. The join rules go to T139
  >     (see "Out of scope").
- `tasks/T138.md:851-858` (verbatim):
  > - **T139 (persistent state, join and leave)** owns T13 and T14:
  >   - TB first round: the joiner is inserted at its initiative, never ahead
  >     of `active`.
  >   - TB later rounds: the joiner enters at the next rollover.
  >   - RT: the joiner is keyed at `elapsed` with `order: None`.
  >   - T139 decides whether a join into a terminal RT encounter (D15: derived,
  >     so a join could revive it) is rejected. It also decides how `OnTick`
  >     pools refresh outside encounters, where there is no encounter clock.

## 6. Save/load and replay

- `World` derives `Serialize` (`world.rs:67`) and a validated `Deserialize`
  (`world.rs:128-...`). Serde shape `WorldRepr` (`world.rs:110-126`):
  entities, transforms, timeline, events, rng, tick, combatants, combat,
  interners, area.
- `CombatState`/`Combatant`/`CombatDefinition` derive
  `Serialize`/`Deserialize` (combat.rs:201, 445, 760) with hand-written
  serializers for `EffectDefinition` (`combat.rs:538`) and
  `CombatDefinition` (`combat.rs:392-426` per contract).
- Absence-compat: non-combat worlds omit `combatants`/`combat`/`interners`/
  `area` (`world.rs:81-98`); TB-only combat omits mode/clock/pending_mode/
  order/recovery_ticks/repeats_in_real_time (T138 B6).
- Load coherence: `World::deserialize` validates dangling transform/combatant/
  timeline/active, combat coherence (roll tag, attribute/health, pools, costs,
  scheduling) at `world.rs:128-...` and T137 B23 messages.
- Replay/goldens pinning them:
  - `crates/crpg-sim/tests/goldens/history_v1.x86_64-pc-windows-msvc.txt` and
    `...-linux-gnu.txt` (`tests/history_golden.rs:28`).
  - `crates/crpg-sim/tests/goldens/history_v2.x86_64-pc-windows-msvc.txt` and
    `...-linux-gnu.txt` (T138b).
  - `crates/crpg-sim/tests/goldens/surface_v1.x86_64-pc-windows-msvc.txt` and
    `...-linux-gnu.txt` (T137c).
  - `crates/crpg-testkit/goldens/*` six files (`combat_basic`,
    `replay_basic`, `srd_lite` × two targets).
  - `crpg-cli` replay fixtures and `crpg-server`/`crpg-net` hash assertions.
- History wrapper persists `version`, `world`, `acknowledged`, `next_seq`,
  `pending` (`history.rs` struct `:404-419`); adjacent-tag payloads with
  `type` before `value`; loader accepts version 1 or 2
  (`history.rs` loader around `:1446-1448` per T138).
- `state_hash`/`history_hash` code unchanged; no exclusions
  (`crates/crpg-sim/src/hash.rs:37-49`).

## 7. Existing tests that pin encounter start/end, mode and history

- `tests/rt_mode.rs`: `r1_start_modes_publish_their_shapes`,
  `r1_release_drops_mode_clock_and_pending`,
  `r1_terminal_rt_keeps_mode_and_pending`,
  `r1_terminal_tb_keeps_mode_and_pending`,
  `r1_encounter_active_beats_mode_unsupported` (B1, B2, B28, B30);
  `r2_clock_and_ability_fields_are_published`, `r2_zero_round_ticks_panics`,
  `r2_zero_recovery_ticks_panics`, `r2_legacy_shape_save_load` (B3, B5, B7);
  `r3_rt_start_on_tb_only_is_rejected_cleanly`, `r3_request_matrix`,
  `r3_request_never_applies_mid_round` (B4, B23, B24); `r4_tb_only_worlds_write_no_new_keys` (B6).
- `tests/rt_tick.rs`: `r5` (clock advance/freeze), `r6` (OnTick delivery),
  `r7` (TB parity), `r8` (RT timeline/keys/boundary), `r9` (execution),
  `r13` (non-participant survival), `r15` (replay determinism).
- `tests/rt_orders.rs`: `r10` (command precedence, legal_actions).
- `tests/rt_switch.rs`: `r11` (mode switching).
- `tests/rt_persistence.rs`: `r14` (RT bytes, load checks, despawn).
- `tests/history_v2.rs`: `r16` (payload encodings), `r17` (emission order),
  `r18` (version stickiness), `r19` (caps).
- `tests/history_v2_golden.rs` with `tests/support/history_v2_schedule.rs`
  and `examples/generate_history_v2_golden.rs`: seed-42 RT golden.
- `tests/surface_golden.rs` with `tests/support/surface_schedule.rs` and
  `examples/generate_surface_golden.rs`: seed-42 surface golden.
- `tests/combat.rs`, `tests/combat_multi.rs`, `tests/combat_persistence.rs`,
  `tests/combat_costs.rs`, `tests/combat_self_kill.rs`: encounter start/end,
  action resolution, damage, death.
- `tests/history.rs`, `tests/history_end_encounter.rs`: history v1
  operations and release.
- `tests/world.rs`: no-dangling invariants on generated worlds.

## 8. Constraints that bind T139

- ADR-0029 T12: one encounter per `World` (`crates/crpg-sim/src/combat.rs`
  `EncounterActive` check); a second engagement joins the existing encounter;
  concurrent encounters out of scope (Open question 7).
- ADR-0029 T13 (join): TB first round insert at initiative, never ahead of
  active; TB later rounds enter at next rollover; RT keyed at `elapsed` with
  `order: None` (`tasks/T138.md:851-858`).
- ADR-0029 T14: T139 decides whether a join into a terminal RT encounter is
  rejected, and how `OnTick` pools refresh outside encounters where there is
  no encounter clock (`tasks/T138.md:856-858`).
- ADR-0029 T15: AI outside tick order; host runs AI between ticks.
- ADR-0029 T16: every World change is a journaled operation (tick, command,
  mode request, start, end, join); pause/auto-pause/timers/AI live in host.
- ADR-0029 Open question 7: one encounter per area; fights merge; concurrent
  encounters would make `CombatState` a collection (separate S-set change).
- S-set definition (`docs/AGENT_TEAM_PLAN.md:131-134`): `crpg-contracts`,
  `World` or tick order, schema versions and migrations, GDExtension
  boundary, workspace `Cargo.toml`, `deny.toml`, `.github/workflows/`.
  `World` struct is S-set (`docs/IMPLEMENTATION_PLAN.md` §1.4).
- `crates/crpg-sim/AGENTS.md`: no `HashMap`/`HashSet`; no `f32`/`f64`
  outside `Transform`; `#![forbid(unsafe_code)]`; dependency direction; T137
  scratch-interner rule; one legality source; combat saves fail fast at load.
- `docs/architecture/crpg-sim.md`: tick order is serial territory; no policy
  in timeline module; `state_hash` exclusions none.
- Root `AGENTS.md`: determinism exact-build; no cross-platform hash
  comparison; re-baselines require review; platform invariant (Windows
  primary, Linux fully supported for headless/tests); every crate root counts
  for `#![forbid(unsafe_code)]` including `src/bin/*.rs`; `deps.py` ALLOWED
  authoritative.
- T137 D3/D13: non-`Int` stats and creature tags outside encounters are
  T139's; if needed inside an encounter, T139 adds carriage with its
  consumer; D6 scratch-interner rule extends to its handles.
- T137 D6: scratch interner per operation; world interner is hashed;
  T139 handles follow the same rule.
- T138 B31: no join operation added by T138; join rules go to T139.

## 9. Open facts

- No persistent creature state storage exists anywhere in `crpg-sim`
  outside an encounter: no `Creature`/`Actor` component store, no stat/pool/
  effect store on `World`, no `World`-level tag set. Searched
  `crates/crpg-sim/src/world.rs`, `combat.rs`, `store.rs`, `area.rs`.
- No join/leave/spawn-into-encounter function exists in `crpg-sim` today;
  searched `crates/crpg-sim/src` for `join`/`leave`/`add_participant`.
- No `OnTick` pool refresh mechanism exists outside an encounter clock;
  `deliver_clock_advance` (`combat.rs:3460-3477`) is encounter-scoped only.
- No terminal-RT-join decision or revival rule exists; T138 leaves it to T139.
- No non-`Int` stat storage or derived-stat evaluation outside encounters;
  `IntFormula`/`DerivedStat` are `CombatDefinition`-scoped
  (`combat.rs:185-220`).
- `crpg-data` creature/prefab shapes and how stats/pools/effects/tags are
  authored outside encounters were not examined here (out of scope for this
  fact sheet; T139 will need them).
- `crpg-persist`/save format beyond `World` serde was not examined.
- Whether `World` entities outside encounters can carry a creature identity
  (only `EntityMeta {}` exists, `world.rs:58-59`) is an open fact.

## Agent log

- 2026-10-30 (UTC) · opencode-go/beam-free + T139-facts · Gathered existing crpg-sim shapes, lifecycle, tick/journal, initiative/RT keying, T137/T138 hand-offs, save/replay and constraints with citations for the T139 contract.
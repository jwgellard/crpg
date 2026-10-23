# T138 facts — `crpg-sim` encounter mode and RT policy (ADR-0029 Follow-on 5)

Fact sheet for the T138 contract. Every `path:line` below was read on this
branch (`task/T138-facts`). Scope: ADR-0029 §"2. Time model" items T1–T16
(`docs/adr/0029-kernel-range-and-time-model.md` lines 179–278) and N5
(line 347). Code scope is `crates/crpg-sim/src/` plus `src/history/`;
test scope is `crates/crpg-sim/tests/`, `crates/crpg-testkit/`, and
`crates/crpg-cli/tests/`.

Base note: the T136a data fields **are** on this base —
`round_ticks` (`crates/crpg-data/src/combat.rs:245`), `recovery_ticks`
(`crates/crpg-data/src/combat.rs:318`), `repeats_in_real_time`
(`crates/crpg-data/src/combat.rs:321`) — so each section cites them directly
rather than falling back to `tasks/T136.md`.

## T1

> "Mode is per encounter."

- `CombatState` (`crates/crpg-sim/src/combat.rs:532`) — has only
  `definition`, `round` (`crates/crpg-sim/src/combat.rs:536`), `active`
  (`crates/crpg-sim/src/combat.rs:538`); no `mode` field exists yet.
- `World.combat: Option<CombatState>` (`crates/crpg-sim/src/world.rs:82`) —
  the encounter slot a mode would live in; `None` when no encounter active.
- `World.combatants: ComponentStore<Combatant>`
  (`crates/crpg-sim/src/world.rs:79`) — per-combatant state store.
- Tests: `crates/crpg-sim/tests/combat.rs`, `crates/crpg-sim/tests/combat_persistence.rs`;
  `crates/crpg-testkit/tests/combat_replay.rs`.
- T136a data: `Ruleset.round_ticks` (`crates/crpg-data/src/combat.rs:245`);
  `Ability.recovery_ticks` (`crates/crpg-data/src/combat.rs:318`);
  `Ability.repeats_in_real_time` (`crates/crpg-data/src/combat.rs:321`).

## T2

> "RT capability is data."

- `EncounterSpec.ruleset: &AuthoredRuleset`
  (`crates/crpg-sim/src/combat.rs:561`) — the authored ruleset the adapter
  reads; `start_encounter` (`crates/crpg-sim/src/combat.rs:921`) validates it
  all-or-nothing with no RT-capability check today.
- `CombatDefinition` (`crates/crpg-sim/src/combat.rs:263`) — the persisted
  symbolic definition; carries no `round_ticks` copy today.
- `AbilityDefinition` (`crates/crpg-sim/src/combat.rs:227`) — per-ability
  runtime definition; carries no `recovery_ticks` / `repeats_in_real_time`
  copy today.
- Data read-time guards (consumed at adapter time, not sim-owned):
  `round_ticks == Some(0)` fails (`crates/crpg-data/src/combat.rs:774`);
  `recovery_ticks == Some(0)` fails (`crates/crpg-data/src/combat.rs:821`).
- Tests: `crates/crpg-sim/tests/combat.rs`; data-side
  `crates/crpg-data` combat suites (out of cited scope, listed only).
- T136a data: `round_ticks` (`crates/crpg-data/src/combat.rs:245`);
  `recovery_ticks` (`crates/crpg-data/src/combat.rs:318`);
  `repeats_in_real_time` (`crates/crpg-data/src/combat.rs:321`).

## T3

> "Encounter clock."

- `World.tick: Tick` (`crates/crpg-sim/src/world.rs:71`) with getter
  `World::tick` (`crates/crpg-sim/src/world.rs:699`) — the only clock in the
  world; no encounter `clock: u64` exists yet.
- `World::advance_tick` (`crates/crpg-sim/src/world.rs:711`) — saturating +1;
  called by `tick` (`crates/crpg-sim/src/tick.rs:21`).
- `CombatState.round: u32` (`crates/crpg-sim/src/combat.rs:536`) — persisted
  saturating `u32`, the counter TB rollover advances today.
- Tests: `crates/crpg-sim/tests/tick.rs`
  (`counter_advances_exactly_one_per_tick`);
  `crates/crpg-testkit/tests/replay.rs` (hash-per-tick sequences).
- T136a data: `round_ticks` (`crates/crpg-data/src/combat.rs:245`).

## T4

> "The TB policy is unchanged (ADR-0013 Decision 5, ADR-0016): one entry per live combatant keyed by initiative, the head acts, an accepted turn-ending action pops it, an empty timeline with two or more alive rolls the round over."

- `Timeline` (`crates/crpg-sim/src/timeline.rs:42`) with
  `entries: BTreeMap<(InitiativeKey, EntityId), ()>`
  (`crates/crpg-sim/src/timeline.rs:43`); `insert`
  (`crates/crpg-sim/src/timeline.rs:95`); `remove`
  (`crates/crpg-sim/src/timeline.rs:101`); ascending `iter`
  (`crates/crpg-sim/src/timeline.rs:128`).
- `end_turn` (`crates/crpg-sim/src/tick.rs:33`) — pops the timeline head,
  never re-queues, never despawns.
- `perform_action` (`crates/crpg-sim/src/combat.rs:1786`) with turn advance
  (`crates/crpg-sim/src/combat.rs:2149`): next head + `TurnStart` refresh
  (`crates/crpg-sim/src/combat.rs:2201`), round rollover `round + 1`
  (`crates/crpg-sim/src/combat.rs:2169`) with `RoundStart` refresh
  (`crates/crpg-sim/src/combat.rs:2172`), terminal otherwise.
- `CombatAction::EndTurn` (`crates/crpg-sim/src/combat.rs:619`) — advances
  with no spend/draw/damage/effect.
- Tests: `crates/crpg-sim/tests/tick.rs`
  (`end_turn_pops_ascending_and_never_despawns`);
  `crates/crpg-sim/tests/combat.rs`; `crates/crpg-sim/tests/combat_multi.rs`.
- T136a data: none consumed (TB-only path reads no RT field).

## T5

> "RT keys."

- `InitiativeKey(pub i32)` (`crates/crpg-sim/src/timeline.rs:25`) — the
  existing key type; `Timeline` orders by `(key, id)`.
- `Combatant.initiative: i32` (`crates/crpg-sim/src/combat.rs:76`) with
  accessor (`crates/crpg-sim/src/combat.rs:97`) — authored key used at
  `start_encounter` timeline publication.
- No ready-clock key, no clock bound, no fail-closed behavior exists yet.
- Tests: `crates/crpg-sim/tests/world.rs` (timeline ordering/dangling
  invariants); `crates/crpg-sim/tests/tick.rs`.
- T136a data: none consumed.

## T6

> "RT round boundary (tick stage 2)."

- `run_systems` (`crates/crpg-sim/src/tick.rs:43`) — hand-written list, today
  exactly `[timeline_system]`; no stage-2 step exists.
- `timeline_system` (`crates/crpg-sim/src/tick.rs:55`) — iterates standing
  entries applying no effect; standing order preserved, no reorder, no rekey.
- Round-start refresh precedent: `RoundStart`
  (`crates/crpg-sim/src/combat.rs:2172`) and `TurnStart`
  (`crates/crpg-sim/src/combat.rs:2201`) pool refreshes, ascending
  `EntityId` iteration is in `legal_actions`
  (`crates/crpg-sim/src/combat.rs:1727`), not in combat advance.
- `Journal`/history has no `RoundStarted` event; `HistoryEvent`
  (`crates/crpg-sim/src/history.rs:77`) lists today's six variants only.
- Tests: `crates/crpg-sim/tests/tick.rs`
  (`real_time_ticks_keep_standing_order`); `crates/crpg-sim/tests/history.rs`.
- T136a data: `round_ticks` (`crates/crpg-data/src/combat.rs:245`).

## T7

> "RT execution (tick stage 3, the `timeline_system` body)."

- `timeline_system` (`crates/crpg-sim/src/tick.rs:55`) — the body T7 would
  extend; today the per-entry loop applies nothing.
- `validate_action` (`crates/crpg-sim/src/combat.rs:1554`) — the admission
  path a due-entry check would reuse; it enforces the active-turn check
  (`OutOfTurn`, `crates/crpg-sim/src/combat.rs:683`) that RT must skip for
  non-head actors.
- `perform_action` resolution path (`crates/crpg-sim/src/combat.rs:1786`):
  named stream `COMBAT_ROLL_STREAM`
  (`crates/crpg-sim/src/combat.rs:55`), interned tag
  (`crates/crpg-sim/src/combat.rs:1841`), cost spend `spend_costs`
  (`crates/crpg-sim/src/combat.rs:2084`), damage, `Died`
  (`crates/crpg-sim/src/event.rs:42`), effect attach `attach_effect`
  (`crates/crpg-sim/src/combat.rs:2109`).
- `Combatant.action_pool` (`crates/crpg-sim/src/combat.rs:81`) and
  `extra_pools` (`crates/crpg-sim/src/combat.rs:84`) — balances a re-key
  would leave untouched (recovery is a key, not a refresh).
- Death removes the scheduling entry at once (see T4 advance,
  `crates/crpg-sim/src/combat.rs:2149`).
- Tests: `crates/crpg-sim/tests/combat.rs`;
  `crates/crpg-sim/tests/combat_multi.rs`;
  `crates/crpg-sim/tests/legal_actions.rs`.
- T136a data: `recovery_ticks` (`crates/crpg-data/src/combat.rs:318`);
  `repeats_in_real_time` (`crates/crpg-data/src/combat.rs:321`).

## T8

> "Orders."

- `Combatant` (`crates/crpg-sim/src/combat.rs:74`) — fields are `placement`,
  `initiative`, `attributes`, `health`, `max_health`, `dead`, `action_pool`,
  `extra_pools`, `attached`; no `order` field exists yet.
- `CombatAction` (`crates/crpg-sim/src/combat.rs:608`) — only `UseAbility`
  (`crates/crpg-sim/src/combat.rs:610`) and `EndTurn`
  (`crates/crpg-sim/src/combat.rs:619`); no `Stop` command.
- `legal_actions` (`crates/crpg-sim/src/combat.rs:1727`) — enumerates for one
  actor holding the turn; `LegalActionsError`
  (`crates/crpg-sim/src/combat.rs:1679`); bound `MAX_LEGAL_ACTIONS`
  (`crates/crpg-sim/src/combat.rs:53`).
- `AttachedEffect` (`crates/crpg-sim/src/combat.rs:63`) — the existing
  per-combatant attached-item shape (`effect`, `expires_round`), the nearest
  structural precedent for an attached order.
- Tests: `crates/crpg-sim/tests/legal_actions.rs`;
  `crates/crpg-sim/tests/combat_multi.rs`.
- T136a data: `repeats_in_real_time`
  (`crates/crpg-data/src/combat.rs:321`).

## T9

> "Switching."

- `CombatState` (`crates/crpg-sim/src/combat.rs:532`) — no `pending_mode`
  field exists yet.
- `CombatError` (`crates/crpg-sim/src/combat.rs:652`) — no
  mode-request/switch error variant exists yet.
- TB rollover site that would host one direction:
  `crates/crpg-sim/src/combat.rs:2169`; T6 boundary would host the other.
- Tests: `crates/crpg-sim/tests/combat_persistence.rs` (state shape
  coverage); none for switching — none found.
- T136a data: `round_ticks` (`crates/crpg-data/src/combat.rs:245`).

## T10

> "Applying a switch."

- Round-start precedent: `RoundStart` refresh of all alive
  (`crates/crpg-sim/src/combat.rs:2172`); `expires_round: u32`
  (`crates/crpg-sim/src/combat.rs:66`) with `saturating_add`
  (`crates/crpg-sim/src/combat.rs:2124`) — effect expiry by round as now.
- `start_encounter` publication (`crates/crpg-sim/src/combat.rs:1402` roll
  tag; `crates/crpg-sim/src/combat.rs:1409` incoming-actor `TurnStart`)
  — the timeline-build precedent a rebuild would follow.
- `HistoryEvent` (`crates/crpg-sim/src/history.rs:77`) — no `ModeChanged`;
  journal path is `commit` (`crates/crpg-sim/src/history.rs:702`).
- Tests: `crates/crpg-sim/tests/history.rs`; none counting refreshes
  across toggles — none found.
- T136a data: none consumed beyond T9's.

## T11

> "Initial mode."

- `start_encounter(world, spec)` (`crates/crpg-sim/src/combat.rs:921`) —
  takes no mode input today; `EncounterSpec`
  (`crates/crpg-sim/src/combat.rs:558`) carries encounter, ruleset,
  abilities, outcome tables, placements, creatures, effects — no session
  setting.
- `HistoryWorld::start_encounter`
  (`crates/crpg-sim/src/history.rs:523`) — the journaled wrapper, also
  mode-free today.
- Tests: `crates/crpg-sim/tests/combat.rs` (adapter cases);
  `crates/crpg-sim/tests/history.rs`.
- T136a data: `round_ticks` (`crates/crpg-data/src/combat.rs:245`)
  (capability source, unread by sim today).

## T12

> "Split parties."

- `World::spawn` (`crates/crpg-sim/src/world.rs:609`) and `World::despawn`
  (`crates/crpg-sim/src/world.rs:629`) — non-participant entity ops,
  independent of combat scheduling.
- `tick` (`crates/crpg-sim/src/tick.rs:21`) — advances counter and runs
  systems for the whole world; combat turn logic lives only in
  `start_encounter`/`perform_action`, never in `tick`/`run_systems`/`end_turn`.
- `Transform { position, velocity }`
  (`crates/crpg-sim/src/transform.rs:20`) — `f32` spatial only.
- Tests: `crates/crpg-sim/tests/tick.rs`; `crates/crpg-sim/tests/world.rs`.
- T136a data: none consumed.

## T13

> "One encounter per World."

- `CombatError::EncounterActive`
  (`crates/crpg-sim/src/combat.rs:656`) — second `start_encounter` fails;
  checked first in pinned precedence (`crates/crpg-sim/src/combat.rs:921`).
- `World.combat` (`crates/crpg-sim/src/world.rs:82`) — single slot; no
  second-encounter structure.
- `end_encounter` (`crates/crpg-sim/src/combat.rs:2288`) — releases the one
  encounter; `EncounterSummary` (`crates/crpg-sim/src/combat.rs:595`).
- Tests: `crates/crpg-sim/tests/combat.rs` (EncounterActive cases);
  `crates/crpg-sim/tests/history_end_encounter.rs`.
- T136a data: none consumed.

## T14

> "Joining (when the join operation exists, Follow-on 6)."

- No join operation exists: `World::spawn`
  (`crates/crpg-sim/src/world.rs:609`) mints non-combat entities only;
  `start_encounter` (`crates/crpg-sim/src/combat.rs:921`) publishes the full
  roster at once; `Timeline::insert`
  (`crates/crpg-sim/src/timeline.rs:95`) is the primitive a join would use.
- `CombatState.active: Option<EntityId>`
  (`crates/crpg-sim/src/combat.rs:538`) — the head a TB join must not precede.
- Tests: none for joining — none found.
- T136a data: none consumed.

## T15

> "AI is outside tick order."

- `run_systems` (`crates/crpg-sim/src/tick.rs:43`) — no AI stage; spec §10
  stage order, one system.
- `legal_actions` (`crates/crpg-sim/src/combat.rs:1727`) — the read-only
  enumeration a host-side AI would call between ticks; changes nothing.
- `HistoryWorld::perform_action`
  (`crates/crpg-sim/src/history.rs:555`) — commands journal through the
  typed wrapper; `read_after` (`crates/crpg-sim/src/history.rs:637`) pages
  at most `MAX_HISTORY_PAGE` (`crates/crpg-sim/src/history.rs:60`).
- Tests: `crates/crpg-sim/tests/legal_actions.rs`.
- T136a data: none consumed.

## T16

> "Exact-build replay."

- `state_hash` (`crates/crpg-sim/src/hash.rs:37`) — BLAKE3 over canonical
  JSON, queue bytes included, no exclusions; finite-float assert
  (`crates/crpg-sim/src/hash.rs:40`).
- `history_hash` (`crates/crpg-sim/src/history.rs:773`) — covers the full
  wrapper; `HISTORY_VERSION` (`crates/crpg-sim/src/history.rs:64`) is 1.
- `HistoryWorld::tick` (`crates/crpg-sim/src/history.rs:502`) — tick is a
  journaled operation; `HistoryWorld::perform_action`
  (`crates/crpg-sim/src/history.rs:555`) journals commands.
- Integer-only encounter state: `round: u32`
  (`crates/crpg-sim/src/combat.rs:536`), `initiative: i32`
  (`crates/crpg-sim/src/combat.rs:76`), `expires_round: u32`
  (`crates/crpg-sim/src/combat.rs:66`); `Transform` is the only float
  (`crates/crpg-sim/src/transform.rs:20`), spatial only.
- Tests: `crates/crpg-sim/tests/tick.rs`
  (`hash_sequence_identity_over_10k_ticks`,
  `queue_and_tick_bytes_are_hashed`, `state_hash_rejects_nan`);
  `crates/crpg-sim/tests/history_golden.rs` with
  `crates/crpg-sim/tests/goldens/history_v1.x86_64-pc-windows-msvc.txt` and
  `crates/crpg-sim/tests/goldens/history_v1.x86_64-unknown-linux-gnu.txt`;
  `crates/crpg-sim/tests/history.rs`; `crates/crpg-testkit/tests/replay.rs`;
  `crates/crpg-testkit/tests/combat_replay.rs`;
  `crates/crpg-testkit/tests/replay_golden.rs`;
  `crates/crpg-testkit/tests/srd_replay.rs`.
- `crates/crpg-cli/tests/replay.rs` exercises golden-line fixtures
  (e.g. line 113) — no direct `state_hash`/`history_hash` use found.
- T136a data: none consumed (integers already; RT adds no float — nothing
  to change).

## N5

> "`History` gains `RoundStarted`, `ModeChanged` and `OrderEnded`."

- `HistoryEvent` (`crates/crpg-sim/src/history.rs:77`) — today's variants:
  `Spawned` (`crates/crpg-sim/src/history.rs:79`), `Despawned`
  (`crates/crpg-sim/src/history.rs:83`), `Died`
  (`crates/crpg-sim/src/history.rs:89`), `ActionResolved`
  (`crates/crpg-sim/src/history.rs:94`), `TurnStarted`
  (`crates/crpg-sim/src/history.rs:111`), `EncounterEnded`
  (`crates/crpg-sim/src/history.rs:118`); none of the three N5 variants
  exists yet.
- `HISTORY_VERSION: u32 = 1` (`crates/crpg-sim/src/history.rs:64`); version
  checked on load (`crates/crpg-sim/src/history.rs:1446`); wrapper `version`
  field (`crates/crpg-sim/src/history.rs:380`).
- Payload string cap `MAX_HISTORY_STRING_BYTES`
  (`crates/crpg-sim/src/history.rs:62`) enforced at commit
  (`crates/crpg-sim/src/history.rs:710`) and at decode
  (`crates/crpg-sim/src/history.rs:1087`,
  `crates/crpg-sim/src/history.rs:1318`).
- Tests: `crates/crpg-sim/tests/history.rs`;
  `crates/crpg-sim/tests/history_golden.rs` (v1 goldens above);
  `crates/crpg-sim/tests/support/history_schedule.rs` (shared schedule).
- T136a data: none consumed.

## Hashed state

`state_hash` (`crates/crpg-sim/src/hash.rs:37`) hashes the whole `World`
with no exclusions. `World` fields (`crates/crpg-sim/src/world.rs:65`):
`entities` (`crates/crpg-sim/src/world.rs:66`),
`transforms` (`crates/crpg-sim/src/world.rs:67`),
`timeline` (`crates/crpg-sim/src/world.rs:68`),
`events` (`crates/crpg-sim/src/world.rs:69`),
`rng` (`crates/crpg-sim/src/world.rs:70`),
`tick` (`crates/crpg-sim/src/world.rs:71`),
`combatants` (`crates/crpg-sim/src/world.rs:79`),
`combat: Option<CombatState>` (`crates/crpg-sim/src/world.rs:82`),
`interners` (`crates/crpg-sim/src/world.rs:88`),
`area` (`crates/crpg-sim/src/world.rs:95`).
`CombatState` fields (`crates/crpg-sim/src/combat.rs:532`): `definition`
(`crates/crpg-sim/src/combat.rs:534`), `round`
(`crates/crpg-sim/src/combat.rs:536`), `active`
(`crates/crpg-sim/src/combat.rs:538`).
Absence-skip shapes (`skip_serializing_if`) are compatibility, not
exclusions: `combatants` (`crates/crpg-sim/src/world.rs:78`),
`combat` (`crates/crpg-sim/src/world.rs:81`),
`interners` (`crates/crpg-sim/src/world.rs:87`),
`area` (`crates/crpg-sim/src/world.rs:94`).
`history_hash` (`crates/crpg-sim/src/history.rs:773`) covers the complete
`HistoryWorld` wrapper: `version` (`crates/crpg-sim/src/history.rs:380`),
world, `acknowledged`/`next_seq`, pending envelopes.

## History today

`HistoryEvent` (`crates/crpg-sim/src/history.rs:77`): `Spawned`
(`crates/crpg-sim/src/history.rs:79`), `Despawned`
(`crates/crpg-sim/src/history.rs:83`), `Died`
(`crates/crpg-sim/src/history.rs:89`), `ActionResolved`
(`crates/crpg-sim/src/history.rs:94`) with symbolic `outcome` text and
`damage`, `TurnStarted` (`crates/crpg-sim/src/history.rs:111`, round widened
to `u64`), `EncounterEnded` (`crates/crpg-sim/src/history.rs:118`).
Version tag is `HISTORY_VERSION = 1`
(`crates/crpg-sim/src/history.rs:64`); envelopes are adjacent-tagged
`type`/`value` payloads under `HistoryEnvelope`
(`crates/crpg-sim/src/history.rs:132`) with manual duplicate-rejecting
decode (`crates/crpg-sim/src/history.rs:141`); string payloads capped at 256
bytes (`crates/crpg-sim/src/history.rs:62`).
Typed mutations: `spawn` (`crates/crpg-sim/src/history.rs:461`),
`despawn` (`crates/crpg-sim/src/history.rs:482`), `tick`
(`crates/crpg-sim/src/history.rs:502`), `start_encounter`
(`crates/crpg-sim/src/history.rs:523`), `perform_action`
(`crates/crpg-sim/src/history.rs:555`), `end_encounter`
(`crates/crpg-sim/src/history.rs:588`); reads `read_after`
(`crates/crpg-sim/src/history.rs:637`), `acknowledge`
(`crates/crpg-sim/src/history.rs:677`).

## Tick order today

One tick (`crates/crpg-sim/src/tick.rs:21`): `advance_tick`
(`crates/crpg-sim/src/world.rs:711`, saturating +1), then `run_systems`
(`crates/crpg-sim/src/tick.rs:43`), which runs exactly one system in spec
§10 stage order: `timeline_system`
(`crates/crpg-sim/src/tick.rs:55`), iterating standing entries with no
effect. `end_turn` (`crates/crpg-sim/src/tick.rs:33`) is separate: pops the
timeline head, no re-queue, no despawn, no counter advance. Combat turns
advance only inside `perform_action`
(`crates/crpg-sim/src/combat.rs:1786`); `tick`/`run_systems`/`end_turn`
know nothing of combat.

## Agent log

- 2026-10-23 (UTC) · opencode-go/muse-spark-1.3-contributor + T138 facts · Wrote the T138 fact sheet mapping ADR-0029 T1–T16 and N5 to existing sim code, tests, and T136a data fields with verified path:line citations.

# T157 facts — "walk, then fight" (exploration into combat, movement in combat)

## Requirements

- `Turn-based and real-time-with-pause combat` (`docs/CRPG_ENGINE_SPEC.md:262`) — spec lists turn-based and real-time-with-pause combat as a target, tick rate 10–20 Hz.
- `Time model` (`docs/CRPG_ENGINE_SPEC.md:316`) — section 2.5 "Time model" introduces the Timeline.
- `Timeline advance policy` (`docs/CRPG_ENGINE_SPEC.md:320`) — real-time-with-pause and turn-based are both expressed as a Timeline; in real time it advances every tick, in turn-based on `EndTurn`; one mechanism, two policies.
- `Action economy as resource pools` (`docs/CRPG_ENGINE_SPEC.md:386`) — a real-time system uses per-ability cooldown pools refreshing on tick; 5e-style `movement` is a pool; costs are declared per-ability.
- `Ticks in every simulation mode` (`docs/CRPG_ENGINE_SPEC.md:626`) — ticks exist in every simulation mode and resume exactly after save/load; a ruleset may expose round- or turn-based time.
- `Attack the enemy, turn-based` (`docs/CRPG_ENGINE_SPEC.md:1524`) — definition-of-done step 9 is a turn-based combat encounter.
- `Mode is per encounter` (`docs/adr/0029-kernel-range-and-time-model.md:181`) — `CombatState` carries `mode ∈ {TurnBased, RealTime}`; no per-World/area/campaign mode; outside an encounter every entity is in real time; authors cannot lock a mode.
- `RT capability is data` (`docs/adr/0029-kernel-range-and-time-model.md:186`) — a ruleset may declare `round_ticks`; an ability may declare `recovery_ticks` and `repeats_in_real_time`; an encounter whose ruleset declares no `round_ticks` is TB-only and an RT request is rejected with typed `ModeUnsupported`.
- `Switching` (`docs/adr/0029-kernel-range-and-time-model.md:235`) — a host-authorized `RequestMode { mode }` sets `pending_mode`; applies only at a round boundary (RT→TB at the T6 boundary, TB→RT at TB rollover).
- `Initial mode` (`docs/adr/0029-kernel-range-and-time-model.md:247`) — `start_encounter` takes the initial mode as a host input; the sim never chooses a mode.
- `One encounter per World` (`docs/adr/0029-kernel-range-and-time-model.md:256`) — a second engagement in the same area joins the existing encounter; concurrent encounters are out of scope.
- `Joining` (`docs/adr/0029-kernel-range-and-time-model.md:260`) — TB first round inserts the joiner at initiative but never ahead of the active combatant; later rounds skip the current round; RT keys the joiner due now.
- `Mode per encounter (ADR-0029 T1)` (`docs/adr/0036-movement-and-navigation.md:23`) — ADR-0036 Context cites ADR-0029 T1: encounters are TB or RT.
- `Turn-based movement cost data` (`docs/adr/0036-movement-and-navigation.md:277`) — a ruleset may declare `movement { speed, measure, turn: { pool, step, step_cost } }`; absent means nothing moves.
- `Cost formula` (`docs/adr/0036-movement-and-navigation.md:286`) — `cost = ceil(L / step) * step_cost` in `u64`; pool must hold at least `cost`; a move never ends the turn.
- `Speed is separate` (`docs/adr/0036-movement-and-navigation.md:581`) — D46c: walking speed is separate from the turn-based allowance (constant or creature stat).
- `T108 record` (`docs/adr/0029-kernel-range-and-time-model.md:4`) — ADR-0029 status: Accepted by the user on 2026-10-21 (T108, D30c); T108 answers kernel range, RT/TB play in one game, ruleset-described player UI.
- `T108 backlog row` (`tasks/BACKLOG.md:195`) — T108 "in PR #61", ADR-0029 (Proposed at that time), nine open questions (D30c).
- `T108 decision record` (`tasks/DECISIONS-2026-10-16.md:141`) — T108 tests the kernel against CRPG system families and decides RT/TB play in one game.

## Simulation

- `EncounterMode` (`crates/crpg-sim/src/combat.rs:76`) — enum with `TurnBased` and `RealTime` variants; `CombatState.mode` (line 849) is hashed and omitted when TB.
- `EncounterClock` (`crates/crpg-sim/src/combat.rs:90`) — RT encounter clock struct; `CombatState.clock` (line 852) is `Option<EncounterClock>`, `None` = TB-only.
- `StandingOrder` (`crates/crpg-sim/src/combat.rs:99`) — RT per-combatant order `{ ability, target, repeat }`; stored on `Combatant.order` (line 223).
- `RT_CLOCK_MAX` (`crates/crpg-sim/src/combat.rs:86`) — `u64` bound `2_147_483_647` for the RT clock; asserted equal to `i32::MAX`.
- `start_encounter` (`crates/crpg-sim/src/combat.rs:1443`) — validates an authored encounter and publishes it in turn-based mode (delegates to `start_encounter_tracked`).
- `start_encounter_in_mode` (`crates/crpg-sim/src/combat.rs:1454`) — publishes in a host-chosen mode; returns `ModeUnsupported` when RT is requested on a ruleset without `round_ticks`.
- `start_encounter_with_members` (`crates/crpg-sim/src/combat.rs:1502`) — adds record-holding members to the start.
- `request_mode` (`crates/crpg-sim/src/combat.rs:4375`) — sets `pending_mode` only; a later request replaces it; requesting the current mode clears it.
- `join_encounter` (`crates/crpg-sim/src/combat.rs:2675`) — joins a record-holding creature into the active encounter (TB or RT), with TB round-position rules.
- `leave_encounter` (`crates/crpg-sim/src/combat.rs:2816`) — removes a combatant through `remove_combatant_tracked` settlement; dead combatants cannot leave.
- `perform_action` (`crates/crpg-sim/src/combat.rs:3803`) — in RT, `UseAbility` from any live combatant stores a `StandingOrder` and returns `Ok(None)`; `EndTurn` is `WrongMode`; `Stop` clears the order.
- `end_encounter` (`crates/crpg-sim/src/combat.rs:4661`) — releases the active encounter (terminal or mid-fight) and returns the retained summary; writes back records that entered.
- `register_creature` (`crates/crpg-sim/src/creature.rs:147`) — registers a creature record in `World.creatures`; adds a `Mover` when the ruleset declares `movement`.
- `CreatureState` (`crates/crpg-sim/src/creature.rs:66`) — persistent creature record type (location cited via its `placement` method).
- `Mover` (`crates/crpg-sim/src/movement.rs:102`) — movement component with `position`, `speed`, `route`; created for spawned participants and registered creatures.
- `World.movers` / `World.nav` (`crates/crpg-sim/src/movement.rs:713`) — `bind_nav` returns cleared `(EntityId, RouteTag)` pairs; `World` gains `movers` and `nav` (skipped when empty/None).
- `World.place` (`crates/crpg-sim/src/movement.rs:744`) — sets a mover's position and clears any route; trusted host operation.
- `World.set_route` (`crates/crpg-sim/src/movement.rs:771`) — replaces an active route after turn/legality checks; movement begins next tick's stage 4.
- `World.clear_route` (`crates/crpg-sim/src/movement.rs:810`) — clears a mover's active route.
- `validate_move` / `perform_move` (`crates/crpg-sim/src/movement.rs:858`) — turn-based atomic move validation and execution; `perform_move` journals `HistoryEvent::Moved`.
- `tick` / `tick_with_movement` (`crates/crpg-sim/src/tick.rs:23`) — `tick` delegates and discards facts; `tick_with_movement` returns `Vec<MovementFact>`.
- `run_systems` (`crates/crpg-sim/src/tick.rs:62`) — ordered list: encounter clock (stage 2), wait (T052), timeline (stage 3), movement (stage 4).
- encounter-clock stage (`crates/crpg-sim/src/tick.rs:71`) — `encounter_clock_system` advances RT clock, delivers `OnTick`, applies round boundary.
- proximity/trigger encounter start: none found in `crates/crpg-sim/src/` (encounter start is an explicit `start_encounter*` call).

## Protocol

- `PROTOCOL_VERSION` v1 (`crates/crpg-net/src/protocol.rs:25`) — wire protocol version 1; no transform op in v1 (line 305).
- `PROTOCOL_VERSION` v3 (`crates/crpg-net/src/protocol_v3.rs:40`) — explicitly selected v3 frames.
- `PROTOCOL_VERSION` v4 (`crates/crpg-net/src/protocol_v4.rs:27`) — explicitly selected v4 lane-0 frames.
- `RejectionCode::ModeUnsupported` (`crates/crpg-net/src/protocol_v3.rs:567`) — value 21; "the ruleset does not support the requested mode."
- `DeltaOp::RoundStarted` (`crates/crpg-net/src/protocol_v3.rs:856`) — tag 14; carries `round`.
- `DeltaOp::ModeChanged` (`crates/crpg-net/src/protocol_v3.rs:861`) — tag 15; carries `mode` and `round`.
- `DeltaOp::OrderEnded` (`crates/crpg-net/src/protocol_v3.rs:868`) — tag 16; carries `actor`.
- `DeltaOp::Clock` (`crates/crpg-net/src/protocol_v3.rs:885`) — tag 19; pause heartbeat.
- `DeltaOp::Joined` / `Left` (`crates/crpg-net/src/protocol_v3.rs:873`) — tags 17/18; encounter join/leave notices.
- `EncounterFacts` mode/pending_mode/clock (`crates/crpg-net/src/facts_v3.rs:87`) — grants and facts for mode, pending mode and round clock.
- `clock_op` (`crates/crpg-net/src/facts_v3.rs:403`) — builds the viewer's `Clock` op.
- `IntentBody::DeclareMove` (`crates/crpg-net/src/protocol_v4.rs:118`) — v4 turn-based combat move verb (`actor`, `destination`).
- `DeltaOp::Moved` (`crates/crpg-net/src/protocol_v4.rs:160`) — v4 projection of `HistoryEvent::Moved` (`net_id`, `points`).
- `MoveBody::MoveTo` / `Halt` (`crates/crpg-net/src/movement_v4.rs:78`) — lane-1 reliable upstream intents (`seq`, `actor`, `destination`).
- `MoveReceipt` (`crates/crpg-net/src/movement_v4.rs:116`) — lane-1 receipt with `seq`, `actor`, `outcome` (`Accepted { start_tick }`, `Superseded`, `Rejected { code }`).
- `MoveSample` (`crates/crpg-net/src/movement_v4.rs:143`) — lane-1 unreliable datagram with `server_tick`, `processed_seq`, position entries.
- v4-as-v3 session (`crates/crpg-server/AGENTS.md:467`) — a v4 session is a v3 session with `v4: Some(V4State)`; every v3 path keeps serving it, so lane-0 combat facts and lane-1 movement share one connection.
- `ModeUnsupported` on real-time open (`crates/crpg-godot/AGENTS.md:206`) — `open_v4_rt*` on `combat_basic` answers 25 `ModeUnsupported` because that fixture's ruleset has no `round_ticks`; use `campaigns/demo/walk`.
- ModeUnsupported raised in sim (`crates/crpg-sim/src/combat.rs:1534`) — `start_encounter_in_mode` returns `CombatError::ModeUnsupported` when RT is requested on a ruleset without `round_ticks`.
- ModeUnsupported also raised in sim (`crates/crpg-sim/src/combat.rs:4381`) — `request_mode` returns `ModeUnsupported` when real time is requested on a TB-only encounter.

## Server host

- `Host::pause` / `unpause` / `paused` (`crates/crpg-server/src/host.rs:4910`) — pause and unpause at a given `now_ms`; `paused()` returns `Option<PauseReason>`.
- `Host::settings` / `set_settings` (`crates/crpg-server/src/host.rs:4873`) — `HostSettings` read/write.
- `Host::advance` (`crates/crpg-server/src/host.rs:4945`) — pause-aware driver over `tick()`: ticks at most once and ends at most one turn per call.
- `Host::tick` (`crates/crpg-server/src/host.rs:4445`) — runs on a paused host too (operator primitive).
- `Host::request_mode` (`crates/crpg-server/src/host.rs:4508`) — host-authorized mode request; journals nothing and records nothing.
- `Host::host_action` (`crates/crpg-server/src/host.rs:4474`) — executes a caller-decided action through host operations.
- `Host::host_move` (`crates/crpg-server/src/host.rs:4708`) — trusted turn-based move (W4), recorded as a `HostOp::Action` host record; validates `EndTurn` turn checks first.
- `Host::move_to` (`crates/crpg-server/src/host.rs:4663`) — real-time/exploration route set (stage-4 stepping); `set_route` rejects combatants in a TB encounter (`MoveError::TurnBased`).
- `Host::halt` (`crates/crpg-server/src/host.rs:4684`) — clears a mover's route.
- `Host::place` (`crates/crpg-server/src/host.rs:4643`) — sets a mover's position and clears any route.
- `Host::bind_nav_mesh` (`crates/crpg-server/src/host.rs:4580`) — bakes/announces nav revision.
- `Host::ingest_move` (`crates/crpg-server/src/host.rs:3715`) — lane-1 admission (decides nothing; decisions happen in `advance`).
- `Host::take_samples` (`crates/crpg-server/src/host.rs:3676`) — frees lane-1 egress room for samples.
- `Host::bind_v4_peer` (`crates/crpg-server/src/host.rs:1377`) — binds a v4 session (v3 session + `V4State`).
- `Host::set_position_withheld` (`crates/crpg-server/src/host.rs:1433`) — publishes withheld/fresh position anchors to one v4 session.
- `Host::drive_ai` (`crates/crpg-server/src/host/ai.rs:85`) — runs `crpg_ai::decide` for the caller's AI-controlled actors and maps decisions through host operations; plans TurnBased vs RealTime based on `combat.mode` (lines 116-137).
- pause heartbeat (`crates/crpg-server/src/host.rs:3178`) — `heartbeat_due` / `send_clock` for v3 pages (line 3180).
- pause heartbeat (`crates/crpg-server/src/host.rs:3330`) — same for v4 pages (line 3332).
- `HostSettings` / `AdvanceReport` / `SINGLE_PLAYER_AUTO_PAUSE_BITS` (`crates/crpg-server/AGENTS.md:60`) — T141b additive host items.
- `Host::paused`/`pause`/`unpause`/`advance`/`turn_timer_remaining_ms` (`crates/crpg-server/AGENTS.md:62`) — T141b additive host items.
- `drive_ai` only `Closed` is `Err` (`crates/crpg-server/AGENTS.md:313`) — refusals become `AiStep::Refused`/`Undecided`, changing nothing.

## Godot client

- `CrpgSession.open` (`crates/crpg-godot/src/shell.rs:40`) — opens a v2 session on a `LocalLink`; campaign dir defaults to `campaigns/fixtures/combat_basic` (`apps/client/scripts/Main.gd:59`).
- `CrpgSession.open_v4` (`crates/crpg-godot/src/shell.rs:63`) — opens a turn-based protocol v4 session on the run loop.
- `CrpgSession.open_v4_rt` (`crates/crpg-godot/src/shell.rs:86`) — opens a real-time protocol v4 session on the run loop.
- `BridgeCore.open` (`crates/crpg-godot/src/core/mod.rs:156`) — v2 session open.
- `BridgeCore.open_v4` (`crates/crpg-godot/src/core/mod.rs:173`) — v4 turn-based open (delegates to `open_v4_with`).
- `BridgeCore.open_v4_rt` (`crates/crpg-godot/src/core/mod.rs:210`) — v4 real-time open (delegates to `open_v4_rt_with`).
- `BridgeCore.open_v4_mode` (`crates/crpg-godot/src/core/mod.rs:245`) — shared body; calls `open_prefix` with the chosen `EncounterMode`.
- `Main.gd` (`apps/client/scripts/Main.gd:24`) — uses `session.open(...)` (v2) against `combat_basic` (campaign dir at line 59).
- `Walk.gd` (`apps/client/scripts/Walk.gd:32`) — uses `session.open_v4_rt(...)` against `campaigns/demo/walk`.
- Scene or mode switching in client code: none found (no `change_scene`/scene switch; scenes are separate `.tscn` entry points).

## Campaign data

- `campaigns/demo/walk/` files: campaign, campaign.lock, abilities/strike.json, areas/start/area.json, areas/start/placements.json, assets/assets.lock, creatures/goblin.json, creatures/hero.json, encounters/first-blood.json, locale/en.json, outcome_tables/roll-under.json, rulesets/minimal-d6.json, variables/campaign_state.json, worlds/world.json (14-ish files, plus .gitattributes).
- `campaigns/fixtures/combat_basic/` files: same set as walk minus `areas/start/placements.json`'s second hero and with no movement block (13 files, plus .gitattributes).
- walk ruleset movement block (`campaigns/demo/walk/rulesets/minimal-d6.json`) — declares `movement` with `speed` constant 6554, `turn.pool` "000...0P", `step` constant 65536, `step_cost` 1, plus `round_ticks: 100`.
- combat_basic ruleset (`campaigns/fixtures/combat_basic/rulesets/minimal-d6.json`) — no `movement` block and no `round_ticks`.
- walk encounter (`campaigns/demo/walk/encounters/first-blood.json`) — 3 participants: hero placement E initiative 10, second hero placement Q initiative 10, goblin placement F initiative 9.
- combat_basic encounter (`campaigns/fixtures/combat_basic/encounters/first-blood.json`) — 2 participants: hero E initiative 10, goblin F initiative 9.
- walk placements (`campaigns/demo/walk/areas/start/placements.json`) — 3 placements (E hero, F goblin, Q second hero) with Fx16_16 positions.
- combat_basic placements (`campaigns/fixtures/combat_basic/areas/start/placements.json`) — 2 placements (E hero, F goblin) at position [0,0,0].
- `Encounter` (`crates/crpg-data/src/combat.rs:763`) — fields: `id`, `slug`, `name`, `note`, `ruleset`, `participants: Vec<EncounterParticipant>`, `respawns`.
- `EncounterParticipant` (`crates/crpg-data/src/combat.rs:752`) — fields: `placement: Ulid`, `initiative: i32`.
- `Document::Encounter` (`crates/crpg-data/src/document.rs:440`) — schema id `crpg.encounter/1`.
- `Trigger` (`crates/crpg-data/src/ir.rs:44`) — variants: `AreaEnter`, `AreaExit`, `Interact`, `DialogueNode`, `QuestStateChange`, `Timer { ticks }`, `CombatStart`, `CombatEnd`, `Death`, `ItemAcquired`, `Custom { id }`.
- `TriggersDocument` (`crates/crpg-data/src/document.rs:354`) — fields: `area`, `graphs: Vec<EventGraph>`, `note`.
- `Document::Triggers` (`crates/crpg-data/src/document.rs:449`) — schema id `crpg.triggers/1`.
- encounter/trigger proximity start in campaign data: none found (no trigger graphs shipped in either campaign; encounters are authored documents, not proximity-triggered).

## Open follow-ups

- BACKLOG.md:121 — "resume for protocol v4 (movement) sessions: T155a/T155b/T155c; no resume version carrying lane-1 progress (D76a)".
- BACKLOG.md:123 — "T026p ... defines protocol v4 (movement); T037 (host movement) and two new follow-ups now carry the rest: a crpg-net-quic lane-1 stream and datagram task, and a crpg-godot bridge task after T037".
- BACKLOG.md:151 — "(crpg-server) make "movement": true the dedicated server default once terminal play and the scripted test clients speak /3 (D76c)".
- BACKLOG.md:246 — "T138a | merged (PR #70) | ... crpg-sim encounter mode and RT policy: merged into the batch with two-keys rt_contract.rs and review fixes".
- BACKLOG.md:294 — "T148 ... crpg-data ruleset movement block (speed, measure, turn-based cost data); ... prerequisite ADR-0036."
- BACKLOG.md:298 — "T036c | done ... | turn-based move, history v3, history_v3 golden, with companions T036c-server and T036c-net in the same batch."
- BACKLOG.md:310 — "T138 | crpg-sim | T138a–e merged (PR #70, 2026-10-24) | Yes | Encounter mode and RT policy (T1–T16); history v2 (N5) | T136 (RT fields)".
- BACKLOG.md:311 — "T139 | ... Persistent creature state outside encounters; join and leave (T13, T14) | T137, T138".
- BACKLOG.md:313 — "T141 | crpg-server | complete (s, a–e), merged in PR #93 (2c85ec6) | ... | P1–P9; mode requests; projection of the new facts; heartbeat; ...".
- BACKLOG.md:345 — "(crpg-server) T141: the bind-time host-facts delta reads the turn timer before bind_v3_peer records its now_ms; pin or change the order."
- BACKLOG.md:346 — "(crpg-server) T141: host join/leave for v3 sessions."
- BACKLOG.md:347 — "(crpg-script) T141: real-time proposals."
- BACKLOG.md:348 — "(crpg-net-quic) T141: QUIC /3 route."
- BACKLOG.md:349 — "(crpg-ai) T141: AI driver for real-time."
- BACKLOG.md:350 — "(crpg-net, crpg-server, crpg-godot) T143 consumers: preview protocol message, host handler using from_wire, HUD; and a line in T141 that its host answers no preview."
- T049b (turn-based `Moved` animation): referenced in `tasks/T049.md:95` and `tasks/T049.md:342` (not in BACKLOG.md).
- T050c: referenced in `tasks/T050.md:157` and `tasks/T050.md:614` (not in BACKLOG.md).

## Agent log

- 2026-11-22 (UTC) · opencode-go/beam-free + T157 facts · Fact sheet for "walk, then fight" exploration-to-combat and movement-in-combat, gathered from spec, ADR-0029, ADR-0036, T108 record, sim/net/server/godot code and campaign data.

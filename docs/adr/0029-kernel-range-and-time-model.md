# ADR-0029: Kernel range, the two-mode time model, and ruleset-described player UI

Date: 2026-10-21 (UTC)
Status: **Accepted** by the user on 2026-10-21 (T108, D30c), with the
recommendation for each of the nine open questions.

Numbering: the brief asked for ADR-0028, but `tasks/T024.md` (S§13) already
reserves ADR-0028 for the snapshot transfer subprotocol, so this ADR takes the
next free number. In this file "ADR-0028" means T024's snapshot ADR.

## Context

T108 (`docs/IMPLEMENTATION_PLAN.md`, 4C) answers three questions from D28l and
D28m (`tasks/DECISIONS-2026-10-16.md`): (1) the range of the seven rules
primitives (spec §3.1) across CRPG system families; (2) real-time-with-pause
(RT) and turn-based (TB) play in one game; (3) how a ruleset describes its
player UI. It gates T034 (movement ADR), T051 (HUD) and T109 (a third ruleset).
Inputs: two blind drafts (Beam, LongCat), `docs/research/T099/R13-deep.md`,
`R14-deep.md`, `docs/research/T104/R16-deep.md`, and the code. Every engine
fact below was checked against the code on `origin/mgr/session-10-batch`.

What is true today:

- **The kernel is wider than the authoring surface.** `crpg-rules` supports
  typed stats (`Int`, `Fixed`, `Bool`, `Enum`, `Tags`, `Dice`) with derived
  `Expr` trees (`stats.rs`), modifiers on stats, rolls and DCs with three
  stacking policies and tag conditions (`modifier.rs`), one-group dice with
  keep/drop (`dice.rs`), margin-band outcome tables with natural-face rules
  and a public pure `OutcomeTable::evaluate` (`resolution.rs:197`), `resolve`
  against a constant, a stat or an opposed roll, and resource pools
  (`resource.rs`). But the data that reaches it is narrow: rulesets declare
  `Int` stats only (`crpg-data/src/combat.rs:44`) and no derived stats; an
  ability has one dice expression, flat per-outcome damage
  (`crpg-sim/src/combat.rs:231`, `:239`) and a defence that is either roll-under
  the actor's attribute or the bare roll against a target stat — the actor's
  attribute is **not** added in the target-stat case (`combat.rs:1851-1900`);
  effects modify only the roll and DC folds with `Add`/`Set`
  (`combat.rs:173-177`, `1464`); combatants carry no tags; initiative is an
  authored constant (`crpg-data/src/combat.rs:376`); an encounter ends when
  fewer than two combatants are alive, with no sides (`combat.rs:2029-2036`).
- **Effects** are not a kernel type: `crpg.effect/1` documents become sim
  `EffectDefinition`s with `duration_rounds`; the kernel keeps no effect store
  (`docs/architecture/crpg-rules.md`).
- **Hooks**: `KernelHook` has nine variants (`hooks.rs`); spec §3.2 also lists
  `OnApplyEffect`, `OnRemoveEffect`, `OnMove`, `OnActionDeclared`. Nothing
  dispatches hooks: the sim emits none, and `crpg-script` has no Lua runtime
  (`crpg-script/src/lib.rs:1-10`); `mlua` arrives with T055 in M8
  (plan line 150).
- **Refresh**: the sim delivers only `TurnStart` and `RoundStart` refreshes
  (`combat.rs:1409`, `2172`, `2201`, `2247`). `OnTick` and `OnRest` pools
  are accepted in data but never refill in play. `ResourcePool` has no
  maximum adjustment and refills only to maximum (`resource.rs:1-9`).
- **Time**: one `World` per area with one `Timeline` container and at most one
  encounter (`world.rs:82`, `combat: Option<CombatState>`). `tick()` advances
  the counter and runs `timeline_system`, whose body is empty
  (`tick.rs:54-60`); `end_turn` is pop-only. TB combat runs on the timeline
  (ADR-0013, ADR-0016). The host ticks only when its owner calls
  `Host::tick` (`crpg-server/src/host.rs:1553`); there is no RT policy, no
  pause, no mode.
- **Wire**: v1/v2 carry `Health`, `Turn {active, round}`, and the events
  `ActionResolved`, `TurnStarted`, `EncounterEnded`; never rolls, DCs or
  margins (ADR-0019). No timeline, pool, stat, effect, mode or pause fact
  exists. A command's `observed_tick` must lie within 200 ticks of the server
  tick (`crpg-net/src/protocol.rs:165`). Snapshot transfer is T024
  (contract drafted).

## Decision

Numbered items are normative; each is written so a test can prove it.

### 1. Kernel range

K1. **Three layers, three verdicts.** A capability is classed by the lowest
layer that lacks it: **kernel** (`crpg-rules`), **surface** (the `crpg-data`
schemas plus the `crpg-sim` data-to-runtime adapter that feeds the kernel),
or **script** (hook dispatch, T053/T055). Only a missing kernel capability is
an abstraction failure in the spec §3.3 sense. A surface gap is expected
generalization work.

K2. **Verdict.** The seven primitives cover all five families except the
three kernel gaps in K3. Every other shortfall found is a surface gap
(table below), closed by the surface tasks in "Follow-on contracts".

K3. **Kernel gaps (the complete list).**
(a) *Success counting*: a dice total is always a kept sum plus offset
(`dice.rs`); dice pools need "count kept dice with face ≥ n" (and, for
botches, "count faces ≤ m"). Smallest change: an evaluation mode on
`DiceExpr` (sum by default, so every existing expression and notation is
unchanged), still one group, one named stream, one trace.
(b) *Pool refill amount and maximum change*: refresh refills to maximum only,
and maximum is fixed at construction. Action-point systems with carry-over
(+n per turn up to a cap) and mid-fight maximum changes need an optional
refill amount and a validated maximum setter that clamps `current`.
(c) *Hook vocabulary*: the four spec hooks above are missing. They are added
with the first dispatcher that consumes them (T053/T055), never ahead of a
consumer (`crpg-sim/AGENTS.md` known trap: vocabulary needs a consumer).
Exploding dice and glitch rules are kernel gaps too, but deferred: no planned
ruleset needs them (Open question 3).

K4. **Not kernel gaps.** Graded percentile success (half/fifth skill) is a
surface composition: one `resolve`, then the public `OutcomeTable::evaluate`
on margins against derived thresholds, with no second draw. Per-encounter
pools are `Never` pools, because the sim builds pools at encounter start.
Stat-threshold modifier conditions ("if skill ≥ 12") use tags projected by the
surface; `ConditionExpr` stays tag-only.

K5. **Out of scope** (spec §3.4 and §22): class, level and progression
models, character generation, encumbrance, economy; reaction and interrupt
windows (they need hook dispatch and an off-turn action design; revisit with
T053); skill-dependent fumble ranges; CoC-style bonus tens dice; balance.

K6. **Primitive coverage per family.** S = supported end to end today;
KG = kernel gap (K3); SG = surface gap; OOS = out of scope.

**d20 (5e, PF2e, SRD)**

| Mechanic | Primitives | Verdict |
|---|---|---|
| d20 + bonus vs AC or DC | Resolution, Modifier, Stat | SG: kernel folds a stat-derived roll modifier; the adapter adds no attribute in the target-stat case |
| Degrees of success, natural 20/1 shift | Resolution | S (outcome-table bands + `NaturalRule::Shift`) |
| Advantage/disadvantage | Resolution (dice) | S (`2d20kh1` / `kl1` notation) |
| Typed bonus stacking | Modifier | S (three policies, per-effect policy) |
| Ability modifier, AC formula | Stat (derived) | SG: `Expr` exists; data has no derived stats |
| Conditions (frightened 2), stat buffs | Effect, Tag, Modifier | SG: no combatant tags; effects touch roll/DC only |
| Damage dice, crit doubling | Resolution (dice) | SG: damage is a flat integer per outcome |
| Action, bonus action, reaction pools | Resource | S for turn pools; reactions OOS (K5) |
| Spell slots per rest | Resource (`OnRest`) | SG: pools live only inside an encounter; no rest |
| Rolled initiative | Resolution | SG: initiative is authored |
| Levels, classes | — | OOS |

**Dice pool (World of Darkness, Shadowrun)**

| Mechanic | Primitives | Verdict |
|---|---|---|
| Pool = attribute + skill + modifiers | Stat, Modifier | SG |
| Count successes ≥ target number | Resolution (dice) | KG K3(a) |
| Opposed pools, net successes | Resolution (`Against::Opposed`) | SG after K3(a) |
| Exploding dice, botch/glitch | Resolution (dice) | KG, deferred |
| Edge/willpower pools | Resource | S as pools; spend-at-roll choice SG |

**Percentile (Call of Cthulhu, BRP, Fallout-like)**

| Mechanic | Primitives | Verdict |
|---|---|---|
| d100 roll-under skill | Resolution | S (`1d100`, actor-attribute defence, as `minimal-d6` does with 2d6) |
| 01 critical / 00 fumble | Resolution | S (`NaturalRule::Override`) |
| Skill minus target defence | Resolution, Modifier | SG: roll-under ignores target stats |
| Graded success (half, fifth) | Resolution | SG (K4 composition) |
| Separate critical-chance roll | Resolution | SG: chained resolutions on one stream |

**Action points (Fallout, Jagged Alliance, PF2e, DOS2)**

| Mechanic | Primitives | Verdict |
|---|---|---|
| AP per turn, costs, several actions per turn | Resource | S (pools, costs, `ends_turn: false`) |
| AP maximum from a stat | Resource, Stat | SG at encounter start; KG K3(b) mid-fight |
| AP carry-over (+n up to cap) | Resource | KG K3(b) |
| Movement costs AP | Resource | T034/T036 (spend from a pool; no kernel change) |
| RT cooldowns (`OnTick`) | Resource | SG: closed by T3 below |
| Overwatch, interrupts | Event/Hook | OOS (K5) |

**Classless skill-based (GURPS, RuneQuest, Elder Scrolls-like)**

| Mechanic | Primitives | Verdict |
|---|---|---|
| Skills defaulting from attributes | Stat (derived) | SG |
| Roll-under skill, margin of success | Resolution | S |
| Quick contests | Resolution (opposed) | SG |
| Advantages and perks | Modifier, Tag | SG |
| Fatigue and mana with rest | Resource (`OnRest`) | SG |
| Improvement by use or XP | Event/Hook (`HookMutation::SetBaseStat`) | OOS until hook dispatch |

K7. **T109 is re-scoped.** "A kernel change is an abstraction failure" means
a `crpg-rules` change. T109 runs in data only: Lua does not exist before
T055, and nothing T109 needs requires it. T109 runs after the surface tasks
(Follow-on 3 and 4). A shortfall T109 finds is reported with its K1 layer;
only kernel shortfalls come back to this ADR (Open question 2).

### 2. Time model

T1. **Mode is per encounter.** `CombatState` (in `World`, hashed) carries
`mode ∈ {TurnBased, RealTime}`. There is no per-World, per-area or
per-campaign mode. Outside an encounter every entity is in real time.
Authors cannot lock a mode (R14 rec 2: Deadfire's campaign lock was reversed).

T2. **RT capability is data.** A ruleset may declare `round_ticks` (a
nonzero tick count per round: 120 is 6 s at 20 Hz), and an ability may declare
`recovery_ticks` (nonzero; default `round_ticks`) and `repeats_in_real_time`
(default false). An encounter whose ruleset declares no `round_ticks` is
TB-only: a request for RT is rejected with a typed `ModeUnsupported` error
and changes nothing. Every new `World` field is absent in TB-only worlds, so
existing saves, hashes and goldens stay byte-identical (the ADR-0013 and
ADR-0020 absence-compatibility pattern).

T3. **Encounter clock.** An RT-capable encounter carries `clock: u64`. In RT
it advances by one per world tick; in TB it advances by `round_ticks` at
each round rollover. Encounter-scoped tick rules read the clock, never the
world tick: an `OnTick(n)` pool refreshes once per advance in which the clock
crosses at least one multiple of `n`. A 6-second cooldown therefore lasts
120 ticks in RT and one round in TB.

T4. **The TB policy is unchanged** (ADR-0013 Decision 5, ADR-0016): one
entry per live combatant keyed by initiative, the head acts, an accepted
turn-ending action pops it, an empty timeline with two or more alive rolls
the round over.

T5. **RT keys.** In RT each live combatant holds exactly one timeline entry,
keyed by its ready clock value. The key is `i32` (`timeline.rs`), so an RT
encounter's clock is bounded; the contract pins the behaviour at the bound,
which must fail closed and never wrap or reorder.

T6. **RT round boundary (tick stage 2).** When the advanced clock is a
multiple of `round_ticks`: apply any pending mode (T9); then `round + 1`
(saturating, as now); then `RoundStart` and `TurnStart` refreshes to every
live combatant in ascending `EntityId` order; effects expire by round as now;
journal one `RoundStarted { round }`.

T7. **RT execution (tick stage 3, the `timeline_system` body).** Take the
list of entries with key ≤ clock, in ascending `(key, EntityId)` order,
before acting. For each one still alive with a standing order: validate it as
a TB `UseAbility` without the active-turn check. If valid, execute it through
the same resolution, cost, damage, effect, RNG stream (`combat.roll`) and
journal path as TB, then re-key the actor to `clock + recovery_ticks`; clear a
one-shot order, keep a repeating one. If invalid, clear it and journal
`OrderEnded { actor }`. A due combatant without an order keeps its key and
acts on the first tick after it receives one. Death removes the entry at
once, as in TB. Terminal rules are the TB rules.

T8. **Orders.** `Combatant` carries `order: Option<StandingOrder { ability,
target, repeat }>` (hashed; absent when `None`). In RT, `UseAbility` from any
live combatant replaces its order and never executes at once; a new `Stop`
command clears it; `EndTurn` is rejected (`WrongMode`). `legal_actions` in RT
lists options for any live combatant. Orders do not exist in TB.

T9. **Switching.** A host-authorized `RequestMode { mode }` sets
`pending_mode` (hashed). A later request replaces it; requesting the current
mode clears it. A pending switch applies only at a round boundary: RT→TB at
the T6 boundary, TB→RT at TB rollover.

T10. **Applying a switch.** RT→TB: clear all orders, rebuild the timeline
from live combatants at their stored initiative, then run the normal TB round
start. TB→RT: key every live combatant at `clock` (due now), then run the T6
round start. Both journal `ModeChanged { mode, round }`. Every round gets
exactly one round-start refresh in either mode, so toggling cannot refill
pools early. A test toggles every round and counts refreshes.

T11. **Initial mode.** `start_encounter` takes the initial mode as a host
input (a session setting). The sim never chooses a mode.

T12. **Split parties.** Only combatants hold timeline entries. While an
encounter is active the host keeps ticking the World at the fixed rate, so
non-participants act in real time beside a TB fight (R14 rec 6). T034/T036
must move non-participants and RT participants every tick, and TB
participants only through their own turn's actions.

T13. **One encounter per World.** A second engagement in the same area joins
the existing encounter; it never starts another one. Concurrent encounters
in one area are out of scope (Open question 7).

T14. **Joining** (when the join operation exists, Follow-on 6). TB, first
round: the joiner is inserted into the current round at its initiative but
never ahead of the active combatant. TB, later rounds: the joiner skips the
current round and enters at the next rollover (R14 rec 3). RT: the joiner is
keyed due now.

T15. **AI is outside tick order.** AI (spec §6.2) runs in the host between
ticks and submits ordinary commands through `legal_actions`: in TB when an
AI-controlled combatant becomes head, in RT when one is due with no order.
Its output enters the World only as journaled commands, so replay never
re-runs AI. This settles the §2.5 "turn-start versus every-tick" mapping and
replaces spec §10 stage 5 as a sim stage.

T16. **Exact-build replay.** Every World change is a journaled operation
(tick, command, mode request, start, end, join). Pause, auto-pause, turn
timers and AI live in the host and reach the World only as journaled
operations. Clock, keys, recovery and rounds are integers; RT adds no float.
Positions stay `f32` in sim and are spatial only.

### 3. Pause and multiplayer authority (host, `crpg-server`)

P1. **Pause is host state.** A paused host does not call `tick`. Pause is
not a World field, is not hashed and is not journaled.

P2. **Commands while paused.** The host still admits and executes commands:
TB actions resolve and RT orders and mode requests are stored. Only ticks
stop (Baldur's Gate "pause and give orders").

P3. **Scope.** Pause stops every World the session's host ticks.

P4. **Rights.** Each bound peer has two independent rights, `pause` and
`unpause`, granted and revoked by the host operator through the trusted API
(the ADR-0022 grant discipline). Anyone holding `pause` pauses everyone
(R14 rec 1). Defaults: embedded single player, the local player holds both;
a hosted multiplayer session, the session owner holds both and other players
hold none until granted; a dedicated persistent world, the operator only
(R14 rec 9). A pause or unpause from a peer without the right is rejected
with a typed code, never ignored.

P5. **Reason.** The host keeps `paused: Option<PauseReason>` with reason
`Player(peer)`, `Auto(trigger)`, `Operator` or `Restore`. Any holder of
`unpause` clears it.

P6. **Auto-pause.** A closed trigger list is evaluated by the host over the
history committed by tick t, before tick t+1: encounter started, round
started (RT), mode changed, order ended, party member died. Toggles are per
session and operator-owned. Defaults: single player, encounter started,
order ended and party member died are on; multiplayer, all off (R14 rec 4).
A trigger that needs a fact the history lacks (for example "enemy sighted")
waits for that `HistoryEvent`.

P7. **Restore.** A host started from a checkpoint or save starts paused with
reason `Restore`. No pause state is persisted.

P8. **Turn timer** (R14 rec 5): optional, default off. On expiry the host
submits `EndTurn` for the active combatant as an ordinary journaled command.
Wall-clock reads stay in `crpg-server`.

P9. **Dialogue** (R14 rec 7) is an input to T054/T057: important dialogue
requests an `Operator` pause after a party vote with host tiebreak; trivial
dialogue and stores never pause.

### 4. What snapshots and the protocol carry

N1. **Facts.** World facts (hashed): mode, pending mode, round, clock (RT),
timeline order with keys (the shared visible order, R14 rec 5, R16 rec 5),
active combatant (TB), pool `current`/`max` per pool id, attached effects with
expiry round, standing orders, stat values. Host facts (not hashed): paused
and reason, the viewer's own pause rights, the auto-pause toggles, the server
tick, the turn-timer deadline. Events: `RoundStarted`, `ModeChanged`,
`OrderEnded`. Intents: `DeclareAction` (TB: act now; RT: set the order),
`EndTurn` (TB), `Stop` (RT), `Pause`, `Unpause`, `RequestMode`.

N2. **Disclosure.** Every new fact passes the per-field grants of T027b
(H-filter). Standing orders default to the controlling peer only. Rolls, DCs
and margins stay off the wire as ADR-0019 says, unless Open question 8 opens
them.

N3. **One protocol version** adds all of N1 after T024's version. v1, v2 and
T024 stay frozen; T024 proceeds unchanged. The same version extends
ADR-0028's snapshot allowlist, so resync and late join see what deltas show.

N4. **Clock heartbeat.** The host sends `Clock { server_tick, paused }` when
pause changes and at a policy interval while ticking. Without it, a ticking
TB fight with no events would push every client's `observed_tick` past the
200-tick freshness window.

N5. **History** gains `RoundStarted`, `ModeChanged` and `OrderEnded`.
`HISTORY_VERSION` becomes 2, with new per-target history goldens; the v1
journal and its goldens stay readable and unchanged.

### 5. Ruleset-described player UI

U1. **Document.** A ruleset may ship one `crpg.ui/1` document. It is
presentation data: never loaded into a World, never hashed, delivered to
clients in T072's presentation manifest. The server ignores it.

U2. **Data only.** No expressions, scripts, layout coordinates or widget
names. Every binding is an authored id or name that resolves in the same
ruleset (stat, pool, ability, effect, outcome label, tag) or campaign (item).
Every label is a locale key (`crpg.locale/1`). Every icon is an asset
reference validated by the asset pipeline (T101).

U3. **Per-object display metadata.** Stats: label, short label,
description, group, and a format from a closed set (`integer`, `signed`,
`fraction` of a stat or pool, `percent`, `boolean`, `enum` labels, `tags`,
`dice`). Pools: label, icon, style (`pips`, `bar`, `number`), shown on the
HUD or not. Abilities: label, description, icon, category tags, target hint,
`show_odds`. Effects: label, icon, show duration. Outcomes: a label for each
outcome including `custom:n`, which the combat log uses.

U4. **Screens.** Closed kinds: `character_sheet`, `action_bar`, `spellbook`,
`inventory`, `level_up`, `combat_hud`, `character_creation`. A screen is an
ordered list of sections. Section kinds are closed too: `stat_group`,
`pool_group`, `ability_list` (explicit ids or a tag filter, with a sort),
`effect_list`, `item_list` (tag filter), `timeline`, `log`, and
`choice_steps`. The client draws each kind with its own widget (T104/T051).

U5. **Generated defaults.** A ruleset without a UI document, or a screen it
omits, gets a default derived deterministically from the ruleset by a pure
`crpg-data` function: all stats in declared order with slug labels,
all abilities on the action bar, health, all pools and the timeline on the
HUD. This is the guarantee that any ruleset displays. The editor, agents and
client call the same function.

U6. **Validation.** Unresolved ids, missing locale keys, unknown kinds and
duplicate bindings are positioned `crpg-data` errors. They fail campaign
validation but never server loading.

U7. **Values come only from replicated facts** (section 4). The client never
evaluates a ruleset formula (spec §9.2). An ungranted field shows as hidden,
never as zero.

U8. `level_up`, `character_creation` and `choice_steps` are reserved. They
bind to progression and creation data families that do not exist yet
(spec §3.4). Until then, the generated default omits them.

U9. **Odds.** `show_odds` displays the outcome distribution from a
server-side read-only preview query (Follow-on 10), never client math
(R16 recs 1 and 8).

U10. **T104 may add screen or section kinds** through a schema version. It
may not add code or expressions.

## Authority and affected surface

- Decided by: the user, at H-accept (pending).
- Authoritative contracts: the follow-on contracts below. This ADR fixes
  decisions, not exact APIs.
- Owning crates: `crpg-rules`, `crpg-data`, `crpg-sim`, `crpg-net`,
  `crpg-server`; `crpg-godot` consumes (T051).

## Compatibility, hash and dependency impact

| Serialized set (plan §1.4) | Touched? | By |
|---|---|---|
| `World` struct | **Yes** | T1, T3, T8, T9 (CombatState/Combatant fields, absent in TB-only worlds); Follow-on 4 and 6 |
| Tick order | **Yes** | T6 adds an encounter-clock system at stage 2; T7 fills the stage 3 `timeline_system` body. No-encounter and TB ticks stay byte-identical |
| `crpg-data` schema versions and migrations | **Yes** | Follow-on 3 (ruleset/ability/effect/encounter next versions with migrations), Follow-on 9 (`crpg.ui/1`) |
| `crpg-contracts` | No | — |
| GDExtension boundary | No | The UI document reaches the client through T072; bridge queries are T045's |
| Workspace `Cargo.toml`, `deny.toml`, workflows | No | No new dependency |

- Not in the set but review-dense: `HISTORY_VERSION` 2 (sim), the next
  protocol version (net, H-filter), additive `crpg-rules` changes (K3).
- Goldens: every existing golden stays byte-identical, and a test proves it.
  RT, history-v2 and generalized-surface goldens are new, per target
  (ADR-0012), H-golden.
- Invariants: no float in core or rules; sim adds integers only; no
  `HashMap`/`HashSet`; no OS branch below the server; dependency direction
  unchanged (`deps.py` `ALLOWED` untouched).

## Alternatives considered

- **Mode per World, area or campaign** (Beam A; LongCat b, c): rejected. A
  split party needs RT beside TB in one area, and campaign locks were
  reversed in Deadfire.
- **Mode per entity, or two timelines** (Beam C; LongCat b, c): rejected.
  This interleaves two policies in one ordered walk, or creates a second
  structure that can disagree with the first.
- **Mode held by the host, outside the hash**: rejected. Mode changes
  behaviour, so ADR-0013 (Decision 1) requires it in the World.
- **Pause in the World or the journal** (Beam open question 2): rejected.
  Nothing advances while paused, so there is nothing to replay.
- **Immediate mode switch**: rejected for now. It refills pools mid-round or
  double-acts a combatant (Open question 1).
- **Effect type, lifetime field or stat conditions in the kernel** (Beam B,
  LongCat): rejected. The surface covers them (K4); the kernel keeps
  meaning-free primitives.
- **Ruleset-shipped widgets or UI scripts** (Beam A, LongCat c): rejected by
  spec §4.7/§12.1 (no client code). **Engine-fixed screens with no ruleset
  input** (Beam B): rejected. Labels, grouping and formats are ruleset
  meaning.
- **BG3-style simultaneous turns for one player's tied characters**:
  declined. The controller is a host fact, and strict `(key, EntityId)` order
  is enough for v1.

## Consequences

- **T034** can be written once this ADR is accepted. Its inputs are T12
  (who moves when), T16 (positions `f32`, costs integer: T034 pins the
  quantization in sim) and the AP verdict in K6 (movement spends a pool; no
  kernel change).
- **T036 and Follow-on 5** both touch `World` and tick order. Follow-on 5
  goes first, so movement lands on the two-mode loop.
- **T109** waits for Follow-on 3 and 4 and runs data-only (K7).
- **T051** builds on U1–U10 and the N1 facts. **T072** carries the UI
  document. **T053/T055** add the K3(c) hooks with their dispatcher.
- Spec §2.5 and §10 need appended notes for T15 after acceptance (the
  manager's edit).
- Harder: the surface tasks are large and S-set, and they queue behind other
  serialized work.

## Follow-on contracts

IDs assigned 2026-10-21 (session 12). One crate each. "Accepted" means this
ADR.

| # | Id | Crate | Contract to write | Depends on | S |
|---|---|---|---|---|---|
| 1 | T134 | `crpg-rules` | K3(a) success-count and face-count evaluation modes on `DiceExpr` (sum default, notation extension, trace unchanged) | Accepted; implement with its first consumer (T137) | — |
| 2 | T135 | `crpg-rules` | K3(b) pool refill amount and validated maximum setter | Accepted | Yes |
| 3 | T136 | `crpg-data` | Surface schemas: typed and derived stats; resolution shape (bonus stats, constant, actor-stat, target-stat or opposed defence, tiered thresholds); damage dice; initiative spec; effects on stats with tag grants and lifetime kinds; combatant tags and sides; `round_ticks`, `recovery_ticks`, `repeats_in_real_time`; migrations. May split the RT fields into a small first task | Accepted; T134 and T135 if exposed | Yes |
| 4 | T137 | `crpg-sim` | Adapter generalization consuming 3, with existing goldens unchanged | T136 | Yes |
| 5 | T138 | `crpg-sim` | Encounter mode and RT policy, T1–T16; history v2 (N5) | T136 (RT fields) | Yes |
| 6 | T139 | `crpg-sim` | Persistent creature state outside encounters (stats, pools, effects, tags); join and leave (T13, T14) | T137, T138 | Yes |
| 7 | T140 | `crpg-net` | Next protocol version: N1–N4 facts, events, intents, snapshot allowlist | T024, T027b, T138 | — (H-filter) |
| 8 | T141 | `crpg-server` | P1–P9; mode requests; projection of the new facts; heartbeat | T138, T140 (in-process pause can come first) | — |
| 9 | T142 | `crpg-data` | `crpg.ui/1` schema, validation and generated defaults (U1–U8, U10) | Accepted; T104 report as input | Yes |
| 10 | T143 | `crpg-sim` | Read-only action preview: exact integer outcome distribution, bounded (U9) | T137 | — |

The `crpg-rules` change itself is outside the S-set, but a new
`ResourcePool` field reaches the hashed `World`, so its contract is
serialized and human-reviewed.

## Open questions for the user

1. **When does a mode switch take effect?** *Recommendation:* at the next
   round boundary (T9), because it cannot be exploited. The cost is up to
   one round of delay, and pausing is still instant.
2. **T109's scope.** *Recommendation:* data only, after Follow-on 3 and 4.
   Drop "+ Lua" until T055. An abstraction failure means a `crpg-rules`
   change (K1, K7).
3. **Kernel additions.** *Recommendation:* approve the contracts for
   Follow-on 1 and 2 now. Implement 1 only with its consumer. Leave exploding
   dice and glitch rules out until a ruleset asks for them.
4. **Default pause rights in multiplayer.** *Recommendation:* as P4. The
   session owner holds both rights, guests get them only by grant, and
   dedicated worlds are operator-only.
5. **Auto-pause defaults.** *Recommendation:* as P6. Three triggers are on
   in single player; all are off in multiplayer.
6. **Do commands run while paused?** *Recommendation:* yes (P2).
7. **One encounter per area.** *Recommendation:* yes, and fights merge
   (T13). Concurrent encounters would turn `CombatState` into a collection,
   which is a separate S-set change, made only if a module needs it.
8. **Roll transparency** (R16 recs 1 and 8) conflicts with ADR-0019's ban on
   rolls on the wire. *Recommendation:* let a ruleset opt in to disclosing
   an actor's own raw roll and modifiers to its controller. DCs stay hidden
   unless the ruleset marks them public. This is an H-filter decision in
   Follow-on 7.
9. **AI outside the tick order** (T15, which amends spec §10 stage 5).
   *Recommendation:* yes. Replay then never re-runs AI.

## Acceptance and evidence

Not yet run. Each follow-on contract turns its items into tests. Two tests
are required: a TB-only fixture and every existing golden stay
byte-identical (T2, Compatibility); and per-target RT goldens prove T6–T10,
including the toggle-refresh count (T10).

## Supersession and corrections

## Agent log

- 2026-10-21 (UTC) · claude-code/opus-5.5 architect + T108 · Filed the proposed kernel-range, two-mode time model and ruleset-UI decision from the Beam and LongCat drafts and the R13/R14/R16 research, after checking each engine claim against the code. The main finding is that the kernel covers the five families except three named gaps, while the data/sim surface is the real limit; numbered 0029 because T024 reserves 0028.
- 2026-10-21 (UTC) · opencode/deepseek-v4.1-flash-free + T108-adr-fix · Corrected the two combat.rs citations, moved the hashed-World rule to ADR-0013 Decision 1, and marked the pool setter S-set because a new ResourcePool field reaches World.
- 2026-10-21 (UTC) · claude-code/opus-5.5 + D30c answered · Marked the ADR accepted with all nine open-question recommendations, as the user answered ("Accepted.").
- 2026-10-21 (UTC) · opencode-go/muse-spark-1.3-contributor + S12-docs · Filled the follow-on table's Id column (T134–T143) and rewrote row-number dependencies as those IDs, per the session 12 assignment; no decision changed.

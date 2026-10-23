# T138 — encounter mode and real-time policy (`crpg-sim`, ADR-0029 T1–T16, N5)

## Task / Crate(s) / Status

ADR-0029 Follow-on 5 (row 5, `docs/adr/0029-kernel-range-and-time-model.md:484`): encounter mode and
real-time policy, T1–T16 (`:181–277`), plus history v2, N5 (`:347`). Sole crate **`crpg-sim`**, S-set
(`World` and tick order), consuming the T136a data fields already on this base:
`Ruleset.round_ticks` (`crates/crpg-data/src/combat.rs:245–247`), `Ability.recovery_ticks`
(`:318–320`), `Ability.repeats_in_real_time` (`:321–323`); data rejects zero values (`:774–775`,
`:821–822`) and adds no other RT rule (`tasks/T136.md:90–92`).

Status: **proposed draft, not approved.** Nothing here is a decision until the architect adjudicates it
against the independent draft and the user approves; every `path:line` was read on this branch.

## Decision

- **D1. Mode is one `EncounterMode` field on `CombatState`** (T1: per encounter, `CombatState` in the hashed
  `World`, `crates/crpg-sim/src/world.rs:82`), default `TurnBased` and omitted when turn-based. *Rejected:*
  `Option<EncounterMode>` (two spellings of TB) and `real_time: bool` (loses the ADR's named pair).
- **D2. Capability and clock are one `Option<EncounterClock>`** on `CombatState`; `None` = TB-only. *Rejected:*
  two flat `Options` (`round_ticks` without `clock`) and `round_ticks` in `CombatDefinition` (its hand-written
  serializer, `combat.rs:391–424`, for no gain).
- **D3. Mode is only set by a host input** (`start_encounter_in_mode`, `request_mode`), never inferred from
  `round_ticks` (T11). *Rejected:* auto-RT when the ruleset declares `round_ticks`.
- **D4. The clock saturates at `RT_CLOCK_MAX = i32::MAX` and freezes**; stages 2–3 do nothing at the bound, so no
  key wraps or reorders (T5). *Rejected:* wrapping; saturating keys (all perpetually due, not fail-closed); a
  `ClockExhausted` error (`tick` returns `()`).
- **D5. Orders are `Combatant.order: Option<StandingOrder>`**, `repeat` fixed from the ability when set (T8).
  *Rejected:* re-reading `repeat` from the definition, and a separate order map.
- **D6. RT `UseAbility` goes through the existing `validate_action`** with a mode-aware turn gate and only sets
  the order: no spend, draw, event or execution, `Ok(None)` (T8). *Rejected:* a second RT admission predicate
  (`crates/crpg-sim/AGENTS.md:235–241`) and executing now.
- **D7. `Stop { actor }` is a `CombatAction` variant; RT `EndTurn` is `WrongMode`.** *Rejected:* a free
  `stop_order` that would need its own admission path and could not be listed by `legal_actions` (T8).
- **D8. `RequestMode` is a separate `request_mode(world, mode)`** (a request has no actor). *Rejected:* a
  `CombatAction` variant (`perform_action` returns an actor/target outcome) and an unjournaled setter.
- **D9. Initial mode is a new `start_encounter_in_mode`; `start_encounter` keeps its signature and means TB.**
  *Rejected:* an `EncounterSpec` field — its literal is built in five crates (e.g.
  `crates/crpg-cli/src/combat_apply.rs:159`), breaking one-crate scope.
- **D10. One round-boundary routine with a mode-specific schedule; a pending switch replaces that schedule**
  (applying it afterwards would refresh every pool twice in the switch round, T10). *Rejected:* immediate
  switching (`:444–445`) and a second per-mode round-start copy.
- **D11. `RoundStarted` is RT-only (steady and switch rounds).** T6 scopes it to the RT boundary; journaling it in
  TB would move every TB history hash, against ADR `:425`.
- **D12. RT execution is a new stage-2 clock system plus the stage-3 `timeline_system` body, both sharing one
  `execute_use` extracted from `perform_action`** (one resolution/cost/damage/effect/RNG/`Died` path, T7).
  *Rejected:* resolving in `perform_action`; a second pipeline copy.
- **D13. `tick` gains a crate-visible `tick_tracked -> TickFacts`; public `tick` keeps `()` and discards.**
  *Rejected:* changing `tick`'s signature, and draining the legacy queue inside a system.
- **D14. Sticky lazy history versioning:** a wrapper starts at 1 and upgrades to 2 on its first v2-only event,
  never downgrading. *Rejected:* always writing 2 (every TB history hash and v1 golden would move; N5 says they
  stay unchanged).
- **D15. New state hashes through ordinary serde with absence forms only; no exclusion.** *Rejected:* a hash
  exclusion (ADR-0009 and `crates/crpg-sim/AGENTS.md:306–309` demand a behaviour proof).
- **D16. `OnTick` delivery is clock-driven**, once per advance that crosses a multiple, with the last crossed
  multiple as the event tick. *Rejected:* delivering `Tick(clock)` only when the landing tick divides (a TB step
  can cross without landing) and deliveries in TB-only encounters.
- **D17. Coherence goes into `World::deserialize`**, beside the existing combat check
  (`crates/crpg-sim/src/world.rs:149–522`). *Rejected:* a new public validation API (T022 owns loading) and
  action-time checks (fail-fast-at-load trap).

## Interface

All new public items are documented (`#![warn(missing_docs)]`, `crates/crpg-sim/src/lib.rs:2`) and
re-exported from `lib.rs`; no dependency is added, no `HashMap`, no float, no `unsafe`.

### `crates/crpg-sim/src/combat.rs`

```rust
use crpg_core::{EntityId, Tick, Ulid}; // Tick added for RefreshEvent::Tick

/// Encounter time policy (ADR-0029 T1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EncounterMode {
    /// One head acts at a time (the unchanged T016 policy).
    #[default]
    TurnBased,
    /// Due standing orders execute inside the tick.
    RealTime,
}

/// The clock of one RT-capable encounter (ADR-0029 T2, T3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EncounterClock {
    /// Ticks per real-time round; nonzero (data-validated).
    pub round_ticks: u32,
    /// Encounter ticks elapsed; never above [`RT_CLOCK_MAX`].
    pub clock: u64,
}

/// The largest encounter clock an RT key can represent (ADR-0029 T5).
pub const RT_CLOCK_MAX: u64 = 2_147_483_647; // i32::MAX

/// One real-time standing order (ADR-0029 T8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct StandingOrder {
    /// The ordered ability identity.
    pub ability: Ulid,
    /// The ordered target identity.
    pub target: EntityId,
    /// Whether the order survives an execution.
    pub repeat: bool,
}
```

`Combatant` (`combat.rs:74–88`) gains, **last**, plus one accessor:

```rust
    /// Real-time standing order; skipped when absent (ADR-0029 T8).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    order: Option<StandingOrder>,
```

and, in `impl Combatant`, `pub fn order(&self) -> Option<StandingOrder> { self.order }`.

`AbilityDefinition` (`:227–252`) gains, **last**:

```rust
    /// Real-time recovery ticks after use; absent means the round length.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery_ticks: Option<u32>,
    /// Whether a real-time standing order repeats; skipped when false.
    #[serde(default, skip_serializing_if = "is_false")]
    pub repeats_in_real_time: bool,
```

`CombatState` (`:532–539`) gains, **after `active`**, in this order:

```rust
    /// Encounter time policy; omitted when turn-based (ADR-0029 T1).
    #[serde(default, skip_serializing_if = "is_turn_based")]
    pub mode: EncounterMode,
    /// Clock and round length; `None` means TB-only (ADR-0029 T2, T3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clock: Option<EncounterClock>,
    /// The next mode request, applied at a round boundary (ADR-0029 T9).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_mode: Option<EncounterMode>,
} // struct ends here

fn is_turn_based(mode: &EncounterMode) -> bool { *mode == EncounterMode::TurnBased }
fn is_false(value: &bool) -> bool { !*value }
```

`CombatAction` (`:607–623`) gains, **last**: `Stop { actor: EntityId }` (clear the actor's RT order,
T8). `CombatError` (`:652–792`) gains, **last**: `ModeUnsupported` and `WrongMode`, with `Display`
(`:794–833`) exactly `ModeUnsupported at combat/mode` and `WrongMode at combat/mode`.

```rust
pub fn start_encounter_in_mode(
    world: &mut World, spec: &EncounterSpec<'_>, mode: EncounterMode,
) -> Result<(), CombatError>;
pub(crate) fn start_encounter_in_mode_tracked(
    world: &mut World, spec: &EncounterSpec<'_>, mode: EncounterMode,
) -> Result<StartFacts, CombatError>;
pub fn start_encounter(world: &mut World, spec: &EncounterSpec<'_>) -> Result<(), CombatError>;
// unchanged signature; delegates with EncounterMode::TurnBased
pub fn request_mode(world: &mut World, mode: EncounterMode) -> Result<(), CombatError>;
fn execute_use(world: &mut World, actor: EntityId, ability: Ulid, target: EntityId)
    -> ActionOutcome; // crate-visible; shared by perform_action and stage 3

pub(crate) struct TransitionFacts {         // gains two fields
    pub turn_started: Option<(EntityId, u32)>,
    pub encounter_ended: Option<(Ulid, u32)>,
    pub round_started: Option<u32>,          // new
    pub mode_changed: Option<EncounterMode>, // new
}
```

Publication (`:1364–1421`): `mode` = the input; `clock` =
`ruleset.round_ticks.map(|round_ticks| EncounterClock { round_ticks, clock: 0 })`;
`pending_mode = None`; timeline keys are authored initiative in TB (unchanged) and
`InitiativeKey(0)` in RT; `active` is the TB head and `None` in RT; the initial `TurnStart` and
`StartFacts::initial_turn` exist only in TB; `Combatant.order = None`; each ability copies
`recovery_ticks` and `repeats_in_real_time.unwrap_or(false)`. `validate_action` (`:1554`) keeps its
signature and precedence with a mode-aware turn stage; `perform_action` (`:1786`) branches on mode;
`advance_tracked` (`:2156`) gains the TB clock advance and TB→RT switch; `legal_actions` (`:1727`)
branches on mode (B14–B19).

### `crates/crpg-sim/src/tick.rs`

```rust
pub(crate) struct TickFacts {
    pub transitions: TransitionFacts,
    pub events: Vec<TickEvent>,
}
pub(crate) enum TickEvent {
    Resolved(ActionOutcome),        // one RT order resolved through the shared path
    OrderEnded { actor: EntityId }, // one due order cleared as invalid
}
pub(crate) fn tick_tracked(world: &mut World) -> TickFacts;
pub fn tick(world: &mut World);              // unchanged; discards the facts
fn encounter_clock_system(world: &mut World, facts: &mut TickFacts); // new stage 2
fn timeline_system(world: &mut World, facts: &mut TickFacts);        // extended body
```

### `crates/crpg-sim/src/history.rs`

```rust
pub const HISTORY_VERSION: u32 = 2;      // newest written; v1 is still read
const HISTORY_VERSION_V1: u32 = 1;       // private: the initial wrapper version

pub enum HistoryEvent {
    // ...the existing six variants unchanged...
    RoundStarted { round: u64 },
    ModeChanged { mode: EncounterMode, round: u64 },
    OrderEnded { actor: EntityId },
}

impl HistoryWorld {
    pub fn start_encounter_in_mode(&mut self, spec: &EncounterSpec<'_>,
        mode: EncounterMode) -> Result<(), HistoryError>;
    pub fn request_mode(&mut self, mode: EncounterMode) -> Result<(), HistoryError>;
}
```

`new`/`new_in_area` set `version: HISTORY_VERSION_V1` (`:394–411`); `commit` (`:702`) raises it to 2 on
a v2-only event and never lowers it. The `HISTORY_VERSION` doc (`:63–64`) and the version-field doc
(`:379`) change to "newest written; 1 is still read". No new `SimEvent` (ADR-0008); RT executions keep
pushing `Died` into the legacy queue exactly as TB (`:2004–2009`). Add `request_mode`,
`start_encounter_in_mode`, `EncounterClock`, `EncounterMode`, `RT_CLOCK_MAX`, `StandingOrder` to the
combat re-exports (`lib.rs:33–40`); nothing is removed.

## Tick order

One tick after T138 (ADR `:417`: stage 2 added, stage 3 filled; no-encounter and TB ticks unchanged):

1. **Stage 1 — world advance (unchanged).** `advance_tick` saturates the world tick by one (`tick.rs:21–24`,
   `world.rs:711`). No encounter rule reads it (T3).
2. **Stage 2 — `encounter_clock_system` (NEW, T6).** No encounter, `mode != RealTime`, no clock, or `clock.clock ==
   RT_CLOCK_MAX` → return; else `old = clock.clock`, `clock.clock = old + 1`.
3. **Stage 2 — tick deliveries and boundary (NEW, T3/T6).** For every live combatant ascending `EntityId`,
   every pool with `OnTick(period)` where `new / period > old / period` receives one
   `RefreshEvent::Tick(Tick::new(new - new % period))`. If `new % round_ticks != 0` or fewer than two are
   alive → return; else `pending = pending_mode.take()`, `round = round.saturating_add(1)`.
4. **Stage 2 — RT→TB switch or steady RT.** `Some(TurnBased)`: clear every order, `RoundStart`-refresh every
   live combatant ascending `EntityId`, drop expired attachments, rebuild the timeline at stored initiative,
   `active = Some(head)`, one `TurnStart` for the head; facts `mode_changed`, `round_started`,
   `turn_started`. Otherwise: `RoundStart` then `TurnStart`-refresh every live combatant ascending
   `EntityId`, drop expired attachments, `active = None`; fact `round_started`.
5. **Stage 3 — `timeline_system` (the empty body is filled, T7).** Not RT, no encounter, frozen clock, or fewer
   than two alive → return. In TB the existing no-op loop stays documented (`tick.rs:55–60`).
6. **Stage 3 — snapshot.** Take every entry with `u64::try_from(key.0) <= clock`, ascending `(key, EntityId)`,
   **before acting**.
7. **Stage 3 — per due entry.** Not a live non-dead combatant → skip; `order == None` → skip (key kept); the order
   fails RT validation → `order = None`, push `OrderEnded { actor: id }`, skip (key kept); valid →
   `execute_use(..)` on the same kernel resolution, `combat.roll` stream, roll tag, costs, damage, exactly-once
   `Died` and effect attach as TB, then push `Resolved(outcome)`.
8. **Stage 3 — after an execution.** Remove newly dead combatants from the timeline and clear their orders. Fewer
   than two alive → `active = None`, record `encounter_ended` once, stop the snapshot; otherwise, if the actor
   lives, clear a one-shot order and re-key it to `min(clock + recovery, RT_CLOCK_MAX)`, `recovery =
   ability.recovery_ticks.unwrap_or(round_ticks)`.
9. **Unchanged.** `end_turn` (`tick.rs:33–36`) stays pop-only; TB turn logic stays in
   `start_encounter`/`perform_action` (T4).

## Hash and history

Hashed through `state_hash`'s full-`World` serde (`hash.rs:37–47`), no exclusions (ADR-0009;
`crates/crpg-sim/AGENTS.md:306–309`):

| State | JSON key and encoding | Present when |
|---|---|---|
| `CombatState.mode` | `"mode":"real_time"` (snake-case text; `"turn_based"` never written) | `RealTime` |
| `CombatState.clock` | `"clock":{"round_ticks":<u32>,"clock":<u64>}` | `Some` |
| `CombatState.pending_mode` | `"pending_mode":"turn_based"\|"real_time"` | `Some` |
| `Combatant.order` | `"order":{"ability":"<ULID>","target":<entity>,"repeat":<bool>}` | `Some` |
| `AbilityDefinition.recovery_ticks` | `"recovery_ticks":<u32>` | `Some` |
| `AbilityDefinition.repeats_in_real_time` | `"repeats_in_real_time":true` | `true` |

New keys are last in their structs, so a TB-only `CombatState` writes exactly `{"definition":…,"round":…,
"active":…}` and a legacy `Combatant` today's nine keys — absence compatibility shapes, not exclusions
(ADR-0013/0020 pattern, `crates/crpg-sim/AGENTS.md:323–330`).

**History v2 (N5).** Envelopes keep the adjacent `type`/`value` tag with `type` first
(`history.rs:1267–1351`); `round` widens to `u64` as `TurnStarted` does (`:111–116`):

```json
{"type":"round_started","value":{"round":3}}
{"type":"mode_changed","value":{"mode":"real_time","round":3}}
{"type":"order_ended","value":{"actor":<entity>}}
```

Payload field order is `mode` then `round`; input key order is free, but duplicate, unknown and missing
keys fail, and three manual visitors are added. The loader accepts `version` 1 or 2 (`:1446–1448`
becomes a closed pair); a version-1 wrapper with any new tag is rejected, all other v1 input decodes
and hashes exactly as today, a version-2 wrapper accepts both vocabularies, and old events under
version 1 are byte-identical to today.

**Emission order.** In one tick: `ModeChanged` (if a switch applied), `RoundStarted` (if a boundary),
then in due order each `ActionResolved` followed by its `Died` when that action killed, or `OrderEnded`;
finally one `EncounterEnded` if the fight ended. `perform_action`/`despawn` keep their order
(`history.rs:796–814`) extended with `ModeChanged`, `RoundStarted`, then `TurnStarted`/`EncounterEnded`.
RT order-set, `Stop` and a mode request emit nothing (no resolution or boundary happened) but are typed
`HistoryWorld` operations, so replay submits and re-derives them from hashed state (T16). `history_hash`
(`:773–785`) therefore covers mode, clock, pending mode, orders, recovery data, the wrapper version and
the new envelopes.

## Behaviour

1. **B1 (T1)** `CombatState.mode` is the only mode in the crate: no World, area or campaign field; outside an
   encounter no mode exists. `start_encounter_in_mode` stores the input; absent `mode` decodes to `TurnBased`,
   which is never serialized; release drops the mode with the state.
2. **B2 (T1)** A terminal encounter keeps its mode until release; terminal never rewrites `mode` or `pending_mode`.
3. **B3 (T2)** `clock` is `Some` exactly when `ruleset.round_ticks` was `Some(n)`, with `round_ticks = n` and
   `clock = 0`; otherwise TB-only (`mode == TurnBased`, `pending_mode == None`). Ability RT fields are copied
   (`None`/`false` when absent) even in a TB-only encounter, where they are dormant and never read.
4. **B4 (T2)** `start_encounter_in_mode(.., RealTime)` on TB-only returns `ModeUnsupported at combat/mode` before
   any entity exists (order: `EncounterActive`, `ModeUnsupported`, then the existing checks). A TB-only
   `request_mode(RealTime)` returns the same error and changes nothing; `request_mode(TurnBased)` is always
   `Ok`.
5. **B5 (T2)** `round_ticks == Some(0)` panics `ruleset round_ticks is validated nonzero by data read-time checks`;
   `recovery_ticks == Some(0)` panics `ability recovery_ticks is validated nonzero by data read-time checks`. No
   `CombatError` covers these (data owns them; B26 rejects them at load).
6. **B6 (T3)** From clock 0: RT advances 1 per world tick while below `RT_CLOCK_MAX`; TB advances `round_ticks` per
   rollover, clamped. At the bound the clock stops and RT stages 2–3 do nothing (freeze, no wrap, no reorder); no
   RT boundary can occur there, so only commands, requests, `end_encounter` or last-`despawn` change state.
7. **B7 (T3)** For every clock advance `old → new`, each live combatant's pool with `OnTick(period)` is refreshed
   exactly once when `new / period > old / period`, with `RefreshEvent::Tick(Tick::new(new - new % period))`; no
   crossing, no delivery. TB-only encounters have no clock and deliver none.
8. **B8 (T3, T7)** Execution re-keys to `clock + recovery` saturated at `RT_CLOCK_MAX`, `recovery =
   ability.recovery_ticks.unwrap_or(round_ticks)`. A 120-tick cooldown costs 120 RT ticks or one TB round.
   Recovery is a key, never a refresh.
9. **B9 (T4)** TB policy is unchanged for TB-only and RT-capable fights: one entry per live combatant at authored
   initiative, head active, pop on an accepted ending action, rollover on an empty timeline with two or more
   alive. The only TB difference is the RT-capable clock advance (B6); TB-only behaviour and hashes are
   byte-identical.
10. **B10 (T5)** In RT every live non-dead combatant holds exactly one entry: fresh RT start keys all at 0, a TB→RT
    switch at the current clock, execution at `clock + recovery`; keys stay in `0..=i32::MAX`, never wrap and
    freeze at the bound. Iteration is ascending `(key, EntityId)` (`timeline.rs:127–130`).
11. **B11 (T6)** At a tick whose advanced clock is a multiple of `round_ticks` with two or more alive: pending
    switch first, then `round + 1` (saturating), then `RoundStart` (and `TurnStart`) for every live combatant
    ascending `EntityId`, expiry of attachments with `expires_round <= round` (`combat.rs:2179`), and one
    `RoundStarted`. Clock 0, a terminal fight and a frozen clock never yield a boundary.
12. **B12 (T7)** Stage 3 snapshots due entries (`key <= clock`, ascending `(key, id)`) before acting, so each acts
    at most once per tick (recovery ≥ 1). A due combatant without an order keeps its key until the first tick
    after it gets one. A valid order runs the shared path then re-keys, clearing a one-shot order and keeping a
    repeating one; `ends_turn` is ignored in RT.
13. **B13 (T7)** An invalid due order (dead/absent target, unknown ability, forbidden self-target, unaffordable,
    actor gone) is cleared with no spend, draw, damage or effect, keeps its key, and journals one
    `OrderEnded { actor }`. Death removes the entry at once and clears the order; fewer than two alive sets
    `active = None`, journals one `EncounterEnded`, and stops the snapshot.
14. **B14 (T8)** In RT, `UseAbility` from any live combatant (in turn or not) validates as TB with the active-turn
    check replaced by the terminal gate, stores `StandingOrder { ability, target, repeat:
    ability.repeats_in_real_time }` replacing any order, never executes now, changes no key/pool/RNG/event, and
    returns `Ok(None)`. In TB it is exactly today's action and `Stop` fails `WrongMode`. In RT `EndTurn` fails
    `WrongMode`; `Stop` clears the order (`Ok(None)`) and is an accepted no-op when none exists. TB combatants
    never hold orders (start, switch, load).
15. **B15 (T8)** RT `legal_actions` gates on `Stop { actor }` (failure = `LegalActionsError::Actor`), lists every
    accepted `UseAbility` over ascending ability × ascending `EntityId`, then `Stop`; no `EndTurn`.
    `MAX_LEGAL_ACTIONS` counts `Stop` and overflow still fails wholly (`TooManyOptions`). TB enumeration is
    unchanged.
16. **B16 (T9)** `request_mode` needs an encounter (`NoEncounter`) and RT capability for `RealTime`
    (`ModeUnsupported`). Asking for the current mode clears `pending_mode`; asking for the other sets/replaces
    it with that mode. It never changes `mode`/timeline immediately and journals nothing; `pending_mode` is
    hashed.
17. **B17 (T9)** A TB-only encounter never holds `pending_mode`, and a pending value always differs from `mode`. On
    a terminal fight a pending switch never applies and stays hashed until release.
18. **B18 (T10)** RT→TB at the boundary: all orders cleared, timeline rebuilt at stored initiative, head active, one
    `TurnStart` for the head, and the round's single `RoundStart` from B11 (no second). Facts `mode_changed`,
    `round_started`, `turn_started`; history order `ModeChanged`, `RoundStarted`, `TurnStarted`.
19. **B19 (T10)** TB→RT at a TB rollover: clock gets its `round_ticks` advance, every live combatant keyed at the
    new clock, `active = None`, and the round start refreshes `RoundStart` then `TurnStart` for every live
    combatant ascending `EntityId` (the switch schedule replaces the TB one). No `TurnStarted`. History order
    `ModeChanged`, `RoundStarted`.
20. **B20 (T10)** Every round in either mode refreshes each live combatant's `OnRoundStart` pools exactly once;
    toggling every round cannot refill twice (tested with a partial-refill pool, T135).
21. **B21 (T11)** The sim never picks the initial mode: `start_encounter` is TB even on an RT-capable ruleset; only
    `start_encounter_in_mode` selects RT. A fresh encounter has `pending_mode == None`.
22. **B22 (T12)** Only combatants ever hold timeline entries in either mode; non-participants are never keyed or
    touched by stages 2–3 and keep acting in real time because the host keeps ticking. Their movement is
    T034/T036.
23. **B23 (T13)** A second start fails `EncounterActive` **before** `ModeUnsupported`; RT adds no collection or
    second encounter.
24. **B24 (T14)** No join operation is added; B26's completeness rule is RT's only roster structural claim. TB
    first-round/later-round and RT due-now rules are recorded for T139.
25. **B25 (T15)** No AI stage or AI state is added; RT `legal_actions` is the same read-only query (no intern, RNG,
    event or mutation), so host AI between ticks submits ordinary orders/requests and replay never runs AI.
26. **B26 (T16)** All new values are integer, bool, ULID or `EntityId` (no new float; `Transform` stays the only
    one, `transform.rs:20`). Load (`world.rs:149–522`) additionally rejects: zero `round_ticks`; a clock above
    `RT_CLOCK_MAX`; RT mode, a pending mode or an order without a clock; `pending_mode == Some(mode)`; RT with
    `active != None`; RT where the timeline is not exactly the live non-dead combatants; an order whose ability
    is not in the definition, whose `repeat` disagrees with `repeats_in_real_time`, or whose holder is dead;
    `recovery_ticks == Some(0)`.
27. **B27 (N5)** `RoundStarted` once per RT boundary (steady, RT→TB, TB→RT), `ModeChanged` once per applied switch,
    `OrderEnded` once per cleared due order, in the order above. Encodings, versions and v1 rules are exactly
    Hash and history; a v1 wrapper decodes unchanged and rejects the three tags; a wrapper with a v2 event
    writes `"version":2` and never returns to 1; a TB-only wrapper stays 1 and emits no new event.

## Tests

Each item names the Behaviour item(s) it proves; public API unless marked `src/`.

- **RT1 (B1, B2)** `tests/rt_mode.rs`: starts set/omit `mode`, one area holds different modes, release drops it, a
  terminal fight keeps it across actions and a save/load round trip.
- **RT2 (B3, B5)** `tests/rt_mode.rs`: `round_ticks: 120` gives
  `Some(EncounterClock { round_ticks: 120, clock: 0 })`; absent stays absent; ability copies
  `Some(60)`/`true`; both zero values panic with B5's exact messages (`#[should_panic(expected = …)]`).
- **RT3 (B4, B16, B17, B23)** `tests/rt_mode.rs`: TB-only RT start → `ModeUnsupported at combat/mode` with state
  untouched; the request matrix (current/other/repeat, `NoEncounter`, TB-only RT); `pending_mode` hashed and
  never equal to `mode`; `EncounterActive` wins over `ModeUnsupported`.
- **RT4 (B6, B8)** `tests/rt_tick.rs`: 0 → 1 per tick; a TB rollover adds exactly `round_ticks`; clamping and
  freeze at `RT_CLOCK_MAX` (timeline and hashes stop changing); recovery override/default, re-key and clamp.
- **RT5 (B7)** `tests/rt_tick.rs`: `OnTick(1)`, `(5)`, `(7)` over a 120-tick TB round and RT 1-tick advances: one
  delivery per crossing, none without, event tick = last crossed multiple, none in TB-only.
- **RT6 (B9)** `tests/rt_tick.rs`: an RT-capable TB fight matches the TB-only fixture in order, head, pop,
  rollover and terminal; only the clock differs.
- **RT7 (B10, B11)** `tests/rt_tick.rs`: one entry per live combatant after start and switch; `(key, id)` order;
  one round increment and full refresh per boundary; none at clock 0, terminal or frozen.
- **RT8 (B12, B13)** `tests/rt_tick.rs`: due combatants act ascending and once; a due combatant without an order
  waits; an unaffordable/dead-target order clears with `OrderEnded` and no spend; one-shot clears, repeating
  stays; a kill ends the fight with one `EncounterEnded` and stops the snapshot.
- **RT9 (B14, B15)** `tests/rt_orders.rs`: RT `UseAbility` → `Ok(None)`, order set/replaced, no key/pool/RNG/event
  change; `repeat` from data; `Stop` clears and is an empty no-op; `EndTurn` and TB `Stop` → `WrongMode`; RT
  `legal_actions` = abilities × targets then `Stop`, no `EndTurn`, bound counts `Stop`; TB enumeration
  unchanged.
- **RT10 (B18, B19)** `tests/rt_switch.rs`: RT→TB at the boundary (orders cleared, initiative restored, head, one
  `TurnStart`); TB→RT at rollover (all keyed at the new clock, `active == None`, RT round start); a request
  never applies mid-round; `ModeChanged` precedes `RoundStarted`; `TurnStarted` only for RT→TB.
- **RT11 (B20, `src/combat.rs` `#[cfg(test)]`)** A `ResourcePool::with_refill` pool with an empty balance gains
  exactly one refill per round while toggling every boundary.
- **RT12 (B21)** `tests/rt_mode.rs`: `start_encounter` on `round_ticks: 120` starts TB with no pending mode;
  `start_encounter_in_mode(RealTime)` starts RT; neither infers from data.
- **RT13 (B22–B25)** `tests/rt_mode.rs`/`tests/rt_tick.rs`: non-combatants hold no entry and are untouched by 1000
  ticks in either mode; a second start fails `EncounterActive`; no join API; repeated RT `legal_actions` leaves
  state and hashes unchanged.
- **RT14 (B26)** `tests/rt_persistence.rs`: exact bytes for RT `CombatState`/`Combatant`/`AbilityDefinition`;
  round trip to an equal world; each B26 rejection as a raw JSON mutation; a TB-only keys walk proving none of
  the new key names appears.
- **RT15 (B27)** `tests/history_v2.rs`: exact encodings/round trips; duplicate, unknown, missing and
  `type`-after-`value` rejection; v1 accepts old tags and rejects each new tag; a v1 wrapper and hash are
  unchanged; upgrade to 2 on the first RT boundary, sticky after acknowledgement; tick event order. Also
  updates `tests/history.rs:828–834` so the unknown-version case uses 3 (2 is now valid) — not a weakening.
- **RT16 (B6–B13, B27, goldens)** `tests/rt_determinism.rs`: 10 000 frozen-seed ticks give identical per-tick
  `state_hash` sequences and `history_hash` on two independent builds; the new per-target RT history golden and
  `crates/crpg-testkit/tests/rt_replay.rs` are compared, never blessed.
- **RT17 (B9, B26, compat)** Existing suites run unchanged: `tests/combat.rs`, `tests/combat_persistence.rs`,
  `tests/history.rs`, `tests/history_golden.rs`, `tests/legal_actions.rs`, `tests/tick.rs`,
  `crates/crpg-testkit/tests/*`, `crates/crpg-cli/tests/replay.rs` — the TB byte-identity proof.

## Goldens and approvals
Every existing golden stays **byte-identical**; none is re-blessed or edited:

- `crates/crpg-sim/tests/goldens/history_v1.x86_64-pc-windows-msvc.txt` and `...x86_64-unknown-linux-gnu.txt`:
  unchanged — the shared TB schedule (`tests/support/history_schedule.rs`) emits no v2-only event, so lazy
  versioning keeps `version: 1` and every step hash identical (ADR `:425`, N5 `:347–349`).
- `crates/crpg-testkit/goldens/*` and the CLI replay fixtures: TB-only worlds, no new key, no hash moves; the
  `combat_persistence` exact-wire cases are unchanged.
- **New** baselines, per native target (ADR-0012), authored by the existing pattern: an RT history schedule with
  `tests/goldens/history_v2.<target>.txt` and a testkit RT replay golden, covering B6–B13, B18–B20, B27. New
  baselines are not re-blesses; a missing baseline fails rather than passing.
- **Proof of TB identity:** RT14's keys walk asserts a TB-only `World`'s serialized `combat`/`combatants` contain
  none of `mode`, `clock`, `pending_mode`, `order`, `recovery_ticks`, `repeats_in_real_time`, and every existing
  golden test passes unchanged.

If any *existing* golden moves that is a stop, not a re-bless; the approval asked here is only that the new
per-target baselines be authored.

## Out of scope

- Any code in this task; pause/multiplayer authority (ADR §3, T141); protocol and N1–N4 (T140); persistent
  creature state and join/leave (T139); T137; `crpg.ui/1`.
- Movement (T034/T036), reactions/interrupts, rest refresh, partial-refill `refill` values entering the sim
  adapter.
- `EncounterSpec` changes (D9) and any other crate — **except** Q1's mechanical line:
  `crates/crpg-script/src/bindings.rs:300` must add `CombatAction::Stop { actor }` to its or-pattern.
- `SimEvent` additions; protocol/host disclosure of orders (T027b/T140).

## Size budget

`src/combat.rs` ≈ +330/−40; `src/tick.rs` ≈ +200/−20; `src/world.rs` ≈ +140/−10; `src/history.rs` ≈ +260/−30;
`src/lib.rs` +2; tests ≈ +1400 across six suites; two schedules and two golden tables; docs ≈ +30 each.

## Open questions (each with a recommendation)

1. **`Stop` costs one downstream line.** `crates/crpg-script/src/bindings.rs:300` must add the variant to its
   or-pattern (T017d precedent for a user-authorized mechanical fix). *Recommend:* accept it; a separate
   `stop_order` would duplicate admission and stay out of `legal_actions`.
2. **Initial-mode API.** *Recommend:* additive `start_encounter_in_mode` (`start_encounter` = TB); an
   `EncounterSpec` field edits five crates.
3. **`RoundStarted` in TB.** *Recommend:* RT-only (D11); TB history identity is an ADR requirement.
4. **Lazy version vs always 2.** *Recommend:* lazy (D14); always-2 moves every TB history hash and the v1 goldens.
5. **Behaviour at `RT_CLOCK_MAX`.** *Recommend:* freeze silently (D4); an error needs a new in-tick channel for a
   state reached after ~3.4 years of continuous RT.
6. **`OnTick` in TB-only encounters.** *Recommend:* none (no clock; they never refilled before T138); a TB-only
   cooldown is a separate data/kernel decision.
7. **RT terminal rejection.** *Recommend:* reuse `OutOfTurn { actor, active: None }` (terminal TB already rejects
   that way) instead of adding an `EncounterOver` variant.
8. **One package or two.** *Recommend:* two under this contract: T138a (`CombatState`/orders/tick/load
   coherence, RT1–RT14) and T138b (history v2, goldens, RT15–RT17), because the budget is well over one session.
9. **RT fields on a TB-only ruleset.** *Recommend:* copy them as dormant values (B3); T136a adds no semantic rule
   (`tasks/T136.md:90–92`), so sim must not invent one.
10. **`TurnStarted` for RT execution.** *Recommend:* no. RT has no active turn (N1 lists "active combatant (TB)");
    RT's per-round fact is `RoundStarted`.

## Agent log

- 2026-10-23 (UTC) · opencode-go/deepseek-v4.1-flash + T138 draft · Wrote the independent T138 proposal:
  per-encounter mode and clock, RT orders and stage 2/3 policy, history v2 with lazy versioning, and a
  byte-identity argument for every existing TB golden.

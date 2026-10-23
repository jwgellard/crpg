# T138-draft-a — encounter mode and real-time policy (`crpg-sim`, ADR-0029 Follow-on 5)

Blind draft against `docs/adr/0029-kernel-range-and-time-model.md` §"2. Time model" (T1–T16)
and N5. Nothing here is approved; a Claude architect adjudicates it against the rival draft.

## Task / Crate(s) / Status

Sole crate: `crpg-sim` (`src/combat.rs`, `src/tick.rs`, `src/history.rs`, new tests).
S-set (`World` and tick order): serialized, human-reviewed.
Consumes the T136a data fields already on this base — `Ruleset.round_ticks`
(`crates/crpg-data/src/combat.rs:247`), `Ability.recovery_ticks`
(`crates/crpg-data/src/combat.rs:320`), `Ability.repeats_in_real_time`
(`crates/crpg-data/src/combat.rs:323`) — with data read-time zero guards
(`crates/crpg-data/src/combat.rs:774`, `crates/crpg-data/src/combat.rs:821`).
Status: **proposed draft**.

## Decision

- **D1. `EncounterMode { TurnBased, RealTime }` in `CombatState`, default TB.**
  Per-encounter (T1), hashed, snake-case serde. Rejected: per-World/area/campaign
  mode and host-held mode (ADR alternatives: split parties need RT beside TB;
  behaviour-affecting mode must hash per ADR-0013 Decision 1).
- **D2. Capability gate at `start_encounter`: RT on a TB-only ruleset fails
  `ModeUnsupported` before any other adapter check except `EncounterActive`,
  with no mutation.** Rejected: silently falling back to TB (a host asking for
  RT must hear that it cannot have it).
- **D3. `clock: u64` in `CombatState`, skipped when 0; TB-only encounters keep
  it 0 forever.** RT ticks `+1`; TB rollovers `+= round_ticks` (RT-capable only).
  Rejected: reading the world tick for encounter rules (T3 names the clock).
- **D4. `MAX_ENCOUNTER_CLOCK: u64 = i32::MAX as u64`; the clock saturates, never
  wraps; re-keys saturate at the same bound; a saturated tick skips the advance
  (and its boundary) but stage 3 still runs.** Rejected: wrapping (reorders the
  timeline) and freezing execution (strands due orders).
- **D5. One legality source extended, not duplicated: `validate_action`
  (`crates/crpg-sim/src/combat.rs:1554`) branches on mode — RT `UseAbility`
  skips only the `OutOfTurn` check; `EndTurn` in RT and `Stop` in TB fail
  `WrongMode`.** Stage 3 calls `validate_action` directly. Rejected: a second
  RT predicate (violates the T028 invariant at `crates/crpg-sim/AGENTS.md`).
- **D6. `request_mode(world, mode)` sets `pending_mode`; it is not a
  `CombatAction`.** Mode requests are host authority, not combatant actions, and
  must not appear in `legal_actions`. Rejected: a `RequestMode` action variant
  (AI enumeration would offer authority it does not hold).
- **D7. `RoundStarted` journals from RT boundaries only, never from TB
  rollover.** TB history streams stay event-identical across the version bump.
  Rejected: uniform round journaling (adds events to every existing TB flow).
- **D8. `OrderEnded` journals on every `Some → None` order transition
  (one-shot executed, invalid cleared, `Stop`) and never on `Some → Some`
  replacement.** Rejected: journaling replacements (no order ended) and
  journaling order sets (setting is not resolving; replay re-issues the command).
- **D9. `start_encounter` takes the mode as a new `EncounterSpec.initial_mode`
  field, not a new function.** One path, no dual-entry drift. Rejected:
  `start_encounter_in_mode` (two entries rot apart; T017d already paid the
  breaking-API price once with authorization).
- **D10. New `AbilityDefinition` RT fields are `Option`, skipped when `None`;
  `None` also means TB-only.** Legacy and TB-only multi-ability bytes stay
  identical. Rejected: non-optional fields with sentinels (every TB-only
  multi-ability golden moves).
- **D11. `OnTick(n)` delivery lives in the stage-2 clock system via the
  crossing rule, for RT ticks and TB rollover jumps alike** (the T135-deferred
  delivery: `RefreshEvent::Tick`,
  `crates/crpg-rules/src/resource.rs:71`, matched at
  `crates/crpg-rules/src/resource.rs:211`). Rejected: leaving `OnTick`
  undelivered (T3 makes the clock the delivery mechanism).
- **D12. `end_turn` keeps its container semantics in RT and the RT controller
  never calls it.** It is mode-agnostic population mechanics, not policy.
  Rejected: mode-checking `end_turn` (policy belongs in the controller).

## Interface (`crates/crpg-sim/src/*.rs`)

`combat.rs` — new mode vocabulary (after `COMBAT_ROLL_TAG`,
`crates/crpg-sim/src/combat.rs:57`):

```rust
/// Encounter time policy (ADR-0029 T1). Per encounter, hashed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EncounterMode { TurnBased, RealTime }

/// Highest encounter-clock value that still keys the `i32` timeline
/// (`crates/crpg-sim/src/timeline.rs:25`); the clock saturates here (T5).
pub const MAX_ENCOUNTER_CLOCK: u64 = i32::MAX as u64;

/// One RT standing order (ADR-0029 T8). Hashed; absent when `None`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct StandingOrder {
    pub ability: Ulid,
    pub target: EntityId,
    pub repeat: bool,
}
```

`Combatant` (`crates/crpg-sim/src/combat.rs:74`) gains, as its last field:

```rust
    /// RT standing order; always `None` in TB (T8).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order: Option<StandingOrder>,
```

plus `pub fn order(&self) -> Option<StandingOrder>` beside `initiative`
(`crates/crpg-sim/src/combat.rs:97`).

`AbilityDefinition` (`crates/crpg-sim/src/combat.rs:227`) gains, as its last
fields:

```rust
    /// Resolved RT recovery ticks (authored value, else the ruleset
    /// `round_ticks`); `None` in TB-only encounters (T2, T7).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery_ticks: Option<u32>,
    /// Whether the RT order repeats; `None` means false (T8).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repeats_in_real_time: Option<bool>,
```

`CombatDefinition` (`crates/crpg-sim/src/combat.rs:263`) gains, as its last
field:

```rust
    /// Ticks per RT round, copied from the authored ruleset; `None` is
    /// TB-only (T2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub round_ticks: Option<u32>,
```

`CombatState` (`crates/crpg-sim/src/combat.rs:532`) gains, in order after
`active` (`crates/crpg-sim/src/combat.rs:538`):

```rust
    /// Time policy of this encounter (T1).
    #[serde(default, skip_serializing_if = "EncounterMode::is_turn_based")]
    pub mode: EncounterMode,
    /// Encounter clock (T3). 0 until the first RT advance or TB rollover
    /// of an RT-capable encounter; always 0 when TB-only.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub clock: u64,
    /// Requested mode awaiting a round boundary (T9).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_mode: Option<EncounterMode>,
```

with `impl EncounterMode { const fn is_turn_based(&self) -> bool }` and a
private `fn is_zero(v: &u64) -> bool`. `EncounterMode::default()` is
`TurnBased`.

`EncounterSpec` (`crates/crpg-sim/src/combat.rs:558`) gains, as its last
field:

```rust
    /// Host-chosen initial mode; the sim never chooses (T11).
    pub initial_mode: EncounterMode,
```

`CombatAction` (`crates/crpg-sim/src/combat.rs:608`) gains:

```rust
    /// Clear the actor's standing order (RT only; idempotent).
    Stop {
        /// The acting combatant; must be a live combatant.
        actor: EntityId,
    },
```

`CombatError` (`crates/crpg-sim/src/combat.rs:652`) gains, after
`AreaMismatch` (`crates/crpg-sim/src/combat.rs:786`):

```rust
    /// RT was requested for a ruleset without `round_ticks`.
    ModeUnsupported,
    /// `EndTurn` in RT, or `Stop` in TB.
    WrongMode {
        /// The acting entity.
        actor: EntityId,
    },
```

`Display`: `ModeUnsupported at spec/ruleset`, `WrongMode at combat/mode`,
matching the hide-fields style (`crates/crpg-sim/src/combat.rs:809`,
`crates/crpg-sim/src/combat.rs:830`). `start_encounter` precedence becomes:
`EncounterActive`, then `ModeUnsupported`, then the existing order
(area checks keep their site at `crates/crpg-sim/src/combat.rs:985`).

```rust
/// Host-authorized mode request (T9): sets, replaces, or clears
/// (`mode == current` clears) `pending_mode`. Fails `NoEncounter`
/// (`crates/crpg-sim/src/combat.rs:654`) with no mutation; fails
/// `ModeUnsupported` when RT is requested on a TB-only encounter.
/// Mutates nothing else, emits no legacy event, touches no RNG.
pub fn request_mode(world: &mut World, mode: EncounterMode) -> Result<(), CombatError>;
```

`tick.rs`: `run_systems` (`crates/crpg-sim/src/tick.rs:43`) becomes two
systems in spec §10 stage order:

```rust
fn run_systems(world: &mut World) {
    // §10 stage 2 (encounter clock) — T138: RT advance + boundary.
    encounter_clock_system(world);
    // §10 stage 3 (timeline order) — the only stage with a system yet.
    timeline_system(world);
}
```

`history.rs`: `HISTORY_VERSION` (`crates/crpg-sim/src/history.rs:64`) becomes
2; `HistoryEvent` (`crates/crpg-sim/src/history.rs:77`) gains, after
`EncounterEnded` (`crates/crpg-sim/src/history.rs:118`):

```rust
    /// An RT round boundary ran (RT only, T6/N5).
    RoundStarted {
        /// The new combat round, widened as in `TurnStarted`
        /// (`crates/crpg-sim/src/history.rs:111`).
        round: u64,
    },
    /// A pending mode applied at a round boundary (T10/N5).
    ModeChanged {
        /// The mode now in force.
        mode: EncounterMode,
        /// The round the switch applied in, widened as above.
        round: u64,
    },
    /// A standing order cleared without replacement (T7–T8/N5).
    OrderEnded {
        /// The combatant whose order cleared.
        actor: EntityId,
    },
```

plus the typed wrapper method:

```rust
impl HistoryWorld {
    /// Journals nothing (like explicit release); replay re-issues the call.
    pub fn request_mode(&mut self, mode: EncounterMode) -> Result<(), HistoryError>;
}
```

## Tick order

One `tick` (`crates/crpg-sim/src/tick.rs:21`): `advance_tick`
(`crates/crpg-sim/src/world.rs:711`, saturating +1), then `run_systems`:

1. **Stage 2 — NEW `encounter_clock_system`.** No encounter, TB-only
   (`round_ticks == None`), or TB mode: no-op. RT mode:
   a. `old = clock`; `clock = min(clock + 1, MAX_ENCOUNTER_CLOCK)`; if the
      clock did not move, stop (frozen at the bound, D4).
   b. `OnTick` delivery (D11): for every live combatant pool with
      `OnTick(n)` (`crates/crpg-rules/src/resource.rs:48`), if
      `new_clock / n != old_clock / n` deliver
      `RefreshEvent::Tick(Tick::new(new_clock))`
      (`crates/crpg-rules/src/resource.rs:71`,
      `crates/crpg-core/src/time.rs:31`); at least one multiple crossed
      means exactly one delivery (T3).
   c. If `new_clock % round_ticks != 0`, stop.
   d. Boundary (T6): if `pending_mode == Some(TurnBased)`, apply the RT→TB
      switch (§T10 below) and stop — the switch consumes the boundary
      (no `RoundStarted`). Else `round = round.saturating_add(1)` (as
      `crates/crpg-sim/src/combat.rs:2169`); `RoundStart` then `TurnStart`
      refreshes to every live combatant in ascending `EntityId` order
      (mirroring `crates/crpg-sim/src/combat.rs:2172` and
      `crates/crpg-sim/src/combat.rs:2201`); effects expire by round as now
      (`expires_round` bound at `crates/crpg-sim/src/combat.rs:2124`);
      journal one `RoundStarted { round }` (D7).
2. **Stage 3 — `timeline_system`** (`crates/crpg-sim/src/tick.rs:55`).
   No-encounter and TB: unchanged (iterate standing entries, no effect).
   RT: snapshot entries with key `<= clock` in ascending `(key, id)` order
   (`crates/crpg-sim/src/timeline.rs:128`) before acting. For each entry
   whose combatant is still alive:
   a. No standing order: keep its key (acts on the first tick after it
      receives one).
   b. Standing order present: `validate_action` the equivalent TB
      `UseAbility` (D5 — no turn check). If valid, execute through the same
      resolution, cost (`spend_costs`,
      `crates/crpg-sim/src/combat.rs:2084`), damage, effect (`attach_effect`,
      `crates/crpg-sim/src/combat.rs:2109`), RNG stream (`combat.roll`,
      `crates/crpg-sim/src/combat.rs:55`, tag interned at
      `crates/crpg-sim/src/combat.rs:1841`) and journal path as TB
      (`ActionResolved` + drained `Died`s, no `TurnStarted` — RT has no
      turns); then re-key the actor to
      `min(clock + recovery_ticks, MAX_ENCOUNTER_CLOCK)` (recovery is a key,
      not a refresh); clear a one-shot order (`OrderEnded`, D8), keep a
      repeating one. If invalid, clear it and journal `OrderEnded`.
   c. Death removes the entry at once, as in TB (advance at
      `crates/crpg-sim/src/combat.rs:2149`).
   d. Terminal rules are the TB rules (`active = None` sites at
      `crates/crpg-sim/src/combat.rs:2045`, `:2226`, `:2271`): after each
      execution, fewer than two alive ends the encounter
      (`EncounterEnded`, first transition only).
3. **TB rollover (in `perform_action`, unchanged policy T4)** gains, in this
   order: the TB→RT pending check (at the existing rollover,
   `crates/crpg-sim/src/combat.rs:2169` — key every live combatant at
   `clock`, i.e. due now, then the T6 round start, §T10 below); the
   `OnTick` crossing delivery for the `clock += round_ticks` jump (D11);
   then the existing `round + 1` / `RoundStart` / re-add-at-initiative path.
   No `RoundStarted` journals here (D7).

§T10 switch application (both directions journal `ModeChanged { mode, round }`
with the post-increment round; exactly one round-start refresh per round):
- RT→TB (at the T6 boundary): discard all orders silently (no `OrderEnded`);
  rebuild the timeline from live combatants at stored initiative (precedent:
  roll tag at `crates/crpg-sim/src/combat.rs:1402`, incoming-actor `TurnStart`
  at `crates/crpg-sim/src/combat.rs:1409`); then the normal TB round start.
- TB→RT (at TB rollover): key every live combatant at `clock` (due now);
  then the T6 round start (also journals `RoundStarted` — now RT).
- Terminal encounters only flip the mode, clear `pending_mode`, and journal
  `ModeChanged` with the unchanged round: no increment, refresh, or rebuild.

`end_turn` (`crates/crpg-sim/src/tick.rs:33`) is untouched (D12).

## Hash and history

- Hashed when present, absent otherwise: `mode` (skip when TB),
  `clock` (skip when 0), `pending_mode` (skip when `None`), `order` (skip
  when `None`), `CombatDefinition.round_ticks` (skip when `None`),
  `AbilityDefinition.recovery_ticks` / `repeats_in_real_time` (skip when
  `None`). `state_hash` (`crates/crpg-sim/src/hash.rs:37`) is unchanged code:
  BLAKE3 over canonical JSON (`crates/crpg-sim/src/hash.rs:48`) with the
  finite-float assert (`crates/crpg-sim/src/hash.rs:41`). Integers only —
  no float is added (T16).
- `history_hash` (`crates/crpg-sim/src/history.rs:773`) covers the wrapper
  unchanged in code; new events ride the existing adjacent `type`/`value`
  envelopes (`crates/crpg-sim/src/history.rs:132`) and the pinned
  `commit` validation order (`crates/crpg-sim/src/history.rs:702`), with the
  256-byte string cap (`crates/crpg-sim/src/history.rs:62`) applying to no
  new field (mode is an enum, rounds/actors are integers).
- Per-tick emission order: stage-2 facts first (`ModeChanged` if a switch
  applied, else `RoundStarted` if a boundary ran), then stage-3 facts in
  execution order (`ActionResolved`, drained `Died`s, `OrderEnded` each).
- `HISTORY_VERSION` 2: the loader accepts 1 and 2
  (`crates/crpg-sim/src/history.rs:1446` gains the `<= 2` bound) and writes
  2; the three new variants decode only under version 2 (unknown/missing
  keys stay rejected); v1 journals and goldens stay readable and unchanged.

## Behaviour

1. **T1 mode per encounter.** `CombatState.mode` defaults TB; TB-mode worlds
   omit it on the wire; two encounters in sequence may use different modes.
2. **T1 no other mode.** No World/area/campaign mode field exists; outside an
   encounter nothing reads `mode`.
3. **T2 capability gate.** `start_encounter` with `initial_mode == RealTime`
   on a ruleset without `round_ticks` fails `ModeUnsupported` after the
   `EncounterActive` check, before collection bounds; world, RNG, events and
   sequence unchanged.
4. **T2 TB-only absence.** A TB-only encounter persists and hashes exactly as
   today (all six new fields absent); a TB-mode encounter on an RT-capable
   ruleset persists `round_ticks` (and resolved per-ability fields) but no
   mode/clock/pending/order.
5. **T3 RT clock.** Each RT tick advances `clock` by exactly 1 until
   `MAX_ENCOUNTER_CLOCK`, where it saturates; encounter rules never read
   `World::tick` (`crates/crpg-sim/src/world.rs:699`).

6. **T3 TB clock.** Each TB rollover of an RT-capable encounter adds   `round_ticks` (saturating `u64`); TB-only clocks stay 0.
7. **T3 `OnTick(n)`.** Delivery happens exactly when the advance crosses at
   least one multiple of `n` (`new/n != old/n`), once per advance, in RT and
   across TB rollover jumps; a 120-tick cooldown lasts 120 RT ticks or one TB
   round.
8. **T4 TB unchanged.** One entry per live combatant at initiative, head acts,
   accepted turn-ending action pops, empty timeline with 2+ alive rolls over —
   the existing suites pass unmodified.
9. **T5 RT keys.** In RT each live combatant holds exactly one entry keyed by
   its ready clock; re-key is `min(clock + recovery, MAX)`; saturation never
   wraps or reorders; due means key `<= clock`.
10. **T6 boundary.** At `clock % round_ticks == 0`: pending RT→TB applies
    first; else `round + 1` saturating, `RoundStart`+`TurnStart` refreshes in
    ascending `EntityId` order, round-based expiry as now, one `RoundStarted`.
11. **T7 execution.** Due snapshot in ascending `(key, id)` order, taken before
    acting; late-arriving entries from this tick's own executions never join
    the pass. Valid orders execute through the TB path (same stream, tag,
    costs, damage, effects, `ActionResolved`+`Died`s, no `TurnStarted`); the
    actor re-keys; one-shots clear (`OrderEnded`), repeating persist.
12. **T7 invalid and orderless.** Invalid due orders clear with `OrderEnded`;
    due combatants without orders keep their keys; death removes the entry
    at once; terminal follows the TB rule.
13. **T8 orders.** In RT, `UseAbility` from any live combatant validates fully
    (D5) and stores/replaces the order, returning `Ok(None)`; it never
    executes at once. `Stop` clears (`Ok(None)`, idempotent — no order is
    still `Ok`). `EndTurn` in RT fails `WrongMode`; `Stop` in TB fails
    `WrongMode`; orders never exist in TB (`order` stays `None`).
14. **T9 switching.** `request_mode` on an active encounter sets `pending_mode`;
    a later request replaces it; requesting the current mode clears it; RT on
    a TB-only ruleset fails `ModeUnsupported`; `NoEncounter` with no encounter.
    Rejection mutates nothing (RNG, events, sequence included).
15. **T10 applying.** Both directions run exactly one round start (no early
    refill), journal `ModeChanged { mode, round }` with the post-increment
    round, and — RT→TB only — discard orders silently then rebuild at stored
    initiative; TB→RT keys all live due now and also journals `RoundStarted`.
    Terminal encounters flip mode with `ModeChanged` and nothing else.
16. **T10 no double refill.** Toggling every round yields exactly one
    round-start refresh per round (counted test).
17. **T11 initial mode.** `start_encounter` reads `EncounterSpec.initial_mode`;
    no path in sim chooses a mode; `HistoryWorld::start_encounter`
    (`crates/crpg-sim/src/history.rs:523`) passes it through.
18. **T12 split parties.** Non-participants hold no timeline entries
    (`World::spawn`, `crates/crpg-sim/src/world.rs:609`, is encounter-blind);
    ticks advance them in real time beside either mode; TB participants move
    only through their own actions.
19. **T13 one encounter.** Second `start_encounter` still fails
    `EncounterActive` (`crates/crpg-sim/src/combat.rs:656`); release is still
    `end_encounter` (`crates/crpg-sim/src/combat.rs:2288`) with no new events.
20. **T14 join precondition.** No join operation is added; spawning during an
    encounter mints a non-combat, unscheduled entity (T139's insertion rules
    TB-first-round / TB-later / RT-due-now will build on this).
21. **T15 AI outside.** No AI stage in `run_systems`; RT `legal_actions`
    (`crates/crpg-sim/src/combat.rs:1727`) enumerates for any live combatant —
    accepted `UseAbility`s over abilities × combatants in the existing
    ascending orders, plus `Stop` iff the combatant holds an order, never
    `EndTurn` — under the unchanged `MAX_LEGAL_ACTIONS` bound
    (`crates/crpg-sim/src/combat.rs:53`); execution always revalidates.
22. **T16 replay.** Clock, keys, recovery and rounds are integers; no float
    added; every World change is a journaled operation (`tick`, command, mode
    request, start, end); pause, timers and AI have no sim surface.
23. **N5 `RoundStarted`.** RT boundaries only (D7); `{ round: u64 }` widened
    like `TurnStarted`; adjacent-tagged; 256-envelope pages
    (`crates/crpg-sim/src/history.rs:60`) and ack rules unchanged.
24. **N5 `ModeChanged` / `OrderEnded`.** `{ mode, round: u64 }` on switch
    application only (never at start); `{ actor }` on every `Some → None`
    transition, never on replace; `request_mode` itself journals nothing.

## Tests

New files (implementer creates; every item names its Behaviour):
- `tests/encounter_mode.rs` — **M1** (B1,B2: default TB wire omits mode;
  second encounter may differ) · **M2** (B3,B4: `ModeUnsupported` precedence
  after `EncounterActive`, no-mutation incl. RNG) · **M3** (B4: TB-only world
  byte-identical to today — canonical JSON equality with a pre-T138 oracle).
- `tests/realtime_tick.rs` — **R1** (B5,B6: 120-tick round at `round_ticks: 120`,
  `clock`/`round` exact per tick) · **R2** (B7: `recovery_ticks` fallback to
  `round_ticks`, explicit override, one-shot vs `repeats_in_real_time`) ·
  **R3** (B7: due-snapshot frozen — execution-spawned entries wait) ·
  **R4** (B11,B12: order set/replace/`Stop`/`WrongMode`s, `Ok(None)` shapes) ·
  **R5** (B5: saturation at `MAX_ENCOUNTER_CLOCK` — frozen clock, still
  executing, never wrapped) · **R6** (B7: `OnTick` crossing — 120-cooldown is
  120 RT ticks and one TB round) · **R7** (B12: non-participant ticks beside
  both modes; TB participant acts only via actions).
- `tests/mode_switch.rs` — **S1** (B14: set/replace/clear pending,
  `ModeUnsupported`, `NoEncounter`, no-mutation) · **S2** (B15 RT→TB: orders
  discarded, initiative rebuild, `ModeChanged`, no `RoundStarted`) ·
  **S3** (B15 TB→RT: keyed due now, `RoundStarted` + `ModeChanged`) ·
  **S4** (B16: toggle-every-round counts exactly one refresh per round) ·
  **S5** (B15: terminal flip journals `ModeChanged` only) · **S6** (B17:
  `initial_mode` TB and RT starts; sim never chooses).
- `tests/history_v2.rs` — **H1** (B23,B24: RT tick emission order;
  TB tick journals nothing new) · **H2** (B24: replace journals nothing;
  **H3** (B20: v1 journal decodes; new variants rejected under v1; wrapper
  writes version 2) · **H4** (B4: TB-only event stream equals the v1 stream
  modulo the version tag).
- Existing suites pin T4/TB-compat: `tests/tick.rs`
  (`real_time_ticks_keep_standing_order`), `tests/combat.rs`,
  `tests/combat_multi.rs`, `tests/legal_actions.rs`,
  `tests/combat_persistence.rs`, `tests/history.rs`.

## Goldens and approvals

- Every existing `state_hash` golden (testkit `replay`/`combat_replay`/
  `replay_golden`/`srd_replay`, cli `replay`) stays byte-identical: TB-only
  worlds omit all six new fields (absence-skip, the ADR-0013/ADR-0020 pattern
  at `crates/crpg-sim/src/world.rs:78`, `:81`, `:87`, `:94`), so their
  canonical bytes are unchanged; a test (M3) proves it. A golden that moves
  is a stop, not a re-bless (ADR compatibility table,
  `docs/adr/0029-kernel-range-and-time-model.md:425`).
- New per-target goldens (ADR-0012, exact-build only): an RT replay golden
  (RT start, orders, 240 ticks, one switch each way) and a history-v2 golden
  from the same script; generated independently per native target, never
  compared across targets.
- `history_v1` goldens are retained as decode-compatibility fixtures (H3);
  no v1 file is rewritten.

## Out of scope

Any code (this is a contract draft); pause and multiplayer authority (ADR §3,
T141); the protocol (T140, N1–N4); persistent creature state and join/leave
beyond the B20 precondition (T139); T137 adapter generalization; `crpg.ui/1`
(U1–U10); AI behaviour itself (T15's host side).

## Size budget

`src/combat.rs` ≈ +150/−20; `src/tick.rs` ≈ +60; `src/history.rs` ≈ +80;
tests ≈ 450 across four files. One implementer session plus adjudication
review; a moving golden is a stop, not a re-bless.

## Open questions (each with a recommendation)

1. **Bound value.** Is saturating at `i32::MAX` (≈3.4 yr at 20 Hz) the right
   fail-closed bound? Recommend yes: keys are `i32`
   (`crates/crpg-sim/src/timeline.rs:25`), so a wider clock buys nothing.
2. **`Stop` with no order: `Ok` or error?** Recommend idempotent `Ok(None)`
   (B13): hosts and AI clear unconditionally without probing first.
3. **Should TB rollover journal `RoundStarted`?** Recommend no (D7): TB flows
   already have per-turn `TurnStarted` granularity and every existing TB
   history stream stays event-identical.
4. **Should order set/replace journal anything?** Recommend nothing (D8):
   the command log carries the operation; events stay facts about resolution.
5. **`request_mode` on a terminal encounter: allow or reject?** Recommend allow
   with minimal application (B15): uniform precondition (encounter active),
   no round/refresh side effects to exploit.
6. **Field or function for the initial mode?** Recommend the
   `EncounterSpec.initial_mode` field (D9): one construction site pattern,
   no second entry point to rot.
7. **Second engagement joining the existing encounter (T13)?** Recommend
   leaving it to T139 with B20 as its precondition.

## Agent log

- 2026-10-23 (UTC) · opencode-go/muse-spark-1.3-contributor + T138 draft · Wrote the blind T138 exact contract (mode, clock, RT execution, orders, switching, history v2) from the T138 fact sheet against ADR-0029 T1–T16/N5 for architect adjudication.

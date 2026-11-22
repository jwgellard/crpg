# T157 — walk, then fight: exploration into combat, movement in combat (blind draft, Beam)

**Status: draft (attempt 1, blind).** Written by Beam without an architect pass.
Nothing here is binding; an architect adjudicates later.

## Summary

When this is done, the player opens one scene and plays one game. They walk a
party of heroes around a 3D area in real time (the `Walk.tscn` scene today):
left-click the ground to move a hero, right-click to stop. Somewhere in the
area an enemy is waiting. When the party gets close enough to that enemy (or
clicks it), the same scene switches from "walking" to "fighting" without
leaving the place: the camera stays, the heroes stay where they are, and the
turn order starts. In the fight the player clicks an enemy to attack (as in
T156) and also clicks the ground to move the active hero, spending a
movement allowance that refills each turn. Health bars, damage numbers and hit
flashes from T156 keep working. When the fight ends, control returns to
walking in the same place. The enemy, both while walking and while fighting,
is played by the same server-side AI the dedicated server uses, running in the
background host; the client never decides a rule.

## What exists

Facts relied on, each with a `path:line` I opened and checked.

- Walk scene today is real-time only: `apps/client/scripts/Walk.gd:32` opens
  `session.open_v4_rt(...)` against `campaigns/demo/walk`; `:60–74` handles
  left/right click for move/halt; `:41–57` polls, syncs proxies and draws
  positions each frame.
- Combat scene today is turn-based only: `apps/client/scripts/Main.gd:19`
  opens `session.open(...)` (v2) against `campaigns/fixtures/combat_basic`;
  T156 switches it to `open_v4_ai`. T156's `Pick.gd` and `Feedback.gd` are
  the click-to-attack and feedback layer (`tasks/T156.md:298–339`).
- Real-time AI open does not exist yet: `BridgeCore` has `open_v4`
  (`crates/crpg-godot/src/core/mod.rs:173`) and `open_v4_rt`
  (`:210`) but no `open_v4_rt_ai`; T156a adds `open_v4_ai` (turn-based)
  (`tasks/T156.md:225–229`). `NpcHook`'s gate is the NPC peer's legal options,
  which are always empty in real time (`tasks/T156.md:198–201`).
- Mode is per encounter and switchable: `CombatState.mode`
  (`crates/crpg-sim/src/combat.rs:849`); `request_mode`
  (`crates/crpg-sim/src/combat.rs:4375`) sets `pending_mode`; applies at a
  round boundary, RT→TB at the T6 boundary (`docs/adr/0029-kernel-range-and-time-model.md:235`).
- Initial mode is a host input: `start_encounter_in_mode`
  (`crates/crpg-sim/src/combat.rs:1454`) publishes in a host-chosen mode and
  returns `ModeUnsupported` when RT is requested without `round_ticks`.
- The walk ruleset supports RT and movement: `round_ticks: 100` and a
  `movement` block (`campaigns/demo/walk/rulesets/minimal-d6.json`); the
  combat_basic ruleset has neither (`campaigns/fixtures/combat_basic/rulesets/minimal-d6.json`).
- Movement in turn-based combat exists: `IntentBody::DeclareMove`
  (`crates/crpg-net/src/protocol_v4.rs:118`), `DeltaOp::Moved`
  (`crates/crpg-net/src/protocol_v4.rs:160`), `Host::host_move`
  (`crates/crpg-server/src/host.rs:4708`) and `BridgeCore::declare_move`
  (`crates/crpg-godot/src/core/mod.rs:269`).
- Real-time movement exists: `MoveBody::MoveTo`/`Halt`
  (`crates/crpg-net/src/movement_v4.rs:78`), `Host::move_to`/`halt`
  (`crates/crpg-server/src/host.rs:4663`/`:4684`) and `BridgeCore::move_to`/`halt`
  (`crates/crpg-godot/src/core/mod.rs:278`/`:291`).
- The enemy AI runs through the host: `Host::drive_ai`
  (`crates/crpg-server/src/host/ai.rs:85`) plans TB vs RT from `combat.mode`
  (`:116–137`) and maps decisions through host operations; the embedded host
  binds area bounds (a box mesh) at `crates/crpg-godot/src/core/mod.rs:540`.
- Mode can be requested by a client with pause rights: `IntentBody::RequestMode`
  (`crates/crpg-net/src/codec_v4.rs:204`) and the server's handling
  (`crates/crpg-server/src/host.rs:6175`); the client has pause rights in
  single-player (`crates/crpg-net/src/protocol_v3.rs` pause rights, default in
  `crates/crpg-godot/src/core/mod.rs:552`).
- Proximity encounter start is not in the sim: encounter start is an explicit
  `start_encounter*` call (`crates/crpg-sim/src/combat.rs:1443`/`:1454`);
  no trigger graphs ship in either campaign
  (`tasks/T157-facts.md:128`).
- Pick/Feedback constraints from T156: `Pick.gd` matches options by target only
  (`tasks/T156.md:206–208`); `Feedback.gd` draws over each proxy's current
  position and works unchanged over `Walk.gd`'s drawn positions
  (`tasks/T156.md:212–213`); `Pick.gd` does not consume the input event
  (`tasks/T156.md:209–212`).

## Gaps

- No real-time server-AI session kind exists; `open_v4_rt` uses the peer NPC, whose legal options are empty in real time.
- No proximity trigger starts a fight; encounter start is an explicit host call and no client path requests a mode switch on approach.
- Walk and combat are two separate scenes/entry points; nothing switches modes in place.
- `Pick.gd` is tuned for turn-based one-actor options and is not exercised during real-time walking.
- No turn-based move UI in the walk scene; clicking ground always sends a real-time route.
- The enemy cannot approach in real time through the embedded host's box mesh beyond the area bounds (no authored nav mesh, only `bind_area_bounds`).
- No client logic decides when walking ends and fighting begins, or returns to walking when the fight ends.

## Packages

### T157a — `crpg-godot` core: a real-time server-AI session open

**S-set:** no.

**Goal.** Add a real-time protocol v4 session kind whose non-player combatants are played
by the server's MVP AI through `Host::drive_ai`, mirroring T156a's turn-based
`open_v4_ai` but with a real-time pacing gate, so the walk scene's enemy can
move and act on its own.

**Files owned.**
- `crates/crpg-godot/src/core/mod.rs` (additions only).
- `crates/crpg-godot/src/core/authority/v4.rs` (additions only).
- `crates/crpg-godot/src/core/authority/v4/tests/rt_ai.rs` (new; declared after
  the existing test modules).

**Exact interface.**
```rust
impl BridgeCore {
    pub fn open_v4_rt_ai(&mut self, campaign_dir: &str, encounter_id: &str,
        player_side: &str, seed: i64) -> BridgeCode;     // open_v4_rt_ai_with(.., SystemClock::new())
    pub(crate) fn open_v4_rt_ai_with(&mut self, campaign_dir: &str, encounter_id: &str,
        player_side: &str, seed: i64, clock: impl Clock) -> BridgeCode;
}
// NpcDriver gains a variant used by the real-time AI open:
enum NpcDriver { Peer, ServerAi }   // already from T156a; reused
// open_v4_mode / open_v4_guarded gain no new parameter beyond T156a's driver.
```
`ServerAiHook` (T156a) is reused; its gate changes from "NPC peer has legal
options" to "call `drive_ai` every cycle", because in real time `legal_intents`
is always empty.

**Behaviour.**
- `open_v4_rt_ai_with` is `open_v4_rt_with` with `NpcDriver::ServerAi`.
- `ServerAiHook::run` in real time: (1) `host.is_closed()` → `Err(Closed)`; (2)
  drain the NPC peer's delivery as today; (3) call `host.drive_ai(&sides,
  &actors, MVP_AI_BUDGET)` every cycle, discarding the report; (4) `Ok(())`.
  `drive_ai`'s own real-time rules already budget one decision per cycle.
- `ai_sides` and the `npc_actors` clone are taken exactly as in T156a A-1.
- Argument, campaign, side and start failures are `open_v4_rt`'s, in its order.

**Tests.** Mirror T156a's A-suite but on an RT ruleset (`campaigns/demo/walk`):
RT-1 the enemy moves toward the party within the first few cycles; RT-2 the
player's `executions()` does not rise when the AI acts; RT-3 shutdown gives 50;
RT-4 `ai_sides` reuse.

**Out of scope.** Nav-mesh authoring beyond `bind_area_bounds`; turn-based
behaviour; any GDScript.

**Open questions.** Should the real-time AI open also accept a `mode` so a fight
can start in RT and switch to TB later (default RT)? See Q1.

### T157b — `crpg-godot` shell: expose the real-time AI open

**S-set:** yes (GDExtension boundary).

**Goal.** One `#[func]` exposing `open_v4_rt_ai`, so GDScript (and tests) can
open the real-time AI session.

**Files owned.**
- `crates/crpg-godot/src/shell.rs`.
- `docs/architecture/crpg-godot.md`, `crates/crpg-godot/AGENTS.md` (named
  sections only).

**Exact interface.**
```rust
#[godot_api]
impl CrpgSession {
    #[func] fn open_v4_rt_ai(&mut self, campaign_dir: GString, encounter_id: GString,
        player_side: GString, seed: i64) -> i64;   // as open_v4_rt, calling core.open_v4_rt_ai
}
```

**Behaviour.** Converts arguments exactly as `open_v4_rt` does; sets `opened_at`
exactly when the code is 0 (invariant 15). The crate keeps exactly one
`unsafe`.

**Tests.** Shell source scan (one `core.open_v4_rt_ai` call); a GDScript
engine gate `rt_ai.gd` mirroring `enemy_ai.gd` but on `campaigns/demo/walk`.

**Out of scope.** Any core change; CI wiring (manager edit).

**Open questions.** None.

### T157c — `apps/client`: walk, then fight in one scene

**S-set:** no (app directory).

**Goal.** Make `Walk.tscn` the single game: real-time walking, then a
turn-based fight in the same place when the party meets the enemy, with
movement during the fight, and a return to walking when it ends.

**Files owned.**
- `apps/client/scripts/Walk.gd` (mode-aware polling, mode switch request,
  TB move, TB attack via `Pick.gd`/`Feedback.gd`).
- `apps/client/scenes/Walk.tscn` (add `Pick` and `Feedback` nodes after `Hud`,
  as T156c does for `Main.tscn`).
- `apps/client/tests/walk_fight.gd` (new).

**Exact interface.** No new Rust API. GDScript constants:
```gdscript
const PROXIMITY_RAW := 131072   # ~2 m in raw units; trigger distance
```
Reuse `Pick.gd` and `Feedback.gd` from T156c unchanged.

**Behaviour.**
- Open with `session.open_v4_rt_ai(_campaign_dir(), "", "", OPEN_SEED)`.
- Each frame, poll; on a new revision, if the view is still real-time and a
  player-controlled proxy is within `PROXIMITY_RAW` of an enemy proxy, call
  `session.submit_option`? No: instead build an intent `RequestMode { mode:
  TurnBased }` via a new `session.request_mode` if added, else use the first
  available option. *Open: see Q2.*
- When `view` reports a turn-based mode (a `ModeChanged` event or a non-empty
  option list for the player), switch input handling: left-click on an enemy
  uses `Pick.gd`; left-click on the ground calls `session.declare_move(...)`
  for the active actor's destination; right-click halts only in real time.
- `Feedback.gd` runs unchanged, reading the view each frame.
- When `view.ended()` becomes true, re-open or reset to a real-time session
  for continued walking. *Open: see Q3.*

**Tests.** Engine gate `walk_fight.gd`: walk to the goblin, fight starts, one
attack lands, one TB move lands, fight ends, bars/numbers appear. Control run
with `CRPG_CAMPAIGN=/no/such/dir` exits 3.

**Out of scope.** New Rust APIs unless Q2/Q3 require them; CI wiring; art.

**Open questions.** Q2, Q3.

## Questions for the user

**Q1. How does the fight start?**
(a) The client detects proximity (party proxy within ~2 m of an enemy proxy)
    and sends a `RequestMode { TurnBased }` intent through the session; the
    server validates and schedules the switch at the next round boundary.
    Recommended.
(b) The server host runs a proximity check each tick and requests the switch
    itself; the client never sends a mode intent.
What changes: (a) keeps the client as a pure presenter of a player action and
needs no new host API; (b) needs a new host-side trigger hook and a game-logic
entry point in the embedded host, which today has none, and risks the client
and host disagreeing about distance.

**Q2. How does the player move during the fight?**
(a) Left-click on the ground calls `session.declare_move(revision, x, y, z)`
    for the active actor, exactly like T156's attack uses `submit_option`;
    the server validates the turn and the movement pool. Recommended.
(b) The player picks a destination from a list of legal moves computed by the
    client.
What changes: (a) reuses the existing lane-0 TB move and needs no new view
    column; (b) needs the host to publish legal destinations and a new view
    field, and duplicates legality the server already owns.

**Q3. What happens when the fight ends?**
(a) The scene stays open and the session returns to real-time mode (the
    server's `ModeChanged` flips the view back to RT); walking resumes in
    place. Recommended.
(b) The scene reloads `Walk.tscn` from scratch, losing positions.
What changes: (a) needs the client to handle a mode switch back to RT and
    to re-enable real-time input; (b) is simpler but loses the "same place"
    feel and re-runs the open path.

**Q4. Does the real-time AI open need its own seed/clock handling?**
(a) Reuse T156a's `open_v4_rt_ai_with(clock)` shape exactly. Recommended.
(b) Add a separate `seed`-driven pacing parameter.
What changes: (a) one consistent open family; (b) a new argument on the shell
    and a second test matrix.

## Risks

- **Determinism.** The real-time AI gate must call `drive_ai` on the runner's
  injected clock, never `Instant`/`SystemTime` in the core
  (`crates/crpg-godot/AGENTS.md` invariant 8); pacing uses `u64` milliseconds.
  Replay goldens are per-target and must be regenerated only by review
  (`AGENTS.md` replay rule).
- **Authority.** The client requests a mode switch and submits moves as intents;
  it never calls `Host::request_mode`, `host_move` or `tick` directly, and the
  embedded host (run loop) stays the only mutator (`crates/crpg-godot/AGENTS.md`
  invariant 2). `Pick.gd` and `Feedback.gd` keep sending only through
  `main.choose` / reading the view (T156 C-12).
- **Replay.** A mode switch must be journaled by the host (`ModeChanged`
  delta) so a replay reproduces the same round boundary; the client's proximity
  threshold is presentation input and must not affect the journal.

## Agent log
- 2026-11-22 (UTC) · opencode-go/beam-free + T157d · Blind first draft of the T157 contract, splitting "walk, then fight" into packages.
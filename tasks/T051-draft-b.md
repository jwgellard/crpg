# T051 — cut-down demo HUD and end screen (`crpg-godot`, `apps/client`)

## Task / Crate(s) / Purpose

Sole crate: `crpg-godot` (plan row `docs/IMPLEMENTATION_PLAN.md:256`, `C → I`,
cut-down demo version with no T104/T108 prerequisite per ADR-0033
`docs/adr/0033-godot-bridge.md:522-524`). The work is GDScript in
`apps/client`, the crate's client artifact (ADR-0033 §14), plus one
Godot-free test in the crate's core. T047d's `CombatHud.gd` shows a first
version (`apps/client/scripts/CombatHud.gd:1-159`); T048 (approved contract
`tasks/T048.md`, in implementation) owns proxies and greys dead combatants,
and "T051 keeps all text in the HUD" (`tasks/T048.md:391`). Contract only.

## Exact contract

Status: **blind draft (b) for adjudication.** ADR-0033 is accepted
(`tasks/T048.md:17`); this contract changes none of its decisions. **No S-set
file is touched:** no `shell.rs`, `Cargo.toml`, `Cargo.lock`,
`crpg.gdextension`, `.github/workflows/`, `deny.toml` or ruleset change
(S-set: `docs/IMPLEMENTATION_PLAN.md:86-95`; T047d, also GDScript-only, was
ruled non-S-set at `tasks/T047.md:28`).

### Decision

- **D1. HUD content and layout (brief item 1).** Keep T047d's structure in
  `apps/client/scripts/CombatHud.gd`: a `CanvasLayer`
  (`apps/client/scripts/CombatHud.gd:1`) building a top-left `PanelContainer`
  with round, active, health, options, log, status and end labels
  (`apps/client/scripts/CombatHud.gd:21-62`). It renders the view and calls
  `choose`; it decides no rule (`apps/client/scripts/CombatHud.gd:2-4`).
  - Turn: keep two lines, `"Round %d" % (view.round() + 1)`
    (`apps/client/scripts/CombatHud.gd:66`) and `"Active: %s"`
    (`apps/client/scripts/CombatHud.gd:67`). *Options:* (a) keep both lines;
    (b) merge into one `"Round N — X's turn"` line. *Consequence:* (b) drops
    the CLI's two-line shape (`"-- Round N --"` plus `"Turn: name"`,
    `crates/crpg-cli/src/demo.rs:479`, `:488`). **Recommendation: (a).**
  - Health: keep one line per row, `"<name>: ?"` when `health_known` is 0
    else `"<name>: <health>/<max_health>"`, plus `" (dead)"` from `dead()`
    (`apps/client/scripts/CombatHud.gd:91-101`; architecture
    `docs/architecture/crpg-godot.md:188-193`). *Options:* (a) keep, no side
    column; (b) add `"<side>"` per line. *Consequence:* (b) duplicates proxy
    columns. **Recommendation: (a)** — all text stays in the HUD
    (`tasks/T048.md:391`), sides stay on proxies and in the log.
  - Option buttons: verbatim `option_labels()` text, one `Button` per label
    in host order (`apps/client/scripts/CombatHud.gd:103-110`; labels are
    `"<ability> -> <target>"` or `"End turn"`,
    `docs/adr/0033-godot-bridge.md:282`, `crates/crpg-cli/src/play.rs:186-202`).
    *Options:* (a) verbatim in host order; (b) regroup by target.
    *Consequence:* (b) reorders the host's legality order, which `crpgc play`
    prints as-is (`crates/crpg-cli/src/play.rs:153-159`).
    **Recommendation: (a).**
  - View members read: round line ← `round()`
    (`crates/crpg-godot/src/shell.rs:167`); active line ← `active()` +
    `names()` (`:161`, `:119`) via `_name_for`
    (`apps/client/scripts/CombatHud.gd:143-147`); health lines ←
    `net_ids()`, `names()`, `health()`, `max_health()`, `health_known()`,
    `dead()` (`:113`, `:119`, `:137`, `:143`, `:149`, `:155`); buttons ←
    `option_labels()` (`:179`); log ← `events()`, `receipts()` (`:191`,
    `:201`); end label ← `ended()` (`:173`). Nothing else is read.
- **D2. Combat log (brief item 2).** Source is `events()` only, appended once
  per revision through the `_logged_revision` gate
  (`apps/client/scripts/CombatHud.gd:112-119`; architecture
  `docs/architecture/crpg-godot.md:191-192`). Rendering events, not diffing
  health, keeps spec §9.2 rule 2 (fact sheet; the client renders `SimEvent`s).
  *Options:* (a) `events()` only; (b) also diff health rows.
  *Consequence:* (b) breaks the spec rule and double-reports.
  **Recommendation: (a).**
  - Order oldest-first, as the view carries them
    (`crates/crpg-godot/src/shell.rs:191`); the box keeps
    `scroll_following` (`apps/client/scripts/CombatHud.gd:52`).
  - Cap: *options* (a) `MAX_LOG_LINES := 200`, drop oldest lines first;
    (b) unbounded. *Consequence:* (b) grows a `RichTextLabel` for a whole
    session. **Recommendation: (a).**
  - Per-kind text (fields from `crates/crpg-godot/src/core/client.rs:168-202`):
    `action_resolved` → `"A | Ability -> T (outcome; N dmg)"`; `turn_started`
    → `"Turn: A (round N)"` (kept from
    `apps/client/scripts/CombatHud.gd:131-135`); `died` → `"A dies."`
    (matches the CLI's `"  {name} dies."`,
    `crates/crpg-cli/src/demo.rs:424`, replacing T047d's `"Died: %s"`,
    `apps/client/scripts/CombatHud.gd:136-137`); `encounter_ended` →
    `"Encounter ended (round N)"` (kept,
    `apps/client/scripts/CombatHud.gd:138-139`); unknown kind → the kind
    string (kept, `:140-141`). Names resolve through `_name_for`, `"-"` when
    absent (`apps/client/scripts/CombatHud.gd:143-147`).
- **D3. Choosing (brief item 3).** Button `i` calls `main.choose(i)`
  (`apps/client/scripts/CombatHud.gd:149-154`), which sends
  `session.submit_option(seen, i)` (`apps/client/scripts/Main.gd:45-48`).
  Empty option arrays mean it is not the player's turn
  (`docs/adr/0033-godot-bridge.md:287`), and on the fixture the goblin opens
  so the first polls have no player options
  (`apps/client/tests/load.gd:44-57`,
  `docs/architecture/crpg-godot.md:232-233`). *Waiting line options:*
  (a) a `_wait_label`, `"Waiting for the other side…"`, visible iff not
  ended and options are empty; (b) no line. *Consequence:* (b) leaves a
  newcomer staring at an empty button box during every NPC turn.
  **Recommendation: (a)** (a label, never a log append — a notice per sync
  would spam).
  - Non-legal option: no button exists for it, so it cannot be pressed; a
    press between syncs returns 4 (stale) or 5 (pending) and retries silently
    (`apps/client/scripts/CombatHud.gd:153`,
    `apps/client/tests/smoke.gd:59-66`). Other codes show
    `session.last_error()` (`apps/client/scripts/CombatHud.gd:74-78`;
    `last_error` text at `crates/crpg-godot/src/shell.rs:91`).
  - Refresh: buttons are rebuilt on every `Hud.sync`, which runs only when
    the revision changes (`apps/client/scripts/Main.gd:40-43`); facts are
    rewritten unconditionally, with no change tracking. A 30 (refused intent)
    is a notice and play continues
    (`apps/client/scripts/Main.gd:30-32`).
- **D4. End of combat (brief item 4).** The client learns it from `ended()`
  (`crates/crpg-godot/src/shell.rs:173`), which latches when an
  `EncounterEnded` op is applied (crate invariant
  `crates/crpg-godot/AGENTS.md:7`, line 7). The label appears on `ended()`
  (`apps/client/scripts/CombatHud.gd:71-72`). The FFI carries no
  winner/verdict accessor beyond `ended()` and the `encounter_ended` event's
  `encounter`/`round` (`crates/crpg-godot/src/core/client.rs:197-201`).
  - Text: *options* (a) `"Encounter ended — Winner: %s"`, where the winner is
    the sole surviving side among rows with `dead == 0`, else `"draw"`;
    (b) fixed `"Encounter ended"` (current
    `apps/client/scripts/CombatHud.gd:71`). *Consequence:* (b) never names
    the outcome the demo was played to learn. **Recommendation: (a)** — the
    rule mirrors the CLI summary's sole-surviving-side-else-draw
    (`crates/crpg-cli/src/demo.rs:521-536`); it is presentation over disclosed
    rows, not a new rule. The health lines already show final health with
    `" (dead)"` (`apps/client/scripts/CombatHud.gd:98-99`), mirroring the
    CLI's `" (dead)"` (`crates/crpg-cli/src/demo.rs:508`).
  - Input after the end: *options* (a) inert — no option buttons exist (the
    host returns no options with no active head,
    `docs/adr/0033-godot-bridge.md:192-194`, L2), the log is frozen at the
    last revision, the window stays open, no restart control; (b) a
    "Play again" button. *Consequence:* (b) needs a reopen path `Main.gd`
    does not have (nothing calls `open` twice; the future reopen task owns
    `$Proxies.clear()` per `tasks/T048.md:73-80`, D6).
    **Recommendation: (a).**
- **D5. FFI (brief item 5). There is none.** No member is added to the 6 +
  16 (`docs/adr/0033-godot-bridge.md:251-285`); "T048 and T051 should need no
  Rust" (`docs/adr/0033-godot-bridge.md:534`). Facts a HUD wants that the view
  does not carry — ability cost, dice faces, per-rejection prose — stay GDScript
  constants or open questions below, never new getters. Rejection prose is
  T051's job (`docs/adr/0033-godot-bridge.md:349`): receipts carry `seq`,
  `applied`, `code` (`crates/crpg-godot/src/core/client.rs:217-226`), the code
  is the frozen `RejectionCode` number
  (`docs/adr/0033-godot-bridge.md:296-297`), and the HUD maps 17
  (`NotAuthorized`, `crates/crpg-net/src/protocol.rs:428`) → `"Not allowed."`,
  18 (`IllegalAction`, `:430`) → `"That action failed."`, anything else →
  `"Rejected (%d)."`. A rejected receipt appends one line per revision (same
  gate as D2); an applied receipt appends nothing (its events speak).
- **D6. Tests (brief item 6).** One shared GDScript case file plus a headless
  runner, following the T048 pattern (fake view in the case file; a standalone
  runner; `smoke.gd` runs the same cases before loading `Main.tscn` so CI's
  existing `client` job gates them with no workflow edit,
  `tasks/T048.md:113-125`). Harness: an inner `FakeView` class returning what
  the constructor was given (T048's Interface, `tasks/T048.md:165-176`). New
  exit code **8** (smoke already uses 0/2/3/4/5,
  `apps/client/tests/smoke.gd:39-70`, and T048 takes 6,
  `tasks/T048.md:124-125`). Runner-only deferred checks are not needed here
  (no freed-node timing: `_clear` uses `remove_child` + `queue_free`,
  `apps/client/scripts/CombatHud.gd:156-159`). Plus one Godot-free Rust test
  (R2) pinning the producer side the HUD depends on.
- **D7. Split, order and boundaries (brief item 7).** One session, one crate
  (`crpg-godot`; `apps/client` is that crate's artifact, as in T047d, so no
  split). Landing order: case file and runner (red check: they fail against
  T047d's HUD), then `CombatHud.gd`, then the `smoke.gd` hook, then R2.
  Boundary with full T051: T104/T108 UX, VFX/audio hooks, `SimEvent`-driven
  effects and any HUD beyond turn/health/options/log/end stay out (full T051
  waits on T048, T104, T108 per `docs/IMPLEMENTATION_PLAN.md:256`; VFX is
  already out at `tasks/T048.md:328`). Boundary with T050: camera, picking,
  selection, active highlight and node metadata stay out
  (`tasks/T048.md:323-328`; `docs/architecture/crpg-godot.md:202`).

### Interface

`apps/client/scripts/CombatHud.gd` is rewritten. Public members are exactly
these. Private helpers are free but must be `_`-prefixed. No `class_name`,
`@export`, signal or autoload.

```gdscript
extends CanvasLayer

const MAX_LOG_LINES := 200

var main: Node = null

func sync(view) -> void
func show_error(text: String) -> void
func show_notice(text: String) -> void
func clear_log() -> void
func winner(view) -> String
```

`winner(view)` is pure: the single side among rows with `dead == 0`, else
`"draw"` (D4). `clear_log()` empties the log and resets `_logged_revision`
to `-1`; nothing calls it yet — the future reopen/reconnect task calls
`$Hud.clear_log()` between `close()` and `open()` and resets `Main.seen`,
mirroring T048's `clear()` (`tasks/T048.md:73-80`). The revision gate keeps
T047d's flaw only until then: a new session's first view is revision 1
(`docs/architecture/crpg-godot.md:94-98`), so without `clear_log()` an old
session's gate would skip the new scene's lines — same trap T048 settled at
`tasks/T048.md:31-41` (D2).

`apps/client/tests/hud_cases.gd` (new) is the case file: `extends RefCounted`,
inner `class FakeView` with `revision()`, `net_ids()`, `names()`, `sides()`,
`health()`, `max_health()`, `health_known()`, `dead()`, `active()`,
`round()`, `ended()`, `option_labels()`, `option_targets()`, `events()`,
`receipts()` returning constructed values; `func run(host: Node) -> String`
returns `""` when every case passes, else `"H<n>: <detail>"`. Each case gets a
fresh `load("res://scripts/CombatHud.gd").new()` added under `host`, with
`main` stubbed by an object exposing `choose(i)` and `session.last_error()`.

`apps/client/tests/hud.gd` (new) is the runner: `extends SceneTree`; exits
**8** unless the result is a `String` equal to `""`; exits **3** if a script
fails to load; otherwise **0**.

`apps/client/tests/smoke.gd` changes in one place: after the `ClassDB` check
and before `Main.tscn` loads, it runs the same cases under `root`; on failure
it prints `smoke.gd FAIL: hud <detail>` and exits **8**. Every other line and
exit code is unchanged (`apps/client/tests/smoke.gd:16-70`).

`crates/crpg-godot/src/core/client/tests.rs` gains one `#[test]` (R2). No
library item changes.

### Behaviour

1. **B1 layout.** `sync(null)` is a no-op (kept,
   `apps/client/scripts/CombatHud.gd:63-65`). Otherwise the panel keeps
   T047d's order: round, active, health box, options box, log
   (`scroll_following`, `:52`), status line (hidden until used, `:55-57`),
   wait label (D3), end label (hidden until `ended()`, `:59-61`).
2. **B2 health.** For each row in view order: `"<name>: ?"` if
   `health_known[i] == 0`, else `"<name>: <health>/<max>"`; append
   `" (dead)"` if `dead[i] != 0`
   (`apps/client/scripts/CombatHud.gd:91-101`). Unknown is never `0/X`.
3. **B3 options.** One `Button` per `option_labels()` entry, verbatim text, in
   host order; pressed `i` calls `main.choose(i)`
   (`apps/client/scripts/CombatHud.gd:103-110`,
   `apps/client/scripts/Main.gd:45-48`). Rebuilt on every sync.
4. **B4 waiting.** `_wait_label.text = "Waiting for the other side…"`; visible
   iff `!view.ended()` and `option_labels()` is empty. Hidden otherwise.
5. **B5 log.** Appends `_event_text` per `events()` entry, oldest first, only
   when `view.revision() != _logged_revision`, then records the revision
   (`apps/client/scripts/CombatHud.gd:112-119`). Past `MAX_LOG_LINES` lines,
   oldest lines are deleted first. `clear_log()` empties the log and resets
   the gate.
6. **B6 rejections.** Same gate as B5: per `receipts()` entry with
   `applied == false`, append `"Rejected: %s" % _rejection_text(code)` (D5
   table). Applied receipts append nothing.
7. **B7 choosing.** Codes 4/5 from `choose` retry silently; anything else
   non-zero shows `main.session.last_error()`
   (`apps/client/scripts/CombatHud.gd:149-154`). After a submit the buttons
   refresh on the next revision change
   (`apps/client/scripts/Main.gd:38-43`); a 30 shows the notice text and
   continues (`apps/client/scripts/Main.gd:30-32`).
8. **B8 end screen.** `view.ended()` shows the end label with
   `"Encounter ended — Winner: %s" % winner(view)` (D4); otherwise hidden.
   Health lines keep showing final values with `" (dead)"`. No button, log
   append, or scene input remains live except the window itself (D4).
9. **B9 no authority leak.** `CombatHud.gd` calls no member on `view` other
   than D1's list; names no session, intent, wire, sim, `Time` or `OS` —
   except reading `main.session.last_error()` for display, as today
   (`apps/client/scripts/CombatHud.gd:152-154`). It holds no gameplay state.
10. **B10 producer facts (Rust, R2).** `entries()` keys are exactly the lists
    the HUD reads (`crates/crpg-godot/src/core/client.rs:168-226`); an
    `EncounterEnded` op latches `ended`; receipts appear once, in frame order,
    in the revision that applied them.

### Tests

Cases H1–H10 live in `hud_cases.gd` and run under both the runner and
`smoke.gd`. Every assertion compares exact strings. `V` is a FakeView with
`revision 7`, `net_ids [1,2]`, `names ["Hero","Goblin"]`,
`sides ["Hero","Goblin"]`, `health [10,3]`, `max [10,6]`, `known [1,1]`,
`dead [0,0]`, `active 2`, `round 0`, `ended false`,
`labels ["Strike -> Goblin","End turn"]`, `targets [2,0]`, no events, no
receipts. "Same object" means `==` on the captured reference.

| # | Case | Valid control |
|---|---|---|
| H1 | **Round and active.** After `sync(V)`: round label `"Round 1"`; active label `"Active: Goblin"` | `active 99` (no row) renders `"Active: -"`, the `_name_for` fallback (`apps/client/scripts/CombatHud.gd:143-147`) |
| H2 | **Health lines.** Two labels: `"Hero: 10/10"`, `"Goblin: 3/6"` | `known [1,0]` gives `"Goblin: ?"`; `dead [0,1]` appends `" (dead)"` (`apps/client/scripts/CombatHud.gd:93-99`) |
| H3 | **Option buttons.** Two buttons with verbatim `"Strike -> Goblin"`, `"End turn"`; pressing 0 calls `choose(0)` | Empty labels leave zero buttons; a `choose` returning 4/5 shows no error (`apps/client/scripts/CombatHud.gd:153`) |
| H4 | **Waiting.** `sync(V)` with `labels []`, not ended: wait label visible; with labels: hidden; with `ended true`: hidden | `sync(null)` leaves every label as it was (`apps/client/scripts/CombatHud.gd:63-65`) |
| H5 | **Log lines.** Events `[action_resolved(Hero,Goblin,Strike,success,4), turn_started(Goblin,0), died(Goblin), encounter_ended(r0)]` append `"Hero \| Strike -> Goblin (success; 4 dmg)"`, `"Turn: Goblin (round 0)"`, `"Goblin dies."`, `"Encounter ended (round 0)"` in order | Unknown `kind "mystery"` appends `"mystery"`; a second `sync` of the same revision appends nothing (`apps/client/scripts/CombatHud.gd:112-119`) |
| H6 | **Log cap.** 205 event lines then `sync`: the log holds 200 lines, the first 5 gone | 200 lines stay whole |
| H7 | **Rejections.** Receipts `[(1,true,0),(2,false,18)]` append exactly one line, `"Rejected: That action failed."`; code 17 gives `"Rejected: Not allowed."`, code 99 gives `"Rejected: Rejected (99)."` | `(1,true,0)` alone appends nothing |
| H8 | **End screen.** `ended true`, `dead [1,0]`: end label visible, `"Encounter ended — Winner: Goblin"`; both alive → `"draw"`; health lines keep `"Hero: 10/10 (dead)"` | Not ended → end label hidden (`apps/client/scripts/CombatHud.gd:72`) |
| H9 | **`winner` purity.** `winner(V-dead-[1,0]) == "Goblin"`, `winner(V) == "draw"` (two live sides), `winner(V-dead-[1,1]) == "draw"`; calling twice changes nothing | Matches the CLI's sole-side-else-draw (`crates/crpg-cli/src/demo.rs:527-536`) |
| H10 | **`clear_log`.** After H5, `clear_log()`: log empty; a later `sync` of the same revision re-appends (gate reset) | Without `clear_log()`, re-sync appends nothing (H5 control) |
| R2 | `hud_reads_only_pinned_keys_and_ended_latches`, in `crates/crpg-godot/src/core/client/tests.rs` with that file's helpers: one frame carrying each of the four event kinds plus receipts `(Applied, Rejected 18)` yields exactly the D2/D5 key lists, `ended == true`, events/receipts present once in that revision; a following state-only frame bumps the revision with empty events and keeps `ended` | A second client run accepting the same frame kinds in another order shows order is frame order, not kind order |

**Red check (harness control).** Run the runner once with the new case file
against T047d's `CombatHud.gd`, before replacing it. It must exit 8
(`_wait_label`, `winner`, `clear_log`, `"dies."` text all missing). Record the
exit code in the completion record.

### Definition of done

Run from Git Bash on Windows, at the worktree root, with `CARGO_TARGET_DIR`
and `GDRUST_MAIN_EXTENSION` unset (`crates/crpg-godot/AGENTS.md:105-115`).

```bash
cargo fmt --all
Godot4 --headless --path apps/client --import
Godot4 --headless --path apps/client -s res://tests/hud.gd; echo $?          # 0
Godot4 --headless --path apps/client -s res://tests/hud.gd; echo $?          # red check first: 8
Godot4 --headless --path apps/client -s res://tests/load.gd; echo $?         # 0
CRPG_CAMPAIGN=/no/such/dir Godot4 --headless --path apps/client -s res://tests/load.gd; echo $?  # 3
Godot4 --headless --path apps/client -s res://tests/smoke.gd; echo $?        # 0
CRPG_CAMPAIGN=/no/such/dir Godot4 --headless --path apps/client -s res://tests/smoke.gd; echo $?  # 3
cargo test -p crpg-godot --locked -j 2   # runs R2
git diff --check
```

No `cargo` change is needed beyond R2's file: R2 is test-only and leaves the
library and FFI untouched (same rule as `tasks/T048.md:343-345`). A by-hand
`Godot4 --path apps/client` run is recorded too: round, active, health,
verbatim buttons, log lines, waiting line during the goblin's turn, and the
winner end screen. This draft itself is at most 35,000 bytes.

### Out of scope

- T048 proxy lifecycle; T049 positions and interpolation.
- T050 camera, picking, selection, active highlight.
- Full T051: T104/T108 UX, VFX/audio hooks, `SimEvent`-driven effects.
- Health on proxies; sides column in health lines (D1).
- Any `Main.gd` change, including resetting `seen` or calling `clear_log()`
  on a reopen — that belongs to the future reopen/reconnect task (D7).
- A restart button or rematch flow (D4).
- Snapshot/`Transfer` routing; new or edited campaign fixtures.
- Any FFI member, signal, class or autoload; any `crpg-net`/`crpg-sim`/
  `crpg-server` change; `ci.yml`, `deny.toml`, `rust-toolchain.toml`,
  `crpg-contracts`.

### What must not change

- Every ADR-0033 decision, notably the 6 + 16 FFI members and no signals
  (`docs/adr/0033-godot-bridge.md:251-285`); cut-down T051 adding no FFI
  member (`:522-524`).
- The library code of `crates/crpg-godot`: `src/**` other than the one test
  appended to `src/core/client/tests.rs`, plus `Cargo.toml` and `tests/`.
- `Cargo.lock`.
- These client files: `crpg.gdextension`, `project.godot`,
  `scenes/Main.tscn`, `scripts/Main.gd`, `scripts/SceneSync.gd`,
  `tests/load.gd`, `THIRD_PARTY_NOTICES`.
- `smoke.gd`'s existing exit codes 0/2/3/4/5, their conditions, and its final
  `count == net_ids == 2` check
  (`apps/client/tests/smoke.gd:34-58`); T048's exit 6/7 if landed.
- T047d's layout formula and mesh dimensions
  (`apps/client/scripts/SceneSync.gd:35-58`).
- No existing test is edited or weakened.
- `deps.py` `ALLOWED`, `determinism.py`'s crate list, and `engine` staying off
  by default.

### Routing and size budget

- **Package:** one session, one crate (`crpg-godot`). `apps/client` is that
  crate's artifact, as in T047d, so no split is needed.
- **S-set:** not S-set, so this is an OpenCode I package on a Windows slot
  that has `Godot4`, like T047d.
- **Size:** `CombatHud.gd` about 200–260 lines, up from 159;
  `hud_cases.gd` about 250–350; `hud.gd` about 20; `smoke.gd` +8; R2 about
  40 Rust lines.
- **Landing order:** case file and runner (red check), then `CombatHud.gd`,
  then the `smoke.gd` hook, then R2.

Files you own:

- `apps/client/scripts/CombatHud.gd`
- `apps/client/tests/hud_cases.gd` (new)
- `apps/client/tests/hud.gd` (new)
- `apps/client/tests/smoke.gd`
- `crates/crpg-godot/src/core/client/tests.rs`
- `tasks/T051-completion.md` (new)

Permissions:

- `goldens:` none. No golden or replay file moves.

**Handed on.** T050 maps a node to its `NetId` and decides picking and
highlight; the HUD's buttons stay the only chooser. Full T051 adds T104/T108
UX, VFX/audio hooks and ruleset-driven screens on top of this log and end
screen; the per-kind line texts and the winner rule are its baseline.

## Open questions for the user

1. **May the end screen name the winner computed from the rows?** The view has
   no verdict accessor (D4); the label derives the sole surviving side from
   `sides()`/`dead()`, else `"draw"`, mirroring the CLI summary
   (`crates/crpg-cli/src/demo.rs:521-536`). Options: (a) yes, as specified;
   (b) fixed `"Encounter ended"` only. **Recommendation: (a)** — a demo played
   to the end should say who won, and the computation is presentation over
   disclosed rows.
2. **Is the rejection wording acceptable?** 17 → `"Not allowed."`, 18 →
   `"That action failed."`, else `"Rejected (%d)."` Options: (a) as pinned;
   (b) your words. **Recommendation: (a)** — short, neutral, and the codes
   stay visible for reports.
3. **Is a 200-line trimmed log the right cap?** Options: (a) 200, oldest
   dropped first; (b) another number. **Recommendation: (a)** — a fixture
   encounter prints dozens of lines; 200 keeps the whole game with headroom.
4. **Should the waiting line exist?** `"Waiting for the other side…"` while
   options are empty and the encounter runs (D3). Options: (a) yes;
   (b) empty button box. **Recommendation: (a)** — the goblin opens every
   default game, so (b) greets every player with silence.
5. **Is a fully inert end screen acceptable for the demo?** No restart
   control; closing and reopening replays (D4). Options: (a) yes, restart
   belongs to the future reopen task; (b) add a replay button now.
   **Recommendation: (a)** — a button without a reopen path is decoration.

No other question needs you. No ADR change is requested.

## Agent log

- 2026-11-03 (UTC) · opencode-go/muse-spark-1.3-contributor + T051 draft · Wrote blind draft (b) of the cut-down T051 HUD/end-screen contract from the code (view members, T047d HUD, CLI wording, T048 test pattern), pinning per-kind log lines, the winner-from-rows end screen, and the fake-view case plus runner pattern with no workflow change.

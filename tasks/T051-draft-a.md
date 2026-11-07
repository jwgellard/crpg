# T051 — cut-down demo HUD and end screen (`crpg-godot`, `apps/client`)

## Task / Crate(s) / Purpose

Sole crate: `crpg-godot` (`docs/IMPLEMENTATION_PLAN.md:256`, `C → I`, prerequisite
T048; the cut-down version has no T104/T108 prerequisite). The work is GDScript in
`apps/client`, the crate's client artifact (ADR-0033 §14,
`docs/adr/0033-godot-bridge.md:522-524`), plus docs. No Rust, no wire, no sim, no
server, no core, no S-set file.

T047d's `apps/client/scripts/CombatHud.gd` already shows a first version: round,
active name, health lines, one button per option, an event log and an end label
(`CombatHud.gd:63-72`). T051 makes it a demo a person can play to the end and
understand: whose turn it is, everyone's health including unknown and zero, the
choices, what just happened, and who won. T048 is being implemented in parallel and
owns proxies, despawn and the grey dead visual (`tasks/T048.md:94-107`); "T051 keeps
all text in the HUD" (`tasks/T048.md:391`).

## Exact contract

Status: **draft (a) for adjudication.** ADR-0033 decisions are not changed. Checked
against the code; `tasks/T051-facts.md` was not trusted. T048 is
assumed landed; otherwise the two `smoke.gd` hooks must be sequenced (D12).

### Decision

- **D1. Scope is a rewrite of `CombatHud.gd` plus its tests and docs.** No change to
  `Main.gd`, `Main.tscn` or `SceneSync.gd`. `Main.gd` already calls `$Hud.sync(next)`
  exactly when the revision changes (`apps/client/scripts/Main.gd:40-43`) and
  `choose(index)` maps to `session.submit_option(seen, index)` (`Main.gd:45-48`), so
  the HUD has every hook it needs.
  - Options: (a) HUD-only; (b) move formatting into `Main.gd`; (c) a shared
    formatter class. **Recommendation: (a).** (b) mixes session ownership with
    presentation and makes the smoke test unable to reach the text; (c) adds a file
    and a class name for one consumer.

- **D2. One panel, one fixed column, named nodes.** Keep T047d's tree:
  `PanelContainer` (min width 420, at `12, 12`) → `MarginContainer` → `VBoxContainer`
  in this order: `Round` (Label), `Turn` (Label), `Health` (VBox), `Options` (VBox),
  `Waiting` (Label), `Log` (RichTextLabel, min height 120, `scroll_following = true`),
  `Status` (Label, hidden, red on error), `End` (Label, hidden, three lines).
  `apps/client/scenes/Main.tscn:25-26` gives the HUD a `CanvasLayer`, so no scene
  change is needed.
  - Options: (a) one top-left panel; (b) three panels (turn/health, log,
    end); (c) a separate end-screen scene. **Recommendation: (a)**, with the end
    block inside the panel. (b) triples the tree and the `sync` writes for a 120 px
    log; (c) needs a second scene and sync path, and layout is T104's.

- **D3. Turn and round.** `Round <round() + 1>` and `Turn: <name>` when `active()` is
  non-zero, else `Turn: none`. The `+1` is ADR-0033's rule
  (`docs/adr/0033-godot-bridge.md:280`); `crpgc play` prints the same displayed
  round (`crates/crpg-cli/src/demo.rs:474-480`, `:443`).
  - Options: (a) keep T047d's `Round N` + `Active: <name>`; (b) `Round N` +
    `Turn: <name>`, the CLI wording; (c) one combined line. **Recommendation: (b)**.
    (a) says "active", which a player must map to "whose turn"; (c) makes the
    round hard to scan while the log also prints turns.

- **D4. Health lines, absent vs zero.** One `Label` per row in `net_ids()` order,
  exactly:
  - known and alive: `<name>: <health>/<max_health>`;
  - known and dead: `<name>: <health>/<max_health> (dead)`;
  - unknown: `<name>: ?`.

  The `(dead)` suffix reads only `dead[i]`; a row whose `health_known` is 0 never
  gets it, because the producer writes `dead = 0` for an absent fact
  (`crates/crpg-godot/src/core/client.rs:414-427`). Health is a whole number: the
  sim divides the authored `Fx16_16` stat by 65536 at start
  (`crates/crpg-sim/src/combat.rs:1300-1319`), so `10/10`, not `655360/655360`.
  - Options: (a) current `<name>: hp/max`; (b) the CLI header's
    `<name> | side <side> | hp/max hp` (`crates/crpg-cli/src/demo.rs:336-343`);
    (c) one bar or one row per side. **Recommendation: (a).** (b) repeats the side on
    every row and, on the fixture, prints "Hero | side Hero"; (c) is T104's layout
    work. Undisclosed health is `?`, never `0/0` (ADR-0033:277).

- **D5. Option buttons.** One `Button` per entry of `option_labels()`, in returned
  (host) order, with the label text **verbatim**. No prefix, no re-derivation.
  `option_targets()` is not read; mapping a clicked proxy to an option is T050's.
  With no labels there is no button, so no submission is possible: that is
  ADR-0033's "not the player's turn" (`:287`), and B5's Waiting line names the active
  combatant.
  - Options: (a) verbatim label; (b) `1. ` prefix as the CLI list (`play.rs:153-159`);
    (c) build labels from `option_targets()` + `names()`. **Recommendation: (a).**
    (b) adds an ordinal the button already answers by position and diverges from the
    label ADR-0033 pins (`:282`); (c) is a second label source and loses End turn's
    special case (`client.rs:540-542`).

- **D6. Choosing, and every code a click can return.** Button `i` calls
  `main.choose(i)` once, after disabling the whole `Options` box. Outcomes:
  - `0`: stay disabled; the receipt bumps the revision (`client.rs:373-387`,
    `:456-459`), so the next `sync` rebuilds the buttons.
  - `4` (stale): if `main.view != null`, `sync(main.view)`; otherwise no-op. No line.
  - `5` (pending): no line; re-enable.
  - `30` (refused): `show_notice(session.last_error())`; re-enable.
  - anything else: `show_error(session.last_error())`, or `show_error("the session is
    not open")` when `main.session == null` (Main returns `1` in that case,
    `Main.gd:45-47`); re-enable.

  - Options for `30`: (a) a notice; (b) silent; (c) an error. **Recommendation:
    (a).** A full link returns 30 with no frame queued (`client.rs:527-528`), so the
    next `poll` can be `0` and silence would hide the click; another choice is
    allowed (ADR-0033:338).
  - Options for `5`: (a) silent; (b) a notice. **Recommendation: (a):** with the box
    disabled a second click is a double-click race, and the first intent's receipt
    will arrive.

- **D7. When the buttons refresh: only inside `sync`, plus D6's stale resync.**
  `Main.gd` calls `sync` exactly on a revision change (`:40-43`), which includes any
  receipt or state change. The HUD has no timer and no per-frame rebuild.
  - Options: (a) revision-driven; (b) every frame; (c) every N ms. **Recommendation:
    (a).** (b)/(c) rebuild a button the player may be pressing and duplicate
    `Main.gd`'s gate. A `30` with no revision change is handled by re-enabling the
    existing nodes in D6, not by rebuilding.

- **D8. The log renders events, then rejected receipts.** Source: `events()`
  (ADR-0033:289-294), then `receipts()` (ADR-0033:296-298), each oldest first. Never
  diff health or names between views: spec §9.2 rule 2 says the client renders
  events and does not infer them (`docs/CRPG_ENGINE_SPEC.md:880`).
  - Options: (a) events only; (b) events + rejected receipts; (c) state diffs.
    **Recommendation: (b).** Rendering `RejectionCode` numbers as player text is
    explicitly T051's job (ADR-0033:349), and a rejected choice produces no event,
    so (a) would swallow it. (c) is banned by the spec and by ADR-0033:297-298.
  - `applied` receipts print nothing: the resolution event already describes the
    action. The two lists carry no shared order, so events print before receipts.

- **D9. Log order, text, and length.** One buffer `_lines: PackedStringArray`; every
  line is appended at the end; when it exceeds `LOG_MAX_LINES = 200` the oldest lines
  are dropped and `Log.text` is rebuilt as `"\n".join(_lines)`. The log is emptied
  only by `clear()`. The start is acknowledged before the wrap, so the opening turn
  arrives as state, not as a `turn_started` event (ADR-0033:92-93); no opening turn
  line is logged, and the header block carries the opening state.
  - Options for the cap: (a) 200 lines; (b) unlimited; (c) 1000. **Recommendation:
    (a).** The panel is 120 px; unbounded growth leaks in a long run.
  - `get_parsed_text()` trimming is lossy and re-parses every frame; the buffer is
    the label's single source.

- **D10. End of combat: `ended()` and a neutral three-line end block.** `ended()` is
  latched by the first `EncounterEnded` (`crates/crpg-godot/src/core/client.rs:361-367`;
  `crates/crpg-godot/src/shell.rs:173-176`). The end block is derived from state, not
  from the event, because `events()` only covers the revision that produced them
  (ADR-0033:284). `End.text` is exactly:
  `Encounter over\nRounds played: <round() + 1>\nWinner: <verdict>`.
  The verdict is `draw` unless exactly one side has a row with `dead() == 0`, in
  which case it is that side's `sides()` value — the CLI's summary rule
  (`crates/crpg-cli/src/demo.rs:502-536`). An unknown `dead` reads 0, i.e. alive
  (ADR-0033:278); the slice discloses everything.
  - Options for the verdict: (a) neutral third person; (b) "You win"/"You lose";
    (c) just "Encounter ended". **Recommendation: (a).** (b) needs the player's side,
    which `view()` does not expose and ADR-0033 forbids adding (Q1); (c) answers no
    question; the health lines already show the dead.
  - When `ended()` is true the `Options` box is empty and `Waiting` hidden even if a
    view (a fake, or a future bug) still offers labels.

- **D11. No FFI, no Rust, no wire, no golden.** ADR-0033 says the cut-down adds no
  FFI member and should need no Rust (`:522-524`, `:534`). No Rust test is added:
  the producer side the HUD consumes is already pinned — labels
  (`crates/crpg-godot/src/core/client/tests.rs:446-490`,
  `crates/crpg-godot/src/core/tests.rs:1049-1050`), event keys
  (`client/tests.rs:269-300`), the `Died`-is-not-death rule
  (`client/tests.rs:641-656`) and the latch/revision bump (`client.rs:361-367`,
  `:456-478`). Facts the HUD lacks are open questions, not members (Q1–Q3). Not
  S-set.

- **D12. Tests: a fake view in a case file, a standalone runner, and a `smoke.gd`
  hook.** Exact shape under Interface.
  - Options: (a) the T048 pattern; (b) only the real session in `smoke.gd`; (c) new
    CI steps. **Recommendation: (a).** The fixture cannot produce unknown health, a
    draw, a rejected receipt or a 250-line log, so (b) proves almost nothing; (c)
    edits an S-set file (`.github/workflows/ci.yml` `client` job, `:313-321`), which
    ADR-0033 §15 gives to T047c. (a) is gated by the existing BR-22 step with no
    workflow change.
  - Exit codes: `hud.gd` 0 pass, 3 script load failure, 6 first case failure.
    `smoke.gd` gains 7 (T048 takes 6); its 0/2/3/4/5 conditions and the final
    `count == net_ids == 2` check are unchanged (`apps/client/tests/smoke.gd:2-8`,
    `:47-58`).

### Interface

`apps/client/scripts/CombatHud.gd` keeps `extends CanvasLayer` and the public
members `Main.gd` already calls. `main` becomes untyped so the case file's
`RefCounted` fake can stand in; it still calls only `choose`, `session.last_error`
and `view`.

```gdscript
extends CanvasLayer

const LOG_MAX_LINES := 200

var main = null                       # the scene root; only choose/view/session are used

func sync(view) -> void               # the whole HUD; null is a no-op
func show_error(text: String) -> void # red status + "ERROR: <text>" line
func show_notice(text: String) -> void# one log line
func clear() -> void                  # session boundary: empty every node and the log
```

Built nodes and their pinned names (`find_child(name, true, false)`; the recursive
search must pass `owned = false`, because nodes created in code have no owner):

| Name | Class | Content |
|---|---|---|
| `Round` | `Label` | D3 |
| `Turn` | `Label` | D3 |
| `Health` | `VBoxContainer` | one `Label` per row, D4 |
| `Options` | `VBoxContainer` | one `Button` per option, D5 |
| `Waiting` | `Label` | D5/B5 |
| `Log` | `RichTextLabel` | D9 |
| `Status` | `Label` | `show_error` text, hidden until used |
| `End` | `Label` | D10 |

Every child of `Health`/`Options` is unnamed; the tests read them positionally.

Each element's only view inputs, with the getter's definition (`shell.rs`) and
T047d's current read (`CombatHud.gd`):

| Element | `view()` members (getter at) | T047d read |
|---|---|---|
| `Round` | `round()` `crates/crpg-godot/src/shell.rs:167-170` | `:66` |
| `Turn` | `active()` `:161-164`, `net_ids()` `:113-116`, `names()` `:119-122` | `:67`, `:143-147` |
| `Health` | `net_ids` `:113`, `names` `:119`, `health` `:137-140`, `max_health` `:143-146`, `health_known` `:149-152`, `dead` `:155-158` | `:83-101` |
| `Options` | `option_labels()` `:179-182`, `ended()` `:173-176` | `:103-110` |
| `Waiting` | `option_labels` `:179`, `active` `:161`, `ended` `:173`, `net_ids`/`names` | new |
| `Log` | `revision()` `:107-110`, `events()` `:191-198`, `receipts()` `:201-208`, `net_ids`/`names` | `:112-141` (events only) |
| `End` | `ended()` `:173`, `round()` `:167`, `net_ids`/`names`/`sides()` `:125-128`/`dead` `:155` | `:71-72` (a fixed label) |
| never | `placements()` `:131-134`, `option_targets()` `:185-188` | not read |

`apps/client/tests/hud_cases.gd` (new) is the case file:

- `extends RefCounted`.
- Inner `class FakeView` whose constructor takes one `Dictionary` and whose getters
  are exactly the 14 members the HUD may read: `revision`, `net_ids`, `names`,
  `sides`, `health`, `max_health`, `health_known`, `dead`, `active`, `round`,
  `ended`, `option_labels`, `events`, `receipts`. It returns what it was given.
- Inner `class FakeSession` with `var text := ""` and
  `last_error() -> String` returning `text`.
- Inner `class FakeHost` (`RefCounted`): `var view = null`, `var session =
  FakeSession.new()`, `var chosen: Array = []`, `var codes: Array = []`;
  `func choose(index: int) -> int` appends `index` to `chosen` and returns
  `codes.pop_front()` (default `0` when empty).
- `func run(root: Node) -> String`: `""` on success, else `"H<n>: <detail>"` for the
  first failure. Each case creates a fresh `Node` host under `root`, a fresh
  `load("res://scripts/CombatHud.gd").new()` added under it, then assigns
  `hud.main = fake`. Nodes are freed with `remove_child` then `queue_free`.

`apps/client/tests/hud.gd` (new) is the runner: `extends SceneTree`; in
`_initialize` it loads the case file and calls `run(root)`; exits 6 unless the result
is a `String` equal to `""`, 3 if the script or the result is not a `String`, else
prints `hud.gd OK` and exits 0.

`apps/client/tests/smoke.gd` gains one block after T048's scene-sync block and before
`Main.tscn` loads: run the HUD cases; on failure print
`smoke.gd FAIL: hud <detail>` and exit 7. The header comment lists 7.

**Unverified (Godot 4.7.2, not runnable here):**

- `Node.find_child(pattern, recursive, owned)` with `owned = false` finds nodes
  created in GDScript, which have no `owner`.
- `PackedStringArray.slice()` exists and returns a copy, and `"\n".join(...)`
  accepts a `PackedStringArray`.
- A `Button`'s `pressed` signal can be fired from a test with
  `button.pressed.emit()`.
- Assigning `RichTextLabel.text` replaces the content and leaves
  `scroll_following` at the end.

T051 relies on no `godot` crate FFI detail: it calls the existing getters
(`crates/crpg-godot/src/shell.rs:107-208`) and adds no Rust.

### Behaviour

1. **B1 build.** After `_ready` the eight nodes above exist under the HUD, in D2's
   order; `Status` and `End` are hidden; every other text is empty; `Health` and
   `Options` have no children; `Log.text == ""`. The panel keeps the 420 px minimum
   width and the log its 120 px minimum height.
2. **B2 turn and round.** D3: `Round` is `round() + 1`; `Turn` is the `names()` entry
   for `active()`, or `none` when `active()` is 0. The name is looked up through
   `net_ids()`, never by row math.
3. **B3 health.** D4, one line per `net_ids()` row in order, exactly the three forms;
   absent is `?`, zero is `0/<max>` and only `dead()` adds ` (dead)`. A `health_known`
   of 0 or 1 decides the form; nothing else does.
4. **B4 options.** D5: one enabled `Button` per `option_labels()` entry, exact text,
   in order; pressing button `i` calls `main.choose(i)` exactly once. With no labels
   the box is empty.
5. **B5 waiting line.** When `option_labels()` is empty and `ended()` is false,
   `Waiting` is visible with `Waiting for <active name>...`, or `Waiting...` when
   `active()` is 0. With any label, or when ended, it is hidden.
6. **B6 refresh.** Buttons and texts change only when `sync` runs; two syncs of the
   same view rewrite the text identically. A sync that changes the labels frees the
   old buttons (`remove_child`, then `queue_free`) and creates new ones.
7. **B7 success.** D6: on `0` every button is disabled, no status and no log line
   appear, and the next `sync` rebuilds enabled buttons.
8. **B8 stale.** D6: on `4`, `sync(main.view)` runs (when non-null), so the box shows
   the labels of `main.view` and is enabled; no status and no line. With a null
   `main.view` nothing changes.
9. **B9 pending, refused, errors.** D6: `5` leaves no line and re-enables; `30` adds
   exactly `session.last_error()` through `show_notice` and re-enables, with `Status`
   untouched; any other non-zero shows `session.last_error()` in `Status` and
   `ERROR: <text>` in the log and re-enables; with a null session the text is
   `the session is not open` and nothing crashes.
10. **B10 log gate, order, cap.** D8/D9: lines are appended oldest first; `events()`
    then `receipts()`; both are gated by one `revision()` value stored in
    `_logged_revision`, so syncing the same revision twice adds nothing, while a new
    revision with identical payloads adds again. The buffer keeps the last 200 lines
    and drops the oldest; `Log.text` is the buffer joined with `\n`.
11. **B11 event text.** One exact line per kind:
    - `action_resolved`: `<actor> | <ability> -> <target>: <verdict>, <damage> damage`,
      where verdict maps `critical_success|success|failure|critical_failure` to
      `critical hit|hit|miss|critical miss` and any other string to itself;
    - `turn_started`: `Turn: <actor> (round <round + 1>)`;
    - `died`: `<entity> dies.`;
    - `encounter_ended`: `Encounter ended (round <round + 1>)`;
    - any other kind: the kind string, unchanged.

    Names resolve through `net_ids()`/`names()`; an id not in the rows prints `-`.
12. **B12 receipts.** D8: `applied == true` prints nothing; `applied == false` prints
    `Rejected: <text>` with the code table below; an unlisted non-zero code prints
    `Rejected: rejected (code <n>).`

    | Code (`crates/crpg-net/src/protocol.rs:393-435`) | Text |
    |---|---|
    | 2 | `the server was busy; try again.` |
    | 7, 8, 19 | `the server could not accept that; try again.` |
    | 10–14 | `the session lost its place; restart the demo.` |
    | 15, 16 | `the choice arrived out of time; try again.` |
    | 17 | `that action is not available to you.` |
    | 18 | `that action was no longer legal.` |
13. **B13 end.** D10: when `ended()` is true, `End` is visible with exactly the three
    lines and the verdict; `Options` is empty and `Waiting` hidden. A later view that
    still says ended keeps the same block even with no events. When `ended()` is
    false, `End` is hidden.
14. **B14 no authority leak, no input.** `CombatHud.gd` calls only the 14 view
    members of Interface; it names no `placements`, `option_targets`, `submit_option`,
    `ClassDB`, `instantiate`, `OS` or `Time`; it defines no `_input`,
    `_unhandled_input` or `_gui_input`; it holds no gameplay state — only the log
    buffer, the logged revision and the last error text (spec
    `docs/CRPG_ENGINE_SPEC.md:310`).
15. **B15 preconditions and `clear`.** `sync(null)` changes nothing. A view with no
    rows empties `Health` and `Options` and shows `Waiting...` when not ended.
    `clear()` empties every node, hides `Status` and `End`, clears the log buffer and
    resets `_logged_revision` to `-1`; a later `sync` rebuilds from that view. Unequal
    parallel columns are a caller bug and are not defended (ADR-0033:271).

### Tests

Cases H1–H20 live in `hud_cases.gd`; all run under `hud.gd` and, via the hook, under
`smoke.gd`. `A` is a `FakeView`:
`revision 1`, ids `[1, 2]`, names `["Hero", "Goblin"]`, sides `["Hero", "Goblin"]`,
health `[10, 6]`, max `[10, 6]`, known `[1, 1]`, dead `[0, 0]`, active `1`, round
`0`, ended `false`, labels `["Strike -> Goblin", "End turn"]`, no events, no
receipts. "Line n" means the n-th `\n`-separated entry of `Log.text`, 0-based.

| # | Case | Valid control |
|---|---|---|
| H1 | **Build (B1).** All eight names resolve with `find_child(name, true, false)`; their parent `VBoxContainer` lists them in D2's order; `Status.visible == false`, `End.visible == false`; `Round.text == ""`; `Health`/`Options` child count 0; `Log.text == ""`; the panel is at `Vector2(12, 12)` with `custom_minimum_size.x == 420.0`; `Log.custom_minimum_size.y == 120.0` and `scroll_following` | After `sync(A)`: `Round.text == "Round 1"`, `Turn.text == "Turn: Hero"`, `Health.get_child_count() == 2`; so presence is not emptiness |
| H2 | **Turn (B2).** `sync(A)` → `"Round 1"`, `"Turn: Hero"`. Then a view with `round 2, active 2` → `"Round 3"`, `"Turn: Goblin"`. Then `active 0` → `"Turn: none"` | round 0 and 2 give different texts, so the `+1` is exercised |
| H3 | **Health (B3).** `sync(A)` → `["Hero: 10/10", "Goblin: 6/6"]`. Known `[1,0]`, health `[10,0]`, max `[10,0]` → second line `"Goblin: ?"` (not `0/0`). dead `[0,1]`, health `[10,0]`, max `[10,6]` → `"Goblin: 0/6 (dead)"` | the first line is `"Hero: 10/10"` in all three states, so the change is the second row's form |
| H4 | **Options (B4).** `sync(A)` → two `Button`s, texts exactly the two labels, both enabled; `button[1].pressed.emit()` → `fake.chosen == [1]`, code 0 → both disabled | a view with `labels []` gives 0 buttons (H5), so the count is content-driven |
| H5 | **Waiting (B5).** labels `[]`, active `2`, not ended → `Waiting.visible`, text `"Waiting for Goblin..."`, 0 buttons. active `0` → `"Waiting..."`. `ended true` → hidden | after `sync(A)` `Waiting` is hidden, so visibility is not always true |
| H6 | **Refresh (B6).** `sync(A)`, keep `b0 = Options.get_child(0)`; `sync(B)` with revision 2 and labels `["End turn"]` → 1 button `"End turn"`, `b0.get_parent() == null` and `b0.is_queued_for_deletion()`, `b0 != Options.get_child(0)` | `b0` was enabled and child of `Options` before the second sync |
| H7 | **Success (B7).** Fresh HUD, `sync(A)`: both buttons enabled, `Status` hidden. Press 0 with `codes = [0]` → every button `disabled`, `Status.visible == false`, `Log.text == ""`. Then `sync(B)` (revision 2) → one enabled button | before the press every button is enabled |
| H8 | **Stale (B8).** `fake.view = B` (labels `["End turn"]`), `fake.codes = [4]`; `sync(A)`, press 0 → `Options` has one button `"End turn"`, enabled, no `Status`, `Log.text == ""`. Then `fake.view = null`, `fake.codes = [4]` again, press 0 → `Options` still holds that same button, enabled, no `Status`, log still empty | without the resync the box would still hold A's two buttons |
| H9 | **Pending and refused (B9).** `codes = [5]` → no line, no `Status`, buttons enabled. `codes = [30]`, `fake.session.text = "INTENT_REFUSED: link full"` → `Log.text == "INTENT_REFUSED: link full"`, `Status.visible == false`, buttons enabled | H10's code 6 produces a `Status` and an `ERROR:` line, so "nothing appeared" is not universal |
| H10 | **Errors (B9).** `codes = [6]`, `text = "NO_SUCH_OPTION: index is out of range"` → `Status.text` equals that, `Log.text` equals `"ERROR: NO_SUCH_OPTION: index is out of range"`, buttons enabled. `fake.session = null`, `codes = [1]` → `Status.text == "the session is not open"`, log `"ERROR: the session is not open"`, no crash | H9's code 5 leaves `Status` hidden |
| H11 | **Event lines (B11).** One revision with the four events → four lines in order: `"Hero | Strike -> Goblin: hit, 2 damage"`, `"Turn: Goblin (round 1)"`, `"Goblin dies."`, `"Encounter ended (round 1)"`. Then revision 2 with one `action_resolved` (`Goblin`, Hero, `success`, 0) → exactly one more line, after them | the new line differs from all four, so the append is visible |
| H12 | **Verdicts (B11).** Four `action_resolved` with `critical_success`, `success`, `failure`, `critical_failure` → `critical hit`, `hit`, `miss`, `critical miss` inside the four lines; a fifth with `weird` prints `weird` | the four mapped texts are distinct, and the fifth differs from all |
| H13 | **Log gate (B10).** Revision 5 with one `action_resolved` and one rejected receipt (code 18) → exactly 2 lines, event first. `sync` of the same object again → still 2. Revision 6 with the same payloads → 4 | the revision differs while the payload is identical, so the gate is the revision and not the content |
| H14 | **Receipt text (B12).** One revision with receipts in order: applied/0 → nothing; 18 → `"Rejected: that action was no longer legal."`; 17 → `"Rejected: that action is not available to you."`; 2 → `"Rejected: the server was busy; try again."`; 9 → `"Rejected: rejected (code 9)."`; 7 → the shared server text | the applied entry adds no line while its neighbours do, in one revision |
| H15 | **Cap (B10).** 250 `show_notice("L<n>")` → 200 lines, first `"L50"`, last `"L249"`. A fresh HUD with 199 → 199 lines, first `"L0"` | the two first-line values differ, so trimming happened and kept the tail |
| H16 | **Sole survivor (B13).** `ended true`, round 4, dead `[0,1]`, labels `["Strike -> Goblin"]` → `End.visible`, text `"Encounter over\nRounds played: 5\nWinner: Hero"`; `Options` 0 children; `Waiting` hidden | the same view with `ended false` hides `End` and shows 1 button, so ended is what changes it |
| H17 | **Draw (B13).** `ended true`, dead `[1,1]`, round 0 → last line `"Winner: draw"` | H16's last line is `"Winner: Hero"` |
| H18 | **End persists (B13).** Revision 7 ended with the `encounter_ended` event → `End` visible, log 1 line. Revision 8 ended, no events → `End` text identical, log still 1 line | revision 8 with `ended false` hides `End` |
| H19 | **Source rule (B14).** Read `res://scripts/CombatHud.gd` with `FileAccess` and strip from the first `#` on each line. Every `view.<name>(` of `r"view\.([a-z_]+)\("` is one of the 14; both `events` and `receipts` occur; no identifier token equals `placements, option_targets, submit_option, ClassDB, instantiate, OS, Time`; no `_input`, `_unhandled_input` or `_gui_input` occurs | the same scanner on `Main.gd` finds `session` and `submit_option`, so the token list is not vacuous |
| H20 | **Null, empty, clear (B15).** `sync(null)` after A leaves every text identical. A 0-row view → `Health`/`Options` empty, `Waiting` `"Waiting..."`. A fresh HUD synced with H16's ended view, then `clear()` → `Log.text == ""`, `End`/`Status` hidden, `Round`/`Turn` `""`, `Health`/`Options` empty, and a later `sync(A)` restores `"Round 1"`/`"Turn: Hero"` | before `clear()` those fields are non-empty and `End` visible |

**Red check (harness control).** Run `hud.gd` against T047d's `CombatHud.gd` before
replacing it: it must exit 6 because `find_child("Round", true, false)` is null
(T047d's labels are unnamed, `CombatHud.gd:39-61`). Record the exit code in the
completion record.

### Definition of done

Run from Git Bash on Windows at the worktree root, with `CARGO_TARGET_DIR` and
`GDRUST_MAIN_EXTENSION` unset (`crates/crpg-godot/AGENTS.md:113-115`).

```bash
cargo fmt --all
cargo clippy -p crpg-godot --all-targets --locked -j 2 -- -D warnings
cargo test -p crpg-godot --locked -j 2
python tools/lint/deps.py
python tools/lint/determinism.py
python -m unittest discover -s tools/lint -p "test_*.py"
cargo clippy -p crpg-godot --features engine --all-targets --locked -j 2 -- -D warnings
cargo build -p crpg-godot --features engine --locked -j 2
# fresh checkout only (Godot 4.7.2 first-scan crash, T047c-fix1):
mkdir -p apps/client/.godot && [ -f apps/client/.godot/extension_list.cfg ] \
  || printf 'res://crpg.gdextension\n' > apps/client/.godot/extension_list.cfg
Godot4 --headless --path apps/client --import
Godot4 --headless --path apps/client -s res://tests/hud.gd; echo $?          # 0
Godot4 --headless --path apps/client -s res://tests/load.gd; echo $?         # 0
CRPG_CAMPAIGN=/no/such/dir Godot4 --headless --path apps/client -s res://tests/load.gd; echo $?   # 3
Godot4 --headless --path apps/client -s res://tests/smoke.gd; echo $?        # 0
CRPG_CAMPAIGN=/no/such/dir Godot4 --headless --path apps/client -s res://tests/smoke.gd; echo $?  # 3
git ls-files --others --exclude-standard -- 'apps/client/*.uid' | xargs -r rm --
git diff --check
```

A by-hand `Godot4 --path apps/client` run is recorded too: play to the end; the last
action's log line, the `(dead)` health line, the grey proxy (T048) and
`Winner: <side>` agree, and the options are gone.

Docs, signed in the same commit:

- `docs/architecture/crpg-godot.md` Client section: replace the `CombatHud.gd`
  bullet (`:188-193`) with D1–D10; add the two test files and exit 7 to the
  `smoke.gd` bullet (`:194-201`); replace "T048 owns respawn/despawn and T049
  positions" (`:202-203`) to name T051's landed HUD.
- `crates/crpg-godot/AGENTS.md`: add the `hud.gd` engine gate; add four traps:
  `find_child` must pass `owned = false` for code-built nodes; log trimming uses the
  `_lines` buffer, never `get_parsed_text()`; remove a button before `queue_free`
  (T048's idiom); `events()`/`receipts()` are per-revision, so the log gate is the
  revision.

### Out of scope

- T050 camera, picking, selection, active highlight, keyboard input, node→`NetId`
  metadata, and using `option_targets()`.
- Full T051 and T104/T108: layout and theme research, a player-relative verdict,
  ruleset-driven widgets, VFX/audio hooks, per-action dice/margin text, an
  encounter-name header, pause/restart, save.
- `Main.gd`, `Main.tscn`, `SceneSync.gd`, `load.gd`, T048's cases and their exit 6.
- Any `shell.rs`, `Cargo.toml`, `Cargo.lock`, `crpg.gdextension`, `project.godot`,
  `deny.toml`, `ci.yml` or ruleset change; any wire, protocol, golden, fixture, sim,
  core, net or server change; any new dependency.
- A HUD session boundary call: `clear()` exists, but nothing calls it until a reopen
  task (T048 D6's rule, `tasks/T048.md:73-80`).

### What must not change

- Every ADR-0033 decision, notably the 6 + 16 members, no signals (`:256-285`), no
  FFI for the cut-down (`:522-524`) and no T104/T108 prerequisite.
- The library code of `crates/crpg-godot` (`src/**`, `Cargo.toml`, `tests/`).
- `Cargo.lock`, the campaign fixtures and `apps/client/THIRD_PARTY_NOTICES`.
- `Main.gd`'s session ownership and `choose(seen, i)`; `Main.tscn`'s node tree;
  `SceneSync.gd`'s proxies, layout and D8 dead visual.
- `smoke.gd`'s existing exit codes 0/2/3/4/5, their conditions, and its final
  `count == net_ids == 2` check; T048's exit 6.
- No existing test is edited or weakened.

### Routing and size budget

- **Package:** one session, one crate (`crpg-godot`), GDScript-only; `apps/client` is
  that crate's artifact as in T047d/T048. No split is needed. Not S-set, so an
  OpenCode I package on a Windows slot with `Godot4`.
- **Landing order:** **after T048 lands** (it rewrites `SceneSync.gd`, adds the
  scene-sync cases and takes `smoke.gd` exit 6), then this contract; then (1)
  `hud_cases.gd` + `hud.gd` (red check), (2) `CombatHud.gd`, (3) the `smoke.gd`
  hook, (4) docs. If T051 must land first, swap the smoke exit codes (T051's cases
  6, T048's 7) and say so in the completion record.
- **Size:** `CombatHud.gd` about 190–240 lines, up from 159; `hud_cases.gd` about
  380–480; `hud.gd` about 30; `smoke.gd` +8; docs +20–30. No Rust line.
- **Boundary with T050:** T050 owns camera, picking, selection and a node→`NetId`
  link; it may add `option_targets()`/`placements()` use. T051 owns all text, and its
  only input is the option buttons. **Boundary with full T051/T104/T108:** everything
  in Out of scope. **Boundary with T048:** T048 owns proxies and the dead visual; T051
  owns text and never reads `placements()`.

### Files you own

- `apps/client/scripts/CombatHud.gd`
- `apps/client/tests/hud_cases.gd` (new)
- `apps/client/tests/hud.gd` (new)
- `apps/client/tests/smoke.gd`
- `docs/architecture/crpg-godot.md`
- `crates/crpg-godot/AGENTS.md`
- `tasks/T051-completion.md` (new)

Permissions: `goldens: none`.

## Open questions for the user

1. **Can the demo say "you won"?** `view()` exposes no player side, and ADR-0033
   forbids adding a member, so the HUD cannot tell the player's side from anyone
   else's. The options:
   - (a) a neutral verdict, `Winner: <side>` / `Winner: draw`, which this contract
     specifies;
   - (b) a `player_side()` member, which needs an ADR amendment and a new FFI
     surface;
   - (c) "You win/lose" inferred from the labels' targets, which is rule-guessing
     and wrong when the player has no options.

   **Recommendation: (a).** The demo still answers "who won", and full T051/T104 can
   add a player-relative verdict when the ruleset supplies the player's role.

2. **Should the end block name the encounter?** The view has no encounter display
   name; only `encounter_ended.encounter` carries the ULID text
   (`crates/crpg-godot/src/core/client.rs:197-201`), and the opening
   turn is state, not an event (ADR-0033:92-93). The options: (a) omit the name, as
   specified; (b) print the 26-character ULID; (c) add a presentation field later.
   **Recommendation: (a).** (b) is noise, and (c) is outside this slice.

3. **Is the shorter combat-log line acceptable?** The view's `action_resolved`
   carries ability, outcome and damage, but no dice, margin or target health
   (`client.rs:129-204`), so the HUD cannot print the CLI's `dice …; margin …` or
   `Goblin: 6/6 hp` lines (`crates/crpg-cli/src/demo.rs:363-392`). The options:
   (a) the B11 line, as specified; (b) extend the v2 event, which is a wire and
   protocol change (T140) and outside this task; (c) infer damage from health diffs,
   which spec §9.2 forbids. **Recommendation: (a).** It says who hit whom with what,
   and the health panel updates in the same revision.

No other question needs you. Every draft-level choice is settled above, and no
ADR-0033 decision is changed.

## Agent log

- 2026-11-03 (UTC) · opencode-go/deepseek-v4.1-flash + T051 draft · Wrote the blind contract draft for the cut-down HUD and end screen: exact HUD/log/end text over the existing `view()`, a revision-gated log of events plus rejected receipts with the RejectionCode text table, fake-view cases under the existing BR-22 step, and no FFI/Rust/wire/S-set change.

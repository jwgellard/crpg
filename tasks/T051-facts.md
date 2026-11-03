# T051 facts — fact sheet for the cut-down T051 contract (demo HUD)

Fact sheet for the cut-down T051 contract (demo HUD). Facts only. Every
`path:line` resolves on branch `task/T051-facts-b1`. T048 (scene sync,
`apps/client/scripts/SceneSync.gd`) is being implemented in parallel; files
are described as they stand on this branch.

## 1. `docs/CRPG_ENGINE_SPEC.md`

- `docs/CRPG_ENGINE_SPEC.md:363` — "It powers the character sheet tooltip, the combat log, the editor's rules debugger, and every rules test you will ever write." (ModifierBreakdown use.)
- `docs/CRPG_ENGINE_SPEC.md:779` — "`SimEvent(SimEvent),          // "Grigor took 7 slashing damage, critical"`" (DeltaOp variant).
- `docs/CRPG_ENGINE_SPEC.md:785` — "Sim events are the same event stream the combat log renders and the replay system records. One mechanism, three uses."
- `docs/CRPG_ENGINE_SPEC.md:865`–`866` — "UI               HUD, character sheet, inventory, dialogue, / combat log, timeline, spell book, journal" (client tree).
- `docs/CRPG_ENGINE_SPEC.md:880` — "The client renders `SimEvent`s, it does not infer them. Do not diff HP to discover that damage happened. The server sends `Damage { target, amount, type, outcome, source }` and the client plays the number, the sound, and the flinch. This makes the combat log, the VFX, and the replay system the same system."
- `docs/CRPG_ENGINE_SPEC.md:1079` — "**T2 Client-side** | UI themes, HUD layouts, model replacements, sounds | **None** | Never executes code. Data only. Never affects sim."
- `docs/CRPG_ENGINE_SPEC.md:1516` — "9. Attack the enemy, turn-based, real dice rolls, combat log, enemy dies."
- `docs/CRPG_ENGINE_SPEC.md:1590` — "**Deliverables:** `crpg-godot` bridge, scene sync, camera, click-to-move with prediction, basic HUD, combat log, dialogue UI, `SimEvent`-driven VFX and audio hooks."

## 2. `docs/adr/0033-godot-bridge.md`

- `docs/adr/0033-godot-bridge.md:24` — "T047 skeleton with embedded host, T048 proxies, a cut-down T051, playing".
- `docs/adr/0033-godot-bridge.md:154` — "8. if anything changed: revision += 1; rebuild CrpgView".
- `docs/adr/0033-godot-bridge.md:156` — "if v.revision() != seen: SceneSync.sync(v); Hud.sync(v)".
- `docs/adr/0033-godot-bridge.md:256` — "There are **no signals** in the slice: `poll()` then `view()` gives one explicit order".
- `docs/adr/0033-godot-bridge.md:266` — "`view() -> CrpgView` | The view from the last changing `poll`; an empty view with revision 0 before that".
- `docs/adr/0033-godot-bridge.md:270` — "`revision() -> int` | Increases when state, options, events or receipts change".
- `docs/adr/0033-godot-bridge.md:271` — "`net_ids() -> PackedInt64Array` | Visible entities, ascending. Every per-entity array below is indexed the same way".
- `docs/adr/0033-godot-bridge.md:278` — "`dead() -> PackedByteArray` | Terminal flag, `0` where unknown".
- `docs/adr/0033-godot-bridge.md:279` — "`active() -> int` | Active `NetId`, or `0` for none".
- `docs/adr/0033-godot-bridge.md:280` — "`round() -> int` | The wire round, unadjusted. The HUD adds 1, as `crpgc play` does".
- `docs/adr/0033-godot-bridge.md:281` — "`ended() -> bool` | An `EncounterEnded` event was applied".
- `docs/adr/0033-godot-bridge.md:282` — "`option_labels() -> PackedStringArray` | `"<ability> -> <target>"` or `"End turn"`, the `crpgc play` wording".
- `docs/adr/0033-godot-bridge.md:283` — "`option_targets() -> PackedInt64Array` | Target `NetId` per option, `0` for End turn".
- `docs/adr/0033-godot-bridge.md:284`–`285` — "`events() -> Array` of dictionaries | Events that produced this revision, oldest first" / "`receipts()` ... oldest first".
- `docs/adr/0033-godot-bridge.md:287` — "Empty option arrays mean it is not the player's turn."
- `docs/adr/0033-godot-bridge.md:291` — "`action_resolved`: `actor`, `target`, `ability` (display name), `ability_id`, `outcome`, `damage`".
- `docs/adr/0033-godot-bridge.md:292` — "`turn_started`: `actor`, `round`".
- `docs/adr/0033-godot-bridge.md:293` — "`died`: `entity`".
- `docs/adr/0033-godot-bridge.md:294` — "`encounter_ended`: `encounter`, `round`".
- `docs/adr/0033-godot-bridge.md:297` — "`receipts()` entries are `seq`, `applied` (bool) and `code` (the frozen `RejectionCode` number, or `0`). Rendering events, not diffing health, keeps spec §9.2 rule 2."
- `docs/adr/0033-godot-bridge.md:349` — "Mapping `RejectionCode` numbers to player text is T051's job."
- `docs/adr/0033-godot-bridge.md:522`–`524` — "**Cut-down T051** is GDScript over `view()`: turn, health, option buttons, log, end screen. It adds no FFI member and has no T104/T108 prerequisite."
- `docs/adr/0033-godot-bridge.md:534` — "the FFI is 6 + 16 members, and T048 and T051 should need no Rust."
- `docs/adr/0033-godot-bridge.md:615` — "| Option list | **B** saw the gap, but placed the fix in T051, which is the wrong crate. T047a owns it |"

## 3. Bridge FFI (`crates/crpg-godot/src/`)

`#[godot_api]` items. Only `crates/crpg-godot/src/shell.rs` contains them (behind `engine` feature). Doc-comment first lines shown.

`CrpgSession` (`shell.rs`):

- `crates/crpg-godot/src/shell.rs:39` — `#[func] fn open(campaign_dir: GString, encounter_id: GString, player_side: GString, seed: i64) -> i64`; doc "Opens a session; 0 on success, otherwise a bridge code." (not a fact source for turn/health/options/events).
- `crates/crpg-godot/src/shell.rs:61` — `#[func] fn poll(&mut self) -> i64`; doc "Advances the session by the wall-clock time since `open`." (drives view; not a per-fact accessor).
- `crates/crpg-godot/src/shell.rs:70` — `#[func] fn view(&self) -> Gd<CrpgView>`; doc "A new immutable snapshot of the current view." (returns all facts).
- `crates/crpg-godot/src/shell.rs:78` — `#[func] fn submit_option(&mut self, revision: i64, index: i64) -> i64`; doc "Submits the option at `index` of the view with `revision`." (choice submission).
- `crates/crpg-godot/src/shell.rs:84` — `#[func] fn close(&mut self)`; doc "Closes the session; idempotent.".
- `crates/crpg-godot/src/shell.rs:91` — `#[func] fn last_error(&self) -> GString`; doc "The last error text, `NAME` or `NAME: detail`.".

`CrpgView` (`shell.rs`):

- `crates/crpg-godot/src/shell.rs:107` — `#[func] fn revision(&self) -> i64`; doc "The revision number; 0 is the empty view." (change detection; not turn/health/options/events).
- `crates/crpg-godot/src/shell.rs:113` — `#[func] fn net_ids(&self) -> PackedInt64Array`; doc "Network ids, ascending." (row index key).
- `crates/crpg-godot/src/shell.rs:119` — `#[func] fn names(&self) -> PackedStringArray`; doc "Display names, parallel to `net_ids`.".
- `crates/crpg-godot/src/shell.rs:125` — `#[func] fn sides(&self) -> PackedStringArray`; doc "Side names, parallel to `net_ids`.".
- `crates/crpg-godot/src/shell.rs:131` — `#[func] fn placements(&self) -> PackedStringArray`; doc "Placement ULID text, parallel to `net_ids`.".
- `crates/crpg-godot/src/shell.rs:137` — `#[func] fn health(&self) -> PackedInt64Array`; doc "Current health, parallel to `net_ids`." → **health**.
- `crates/crpg-godot/src/shell.rs:143` — `#[func] fn max_health(&self) -> PackedInt64Array`; doc "Maximum health, parallel to `net_ids`." → **health**.
- `crates/crpg-godot/src/shell.rs:149` — `#[func] fn health_known(&self) -> PackedByteArray`; doc "1 where health is known, else 0." → **health**.
- `crates/crpg-godot/src/shell.rs:155` — `#[func] fn dead(&self) -> PackedByteArray`; doc "1 where the entity is dead, else 0." → **health/encounter state**.
- `crates/crpg-godot/src/shell.rs:161` — `#[func] fn active(&self) -> i64`; doc "The active entity's network id, or 0." → **turn** (active actor).
- `crates/crpg-godot/src/shell.rs:167` — `#[func] fn round(&self) -> i64`; doc "The turn round." → **turn**.
- `crates/crpg-godot/src/shell.rs:173` — `#[func] fn ended(&self) -> bool`; doc "Whether the encounter has ended." → **encounter-over**.
- `crates/crpg-godot/src/shell.rs:179` — `#[func] fn option_labels(&self) -> PackedStringArray`; doc "The player's option labels, in host order." → **options/legal intents**.
- `crates/crpg-godot/src/shell.rs:185` — `#[func] fn option_targets(&self) -> PackedInt64Array`; doc "The target id of each option, 0 for end turn." → **options/legal intents**.
- `crates/crpg-godot/src/shell.rs:191` — `#[func] fn events(&self) -> Array<VarDictionary>`; doc "This revision's events, oldest first, each a dictionary with a `kind`." → **events/log lines**.
- `crates/crpg-godot/src/shell.rs:201` — `#[func] fn receipts(&self) -> Array<VarDictionary>`; doc "This revision's receipts, oldest first." → **receipts**.

No `outcome`/victory/defeat accessor beyond `ended()` and event dictionaries (`encounter_ended` event carries `encounter`, `round`). Outcome verdict text is not in the FFI; `action_resolved.outcome` is the symbolic outcome string.

## 4. `apps/client/scripts/CombatHud.gd`, `Main.gd`, `scenes/Main.tscn`

`CombatHud.gd` (CanvasLayer):

- `apps/client/scripts/CombatHud.gd:1` — "extends CanvasLayer".
- `apps/client/scripts/CombatHud.gd:2`–`4` — "T047d: round, active name, health lines, one button per option, the event / log and the end label. It renders the view and calls `choose`; it decides no / rule (ADR-0033 §14)."
- `apps/client/scripts/CombatHud.gd:6` — "var main: Node = null".
- `apps/client/scripts/CombatHud.gd:8`–`14` — fields `_round_label`, `_active_label`, `_health_box`, `_options_box`, `_log`, `_end_label`, `_status_label`, `_logged_revision`.
- `apps/client/scripts/CombatHud.gd:17`–`19` — `_ready` sets `main = get_parent()` and calls `_build()`.
- `apps/client/scripts/CombatHud.gd:21`–`62` — `_build` creates PanelContainer, MarginContainer, VBoxContainer; `_round_label`, `_active_label`, `_health_box`, `_options_box`, `_log` (RichTextLabel, min size y 120, scroll_following), `_status_label` (hidden), `_end_label` (hidden).
- `apps/client/scripts/CombatHud.gd:63` — "func sync(view) -> void:".
- `apps/client/scripts/CombatHud.gd:64`–`65` — null guard.
- `apps/client/scripts/CombatHud.gd:66` — `_round_label.text = "Round %d" % (view.round() + 1)`.
- `apps/client/scripts/CombatHud.gd:67` — `_active_label.text = "Active: %s" % _name_for(view, view.active())`.
- `apps/client/scripts/CombatHud.gd:68` — `_sync_health(view)`.
- `apps/client/scripts/CombatHud.gd:69` — `_sync_options(view)`.
- `apps/client/scripts/CombatHud.gd:70` — `_sync_events(view)`.
- `apps/client/scripts/CombatHud.gd:71` — `_end_label.text = "Encounter ended"`.
- `apps/client/scripts/CombatHud.gd:72` — `_end_label.visible = view.ended()`.
- `apps/client/scripts/CombatHud.gd:74`–`78` — `show_error(text)` sets status label red and appends "ERROR: " + text to log.
- `apps/client/scripts/CombatHud.gd:80`–`81` — `show_notice(text)` appends text to log.
- `apps/client/scripts/CombatHud.gd:83`–`101` — `_sync_health`: clears box; reads `view.net_ids()`, `names()`, `health()`, `max_health()`, `health_known()`, `dead()`; line format `"<name>: ?"` if unknown else `"<name>: <health>/<max_health>"`, appends " (dead)" if dead.
- `apps/client/scripts/CombatHud.gd:103`–`110` — `_sync_options`: clears box; one Button per `view.option_labels()` with text label; `pressed` connects `_on_option.bind(i)`.
- `apps/client/scripts/CombatHud.gd:112`–`119` — `_sync_events`: skips if `view.revision()` == `_logged_revision`; else sets logged revision and appends `_event_text` per `view.events()`.
- `apps/client/scripts/CombatHud.gd:120`–`141` — `_event_text`: action_resolved → `"%s -> %s: %s (%s, %d)" % [actor name, target name, ability, outcome, damage]`; turn_started → `"Turn: %s (round %d)" % [actor name, round]`; died → `"Died: %s"`; encounter_ended → `"Encounter ended (round %d)"`; default → kind.
- `apps/client/scripts/CombatHud.gd:143`–`147` — `_name_for` returns "-" if net_id not found else `view.names()[index]`.
- `apps/client/scripts/CombatHud.gd:149`–`154` — `_on_option(index)` calls `main.choose(index)`; on code not 0/4/5 shows error with `main.session.last_error()`.
- `apps/client/scripts/CombatHud.gd:156`–`159` — `_clear` removes and frees children.

`Main.gd` (Node3D):

- `apps/client/scripts/Main.gd:1` — "extends Node3D".
- `apps/client/scripts/Main.gd:2`–`3` — "T047d: owns the session, polls it once per frame, and feeds both the proxy / view and the combat HUD. No rule is decided here (ADR-0033 §14)."
- `apps/client/scripts/Main.gd:5` — "const OPEN_SEED := 1".
- `apps/client/scripts/Main.gd:7`–`11` — fields `session`, `seen`, `view`, `open_code`, `stop_code`.
- `apps/client/scripts/Main.gd:13`–`23` — `_ready`: instantiates `CrpgSession`; on null sets `open_code = 2` and shows error; calls `session.open(_campaign_dir(), "", "", OPEN_SEED)`; on non-zero sets `open_code` and shows error; else `view = session.view()`.
- `apps/client/scripts/Main.gd:26`–`43` — `_process`: guards on session/open_code/stop_code; `session.poll()`; on code 30 shows notice; on other non-zero sets stop_code, shows error, returns; then `next = session.view()`, `view = next`; if revision changed updates `seen`, calls `$Proxies.sync(next)` and `$Hud.sync(next)`.
- `apps/client/scripts/Main.gd:45`–`48` — `choose(index)` returns 1 if session null else `session.submit_option(seen, index)`.
- `apps/client/scripts/Main.gd:50`–`53` — `_exit_tree` closes session.
- `apps/client/scripts/Main.gd:55`–`59` — `_campaign_dir`: uses `CRPG_CAMPAIGN` env if non-empty else `res://` joined to `../../campaigns/fixtures/combat_basic`.

`Main.tscn`:

- `apps/client/scenes/Main.tscn:3`–`5` — ext resources Main.gd, SceneSync.gd, CombatHud.gd.
- `apps/client/scenes/Main.tscn:10` — root `Main` Node3D with Main.gd.
- `apps/client/scenes/Main.tscn:13` — Camera3D transform.
- `apps/client/scenes/Main.tscn:16` — Sun DirectionalLight3D.
- `apps/client/scenes/Main.tscn:19` — Ground MeshInstance3D.
- `apps/client/scenes/Main.tscn:22`–`23` — `Proxies` Node3D with SceneSync.gd.
- `apps/client/scenes/Main.tscn:25`–`26` — `Hud` CanvasLayer with CombatHud.gd.

## 5. `docs/architecture/crpg-godot.md` and `crates/crpg-godot/AGENTS.md`

Architecture doc Client section (`docs/architecture/crpg-godot.md:165`–`203`):

- `docs/architecture/crpg-godot.md:165` — "## Client (T047d)".
- `docs/architecture/crpg-godot.md:167`–`170` — "`apps/client` is the terminal-free slice... All of it is presentation; the options and ids come from the / bridge, and the scene decides no rule (ADR-0033 §14)."
- `docs/architecture/crpg-godot.md:172`–`180` — `Main.gd` contract: exposes `session`, `seen`, `view`, `choose(i)`; `_ready` instantiates and opens (fixture, default side, seed 1); `_process` one poll; 30 is notice; other non-zero sets stop_code; `_exit_tree` closes; open_code/stop_code read by smoke test.
- `docs/architecture/crpg-godot.md:181`–`187` — `SceneSync.gd` spawns once from first non-empty view; no despawn (T048); proxy naming and layout; `proxy_count()`.
- `docs/architecture/crpg-godot.md:188`–`193` — `CombatHud.gd`: "Round <round + 1>, Active: <name>, one health line per row (`?` when health_known is 0, `(dead)` from dead column), one button per option calling `choose(index)`, event log from `events()` (once per revision), status line, end label when `ended()`."
- `docs/architecture/crpg-godot.md:194`–`201` — `tests/smoke.gd` (E2): chooses option 0; exits 0 on ended with proxy_count == net_ids size == 2; 5 mismatch; 3 failed open/poll/choose; 2 class missing; 4 after 120 s.
- `docs/architecture/crpg-godot.md:202` — "No input. Camera control and picking are T050...".
- `docs/architecture/crpg-godot.md:203` — "T048 owns respawn/despawn and T049 positions."
- `docs/architecture/crpg-godot.md:232`–`233` — "The default opening actor is the goblin, so with `player_side = ""` the / first poll has no options; `load.gd` polls until the Hero's turn arrives."
- `docs/architecture/crpg-godot.md:238`–`250` — Windows client build in CI: scope, cache, clippy, build --features engine, notice, Godot fetch, import, load.gd, control, smoke.gd, control, artifact; G3/G5 exit 3.

T051/deferred lines in architecture doc:

- `docs/architecture/crpg-godot.md:182` — "there / is no despawn; that is T048)".
- `docs/architecture/crpg-godot.md:188`–`193` — CombatHud description (above).
- `docs/architecture/crpg-godot.md:202`–`203` — "T048 owns respawn/despawn and T049 positions."
- `docs/architecture/crpg-godot.md:257` — "Added the Client section: ... so T048/T050 extend a stated baseline rather than re-deriving one."

`crates/crpg-godot/AGENTS.md` T051/deferred lines:

- `crates/crpg-godot/AGENTS.md:105`–`108` — "The fixture's opening actor is the goblin... The default player side is / still the first combatant's (Hero), so the default view has no player / options on the first poll; tests that need options open with `"Goblin"`."
- `crates/crpg-godot/AGENTS.md:120`–`121` — "Do not widen K31's clock ban back over `shell.rs`."
No other "T051" or "deferred" lines in `crates/crpg-godot/AGENTS.md`.

Engine-gate test list (AGENTS.md):

- `crates/crpg-godot/AGENTS.md:73`–`83` — cargo build/clippy with --features engine, Godot import and load.gd/smoke.gd exit codes.

## 6. Every mention of T051

- `docs/IMPLEMENTATION_PLAN.md:256` — "| T051 | C → I | crpg-godot | T048, T104, T108 | HUD, combat log, `SimEvent`-driven VFX/audio hooks. Cut-down demo version has no T104/T108 prerequisite (ADR-0033). |"
- `docs/IMPLEMENTATION_PLAN.md:264` — "Player UX ... findings become T050/T051 acceptance criteria".
- `docs/IMPLEMENTATION_PLAN.md:276` — "| T057 | C → I | crpg-godot | T051, T054 | Dialogue UI and quest journal |"
- `docs/IMPLEMENTATION_PLAN.md:412` — "Demo-first (user, 2026-11-01) | T027b, T046, T045, T047, T048, cut-down T051 | — | ahead of movement, nav and the rest of M4 |"
- `docs/IMPLEMENTATION_PLAN.md:452` — "ruleset-driven player UI in T104 and T108 (gating T051)".
- `docs/IMPLEMENTATION_PLAN.md:454` — "cut-down T051 ahead of movement, nav and the rest of M4".
- `docs/IMPLEMENTATION_PLAN.md:455` — "noted the cut-down T051 has no T104/T108 prerequisite."
- `docs/PROJECT_STATE.md:7` — "T048 implementation once approved, then the cut-down T051; T047e stays a follow-up."
- `docs/adr/0029-kernel-range-and-time-model.md:17` — "It gates T034 (movement ADR), T051 (HUD) and T109 (a third ruleset)."
- `docs/adr/0029-kernel-range-and-time-model.md:376` — "The client draws each kind with its own widget (T104/T051)."
- `docs/adr/0029-kernel-range-and-time-model.md:410` — "`crpg-godot` consumes (T051)."
- `docs/adr/0029-kernel-range-and-time-model.md:466` — "**T051** builds on U1–U10 and the N1 facts. **T072** carries the UI document."
- `docs/adr/0033-godot-bridge.md:24` — "a cut-down T051".
- `docs/adr/0033-godot-bridge.md:349` — "Mapping `RejectionCode` numbers to player text is T051's job."
- `docs/adr/0033-godot-bridge.md:522` — "**Cut-down T051** is GDScript over `view()`".
- `docs/adr/0033-godot-bridge.md:524` — "on full T051."
- `docs/adr/0033-godot-bridge.md:534` — "T048 and T051 should need no Rust."
- `docs/adr/0033-godot-bridge.md:615` — "placed the fix in T051, which is the wrong crate."
- `docs/design/T108/check-draft-beam.md:18` — "T051 waits on T108 and T104 at plan:243".
- `docs/design/T108/draft-beam.md:278` — "T051 (HUD, combat log, `SimEvent`-driven VFX/audio) waits on T108 and".
- `docs/design/T108/draft-beam.md:345` — "T104 (player UX research) and T051 (HUD) inherit the `ui` descriptor".
- `docs/design/T108/draft-beam.md:346` — "T051's acceptance criteria should check that a ruleset with no".
- `docs/design/T108/draft-longcat.md:362` — "T050/T051 wait on both (D28m,".
- `tasks/DECISIONS-2026-10-16.md:119` — "T104: player-UX research and a combat-HUD prototype before the T050/T051".
- `tasks/DECISIONS-2026-10-16.md:151` — "T051 waits on both."
- `tasks/T027b-facts.md:60` — "PLAN demo-first ... cut-down T051".
- `tasks/T045-draft-a.md:37` — "a cut-down T051".
- `tasks/T045-draft-a.md:279` — "T051 cut-down ... HUD, combat log, `SimEvent`-driven VFX/audio".
- `tasks/T045-draft-a.md:282` — "Full T051 waits on T048, T104, T108".
- `tasks/T045-draft-b.md:47` — "a cut-down T051 that shows and plays turn-based combat as `crpgc play` does".
- `tasks/T045-draft-b.md:264` — "to UI text is T051's job."
- `tasks/T045-draft-b.md:319` — "T051 consumes `events()`, `receipts()` and the signals".
- `tasks/T045-draft-b.md:337` — "T051 adds one trusted host query returning".
- `tasks/T045-facts.md:62` — "T051 ... prerequisites T048, T104, T108".
- `tasks/T045-facts.md:63` — "findings become T050/T051 acceptance criteria".
- `tasks/T046-draft-a.md:597` — "the combat log and `ended()` (T047/T051)".
- `tasks/T047-draft-a.md:601` — "T051's full HUD/log/VFX".
- `tasks/T047-draft-b.md:281` — "to player text is T051's job."
- `tasks/T047-draft-b.md:465` — "cut-down T051 HUD/".
- `tasks/T047-draft-b.md:497` — "cut-down T051 HUD/log".
- `tasks/T047.md:642` — "cut-down T051 HUD beyond T047d's".
- `tasks/T048-draft-a.md:12` — "HUD T051".
- `tasks/T048-draft-a.md:327` — "T051 HUD, log, VFX/audio".
- `tasks/T048-draft-a.md:367` — "T051: the proxy shows no numbers and consumes no events;".
- `tasks/T048-draft-b.md:14` — "HUD T051 (cut-down)".
- `tasks/T048-draft-b.md:85` — "HUD turn/health/options/log/end work beyond reading the same view (cut-down T051, ...)".
- `tasks/T048-draft-b.md:93` — "HUD keeps sole ownership of round/active/health/options/log/end rendering (`apps/client/scripts/CombatHud.gd:63-147`)".
- `tasks/T048-facts.md:22` — "a cut-down T051, playing".
- `tasks/T048-facts.md:35` — "T048 and T051 should need no Rust."
- `tasks/T048-facts.md:57` — "T048 and T051 should need no Rust."
- `tasks/T048-facts.md:58` — "T048 and the cut-down T051".
- `tasks/T048-facts.md:60` — "T051 | ... | T048, T104, T108 | ...".
- `tasks/T048-facts.md:61` — "Demo-first ... cut-down T051".
- `tasks/T048-facts.md:69` — "Full T051 waits on T048, T104, T108".
- `tasks/T048-facts.md:74` — "T051 ... prerequisites T048, T104, T108".
- `tasks/T048.md:328` — "T051 HUD changes and event-driven VFX."
- `tasks/T048.md:391` — "T051 keeps all text in the HUD."
- `tasks/T138.md:945` — "Revisit with T051/T141."

## 7. Headless Godot tests and CI `client` job

Tests:

- `apps/client/tests/load.gd:1`–`5` — E1 (T047b): proves gdext library loads and CrpgSession works end to end; exit 0 pass; 2 class missing; 3 first failed check; control CRPG_CAMPAIGN missing → open = 20 → exit 3.
- `apps/client/tests/load.gd:18`–`21` — exits 2 when `ClassDB.class_exists("CrpgSession")` false.
- `apps/client/tests/load.gd:26`–`29` — opens with seed 1; non-zero open → exit 3.
- `apps/client/tests/load.gd:30`–`33` — first poll must be 0.
- `apps/client/tests/load.gd:34`–`43` — revision == 1; net_ids size == 2; names == ["Hero", "Goblin"].
- `apps/client/tests/load.gd:46`–`57` — waits up to 5000 ms for option_labels (goblin opens, NPC acts after 600 ms).
- `apps/client/tests/load.gd:58`–`60` — events is Array.
- `apps/client/tests/load.gd:61` — `submit_option(-1, 0) == 3`.
- `apps/client/tests/load.gd:64` — `submit_option(rev, 99) == 6`.
- `apps/client/tests/load.gd:67`–`70` — after close, poll == 1.
- `apps/client/tests/smoke.gd:1`–`8` — E2 (T047d): playthrough of Main.tscn; exits 0 on ended with proxy_count == net_ids size == 2; 5 mismatch; 3 failed open/poll/choose; 2 class missing; 4 after 120 s.
- `apps/client/tests/smoke.gd:10` — `TIMEOUT_MS := 120000`.
- `apps/client/tests/smoke.gd:34`–`46` — quits 3 on non-zero open_code/stop_code.
- `apps/client/tests/smoke.gd:47`–`58` — on ended checks proxy_count == ids.size() == 2; mismatch → exit 5.
- `apps/client/tests/smoke.gd:59`–`66` — chooses option 0 when options exist and revision not chosen; retries 4/5; other failure → exit 3.

CI `client` job (`.github/workflows/ci.yml`, Windows only):

- `docs/architecture/crpg-godot.md:238`–`248` — job runs scope, cache, clippy, build --features engine, notice check, Godot fetch, G1 import, G2 load.gd, G3 control, G4 smoke.gd, G5 control, artifact; G3/G5 must exit 3.
- `.github/workflows/ci.yml:192` — "client:" job, `runs-on: windows-latest`, `timeout-minutes: 60`.
- `.github/workflows/ci.yml:195`–`219` — scope step (documentation-only PRs skip gates).
- `.github/workflows/ci.yml:220`–`221` — rust-cache; clippy `--features engine --all-targets`.
- `.github/workflows/ci.yml:222` — build `--features engine`.
- `.github/workflows/ci.yml:223`–`237` — licence notice check (8 packages).
- `.github/workflows/ci.yml:238`–`266` — fetch Godot 4.7.2, digest check, version check.
- `.github/workflows/ci.yml:267`–`276` — seed extension list.
- `.github/workflows/ci.yml:277`–`286` — import project (exit 0).
- `.github/workflows/ci.yml:287`–`296` — E1 load proof (exit 0).
- `.github/workflows/ci.yml:297`–`308` — E1 control (exit 3).
- `.github/workflows/ci.yml:309`–`318` — BR-22 smoke (exit 0).
- `.github/workflows/ci.yml:319`–`330` — BR-22 control (exit 3).
- `.github/workflows/ci.yml:331`–`360` — stage artifact `crpg-client-slice-windows-x86_64`.

## 8. Test campaign (default `CRPG_CAMPAIGN`)

Default path: `apps/client/scripts/Main.gd:55`–`59` — uses `CRPG_CAMPAIGN` env if set else `res://` joined to `../../campaigns/fixtures/combat_basic` (globalized). Also `apps/client/tests/load.gd:11`–`15`.

Campaign identity:

- `campaigns/fixtures/combat_basic/campaign.json:...` — name "fixture.combat-basic"; id `...0000000000000000000000000B`; engine ">=0.1.0, <1.0.0"; entry area `...0D`, spawn `...0E`, world `...0C`.

Combatants (from placements + creatures + locale):

- Hero: `campaigns/fixtures/combat_basic/creatures/hero.json` — id `...0G`, name key `fixture.hero` → "Hero" (`locale/en.json`), health stat 655360 (10.0 as Fx16_16), stats might 524288, guile 393216, resolve 458752; faction null → side "Hero"; placement `...0E` (`hero-spawn`), initiative 10.
- Goblin: `campaigns/fixtures/combat_basic/creatures/goblin.json` — id `...0H`, name key `fixture.goblin` → "Goblin", health 393216 (6.0), might 393216, guile 458752, resolve 327680; side "Goblin"; placement `...0F` (`goblin-spawn`), initiative 9.

Encounter:

- `campaigns/fixtures/combat_basic/encounters/first-blood.json` — id `...0N`, name key `fixture.first-blood` → "First Blood", ruleset `...0J` (minimal-d6), participants Hero (initiative 10, placement `...0E`) and Goblin (initiative 9, placement `...0F`).

Ruleset/ability:

- `campaigns/fixtures/combat_basic/rulesets/minimal-d6.json` — one ability `...0K` (Strike), health stat "health", action pool max 1 refresh on_turn_start.
- `campaigns/fixtures/combat_basic/abilities/strike.json` — "Strike", attribute might, dice 2d6, cost 1, damage 2 on success / 0 on failure, requires target, allow_self_target false, outcome table `...0M` (roll-under).
- `campaigns/fixtures/combat_basic/outcome_tables/roll-under.json` — bands: min_margin -inf → success; min_margin 1 → failure.

Encounter end (victory/defeat rule): `crates/crpg-sim/src/combat.rs:3336`–`3348` — when fewer than two combatants alive (`alive < 2`), active set to None and `encounter_ended` fact emitted. `crates/crpg-sim/src/combat.rs:3765` — "fewer than two" (T138 D15) for RT. So encounter ends when fewer than 2 combatants alive; summary winner = sole surviving side, or "draw" if more than one side alive (or none) (`crates/crpg-cli/src/demo.rs:528`–`548`).

## 9. CLI terminal play (`crpgc play`)

- `crates/crpg-cli/src/play.rs:1` — "`crpgc play` — playable terminal combat (T146)."
- `crates/crpg-cli/src/play.rs:39`–`46` — `run(campaign_root, encounter, side, seed, engine_text)`.
- `crates/crpg-cli/src/play.rs:100` — `show.header(&world)` prints campaign/encounter header and combatants with hp.
- `crates/crpg-cli/src/play.rs:101` — `show.after(&world)` prints deaths/round/turn.
- `crates/crpg-cli/src/play.rs:102`–`106` — loop: if no active combatant, `show.summary()` and success.
- `crates/crpg-cli/src/play.rs:110`–`113` — legal_actions for active.
- `crates/crpg-cli/src/play.rs:114`–`127` — player side prompts; NPC side uses fixed rule (first attack on living opponent, else first).
- `crates/crpg-cli/src/play.rs:128` — `adapter.perform(&mut world, &chosen)`.
- `crates/crpg-cli/src/play.rs:129` — `print_applied` prints action then `show.after`.
- `crates/crpg-cli/src/play.rs:135`–`141` — prompt block: round/turn header, health lines `"<name> <health>/<max> hp"`, "Options:", numbered options, "q. quit".
- `crates/crpg-cli/src/play.rs:185`–`203` — option text: `"<ability> -> <target>"` or "End turn" or "Stop".
- `crates/crpg-cli/src/demo.rs:329` — `header`: "== Combat demo: {campaign} ==\nEncounter: {encounter}\nCombatants:\n  {name} | side {side} | {hp}/{max} hp\n".
- `crates/crpg-cli/src/demo.rs:350` — `attack`: "[{n}] {actor} | {ability} -> {target}\n" then dice/total/faces, margin, verdict, damage, target hp.
- `crates/crpg-cli/src/demo.rs:398` — `end_turn`: "[{n}] {actor} | ends turn\n".
- `crates/crpg-cli/src/demo.rs:409` — `after`: deaths "  {name} dies.\n"; round "-- Round {n} --\n" only when changed; "Turn: {name}".
- `crates/crpg-cli/src/demo.rs:439` — `round_and_turn`: round line + turn line.
- `crates/crpg-cli/src/demo.rs:497` — `summary`: "== Summary ==\nRounds played: ...\nFinal health:\n  {name} {hp}/{max} hp (dead)\nSurvivors: ...\nWinner: ...\n".

## Agent log
- 2026-11-03 (UTC) · opencode-go/beam-free + T051 facts · Fact sheet of repository facts for the cut-down T051 contract.
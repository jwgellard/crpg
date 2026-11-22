# T156 facts — combat demo v1 (enemy AI, click to attack, visual feedback)

## Requirements

- `Spec §155` (`docs/CRPG_ENGINE_SPEC.md:155`) — "Windows single-player embeds the same platform-neutral authoritative server implementation in-process, behind an in-memory `Transport` boundary."
- `Spec §724` (`docs/CRPG_ENGINE_SPEC.md:724`) — "AI is server-side, budgeted, and time-sliced. Combat AI computes on turn start with a node budget."
- `Spec §888` (`docs/CRPG_ENGINE_SPEC.md:888`) — "The client renders `SimEvent`s, it does not infer them. ... The server sends `Damage { target, amount, type, outcome, source }` and the client plays the number, the sound, and the flinch."
- `Spec §875` (`docs/CRPG_ENGINE_SPEC.md:875`) — "VFX / audio      driven by SimEvent stream."
- `Spec §1598` (`docs/CRPG_ENGINE_SPEC.md:1598`) — Phase 5 deliverables include "SimEvent-driven VFX and audio hooks."
- `Spec §19.1` (`docs/CRPG_ENGINE_SPEC.md:1649`) — very-small backlog items include "A Godot scene that spawns N `Node3D` proxies from an external array."
- `ADR-0012 Decision 3` (`docs/adr/0012-windows-primary-platform.md:15`) — "One platform-neutral authoritative server implementation is hosted in-process for Windows single-player and by dedicated processes on Windows and Linux."
- `ADR-0012 Decision 4` (`docs/adr/0012-windows-primary-platform.md:30`) — "The client/server authority boundary is unchanged in-process. Transport may be in-memory, but clients still cannot mutate authoritative state directly. Embedded hosting is not a separate single-player simulation."
- `ADR-0033 §2` (`docs/adr/0033-godot-bridge.md:67`) — "The embedded authority is built once, then wrapped."
- `ADR-0033 Consequences` (`docs/adr/0033-godot-bridge.md:502`) — "What the thin slice cuts. ... all art, audio and VFX (primitives, plus event hooks with nothing attached); ... real AI; ... click-to-move and the camera rig".
- `T050` (`tasks/T050.md:19`) — "through the embedded single-player server (the same authoritative host on its run loop; no client-side stand-in for a server duty)."
- `T051` (`tasks/T051.md:498`) — out of scope lists "T050: camera, picking, selection, active highlight, keyboard input" and VFX hooks are deferred.
- `T056` (`tasks/T056.md:1`) — "T056 — MVP AI: move to the nearest hostile and attack (`crpg-ai`, then `crpg-testkit`)".
- `T153` (`tasks/T153.md:250`) — "Single-player hosts auto-pause on `EncounterStarted` (`SINGLE_PLAYER_AUTO_PAUSE_BITS`, `:423–425`) and a restored host starts paused (`:901`)".

## AI driver today

- `Host::drive_ai` (`crates/crpg-server/src/host/ai.rs:85`) — signature `&mut self, sides: &crpg_ai::Sides, actors: &[EntityId], budget: AiBudget -> Result<AiReport, HostError>`; returns `Err(HostError::Closed)` only for a shut-down host, otherwise `Ok`; acts for turn-based head (if in `actors`) at most once per call within `decisions_per_turn`, or for up to `actors_per_drive` real-time candidates.
- `AiBudget`/`MVP_AI_BUDGET` (`crates/crpg-server/src/host/ai.rs:19`, `:28`) — `decisions_per_turn: u32`, `actors_per_drive: u32`; MVP is 16/8.
- `AiStep`/`AiReport` (`crates/crpg-server/src/host/ai.rs:35`, `:63`) — `AiStep::{Applied, Refused, Undecided}`; `AiReport { steps: Vec<AiStep>, capped: bool }`.
- Dedicated binary `AiFile` (`crates/crpg-server/src/main.rs:309`) — `sides: Vec<SideEntry>`, `controlled: Vec<EntityId>`; `crates/crpg-server/src/main.rs:678` reads `entry.control` into `ControlGrant { actors }` for invitations; `crates/crpg-server/src/main.rs:718` builds `Some((sides, file.controlled))` from the AI file.
- Dedicated binary AI hook (`crates/crpg-server/src/main.rs:778`) — `if let Some((sides, controlled)) = ai { let hook = Box::new(move |quic, _now| { quic.drive_ai(&sides, &controlled, MVP_AI_BUDGET).map(|_report| ()) }); hosting.set_ai(Some(hook)); }`.
- `QuicHosting::drive_ai` pass-through (`crates/crpg-server/src/run.rs:934`) — `match self.ai.as_mut() { Some(hook) => hook(&mut self.quic, now_ms), None => Ok(()) }`.
- `QuicHost::drive_ai` (`crates/crpg-server/src/quic.rs:2737`) — `pub fn drive_ai(&mut self, sides: &crpg_ai::Sides, actors: &[EntityId], budget: AiBudget) -> Result<AiReport, QuicHostError>`; `Poisoned` first, then `self.host.drive_ai(...)`.
- `LocalHost::set_ai`/`ai` field (`crates/crpg-server/src/run.rs:343`, `:397`) — `ai: Option<AiHook>`; `set_ai(&mut self, hook: Option<AiHook>)`.
- `HostLoop::run_cycle` (`crates/crpg-server/src/run.rs:183`) — calls `self.hosting.intake(now_ms)?; self.hosting.drive_ai(now_ms)?; ...` (AI stage is between intake and advance).
- `runner::spawn` (`crates/crpg-server/src/runner.rs:88`) — drives `host_loop.step(clock.now_ms())` on a background thread; no AI-specific step beyond the loop's `drive_ai`.
- Embedded host AI: `crates/crpg-godot/src/core/mod.rs:596` calls `local.set_ai(Some(NpcHook::new(npc, npc_sides).into_hook()))`, not the server's `drive_ai`/MVP budget.

## Embedded host in crpg-godot

- `BridgeCore::open` (`crates/crpg-godot/src/core/mod.rs:156`) — `(&mut self, campaign_dir: &str, encounter_id: &str, player_side: &str, seed: i64) -> BridgeCode`, builds a v2 `HistoryWorld`/`Host` behind a `LocalLink`.
- `BridgeCore::open_v4` (`crates/crpg-godot/src/core/mod.rs:173`) — same signature, turn-based v4 session on `HostLoop`/`LocalHost`/runner.
- `BridgeCore::open_v4_with` (`crates/crpg-godot/src/core/mod.rs:190`) — `pub(crate)` with `clock: impl Clock`.
- `BridgeCore::open_v4_rt` (`crates/crpg-godot/src/core/mod.rs:210`) — same signature, real-time v4 session.
- `BridgeCore::open_v4_rt_with` (`crates/crpg-godot/src/core/mod.rs:227`) — `pub(crate)` with `clock: impl Clock`.
- `BridgeCore::open_on` (`crates/crpg-godot/src/core/mod.rs:380`) — shared v2 open body.
- `BridgeCore::open_v4_guarded` (`crates/crpg-godot/src/core/mod.rs:493`) — v4 body; builds `Host::from_history`, binds player via `bind_v4_peer` and NPC via `bind_peer`, attaches `LocalHost`, sets `NpcHook` as AI hook (`:596`), spawns runner (`:600`).
- `BridgeCore::controlled` (`crates/crpg-godot/src/core/mod.rs:247`) — `&self -> Vec<i64>`; player replica ids ascending, empty outside `OpenV4`.
- Campaign fixture (`campaigns/fixtures/combat_basic/encounters/first-blood.json:1`) — participants initiative 10/9; ruleset `minimal-d6` with no `movement` or `round_ticks` (`campaigns/fixtures/combat_basic/rulesets/minimal-d6.json` grep returned none).
- Campaign area (`campaigns/fixtures/combat_basic/areas/start/area.json:1`) — bounds 0..65536 per axis; placements both at position 0,0,0 (`campaigns/fixtures/combat_basic/areas/start/placements.json:1`).
- Shell `#[func]` names (`crates/crpg-godot/src/shell.rs`): `open` (`:40`), `open_v4` (`:63`), `open_v4_rt` (`:86`), `controlled` (`:108`), `render_positions` (`:114`), `declare_move` (`:120`), `move_to` (`:126`), `halt` (`:132`), `poll` (`:138`), `view` (`:147`), `submit_option` (`:155`), `close` (`:161`), `last_error` (`:168`), `revision` (`:184`), `net_ids` (`:190`), `names` (`:196`), `sides` (`:202`), `placements` (`:208`), `health` (`:214`), `max_health` (`:220`), `health_known` (`:226`), `dead` (`:232`), `active` (`:238`), `round` (`:244`), `ended` (`:250`), `option_labels` (`:256`), `option_targets` (`:262`), `positions` (`:268`), `position_known` (`:274`), `moving` (`:280`), `move_receipts` (`:286`), `events` (`:296`), `receipts` (`:306`).

## Choosing an action today

- `CombatHud.gd` `_sync_options` (`apps/client/scripts/CombatHud.gd:118`) — clears `_options_box`, one `Button` per `view.option_labels()` entry, text = label, `disabled = false`, pressed connects `_on_option.bind(i)`.
- `CombatHud.gd` `_on_option` (`apps/client/scripts/CombatHud.gd:203`) — calls `main.choose(index)`; codes 0/4/5 silent, 30 shows notice, otherwise error with `main.session.last_error()` or `NOT_OPEN`.
- `Main.gd` `choose` (`apps/client/scripts/Main.gd:45`) — `if session == null: return 1` else `session.submit_option(seen, index)`.
- `Main.gd` `_process` (`apps/client/scripts/Main.gd:24`) — polls once per frame; on revision change calls `$Proxies.sync(next)` and `$Hud.sync(next)` (`apps/client/scripts/Main.gd:38`).
- `BridgeCore::submit_option` is invoked by shell `submit_option` (`crates/crpg-godot/src/shell.rs:155`).
- `LegalIntent` (`crates/crpg-server/src/host.rs:320`) — `{ actor: NetId, body: IntentBody }` in peer's replica ids.
- `Host::legal_intents` (`crates/crpg-server/src/host.rs:1102`) — returns `Vec<LegalIntent>` in sim enumeration order; empty without active encounter or when active head not controlled/disclosed.
- `Offered` (`crates/crpg-godot/src/core/client.rs:362`) — `{ actor: NetId, body: IntentBody }` carried per option.
- `option_labels` (`crates/crpg-godot/src/core/client.rs:661`) — maps `Offered` to labels/targets: `EndTurn` -> "End turn"/target 0; `DeclareAction { ability, target }` -> "<ability name> -> <target name>" and target id.
- Shell view accessors (`crates/crpg-godot/src/shell.rs:256`, `:262`) — `option_labels()` and `option_targets()`.

## Picking in 3D

- `SceneSync.gd` `_make_proxy` (`apps/client/scripts/SceneSync.gd:136`) — `Node3D` named `proxy_<net_id>` with child `MeshInstance3D` "Mesh" and `Label3D` "Label" at `y = 1.2`, billboard enabled; no collision shape.
- `SceneSync.gd` `sync` (`apps/client/scripts/SceneSync.gd:61`) — reconciles proxies against view rows (spawn/free/refresh); sets proxy.position by side/rank, mesh side colour, label text, dead tint; may only read `net_ids/names/sides/dead`.
- `SceneSync.gd` proxy map (`apps/client/scripts/SceneSync.gd:7`) — `_proxies: Dictionary` int NetId -> Node3D; `proxy_count()` counts children whose names begin with `proxy_`.
- `Walk.gd` `ground_point` (`apps/client/scripts/Walk.gd:129`) — projects ray from camera through screen point, intersects y=0 plane, returns Vector3 or null.
- `Walk.gd` `click_ground` (`apps/client/scripts/Walk.gd:93`) — selects controlled proxy within `PICK_RADIUS` (0.6) by horizontal distance; otherwise calls `session.move_to(selected, raw point)`.
- `Walk.gd` `draw_rows` (`apps/client/scripts/Walk.gd:141`) — writes each proxy's position from `render_positions()` outside `SceneSync`.
- `Walk.gd` `_unhandled_input` (`apps/client/scripts/Walk.gd:58`) — handles left/right mouse button presses, calls `ground_point`, increments `input_calls`.

## Feedback inputs

- `CombatHud.gd` `_event_text` (`apps/client/scripts/CombatHud.gd:179`) reads event keys: `kind`, `actor`, `ability`, `target`, `outcome`, `damage`, `round`, `entity`; renders `action_resolved`, `turn_started`, `died`, `encounter_ended`, and other kinds as kind text.
- `ViewEvent` entries (`crates/crpg-godot/src/core/client.rs:253`) define keys: `action_resolved` (`actor`, `target`, `ability`, `ability_id`, `outcome`, `damage`), `turn_started` (`actor`, `round`), `died` (`entity`), `encounter_ended` (`encounter`, `round`), `moved` (`entity`, `points`).
- Health fields (`crates/crpg-godot/src/core/client.rs:78`, `:88`) — `health()`, `health_known()`, `max_health()`, `dead()`; shell accessors at `crates/crpg-godot/src/shell.rs:214`, `:220`, `:226`, `:232`.
- `SessionView::revision` (`crates/crpg-godot/src/core/client.rs:53`); shell `revision()` (`crates/crpg-godot/src/shell.rs:184`).
- Proxy labels (`apps/client/scripts/SceneSync.gd:86`) — `Label3D` named "Label" at `y = 1.2`, billboard enabled, text set from `names[i]` in `sync` (`:64`); dead tint modifies label modulate (`:138`).
- `CombatHud.gd` `sync` (`apps/client/scripts/CombatHud.gd:48`) — turn line `Turn: <name>` from active id; health lines `<name>: ?` or `<name>: <health>/<max_health>` plus `(dead)`; log gate by revision (`:112`).
- Events/receipts arrays (`crates/crpg-godot/src/core/client.rs:123`, `:128`); shell `events()`/`receipts()` (`crates/crpg-godot/src/shell.rs:296`, `:306`).

## Tests to copy

- `hud.gd` (`apps/client/tests/hud.gd:1`) — runs `hud_cases.gd`, exits 0 on "PASS", 6 on anything else, 3 if cases fail to load; H20 real session runs in `smoke.gd`.
- `hud_cases.gd` (`apps/client/tests/hud_cases.gd`) — HUD cases H1-H19 (file exists; cite `apps/client/tests/hud_cases.gd:1`).
- `smoke.gd` (`apps/client/tests/smoke.gd:1`) — headless Main.tscn playthrough; runs scene-sync then HUD cases first; exits 0 on ended view with proxy count matching net ids, 5 on mismatch, 7 on HUD case failure, 3 on open/poll/choose failure, 4 after 120s.
- `scene_sync.gd` (`apps/client/tests/scene_sync.gd:1`) — runs scene-sync cases S1-S19 and checks S14 two frames later; exits 0 on "PASS", 6 otherwise, 7 on deferred-free failure, 3 on load failure.
- `walk.gd` (`apps/client/tests/walk.gd:1`) — WK1-WK10 walk scene; exits 0 pass, 2 class missing, 3 unexpected code, 4 timeout (20s), 6 value check failed.
- `movement.gd` (`apps/client/tests/movement.gd`) — exists (T151c) for MOVE variant; file present at `apps/client/tests/movement.gd:1`.
- Godot run commands (`crates/crpg-godot/AGENTS.md:102`) — `Godot4 --headless --path apps/client -s res://tests/<name>.gd`; CI uses same (`.github/workflows/ci.yml:299`, `.github/workflows/ci.yml:319`).
- `drive_ai` tests: `crates/crpg-server/tests/host_ai.rs` (H1-H11, lines `:555`-`:1217`) and `crates/crpg-server/tests/host_ai_contract.rs` (H1-H11 plus D* cases, lines `:543`-`:1537`).
- Dedicated loop `drive_ai` tests: `crates/crpg-server/tests/run_dedicated.rs` (`fa_3_drive_ai_passes_the_budget_through` at `:627`, `fa_7` at `:713`).

## Open follow-ups

- `tasks/BACKLOG.md:146` — "T050 | contract approved D77 ... | — | click-to-move in the Godot client: real-time v4 open, `controlled()`, `render_positions()`, `campaigns/demo/walk`, `Walk.tscn`".
- `tasks/BACKLOG.md:275` — "T048 | merged ... | `SceneSync.gd` reconcile + S1–S14 + R1 ...".
- `tasks/BACKLOG.md:280` — "T051 | merged ... | `CombatHud.gd` rewrite, `hud_cases.gd` H1–H19, `hud.gd`, `smoke.gd` H20 + exit 7 ...".
- `tasks/BACKLOG.md:539` — "13 integrated smoke | embedded: crpg-testkit desync D5; dedicated (Windows and Linux): crpg-server dedicated_smoke G1 ...".

## Agent log

- 2026-11-22 (UTC) · opencode-go/beam-free + T156 facts · Fact sheet for combat demo v1 gathered with path:line citations.
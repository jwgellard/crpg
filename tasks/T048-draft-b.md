# T048 — scene sync and proxy lifecycle (`crpg-godot`, `apps/client`): blind contract draft (b)

## Task / Crate(s) / Purpose

Sole crate: `crpg-godot` presentation side, implemented as `apps/client`
GDScript only. Plan row `docs/IMPLEMENTATION_PLAN.md:252` (`C → I`,
prerequisite T047). Spec §19.2 #12. ADR-0033 accepted; this draft changes no
ADR-0033 decision. T047 (on this branch) spawns one `proxy_<id>` node per
replica row from the first non-empty revision and never despawns
(`apps/client/scripts/SceneSync.gd:4`, `:8-14`; `docs/architecture/crpg-godot.md:181-187`).
T048 owns everything after that: late spawns, despawns, per-revision refresh
of proxy-held facts, and the `NetId`↔node lifecycle, so the scene matches the
replica after every delta, reconnect or bulk snapshot. Positions are T049;
camera/input T050; HUD T051 (cut-down) — T048 names only what it hands each.

## Exact contract

Status: **draft, undecided**. Prerequisites on this branch: T047 skeleton +
T047b shell + T047d client (spawn-once `SceneSync.gd`, `Main.gd` poll loop,
`smoke.gd` playthrough) and T046 replica (`crates/crpg-net/src/replica.rs`).

### Decision

- **D1. Where the diff lives: GDScript-only in `apps/client/scripts/SceneSync.gd`; no Rust, no shell, no new file.** Options: (A) extend `SceneSync.gd` with a diff over `view.net_ids()` (recommended); (B) a Godot-free core helper (e.g. `core/sync.rs`) returning spawn/despawn/update lists; (C) new `#[func]`s on `CrpgView`/`CrpgSession` exposing the diff. Consequences: (A) keeps ADR-0033 §12 `T048 … It adds no FFI member` (`docs/adr/0033-godot-bridge.md:517`) and `T048 and T051 should need no Rust` (`docs/adr/0033-godot-bridge.md:534`); it is the only option that changes no S-set file. (B) adds a tested-but-unneeded Rust layer the ADR already rejected as harder without benefit. (C) touches the GDExtension boundary (S-set) for logic GDScript can do. **Recommend (A).** `Main.gd` keeps its single call site `$Proxies.sync(next)` on revision change (`apps/client/scripts/Main.gd:42`); no `Main.gd` logic change except any rename this contract pins. `pub` items: none in Rust; GDScript `sync(view)`, `proxy_count()` stay, plus the pinned private helpers below.
- **D2. Diff inputs, outputs, ordering, idle behaviour** (see Behaviour B1–B3). Inputs are the already-exposed view getters `revision()`/`net_ids()` plus the parallel fact arrays (`crates/crpg-godot/src/shell.rs:107-158`; core `crates/crpg-godot/src/core/client.rs:49-116`). Output is three in-memory lists per `sync`: spawn (ascending `NetId`), despawn (ascending `NetId`), update (ascending `NetId`). No new revision ⇒ no work (keep T047's early return shape, `apps/client/scripts/SceneSync.gd:8-13`). **Recommend** exactly this; the alternative (re-diffing on every `_process`) wastes work and risks frame-order flapping.
- **D3. FFI: no change.** No new/changed `#[func]` or signal; error mapping unchanged (codes 1–6/20–25/30/50–55 stay in the core, `crates/crpg-godot/src/shell.rs:36-94`). GDScript uses each list as: spawn → `_spawn_proxy`, despawn → free, update → refresh held facts. **Recommend** no FFI change because the view already exposes every fact T048 needs (`net_ids`, `names`, `sides`, `placements`, `health`, `max_health`, `health_known`, `dead`, `active`, `round`, `ended`, `docs/adr/0033-godot-bridge.md:270-285`). This does not touch the GDExtension boundary (S-set): `shell.rs`, `crpg.gdextension` (`apps/client/crpg.gdextension:1-8`), and CI stay unchanged. If a needed fact is missing, stop and ask (open question Q5) rather than adding a member.
- **D4. Proxy lifecycle** (see B4–B7). Create with `Node3D.new()`, name `proxy_<id>`, add as child of `Proxies` (`apps/client/scripts/SceneSync.gd:35-58`); free with `queue_free()` (the deferred-free call already used in `apps/client/scripts/CombatHud.gd:156-159`); node holds a `MeshInstance3D` (capsule first side, box others, `apps/client/scripts/SceneSync.gd:40-49`) and a billboarded `Label3D` name (`apps/client/scripts/SceneSync.gd:52-56`); a reappearing `NetId` is a fresh spawn. **Recommend** this verbatim extension of T047's layout.
- **D5. Bulk snapshot and reconnect: the same diff replaces the scene** (see B8). No special path: free every node whose id is absent, spawn every unknown id. `close()`/failed `open` clears all proxies; a fence keeps the last scene and stops polling (`apps/client/scripts/Main.gd:33-37`).
- **D6. Tests: numbered below; none Godot-free-new, rest headless; CI `client` job unchanged** (see Tests). The core revision/diff inputs are already covered in Rust (`crates/crpg-godot/src/core/client/tests.rs`, e.g. revision-once and row-order cases cited in `tasks/T048-facts.md:219`); T048 adds no Rust so it adds no Rust tests.
- **D7. One session, one crate (`apps/client`), no split.** T048 is GDScript-only and small (see Routing). Splitting would cross the one-task-one-crate rule for no benefit.

### Interface (`apps/client/scripts/SceneSync.gd`; `Main.gd` call site unchanged)

```gdscript
# Kept (exact current shapes):
func sync(view) -> void            # SceneSync.gd:8; called from Main.gd:42 on revision change
func proxy_count() -> int          # SceneSync.gd:28; counts children named proxy_*
# New private helpers (exact names this contract pins):
func _spawn_proxy(net_id: int, display_name: String, side_index: int, within: int) -> void  # exists, SceneSync.gd:35
func _remove_proxy(node: Node) -> void      # single free site; uses queue_free
func _refresh_proxy(node: Node, display_name: String, side: String, dead: int, known: int, hp: int, max_hp: int) -> void
func _side_layout() -> Dictionary          # { side: { order: int, count: int } } rebuilt per sync; see B5
func clear_proxies() -> void               # frees every proxy_* child; called on close/failed open
```

No Rust signature changes: `CrpgSession.open/poll/view/submit_option/close/last_error` (`crates/crpg-godot/src/shell.rs:40-94`) and all 16 `CrpgView` getters (`crates/crpg-godot/src/shell.rs:107-208`) are unchanged. `Main.tscn` node names `Proxies`/`Hud` (`apps/client/scenes/Main.tscn:22-26`) and `project.godot` (`apps/client/project.godot:7-11`) are unchanged. `Main.gd` keeps `seen`/`view`/`open_code`/`stop_code`/`choose` (`apps/client/scripts/Main.gd:7-48`).

### Behaviour

1. **B1 — call discipline.** `Main.gd` calls `$Proxies.sync(next)` exactly when `next.revision() != seen`, then `$Hud.sync(next)` (`apps/client/scripts/Main.gd:38-43`). T048 does not add polling, signals, or a second call site. `Hud.sync` keeps its own per-revision event guard (`apps/client/scripts/CombatHud.gd:112-118`); `SceneSync` keys off the passed view, not the HUD.
2. **B2 — diff inputs and idle rule.** On `sync(view)`: return if `view == null`; read `ids = view.net_ids()` (`apps/client/scripts/SceneSync.gd:11`), plus `names/sides = view.names()/view.sides()` (`apps/client/scripts/SceneSync.gd:15-16`) and `dead/health_known/health/max_health` for refresh. If `ids.size() == 0`, do nothing and stay unspawned (keep `apps/client/scripts/SceneSync.gd:12-13` shape; the `_spawned` flag is replaced by "have we ever built a node", see B8). If `view.revision()` equals the last synced revision, return with no node work. Otherwise compute, in this order: (i) despawn = live `proxy_*` ids absent from `ids`; (ii) spawn = `ids` with no live node; (iii) update = intersection whose held facts differ. The replica behind the view lists rows ascending by `NetId` (`crates/crpg-net/src/replica.rs:148-151`; T046 contract `tasks/T046.md:271`), and the view preserves that order (`crates/crpg-godot/src/core/client.rs:396-401`); GDScript must not re-sort by name/side.
3. **B3 — deterministic order.** Free despawn ids ascending; spawn new ids ascending; refresh updates ascending. Side-layout assignment (`_side_layout`) is deterministic per sync (B5). No randomness, no frame-index dependence.
4. **B4 — create.** `_spawn_proxy` keeps T047's body: `Node3D.new()`, `proxy.name = "proxy_%d" % net_id` (`apps/client/scripts/SceneSync.gd:36-37`), position from `(side_index, within)` (`apps/client/scripts/SceneSync.gd:38`), capsule for side index 0 else box (`apps/client/scripts/SceneSync.gd:40-49`), billboarded `Label3D` with display name (`apps/client/scripts/SceneSync.gd:52-56`), `add_child(proxy)` (`apps/client/scripts/SceneSync.gd:58`). Node holds no gameplay state (spec mirror rule, `docs/CRPG_ENGINE_SPEC.md:310`); it holds mesh + label only.
5. **B5 — layout for late spawns.** Rebuild `_side_layout` each sync from the current `ids`/`sides` in row order: first-seen side gets order 0, next new side 1, etc. (the T047 `side_order` loop, `apps/client/scripts/SceneSync.gd:17-24`); `within` counts per side in row order. Existing nodes keep their positions unless their side's order changes due to a side vanishing; new nodes take the position the same loop yields. Alternative (freeze positions forever) leaves holes when a whole side leaves; alternative (sort sides alphabetically) renames T047's layout mid-demo. **Recommend** rebuild-per-sync: deterministic, matches T047 on the fixture (`names [Hero, Goblin]`, `load.gd:41`), self-heals after bulk replace.
6. **B6 — refresh.** For each update node: set label text to the row's name; set dead visual from `dead[i] != 0` (append `" (dead)"` to the HUD line is HUD's job, `apps/client/scripts/CombatHud.gd:98-99`; the proxy shows it as a grey modulate plus label suffix — exact visual pinned in implementation, no rule read); reflect `health_known == 0` by dimming rather than showing numbers (never present unknown as zero, ADR table `docs/adr/0033-godot-bridge.md:275-278`; core already maps unknown to `(0,0,known 0,dead 0)`, `docs/architecture/crpg-godot.md:105-108`). A `Died` event never sets `dead` by itself (crate invariant 7, `crates/crpg-godot/AGENTS.md:32`); only the `dead` column does. Never infer damage by diffing HP (spec `docs/CRPG_ENGINE_SPEC.md:880`).
7. **B7 — free and reappearance.** `_remove_proxy` detaches and `queue_free()`s the node (same call family as `apps/client/scripts/CombatHud.gd:156-159`). Despawned ids leave no node and no entry. If a freed `NetId` later reappears (re-summon, re-join, new encounter reusing ids), it is a fresh `_spawn_proxy` with current facts; never reuse, reparent, or resurrect the old node. `proxy_count()` keeps counting `proxy_*` children (`apps/client/scripts/SceneSync.gd:28-33`); during the deferred-free window it may transiently include a queued-for-free node — tests compare after an idle frame (see H-notes). **Unverified:** the exact deferred timing of `queue_free` (engine behaviour, not in repo; see list below).
8. **B8 — bulk snapshot and reconnect.** In this slice a `Transfer` entry fences as `UNEXPECTED_TRANSFER`/54 (`docs/adr/0033-godot-bridge.md:231-232`; core invariant 4, `crates/crpg-godot/AGENTS.md:16-18`), and `apply_snapshot` has no caller (`tasks/T046.md:328`); so "bulk" means any revision whose `net_ids` set differs wholesale (first non-empty view after `open`, post-fence reopen, future snapshot wiring). Handling is the same diff: free all stale, spawn all new, refresh the rest — no separate code path, no `Transfer` routing. `clear_proxies()` runs when `open` fails, after `close()`, and when `Main.gd` records `stop_code` (`apps/client/scripts/Main.gd:33-37` keeps scene frozen on fence; clearing happens on the subsequent `close`/re-`open`, not mid-fence). Reconnect (remote) does not exist in the slice; when it arrives, its first view is handled as a bulk replace by this same rule.

### Tests

Godot-free: **no new tests.** The diff inputs (ascending rows, revision bumps at most once, unknown-health mapping, `Died`-is-notice, `ended` latch) are already pinned in the core (`crates/crpg-godot/src/core/client.rs:443-480`; `docs/architecture/crpg-godot.md:96-108`). Adding Rust for GDScript node bookkeeping would violate D1. Valid control: `cargo test -p crpg-godot --locked -j 2` still passes unchanged.

Headless (`Godot4 --headless --path apps/client -s …`, same `--import`-first rule as `docs/architecture/crpg-godot.md:228-229`): synthetic-view script(s) driving `SceneSync.sync` with a stub view object exposing the same getters (`revision/net_ids/names/sides/dead/health_known/health/max_health`), so despawn/bulk cases do not depend on scripted combat producing them. Each case has its control:

- **H1 late spawn.** View v1 `{1,2}` synced, then v2 `{1,2,7}` ⇒ one `proxy_7` appears, `proxy_count 3`, old nodes untouched (same instance paths). *Control:* v2 `{1,2}` ⇒ no new node, same count.
- **H2 leave despawn.** v1 `{1,2,3}` then v2 `{1,3}` ⇒ `proxy_2` gone after an idle frame, count 2, `proxy_1/3` instances preserved. *Control:* v2 `{1,2,3}` with changed health only ⇒ count 3, no node freed.
- **H3 fact refresh.** Same ids, new names/`dead`/health ⇒ label text and dead visual update, no spawn/free (count unchanged, instance paths unchanged). *Control:* identical view, new revision, identical facts ⇒ no node work (count + paths unchanged).
- **H4 `NetId` reappears.** v1 `{1,2}`, v2 `{1}` (free 2), v3 `{1,2}` ⇒ `proxy_2` is a new instance with v3 facts, never the old node. *Control:* without the v2 gap, v3 with same facts ⇒ instance preserved.
- **H5 bulk replace.** v1 `{1,2}` then v2 `{9,10,11}` ⇒ stale both freed, three spawned ascending (`proxy_10` before `proxy_11` in child order), count 3. *Control:* v2 `{1,2}` ⇒ zero frees/spawns.
- **H6 reconnect/clear.** After H5 state, `clear_proxies()` ⇒ count 0; next non-empty sync respawns from scratch. *Control:* `clear_proxies()` on empty ⇒ 0, no error. Failed-`open` control keeps E1's (`CRPG_CAMPAIGN` missing dir ⇒ exit 3, `apps/client/tests/load.gd:5`).
- **H7 dead visual.** Row with `dead=1` ⇒ proxy shows the pinned dead visual; `Died`-event-only (column 0) ⇒ no dead visual. *Control:* `dead=0, health_known=0` ⇒ dimmed/unknown visual, never `0/0` presented as known.
- **H8 ordering.** Spawn batch `{5,3,9}` presented ascending `{3,5,9}` ⇒ children ordered `proxy_3, proxy_5, proxy_9`. *Control:* reverse insertion input still yields ascending order (proves no input-order dependence).

CI: the `client` job (`.github/workflows/ci.yml`, Windows only per `tasks/T048-facts.md:235-249`) is **unchanged** — no new job, no new required check, no S-set file touched. The new script(s) run as extra `-s` steps inside the existing job only if the implementer wires them there; the contract requires the scripts exist and pass locally, and **recommends** running them in the existing job without changing its shape. `smoke.gd`'s end-state assertion (`proxy_count == net_ids == 2`, `apps/client/tests/smoke.gd:49-57`) is kept for the fixture playthrough; if fixture combat ever despawns, the assertion moves to "count equals current `net_ids().size()`" — flagged in implementation, not changed here.

### Definition of done

- `git diff --stat` touches only `apps/client/scripts/SceneSync.gd`, headless test script(s) under `apps/client/tests/`, and this contract. No `.rs`, `.toml`, `.gdextension`, workflow, or `deny.toml` change.
- New headless scripts exit 0; each control above exits non-0 when its guard is inverted (implementer wires the inversion or documents the manual run).
- Existing gates still pass unchanged: `Godot4 --headless --path apps/client --import`, `-s res://tests/load.gd` (exit 0; missing-dir control exit 3), `-s res://tests/smoke.gd` (exit 0 on `ended()` with count == size).
- No `cargo` change to verify (no Rust touched); `git diff --check` clean.

### Out of scope

Positions/interpolation (T049: later adds `positions()`/`previous_positions()` to `CrpgView`, `docs/adr/0033-godot-bridge.md:519-521`); camera rig and picking/click-to-option (T050); HUD turn/health/options/log/end work beyond reading the same view (cut-down T051, `docs/adr/0033-godot-bridge.md:522-524`); signals (ADR has none, `docs/adr/0033-godot-bridge.md:256-258`); remote connect/resume, saves, exports, editor, real AI, grant narrowing/`Transfer` routing, real-time encounters, second human peer; any wire/protocol/golden/sim/core/net/server change; any new FFI member.

### What must not change

Every ADR-0033 decision (esp. §10 FFI shape, §11 codes, §12 one-`unsafe`, §14 client-only layout); T046 replica and T047/T047a surfaces and tests; every wire, golden, fixture, sim, core, rules, data, net or server byte or signature; `Cargo.toml`/`Cargo.lock`, `rust-toolchain.toml`, `crpg-contracts`; `deps.py` `ALLOWED`; determinism crate list; `engine` off by default; no `godot` edge elsewhere; `lib.rs` keeps `allow(unsafe_code)`+`warn(missing_docs)` (`crates/crpg-godot/src/lib.rs:1-2`), `core/mod.rs` keeps `forbid(unsafe_code)` (`crates/crpg-godot/src/core/mod.rs:1`). A needed change outside `apps/client` + this contract is a stop, not a fix.

### Routing and size budget

One session, one crate (`apps/client` GDScript), landing in T048's implementation. Size: `SceneSync.gd` about +80–140 lines (diff + refresh + clear); test script(s) about 150–250 lines; this contract 0 code. Boundary handed to T049: node names (`proxy_<id>`), child order ascending, and instance-identity rule are stable; T049 only moves nodes. To T050: clicking a proxy resolves via `option_targets()` (`docs/adr/0033-godot-bridge.md:283`), which T048 does not consume. To T051: HUD keeps sole ownership of round/active/health/options/log/end rendering (`apps/client/scripts/CombatHud.gd:63-147`); T048 hands it an in-step scene, nothing else.

## Open questions for the user

1. **Dead visual on the proxy: grey modulate + label suffix, or label only? Recommend:** grey modulate (e.g. `modulate = Color(0.45,0.45,0.45)`) plus keeping the name text unchanged (HUD already appends `" (dead)"`, `apps/client/scripts/CombatHud.gd:98-99`). Rationale: visible at a glance, no duplicated string logic in the scene. Alternative: label suffix only; rejected as double bookkeeping.
2. **Side layout on despawn: rebuild per sync (B5) or freeze positions of survivors? Recommend:** rebuild per sync. Rationale: deterministic, matches T047 on the fixture, self-heals bulk replace. Alternative freeze leaves gaps when a side leaves.
3. **Refresh scope: name/side/dead/known-handling only, or also mesh swap on side change? Recommend:** refresh facts + relabel; swap mesh only if a row's side index changed (cheap, keeps capsule/box meaning). Alternative never-swap lies about side after a grant change that never happens in-slice.
4. **Harness: synthetic stub-view script vs extended playthrough? Recommend:** synthetic stub (deterministic despawn/bulk without depending on combat producing them); keep `smoke.gd` as the end-to-end playthrough. Alternative playthrough-only cannot cover H2/H4/H5 on the fixed 2-combatant fixture (`apps/client/tests/smoke.gd:49-57`).
5. **If any T048 need forces an FFI addition, stop or add? Recommend:** stop and return to the user (ADR-0033 §12 says no FFI member; S-set review required). No such need is foreseen.

## Unverified list

`godot`-crate Rust API details: none relied on (no Rust change). GDScript/engine behaviours not checkable in the repo or `~/.cargo/registry` and therefore **unverified**: exact deferred timing of `queue_free()` (H-cases compare after an idle frame for this reason); whether a queued-for-free node still matches `get_children()` during that window; `is_instance_valid` semantics if the implementer uses it; default `Label3D`/`MeshInstance3D` property values beyond what `SceneSync.gd:35-58` sets.

## Agent log

- 2026-11-02 (UTC) · opencode-go/muse-spark-1.3-contributor + T048 draft · Blind contract draft (b): GDScript-only scene sync with diff order, lifecycle, bulk handling, headless test list and boundaries, changing no ADR-0033 decision or FFI.

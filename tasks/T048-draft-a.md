# T048 — scene sync and proxy lifecycle (`crpg-godot`, `apps/client`): blind contract draft (a)

## Task / Crate(s) / Purpose

Route: implement, doc-only draft. Plan row `docs/IMPLEMENTATION_PLAN.md:252`
(`C → I`, `crpg-godot`, prerequisite T047, spec §19.2 #12 `docs/CRPG_ENGINE_SPEC.md:1686`).
T047d spawns one node per row from the first non-empty view and never removes
or refreshes one (`apps/client/scripts/SceneSync.gd:8-26`, `:35-58`;
`docs/architecture/crpg-godot.md:181-187`). T048 makes the scene a function of
*every* view revision: spawn rows that appear, free rows that leave, refresh the
facts a proxy holds, and pin the `NetId`↔node lifecycle across a session
restart. Positions are T049, camera/input T050, HUD T051
(`docs/adr/0033-godot-bridge.md:519-524`). Contract only; nothing here is code.

## Exact contract

Status: **blind draft for adjudication**, not approved. ADR-0033 is accepted
(`docs/adr/0033-godot-bridge.md:1-4`). This draft changes no ADR decision: where
that ADR fixes a choice it is recorded as a rejected alternative, not reopened.

### Decision

**D1. The diff is GDScript, in `apps/client/scripts/SceneSync.gd`.** The file
already owns the `NetId`→node map and is attached to the `Proxies` node
(`apps/client/scenes/Main.tscn:22-23`). T048 changes that one script;
`Main.gd` needs no edit because it already calls `$Proxies.sync(next)` exactly
when the revision changes (`apps/client/scripts/Main.gd:40-43`).

| Option | Consequence | Verdict |
|---|---|---|
| **(a) GDScript in `SceneSync.gd`** | No FFI member; ADR-0033:517 and `:534` hold; the diff (ids → three lists) is ~30 lines and needs the tree anyway to spawn/free | **Recommended** |
| (b) Godot-free `core/scene.rs` plus a `#[func]` returning the lists | `cargo test -p crpg-godot` could test the pure part, but it needs a new FFI member and revision bookkeeping across the boundary, and contradicts ADR-0033:517 ("adds no FFI member"). An ADR amendment, not a T048 choice | Rejected |
| (c) Same pure diff in GDScript but in a new `SceneDiff.gd` | Testable without a tree; +1 file and a second place to state "no gameplay" | Rejected: the tree is needed for every non-trivial case anyway |

There is no visibility keyword in GDScript. The contract surface is
`sync(view) -> void` and `proxy_count() -> int` (kept because
`apps/client/tests/smoke.gd:50` calls it); every other member is `_`-prefixed
and private by convention. No `class_name`, no `@export`, no signal, no new
autoload.

**D2. One reconciliation per revision, over ids.** Inputs: `view.revision()`,
`view.net_ids()`, then `names()`, `sides()`, `dead()` indexed by position. No
replica call is possible or needed: `ReplicaWorld` is a private field of
`Client` (`crates/crpg-godot/src/core/client.rs:278-286`) and only
`SessionView` crosses to GDScript (`crates/crpg-godot/src/shell.rs:104-208`).
`net_ids()` is ascending (`crates/crpg-net/src/replica.rs:148-151`,
`crates/crpg-godot/src/core/client.rs:396-401`), every per-entity array is
parallel (`docs/adr/0033-godot-bridge.md:271-285`,
`docs/architecture/crpg-godot.md:105-108`).

| Option | Consequence | Verdict |
|---|---|---|
| **Reconcile ids against `_proxies`, recompute layout, refresh all rows** | A view is a complete projection, so it works for deltas, a bulk snapshot and a restart with one code path; O(n) per revision | **Recommended** |
| Apply only spawn/despawn and cache "last seen" values per proxy | Needs change tracking and still re-reads arrays; a stale cache is a new bug class | Rejected |
| Diff against a remembered id array instead of the node map | Two sources of truth; a `queue_free`d node or an exception desynchronises them | Rejected |

Outputs of a private, non-mutating `func _plan(ids) -> Dictionary`:
`{"despawn": Array[int], "spawn": Array[int], "refresh": Array[int]}` (the
update list), each ascending. `despawn` = keys of `_proxies` absent from `ids`, **sorted**
(`Array.sort()`); `spawn` = ids absent from `_proxies`, in given (ascending)
order; `refresh` = every id, ascending. `refresh` is not narrowed: any of
name/side/dead can change while the id set does not (a health-only frame bumps
the revision, `crates/crpg-godot/src/core/client/tests.rs:385-390`), and
setting four properties is cheaper than tracking which changed.

Revision handling: `_revision` starts at `-1`. `sync` returns immediately when
`view == null` or `view.revision() == _revision`. It clears everything first
when `revision == 0` or `revision < _revision`, then sets `_revision`. A
fresh node fed a non-zero revision spawns all rows, because `_proxies` is
empty. No new revision ⇒ no node is created, freed or touched.

**D3. No FFI change, no signal, no error mapping.** Every fact the diff reads
already exists with these exact signatures: `revision() -> i64`
(`shell.rs:108`), `net_ids() -> PackedInt64Array` (`:114`),
`names() -> PackedStringArray` (`:120`), `sides() -> PackedStringArray`
(`:126`), `dead() -> PackedByteArray` (`:156`). Nothing is added or changed.
The three lists never cross the GDExtension boundary: they are GDScript locals.
`sync` returns `void` and cannot produce a `BridgeCode`; the only codes the
client acts on stay `Main.gd:29-37`'s (0 clean, 30 notice, 50–55 stop). No
`#[signal]` exists today and none is added (`docs/adr/0033-godot-bridge.md:256-258`;
`crates/crpg-godot/src/shell.rs:36-209` has no `#[signal]`). **S-set: not touched.**
`shell.rs`, `Cargo.toml`, `crpg.gdextension`, the six `CrpgSession` functions
and the sixteen `CrpgView` getters (`shell.rs:39-208`) are unchanged, so the
`unsafe impl` review and the GDExtension surface review do not reopen. The
`client` CI job is S-set separately and is not touched (D9).

| Option | Consequence | Verdict |
|---|---|---|
| **No FFI member** | ADR-0033:517, :534 hold; presentation work in the presentation file | **Recommended** |
| A `CrpgView.changed_net_ids()` / `diff(previous: PackedInt64Array)` member | Puts presentation bookkeeping in Rust, adds a member and a signature to review for one O(n) pass | Rejected |
| `spawned`/`despawned` signals | ADR-0033 §10 rejects signals: `poll` then `view` is the explicit order; a signal races the view | Rejected |

**D4. Proxy lifecycle.**

| Fact | Value |
|---|---|
| Map | `_proxies: Dictionary`, key `int` `NetId`, value `Node3D`; the map, not `get_node()` by name, is the lookup |
| Name | `proxy_<id>` exactly (`"proxy_%d" % net_id`), the T047d form (`SceneSync.gd:37`); names can never collide while in the tree because ids are unique and a free removes the node before a spawn |
| Holds | `Node3D` + `MeshInstance3D` (capsule iff side index 0, else box) + billboarded `Label3D`; `set_meta("net_id", net_id)`; dead tint. No script, no health numbers, no gameplay state (spec `docs/CRPG_ENGINE_SPEC.md:310`) |
| Create | `_spawn_proxy(...) -> Node3D` keeps T047d's body (`SceneSync.gd:35-58`), gains the metadata, returns the node; `add_child` after naming/positioning |
| Free | `_free_proxy(id)`: `remove_child(proxy)` then `proxy.queue_free()`, then `_proxies.erase(id)` — the same idiom `CombatHud.gd` uses (`apps/client/scripts/CombatHud.gd:156-159`). `remove_child` first makes `proxy_count()`/`get_node_or_null("proxy_<id>")` correct in the same call, while `queue_free` still releases the object at frame end |
| NetId reappears | Impossible within a live replica: a removal retires the id (`crates/crpg-net/src/replica.rs:291-292`) and `high_water` survives a snapshot (`:240-241`, `:263-267`). A reused id therefore means a new replica (`BridgeCore::open`, `crates/crpg-godot/src/core/mod.rs:349-358`), whose view is revision 0 (`:358`); `sync` has already cleared, so the row is a fresh spawn, never a reused node |

`_clear_all` iterates a sorted copy of `_proxies.keys()` (erasing inside a
`Dictionary` loop is the classic GDScript skip bug), then empties
`_side_order`.

| Choice | Options and consequence | Verdict |
|---|---|---|
| Lookup | **`Dictionary` keyed by `NetId`**; vs `get_node("proxy_<id>")` every sync (string parsing, and Godot's automatic `@N` rename poisons it); vs parallel arrays (index shifts on despawn) | **Dictionary** |
| Free | **`remove_child` then `queue_free`**; vs `queue_free` alone (the node is still a child and still `proxy_count`ed this frame, so the scene and the count disagree); vs `free()` (frees immediately while a later T050 raycast or a held reference may still point at it) | **`remove_child` + `queue_free`** |
| Identity | **`proxy_<id>` name plus `net_id` metadata**; vs name only (T050 and tests must parse strings); vs a sequential name (`Proxy1`) which loses the only wire-stable key | **Name + metadata** |
| Reuse on return | **Always spawn a fresh node**; vs revive a freed one (impossible after `queue_free`, and it would carry stale name/tint) | **Fresh spawn** |

**D5. Bulk snapshot and reconnect = the same sweep.** A view is a whole
projection, never a delta, so "bulk" needs no separate handler: D2's `despawn`
sweep removes every stale proxy and `spawn` adds every missing row. This is
also the honest state of the slice: the bridge never applies a publication —
`client.rs` only drains and `apply_delta`s (`crates/crpg-godot/src/core/client.rs:321-330`),
`apply_snapshot` (`crates/crpg-net/src/replica.rs:220`) has no bridge caller,
and a `DeliveryKind::Transfer` fences as 54 (`crates/crpg-godot/src/core/authority.rs:62-64`;
ADR §8). Reconnect today means `close` + `open`: the old scene is replaced
because the view is revision 0 again (`crates/crpg-godot/src/core/mod.rs:161`,
`:358`; the core test asserts that reset, `crates/crpg-godot/src/core/tests.rs:1007`).
The view exposes no epoch getter (`shell.rs:107-208`), so revision 0 is the
only "new session" signal; that is sufficient exactly because id reuse implies
a new replica (D4).

| Option | Consequence | Verdict |
|---|---|---|
| **One reconcile for deltas, snapshots and restarts** | No path can leave a stale node; a future transfer task inherits a correct scene | **Recommended** |
| A snapshot-specific "clear and respawn everything" path | Visually similar, but it destroys nodes on a future in-session resync, which would drop T049 interpolation and T050 selection; and the view carries no "this was a snapshot" marker to branch on | Rejected |
| Assume snapshots never reach the slice | True today, and it is how stale proxies would appear the day `Transfer` is wired | Rejected |

**D6. Layout stays a placeholder and becomes stable per side.** Positions stay
`x = 3 × side index`, `z = 1.5 × within-side index`, `y = 0`
(`SceneSync.gd:38`; `docs/architecture/crpg-godot.md:181-187`), recomputed for
every row from the current view on every sync (so a despawn closes the gap).
`_side_order` is session state: a side appends when first seen in a view and
keeps its column until `_clear_all`, even if it has no row for a while (the one
mutation in `_layout` is that append); a side first seen after a clear starts a
new order. Within-side slots are the
ascending-id rank among current rows on that side. Only D6's inputs change; no
wire position is read, because none exists (ADR §14; T049 owns `positions()`).

| Option | Consequence | Verdict |
|---|---|---|
| **Stable side columns, recomputed within-side slots** | A side that leaves does not swap everyone's mesh and x; one extra member reset on session start | **Recommended** |
| Recompute side order from each view | Simpler, but losing the first side silently swaps capsule/box and x for the survivors | Rejected |
| Keep per-side slots forever, never reuse a slot | No shift on despawn, but z grows without bound across a long fight | Rejected |

**D7. What refresh does.** Per id, ascending: `Label3D.text = names[i]`;
mesh kind = capsule iff that row's side index is 0 else box, derived from the
same side index as the position so the two can never disagree (with a stable
`_side_order` it changes only after a clear); dead tint `modulate = Color(0.35,
0.35, 0.35)` when `dead()[i] != 0`, else `Color.WHITE`, on both the
`MeshInstance3D` and the `Label3D`. Positioning is D6. `dead()` is a disclosed fact, `0` where
unknown (`docs/adr/0033-godot-bridge.md:278`); the tint is not an inference
from health and `Died` notices stay out of it (crate invariant 7,
`crates/crpg-godot/AGENTS.md:32-33`). The fixture does produce it: the goblin
ends `dead = 1` (`crates/crpg-godot/src/core/tests.rs:1205`). Nothing else on
the proxy: no health, no round, no active highlight, no events (spec §9.2
rules 1–3, `docs/CRPG_ENGINE_SPEC.md:879-881`).

| Option for "show `dead()`" | Consequence | Verdict |
|---|---|---|
| **Grey `modulate` on mesh and label** | Visible, reversible, no new geometry, one property per child | **Recommended** |
| Hide the proxy (`visible = false`) | The body vanishes from the battlefield and a raycast/selection in T050 cannot find it | Rejected |
| A distinct dead mesh/pose | Art and an asset pipeline; the slice has placeholders only | Later task |
| Nothing (`dead` is HUD text only) | Fails ADR-0033:517's "show `dead()`"; the scene would not match the view | Rejected |

**D8. No gameplay, no rules, no clock.** The scene reads the view and sets
node properties. It never names `session`, submits, reads `events()`/receipts,
or derives a fact the view did not disclose. Proxies hold no authoritative
state; spec calls the alternative an *authority leak*
(`docs/CRPG_ENGINE_SPEC.md:310`).

**D9. Tests, engine gates and CI.** One new headless GDScript test and one new
Godot-free Rust test (T and R, below). The `client` job
(`.github/workflows/ci.yml:192`) is S-set and unchanged; BR-22
(`:313-320`) still passes because the fixture never despawns a row: two
participants (`campaigns/fixtures/combat_basic/encounters/first-blood.json:4-13`),
death is a health fact, and BR-22 asserts count == 2 at the end
(`apps/client/tests/smoke.gd:49-58`).

### Interface

`apps/client/scripts/SceneSync.gd` (all of it; T047d's `_spawned` and the
first-view early return at `:6`, `:9-14` are deleted):

```gdscript
extends Node3D
# T048: reconcile the proxy scene to every view revision. Presentation only.

var _revision: int = -1
var _proxies: Dictionary = {}        # int NetId -> Node3D
var _side_order: Array[String] = []  # stable for the session; reset on D5

func sync(view) -> void: ...                 # public: gate, clear/plan/apply
func proxy_count() -> int: ...               # public: unchanged meaning (smoke.gd:50)

func _plan(ids) -> Dictionary: ...            # private, non-mutating: {"despawn","spawn","refresh"}, ascending
func _clear_all() -> void: ...                # private: free every proxy, reset _side_order
func _free_proxy(net_id: int) -> void: ...    # private: remove_child then queue_free, erase
func _layout(ids, sides) -> Dictionary: ...    # private: {"positions": Array[Vector3], "side_indexes": Array[int]} parallel to ids
func _spawn_proxy(net_id: int, display_name: String, side_index: int, within: int) -> Node3D
func _refresh_proxy(proxy: Node3D, id: int, display_name: String, side_index: int, dead: bool) -> void
```

`tests/scene_sync.gd` is a `SceneTree` script with an inner `FakeView` class
providing exactly `revision()`, `net_ids()`, `names()`, `sides()`, `dead()`.

`crates/crpg-godot/src/core/client/tests.rs` gains one test; no library item
changes. No other file changes except the two docs in Definition of done.

### Behaviour

B1. **Gate.** `sync(null)` is a no-op. `sync(view)` with
`view.revision() == _revision` is a no-op. Otherwise `_revision` is updated
after the clear step. Calling `sync` twice with one view changes nothing.

B2. **Session reset.** `revision == 0` or `revision < _revision` frees every
proxy and resets `_side_order` before planning. `revision == 0` on a fresh node
is an empty no-op. A forward revision never clears.

B3. **Plan.** `_plan` returns the three ascending lists of D2 and mutates
nothing; a view with no rows returns `{"despawn": <current ids sorted>,
"spawn": [], "refresh": []}`.

B4. **Despawn.** Every planned despawn is applied before any spawn, in
ascending id order, with D4's `remove_child` + `queue_free`. After the call no
freed node is a child and `get_node_or_null("proxy_<id>")` is `null`.

B5. **Spawn.** Missing rows are created in the view's order, named
`proxy_<id>`, carrying the metadata and the T047d mesh/label.

B6. **Layout.** Every row gets D6's position on every sync; a despawn shifts
later rows on that side up; a surviving side never changes its column.

B7. **Refresh.** Every row gets D7's name, mesh kind and dead tint, every sync.

B8. **Empty view.** An empty `net_ids()` at any revision leaves `Proxies` with
no `proxy_` child and `proxy_count() == 0`.

B9. **Determinism.** The only iteration orders are the view's ascending ids and
`_plan`'s sorted despawn list; no `Dictionary` iteration order is observable.
After any legal sequence, `get_children()` proxy names are in ascending id
order (ids only increase within a replica; a reused id follows a clear).

B10. **No authority leak.** `SceneSync.gd` never names the session, an intent,
events, receipts, health or an option, and never calls anything on `view` other
than B1–B7's five getters.

B11. **Errors.** `sync` produces no `BridgeCode` and has no failure return. A
`view` violating the parallel-array contract is a caller bug, not defended
against: the view contract guarantees `net_ids().len()` entries
(`docs/architecture/crpg-godot.md:105-108`).

B12. **Bulk and restart.** A view that replaces every row is handled by B2–B7
with no special case; no stale proxy survives a clear or a sweep.

### Tests

`T` = `apps/client/tests/scene_sync.gd` (`Godot4 --headless`, exit 0 pass,
3 first failed check, 6 a queued free still alive, 2 preload failure). Each
case asserts exact values and runs its control in the same process. `R` =
`cargo test -p crpg-godot`.

| # | Case | Runs | Valid control |
|---|---|---|---|
| T1 | First view `[1,2]`: `_plan` = `despawn []`, `spawn [1,2]`, `refresh [1,2]`; two children, names `proxy_1`/`proxy_2`, positions `(0,0,0)`/`(0,0,1.5)`, metadata 1/2 | Godot | Before sync count 0; `sync(null)` leaves it 0 |
| T2 | Late row `[1,2,3]` (new side): `spawn [3]`; `proxy_3` at `x=6`; counts 2→3 | Godot | Nodes 1 and 2 are `is_same` before/after and keep their positions |
| T3 | Leave from `[1,2,3]` (all one side) to `[1,3]`: `despawn [2]`; count 2; `get_node_or_null("proxy_2") == null` in the same call; node 3 moves from `z=3.0` to `z=1.5` | Godot | Nodes 1 and 3 are not freed and `proxy_count()` equals `net_ids().size()` |
| T4 | All rows leave (`[]` at revision > 0): `despawn [1,3]` sorted, count 0 | Godot | The revision-0 empty view at start also gives count 0 and does not crash |
| T5 | Dead tint: `dead [1,0]` greys `proxy_1`'s mesh and label (`Color(0.35,0.35,0.35)`) and leaves `proxy_2` white | Godot | A later `dead [0,0]` restores both to `Color.WHITE` (not one-way) |
| T6 | Refresh with unchanged ids: `names[1]` changes and `dead[1]` flips; label text and tint change, ids/positions unchanged | Godot | `proxy_1`'s label and tint are untouched |
| T7 | Stable side column: `[1 Hero,2 Goblin]` then `[2]`; Goblin stays at `x=3`, still a box | Godot | A fresh `SceneSync` fed only `[2 Goblin]` puts it at `x=0` (session state, not luck) |
| T8 | Reused id after a clear: revision 0, then a new view with ids `[1]`, a different name and a first-seen side; `proxy_1` is a new node (different object id), capsule not box, new name | Godot | A forward revision with id 1 present does not recreate it and keeps its mesh |
| T9 | Idempotence: the same view twice leaves `_revision`, identities, positions and tints unchanged; a **new** `FakeView` object with the same revision but different names/dead also changes nothing | Godot | A different revision with identical ids and `dead` flipped still refreshes (the guard is the revision, not the ids or object identity) |
| T10 | Child order after churn (`[1,2,3]`→`[1,3]`→`[1,3,4]`): `get_children()` proxy names ascend by id | Godot | The same assertion on a view with one row is non-vacuous (exactly one name) |
| T11 | Source scan of the comment-stripped `res://scripts/SceneSync.gd` (the token method of `crates/crpg-godot/tests/source_rules.rs:27-45`): no equal token `session`, `submit_option`, `events`, `receipts`, `health_known`, `option_labels`; contains `queue_free` | Godot | The same predicate on `Main.gd` finds `session` and `submit_option`; a literal string without `queue_free` fails |
| T12 | Free is deferred: capture the despawned node, sync, assert `is_instance_valid` immediately, `await process_frame`, assert invalid | Godot | A surviving node is still valid after the frame; exit 6 if a queued free is still valid after two frames |
| R1 | A view can be empty above revision 0: `enter(1)` → rev 1 with one row; `leave(1)` → rev 2, `net_ids`/`names`/`sides`/`dead` empty, `active() == 0`, next idle step keeps the same `Rc` | Godot-free | The revision-1 assertion (one row, rev 1) proves the empty result is not the default view; `close`'s revision-0 reset is already pinned (`crates/crpg-godot/src/core/tests.rs:1007`) |

CI: no change. BR-22 (`ci.yml:313-320`) still proves spawn-from-first-view and
the final count; it cannot prove T3/T4/T12 because the fixture has no leave
(`first-blood.json:4-13`). T runs from the completion record; question 4 asks
whether a one-step S package should gate it.

| Where the cases run | Consequence | Verdict |
|---|---|---|
| **New `apps/client/tests/scene_sync.gd`** | Own exit codes, isolated from the playthrough, runs with the same `--import`/`-s` rule as E1; not gated until an S package adds a step | **Recommended** |
| Fold the assertions into `smoke.gd` | Gated by BR-22's existing step, but it mixes unit cases with a playthrough, and smoke's exit codes (0–5) already mean other things; a failure is ambiguous | Rejected |
| Port the fake view through Rust | Contradicts D1 and would need an FFI member to reach the scene; cannot test node freeing at all | Rejected |

### Definition of done

Windows local, from Git Bash (`Godot4` is windowed; PowerShell sees no exit
code, `crates/crpg-godot/AGENTS.md:113-115`):

```text
cargo fmt --all
cargo clippy -p crpg-godot --all-targets --locked -j 2 -- -D warnings
cargo test -p crpg-godot --locked -j 2
python tools/lint/deps.py
python tools/lint/determinism.py
python -m unittest discover -s tools/lint -p "test_*.py"
cargo build -p crpg-godot --features engine --locked -j 2
Godot4 --headless --path apps/client --import
Godot4 --headless --path apps/client -s res://tests/scene_sync.gd   # exit 0
Godot4 --headless --path apps/client -s res://tests/smoke.gd        # exit 0
git diff --check
```

Delete the untracked `*.uid` files `--import` writes before a scope check.
`--import` first: `-s` needs `.godot/`. `CARGO_TARGET_DIR` unset. Docs, signed:
`docs/architecture/crpg-godot.md` (Client section: reconcile, clear-on-zero,
free idiom, side stability) and `crates/crpg-godot/AGENTS.md` (one GDScript
trap: no authority in a proxy; no dict-order iteration). Record in the
completion record: the T run, the BR-22 run, and one by-hand
`Godot4 --path apps/client` observation that the last revision shows one grey
proxy and no other visual change.

### Out of scope

T049 `positions()`/`previous_positions()` and interpolation; T050 camera,
picking, click-to-move, selection, active highlight; T051 HUD, log, VFX/audio
hooks; health numbers or bars on a proxy; new FFI members, signals, classes or
autoloads; new or edited campaign fixtures; remote connect/resume; save, load,
export; the editor; any `crpg-core`/`crpg-net`/`crpg-sim`/`crpg-server` API or
behaviour; `ci.yml`, `deny.toml`, `rust-toolchain.toml`, `crpg-contracts`;
`Main.gd`, `CombatHud.gd`, `load.gd`, `smoke.gd` (unmodified).

### What must not change

Every ADR-0033 decision, in particular the 6 + 16 FFI members and "no signals"
(`docs/adr/0033-godot-bridge.md:251-285`), T048 adding no FFI member (`:517`)
and needing no Rust (`:534`). All `CrpgView` getter signatures
(`crates/crpg-godot/src/shell.rs:107-208`); `proxy_count()`'s meaning and the
`proxy_<id>` naming; T047d's layout formula (client-only, ADR §14);
`Main.gd`'s one `poll()` per `_process` and revision gate
(`apps/client/scripts/Main.gd:26-43`); the core's Godot-free / no-float /
no-`HashMap` invariants (`crates/crpg-godot/AGENTS.md:12-38`) — note the
GDScript layout's `3.0`/`1.5` floats are presentation and already exist
(`SceneSync.gd:38`), not core code; `determinism.py`'s crate list; the
`engine` feature staying off by default; `deps.py` `ALLOWED`.

### Routing and size budget

One session, one package. `crpg-godot` is the plan row's crate
(`docs/IMPLEMENTATION_PLAN.md:252`); `apps/client` is that crate's consumer and
artifact, not a second crate (`docs/adr/0033-godot-bridge.md:406-424`), so the
one-crate rule holds. No split: about 120–180 lines of GDScript (from 58),
about 200 lines of test GDScript, one ~25-line Rust test, and two signed doc
paragraphs. Landing order inside the single package: `SceneSync.gd`, then
`tests/scene_sync.gd` (red against the old script for T3/T4/T5), then R1, then
the docs. If the architect insists on two packages: T048-1 (script + tests +
R1), T048-2 (docs), same crate, no second crate or branch.

**Boundaries handed on.** T049: `_layout` is the only position source;
`_proxies` is the `NetId`→node map; sync stays revision-gated, so T049 adds a
per-frame interpolation step in `Main.gd`/a new method and replaces `_layout`
with `positions()`/`previous_positions()` (`docs/adr/0033-godot-bridge.md:519-521`).
T050: the node name `proxy_<id>` and `set_meta("net_id", net_id)` are stable for
the life of a row, so picking maps node→`NetId` without parsing names; T050
adds any `proxy_for`/selection accessor it needs, and decides whether a dead
proxy is pickable. T051: the proxy shows no numbers and consumes no events;
HUD text stays in `CombatHud.gd:83-101`, and the event/VFX layer reads
`events()` itself, once per revision.

## Open questions for the user

1. **Is "revision 0 means a new scene" the right restart signal?** It is the
   only one the view exposes (`shell.rs:107-208`), and it is sound while id
   reuse implies a new replica (D4). **Recommend:** accept for T048; add no
   getter. The future transfer/resume task must either reset the revision on a
   full resync or argue for an epoch fact, in its own contract.
2. **Stable side columns or recomputed order?** **Recommend:** stable (D6);
   the cost is one reset and it prevents a silent capsule/box swap when a side
   leaves.
3. **Is a grey `modulate` enough for "show `dead()`"?** **Recommend:** yes
   (D7); a distinct dead mesh is art, not the slice. It is observable in the
   demo because the fixture ends with `dead = 1`.
4. **Must the headless test be gated in CI?** `ci.yml` is S-set and the brief
   prefers not to touch it. **Recommend:** no change in T048; run T in the
   completion record, and if the user wants it gated, file a one-step S package
   (one `scene_sync.gd` invocation beside BR-22), since it is the only
   automated proof of despawn and deferred free. Do not fold it into
   `smoke.gd`.
5. **Should T048 add a spawning/leaving encounter to prove despawn
   end-to-end?** That is campaign data, and fixtures are outside every T048
   file. **Recommend:** no; the fake-view test is the proof, and the demo's
   two-row fixture is unchanged.
6. **Does T050 need a public `proxy_for(id)` now?** **Recommend:** no; the
   metadata plus the map are enough, and T050 adds an accessor when a caller
   exists (no dead surface).
7. **Should an active-row highlight be part of this?** **Recommend:** no;
   `active` is HUD text today (`CombatHud.gd:67`) and selection/highlight is
   T050. T048 refreshes exactly name, side/mesh and dead.

## Unverified

Checkable only in the engine or on a machine, not in this repository:

- GDScript untyped `Dictionary` with int keys, `keys()` and `Array.sort()` on
  the resulting ints as written.
- `CanvasItem`/`GeometryInstance3D.modulate` on `MeshInstance3D` and
  `Label3D`, and whether tinting both children is the intended look.
- `set_meta`/`get_meta` surviving `remove_child` and being readable before the
  node is freed.
- `queue_free()` on a node already removed from the tree still freeing at frame
  end, and `is_instance_valid` timing around it.
- `await process_frame` inside a `SceneTree`-extending script (T12's runner).
- Headless creation of `CapsuleMesh`/`BoxMesh`/`Label3D` and
  `get_children()` order under `--headless`.
- No `godot` **crate** API is newly relied on: the cited `PackedInt64Array::from`,
  `PackedByteArray::from`, `PackedStringArray` collect and `Gd::from_object`
  uses already exist at `shell.rs:72-115` and are unchanged.

## Agent log

- 2026-11-02 (UTC) · opencode-go/deepseek-v4.1-flash + T048 draft · Drafted the T048 contract from the checked source: a GDScript reconcile in `SceneSync.gd` with a pure `_plan`, `remove_child`+`queue_free`, stable side columns, revision-0 session reset, a headless fake-view suite and one Godot-free empty-view test, holding ADR-0033's no-FFI decision.

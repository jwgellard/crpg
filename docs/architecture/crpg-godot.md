# crpg-godot — architecture

The Godot presentation bridge (spec §9, ADR-0033). It is the only crate allowed
to depend on `godot` or to carry `unsafe`. The design splits in two: a
Godot-free core that builds and tests headless on Windows and Linux, and a thin
gdext shell that converts values and calls the core.

**State after T047b (2026-11-02):** the Godot-free core is complete and the
thin gdext shell (`src/shell.rs`, behind the off-by-default `engine` feature)
and the `apps/client` load test exist. T047-2 completed the core:
`core/errors.rs` (the closed code table and host-fault mapping), `core/link.rs`
(the bounded in-process two-ended link), `core/client.rs` (replica
application, immutable view revisions, rows, turn, options, receipts and
intent submission), `core/authority.rs` (the campaign walk, the embedded
`Host`, the NPC driver, the delivery hand-off and the presentation tables) and
`core/mod.rs` (`BridgeCore`, `open`/`poll`/`submit_option`/`close`, the panic
guard and the fatal fence) exist.

## Position

`crpg-godot` sits above `crpg-server`, `crpg-net`, `crpg-sim` and `crpg-core`.
It owns presentation only: no rule, wire format, host policy, campaign loader
or engine type may move into it. The authority runs in-process behind
`LocalLink`; the client cannot mutate it.

## Modules

- `core/mod.rs` — `#![forbid(unsafe_code)]`; private module declarations and
  public re-exports (`BridgeCode`, `SessionView`, `ViewEvent`, `ViewReceipt`,
  `ViewValue`, `BridgeCore`). Never write a bare `core::` path: the module
  shadows the `core` crate. `BridgeCore` holds one `State` (`Closed`,
  `Open(Box<Live>)`, `Fenced(code)`), the current `Rc<SessionView>` and the
  `last_error` text. Every `open`/`poll`/`submit_option`/`close` runs its
  `Live` work inside `catch_unwind(AssertUnwindSafe(..))`; `open_on` takes a
  caller link so tests can inspect both queues.
- `core/errors.rs` — `BridgeCode` (20 stable numbers from ADR-0033 §11,
  `name()` upper-case, `is_fatal()` exactly 50–55), `Fault` (code + detail)
  and `host_fault` (`Closed` → 50, every other `HostError` → 51).
- `core/link.rs` — `LocalLink` owns two independent FIFOs of whole frames, each
  capped at `cap` (default `LINK_FRAMES`, the host's per-peer egress depth,
  128). `authority_end()` sends down and receives up; `client_end()` sends up
  and receives down. A full queue returns `LinkFull` and changes nothing;
  `recv` is FIFO or `None`.
- `core/client.rs` — `Client` owns one `ReplicaWorld`, the current
  `Rc<SessionView>`, the offered options, the pending sequence and the next
  sequence. `step` drains the down queue, decodes (`52` on failure), applies
  (`53` on refusal), collects events/receipts and rebuilds the view at most
  once per call. `submit` validates the client-owned prefix of B14 and sends
  at most one encoded intent.
- `core/authority.rs` — `WalkFs`/`RealFs`/`collect_campaign` (a private copy
  of the CLI walker: no link following, sorted encoded-byte DFS, non-UTF-8
  names refused, classifier errors `21`, filesystem errors `20`);
  `check_binding` (`25`); `Presentation` (D8's name/side/ability resolution);
  `Authority` (the `Host`, both peer handles, the NPC side map, the pacer, the
  in-flight sequence and the recorded `(EntityId, placement)` order);
  `hand_off`, `npc_results`, `choose_npc` and `NpcPacer`; and the open helpers
  `SpecParts`, `grants`, `ruleset_abilities`, `first_encounter` and
  `presentation_tables`.

- `shell.rs` — private, compiled only with `--features engine`. One
  `unsafe impl ExtensionLibrary for CrpgExtension` (the crate's only
  hand-written `unsafe`), `CrpgSession` (a `RefCounted` holding a `BridgeCore`
  and the `Instant` taken when `open` returns 0) and `CrpgView` (a `RefCounted`
  wrapping one `Rc<SessionView>`; `no_init`). Each `CrpgSession` function
  converts its arguments and makes exactly one core call; `poll` passes
  `u64::try_from(elapsed().as_millis()).unwrap_or(u64::MAX)` as the single time
  source and `Host::tick` is never called. `view()` returns a new `CrpgView`
  per call. `CrpgView` getters return `i64`, `bool`, `PackedInt64Array`,
  `PackedByteArray`, `PackedStringArray` or `Array<VarDictionary>` (built from
  `entries()`, so a new field is added in one place).

## Flow in this half

1. `BridgeCore::open` checks the arguments (`3`) and the state (`2`), then
   walks the campaign (`20`/`21`), parses the compile-time engine version,
   loads and validates (`21`), selects the encounter (`22`/`23`), assembles
   the `EncounterSpec`, starts a `HistoryWorld`, checks the placement binding
   (`25`), picks or validates the player side (`24`), acknowledges the start
   journal, wraps the host, binds both peers with full disclosure (player as a
   snapshot peer, NPC as a plain peer), builds both presentation tables and
   creates the client. On success the view is empty revision 0 and
   `last_error` is cleared; nothing is retained on failure.
2. `BridgeCore::poll` runs the authority step, then the client step, then
   publishes the client's view. `0` is a clean poll, `30` means an upstream
   frame was refused or a failure-budget reply was dropped, and `50`–`55`
   fences.
3. The authority step drains the player's frames into `ingest`, drives the NPC
   (`legal_intents` → pacer → `choose_npc` → one encoded intent), pumps,
   folds the pump's NPC results, hands the player's delivery to the link,
   acknowledges the NPC's delivery unread, watermarks captures and finally
   reads the player's next options.
4. The client step decodes and applies every queued frame; a bad frame is
   fatal and is never skipped.
5. The view is rebuilt from `replica.entities()` and `turn()`: rows ascending
   by `NetId`, health-known flags, live active filter, host-ordered labels.
   The revision becomes `previous + 1` when this step collected any event or
   receipt, or when rows/active/round/ended/options differ. Otherwise the
   previous `Rc`, with its own events and receipts, is kept unchanged.
6. `submit_option` checks negative arguments (`3`), the fence (`c`), `Closed`
   (`1`), stale revision (`4`), index (`6`) and pending (`5`), encodes, then
   sends. A full link is `30`; only success advances `seq` and sets pending.

## View contract

Every slice has `net_ids().len()` entries. Missing presentation rows produce
`""`, never a panic. Unknown health is `(0, 0, known 0, dead 0)`; a `Died`
notice never sets `dead`. `encounter_ended` latches `ended`. `entries()` keys
start with `"kind"` for events and are `seq, applied, code` for receipts.

## Errors, fence and panic policy

Every non-zero result sets `last_error` to `NAME` or `NAME: detail`; only a
successful `open` clears it. A fatal fault (`50`–`55`) records its code and
text, replaces the state with `Fenced(code)` and shuts the host down and drops
the `Live` inside one ignored `catch_unwind`, so a panic during teardown
cannot change the fault. The view keeps its last revision. `close` is
idempotent, always ends `Closed` with the empty revision-0 view, keeps the
last error text, and turns a teardown panic into `PANICKED: …` without
fencing. A panic caught in `open`/`poll`/`submit_option` fences as `55` with
`PANICKED: <payload>`; `Drop for BridgeCore` runs `close`, so no panic
escapes. `#[cfg(test)] Probe` injects a panic at each boundary and at
`Live::drop`; it is compiled out of the library.

## FFI, pins and build (T047b)

- **Pins.** `godot = "=0.5.5"` with `features = ["api-4-7"]`, optional, behind
  `engine = ["dep:godot"]` (off by default, so `cargo test` and the headless
  server build never compile or link Godot). `api-4-7` reads the vendored
  `gdextension-api` 0.5.1 JSON (4.7.0 stable, single precision) and needs no
  Godot binary at build time. The engine used to load it is
  `4.7.2.stable.official.ed1daf0bf` (ADR-0001 pin). The lock gains exactly 13
  packages: `godot`, `godot-core`, `godot-ffi`, `godot-bindings`,
  `godot-codegen`, `godot-macros`, `godot-cell`, `gdextension-api`, `glam`,
  `heck`, `nanoserde`, `nanoserde-derive`, `venial`.
- **`[lib]`** is `crate-type = ["cdylib", "rlib"]`: the `cdylib` is the DLL the
  engine loads, the `rlib` is what tests and other crates link.
- **Build.** `cargo build -p crpg-godot --features engine --locked -j 2`
  writes `target/<profile>/crpg_godot.dll`. `apps/client/crpg.gdextension`
  points at it with `res://../../target/{debug,release}/crpg_godot.dll`,
  entry symbol `gdext_rust_init`, `compatibility_minimum = 4.7`,
  `reloadable = false`.
- **Load proof.** `apps/client/tests/load.gd` (E1) is a `SceneTree` script:
  `Godot4 --headless --path apps/client --import`, then
  `Godot4 --headless --path apps/client -s res://tests/load.gd` must exit 0.
  It exits 2 when `ClassDB.class_exists("CrpgSession")` is false (the `--import`
  exit code proves nothing about the DLL) and 3 on the first failed check;
  `CRPG_CAMPAIGN` pointing at a missing directory is its control (exit 3).
  The measured ladder step, build time and memory, the working `.gdextension`
  keys and the thread observation are in `tasks/T047b-completion.md`.

## The one `unsafe` (H-unsafe)

`#[gdextension]` requires `unsafe impl ExtensionLibrary` to confirm the opt-in
to godot-rust's safety model. The comment block above it in `shell.rs` states
why the opt-in is upheld: Godot loads the library once; only `CrpgSession` and
`CrpgView` are registered and they use gdext's checked conversions with no raw
pointer; `experimental-threads` is off, so gdext asserts main-thread access;
and this is the only hand-written `unsafe`. Macro expansion adds
`#[unsafe(no_mangle)] unsafe extern "C" fn gdext_rust_init` and similar, which
is why the crate root keeps `#![allow(unsafe_code)]`; `core/mod.rs` begins with
`#![forbid(unsafe_code)]`. `tests/source_rules.rs` K32 counts `unsafe` tokens in
`lib.rs` and `shell.rs` (comments stripped): exactly one, on that impl, under a
`SAFETY:` comment block. Any other `unsafe` needs an ADR-0033 amendment.

## Client (T047d)

`apps/client` is the terminal-free slice: `Godot4 --path apps/client` opens
`scenes/Main.tscn`, whose root `Main.gd` owns one `CrpgSession` and drives the
whole view. All of it is presentation; the options and ids come from the
bridge, and the scene decides no rule (ADR-0033 §14).

- **`scripts/Main.gd`** (`Node3D`) exposes `session`, `seen`, `view`, and
  `choose(i)` → `session.submit_option(seen, i)`. `_ready` instantiates
  `CrpgSession` and calls `open` (fixture, default side, seed 1); `_process`
  makes exactly one `poll()` and, when the revision changes, updates `seen` and
  calls `Proxies.sync(view)` and `Hud.sync(view)`. A 30 (intent refused) is a
  notice shown in the log and processing continues; any other non-zero code
  sets `stop_code`, shows `last_error()` and stops polling, since only `close`
  leaves a fence. `_exit_tree` calls `close()`. `open_code`/`stop_code` are
  read by the smoke test so a failed open is not mistaken for a slow turn.
- **`scripts/SceneSync.gd`** keeps no revision state: `sync(view)` reconciles
  the view's rows against the live `proxy_<id>` map whatever the revision, so
  `Main.gd` stays the only gate. `plan(ids)` is a pure diff into three disjoint
  ascending lists: despawn (live ids absent), spawn (ids with no proxy) and
  refresh (ids that keep theirs). `sync` frees every despawn with
  `remove_child` then `queue_free` (a same-frame re-spawn keeps its name),
  spawns the rest, then rewrites each row's position, mesh, name and dead tint
  and moves it to child `i`. A proxy holds exactly a `Mesh` `MeshInstance3D`
  and a billboarded `Label` `Label3D` at `y = 1.2`. The first side ever seen is
  a capsule and every other side a box, with `x = 3 × side index`,
  `z = 1.5 × rank within that side`, `y = 0`; the side order is
  first-appearance order and changes only through `clear()`, which frees
  everything for a session boundary. T048b lifts only each proxy's `Mesh`
  child by half its height (`mesh.position.y = height / 2`: capsule `0.6`, box
  `0.5`) while the proxy node keeps `y = 0`, so the mesh's lower extent rests
  on the ground and the label's local `y = 1.2` is unchanged. Each side's mesh
  also carries a `StandardMaterial3D` surface-0 override from a four-entry
  `SIDE_COLORS` palette — `Color(0.20, 0.60, 1.00)`, `(1.00, 0.35, 0.20)`,
  `(0.25, 0.85, 0.45)`, `(0.75, 0.45, 0.95)` (alpha 1) — indexed by the side
  index with a mod-4 wrap. A dead row stays in place, greyed: its
  `material_override` is a `StandardMaterial3D` with `albedo_color` 35 % and
  the label `modulate` is the same, and that override wins over the surface
  colour while dead and is cleared to reveal it again. `proxy_count()` counts
  the `proxy_` children. The wire has no position, so this layout is
  client-only.
- **`scripts/CombatHud.gd`** (`CanvasLayer`) renders the view and decides no
  rule. It builds a fixed, named tree in `_init`, not `_ready`, because `_ready`
  does not run inside `SceneTree._initialize` and the cases drive the HUD there:
  `Panel > Margin > Column`, whose children are `Round`, `Turn`, `Health`,
  `Options`, `Waiting`, `Log` (`RichTextLabel`), `Status` and `End`. `_ready`
  only sets `main = get_parent()`. `sync(view)` writes `Round <round + 1>`;
  `Turn: <name>` (name looked up in `net_ids()`, `-` when the id is no row);
  one health `Label` per row (`<name>: ?` when `health_known` is 0, otherwise
  `<h>/<max>`, plus ` (dead)`); one never-disabled `Button` per option label,
  whose press calls `main.choose(i)` (0, 4 and 5 are silent, 30 is a log
  notice, anything else `show_error`; `NOT_OPEN` when `main.session` is null);
  and `Waiting for <name>...` (`Waiting...` for `-`), visible exactly when the
  encounter has not ended and there are no options. The log is a private
  200-line buffer written to `Log.text` (never `append_text`, which parses
  BBCode). Each new revision appends its `events()` then its rejected
  `receipts()`, once: `<actor> | <ability> -> <target>: <verdict>, <n> damage`
  (verdicts as the CLI), `Turn: <name> (round <r + 1>)`, `<name> dies.`,
  `Encounter ended (round <r + 1>)`, other kinds unchanged, and
  `Rejected: <not allowed|no longer legal|refused by the server> (code <n>)`
  for codes 17, 18 and the rest. When `ended()`, `End` shows the three lines
  `Encounter ended`, `Rounds played: <round + 1>` and `Winner: <side|draw>`;
  the winner is the one side with a row whose `dead` is 0. `show_error` sets `Status` red and logs
  `ERROR: ...`, `show_notice` logs the text, `clear()` returns the HUD to its
  built state for a session boundary (nothing calls it yet). It reads only the
  14 view members listed in the T051 contract, never `placements()` or
  `option_targets()` (T050).
- **`tests/smoke.gd`** (E2) is a `SceneTree` script that instantiates
  `Main.tscn` under the root under the same `--import`/`-s` rule as E1. It runs
  the scene-sync cases first and exits 6 if one fails, then the HUD cases and
  exits 7 if one fails. Then it
  chooses option 0 whenever options exist for a revision it has not chosen,
  retries 4/5, exits 3 on another failure (including a failed open), 2 when the
  class is missing, 4 after 120 s, and 0 only when `ended()` and
  `$Proxies.proxy_count() == net_ids().size() == 2`; a mismatch is 5. At that
  ended view H20 (`_hud_end_problem`) also checks the real HUD: `End` visible
  with the survivor's side, the loser's `<name> dies.` line and a `Goblin |
  Strike -> Hero: ` line; a problem exits 7. The
  goblin's initiative means the first polls have no player options, so the
  script waits rather than treating that as a failure.
- **`tests/scene_sync_cases.gd`** holds S1–S13 and S15–S18 for the reconcile
  against an inner `FakeView`; `run(host)` returns `"PASS"` or `"S<n>: detail"`
  and records the freed and kept nodes S14 checks. S15–S18 prove T048b's lift
  and palette: S15 the mesh-only lift, S16 the palette and mod-4 wrap, S17 the
  dead override winning then restoring the side colour, S18 the unchanged label
  contract. **`tests/scene_sync.gd`** runs those
  cases and exits 6 on a failure, 7 when a freed proxy is still valid after
  two frames or a kept one is not, and 3 when a script fails to load.
- **`tests/hud_cases.gd`** holds H1-H19 against an inner `FakeView` with
  exactly the 14 getters the HUD may read, a `FakeSession` and a `FakeMain`;
  every case returns the token `"ok"` (or a detail) and `run(root)` returns
  `"PASS"` or `"H<n>: detail"`, so a crash (`""` from a `-> String` function)
  fails. **`tests/hud.gd`** runs them and exits 6 on a failure, 3 when the case
  file fails to load.
- **The walk scene (T050b).** `scenes/Walk.tscn` is the click-to-move demo
  (`Godot4 --path apps/client res://scenes/Walk.tscn`); `Main.tscn` stays the
  default scene and the combat demo. `scripts/Walk.gd` opens a real-time v4
  session over `campaigns/demo/walk`, polls it once per frame, feeds
  `SceneSync.gd` and `CombatHud.gd` unchanged, and draws every proxy itself
  after `sync` from `render_positions()` (`SceneSync.gd` may call only
  `view.net_ids/names/sides/dead`, S13, so a position never goes through it).
  The camera is fixed, aimed at the floor's centre. A left click on the ground
  within 0.6 m of a controlled proxy selects it, otherwise sends `move_to` for
  the selected actor to the click, raw (`round(m * 65536)`) on the plane
  `y = 0`; a right click sends `halt`. One press is one call. The client
  decides no rule: the host snaps, clamps and refuses, and a refusal shows as a
  HUD notice. No path preview (it needs a client mesh); pan and zoom are later.
  `tests/walk.gd` is the local engine gate for WK1-WK10 and exits 3 when the
  open fails.

## Protocol v4 sessions (T151)

T151 adds a protocol v4 session beside the frozen v2 one, so a Godot
client can see positions and `Moved`, move a combatant in turn-based
play (`DeclareMove`, lane 0) and send real-time moves (`MoveTo`/`Halt`,
lane 1) with their receipts, all through the embedded authority.
T151a is the engine-neutral v4 client half below; T151b wires the v4
session and `BridgeCore` onto T154's run loop; T151c is the
GDExtension surface. v2 keeps every byte and behaviour.

- **v2 is frozen.** `open` and the v2 `Client` are untouched; a v2 view
  fills the position columns with zeros and `move_receipts` empty. All
  v4 work goes in `core/client_v4.rs` and `core/authority/v4.rs`.
- **First-byte routing.** One FIFO per direction carries all three
  lane families: `0x82` frames go to the `ReassemblyPoolV4`'s transfer
  offer, `0xA1` to the lane-1 down decoder, anything else (an empty
  frame included) to the pool's delta offer. `link.rs` is unchanged.
- **The v4 client half (`core/client_v4.rs`).** `ClientV4` holds the
  pool (one receiver, `AwaitSnapshot`; `Live` under `live_for_test`),
  the `ReplicaWorldV4`, the lane-0 intent state (`pending`,
  `next_seq` from `SEQ_FIRST`) and the lane-1 `MoveOutbox`. It answers
  each `Published` transfer with a `Published` up reply, applies every
  pooled update in FIFO order, and rebuilds the view at most once per
  step exactly as the v2 client does, plus the columns below. A bad
  frame is fatal (`52`/`53`) and never skipped. `declare_move` takes
  the current options' actor and the destination; `move_to`/`halt`
  mint one outbox intent and restore the outbox when the link is full.
- **View columns.** `positions()` (three raw `Fx16_16` integers per
  row, `0, 0, 0` where undisclosed), `position_known()`, `moving()`
  (from the replica's disclosed position: set by an anchor, removed
  by a `None` position, replaced only by a strictly newer sample) and
  `move_receipts()` (one row per admitted lane-1 receipt, oldest
  first). `Moved` is an event of kind `moved`, only for a live row.
- **The v4 session on the run loop (`core/authority/v4.rs`,
  `core/mod.rs`).** T151b wires the client half onto T154's loop: a v4
  session's server runs on `crpg_server::run` over `LocalHost`, driven
  by `crpg_server::runner` on its own thread; the bridge never ticks,
  pumps or drives lane 1. `open_v4` binds the mesh from the encounter's
  area (`encounter_bounds`), binds the player with `bind_v4_peer` (the
  NPC stays a v2 `bind_peer`), wraps the host in `LocalHost`, installs
  today's NPC driver as the loop's AI hook (`NpcHook`, Reading 20), and
  spawns the runner. `poll` steps `ClientV4` over the `LocalClient` and
  reads the player's legal options between cycles (`read_options`
  through `RunnerHandle::submit`) without ever blocking the frame; the
  bridge holds the client half, the runner owns the host. `declare_move`,
  `move_to` and `halt` are the move entry points.
- **The shell surface (T151c).** `CrpgSession` gains `open_v4` (as `open`,
  including setting `opened_at`), `declare_move(revision, x, y, z)`,
  `move_to(actor, x, y, z)` and `halt(actor)`; `CrpgView` gains
  `positions`, `position_known`, `moving` and `move_receipts` (through
  `dictionary`). Each `#[func]` makes one core call and does no
  arithmetic on its arguments; coordinates are raw `Fx16_16` integers
  (metres x 65,536), `actor` is a `NetId`, and a v2 session answers a
  move call `3`. `apps/client/tests/movement.gd` (M1-M5, a local engine
  gate like `scene_sync.gd`, not in CI) builds the MOVE ruleset at test
  time in `user://t151_move/` because no checked-in ruleset may declare
  `movement`; it exits 0 pass, 2 class missing, 3 unexpected code, 4
  timeout (20 s), 5 MOVE edit failed, 6 value check failed. A view keeps
  its receipts until the next revision, so the script waits for a later
  revision before reading a second receipt.

### Real-time sessions, `controlled()` and `render_positions()` (T050a)

- **`open_v4_rt`.** `open_v4_rt(_with)` is `open_v4(_with)` with one
  difference: `open_prefix` starts the encounter with
  `HistoryWorld::start_encounter_in_mode(spec, mode)`, `RealTime` for
  `open_v4_rt*` and `TurnBased` for `open` and `open_v4*` (so no
  turn-based byte changes). In real time lane 1 walks combatants; nobody
  has combat options, because `legal_intents` is empty without an active
  turn. A ruleset with no `round_ticks` refuses the real-time start:
  `25` `START_FAILED: ModeUnsupported at combat/mode`, closed, no runner.
- **`controlled()`.** The player's control grant is fixed at open, so the
  list lives in `LiveV4`, not in the view: after `presentation_tables`
  each granted entity is mapped with `Host::replica_id` and the ids are
  stored ascending. It is empty in `Closed`, `Open` (v2) and `Fenced`.
  Dead or departed rows stay in it; the caller intersects with
  `net_ids()` and the host still decides every move.
- **`render_positions()`.** Three raw `Fx16_16` integers per row of
  `view()`, in `net_ids()` order. In T050a it is the view's positions;
  T049a replaces the v4 body with smoothed positions, with no shell or
  GDScript change.
- **The walk campaign.** `campaigns/demo/walk` is a copy of `combat_basic`
  with `movement` (2 m/s at 20 Hz), `round_ticks` 100, a 16 x 12 m floor
  and a second Hero placement (`...0Q`). It lives outside
  `campaigns/fixtures` because no fixture may declare `movement`. Rows
  are `n(1)` Hero (3, 0, 5), `n(2)` Hero (3, 0, 7), `n(3)` Goblin
  (12, 0, 6). The tests are `authority/v4/tests/rt.rs` (module
  `authority::v4::rt_tests`, under a `tests` directory so K31's core
  scan skips it); its `ManualClock`, `walk_root` and `settle` are
  `pub(crate)` for T049a.

## Decisions and traps

Governed by ADR-0033 and ADR-0012. The core holds no float,
`HashMap`/`HashSet`, clock or thread. Only `shell.rs` carries `unsafe`; this
crate root allows it for macro expansion, `core` forbids it.
Errors never carry control flow: tests assert codes and stable integers, not
diagnostic prose.

- **Acknowledge only what the link accepted.** `hand_off` stops at the first
  `LinkFull` and returns the last ordinal actually queued; acknowledging a
  refused frame makes the host drop it silently.
- **The NPC is a peer, not a second rule path.** It reads `legal_intents` and
  encodes the same `IntentFrame` a player would; a retained `QueueFull` is a
  hold, not a rejection.
- **Every poll watermarks captures.** Without it admission eventually stops
  with `backpressured: 1`.
- **`CARGO_TARGET_DIR` and `GDRUST_MAIN_EXTENSION` break E1.** The
  `.gdextension` path assumes `target/` at the workspace root, so a relocated
  target directory means the DLL is not found and `load.gd` exits 2.
  `GDRUST_MAIN_EXTENSION`, if set when `godot-macros` compiles, suppresses the
  `gdext_rust_init` entry point (`gdextension.rs:89-95`) and the class never
  loads. Unset both.
- **`Godot4` is a windowed program.** Under PowerShell it neither blocks nor
  reports an exit code; run `Godot4 ... -s` from Git Bash. `Godotconsole.exe`
  does not work on this machine.
- **Run `--import` once before any `-s` run** so `.godot/` exists; it also
  writes `*.uid` files, which are not committed.
- **The default opening actor is the goblin**, so with `player_side = ""` the
  first poll has no options; `load.gd` polls until the Hero's turn arrives.
- **The walker is a copy, not a reuse.** `crpg-cli` is bin-only; importing it
  would drag testkit and `same-file` into the DLL. Keep the two walkers'
  refusal shapes aligned by the K34 cases.

## Windows client build in CI

The `client` job (`.github/workflows/ci.yml`, Windows only, no Linux job
compiles the engine) runs in this order: scope, cache, clippy, build with
`--features engine`, the notice check, the Godot fetch (SHA-256 and SHA-512
checked before unpacking), G1 `--import`, G2 `load.gd`, G3 its control, G4
`smoke.gd`, G5 its control, then the artifact. G3 and G5 must exit 3, which
also proves the `_console.exe` passes exit codes through. The artifact
`crpg-client-slice-windows-x86_64` holds `target/debug/crpg_godot.dll`, the
tracked files of `apps/client` (with `THIRD_PARTY_NOTICES`), the 14 fixture
campaign files and both licence files, laid out as in the repository. BR-21
(no godot package in the default graph, plus an engine-feature control) is a
step of `check`. The pins and exact steps are in `tasks/T047c.md`.

Agent log:

- 2026-11-02 (UTC) · opencode-go/deepseek-v4.1-flash + T047-1 · Opened the crate document for the first real code: the Godot-free errors, link and client modules, their flow and view contract, with the authority and shell parts marked as later T047 packages.
- 2026-11-02 (UTC) · opencode-go/deepseek-v4.1-flash + T047-2 · Completed the core document with the authority half, `BridgeCore`, the campaign walk, the NPC driver, the exact open/poll flow, the fence and panic policy, and the traps found while implementing them; the shell remains T047b.
- 2026-11-02 (UTC) · claude-code/sonnet-5.5 + T047b · Documented the gdext shell, pins, build, the load proof and the one `unsafe` impl, plus the environment traps found while building it.
- 2026-11-02 (UTC) · opencode-go/deepseek-v4.1-flash + T047d · Added the Client section: the four GDScript files' contracts, the client-only proxy layout, the HUD text rules and E2's exit codes, so T048/T050 extend a stated baseline rather than re-deriving one.
- 2026-11-02 (UTC) · claude-code/sonnet-5.5 + T047c · Added the CI section: the `client` job's order, the artifact layout and where BR-21 sits, pointing at the contract for the pins.
- 2026-11-03 (UTC) · opencode-go/deepseek-v4.1-flash + T048 · Replaced the spawn-once SceneSync bullet with the revision-free reconcile (`plan`/`sync`/`clear`), documented the deferred-free idiom, the stable side order, the greyed-in-place dead visual and the two new test files, and gave `smoke.gd` its exit 6.
- 2026-11-07 (UTC) · opencode-go/deepseek-v4-pro + T048-fix · Documented that the scene-sync cases report success with an explicit `"PASS"` token, since a runtime error in a `-> String` GDScript function returns `""` rather than `null` and must not read as a pass.
- 2026-11-07 (UTC) · claude-code/sonnet-5.5 + T051 · Replaced the CombatHud bullet with the `_init`-built named tree, the HUD texts, the 200-line log buffer and gate and the end block, and added `hud_cases.gd`, `hud.gd` and smoke's exit 7 and H20.
- 2026-11-07 (UTC) · opencode-go/deepseek-v4-pro + T048b · Amended the SceneSync bullet for the mesh-only lift (D44b) and per-side surface palette, and extended the scene-sync cases bullet to S15–S18, so the client-side layout matches the approved T048b contract.
- 2026-11-19 (UTC) · opencode-go/muse-spark-1.3-contributor + T151a · Added the Protocol v4 sessions section for the v4 client half: the frozen v2, first-byte routing, `ClientV4`'s pool/replica/outbox shape, the position columns and their later parts, so T151b wires a documented baseline rather than re-deriving it.
- 2026-11-19 (UTC) · opencode-go/deepseek-v4-pro + T151b · Replaced the "Later parts" bullet with the v4-session-on-the-run-loop description: `encounter_bounds`, the NPC driver as `NpcHook` on the loop's AI stage, legal options read between cycles through `RunnerHandle::submit`, and the three move entry points, leaving only the shell (T151c) as later.
- 2026-11-21 (UTC) · claude-code/sonnet-5.5 + T151c · Replaced the "Later parts" bullet with the shell surface (`open_v4`, `declare_move`, `move_to`, `halt`, four view getters) and the `movement.gd` gate: how MOVE is built at test time, its exit codes and the receipt-revision trap.
- 2026-11-22 (UTC) · claude-code/sonnet-5.5 + T050a · Added the "Real-time sessions, `controlled()` and `render_positions()`" subsection: the real-time open, the session-held controlled list, the unsmoothed render hook and the walk campaign.
- 2026-11-22 (UTC) � claude-code/sonnet-5.5 + T050b � Replaced the "No input." bullet with the walk scene's input and drawing rules, now that picking and click-to-move exist.

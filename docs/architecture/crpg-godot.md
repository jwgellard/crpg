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
- **`scripts/SceneSync.gd`** spawns once from the first non-empty view (there
  is no despawn; that is T048). One `Node3D` named `proxy_<id>` per row holds a
  `MeshInstance3D` and a billboarded `Label3D` name. The first side seen is a
  capsule and every other side a box; the position is
  `x = 3 × side index`, `z = 1.5 × index within that side`, `y = 0`, where
  the side index is the order of first appearance. `proxy_count()` counts the
  `proxy_` children. The wire has no position, so this layout is client-only.
- **`scripts/CombatHud.gd`** (`CanvasLayer`) builds its panel in `_ready`:
  `Round <round + 1>`, `Active: <name>`, one health line per row (`?` when
  `health_known` is 0, `(dead)` from the `dead` column), one button per option
  that calls `choose(index)`, an event log built from each view's `events()`
  (once per revision), a status line for errors/notices, and an end label that
  appears when `ended()`. `show_error` also receives a failed open.
- **`tests/smoke.gd`** (E2) is a `SceneTree` script that instantiates
  `Main.tscn` under the root under the same `--import`/`-s` rule as E1. It
  chooses option 0 whenever options exist for a revision it has not chosen,
  retries 4/5, exits 3 on another failure (including a failed open), 2 when the
  class is missing, 4 after 120 s, and 0 only when `ended()` and
  `$Proxies.proxy_count() == net_ids().size() == 2`; a mismatch is 5. The
  goblin's initiative means the first polls have no player options, so the
  script waits rather than treating that as a failure.
- **No input.** Camera control and picking are T050 and deliberately absent;
  T048 owns respawn/despawn and T049 positions.

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

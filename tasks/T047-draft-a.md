# T047 — Godot bridge skeleton (`crpg-godot`): contract draft (a)

Status: **blind draft, not approved.** Inputs: accepted ADR-0033 (D41d),
D41a–D41c (`tasks/DECISIONS-D41.md:3-46`), T046
(`crates/crpg-net/src/replica.rs`) and T047a
(`crates/crpg-server/src/host.rs:661-732`). No ADR decision changes; no wire,
protocol, golden, sim, core, net or server change.

## Task / Crate(s) / Purpose

Sole crate: `crpg-godot` (plan `docs/IMPLEMENTATION_PLAN.md:249`), plus the
Godot project `apps/client` (not a Rust package; `docs/CRPG_ENGINE_SPEC.md:167`).
Open the crate per `docs/architecture/README.md:58`: gdext extension,
Godot-free core under a thin shell, one embedded authority, `LocalLink`, proxy
spawn from one bulk `view()` (spec §19.1 #22,
`docs/CRPG_ENGINE_SPEC.md:1666`), Windows client build (gate 12).

## Exact contract

### Decision

Each numbered item gives options and a recommendation where ADR-0033 leaves a
choice, else a rejected alternative. **Unverified** marks a fact no local source
settles; see U.

1. **Pins (item 1).** `godot = { version = "=0.5.5", optional = true,
   features = ["api-4-7"] }`, default features ON, exactly as ADR §13
   (`docs/adr/0033-godot-bridge.md:376-389`). Default features are
   `__codegen-full` (`godot-0.5.5/Cargo.toml:75`); `lazy-function-tables`
   exists (`godot-0.5.5/Cargo.toml:81`) and is off. Engine:
   `4.7.2.stable.official.ed1daf0bf` (verified by `Godot4 --version` on
   2026-11-02). API level `api-4-7`, not `api-custom`: only `api-custom`
   pulls `which` (`godot-bindings-0.5.5/Cargo.toml:59-65`, `:93`), and
   `api-4-7` needs no Godot binary and no `GODOT4_BIN`. `gdextension-api` is
   not a direct dependency: `godot-bindings` asks for
   `gdextension-api = "0.5.0"` (`godot-bindings-0.5.5/Cargo.toml:76`); the lock
   resolves the 0.5 line (ADR records 0.5.1). Exact resolved version
   **unverified** until the lock is written. Licence: D41c(a), eight packages
   only.
   - Options: (a) the ADR manifest, default features on; (b)
     `default-features = false, features = ["api-4-7", "lazy-function-tables"]`.
     (b) is ADR §13's own verification item
     (`docs/adr/0033-godot-bridge.md:550-551`).
   - Consequence: (a) is the crate's tested path and may make the first engine
     build heavy (`-j 2` is ADR §15's answer); (b) is smaller but is minimal
     codegen, where a needed generated class can be missing. **Recommend (a)
     for the first build**, applying (b) only if the measured engine build
     exceeds 20 minutes or the runner's memory; record the measurement and the
     switch. `engine` stays off by default; `experimental-threads` is never
     enabled.
   - Engine pin: the string above in the completion record and
     `docs/architecture/crpg-godot.md`; `.gdextension`
     `compatibility_minimum = "4.7"`. An engine upgrade is a separate reviewed
     pin change.

2. **Module layout (item 2).** ADR §1's six files plus the existing root
   (`crates/crpg-godot/src/lib.rs:1-4`); no seventh module. `lib.rs` keeps
   `#![allow(unsafe_code)]` and `#![warn(missing_docs)]` and declares
   `pub mod core;` plus `#[cfg(feature = "engine")] pub mod shell;`.
   `core/mod.rs` carries `#![forbid(unsafe_code)]` and declares the four
   submodules privately, re-exporting the public surface. The one
   `unsafe impl` lives at the end of `shell.rs` with a `SAFETY:` comment.
   - Trap: a crate-root module named `core` shadows the `core` crate for
     `use core::...` paths (`crates/crpg-net/src/replica.rs:8` uses
     `use core::fmt;`). The bridge's core files must use `std::` only. Not an
     ADR change, only a spelling rule.
   - Options: (a) the ADR names; (b) rename the module `bridge`. **Recommend
     (a)**: the ADR fixes the paths; (b) is documentation churn.

   | File | Owns | Public items |
   |---|---|---|
   | `lib.rs` | root attrs, module declarations | none |
   | `core/mod.rs` | `BridgeCore`, `SessionView`, `BridgeFault`, `BridgeIntent`, `PresentationTable`, `guard` | `BridgeCore`, `SessionView`, `ViewOption`, `ViewEvent`, `ViewReceipt`, `BridgeFault` |
   | `core/authority.rs` | `Host`, campaign bytes, peers, NPC driver, grants, presentation table, captures, delivery hand-off | `pub(crate)` only |
   | `core/client.rs` | `ReplicaWorld`, epoch, seq, pending intent, view rebuild, events/receipts | `pub(crate)` only; names no `crpg_server`/`crpg_sim` |
   | `core/link.rs` | `LocalLink`, two `Transport` ends, bounded FIFO | `pub(crate)` |
   | `core/errors.rs` | `BridgeCode` table, mappings, `BridgeFault` | `BridgeCode`, `BridgeFault` |
   | `core/tests.rs` | `#[cfg(test)]` seams and the fault tests | test-only |
   | `shell.rs` | gdext classes, conversions, `guard` calls, `ExtensionLibrary` | `CrpgSession`, `CrpgView`, `CrpgExtension` |

   `crpg_server` and `crpg_sim` appear only in `core/authority.rs` (and
   `core/mod.rs`, which drives it); `EntityId` and `crpg_core` never appear in
   `core/client.rs` or the view types. That is BR-10's compile-visible form.

3. **Construction (item 3).** `BridgeCore::open(campaign_dir: &Path,
   encounter_id: &str, player_side: &str, seed: u64)` runs in this order on
   the calling (Godot main) thread, all-or-nothing:
   1. Walk `campaign_dir` into `BTreeMap<SourcePath, Vec<u8>>`: private copy of
      `crates/crpg-cli/src/main.rs:1125-1262`; root must be a directory and
      not a symlink; names sorted by encoded bytes; directories recursed;
      only non-directories classified by `crpg_data::campaign_document_path`
      (`crates/crpg-data/src/validation.rs:261`); `Ok(None)` ignored; a
      document-path symlink is refused; regular files read. No
      `canonicalize`, no follow. The bridge's only filesystem I/O.
   2. `crpg_data::load_campaign(&files, &engine)` (`crates/crpg-data/src/loader.rs:251`),
      with the version parsed from `env!("CARGO_PKG_VERSION")` by inference
      (no `semver` edge; the CLI does the same at
      `crates/crpg-cli/src/main.rs:1546`), then `crpg_data::validate`
      (`crates/crpg-data/src/validation.rs:884`). Any finding stops, as
      `crates/crpg-cli/src/play.rs:61-63`.
   3. Resolve the encounter: empty means the first `Document::Encounter` in
      `loaded.documents` order (`crates/crpg-cli/src/play.rs:262-270`); a
      non-ULID string is `BAD_ARGUMENT` (3), a valid unknown one is 23.
      Assemble `EncounterSpec` as `crates/crpg-cli/src/combat_apply.rs:211-288`
      (encounter, ruleset, abilities, outcome tables, placements with area,
      creatures, effects). Missing references are 25 defence-in-depth, since
      validation already rejects them.
   4. `HistoryWorld::new(seed)` (`crates/crpg-sim/src/history.rs:425`) then
      `history.start_encounter(&spec)` (`history.rs:634`), never the bare
      `crpg_sim::start_encounter`, so the start events are journaled.
   5. Record `(EntityId, placement)` from `history.world().combatants()`
      (`crates/crpg-sim/src/world.rs:1087`) in ascending `EntityId`, and check
      count and order against `encounter.participants` as
      `combat_apply.rs:290-310`; mismatch is 25.
   6. `history.acknowledge(history.last_sequence())` (`history.rs:744`,
      `:810`) because `Host::from_history` refuses a non-empty journal
      (`crates/crpg-server/src/host.rs:566-575`).
   7. `Host::from_history(history, HostConfig { protocol: V2, incarnation: 1 },
      0)` (`host.rs:566`; `HostConfig` at `host.rs:113`). V2 is required for
      event ops; incarnation 1 because saves are cut.
   8. Bind the player with `bind_snapshot_peer` (`host.rs:776`) and the NPC with
      `bind_peer` (`host.rs:763`), both at `now_ms = 0`. Player control = every
      combatant on the chosen side; NPC control = every other combatant.
      Grants for both = all participants with every `EntityDisclosure` flag
      true, `abilities = ruleset.abilities`, `encounters = [encounter.id]`,
      `turn_round = turn_state = encounter_end = true` (`host.rs:145-185`).
      That is "full disclosure for this encounter". Empty `player_side` means
      the first combatant's side in ascending `EntityId`; a requested side
      matching none is 24.
   9. `Host::epoch(player)` (`host.rs:632`) → `SessionEpoch([u8;16])` →
      `ReplicaWorld::new` (`crates/crpg-net/src/replica.rs:132`), and
      `LocalLink` with 128 frames each way
      (`crates/crpg-net/src/protocol.rs:139`).
   10. Build the player's presentation table from step 5 plus
       `Host::replica_id(player, entity)` (`host.rs:715`) and the campaign's
       campaign/locale/placement/creature/faction documents; any `None` is 25.
       The NPC keeps its own table keyed by its ids. Copy the name rule from
       `crates/crpg-cli/src/demo.rs:131-185` and `:217-236`: name = creature
       name key, else placement name key, else placement ULID text; side =
       faction name key when the creature names a resolvable faction, else the
       creature name key, else the name. `combat_basic` yields `Hero`/`Goblin`.
   11. Return the core; the view is empty revision 0 until the first `poll`.
   - `LocalLink` (ADR §3): one `Rc<RefCell<VecDeque<Vec<u8>>>>` per direction,
     each capped at 128 frames; two `LinkEnd`s implement
     `crpg_net::transport::Transport` (`crates/crpg-net/src/transport.rs:14`)
     with `Error = LinkError::Full`. `Rc`/`RefCell` make `BridgeCore` `!Send`
     by construction; no thread, no `Send` at the FFI,
     `experimental-threads` off.
   - Time (ADR §5): the shell records `Instant::now()` in `open` and passes
     `elapsed().as_millis()` capped to `u64`; the core takes `now_ms` on every
     call. No FFI member takes a timestamp. `Host::tick` is never called.
   - Options: (a) `Rc<RefCell>` ends; (b) two ends borrowing one `LocalLink`.
     **Recommend (a)**: each half owns its end, no lifetime on `BridgeCore`,
     and `!Send` is automatic. (b) couples the halves' lifetimes for no gain.

4. **Per-frame flow (item 4).** `Main.gd::_process` calls `session.poll()`
   once; `poll` does exactly (ADR §6):
   1. Drain the client→authority link FIFO; for each frame
      `Host::ingest(player, bytes, now_ms)` (`host.rs:976`).
      `IngestDisposition::Refused` (`host.rs:269`) → clear the pending player
      intent, report `INTENT_REFUSED` (30); no reply exists.
   2. NPC step (B8): if no due and `Host::legal_intents(npc)` (`host.rs:671`)
      is non-empty, set `due = now_ms + 600`; if `now_ms >= due`, encode one
      intent and `Host::ingest(npc, ...)`, then clear `due`. At most one NPC
      ingest per poll.
   3. `Host::pump(now_ms)` (`host.rs:1026`). `backpressured == 1` is not an
      error (retried next poll). An NPC `Rejected` result is fatal 51.
      `dropped_replies > 0` → if a player intent is pending, clear it and
      report 30.
   4. Per peer `Host::take_delivery_entries(peer)` (`host.rs:1767`):
      player: `Delta` → `link.send`; on `LinkError::Full` stop hand-off;
      `Transfer` → fatal 54. Then
      `Host::acknowledge_delivery(player, last_ordinal_accepted)`
      (`host.rs:1793`) only if at least one entry was accepted. NPC:
      `take_delivery_entries`, then acknowledge its last ordinal unread.
   5. `Host::acknowledge_captures(Host::last_capture())`
      (`host.rs:2015`, `:1986`). Required: unacknowledged captures cap
      admission and stop play at 4,096 records.
   6. `Host::legal_intents(player)` → the offered list (T047a); an `Err` maps
      per B19.
   7. Client: `recv` the authority→client FIFO to empty;
      `codec_v2::decode_delta` (`crates/crpg-net/src/codec_v2.rs:595`) → any
      `Err` is fatal 52; `ReplicaWorld::apply_delta`
      (`crates/crpg-net/src/replica.rs:188`) → any `Err` is fatal 53. On
      success, walk `frame.ops` in order collecting events and receipts
      (B16). `server_tick` becomes the next `observed_tick`.
   8. If `(replica, offered options, events, receipts)` changed since the last
      committed view, `revision += 1` and rebuild `SessionView`. Otherwise
      nothing changes; the view's events/receipts are exactly those of the
      revision that produced it.
   - `submit_option` only writes the client→authority queue; it reaches the
     host on the next `poll`, so there is one queue and one ordering. The
     `NetId`→`EntityId` view (ADR §9): `EntityId` is named only in
     `core/authority.rs`; every id in `client.rs`, `SessionView` and the FFI is
     a `NetId` or its `i64` value. The host's live map stays the source of
     truth; the presentation table is a copy built once at open.
   - Options: (a) the ADR order; (b) pump before ingest / legal_intents before
     pump. **Recommend (a)**: it is the ADR and gives one-poll latency. (b)
     reports stale options and adds a second ordering.

5. **FFI surface (item 5).** Two `RefCounted` classes, no signals. Every member
   is main-thread only. `#[func]` returns use the Rust types; GDScript sees
   `int` for `i64`, `String` for `GString`, `bool`, objects, packed arrays and
   `Array`.

   | Class | Rust `#[func]` signature |
   |---|---|
   | `CrpgSession` | `open(&mut self, campaign_dir: GString, encounter_id: GString, player_side: GString, seed: i64) -> i64` |
   | | `poll(&mut self) -> i64` |
   | | `view(&mut self) -> Gd<CrpgView>` |
   | | `submit_option(&mut self, revision: i64, index: i64) -> i64` |
   | | `close(&mut self)` |
   | | `last_error(&self) -> GString` |
   | `CrpgView` | `revision(&self) -> i64` |
   | | `net_ids(&self) -> PackedInt64Array` |
   | | `names(&self) -> PackedStringArray` |
   | | `sides(&self) -> PackedStringArray` |
   | | `placements(&self) -> PackedStringArray` |
   | | `health(&self) -> PackedInt64Array` |
   | | `max_health(&self) -> PackedInt64Array` |
   | | `health_known(&self) -> PackedByteArray` |
   | | `dead(&self) -> PackedByteArray` |
   | | `active(&self) -> i64` |
   | | `round(&self) -> i64` |
   | | `ended(&self) -> bool` |
   | | `option_labels(&self) -> PackedStringArray` |
   | | `option_targets(&self) -> PackedInt64Array` |
   | | `events(&self) -> Array<VarDictionary>` |
   | | `receipts(&self) -> Array<VarDictionary>` |

   Classes: `#[derive(GodotClass)] #[class(base = RefCounted, init)] struct
   CrpgSession`; `#[derive(GodotClass)] #[class(base = RefCounted, no_init)]
   struct CrpgView`. `CrpgView` is built with `Gd::from_object(CrpgView {
   view })` (`godot-core-0.5.5/src/obj/gd.rs:170`); `CrpgSession` is
   GDScript-constructible, `CrpgView` is not. `entry_symbol` is the macro
   default `gdext_rust_init` (`godot-macros-0.5.5/src/gdextension.rs:52-54`).
   The single `unsafe` token is `#[gdextension] unsafe impl ExtensionLibrary
   for CrpgExtension {}` at the end of `shell.rs`.
   - `SessionView` is the Godot-free payload; the shell copies it into a cached
     `Gd<CrpgView>` only on a revision change and builds packed arrays with
     `PackedArray::from(&[T])`/`FromIterator`
     (`godot-core-0.5.5/src/builtin/collections/packed_array.rs:604,672`),
     `VarDictionary::new`/`set` (`.../dictionary.rs:127,321`) and
     `Array::new`/`push` (`.../array.rs:253,338`). `view()` before the first
     poll creates the empty revision-0 object.
   - Panic policy (B20): every `#[func]` does conversion, then exactly one
     `guard(&mut self.core, |core| ...)` call. `guard` lives in `core/mod.rs`
     (Godot-free, unit-tested), uses `catch_unwind(AssertUnwindSafe(...))`,
     and on a panic takes and drops the core (its `Drop` runs `Host::shutdown`)
     and returns `BridgeCode::Panicked` (55). `view()` returns a cloned
     `SessionView` from the same single call only when the revision changed, so
     the common path clones a `u64`. The shell does no indexing, `unwrap`,
     `expect` or arithmetic on GDScript values. No `panic = "abort"` profile.
   - `last_error` text: the mapped error's `Display` (or the fixed string for
     1–6/22–24/54); a panic payload as text, else `"panic"`. It is set by
     every non-zero return and cleared only by `open`. It is never control
     flow.

6. **Error mapping (item 5, ADR §11).** One closed `#[repr(i64)]` enum in
   `core/errors.rs`; discriminants are the table below. `is_fatal()` is true
   only for 50–55. On a fatal code the core shuts the host down, drops it, and
   every later `poll`/`submit_option` returns the same code until `close`;
   `view()` stays frozen at the last revision.

   | Code | Name | Rust source | Sticky |
   |---|---|---|---|
   | 0 | `OK` | success | — |
   | 1 | `NOT_OPEN` | no core (before `open`, after `close`) | — |
   | 2 | `ALREADY_OPEN` | `open` while a core exists | — |
   | 3 | `BAD_ARGUMENT` | negative/oversized `i64`, non-ULID `encounter_id` | — |
   | 4 | `STALE_OPTIONS` | `revision != view.revision` | — |
   | 5 | `INTENT_PENDING` | one intent outstanding | — |
   | 6 | `NO_SUCH_OPTION` | `index >= options.len()` | — |
   | 20 | `CAMPAIGN_IO` | any walk/`fs` failure, non-UTF-8 name, symlinked document, non-directory root | not opened |
   | 21 | `CAMPAIGN_INVALID` | `load_campaign` `Err`, any validation finding, `campaign_document_path` `Err` | not opened |
   | 22 | `NO_ENCOUNTER` | no encounter document | not opened |
   | 23 | `UNKNOWN_ENCOUNTER` | valid ULID absent | not opened |
   | 24 | `UNKNOWN_SIDE` | side matches no combatant | not opened |
   | 25 | `START_FAILED` | spec assembly, `start_encounter`, count/order check, `acknowledge`, `from_history`, either bind, table `None` | not opened |
   | 30 | `INTENT_REFUSED` | player `Refused`, full link on submit, dropped reply | open, non-fatal |
   | 50 | `HOST_CLOSED` | `HostError::Closed` | fatal |
   | 51 | `HOST_FAILED` | any other `HostError`; NPC `Rejected` or `Refused` | fatal |
   | 52 | `FRAME_UNDECODABLE` | `CodecError` on a delivered frame | fatal |
   | 53 | `REPLICA_REJECTED` | `ReplicaError` | fatal |
   | 54 | `UNEXPECTED_TRANSFER` | `DeliveryKind::Transfer` | fatal |
   | 55 | `PANICKED` | `guard` caught a panic | fatal |

   Precedence: a sticky fatal code is returned before 1/4/6/5; `NOT_OPEN` is
   returned before 4/6/5. `submit_option` checks 1 → 4 → 6 → 5, matching
   BR-11. A gameplay rejection is a receipt, not a poll code: `poll` returns 0.

7. **Proxy spawn (item 6).** One `Node3D` proxy per participant, spawned in
   `SceneSync.gd` from exactly one bulk `view()` call on the first non-empty
   revision (spec §19.1 #22): iterate `net_ids()` once, reading the parallel
   arrays at the same index. What a proxy holds: a `Node3D` named
   `proxy_<net_id>`, a `MeshInstance3D` (capsule for side index 0, box for
   side index 1, a fixed palette), a `Label3D` with `names()[i]`, and the
   `net_id`/`sides()[i]` strings. Position is client-only (the wire has no
   transform, `crates/crpg-net/src/protocol.rs:305`): `x = 3 * side_index`,
   `z = 1.5 * index_within_side`, `y = 0`. `SceneSync.proxy_count()` is a plain
   GDScript method for the smoke test. T047 spawns once and never despawns;
   later revisions only refresh the HUD. Deferred to T048: diffing `net_ids()`
   between revisions, freeing on leave, `dead()` visuals, per-entity updates,
   and any selection/picking. No Rust change.
   - Options: (a) one call plus parallel indexing (the ADR); (b) per entity or
     per field. **Recommend (a)**: ADR-0003's bulk pattern
     (`docs/adr/0003-gdextension-rendering-spike.md:48`); (b) is N× crossings.

8. **`apps/client` (item 7).** Files exactly as ADR §14:

   | File | Contents |
   |---|---|
   | `project.godot` | name, main scene `res://scenes/Main.tscn`, 1280×720, fixed camera |
   | `crpg.gdextension` | Windows x86_64 debug/release only; `entry_symbol="gdext_rust_init"`, `compatibility_minimum="4.7"`, `reloadable=false`; `windows.debug.x86_64="res://../../target/debug/crpg_godot.dll"` and the release twin |
   | `.gitignore` | `.godot/` and `export/` |
   | `scenes/Main.tscn` | `Main` (`Node3D`, script), `Camera3D`, ground, `Proxies`, `Hud` (`CanvasLayer`) |
   | `scripts/Main.gd` | open, one `poll()` per `_process`, option callback, `close()` in `_exit_tree` |
   | `scripts/SceneSync.gd` | `RefCounted`; `new(root)`, `sync(view)`, `proxy_count()` |
   | `scripts/CombatHud.gd` | `RefCounted`; `sync`, `show_error`, `on_option`; round, turn, health, buttons, log, end |
   | `tests/smoke.gd` | headless playthrough (BR-22) |

   Campaign path: `ProjectSettings.globalize_path("res://")` +
   `"../../campaigns/fixtures/combat_basic".simplify_path()`; `CRPG_CAMPAIGN`
   overrides it for the smoke's negative control. Scripts call
   `session.open(dir, "", "", 1)`, keep `seen = view.revision()`, and call
   `submit_option(seen, index)`; they never see bytes, seq or epoch. The demo
   at the end of T047: launch, two named proxies, health and round, turn
   marker, option buttons; click Strike repeatedly while it is the player's
   turn; the Rust NPC driver answers after 600 ms; the encounter ends, buttons
   clear and an end label appears. No click-to-move, camera control,
   selection, save, resume, art or export.
   - `.gdextension` path options: (a) the `res://../../target/...` form above;
     (b) a project-relative OS path. **Recommend (a)**; if Godot normalizes
     `res://` upward, use (b) and record the working form. Build with
     `CARGO_TARGET_DIR` unset (the smoke asserts the DLL exists).

9. **Windows build and T047c (item 8).** Exactly:
   ```
   cargo build -p crpg-godot --features engine -j 2          # first build writes Cargo.lock
   cargo build -p crpg-godot --features engine --locked -j 2
   Godot4 --headless --path apps/client --import
   Godot4 --path apps/client
   Godot4 --headless --path apps/client -s tests/smoke.gd
   ```
   The first build writes the lock (U10); every later command is `--locked`.
   T047 hands T047c: the two `cargo` commands with `--features engine`,
   `--headless --import`, the BR-22 command and its negative control, the
   artifact (`target/{debug,release}/crpg_godot.dll` plus `apps/client/**`),
   the eight licence package names, and the rule that no Linux job may enable
   `engine`. T047c owns C1–C7 (`docs/adr/0033-godot-bridge.md:432-452`) and
   the workflow/deny files; T047 touches none. Its Windows `client` job runs
   the same engine clippy (`--all-targets`), build, `--import` and BR-22
   commands (ADR C1).
   - Options: (a) T047c carries the eight `[[licenses.exceptions]]` with
     `name` + `allow = ["MPL-2.0"]`, no version; (b) add `version = "=0.5.5"`
     per package. **Recommend (a)**: exactly the eight names D41c approved;
     versions are already frozen in the lock, and a ninth package still fails
     by name. (b) needs a different version for `gdextension-api` and turns
     every godot bump into a deny edit.

### Interface

`crates/crpg-godot/Cargo.toml` (the whole change; `crates/crpg-godot/Cargo.toml:7`
is empty today):
```toml
[lib]
crate-type = ["cdylib", "rlib"]

[features]
engine = ["dep:godot"]

[dependencies]
crpg-core   = { path = "../crpg-core" }
crpg-data   = { path = "../crpg-data" }
crpg-net    = { path = "../crpg-net" }
crpg-server = { path = "../crpg-server" }
crpg-sim    = { path = "../crpg-sim" }
godot = { version = "=0.5.5", optional = true, features = ["api-4-7"] }
```
No `[dev-dependencies]`, no `build.rs`, no `semver`. The workspace root needs
no change: `members = ["crates/*"]` (`Cargo.toml:3`) already covers the crate,
and D27d puts the pin in `crpg-godot` only.

Godot-free core surface (`crates/crpg-godot/src/core/`):
```rust
pub struct BridgeCore { /* private */ }
impl BridgeCore {
    pub fn open(campaign_dir: &Path, encounter_id: &str, player_side: &str,
                seed: u64) -> Result<Self, BridgeFault>;
    pub fn poll(&mut self, now_ms: u64) -> Result<(), BridgeFault>;
    pub fn view(&self) -> &SessionView;
    pub fn submit_option(&mut self, revision: u64, index: usize)
        -> Result<(), BridgeFault>;
}

pub struct BridgeFault { pub code: BridgeCode, pub detail: String }
pub struct SessionView { /* private */ }   // + empty() pub(crate)
pub struct ViewOption { pub label: String, pub target: i64 }
pub enum ViewEvent { ActionResolved { .. }, TurnStarted { .. },
                     Died { entity: i64 }, EncounterEnded { .. } }
pub struct ViewReceipt { pub seq: i64, pub applied: bool, pub code: i64 }
```
`SessionView` getters return the slices listed in the FFI table (plus
`options()`, `events()`, `receipts()`). All ids/health/round are already
`i64`/`u8`; the core converts `u64`/`u32` with `i64::try_from(..).unwrap_or(i64::MAX)`
and no panic. `BridgeCode` is `#[repr(i64)]` with the 20 variants above,
`as_i64()`, `name()`, `is_fatal()`.

Protected files and why:

| File | Task | Why |
|---|---|---|
| `crates/crpg-godot/Cargo.toml` | T047 | `[lib]`, `engine`, five legal edges, optional exact `godot` |
| `Cargo.lock` | T047 | adds the eight MPL packages plus `glam`, `heck`, `nanoserde`, `venial` (and their transitives); `proc-macro2`/`quote` already present |
| `deny.toml` | T047c | eight `[[licenses.exceptions]]`, `name` + `allow = ["MPL-2.0"]`; the existing `[licenses].allow` list (`deny.toml:20-29`) is untouched |
| `Cargo.toml` (workspace) | none | `members = ["crates/*"]` already includes the crate; no pin belongs there |
| `.github/workflows/ci.yml` | T047c | C1–C5 |

No other protected file changes; `crates/crpg-godot/src/lib.rs:1-4` gains only
the module declarations.

### Behaviour

B1. **Open.** `open` follows Decision 3 exactly, all-or-nothing; a failed
open leaves no host and no core, and a later `open` may succeed. A second
`open` while a core exists is 2.

B2. **Open faults.** Walk/`fs` → 20; load/validate/classifier → 21; no
encounter → 22; unknown ULID → 23; unknown side → 24; assembly/start/ack/
wrap/bind/table → 25; non-ULID or negative int → 3. Codes 20–25 are never
sticky: the session stays closed.

B3. **Presentation.** For every `net_ids` row, `placements()` is the campaign
placement ULID text of the entity that `replica_id` mapped to that id, and
`names()`/`sides()` follow the copied rule. A row without a table entry is
defensive: empty strings, never a panic, never a raw id.

B4. **Link.** Each direction is a bounded FIFO of whole frames, capacity 128
(`crates/crpg-net/src/protocol.rs:139`). `send` on a full queue is
`LinkError::Full` and changes nothing. `recv` returns oldest-first; the client
never acknowledges anything.

B5. **Thread and time.** One thread; no `thread::spawn`, no `Send` at the FFI;
`now_ms` is monotonic from the shell's `Instant` and is the only time input.
`Host::tick` is never called; `_physics_process` is not used.

B6. **Poll order.** Steps 1–8 of Decision 4, in that order, exactly once per
call. `poll` always pumps, even with no ingress.

B7. **Player ingest.** A `Refused` clears the pending intent and reports 30;
`Staged` waits for the pump. `submit_option` only enqueues.

B8. **NPC driver.** On the trusted side only. If `legal_intents(npc)` is
non-empty and no due, `due = now_ms + 600`. At `now_ms >= due`, one intent:
first `DeclareAction` whose target is on another side (sides from the NPC
table; live targets only), else the first option. `seq` is the driver's own
counter from 1, `observed_tick = Host::server_tick()`, `epoch =
Host::epoch(npc)`, encoded with `codec_v2::encode_intent`
(`crates/crpg-net/src/codec_v2.rs:242`). Submitting clears `due`. A pump
`Rejected` or an ingest `Refused` for the NPC is fatal 51.

B9. **Pump.** `backpressured` 1 is retried next poll and is not an error;
`dropped_replies > 0` reports 30 and clears a pending player intent; an NPC
rejection is 51; any `HostError` maps per code 50/51.

B10. **Hand-off.** Player entries in ordinal order: `Delta` → link, `Transfer`
→ 54; on `Full`, ack exactly the last accepted ordinal and leave the rest for
a later poll. Never ack what the link refused. NPC entries are acked unread.
The remainder is delivered in order exactly once.

B11. **Captures.** After hand-off, `acknowledge_captures(last_capture())`;
`capture_acknowledged == last_capture` after every poll, including a poll whose
hand-off was blocked.

B12. **Options.** `legal_intents(player)` after the pump, in the sim's order,
translated to `BridgeIntent { actor: NetId, body: IntentBody }`; empty when it
is not the player's turn. Options are advisory; execution revalidates.

B13. **Client frames.** Decode first (52 on error), then apply (53). A bad
frame is never skipped. `server_tick` from the last accepted frame is the next
`observed_tick`.

B14. **Revision.** Increments exactly once when state, options, events or
receipts differ from the committed view; never otherwise. Before the first
changing poll: revision 0, empty arrays, `active 0`, `round 0`, `ended false`.

B15. **View contents.** All nine per-entity arrays have `net_ids.len()` entries
in ascending order. `health_known = 0` means unknown and health/max read 0; a
known 0 is `health_known = 1` with 0. `dead` comes from `HealthFact` only,
never `Died`. `u32::MAX` survives as 4294967295. `active` is the turn's active
`NetId` only if that row is live, else 0; `round` is the wire round unadjusted.

B16. **Events and receipts.** Collected from `frame.ops` in frame order after a
successful apply, oldest first, once, only in the revision that applied them.
Keys: `action_resolved` (`actor`, `target`, `ability` display name, `ability_id`
ULID text, `outcome` wire text, `damage`), `turn_started` (`actor`, `round`),
`died` (`entity`), `encounter_ended` (`encounter` ULID text, `round`). Receipts:
`seq`, `applied`, `code` (`RejectionCode::as_u8` as an int,
`crates/crpg-net/src/protocol.rs:437`, or 0). `ended` latches on
`EncounterEnded`.

B17. **Submit.** Precedence 1 → 4 → 6 → 5. Then build `IntentFrame { epoch,
lane: LANE_COMBAT, seq: next_seq, observed_tick: replica.server_tick(),
actor: option.actor, body: option.body }`, encode, enqueue, set pending, bump
`seq`. A full link clears pending and returns 30. A receipt for the pending
seq clears it, applied or rejected.

B18. **Close and drop.** `close` is idempotent: `Host::shutdown`, drop the
core, clear the view, leave `last_error`. `Drop` does the same if `close` was
not called. After close, `poll`/`submit_option` are 1 and `view` is empty
revision 0; `open` may follow.

B19. **Errors.** The table in Decision 6, with `last_error` set on every
non-zero return. Sticky 50–55 wins over 1/4/6/5. 30 is not fatal and the
session stays live.

B20. **Panic and unsafe.** Exactly one hand-written `unsafe` token in
`shell.rs`; `core/mod.rs` has `#![forbid(unsafe_code)]`; `lib.rs` keeps
`#![allow(unsafe_code)]`. A panic anywhere in the guarded call drops the core
and returns 55; later calls repeat 55; the core is gone.

### Tests

Default-feature core tests (both targets, `cargo test -p crpg-godot`) live in
`tests/bridge_core.rs` (public API) and `src/core/tests.rs` (`#[cfg(test)]`;
seams: `LocalLink::with_capacity`, `inject_panic`, `submit_body`, `authority()`,
`authority_hash()`). No new dependency; a scratch-campaign helper copies the
fixture to `std::env::temp_dir()` and removes it on drop. Every case has a
valid control.

| ID | B / BR | Case | Valid control |
|---|---|---|---|
| C1 | B1, B3 / BR-1, BR-17 | Open `combat_basic` (`""`,`""`): 0; first poll revision 1; `net_ids` `[1,2]` ascending, two distinct placements `…0E`/`…0F`, names `Hero`/`Goblin`, `active` in `{1,2}`, `health_known` all 1 | Explicit `…0N`/`Hero` gives the same; a swapped placement expectation fails |
| C2 | B2 / BR-2 | 20 nonexistent dir; 21 empty/truncated doc; 22 encounter dir removed; 23 valid absent ULID; 24 `Nobody`; each leaves no core and a later valid open gives 0 | The unmodified fixture before and after; each bad case differs in one input |
| C3 | B2 | 25 is the pure mapper: `HistoryError`/`HostError`/assembly fault → 25; 20–24 have distinct codes | A real `CAMPAIGN_INVALID` diagnostic is 21, not 25 |
| C4 | B4 / BR-7 | `with_capacity(1)`: first poll accepts one entry and acks it; submit + poll then blocks with 0 accepted and the ack unchanged; drain, poll: the remainder arrives in order, once, and `take_delivery_entries` ends empty | Capacity 128 delivers all in one poll |
| C5 | B4 / BR-8 | Fabricated `DeliveryEntry { kind: Transfer, .. }` → 54; later polls repeat 54 | A `Delta` entry at the same ordinal is delivered |
| C6 | B6 / BR-6 | Idle `poll`: `authority_hash`, `executions`, revision unchanged | One valid `EndTurn` changes all three |
| C7 | B8 / BR-4 | Chooser over fixed lists: first attack on another side; none → first option | All targets on the actor's side → first option |
| C8 | B8 / BR-5 | Script 0→1→600→601→602: no NPC ingest before 601, exactly one at 601, none at 602; one per poll | 600 with the due at 601 gives none; 599/600/601 pin the boundary |
| C9 | B13 / BR-12 | `submit_body(DeclareAction { target: NetId(99) })`; poll 0, receipt `{seq 1, applied false, code 17}`, session live | A valid offered option gives `{applied true, code 0}` |
| C10 | B13 / BR-13 | Push `[0xff]` on the authority→client link: poll 52, host closed, every later poll 52 | `decode_delta(&[0xff])` is `Err`; a `codec_v2::encode_delta` frame is `Ok` |
| C11 | B15 / BR-14 | Unit: a row with `health: None` gives `health_known 0`, health 0; every array equals `net_ids.len()` | `Some(HealthFact)` gives 1 and its values; a row added to `net_ids` breaks equality |
| C12 | B15 / BR-15 | `health = u32::MAX` → `health()` = 4294967295, `health_known` 1 | A known 0 is 0 with `health_known` 1; unknown is 0 with 0 |
| C13 | B16 / BR-16 | One 4-op frame (action, turn, died, ended): events and receipts once, in order, only in that revision; a no-change poll keeps them; the next change replaces them | A state-only frame bumps the revision with empty events |
| C14 | B17 / BR-11 | 1 before open; 4 stale; 6 out-of-range; 5 pending; in that precedence; nothing enqueued | The same call fresh and in range returns 0 and enqueues exactly one frame |
| C15 | B17 / BR-18 | `seed = -1`, `revision = -1`, `index = -1` → 3, no core call | `seed 0`, revision current, index 0 → 0 |
| C16 | B20 / BR-19 | `inject_panic`; guarded poll → 55; later polls 55; the core slot is `None` | No injection → 0 and the core stays |
| C17 | B19 | Every code's `name`/`is_fatal`/`as_i64`: only 50–55 fatal; 30 open | 0 is not fatal and is `OK` |
| C18 | B20 / BR-20 | Comment-stripped word count: `shell.rs` has exactly one `unsafe`; `core/mod.rs` has `forbid`; `lib.rs` has `allow` | The same counter over `"unsafe impl"` returns 1 and over `"// unsafe"` returns 0 |
| C19 | B1 / BR-10 | Source scan: `client.rs` has no `crpg_server`, `crpg_sim`, `crpg_core` or `EntityId` | `authority.rs` contains `crpg_server` and `EntityId` |
| C20 | B1 / BR-3 | Same seed and action script: `authority_hash` after the bridge equals an oracle `HistoryWorld` given the same actions and acknowledgements (unit test; NetId→EntityId by the ascending full-grant map) | One different action diverges |
| C21 | B4 | `with_capacity(2)`: third send is `Full`; recv returns FIFO | Capacity 3 accepts three; a recv on empty is `None` |
| C22 | B11 / BR-9 | Every poll: `capture_acknowledged == last_capture` | The same under a full link (step 5 is independent of hand-off) |
| C23 | B14 | Revision bumps once per changing poll; 0 before the first; no bump on idle | Two submits in one poll still bump once |
| C24 | B18 | `close` twice is a no-op; `poll`/`submit_option` → 1; `view` revision 0; reopen gives 0 | A second `open` before `close` is 2 |
| C25 | B3 | Unit: `classify_entry` with a document path and `Symlink` → 20; `assets/x.png` and `Symlink` → ignored | `File` with the same document path reads |
| C26 | B5 | Source scan: `src/core/` has no `Instant`, `std::time`, `thread`, `f32`, `f64`, `HashMap`, `HashSet` | `shell.rs` has `Instant`; the counter over a probe string proves the scan runs |

Engine tests (Windows only, not `cargo test`):
- E1: `Godot4 --headless --path apps/client --import` exits 0 and the extension
  loads (first `api-4-7` load on 4.7.2). Control: a renamed DLL fails the
  import.
- E2 (BR-22): `Godot4 --headless --path apps/client -s tests/smoke.gd` exits
  0; `proxy_count() == 2`; `ended()` true; option 0 is `Strike -> Goblin` at
  the first turn. Control: `CRPG_CAMPAIGN` nonexistent → `open` 20, exit 3.
- E3: the engine-gated clippy/build commands in the DoD compile the shell.
- Left to T047c: BR-21 (no `godot` in the default graph on both targets), the
  C5 deny negative control, the Windows `client` job, the artifact upload and
  the required-check change.

### Definition of done

On the implementation branch (none run now):
```
cargo fmt --all
cargo clippy -p crpg-godot --all-targets --locked -- -D warnings
cargo test -p crpg-godot --locked
cargo clippy -p crpg-godot --features engine --all-targets --locked -- -D warnings
cargo build -p crpg-godot --features engine --locked
Godot4 --headless --path apps/client --import
Godot4 --headless --path apps/client -s tests/smoke.gd
python tools/lint/deps.py
python tools/lint/determinism.py
python -m unittest discover -s tools/lint -p "test_*.py"
cargo deny check
git diff --check
```
`cargo deny check` is green only with T047c's eight exceptions in the tree
(U1). Docs, each signed: `docs/architecture/crpg-godot.md` (layers, FFI table,
flow, error table, pins, traps), `crates/crpg-godot/AGENTS.md` (invariants and
gates), and `docs/architecture/README.md:58` flipped to the new doc. The
completion record names the engine string, the resolved `gdextension-api`
version, the measured engine build time and peak memory, the working
`.gdextension` keys, and the licence audit of `glam`, `heck`, `nanoserde`,
`venial` (expected permissive; **unverified** until checked).

### Out of scope

T048 proxy lifecycle; T049 interpolation and `positions()`; T050 camera,
click-to-move and selection; T051's full HUD/log/VFX; T047c's workflow and
`deny.toml`; remote connect, resume, save/load, export, `Document`, `Assets`,
`Preview`, `Diagnostics`, the editor, real AI, grant narrowing, snapshot
transfer, real-time encounters, more than one human peer, and hoisting the CLI
glue.

### What must not change

Every ADR-0033 decision; T046's replica surface and tests; T047a's host
surface and tests; every wire, protocol, golden, fixture, sim, core, rules,
net and server byte or signature; the workspace `Cargo.toml`; `deny.toml`,
`.github/workflows/` and the rulesets in T047; `determinism.py` and its crate
list (`tools/lint/determinism.py:57`); `tools/lint/deps.py`'s `ALLOWED` table
(`tools/lint/deps.py:71`). `engine` stays off by default; no other crate gains
a `godot` edge; no thread, `Send`, float or `HashMap`/`HashSet` in
`crpg-godot`; `lib.rs` keeps `allow(unsafe_code)` and `core/mod.rs` keeps
`forbid`; `shell.rs` keeps exactly one `unsafe` token. A needed change outside
`crpg-godot`, `apps/client` and its two docs is a stop, not a fix.

### Routing and size budget

`crpg-godot` is in the plan §1.4 S-set (`docs/IMPLEMENTATION_PLAN.md:95-98`)
and H-unsafe applies to the one line. One crate; `apps/client` is not a Rust
package. Sizes: `src/core/*` 700–950 lines with docs; `src/shell.rs` 250–350;
`tests/bridge_core.rs` 600–800; `src/core/tests.rs` 400–550;
`apps/client` ~250 GDScript lines plus two scenes; architecture doc ~80;
`AGENTS.md` ~40; manifest ~15; `Cargo.lock` ~30 lines. **Split, in landing
order, if more than one session:**
1. **S1 (one session, `crpg-godot`).** Manifest gains the five edges only (no
   `godot`, no `[lib]`, no feature); `core/errors.rs`, `core/link.rs`,
   `core/authority.rs`, `core/client.rs`, `core/mod.rs`, `core/tests.rs`,
   `tests/bridge_core.rs`, and the architecture/AGENTS first versions. All
   default-feature tests and C1–C26 except the engine ones. `Cargo.lock` and
   `deny.toml` unchanged, so `deny` stays green.
2. **S2 (one session, `crpg-godot` + `apps/client`).** `[lib]`, `engine`,
   `godot`, `Cargo.lock`, `shell.rs`, `apps/client` (all files),
   E1–E3, the FFI/pins sections of both docs, and the completion record.
   The H-unsafe review covers this session's one line.
   T047c is not a session here: it is the separate S package in the same batch,
   after S2's tree exists.

### Open questions for the user

U1. **`deny.toml` timing.** ADR §15 makes T047c the owner, yet T047's
`cargo deny check` and CI's `deny` job run on its PR. If cargo-deny resolves
the optional graph (C5's unproved point), T047 alone fails. **Recommendation:**
the batch is reviewed and merged as one; T047's DoD runs `cargo deny check`
with T047c's eight exceptions applied in the working tree, and the completion
record says so. If the user wants each PR green alone, T047c's `deny.toml`
hunk must be the batch's first commit; that is an S-set ordering decision.

U2. **Godot feature set.** The ADR leaves `default-features = false` /
`lazy-function-tables` to the first build. **Recommendation:** the ADR
manifest (defaults on) first; switch only on a measured failure, and record
the measurement.

U3. **NPC ingest refusal.** The ADR names only a `Rejected` pump result as
fatal. **Recommendation:** an NPC `IngestDisposition::Refused` is fatal 51: the
600 ms pacing leaves budget room, so a refusal is a fault; idling the NPC would
stall the fight.

U4. **Dropped reply with no player intent pending.** The ADR says
`dropped_replies > 0` reports 30. **Recommendation:** report 30 and clear only
when a player intent is pending; a dropped NPC receipt must not consume a
player's choice.

U5. **NPC due after a blocked pump.** A staged NPC intent whose pump is
`backpressured` stays staged while its due is cleared. **Recommendation:** keep
it cleared; the staged frame is the retry, and resubmitting would duplicate.

U6. **`view()` identity.** The ADR is silent. **Recommendation:** one cached
`Gd<CrpgView>` refreshed only on a revision change; no per-frame allocation.

U7. **25 end-to-end coverage.** A valid campaign either starts or fails as
20–24; `start_encounter` panics on empty participants because data read-time
checks reject them (`crates/crpg-sim/src/combat.rs:1443-1445`).
**Recommendation:** test 25 at the pure mapping seam (C3) and record that no
public input constructs it; no test-only fault path in production code.

U8. **Name/side fallback.** The ADR says "a private copy of `demo.rs:217`".
**Recommendation:** copy it byte-for-byte, including the faction-else-creature
side rule; `combat_basic` then shows `Hero`/`Goblin`, which E2 asserts.

U9. **`compatibility_minimum`.** `"4.7"` vs `"4.7.2"` is not settled by any
repository file. **Recommendation:** `"4.7"` (the API level), recording the
working value after E1.

U10. **First-build lock.** ADR §15's `--locked` cannot hold for a dependency
the lock has never seen. **Recommendation:** the one unwrapped build writes
`Cargo.lock`; every gate afterwards is `--locked`. A command-order fix, not a
decision change.

U. **Unverified list:** (1) exact `#[gdextension]`/`#[godot_api]`/`#[func]`
syntax and `Array<VarDictionary>`/`Gd<CrpgView>` marshalling (the macros and
`PackedArray` conversions exist, per the sources cited); (2) `.gdextension`
keys, `compatibility_minimum`, `reloadable`, `res://../../target/...` and
`CARGO_TARGET_DIR`; (3) the resolved `gdextension-api` version and that
`api-4-7` bundles a 4.7.0 single-precision JSON; (4) the first `api-4-7` load
on 4.7.2; (5) whether default-feature codegen fits the build/CI budget;
(6) `cargo deny`'s feature resolution (C5); (7) a `-s` `SceneTree` script's
`_process`, `--import`'s exit code and `-s` path resolution;
(8) `multiple-versions = "deny"` / `unmaintained = "all"` against the godot
tree, and the four non-MPL licences; (9) `String.simplify_path`/`path_join`
(low risk).

## Agent log

- 2026-11-02 (UTC) · opencode-go/deepseek-v4.1-flash + T047 draft · Pinned the bridge skeleton against the landed T046/T047a code: exact manifest and engine/API pins, the core/shell split, the open and per-frame paths, the 20-code table, `apps/client`, the build commands, C1–C26 with controls, and the two-session split, marking every fact the repository cannot settle.

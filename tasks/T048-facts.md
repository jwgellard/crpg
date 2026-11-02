# T048 facts — scene sync and proxy lifecycle (§19.2 #12)

Fact sheet for the T048 contract author. Facts only. Every `path:line` is
relative to the repository root and resolves on this branch.

## 1. Spec text: scene sync, proxies, `NetId`, spawn/despawn, interest, presentation

- `docs/CRPG_ENGINE_SPEC.md:310` — "Godot's SceneTree is used on the client only, as a **presentation mirror**: each replicated entity gets a `Node3D` proxy created and destroyed by a single `SceneSyncSystem`. The proxy holds no gameplay state. If you ever find gameplay logic in a Godot node, that is a bug with a specific name: *authority leak*."
- `docs/CRPG_ENGINE_SPEC.md:170` — "`crpg-net` owns filtered replica storage, delta application and engine-neutral read queries; `crpg-godot` owns the engine-facing query and presentation objects, and neither gets mutable authoritative access."
- `docs/CRPG_ENGINE_SPEC.md:855`–`869` — client tree lists `SceneSync        entity → Node3D proxy lifecycle` at `:863`.
- `docs/CRPG_ENGINE_SPEC.md:880` — "The client renders `SimEvent`s, it does not infer them. Do not diff HP to discover that damage happened."
- `docs/CRPG_ENGINE_SPEC.md:755` — events stream includes "sim events: damage, effects, dialogue, quest, spawns".
- `docs/CRPG_ENGINE_SPEC.md:815` — "Tier 1 (now): interest set = the area the player's character is in."
- `docs/CRPG_ENGINE_SPEC.md:1152` — "bulk scene synchronization and animation LOD are separate bridge work".
- `docs/CRPG_ENGINE_SPEC.md:1480` — desync test: "run server and client replicas over the simulated network for 5,000 ticks and assert the replica's visible state matches the server's filtered projection."
- `docs/CRPG_ENGINE_SPEC.md:1590` — deliverables include "scene sync".
- `docs/CRPG_ENGINE_SPEC.md:1666` — "22. A Godot scene that spawns N `Node3D` proxies from an external array."
- `docs/CRPG_ENGINE_SPEC.md:1686` — "12. Scene sync on the client with proxy lifecycle."

## 2. ADR-0033 passages on proxies, scene sync, `net_ids`, present, revisions, frames, despawn, T048

- `docs/adr/0033-godot-bridge.md:24` — "T047 skeleton with embedded host, T048 proxies, a cut-down T051, playing".
- `docs/adr/0033-godot-bridge.md:154` — "8. if anything changed: revision += 1; rebuild CrpgView".
- `docs/adr/0033-godot-bridge.md:156` — "if v.revision() != seen: SceneSync.sync(v); Hud.sync(v)".
- `docs/adr/0033-godot-bridge.md:270` — "`revision() -> int` | Increases when state, options, events or receipts change".
- `docs/adr/0033-godot-bridge.md:271` — "`net_ids() -> PackedInt64Array` | Visible entities, ascending. Every per-entity array below is indexed the same way".
- `docs/adr/0033-godot-bridge.md:278` — "`dead() -> PackedByteArray` | Terminal flag, `0` where unknown".
- `docs/adr/0033-godot-bridge.md:283` — "`option_targets() -> PackedInt64Array` | Target `NetId` per option, `0` for End turn, so clicking a proxy can find its option later".
- `docs/adr/0033-godot-bridge.md:284`–`285` — "`events()` / `receipts()` ... oldest first".
- `docs/adr/0033-godot-bridge.md:341`–`342` — "`FRAME_UNDECODABLE` ... `REPLICA_REJECTED`".
- `docs/adr/0033-godot-bridge.md:415` — "scripts/SceneSync.gd   NetId -> placeholder primitive proxies; no gameplay logic".
- `docs/adr/0033-godot-bridge.md:423` — "Proxy layout is client-only (one row per side)".
- `docs/adr/0033-godot-bridge.md:517` — "**T048** owns proxy lifecycle in GDScript: diff `net_ids()` between revisions, free on leave, show `dead()`. It adds no FFI member."
- `docs/adr/0033-godot-bridge.md:525`–`529` — "T046 must provide ... applying a v2 `DeltaFrame`, in order, with an error rather than a skip on a gap; entity listing ascending by `NetId`; health absent distinct from zero; the turn fact. ... `apply_delta` returns `()`: T048 diffs `net_ids` between revisions itself".
- `docs/adr/0033-godot-bridge.md:534` — "the FFI is 6 + 16 members, and T048 and T051 should need no Rust."

## 3. Architecture doc and crate AGENTS: proxy layout, first vs later revisions, deferred-to-T048 lines

- `docs/architecture/crpg-godot.md:8`–`17` — state after T047b: Godot-free core complete; shell and client load test exist.
- `docs/architecture/crpg-godot.md:96` — "The revision becomes `previous + 1` when this step collected any event or receipt, or when rows/active/round/ended/options differ. Otherwise the previous `Rc`, with its own events and receipts, is kept unchanged."
- `docs/architecture/crpg-godot.md:105`–`108` — view contract: "`net_ids().len()` entries. Missing presentation rows produce `""`, never a panic. Unknown health is `(0, 0, known 0, dead 0)`; a `Died` notice never sets `dead`. `encounter_ended` latches `ended`."
- `docs/architecture/crpg-godot.md:172`–`187` — `Main.gd`: `_process` makes one `poll()`; when revision changes updates `seen` and calls `Proxies.sync(view)` and `Hud.sync(view)`. `SceneSync.gd` spawns once from first non-empty view; "there is no despawn; that is T048"; one `Node3D` named `proxy_<id>` per row; capsule/box by side; position `x = 3 × side index`, `z = 1.5 × index within that side`, `y = 0`; `proxy_count()` counts `proxy_` children.
- `docs/architecture/crpg-godot.md:199`–`200` — smoke test requires `$Proxies.proxy_count() == net_ids().size() == 2`.
- `docs/architecture/crpg-godot.md:203` — "T048 owns respawn/despawn and T049 positions."
- `docs/architecture/crpg-godot.md:257` — "Added the Client section: ... so T048/T050 extend a stated baseline rather than re-deriving one."
- `crates/crpg-godot/AGENTS.md:1`–`6` — design pointer and T047 prerequisite. (No T048 mention in this file beyond the architecture/ADR pointers above.)

Deferred-to-T048 / that-is-T048 lines:
- `docs/architecture/crpg-godot.md:182` — "is no despawn; that is T048)".
- `docs/architecture/crpg-godot.md:203` — "T048 owns respawn/despawn and T049 positions."
- `apps/client/scripts/SceneSync.gd:4` — "and never despawns (that is T048)."

## 4. Every mention of T048 (tasks/, docs/, crates/, apps/)

- `docs/adr/0033-godot-bridge.md:24` — "T048 proxies".
- `docs/adr/0033-godot-bridge.md:517` — "T048 owns proxy lifecycle in GDScript...".
- `docs/adr/0033-godot-bridge.md:534` — "T048 and T051 should need no Rust.".
- `docs/PROJECT_STATE.md:7` — "T048 and the cut-down T051; T098e and T047e are follow-ups.".
- `docs/IMPLEMENTATION_PLAN.md:252` — "T048 | C → I | crpg-godot | T047 | Scene sync and proxy lifecycle (§19.2 #12)".
- `docs/IMPLEMENTATION_PLAN.md:255` — "T051 | ... | T048, T104, T108 | ...".
- `docs/IMPLEMENTATION_PLAN.md:412` — "Demo-first ... T027b, T046, T045, T047, T048, cut-down T051".
- `docs/IMPLEMENTATION_PLAN.md:454` — demo-first row added.
- `docs/architecture/crpg-godot.md:182` — "that is T048".
- `docs/architecture/crpg-godot.md:203` — "T048 owns respawn/despawn...".
- `docs/architecture/crpg-godot.md:257` — "so T048/T050 extend...".
- `tasks/T027b-facts.md:60` — "PLAN demo-first ... T048 ...".
- `tasks/T045-draft-a.md:37` — "T047 skeleton plus embedded host, T048 proxies...".
- `tasks/T045-draft-a.md:270` — "T048 (scene sync, `:250`): the `NetId`→proxy lifecycle...".
- `tasks/T045-draft-a.md:282` — "Full T051 waits on T048, T104, T108".
- `tasks/T045-draft-b.md:46` — "T047 skeleton plus embedded host, T048 proxies...".
- `tasks/T045-draft-b.md:224` — "Transfer -> receive into replica (T046); fenced until T048's test".
- `tasks/T045-draft-b.md:312` — "T048 owns proxy lifecycle: diff `net_ids()`/`present()` between frames, free on leave/despawn...".
- `tasks/T045-facts.md:60` — "T048 ... Scene sync and proxy lifecycle (§19.2 #12)".
- `tasks/T045-facts.md:62` — "T051 ... prerequisites T048, T104, T108".
- `tasks/T046.md:95` — "`apply_delta` returns `()`: T048 diffs `net_ids` between revisions itself".
- `tasks/T047-draft-a.md:302` — "Deferred to T048: diffing `net_ids()` between revisions, freeing on leave, `dead()` visuals...".
- `tasks/T047-draft-b.md:351` — "Proxy lifecycle (diff/free/`dead()`) is T048's; scene sync beyond spawn is T048's...".
- `tasks/T047-draft-b.md:463` — "T048 proxy lifecycle/scene sync (diff `net_ids`, free on leave, `dead()`)".
- `tasks/T047-draft-b.md:497` — "Left to: T048 proxy lifecycle...".
- `tasks/T047.md:637` — "No despawn, picking or camera control (T048/T050).".
- `tasks/T047.md:641` — "T048 lifecycle, T049 `positions()`...".
- `apps/client/scripts/SceneSync.gd:4` — "and never despawns (that is T048)."

## 5. Replica API T048 will read (owner: `crpg-net`, `crates/crpg-net/src/replica.rs`)

Type: `ReplicaWorld` (struct, `:121`). Fields private; no `Default`, no restore, no serde
(`tasks/T046.md:151`–`162`).

Public methods:

- `:132` `pub fn new(epoch: SessionEpoch) -> Self` — doc `:131` "A fresh, empty replica for `epoch`. The epoch never changes."
- `:144` `pub fn epoch(&self) -> SessionEpoch` — doc `:143` "The session generation this replica belongs to."
- `:149` `pub fn entities(&self) -> &[ReplicaEntity]` — doc `:148` "Every live row, ascending `NetId`; at most `MAX_REPLICA_ENTITIES`."
- `:154` `pub fn entity(&self, net_id: NetId) -> Option<&ReplicaEntity>` — doc `:153` "One live row by binary search; `None` for any id not live."
- `:159` `pub fn len(&self) -> usize` — doc `:158` "The number of live rows."
- `:164` `pub fn is_empty(&self) -> bool` — doc `:163` "Whether no row is live."
- `:169` `pub fn turn(&self) -> Option<&TurnFact>` — doc `:168` "The last applied turn fact; `None` before any in this replica."
- `:175` `pub fn next_event_seq(&self) -> u64` — doc `:174` "The only `event_seq` `apply_delta` accepts next..."
- `:180` `pub fn server_tick(&self) -> u64` — doc `:179` "The last accepted frame's `server_tick` or publication's `base_tick`."
- `:188` `pub fn apply_delta(&mut self, frame: &DeltaFrame) -> Result<(), ReplicaError>` — doc `:184` "Applies one decoded v2 lane-0 delta in strict sequence order."
- `:220` `pub fn apply_snapshot(&mut self, publication: &Publication) -> Result<(), ReplicaError>` — doc `:216` "Replaces the replica with one published snapshot body (N5)."

`ReplicaEntity` (`:24`): `net_id: NetId`, `health: Option<HealthFact>` (absent = unknown, never zero).

GDScript-facing view T048 actually reads (owner: `crpg-godot`, `crates/crpg-godot/src/core/client.rs`, `SessionView`):
- `:49` `revision(&self) -> i64`; `:54` `net_ids(&self) -> &[i64]`; `:59` `names`; `:64` `sides`; `:69` `placements`; `:74` `health`; `:79` `max_health`; `:84` `health_known`; `:89` `dead`; `:94` `active`; `:99` `round`; `:104` `ended`; `:109` `option_labels`; `:114` `option_targets`; `:119` `events`; `:124` `receipts`.
Shell getters mirror these in `crates/crpg-godot/src/shell.rs` (`:107`–`:208`).

## 6. `crates/crpg-godot/src/` files, godot_api/func/signal items, proxy creation today

Files:
- `crates/crpg-godot/src/lib.rs` — crate root; `#![allow(unsafe_code)]`, `#![warn(missing_docs)]`; declares `core`, and `shell` under `engine`.
- `crates/crpg-godot/src/core/mod.rs` — Godot-free core; `BridgeCore`, `State`, `Live`, view re-exports.
- `crates/crpg-godot/src/core/errors.rs` — `BridgeCode` (20 codes), `Fault`, `host_fault`.
- `crates/crpg-godot/src/core/link.rs` — `LocalLink`, two FIFOs, cap 128.
- `crates/crpg-godot/src/core/client.rs` — `Client`, `SessionView`, `ViewEvent`, `ViewReceipt`, `ViewValue`.
- `crates/crpg-godot/src/core/authority.rs` — `Authority`, `Presentation`, campaign walk, NPC driver, presentation tables.
- `crates/crpg-godot/src/core/tests.rs` + `errors/tests.rs` + `link/tests.rs` + `client/tests.rs` + `authority/tests.rs` — Godot-free tests.
- `crates/crpg-godot/src/shell.rs` — gdext shell; `CrpgSession`, `CrpgView`, unsafe impl.
- `crates/crpg-godot/tests/bridge.rs` — public API fixture test.
- `crates/crpg-godot/tests/source_rules.rs` — source invariants.

`#[godot_api]` items: `crates/crpg-godot/src/shell.rs:36` (`CrpgSession`), `:104` (`CrpgView`). No `#[signal]` items exist. `#[func]` items:

`CrpgSession` (`shell.rs`):
- `:39` `fn open(&mut self, campaign_dir: GString, encounter_id: GString, player_side: GString, seed: i64) -> i64`
- `:61` `fn poll(&mut self) -> i64`
- `:70` `fn view(&self) -> Gd<CrpgView>`
- `:78` `fn submit_option(&mut self, revision: i64, index: i64) -> i64`
- `:84` `fn close(&mut self)`
- `:91` `fn last_error(&self) -> GString`

`CrpgView` (`shell.rs`):
- `:107` `fn revision(&self) -> i64`
- `:113` `fn net_ids(&self) -> PackedInt64Array`
- `:119` `fn names(&self) -> PackedStringArray`
- `:125` `fn sides(&self) -> PackedStringArray`
- `:131` `fn placements(&self) -> PackedStringArray`
- `:137` `fn health(&self) -> PackedInt64Array`
- `:143` `fn max_health(&self) -> PackedInt64Array`
- `:149` `fn health_known(&self) -> PackedByteArray`
- `:155` `fn dead(&self) -> PackedByteArray`
- `:161` `fn active(&self) -> i64`
- `:167` `fn round(&self) -> i64`
- `:173` `fn ended(&self) -> bool`
- `:179` `fn option_labels(&self) -> PackedStringArray`
- `:185` `fn option_targets(&self) -> PackedInt64Array`
- `:191` `fn events(&self) -> Array<VarDictionary>`
- `:201` `fn receipts(&self) -> Array<VarDictionary>`

Unsafe: `crates/crpg-godot/src/shell.rs:25`–`26` `unsafe impl ExtensionLibrary for CrpgExtension {}` under SAFETY comment (`:19`–`24`).

How proxies are created today (GDScript, `apps/client/scripts/SceneSync.gd`):
- `:8` `func sync(view)` returns early if `_spawned` or empty; sets `_spawned = true` at `:14`.
- `:19`–`26` loop builds `side_order`/`side_counts`, calls `_spawn_proxy(ids[i], names[i], side_index, within)` at `:26`.
- `:35` `_spawn_proxy` creates `Node3D.new()` (`:36`), sets `proxy.name = "proxy_%d" % net_id` (`:37`), position `Vector3(3.0 * float(side_index), 0.0, 1.5 * float(within))` (`:38`), adds `MeshInstance3D` (capsule if side_index == 0 else box) (`:40`–`49`) and billboarded `Label3D` with display name (`:52`–`56`), then `add_child(proxy)` (`:58`).
- `:28` `proxy_count()` counts children whose name begins with `proxy_`.
- No node is freed today; `apps/client/scripts/Main.gd:42` calls `$Proxies.sync(next)` on revision change.

## 7. `apps/client/` files and proxy node sites

Files:
- `apps/client/crpg.gdextension` — DLL path/entry symbol (Windows x86_64 debug+release).
- `apps/client/project.godot` — project config.
- `apps/client/scenes/Main.tscn` — camera, ground plane, HUD root.
- `apps/client/scripts/Main.gd` — owns `CrpgSession`, polls once per `_process`, input.
- `apps/client/scripts/SceneSync.gd` — NetId → primitive proxy spawn (no despawn).
- `apps/client/scripts/CombatHud.gd` — round, active, health, options, log, end.
- `apps/client/tests/load.gd` — headless load proof (E1).
- `apps/client/tests/smoke.gd` — headless playthrough (BR-22).

Proxy node create/lookup/free sites:
- `apps/client/scripts/SceneSync.gd:8` `sync(view)`; `:14` `_spawned = true`; `:35`–`58` `_spawn_proxy` creates node (`:36`), names it `proxy_<id>` (`:37`), adds child (`:58`).
- `apps/client/scripts/SceneSync.gd:28` `proxy_count()` iterates `get_children()` and counts names beginning `proxy_`.
- `apps/client/scripts/Main.gd:42` `$Proxies.sync(next)` on revision change.
- `apps/client/tests/smoke.gd:50` asserts `proxy_count() == net_ids().size() == 2` (exit 0 only then).
- No free/queue_free of proxy nodes exists anywhere today.

## 8. Existing tests

Godot-free (`cargo test -p crpg-godot`):
- `crates/crpg-godot/tests/bridge.rs:20` `fixture_opens_and_polls_to_the_expected_view`
- `crates/crpg-godot/tests/bridge.rs:60` `close_is_idempotent_through_the_public_api`
- `crates/crpg-godot/tests/source_rules.rs:82` `the_core_has_no_engine_clock_or_bare_core_path`
- `crates/crpg-godot/tests/source_rules.rs:189` `the_crate_has_exactly_one_unsafe_on_the_extension_impl`
- `crates/crpg-godot/src/core/tests.rs:138` `fixture_view_is_exact_and_rows_use_the_player_replica_map`
- `crates/crpg-godot/src/core/tests.rs:184` `open_failure_codes_and_precedence_leave_no_session`
- `crates/crpg-godot/src/core/tests.rs:252` `npc_fires_once_at_the_delay_and_never_twice_in_a_poll`
- `crates/crpg-godot/src/core/tests.rs:327` `idle_poll_changes_nothing`
- `crates/crpg-godot/src/core/tests.rs:369` `a_full_link_defers_the_next_frame_until_it_is_read`
- `crates/crpg-godot/src/core/tests.rs:480` `captures_are_acknowledged_every_poll`
- `crates/crpg-godot/src/core/tests.rs:514` `the_authority_hash_matches_an_independent_oracle`
- `crates/crpg-godot/src/core/tests.rs:643` `submit_precedence_and_a_successful_send`
- `crates/crpg-godot/src/core/tests.rs:672` `submit_precedence_pairs`
- `crates/crpg-godot/src/core/tests.rs:715` `gameplay_rejection_is_a_receipt`
- `crates/crpg-godot/src/core/tests.rs:768` `dropped_replies_are_intent_refused_not_fatal`
- `crates/crpg-godot/src/core/tests.rs:810` `oversized_frames_are_refused_without_fencing`
- `crates/crpg-godot/src/core/tests.rs:824` `corrupt_frame_fences_and_close_resets`
- `crates/crpg-godot/src/core/tests.rs:859` `time_regression_fences`
- `crates/crpg-godot/src/core/tests.rs:875` `argument_checks_run_before_the_session`
- `crates/crpg-godot/src/core/tests.rs:927` `panics_are_caught_at_every_boundary`
- `crates/crpg-godot/src/core/tests.rs:992` `close_is_idempotent_and_keeps_the_error_text`
- `crates/crpg-godot/src/core/tests.rs:1014` `an_absent_ruleset_is_campaign_invalid`
- `crates/crpg-godot/src/core/tests.rs:1043` `fixture_strike_discloses_every_event_field`
- `crates/crpg-godot/src/core/tests.rs:1212` `a_refused_frame_clears_the_pending_submit`
- `crates/crpg-godot/src/core/tests.rs:1235` `public_open_uses_the_128_frame_link`
- `crates/crpg-godot/src/core/errors/tests.rs:10` `every_code_matches_the_adr_table`
- `crates/crpg-godot/src/core/errors/tests.rs:54` `host_failures_split_on_closed`
- `crates/crpg-godot/src/core/link/tests.rs:8` `a_full_direction_refuses_and_stays_fifo`
- `crates/crpg-godot/src/core/link/tests.rs:33` `the_down_direction_refuses_at_the_cap_and_stays_fifo`
- `crates/crpg-godot/src/core/link/tests.rs:57` `each_direction_has_its_own_queue`
- `crates/crpg-godot/src/core/client/tests.rs:120` `the_default_view_is_empty_revision_zero`
- `crates/crpg-godot/src/core/client/tests.rs:135` `rows_follow_the_replica_and_known_health_is_independent_of_zero`
- `crates/crpg-godot/src/core/client/tests.rs:209` `events_and_receipts_are_exact_and_not_repeated`
- `crates/crpg-godot/src/core/client/tests.rs:339` `revision_bumps_at_most_once_and_keeps_the_rc_when_idle`
- `crates/crpg-godot/src/core/client/tests.rs:394` `active_must_be_a_live_row_and_round_is_unadjusted`
- `crates/crpg-godot/src/core/client/tests.rs:415` `pending_is_cleared_only_by_its_receipt_or_a_refusal`
- `crates/crpg-godot/src/core/client/tests.rs:445` `option_labels_use_the_full_table_then_their_fallbacks`
- `crates/crpg-godot/src/core/client/tests.rs:495` `bad_frames_are_fatal_and_never_skipped`
- `crates/crpg-godot/src/core/client/tests.rs:520` `submit_checks_in_order_and_only_the_success_sends`
- `crates/crpg-godot/src/core/client/tests.rs:584` `missing_ability_uses_ulid_text_and_wide_saturates`
- `crates/crpg-godot/src/core/client/tests.rs:642` `died_does_not_set_the_dead_column`
- `crates/crpg-godot/src/core/authority/tests.rs:126` `hand_off_stops_at_the_first_full_link`
- `crates/crpg-godot/src/core/authority/tests.rs:159` `choose_npc_prefers_a_cross_side_attack`
- `crates/crpg-godot/src/core/authority/tests.rs:205` `pacer_arms_then_fires_only_at_the_due_time`
- `crates/crpg-godot/src/core/authority/tests.rs:231` `npc_results_keep_a_held_intent_and_fence_a_rejection`
- `crates/crpg-godot/src/core/authority/tests.rs:388` `collect_campaign_refuses_the_same_shapes_as_the_cli_walker`
- `crates/crpg-godot/src/core/authority/tests.rs:480` `check_binding_requires_the_authored_order`
- `crates/crpg-godot/src/core/authority/tests.rs:524` `npc_delivery_is_acknowledged_and_its_inflight_clears`

GDScript/headless (apps/client):
- `apps/client/tests/load.gd` — E1 load proof; exit 0 pass, 2 class missing, 3 first failed check; control via `CRPG_CAMPAIGN` missing dir exits 3.
- `apps/client/tests/smoke.gd` — BR-22 playthrough; chooses option 0 per new revision, exits 0 only when `ended()` and proxy_count == net_ids size == 2.

CI (`/.github/workflows/ci.yml`):
- `:192` `client` job (Windows only).
- `:225` clippy with `--features engine`.
- `:227` build with `--features engine`.
- `:229` licence notice check (8 packages).
- `:247` fetch Godot 4.7.2 with SHA-256/SHA-512.
- `:275` seed `.godot/extension_list.cfg`.
- `:284` import project.
- `:293` E1 load proof.
- `:302` E1 control.
- `:313` BR-22 smoke.
- `:322` BR-22 control.
- `:333` stage client slice.
- `:351` upload artifact.

## 9. Godot-free testability and `engine` feature gating

- `crates/crpg-godot/Cargo.toml:10`–`11` — `[features]` `engine = ["dep:godot"]`.
- `crates/crpg-godot/Cargo.toml:13`–`19` — dependencies; `godot = { version = "=0.5.5", optional = true, features = ["api-4-7"] }`.
- `crates/crpg-godot/src/lib.rs:14`–`15` — `#[cfg(feature = "engine")] mod shell;`.
- `crates/crpg-godot/src/core/mod.rs:1` — `#![forbid(unsafe_code)]`; core has no godot import.
- `crates/crpg-godot/src/core/mod.rs:8`–`11` — `mod authority; mod client; mod errors; mod link;`.
- `crates/crpg-godot/src/shell.rs:12` — `use godot::prelude::*;` (compiled only with `engine`).
- `tools/lint/determinism.py:57` — `CRATES = ["crpg-core", "crpg-rules", "crpg-sim"]`; `crpg-godot` is not scanned.
- Godot-free testable modules: `core/errors.rs`, `core/link.rs`, `core/client.rs`, `core/authority.rs`, `core/mod.rs` and their `tests.rs` children; `tests/bridge.rs`, `tests/source_rules.rs`. All compile with default features (no `engine`) on both targets.

## Agent log

- 2026-11-02 (UTC) · opencode-go/beam-free + T048 facts · Fact sheet for the T048 contract: spec, ADR-0033, architecture/AGENTS, all T048 mentions, replica/view API, godot src files and funcs, client files and proxy sites, tests, and engine gating.
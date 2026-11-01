# ADR-0033 (proposed): The Godot bridge — FFI surface, threading and the embedded host

Date: 2026-11-01 (UTC)
Status: **Proposed draft** — one of two independent drafts written by T045. The number is
provisional; the adjudicating architect assigns it on acceptance. Nothing here is approved
or implemented.

## Context

Spec §11.3 keeps the Rust↔GDScript surface to roughly five object types and says everything
else stays in Rust (`docs/CRPG_ENGINE_SPEC.md:1008`, `docs/CRPG_ENGINE_SPEC.md:1015`,
`docs/CRPG_ENGINE_SPEC.md:1019`). E012/T032 fixed the package shape: client and editor are
Godot projects (`apps/client`, `apps/editor`) over `crpg-godot` and never Rust packages; there
is no `crpg-client-bridge`; `crpg-server` owns the reusable host as a library; `crpg-net` owns
replica storage and engine-neutral queries; `crpg-godot` owns the engine-facing query objects
(`tasks/E012-binary-crate-naming.md:65`, `tasks/E012-binary-crate-naming.md:68`,
`tasks/E012-binary-crate-naming.md:72`, `docs/CRPG_ENGINE_SPEC.md:163`). No new package and no
`ALLOWED` change followed (`tasks/E012-binary-crate-naming.md:74`), and `crpg-godot` may
already import any workspace crate (`tools/lint/deps.py:71`).

As built, `crpg-godot` is empty: an empty dependency table
(`crates/crpg-godot/Cargo.toml:7`), a four-line root with `#![allow(unsafe_code)]`
(`crates/crpg-godot/src/lib.rs:1`), and an architecture doc due with its first task
(`docs/architecture/README.md:58`). ADR-0003's bulk `snapshot()` is the prior pattern
(`docs/adr/0003-gdextension-rendering-spike.md:50`).

The host is the right embedded authority and its shape is fixed. It is one serial owner of a
private `HistoryWorld` (`crates/crpg-server/src/host.rs:524`); every call takes an injected
`now_ms` (`crates/crpg-server/src/host.rs:549`, `crates/crpg-server/src/host.rs:904`,
`crates/crpg-server/src/host.rs:954`); its source has no clock, thread, sleep or socket
(`crates/crpg-server/AGENTS.md:69`); no client receives `&Host`, `&mut World` or capture
access (`crates/crpg-server/src/lib.rs:29`). Ingest is byte-level admission only
(`crates/crpg-server/src/host.rs:904`), and delivery is raw bytes in ordinal order
(`crates/crpg-server/src/host.rs:1695`). The authority boundary is unchanged in-process
(`docs/adr/0012-windows-primary-platform.md:30`), and Linux headless must not depend on
Godot (`docs/adr/0012-windows-primary-platform.md:23`).

Two wire facts bound the first slice. There is no transform op in v1
(`crates/crpg-net/src/protocol.rs:305`), so there is nothing to interpolate until the lane-1
movement channel exists. The intent vocabulary is exactly `DeclareAction { ability, target }`
and `EndTurn` (`crates/crpg-net/src/protocol.rs:268`); downstream is health, turn, receipts
and the v2 events (`crates/crpg-net/src/protocol.rs:309`,
`crates/crpg-net/src/protocol_v2.rs:88`).

Standing order (user, 2026-11-01): T045 runs before the rest of M4, so movement and pathfinding
do not exist. The first consumer is the thin slice — T047 skeleton plus embedded host, T048
proxies, a cut-down T051 that shows and plays turn-based combat as `crpgc play` does in the
terminal (`crates/crpg-cli/src/play.rs:110`, `crates/crpg-cli/src/play.rs:128`). Placeholder
primitives stand in for art, so T105 does not gate it. This ADR must not paint the later full
client into a corner, and the client never mutates authoritative state directly.

Flagged source: `docs/IMPLEMENTATION_PLAN.md:247` and `docs/IMPLEMENTATION_PLAN.md:249` still
say `client/`. E012 resolved `apps/client` (`tasks/E012-binary-crate-naming.md:65`), and the
spec agrees (`docs/CRPG_ENGINE_SPEC.md:855`, `docs/CRPG_ENGINE_SPEC.md:1206`). This draft uses
`apps/client`; those rows need a manager edit, not a workaround here.

## Decision

1. **The bridge is `crpg-godot`; the client project is `apps/client`.** T047 opens the crate as
   a `cdylib` plus a `lib` target and creates `apps/client` with one scene; the crate already
   has the legal edges (`tools/lint/deps.py:71`) and the boundary is in the reviewed S-set
   (`docs/IMPLEMENTATION_PLAN.md:95`). **Rejected:** a new `crpg-client` package — E012 closed
   it (`tasks/E012-binary-crate-naming.md:68`) and it would need an `ALLOWED` change.

2. **The embedded authority is built once, before the wrap, and never touched again.** The
   bridge reads the campaign directory (an OS path, not `res://`; its only filesystem I/O, since
   `crpg-data` does none — `crates/crpg-data/src/loader.rs:250`), validates it, assembles the
   `EncounterSpec`, starts the encounter on a `HistoryWorld`
   (`crates/crpg-sim/src/history.rs:634`), acknowledges the start range
   (`crates/crpg-sim/src/history.rs:810`), then hands the wrapper to `Host::from_history`
   (`crates/crpg-server/src/host.rs:556`); `Host::new` has no encounter and `from_history`
   refuses a non-empty journal (`crates/crpg-server/src/host.rs:549`,
   `crates/crpg-server/src/host.rs:561`). The slice passes `incarnation = 1`; it must strictly
   increase across restores (`crates/crpg-server/src/host.rs:115`). **Rejected:** a "start
   encounter" intent — no such verb exists (`crates/crpg-net/src/protocol.rs:268`).

3. **The bridge preserves the byte boundary; it does not call the sim.** GDScript input becomes
   canonical intent bytes through `crpg-net`'s encoder (`crates/crpg-net/src/codec.rs:154`),
   enters through `Host::ingest`, and leaves through `Host::take_delivery_entries`; no sim
   mutator is reachable after the wrap (`crates/crpg-server/AGENTS.md:43`). An in-memory
   `Transport` is not used: the host already takes bytes
   (`crates/crpg-server/src/host.rs:904`), and that fabric exists to prove the net layer
   (`crates/crpg-net/src/sim.rs:385`). **Rejected:** calling `crpg_sim::perform_action` from the
   bridge, as the CLI does (`crates/crpg-cli/src/play.rs:128`) — it would make the client the
   decider, which spec §9.2 forbids (`docs/CRPG_ENGINE_SPEC.md:879`).

4. **One thread: the Godot main thread owns the host and the replica.** `_process`, every FFI
   method and every signal run there; the host is a serial owner with no internal thread
   (`crates/crpg-server/src/host.rs:524`, `crates/crpg-server/AGENTS.md:69`). **Rejected:** a
   worker thread owning the host plus a channel — it needs double buffering, a second clock and
   cross-thread signal emission, and Godot object access is main-thread.

5. **The bridge owns the clock and pumps once per `_process`.** `now_ms` is monotonic
   milliseconds from the bridge's own start instant, read in Rust; no FFI method takes a
   timestamp, so GDScript cannot regress time (`crates/crpg-server/src/host.rs:679`). `poll()`
   calls `Host::pump` once (`crates/crpg-server/src/host.rs:954`); the QUIC adapter's "at most
   every 50 ms" rule is for hello decisions (`crates/crpg-server/AGENTS.md:226`) and no hellos
   exist here. **Rejected:** a fixed 20 Hz authority tick as in the spike
   (`docs/adr/0003-gdextension-rendering-spike.md:18`); the slice has no movement and
   `Host::tick` journals nothing (`crates/crpg-server/src/host.rs:1950`).

6. **Delivery is drained in order, applied, then acknowledged by ordinal.** Each entry is
   decoded by its `DeliveryKind` (`crates/crpg-server/src/host.rs:185`), applied to the replica,
   and the batch's last ordinal acknowledged (`crates/crpg-server/src/host.rs:1721`); a failure
   stops the batch *before* the acknowledgement and fences with `Internal`. **Rejected:** skip
   and acknowledge past a bad entry — the replica would silently diverge and the host would
   never redeliver the frame (`crates/crpg-server/src/host.rs:1682`).

7. **The embedded peer binds as a snapshot session, and the kind never changes.**
   `bind_snapshot_peer` binds like `bind_peer` with delta state from seq 1
   (`crates/crpg-server/src/host.rs:704`,
   `docs/adr/0031-host-snapshot-sender-and-snapshot-sessions.md:26`) but lets a later narrowing
   stage a snapshot instead of resetting delivery (`docs/architecture/crpg-server.md:167`).
   **Rejected:** `bind_peer` now and rebind later — the kind is fixed for the session's life
   (`docs/adr/0031-host-snapshot-sender-and-snapshot-sessions.md:31`), so a later interest
   change would reset `event_seq` to 1, a visible reset in the full client.

8. **The FFI exposes `NetId` only; `EntityId` never crosses.** The host's map is private with no
   accessor (`crates/crpg-server/src/host.rs:456`, `crates/crpg-server/AGENTS.md:43`) and raw
   arena ids are never serialized (`crates/crpg-net/src/protocol.rs:209`). The bridge captures
   the `EntityId`s it needs for `ControlGrant`/`DisclosureGrants` *before* the wrap, then keeps
   a one-way `NetId → presentation key` table built at bind from the documented ascending-
   `EntityId` minting order (`docs/architecture/crpg-server.md:161`). **Rejected:** expose
   `EntityId` or keep a bridge-side map — a generation-bearing arena id is a stale-id hazard
   (`crates/crpg-net/src/protocol.rs:213`) and a second map can disagree with the host's.

9. **The view is one bulk, read-only object rebuilt per pump.** `CrpgView` returns parallel
   arrays in one call per frame, following ADR-0003's finding that bulk transfer beats
   per-entity calls (`docs/adr/0003-gdextension-rendering-spike.md:50`); it has no mutators.
   **Rejected:** per-entity getters or `Node3D` properties as state — spec §9.2 requires a query
   API (`docs/CRPG_ENGINE_SPEC.md:881`) and N getters per frame is what ADR-0003 measured.

10. **Intent submission is typed to the two v1 verbs.** `submit_declare_action` and
    `submit_end_turn` build an `IntentFrame` (`crates/crpg-net/src/protocol.rs:245`) with the
    cached epoch (`crates/crpg-server/src/host.rs:622`), a bridge-owned per-lane sequence, and
    `observed_tick` from `server_tick` (`crates/crpg-server/src/host.rs:630`). **Rejected:** a
    generic `submit(bytes)` — it puts wire encoding in GDScript and would let the client invent
    verbs the protocol lacks (`crates/crpg-net/src/protocol.rs:262`).

11. **Errors are a closed numeric code space; gameplay rejections are not errors.** Host and
    codec failures return a table code, with the message in `CrpgDiagnostics`; a rejected
    command is a receipt (`crates/crpg-net/src/protocol.rs:378`) in the view. **Rejected:** one
    code per enum variant — `CodecError` (`crates/crpg-net/src/codec.rs:52`) and
    `RejectionCode` (`crates/crpg-net/src/protocol.rs:393`) are internal vocabularies whose
    order is not a promise.

12. **No hand-written `unsafe`, and no panic crosses the boundary.** The crate keeps
    `#![allow(unsafe_code)]` (`crates/crpg-godot/src/lib.rs:1`) for gdext's generated code, not
    ours (`tools/lint/deps.py:267`); all logic lives in a Godot-free module returning `Result`
    and the `#[godot_api]` shell only converts. **Rejected:** `panic = "abort"` — it would kill
    the editor and change every crate's build.

13. **The engine dependency is an exact, non-default `engine` feature.** The manifest gains
    `godot = { version = "=0.5.5", optional = true }` and `engine = ["dep:godot"]`, as D27d pins
    it, `crpg-godot` only (`tasks/DECISIONS-2026-10-04.md:69`); with the feature off the crate
    compiles to the Godot-free core and its tests. **Rejected:** an unconditional dependency —
    `cargo test --workspace` runs on Linux (`.github/workflows/ci.yml:105`), Linux headless must
    not depend on Godot (`docs/adr/0012-windows-primary-platform.md:23`,
    `docs/CRPG_ENGINE_SPEC.md:1340`), and gdext may want `GODOT4_BIN`.

## The FFI surface

Three classes: spec §11.3's `Session` and `Diagnostics` (`docs/CRPG_ENGINE_SPEC.md:1017`) plus
the §9.1 query object (`docs/CRPG_ENGINE_SPEC.md:861`). `Document`, `Assets` and `Preview` are
not in this slice; the editor is a later project. Class names are prefixed because GDExtension
class names are global in a project. Every member below runs on the Godot main thread; none is
thread-safe or callable from a worker.

| Class | Member | Signature | Thread |
|---|---|---|---|
| `CrpgSession` | `open` | `open(campaign_dir: String, encounter_id: String, seed: int, incarnation: int) -> int` | main |
| `CrpgSession` | `close` | `close() -> void` | main |
| `CrpgSession` | `poll` | `poll() -> int` (entries applied, or an error code) | main |
| `CrpgSession` | `view` | `view() -> CrpgView` | main |
| `CrpgSession` | `submit_declare_action` | `submit_declare_action(actor: int, ability: String, target: int) -> int` | main |
| `CrpgSession` | `submit_end_turn` | `submit_end_turn(actor: int) -> int` | main |
| `CrpgSession` | `server_tick` | `server_tick() -> int` | main |
| `CrpgSession` | signal | `delta_applied(count: int)` | main |
| `CrpgSession` | signal | `action_resolved(actor: int, target: int, ability: String, outcome: String, damage: int)` | main |
| `CrpgSession` | signal | `turn_started(actor: int, round: int)` | main |
| `CrpgSession` | signal | `encounter_ended(round: int)` | main |
| `CrpgSession` | signal | `failed(code: int, text: String)` | main |
| `CrpgSession` | signal | `closed()` | main |
| `CrpgView` | `entity_count` | `entity_count() -> int` | main |
| `CrpgView` | `net_ids` | `net_ids() -> PackedInt64Array` | main |
| `CrpgView` | `health` | `health() -> PackedInt32Array` | main |
| `CrpgView` | `max_health` | `max_health() -> PackedInt32Array` | main |
| `CrpgView` | `dead` | `dead() -> PackedByteArray` | main |
| `CrpgView` | `present` | `present() -> PackedByteArray` | main |
| `CrpgView` | `placements` | `placements() -> PackedStringArray` | main |
| `CrpgView` | `display_names` | `display_names() -> PackedStringArray` | main |
| `CrpgView` | `active` | `active() -> int` (NetId, or 0 for terminal) | main |
| `CrpgView` | `round` | `round() -> int` | main |
| `CrpgView` | `events` | `events() -> Array[Dictionary]` | main |
| `CrpgView` | `receipts` | `receipts() -> Array[Dictionary]` | main |
| `CrpgDiagnostics` | `last_code` | `last_code() -> int` | main |
| `CrpgDiagnostics` | `last_text` | `last_text() -> String` | main |
| `CrpgDiagnostics` | `count` | `count() -> int` | main |
| `CrpgDiagnostics` | signal | `reported(code: int, text: String)` | main |

Every `CrpgView` array is indexed in the order of `net_ids()`, so GDScript never zips two
differently ordered lists. `placements()` and `display_names()` come from the loaded campaign
(document `name` fields are locale keys — `crates/crpg-cli/src/demo.rs:108`), so the client
needs no second data load. `events()` and `receipts()` hold the frame's newly applied entries,
oldest first, and are cleared by the next `poll()`. Event keys are `kind`, `actor`, `target`,
`ability`, `outcome`, `damage`, `round`; receipt keys are `seq`, `applied`, `code`. `kind` is
`action_resolved`, `turn_started` or `encounter_ended` (`crates/crpg-net/src/protocol_v2.rs:92`,
`crates/crpg-net/src/protocol_v2.rs:107`, `crates/crpg-net/src/protocol_v2.rs:114`);
undisclosed fields are omitted, never zero-filled (`crates/crpg-net/src/protocol_v2.rs:82`).
`#[godot_api]`, `#[signal]` and the `Packed*Array` conversions are gdext 0.5.5 details **to
verify at T047**, as is whether registration uses `ExtensionLibrary` or the newer `Engine` path.

## Per-frame flow

```text
Godot main thread, once per rendered frame

_process(delta)
  |- Session.poll()                              [Rust, no Godot types]
  |    1. now_ms <- bridge clock (monotonic, from open)
  |    2. Host::pump(now_ms)                     crates/crpg-server/src/host.rs:954
  |    3. Host::take_delivery_entries(peer)      crates/crpg-server/src/host.rs:1695
  |         Delta    -> decode, apply to replica (T046)
  |         Transfer -> receive into replica (T046); fenced until T048's test
  |    4. Host::acknowledge_delivery(peer, last) crates/crpg-server/src/host.rs:1721
  |    5. rebuild CrpgView arrays + resolve names
  |    6. emit one signal per new event/receipt
  \- SceneSync._sync(Session.view())             [GDScript]
       spawn / update / free Node3D proxies by NetId; no gameplay logic
```

Input handlers call `Session.submit_*` on the same thread. A submit encodes and calls
`Host::ingest` immediately (`crates/crpg-server/src/host.rs:904`); the next `poll()` pumps it.
There is no second queue and therefore no second ordering. `_physics_process` is not used: the
authority has no physics step, and tying it to one would invent a tick the sim does not have.

## Error mapping

Codes are stable integers grouped by origin; `0` is success. The same code is returned and
emitted as `failed(code, text)`; `CrpgDiagnostics.last_text()` is for humans, not control flow.

| Origin | Code | GDScript action |
|---|---|---|
| `HostError::Closed` (`crates/crpg-server/src/host.rs:325`) | 10 | stop polling; show "closed" |
| `HostError::TimeRegression` (`:327`) | 11 | bug: never retry |
| `HostError::UnknownPeer` (`:329`) | 12 | bug: reopen |
| `HostError::ServerBusy` (`:331`) | 13 | bug: one peer only in the slice |
| `HostError::QueueFull` (`:333`) | 14 | retry next frame; nothing lost |
| `HostError::InvalidControl`/`InvalidGrants` (`:335`, `:337`) | 15/16 | bug: report |
| `HostError::PendingHistory` (`:339`) | 17 | bug: authority built wrong |
| `HostError::CounterExhausted` (`:341`) | 18 | terminal: reopen |
| `HostError::InvalidPageLimit`/`StaleCaptureCursor`/`FutureCaptureCursor` (`:343`, `:348`, `:355`) | 19/20/21 | not reachable in the slice |
| `HostError::FutureDeliveryCursor` (`:362`) | 22 | bug: ack bookkeeping |
| `HostError::Invariant` (`:369`) | 99 | terminal: report |
| any `CodecError` (`crates/crpg-net/src/codec.rs:52`) | 100 | bug: frame or intent shape |
| bridge lifecycle (not open, already open, unknown NetId) | 110/111/112 | usage error; no host call |
| `crpg-data` load or validation failure | 200 | show the diagnostic, do not start |
| no encounter / encounter start failure | 201/202 | show, do not start |
| panic caught at the boundary | 900 | terminal: report |

Gameplay rejection is `ReceiptStatus::Rejected(RejectionCode)`
(`crates/crpg-net/src/protocol.rs:378`) inside a delivery entry: no error code, no fence, and a
normal "that move is not legal" message. Mapping the 20 frozen codes
(`crates/crpg-net/src/protocol.rs:393`) to UI text is T051's job.

## Panic and unsafe policy

- `unsafe` stays confined to `crpg-godot`, which the lint enforces by skipping only that crate
  root (`tools/lint/deps.py:267`). T047 writes no `unsafe` block; gdext macros are the only
  producer. Any later block carries a `// SAFETY:` comment, and the crate enables
  `clippy::undocumented_unsafe_blocks` at `deny`.
- Every FFI entry point is a thin wrapper over a private, Godot-free `BridgeCore` method
  returning `Result<_, BridgeError>`. The wrapper does no indexing, `unwrap`, `expect`,
  `panic!` or unchecked arithmetic on values that came from GDScript.
- A panic must not unwind into Godot. The wrapper catches it, returns `900`, emits `failed` and
  fences the session. Whether the pinned gdext already catches at the boundary is **to verify
  at T047**; the wrapper catches regardless, so the answer changes code size, not the contract.
- No profile change (`panic = "abort"` would abort the editor).

## Build and CI

- `crates/crpg-godot/Cargo.toml` gains `[lib] crate-type = ["cdylib", "lib"]` (there is no
  `[lib]` today — `crates/crpg-godot/Cargo.toml:1`) and the optional `godot` pin from decision
  13 (`tasks/DECISIONS-2026-10-04.md:69`). No other manifest changes; the lint already allows
  `godot` only here (`tools/lint/deps.py:232`).
- Windows client build: `cargo build -p crpg-godot --features engine`; the resulting
  `crpg_godot.dll` is referenced by an `apps/client` `.gdextension` file. The exact file name,
  entry symbol and `.gdextension` keys are **to verify at T047**.
- The engine is the maintainer's installed Godot 4 build, recorded in T047's completion record
  and pinned per ADR-0001 (`tasks/DECISIONS-2026-10-04.md:70`,
  `docs/adr/0001-godot-pinned-not-forked.md:13`). The local build is
  `4.7.2.stable.official.ed1daf0bf`; the pinned binding is 0.5.5 (open question 3).
- Existing CI runs clippy and tests `--workspace` on Ubuntu and Windows
  (`.github/workflows/ci.yml:105`, `.github/workflows/ci.yml:157`,
  `.github/workflows/ci.yml:159`). With `engine` off those jobs compile and test the whole
  bridge core and need no Godot, so Linux headless stays Godot-free
  (`docs/CRPG_ENGINE_SPEC.md:1340`).
- T047 adds one Windows job, `bridge`: `--features engine`, clippy, the crate's tests and the
  client artifact for gate 12 (`docs/CRPG_ENGINE_SPEC.md:1325`). It is a real job, not a
  placeholder (`docs/CRPG_ENGINE_SPEC.md:1332`); the docs-only skip is unaffected
  (`.github/workflows/ci.yml:138`). `cargo deny` reads the manifest graph whether or not a
  feature is on, so the `godot` licence reaches the `deny` job (open question 1).

## Consequences

- **T047** opens `crates/crpg-godot`: `cdylib`+`lib`, the `engine` feature, the Godot-free
  `BridgeCore`, the three classes, campaign→authority construction, the embedded peer, proxy
  spawn from one bulk view, the `apps/client` project and the Windows build. It also writes
  `docs/architecture/crpg-godot.md` and `crates/crpg-godot/AGENTS.md`
  (`docs/architecture/README.md:58`), and as an S-set package needs human review
  (`docs/IMPLEMENTATION_PLAN.md:95`).
- **T048** owns proxy lifecycle: diff `net_ids()`/`present()` between frames, free on
  leave/despawn, and run the first test that reaches the `Transfer` arm by narrowing the peer's
  grants. It adds no FFI method.
- **T049** is blocked by the missing lane-1 movement channel
  (`crates/crpg-net/src/protocol.rs:305`); there is nothing to interpolate in this slice. When
  movement lands it adds `positions()` and `previous_positions()` to `CrpgView` — additive
  methods, which is why the view is an object and not a fixed tuple.
- **T051** consumes `events()`, `receipts()` and the signals for the HUD, combat log and
  `SimEvent`-driven VFX/audio (`docs/CRPG_ENGINE_SPEC.md:880`). The cut-down slice needs the
  option list (open question 2) and does not need T104/T108, so
  `docs/IMPLEMENTATION_PLAN.md:254`'s prerequisites no longer match the demo-first order and
  must be re-issued by the manager.
- No change to `crpg-server`, `crpg-net`, `crpg-sim` or `crpg-core`; no new package; no
  `ALLOWED` change; no golden, replay or hash change.

## Open questions (each with a recommendation)

1. **Does the `godot` crate's licence pass `deny.toml`?** The allow list is permissive only, and
   the file says another licence family is an ADR, not a one-line edit (`deny.toml:20`,
   `deny.toml:30`). *Recommendation:* verify the pinned crate's declared licence from
   `godot-0.5.5` itself; if it is MPL-2.0, accept it here as a file-level weak copyleft for one
   shipped dependency, record the obligation beside the other third-party notices
   (`docs/CRPG_ENGINE_SPEC.md:1774`), and amend `deny.toml` under that decision.
2. **Where does the client's option list come from?** `legal_actions` is sim-side and
   authoritative (`crates/crpg-sim/src/combat.rs:3177`; `crates/crpg-cli/src/play.rs:110`), and
   the wire has no option query. *Recommendation:* T051 adds one trusted host query returning
   the peer's permitted options as canonical intent bytes, exposed as
   `CrpgSession.legal_actions()`; GDScript only renders. Rejected: computing legality in the
   bridge (a second legality source) or inventing a v3 message now.
3. **Do `godot` 0.5.5 and Godot 4.7.2 agree?** gdext validates the engine against its bundled
   API. *Recommendation:* verify at T047 before any client code; if they disagree, the user
   chooses between a newer exact `godot` pin (a D27d amendment) and a different installed 4.x
   build. This ADR picks neither.
4. **How does a proxy know which art to use?** The wire discloses `NetId` and health only
   (`crates/crpg-net/src/protocol_transfer.rs:287`); no archetype or placement travels.
   *Recommendation:* the one-way table from decision 8, pinned by a T047 test against the
   documented minting order (`docs/architecture/crpg-server.md:161`). The durable answer is a
   disclosure field, which is T027b/T050 work; do not add one here.
5. **Does the pinned gdext catch panics at the boundary?** *Recommendation:* assume not, wrap
   every entry point, and delete the wrapper only if T047 proves the boundary already catches.
6. **Does the bridge own the campaign→`EncounterSpec` adapter?** The CLI has one
   (`crates/crpg-cli/src/combat_apply.rs:285`) and `crpg-godot` may not depend on `crpg-cli`.
   *Recommendation:* T047 writes a private copy behind one function, and a later task hoists it
   into `crpg-sim` (which already imports `crpg-data`) so the CLI and bridge share one mapping.
   Do not hoist it in T047: it would change a sim API and could move a CLI golden.

## Agent log

- 2026-11-01 (UTC) · opencode-go/deepseek-v4.1-flash + T045 draft · Drafted the bridge ADR from the fact sheet: a Godot-free `BridgeCore` under a thin gdext shell, main-thread serial host, byte-preserving ingest/pump/delivery, `NetId`-only FFI, closed error codes and an `engine` feature that keeps Linux headless free of Godot, with the licence, option-list and engine-version questions left explicit.

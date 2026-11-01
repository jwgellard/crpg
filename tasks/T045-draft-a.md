# ADR-T045a (draft): Godot bridge — FFI surface, threading, embedded host, error mapping, identity view

Date: 2026-11-01 (UTC)
Status: **Proposed draft** — blind draft for adjudication against an
independent draft. Nothing here is approved. Written from
[tasks/T045-facts.md](../tasks/T045-facts.md); every existing type or rule
cites `path:line`. Anything about the `godot` crate API that could not be
verified in this workspace is marked **to verify at T047**.

## Context

Spec §11.3 names the FFI rule: "The FFI surface (keep it small)"
(`docs/CRPG_ENGINE_SPEC.md:1008`), "roughly five object types"
(`docs/CRPG_ENGINE_SPEC.md:1015`): `Document`, `Session`, `Assets`,
`Diagnostics`, `Preview` with the verbs listed at
(`docs/CRPG_ENGINE_SPEC.md:1017`). "Everything else stays in Rust"
(`docs/CRPG_ENGINE_SPEC.md:1019`), which keeps the engine-swap escape
hatch real. The verbs are design intent "until their owning task pins
exact signatures" (`docs/CRPG_ENGINE_SPEC.md:1013`), and E022 owns the
exact host/editor/FFI interfaces (`tasks/E012-binary-crate-naming.md:81`).

Placement is settled: `crpg-server` owns the reusable platform-neutral
host as a library plus a thin dedicated binary, the embedded
single-player uses it "through a `crpg-godot` adapter", and client and
editor are Godot projects over `crpg-godot`, "never Rust packages"
(`docs/CRPG_ENGINE_SPEC.md:163`–`167`). The layer diagram puts
`crpg-godot [cdylib]` over the server library
(`docs/CRPG_ENGINE_SPEC.md:190`–`193`), with "pure Rust, no Godot types
below crpg-godot" (`docs/CRPG_ENGINE_SPEC.md:207`), because the adapter
imports the server library (`docs/CRPG_ENGINE_SPEC.md:218`–`219`).
`crpg-godot` owns engine-facing query/presentation objects; neither it
nor the client gets mutable authoritative access
(`docs/CRPG_ENGINE_SPEC.md:168`–`171`).

Standing order (user, 2026-11-01): T045 runs before the rest of M4, so
movement and pathfinding do not exist yet. The first consumer is a thin
slice — T047 skeleton plus embedded host, T048 proxies, a cut-down T051
(turn-based combat shown and played, as `crpgc play` does in the
terminal: build the world, `adapter.start`, list `legal_actions`,
`adapter.perform`, print through `Show`
(`crates/crpg-cli/src/play.rs:78`–`128`)). Placeholder primitives stand
in for art, so T105 does not gate it. The ADR must not paint the later
full client into a corner, and the client never mutates authoritative
state directly (`docs/CRPG_ENGINE_SPEC.md:1847`).

The spike evidence says bulk snapshots are the right pattern:
`CharacterHerd` with one bulk `snapshot()` per frame
(`docs/adr/0003-gdextension-rendering-spike.md:17`–`20`), 200 characters
at 231.7 fps with 87.4 µs FFI cost (`:37`), FFI cost linear at
~0.4 µs/character and never the bottleneck (`:48`–`49`).

## Decision

1. **The bridge is `crpg-godot` only; no new crate, no new package.**
E012 resolved "option a": client and editor are Godot projects
(`apps/client`, `apps/editor`) over the `crpg-godot` GDExtension crate,
"never Rust packages", and "there is no `crpg-client-bridge` crate"
(`tasks/E012-binary-crate-naming.md:65`–`69`). The `ALLOWED` table
already permits `crpg-godot` every workspace edge with no exclusion
(`tools/lint/deps.py:71`), so no lint change is needed. *Rejected: a
separate `crpg-client-bridge` crate* — a package, an `ALLOWED` row, and
a second FFI owner for no capability the cdylib cannot hold.
*Flag:* the plan rows say "`client/` project layout"
(`docs/IMPLEMENTATION_PLAN.md:247`) and "`client/` project"
(`docs/IMPLEMENTATION_PLAN.md:249`), but E012 resolved `apps/client`
(`tasks/E012-binary-crate-naming.md:65`), and the spec layer diagram
agrees (`docs/CRPG_ENGINE_SPEC.md:186`). The rows should read
`apps/client`.

2. **GDScript sees five classes, matching the §11.3 five — but the thin
slice opens with two.** The full surface mirrors the spec list
(`docs/CRPG_ENGINE_SPEC.md:1017`): `CrdDocument`, `CrdSession`,
`CrdAssets`, `CrdDiagnostics`, `CrdPreview`. T047 exposes only
`CrdSession` (connect/play/intent/query replica) and the read-only
query object it returns; the other four are stubs returning
`UNIMPLEMENTED` until their owning tasks pin them. *Rejected: exposing
the full five at T047* — four have no owning-task signatures yet
(`tasks/E012-binary-crate-naming.md:81`), and dead FFI is attack
surface plus H-unsafe review load.

3. **Exact FFI surface (thin slice; signatures pinned at T047, shapes
below are the draft).** All classes run on the Godot main thread; none
spawns a thread (decision 4). Godot-side spelling (`StringName`,
`signal` emit form, property syntax) is **to verify at T047** against
the pinned `godot` crate (D27d: exact pin, `crpg-godot` only,
`tasks/DECISIONS-2026-10-04.md:68`–`69`).
- `CrdSession`: `connect_embedded(seed: int, config: Dictionary)` (T047;
`connect_remote` comes later); `new_peer(grants) -> int`;
`submit_intent(peer, bytes) -> Dictionary` (`accepted`, `code`);
`poll() -> Dictionary`; `shutdown() -> void`.
- `CrdReplicaView` (read-only; the only mutation path is `apply_delta`
inside Rust, per the `ReplicaWorld` rule at
`docs/CRPG_ENGINE_SPEC.md:175`): `entity_count()`, `health_of(net_id)`
(`{health, max_health, dead}`), `turn()` (`{active, round}`),
`snapshot_bulk()` (current+previous transforms, spike pattern at
`docs/adr/0003-gdextension-rendering-spike.md:20`).
- Signals, main thread, deferred to frame end:
`combat_log(PackedStringArray)`, `turn_started(active, round)`,
`encounter_ended(encounter, round)`, `bridge_error(code, message)`.
- The client may compute predictions and display hints but "must
recompute nothing that determines an outcome"
(`docs/CRPG_ENGINE_SPEC.md:879`); UI reads the replica "through a
stable query API", never through Godot nodes
(`docs/CRPG_ENGINE_SPEC.md:881`).
*Rejected: mirroring `EntityId`, `World`, or history types in
GDScript* — raw arena ids are never serialized
(`crates/crpg-net/src/protocol.rs:209`), and GDScript holding
authority-shaped handles is how a view becomes a mutator.

4. **Threading: one thread (the Godot main thread) drives everything in
the embedded slice; time is injected.** The host is one serial owner
with no thread, socket or clock field
(`crates/crpg-server/src/host.rs:524`); construction, `bind_peer`,
`ingest` and `pump` each take an explicit `now_ms`
(`crates/crpg-server/src/host.rs:549`, `:691`, `:904`, `:954`), and an
earlier `now_ms` is rejected (`:679`, variant at `:327`). The adapter
"reads no clock, spawns no thread, opens no socket"
(`crates/crpg-server/src/quic.rs:21`); the socket, runtime and I/O
thread belong to the transport server, and `crpg-server` source
carries "no clock, thread, sleep or socket"
(`crates/crpg-server/AGENTS.md:69`). The bridge owns no thread either:
`_process` computes `now_ms` from the engine clock once per frame and
calls `pump` → `take_delivery_entries` → apply → present on the main
thread; the "Pump at most every 50 ms" cadence
(`crates/crpg-server/AGENTS.md:228`) is every frame at 60 fps.
*Rejected: a worker-thread pump* — locking around the serial `Host`
(`:524`) and a cross-thread handoff for zero cadence gain in the
turn-based slice; revisit only on a measured frame budget. The
real-transport adapter (`QuicHost::start` at
`crates/crpg-server/src/quic.rs:466`, per-pump `QuicHost::pump` at `:501`) and its I/O thread stay a dedicated-host concern.

5. **Embedded host path: `crpg-godot` adapter owns one `Host`, fed by an
in-memory byte pipe.** `Host::from_history(history, HostConfig
{ protocol, incarnation }, now_ms)` (`:556`), refusing a non-empty sim
journal (`:562`); the empty `Host::new` (`:549`) is not a playable
authority ("gameplay needs the `from_history` path"). Each local player
binds with `bind_peer` (`:691`); the snapshot-session kind
(`bind_snapshot_peer`, `:704`) is available from T047 but required only
when narrowing must avoid a fence — the slice binds lane-0. Intents
travel as bytes: the `Transport` trait is a bounded in-memory pipe "No
threads, no clock, no I/O" (`crates/crpg-net/src/transport.rs:11`–`14`),
with `InMemoryTransport` as the deterministic fabric
(`crates/crpg-net/src/sim.rs:385`–`396`) clocked only by `SimClock`
(`:49`). "The authority boundary is unchanged when transport is
in-memory" (`docs/CRPG_ENGINE_SPEC.md:177`); the client-facing
capability stays "a copyable `PeerHandle` plus the adapter's byte
channel" (`crates/crpg-server/src/lib.rs:29`), with no accessor
returning world state (`crates/crpg-server/AGENTS.md:44`; maps at
`crates/crpg-server/src/host.rs:456`). Trusted `tick()` (`:1950`,
"journals nothing") is called by the adapter, never by GDScript.
*Rejected: driving the embedded game through `crpgc play`'s direct
`World` path (`World::new`, `CombatAdapter::perform` at
`crates/crpg-cli/src/play.rs:80`, `:128`)* — that path has "no server
edge" (`crates/crpg-cli/Cargo.toml:7`) and bypasses the authority
boundary the product must enforce.

6. **`net_id`↔`EntityId` view: the map lives in the host; the bridge
holds `NetId`→proxy only.** Per peer the host keeps a live
`EntityId → NetId` map (`crates/crpg-server/src/host.rs:456`), with
`NetId` a "Client-scoped replica id": zero never valid, ids never
reused within one epoch (`crates/crpg-net/src/protocol.rs:209`–`217`).
The bridge keeps a `NetId → Node3D proxy` table plus health/turn facts
shaped like the snapshot body (`SnapshotEntity { entity: NetId,
health }`, `HealthFact`, `TurnFact` at
`crates/crpg-net/src/protocol_transfer.rs:287`, `:296`, `:307`).
`EntityId` never crosses the FFI; spawn reads the staged snapshot body
(mapped entities, health, turn when granted —
`docs/architecture/crpg-server.md:202`), and revoked ids retire
while re-granted entities get fresh ids (`:161`–`162`). V1 vocabulary:
`DeltaFrame { lane, server_tick, event_seq, ops }`
(`crates/crpg-net/src/protocol.rs:292`) with `EntityEnter`,
`EntityLeave`, `Health`, `Turn`, `Receipt` (`:309` ff.); there is "no
transform op in v1" (`:305`), so placement comes from snapshot facts
plus the bulk transform array. V2 event ops in `crates/crpg-net/src/protocol_v2.rs` (`ActionResolved`
at `:92`, `TurnStarted` at `:107`, `EncounterEnded` at `:114`) feed log and VFX hooks carrying
"only disclosable identities, symbolic outcomes, reported damage, and
widened rounds" (`:82`–`84`).
*Rejected: exposing the host's `EntityId → NetId` map to GDScript* —
leaked authority-shaped identity and stale-handle bugs across
revocation; the bridge re-resolves through delivery.

7. **Per-frame flow from `_process` through pump to proxies.**
(a) `_process`: `now_ms` from the engine clock; reject regression
(`crates/crpg-server/src/host.rs:679`). (b) Drain the intent outbox: `ingest(peer, bytes, now_ms)`
(`:904`), byte-level admission only. (c) `pump(now_ms)` (`:954`);
`admitted == applied + rejected` plus backpressure (`:292` ff.).
(d) `take_delivery_entries(peer)` (`:1695`) — `DeliveryEntry { ordinal,
kind, bytes }` (`:194`) — channel by `DeliveryKind::{Delta, Transfer}`
(`:185`), never by reading frame bytes. (e) Apply: v1 deltas update
query tables; snapshot `Begin`/`Chunk` rebuild spawn state.
(f) Acknowledge by ordinal (`:1721`; past-the-end is
`FutureDeliveryCursor` at `:362`). (g) Present: one bulk
`snapshot_bulk()` per frame, interpolate between the last two ticks
(`HerdMain.gd` pattern, `:22`–`24` in
`docs/adr/0003-gdextension-rendering-spike.md`), emit
`combat_log`/`turn_started`/`encounter_ended` from v2 events. No
gameplay logic in GDScript (`:26`).
*Rejected: `pump` per proxy or per event* — one pump per frame bounds
FFI crossings and keeps the serial host's FIFO order legible.

8. **Error mapping: every `HostError` crosses as (code, message); every
refusal is recoverable by retry or rebind, never by client repair.**
Refusals surface through `ReceiptStatus::{Applied,
Rejected(RejectionCode)}` (`:378` in
`crates/crpg-net/src/protocol.rs`), duplicates returning the terminal
receipt and never re-executing (`:375`); the rejection enum is closed
with frozen numbers (`:393`). Bridge mapping: `TimeRegression` (`:327`
in `crates/crpg-server/src/host.rs`) and `FutureDeliveryCursor`
(`:362`) are fatal `bridge_error` signals (clock/cursor bug — GDScript
cannot repair either); `SessionExpired` (fence path, `:790`) ends the
peer with a rebind prompt; `PendingHistory` (`:339`) fails
`connect_embedded` immediately; `NotSnapshotPeer` (`:237`) is a bridge
bug, asserted in debug, `bridge_error` in release. Shutdown is an
idempotent fence (`:1969`).
*Rejected: string-only errors* — GDScript needs stable codes to branch
on, and `Display` text is not a contract (any change needs an ADR,
`crates/crpg-server/AGENTS.md:19`).

9. **Panic and `unsafe` policy: `unsafe` lives in `crpg-godot` alone;
panics never cross the FFI.** "Only `crpg-godot` may use `unsafe`"
(`AGENTS.md:25`–`27`; spec exception at
`docs/CRPG_ENGINE_SPEC.md:1103`); every other crate root carries
`#![forbid(unsafe_code)]`, enforced by lint
(`tools/lint/deps.py:16`–`17`), with `crpg-godot` the single skip
(`:75`, `:279`) and the only crate allowed the `godot` dependency
(`:12`, check at `:233`, violation text at `:245`). The crate root
already declares the exception (`#![allow(unsafe_code)]` at
`crates/crpg-godot/src/lib.rs:1`, intent at `:3`–`4`). All `unsafe`
blocks need H-unsafe review (plan gate at
`docs/IMPLEMENTATION_PLAN.md:249`). Panic behavior at the
`godot`-crate boundary (unwind vs. abort, logging) is **to verify at
T047**; the draft requires a Rust panic in a GDScript-called method to
become a `bridge_error` or a controlled abort, never an unwind into
the engine. `crpg-godot` is outside the determinism lint's scope
(`tools/lint/determinism.py:57`), but the bridge still adds no float
or hash-order nondeterminism: transforms are presentation copies,
never rule inputs.
*Rejected: `unsafe` anywhere else* — the lint and the spec exception
both forbid it.

10. **Build and CI: Windows builds the client; Linux headless never
touches Godot.** Gate 12 builds "Windows client + editor + server and
Linux headless server artifacts" (`docs/CRPG_ENGINE_SPEC.md:1325`);
gates 7–13 are capability-gated, "not placeholder jobs" (`:1332`), and
"Linux headless gates must not depend on Godot" (`:1340`). So the
`crpg-godot` cdylib and `apps/client` build and run only on Windows;
no Linux CI job builds them. Workspace gates stay `cargo fmt`,
`clippy --workspace --all-targets --locked`, `test --workspace
--locked` on both OSes (`.github/workflows/ci.yml:150`, `:157`,
`:159`), with the docs-only PR skip (`.github/workflows/ci.yml:138`). The GDExtension
boundary is in the serialized human-reviewed set
(`docs/IMPLEMENTATION_PLAN.md:95`–`98`), and T047 "Opens the crate"
with H-unsafe (`:249`); Godot 4.7.2 is the installed engine (verified
`Godot4 --version` on 2026-11-01). The only dated `godot`-crate pin
authority is D27d (`tasks/DECISIONS-2026-10-04.md:68`–`72`), so T047
re-confirms both versions in its completion record. *Rejected:
building the client on Linux CI* — ADR-0012 promises no Linux GUI
builds (`docs/adr/0012-windows-primary-platform.md:23`).

## Consequences

- **T047** (skeleton, `docs/IMPLEMENTATION_PLAN.md:249`): opens
`crpg-godot` with the exact pin (D27d), `CrdSession`/`CrdReplicaView`,
the embedded host adapter (decision 5), one bulk `snapshot_bulk()` per
frame, proxy spawn "from a bulk snapshot" (§19.1 #22), the Windows
client build for gate 12, plus the crate's architecture doc and
`AGENTS.md` (first-code rule, `AGENTS.md:59`–`61`) and the installed
engine version per D27d and ADR-0001's pin rule
(`docs/adr/0001-godot-pinned-not-forked.md:13`).
- **T048** (scene sync, `:250`): the `NetId`→proxy lifecycle on decision
6 — spawn from snapshot bodies and `EntityEnter`, retire on
`EntityLeave`/revocation; narrow-by-transfer when the snapshot kind is
adopted.
- **T049** (interpolation, `:251`): the buffer between the last two bulk
snapshots; the spike's previous+current pair (`:20` in
`docs/adr/0003-gdextension-rendering-spike.md`) is the input contract,
and the buffer lives outside the sim, never as a mutable replica
(`docs/CRPG_ENGINE_SPEC.md:175`).
- **T051 cut-down** (`docs/IMPLEMENTATION_PLAN.md:254`): HUD, combat log, `SimEvent`-driven VFX/audio
hooks fed by v2 events (`:92`–`114` in
`crates/crpg-net/src/protocol_v2.rs`) and the replay-recorded event
stream (`docs/CRPG_ENGINE_SPEC.md:785`). Full T051 waits on T048, T104,
T108; the slice needs turn/health display, `legal_actions` buttons, log.
- Nothing here changes the wire, the host API, goldens, or any edge:
`crpg-godot` may already import `crpg-data`, `crpg-net`, `crpg-server`
(`tools/lint/deps.py:71`).

## Open questions (each with a recommendation)

1. **Exact `godot`-crate class registration and signal syntax?**
Unverified here: no `godot` package in `Cargo.lock` (verified by
search), no `.gdextension` file in the tree (verified by glob).
*Recommendation:* T047 verifies against the pinned crate docs and
records the pattern in the crate's `AGENTS.md`; this ADR's method
names are intent until then.
2. **Does the thin slice need `bind_snapshot_peer` or is lane-0
enough?** Narrowing lane-0 resets delivery
(`docs/adr/0031-host-snapshot-sender-and-snapshot-sessions.md:44`–`55`),
snapshot sessions narrow by transfer. *Recommendation:* lane-0 for
T047; adopt the snapshot kind when grant narrowing (T027b/c) arrives.
3. **Where does `now_ms` come from — engine ticks or wall clock?**
The host needs only monotonic non-regressing input (`:679` in
`crates/crpg-server/src/host.rs`). *Recommendation:* engine time in
milliseconds per frame; wall clock only if the engine clock can stall
while the host must advance.
4. **Save/load in the embedded slice now or with T040?**
`save_host_file`/`load_host_file` with `SaveIdentity` exist
(`:272`, `:287`, `:89`–`:98` in `crates/crpg-server/src/save.rs`),
but slot policy and the incarnation source belong to T040.
*Recommendation:* debug-menu action only, caller-held incarnation
counter; full policy waits for T040.
5. **`connect_remote` (QUIC net client in `crpg-godot`) in T047 or
later?** The spec's Rust-side list includes a "net client (quinn)"
(`docs/CRPG_ENGINE_SPEC.md:857`), but the thin slice is embedded
only. *Recommendation:* later; `CrdSession` reserves the name so
remote connect does not reshape the FFI.

## Agent log

- 2026-11-01 (UTC) · opencode-go/muse-spark-1.3-contributor + T045 draft · Drafted the blind bridge ADR from the T045 facts sheet with verified path:line citations, leaving godot-crate API details to T047 verification and flagging the plan's `client/` path against E012's `apps/client`.

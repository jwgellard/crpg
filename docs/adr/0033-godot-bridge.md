# ADR-0033: The Godot bridge: FFI surface, threading and the embedded host

Date: 2026-11-01 (UTC)
Status: **Proposed** — filed by T045 for review; nothing here is approved or
implemented. Acceptance is the user's (plan wave 7, "Accept the bridge ADR").

## Context

Spec §11.3 keeps the Rust↔GDScript surface to roughly five object types and
says everything else stays in Rust. E012 settled the packaging: client and
editor are Godot projects (`apps/client`, `apps/editor`) over the `crpg-godot`
crate, there is no bridge package, `crpg-server` owns the host library, and
`crpg-net` owns replica storage while `crpg-godot` owns the engine-facing query
objects. The E022 ledger left the FFI shapes to the owning task, which is this
one. ADR-0003 showed one bulk call per frame is the right crossing pattern.

`crpg-godot` is empty today (empty `[dependencies]`, a four-line root). The
host is one serial owner of a private `HistoryWorld`, takes `now_ms` on every
timed call, and has no clock, thread or socket
(`crates/crpg-server/src/host.rs:524`, `crates/crpg-server/AGENTS.md:69`).

By the user's demo-first order (2026-11-01) this runs before the rest of M4.
Movement and pathfinding do not exist. The first consumer is a thin slice:
T047 skeleton with embedded host, T048 proxies, a cut-down T051, playing
turn-based combat as `crpgc play` does in the terminal, on placeholder
primitives. This ADR must not paint the later full client into a corner.

Four source facts shape the result, and neither blind draft handled all four:

1. **`crpgc play` is not a model for the authority path.** It owns a `World`
   and calls `crpg_sim::perform_action` directly, with no server edge
   (`crates/crpg-cli/src/play.rs:128`, `crates/crpg-cli/Cargo.toml:7`). The
   client must not do that.
2. **The wire has no option list and the host has no option query.**
   `crpg_sim::legal_actions` needs a `&World` (`crates/crpg-sim/src/combat.rs:3177`)
   and no accessor returns one (`crates/crpg-server/AGENTS.md:43`). So the
   client cannot learn what `crpgc play` prints as "Options" without a
   `crpg-server` change.
3. **Nothing plays the other side.** `crpgc play` picks enemy actions in the
   terminal loop (`crates/crpg-cli/src/play.rs:210`). `crpg-ai` is an empty
   crate, and neither the host nor the wire has a driver.
4. **The wire says who, never what.** A `NetId` carries no archetype, name,
   side or position, and there is no transform op
   (`crates/crpg-net/src/protocol.rs:305`).

## Decision

### 1. One crate, three parts

The bridge is `crpg-godot` only: no new package, no `ALLOWED` change
(`tools/lint/deps.py:71` already permits every workspace edge).

```text
crates/crpg-godot/src/
  lib.rs                  crate root; keeps #![allow(unsafe_code)]; declares core, shell
  core/mod.rs             #![forbid(unsafe_code)]; BridgeCore; no godot types; always compiled
  core/authority.rs       trusted half: owns Host, campaign, NPC driver. May use crpg_server, crpg_sim
  core/client.rs          untrusted half: replica, intent encoding, view building. Bytes only
  core/link.rs            LocalLink: the in-memory transport between the two halves
  core/errors.rs          BridgeCode table (section 11)
  shell.rs                #[cfg(feature = "engine")]; gdext classes; conversion and guard only
```

`client.rs` names neither `crpg_server` nor `crpg_sim` (BR-10). That is the
compile-visible form of "the client cannot mutate authoritative state".

### 2. The embedded authority is built once, then wrapped

`BridgeCore::open` does this, in order, on the calling thread:

1. Read the campaign directory (an OS path, not `res://`) into
   `BTreeMap<SourcePath, Vec<u8>>`: regular files only, no symlink following,
   classified by `crpg_data::campaign_document_path`
   (`crates/crpg-data/src/validation.rs:261`). This is the bridge's only
   filesystem I/O; `crpg-data` does none.
2. `crpg_data::load_campaign`, then `crpg_data::validate`; any finding stops.
3. Assemble `EncounterSpec` from the documents. This is a private copy of the
   assembly in `crates/crpg-cli/src/combat_apply.rs:211`, because `crpg-cli`
   has no library target. A follow-on hoists it (Consequences).
4. `HistoryWorld::new(seed)`, then `start_encounter(&spec)`, turn-based only.
5. Record `(EntityId, placement)` from `history.world().combatants()`, and
   `history.acknowledge(history.last_sequence())`, because `Host::from_history`
   refuses a non-empty journal (`crates/crpg-server/src/host.rs:561`).
6. `Host::from_history(history, HostConfig { protocol: V2, incarnation: 1 }, 0)`.
7. Bind two peers with full disclosure for this encounter: the player with
   `bind_snapshot_peer` (decision 8), controlling the chosen side, and the NPC
   driver with `bind_peer`, controlling every other combatant (decision 7).
8. Build the presentation table (decision 9).

`V2` is required: the combat log needs the event ops. `incarnation` is `1`
because every `open` starts a new authority and saves are cut. GDScript chooses
none of these. The start events are acknowledged before the wrap, so the first
turn reaches the client as `Turn` state, not as a `TurnStarted` event.

### 3. The in-memory transport is `LocalLink`

`LocalLink` is a pair of bounded in-process FIFO byte queues implementing
`crpg_net::transport::Transport` (`crates/crpg-net/src/transport.rs:14`), each
direction capped at 128 frames, the T022 per-peer log cap. `QuicClient` already
implements the same trait (`crates/crpg-net-quic/src/client.rs:503`), so a later
remote client swaps the client half's transport without reshaping the FFI.

- The client half holds one end and never a `Host` or `PeerHandle`.
- The authority half holds the other end, and feeds `Host::ingest`.
- Downstream it follows the QUIC adapter's rule: hand off, then acknowledge by
  the last accepted ordinal, and never acknowledge what the link refused
  (`crates/crpg-server/AGENTS.md:166`, `:178`). The client never acknowledges.
- The link carries lane-0 frames only. It needs no channel tag because a
  `DeliveryKind::Transfer` entry is a fatal fence in this slice (decision 8).

### 4. One thread

Everything in sections 1–3 runs on the Godot main thread. The bridge spawns no
thread and has no `Send` type at the FFI. The `godot` crate's
`experimental-threads` feature stays off.

### 5. Time

`shell.rs` reads a monotonic `std::time::Instant` taken at `open` and passes
elapsed milliseconds to `BridgeCore::poll(now_ms)`. No FFI method takes a
timestamp, so GDScript cannot cause `TimeRegression`, and tests script the
clock. The bridge pumps once per `poll()`, with no throttle: the "at most every
50 ms" note concerns QUIC hello decisions (`crates/crpg-server/AGENTS.md:226`),
which do not exist here. `Host::tick` is never called: `crpgc play` needs no
tick, the slice has no movement, and `tick` journals nothing
(`crates/crpg-server/src/host.rs:1950`). `_physics_process` is not used.

### 6. Per-frame flow

GDScript calls `session.poll()` exactly once per `_process`. `poll` does:

```text
Godot main thread, once per rendered frame
Main.gd _process(delta)
  session.poll()
    shell: now_ms <- Instant; enter panic guard
    core.authority.step(now_ms)
      1. recv from link -> Host::ingest(player, bytes, now_ms)
            Refused => report INTENT_REFUSED (no reply exists; clears pending)
      2. NPC step (decision 7): at most one Host::ingest(npc, ...)
      3. Host::pump(now_ms)
            backpressured => not an error; retried next poll
            npc result Rejected => fatal HOST_FAILED
            dropped_replies > 0 => report INTENT_REFUSED
      4. per peer: Host::take_delivery_entries
            player: Delta -> link.send; Transfer -> fatal UNEXPECTED_TRANSFER
            Host::acknowledge_delivery(peer, last ordinal handed off)
            npc: acknowledge the last ordinal, unread
      5. Host::acknowledge_captures(Host::last_capture())
      6. Host::legal_intents(player) -> report.options
    core.client.step(report)
      7. recv from link -> codec_v2::decode_delta -> apply to replica (T046)
         collect event ops and receipts in frame order
      8. if anything changed: revision += 1; rebuild CrpgView
  v = session.view()
  if v.revision() != seen: SceneSync.sync(v); Hud.sync(v)
```

Step 5 is required. Unacknowledged captures cap admission at 4,096 records, and
admission then stops with `backpressured: 1`
(`crates/crpg-server/AGENTS.md:219`). The slice keeps no capture consumer.

Step 6's `report.options` is the one thing that crosses the halves outside the
link. It is one-way, by value, and confers nothing: execution revalidates. It
exists because the wire has no option message.

`submit_option` only writes to the link. It reaches the host on the next
`poll`, so there is one queue and one ordering.

### 7. Options, identity and NPC turns need a small host read API (T047a)

A new `crpg-server` package, here **T047a**, comes before T047, because one task
is one crate. It adds two read-only trusted methods. `crates/crpg-server/AGENTS.md:19`
requires an ADR for any public change, and this is that ADR.

```rust
/// One action `peer` may request now, in that peer's replica ids.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegalIntent {
    pub actor: NetId,
    pub body: IntentBody,
}

impl Host {
    pub fn legal_intents(&self, peer: PeerHandle) -> Result<Vec<LegalIntent>, HostError>;
    pub fn replica_id(&self, peer: PeerHandle, entity: EntityId)
        -> Result<Option<NetId>, HostError>;
}
```

- **L1.** Both: `Closed` after shutdown, checked first, then `UnknownPeer`.
- **L2.** `legal_intents` with no active head (no encounter, or terminal) is `Ok(vec![])`.
- **L3.** The active entity not in the peer's control, not in its map, or
  lacking `present && action_actor` is `Ok(vec![])`.
- **L4.** Otherwise it calls `crpg_sim::legal_actions(world, active)` once and
  preserves the returned order. `Actor(_)` is `Ok(vec![])`.
  `TooManyOptions` is `HostError::Invariant("legal options exceed their bound")`.
- **L5.** An option is kept only if `resolve_action`'s mapping, ownership and
  disclosure rules accept it (`crates/crpg-server/src/host.rs:2230`), so a kept
  option cannot draw `NotAuthorized` at that instant. `Stop` has no wire verb
  and is omitted.
- **L6.** Both are `&self` and touch no counter, ledger, rate token or clock;
  `authority_hash` is unchanged. Neither is reachable from a client object.

This adds no second legality source: `crpg_sim` still decides, per ADR-0018, and
options "confer no authorization".

**NPC driver.** It lives in `core/authority.rs`, on the trusted side, and goes
through the same `ingest` path as a player, on its own peer. In `step`:

- If `legal_intents(npc)` is non-empty and no intent is due, due time becomes
  `now_ms + NPC_ACTION_DELAY_MS`, with `NPC_ACTION_DELAY_MS: u64 = 600`, so a
  person can watch each enemy action. The delay is presentation pacing: time
  never enters the sim, and the sequence of actions alone decides the outcome.
- At `now_ms >= due`, it submits one intent chosen by `crpgc play`'s rule
  (`crates/crpg-cli/src/play.rs:210`): the first `DeclareAction` whose target
  is on another side, else the first option. `legal_actions` lists live targets
  only, so no health read is needed. It reads sides from the presentation table.
- Submitting clears the due time. `observed_tick` is `Host::server_tick()`, and
  `seq` is the driver's own counter.

When real AI arrives the driver is replaced behind the host. The FFI does not
change.

### 8. Session kind

The player binds with `bind_snapshot_peer`: same first delivery as `bind_peer`,
and the session kind `/2` QUIC peers get, so the embedded path exercises the
remote shape. Grants never narrow in the slice and nothing sends `Resync`, so a
`Transfer` entry is unreachable. It fences as `UNEXPECTED_TRANSFER` rather than
being silently dropped. The first task that narrows a live session wires
`DeliveryKind::Transfer` through a channel tag on the link and T046's receive path.

### 9. The `NetId`↔`EntityId` view

`EntityId` never crosses into the client half or the FFI; every id GDScript sees
is a `NetId`. The host's live map stays the single source of truth.

At `open`, the authority half builds a `PresentationTable`:
`BTreeMap<NetId, { placement: Ulid, name: String, side: String }>` from the
`(EntityId, placement)` list of decision 2, `Host::replica_id(player, entity)`,
and the campaign's locale, creature and faction documents (a private copy of
`crates/crpg-cli/src/demo.rs:217`'s rule). It hands the table to the client half
by value. The NPC driver keeps its own table keyed by the NPC peer's ids, since
ids are per peer. The table also carries ability display names by `Ulid`.

This is local presentation data, not disclosure: it works because single player
has the whole campaign on disk. A remote client needs a disclosed presentation
field (T027b/T140 territory). The FFI shape does not change when it arrives.

### 10. The FFI surface

Two classes, both `RefCounted`, both prefixed because GDExtension class names
are global. Spec §11.3's `Document`, `Assets`, `Preview` and `Diagnostics` wait
for their owning tasks; stubs are not added. `Session` covers "connect / play /
intent / query replica"; `CrpgView` is the §9.1 query API. There are **no
signals** in the slice: `poll()` then `view()` gives one explicit order, and a
signal fired mid-`poll` would race the view it describes. Signals are additive.

Every member runs on the Godot main thread and is not callable from a worker.

| Class | Member | Meaning |
|---|---|---|
| `CrpgSession` | `open(campaign_dir: String, encounter_id: String, player_side: String, seed: int) -> int` | Decision 2. Empty `encounter_id` means the first encounter in document order; empty `player_side` means the first combatant's side, as `crpgc play` does. Returns a code |
| | `poll() -> int` | Section 6. Returns `0` or a code; a fatal code repeats until `close` |
| | `view() -> CrpgView` | The view from the last changing `poll`; an empty view with revision 0 before that |
| | `submit_option(revision: int, index: int) -> int` | Sends the option at `index` of the view with that `revision`. Returns a code |
| | `close() -> void` | `Host::shutdown`, drop the core. Idempotent; also runs on drop. `open` may follow |
| | `last_error() -> String` | Human text for the last non-zero code, or empty. Never control flow |
| `CrpgView` | `revision() -> int` | Increases when state, options, events or receipts change |
| | `net_ids() -> PackedInt64Array` | Visible entities, ascending. Every per-entity array below is indexed the same way |
| | `names() -> PackedStringArray` | Display name, from the presentation table |
| | `sides() -> PackedStringArray` | Display side |
| | `placements() -> PackedStringArray` | Placement ULID text, the later key for art |
| | `health() -> PackedInt64Array` | Current health; `0` when `health_known` is false |
| | `max_health() -> PackedInt64Array` | Maximum health; same rule |
| | `health_known() -> PackedByteArray` | `1` where health is disclosed. Undisclosed is never presented as zero |
| | `dead() -> PackedByteArray` | Terminal flag, `0` where unknown |
| | `active() -> int` | Active `NetId`, or `0` for none |
| | `round() -> int` | The wire round, unadjusted. The HUD adds 1, as `crpgc play` does |
| | `ended() -> bool` | An `EncounterEnded` event was applied |
| | `option_labels() -> PackedStringArray` | `"<ability> -> <target>"` or `"End turn"`, the `crpgc play` wording |
| | `option_targets() -> PackedInt64Array` | Target `NetId` per option, `0` for End turn, so clicking a proxy can find its option later |
| | `events() -> Array` of dictionaries | Events that produced this revision, oldest first |
| | `receipts() -> Array` of dictionaries | Receipts that produced this revision, oldest first |

Empty option arrays mean it is not the player's turn.

`events()` entries always have `kind`:

- `action_resolved`: `actor`, `target`, `ability` (display name), `ability_id`, `outcome`, `damage`
- `turn_started`: `actor`, `round`
- `died`: `entity`
- `encounter_ended`: `encounter`, `round`

`outcome` is the wire's symbolic text. `receipts()` entries are `seq`, `applied`
(bool) and `code` (the frozen `RejectionCode` number, or `0`). Rendering events,
not diffing health, keeps spec §9.2 rule 2.

Health arrays are 64-bit because the wire's `u32` does not fit a 32-bit Godot
integer. GDScript `int` values are `i64`; a negative or out-of-range argument
is `BAD_ARGUMENT` with no core call.

**Submission is by option index only.** GDScript cannot name an ability, target
or actor, and never sees wire bytes, a seq or an epoch. A generic `submit(bytes)`
and typed `submit_declare_action` are both rejected: the first puts encoding in
GDScript, the second invites client-side legality. `submit_option` checks, in
order: open, `revision` matches the current view, `index` in range, no intent
pending. The client half then builds the `IntentFrame` with the epoch given at
open, its own seq, and `observed_tick` from the last frame's `server_tick`, and
encodes with `codec_v2::encode_intent`. One intent may be outstanding; the
receipt or an `INTENT_REFUSED` report clears it.

A gameplay rejection is a receipt, not an error, and `poll` still returns `0`.

### 11. Error mapping

Codes are stable integers in one closed table, `0` being success. T047 pins them
in `core/errors.rs` and tests them. Enum variants map to these groups, not one
code each, because GDScript's action is the same inside a group, and `CodecError`
and `HostError` order is no promise. The variant's text goes in `last_error()`.

| Code | Name | Source | Session | GDScript action |
|---|---|---|---|---|
| 0 | `OK` | | unchanged | |
| 1 | `NOT_OPEN` | call before `open` | unchanged | caller bug |
| 2 | `ALREADY_OPEN` | second `open` | unchanged | caller bug |
| 3 | `BAD_ARGUMENT` | negative or oversized `int`, bad ULID text | unchanged | caller bug |
| 4 | `STALE_OPTIONS` | `revision` is not the current one | unchanged | re-read the view |
| 5 | `INTENT_PENDING` | an intent is outstanding | unchanged | wait |
| 6 | `NO_SUCH_OPTION` | `index` out of range | unchanged | caller bug |
| 20 | `CAMPAIGN_IO` | directory read failed | not opened | show text |
| 21 | `CAMPAIGN_INVALID` | `load_campaign` error or validation finding | not opened | show text |
| 22 | `NO_ENCOUNTER` | campaign has none | not opened | show text |
| 23 | `UNKNOWN_ENCOUNTER` | `encounter_id` unknown | not opened | show text |
| 24 | `UNKNOWN_SIDE` | `player_side` unknown | not opened | show text |
| 25 | `START_FAILED` | `start_encounter`, `from_history` or bind failed | not opened | show text |
| 30 | `INTENT_REFUSED` | `IngestDisposition::Refused`, a full link, or a dropped reply | open | allow another choice |
| 50 | `HOST_CLOSED` | `HostError::Closed` | fatal | stop; show text |
| 51 | `HOST_FAILED` | any other `HostError`, or a rejected NPC intent | fatal | stop; show text |
| 52 | `FRAME_UNDECODABLE` | `CodecError` on a delivered frame | fatal | stop; show text |
| 53 | `REPLICA_REJECTED` | T046 refused a frame | fatal | stop; show text |
| 54 | `UNEXPECTED_TRANSFER` | `DeliveryKind::Transfer` in the slice | fatal | stop; show text |
| 55 | `PANICKED` | panic caught by the guard | fatal | stop; show text |

Fatal means: the core calls `Host::shutdown`, drops the host, and every later
`poll` returns the same code until `close`. A bad frame is never skipped: the
replica would silently diverge. Mapping `RejectionCode` numbers to player text is
T051's job.

### 12. Panic and `unsafe` policy

- **`unsafe`.** Exactly one `unsafe` token in hand-written source: the entry
  point `unsafe impl ExtensionLibrary` in `shell.rs`, which the crate requires
  (`godot-core-0.5.5/src/init/mod.rs:557`). It carries a `// SAFETY:` comment and
  is the H-unsafe review item. `core/mod.rs` carries `#![forbid(unsafe_code)]`.
  The root stays `#![allow(unsafe_code)]` for macro expansion. Any further
  `unsafe` needs an amendment to this ADR.
- **Layering.** Every `#[func]` is conversion plus one guarded `core` call. The
  shell does no indexing, `unwrap`, `expect` or arithmetic on GDScript values.
- **Panics.** Verified in the pinned source: `handle_fallible_varcall` and
  `handle_fallible_ptrcall` catch unwinds at the boundary
  (`godot-core-0.5.5/src/private.rs:465`, `:484`). That protects the engine, but
  GDScript then just aborts its function and our session stays live and
  possibly half-updated. So every call also goes through one
  `guard(|core| ...)` using `std::panic::catch_unwind`: on a panic it drops the
  core, records `PANICKED`, and returns `55`. No authority survives a panic.
- **Profile.** No `panic = "abort"`: it would take the editor down, and it would
  change every crate's build.

`crpg-godot` stays outside `determinism.py`. It reads no rule from presentation
values, and the slice needs no float.

### 13. Manifest and pins

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

- **Binding.** `godot` `=0.5.5` is the newest version on the crates.io index on
  2026-11-01, D27d's rule. T047 re-checks at contract time.
- **Engine.** `4.7.2.stable.official.ed1daf0bf`, installed and recorded per D27d
  and ADR-0001. The crate ships `api-4-7` over the prebuilt API
  `gdextension-api-0.5.1/src/4.7/extension_api.json` (4.7.0 stable, single
  precision). Only `api-custom` pulls in `which`, so the build needs no Godot
  binary and no `GODOT4_BIN`. An exact pin plus `api-4-7` is therefore compatible
  on paper; the first load on 4.7.2 proves it.
- **`engine` is off by default.** `cargo clippy --workspace` and
  `cargo test --workspace` compile and test the whole of `core` on both targets
  with no Godot code. Linux headless stays Godot-free, and every behaviour except
  the shell gets both-target coverage. `Cargo.lock` lists the `godot` packages
  either way, which is a lock entry, not a build input.

### 14. `apps/client` layout

```text
apps/client/
  project.godot
  crpg.gdextension       Windows x86_64 debug and release only
  .gitignore             .godot/ and exports
  scenes/Main.tscn       camera, ground plane, HUD root
  scripts/Main.gd        open, poll once per _process, input
  scripts/SceneSync.gd   NetId -> placeholder primitive proxies; no gameplay logic
  scripts/CombatHud.gd   turn, health, option buttons, log
  tests/smoke.gd         headless playthrough (BR-22)
```

Plan rows T045 and T047 say `client/`; E012 and spec §14 say `apps/client`, which
wins. The DLL is loaded from the workspace `target/` directory; nothing is copied
in. Campaign path for the slice: `campaigns/fixtures/combat_basic` through
`ProjectSettings.globalize_path`. Proxy layout is client-only (one row per side),
because the wire has no position. It decides nothing.

### 15. Windows build and CI

Local: `cargo build -p crpg-godot --features engine --locked -j 2`, then
`Godot4 --path apps/client`. Keep `-j 2`: commit memory is tight on this machine,
and full `godot` codegen is heavy.

`.github/workflows/`, `deny.toml` and the rulesets are S-set. T047 changes none of
them. A separate S package, here **T047c**, must do exactly this, and must land in
T047's batch, because if `deny` does see the engine graph, T047 alone fails it:

- **C1.** Add a Windows-only job `client`, reporting on every run with the same
  docs-only classification as `check`, that runs:
  `cargo clippy -p crpg-godot --features engine --all-targets --locked -- -D warnings`,
  `cargo build -p crpg-godot --features engine --locked`,
  `--headless --path apps/client --import`, and BR-22.
- **C2.** Fetch Godot 4.7.2 at the exact version with a pinned SHA-256, no
  floating download.
- **C3.** Upload `crpg_godot.dll` plus `apps/client` as the gate 12 slice artifact.
- **C4.** On Linux and Windows `check`, prove BR-21: the default-feature graph has
  no `godot` package. No Linux job ever enables `engine`.
- **C5.** Make `cargo deny` see the engine graph. `deny.toml` has no `[graph]`
  section, so it probably resolves default features only and may never look at
  `godot`. The task proves coverage with a negative control: remove the licence
  exception and watch `deny` fail, on `licenses`.
- **C6.** Add the per-crate licence exceptions, if the user accepts decision U1.
- **C7.** The user adds `client` to the master ruleset's required checks.

The existing `check`, `deny` and lint jobs are otherwise unchanged. `apps/` is
outside the docs-only allowlist, so a client change runs every gate.

## Authority and affected surface

- Decided by: not yet decided; awaiting the user.
- Authoritative contract: this ADR for the shapes; `tasks/T047a.md`, `tasks/T047.md`
  and `tasks/T047c.md`, once written, for exact signatures.
- Owning crates/files: `crates/crpg-godot`, `apps/client`; `crates/crpg-server`
  for T047a; `deny.toml` and `.github/workflows/ci.yml` for T047c.

## Compatibility, hash and dependency impact

- Persisted/serialized shapes: unchanged. No wire, save or checkpoint change.
- Replay, `state_hash`, goldens: unchanged. BR-3 proves the bridge changes no outcome.
- Dependency graph: `ALLOWED` unchanged. External: `godot` `=0.5.5` under D27d,
  subject to U1, plus its tree (`glam` 0.32, `venial`, `litrs` and others), audited
  T018a-style in T047's completion record.
- Platform scope (ADR-0012): `engine` and `apps/client` are Windows only. `core` is
  platform-neutral and tested on both targets. The DLL bundles `crpg-server`, so it
  also bundles the QUIC tree, which the remote client will need anyway.

## Alternatives considered

- **Call `crpg_sim` from the bridge, as the CLI does.** Rejected: it makes the
  client the decider and bypasses the host.
- **Call `Host` directly from one struct, with no link.** Rejected: bytes would
  still flow, but nothing would stop a later edit from reaching into the host.
- **Use `crpg_net::sim::InMemoryTransport`.** Rejected: that is the fault-injecting
  test fabric with its own `SimClock`.
- **A worker thread owning the host.** Rejected: double buffering, a second clock
  and cross-thread handoff for no gain in a turn-based slice.
- **A fixed 20 Hz authority tick, as in the spike.** Rejected: nothing advances on
  a tick here.
- **Build options in the client from campaign data and replica state.** Rejected: a
  second legality source, and it would offer unaffordable actions.
- **Infer `NetId → placement` from ascending-`EntityId` minting order.** Rejected:
  it pins an implementation order that nothing promises, when a read of the host's
  own map is one line.
- **A new protocol message for options now.** Rejected: T140 owns the next protocol
  version, and an embedded slice does not need it.
- **Depend on `godot` unconditionally.** Rejected: `cargo test --workspace` runs on
  Linux, which must not depend on Godot.
- **Exposing all five §11.3 classes as stubs.** Rejected: dead surface that still
  needs H-unsafe review.

## Consequences

**What the thin slice cuts.** Movement, transforms, interpolation, prediction,
click-to-move and the camera rig (one fixed camera); all art, audio and VFX
(primitives, plus event hooks with nothing attached); remote connect and resume;
save and load; `Document`, `Assets`, `Preview`, `Diagnostics` and the editor; an
exported `.exe` (the demo runs through the installed Godot, because an export needs
export templates); T104 UX research and T108 ruleset-driven UI; real AI;
perception, grant narrowing and snapshot transfer; real-time encounters; more than
one human peer.

- **T047a** (`crpg-server`, new): decision 7, tests L1–L6 in
  `crates/crpg-server/tests/host_legal.rs`, plus its `AGENTS.md` surface list and
  architecture doc. Precedes T047.
- **T047** opens the crate: sections 1–13, `apps/client`, proxy spawn from one bulk
  `view()` (§19.1 #22), BR-1–BR-22, `docs/architecture/crpg-godot.md` and
  `crates/crpg-godot/AGENTS.md`. Plan prerequisite T105 drops for this slice; T047a
  joins. H-unsafe applies to the one `unsafe impl`.
- **T048** owns proxy lifecycle in GDScript: diff `net_ids()` between revisions, free
  on leave, show `dead()`. It adds no FFI member.
- **T049** is out of the slice and blocked on the lane-1 movement channel: there is
  nothing to interpolate. It later adds `positions()` and `previous_positions()`
  to `CrpgView`; adding to the view object is why this is an object.
- **Cut-down T051** is GDScript over `view()`: turn, health, option buttons, log,
  end screen. It adds no FFI member and has no T104/T108 prerequisite; those stay
  on full T051.
- **T046** must provide, for T047: applying a v2 `DeltaFrame`, in order, with an error
  rather than a skip on a gap; entity listing ascending by `NetId`; health absent
  distinct from zero; the turn fact. The bridge decodes frames, so it takes event ops
  and receipts itself and asks the replica only for state. If T046's contract
  differs, T047's contract adapts; this ADR assumes nothing else of it.
- **Follow-on (unnumbered):** hoist the campaign walk, `EncounterSpec` assembly and
  name rules out of `crpg-cli` so the two consumers share them. Not in T047: it would
  touch `crpg-sim` or `crpg-data` and could move a CLI golden.
- **Easier:** both `core` halves test headlessly on both targets; the FFI is 6 + 16
  members, and T048 and T051 should need no Rust.
- **Harder:** three small copies of CLI glue exist until the follow-on, and the
  client half relies on a local presentation table that a remote client cannot have.

**To verify at T047.** Not verifiable without a build, and nothing here depends on
the answer changing the shapes above:

- Exact `#[gdextension]`, `GodotClass`, `#[godot_api]` and `#[func]` syntax, and
  `RefCounted` base and init. The macros exist in `godot-macros-0.5.5`.
- Dictionary and array conversions: the crate names `VarDictionary`, and typed
  `Array` forms may differ from the table above.
- `.gdextension` keys, entry symbol, `compatibility_minimum`, `reloadable`, and
  whether a `res://../../target/...` library path loads, including when
  `CARGO_TARGET_DIR` is set.
- First load of `api-4-7` on 4.7.2.
- Off-main-thread behaviour of a `#[func]`.
- Whether `default-features = false` or `lazy-function-tables` is needed to keep
  build memory and time acceptable; record the measured build in the completion record.
- `multiple-versions = "deny"` and `unmaintained = "all"` against the `godot` tree.

## Decisions for the user

### U1. May the client ship with a dependency under the MPL-2.0 licence?

**What it is.** A licence is the legal terms a library comes with. The project's
rule (`deny.toml`) allows only "permissive" licences, which ask for little more than
keeping a notice. Godot itself is permissive, but the Rust library that lets our code
talk to Godot, the `godot` crate, is under MPL-2.0. All eight packages in its family
are: `godot`, `godot-bindings`, `godot-cell`, `godot-codegen`, `godot-core`,
`godot-ffi`, `godot-macros` and `gdextension-api` (checked in each package's manifest).
MPL-2.0 is "weak copyleft": it applies per file. If we change one of *their* files and
ship it, we must publish that changed file. Our own files stay ours, under any terms.

**Why it matters.** `deny.toml` says MPL-2.0 is deliberately not on the list and that
needing it takes a decision, not a one-line edit. You approved the `godot` crate in
D27d, but that question was about versions, not licences. Until this is answered,
T047 may not add the dependency, so the demo cannot land.

**Options.**

- **(a) Allow MPL-2.0 for those eight packages only.** The general rule stays strict;
  any other MPL package still fails. Cost: a licence notice in the shipped product, and
  publishing any edits we make to those files (we plan none).
- **(b) Allow MPL-2.0 everywhere.** One line, but future packages pass without anyone
  deciding, which is the drift the rule exists to stop.
- **(c) Refuse it, and write our own bridge in C++** (spec risk 17's fallback). No
  licence change, but it means unsafe C++ across the whole boundary, and months before
  any Godot demo.

**Recommendation: (a).** It is what the crate family needs and nothing more.

### Also needed from the user (not choices)

- Accept or amend this ADR, as plan wave 7 requires.
- H-unsafe review of the one `unsafe impl` in T047, S-set review of T047 and T047c.
- Add `client` to the required checks once T047c lands (C7).

## Adjudication

Draft A is `tasks/T045-draft-a.md` (Muse), draft B is `tasks/T045-draft-b.md`
(DeepSeek). B won most points. Both were checked against source.

| Point | Outcome |
|---|---|
| No new crate; `apps/client` over plan's `client/` | Both agree; adopted |
| Godot-free core under a thin shell | **B**. A had no equivalent |
| Campaign → authority path, acknowledge before wrap | **B**. A left `from_history` input unexplained |
| Transport | **Neither.** A named a pipe, but its flow called `Host` directly; B dropped it. The brief and `AGENTS.md` require the in-memory transport, and A's own flow did not honour that, so `LocalLink` |
| Delivery ack | **Neither.** B acknowledged after apply; the QUIC adapter's hand-off rule wins, so both paths match |
| One thread | Both agree; adopted |
| Clock | **B**: bridge clock in Rust, none at the FFI. A's engine clock lets GDScript reach time |
| Pump cadence | **B**. A misread the 50 ms QUIC hello note as a host cadence |
| `Host::tick` | **B**: never called. A had the adapter call it |
| Session kind | **B**, on a different reason: parity with `/2` peers. "Kind is fixed" costs nothing here, since sessions die with the process |
| `NetId`-only FFI | Both agree; adopted |
| `NetId` → presentation | **Neither.** A had no answer; B inferred it from minting order. A host read replaces the inference |
| Class set | **A's two classes**, without its four stubs and without B's `CrpgDiagnostics` |
| Names | **B**: `Crpg` prefix |
| Signals | **Neither.** Both had them; `poll` then `view` is one explicit order |
| View arrays | **B**, corrected: `PackedInt32Array` cannot hold a `u32`, so 64-bit, plus `health_known`. A's `snapshot_bulk` transforms do not exist; there is no transform op |
| Intent submission | **Neither.** B's typed verbs beat A's raw bytes, but an option index beats both |
| Option list | **B** saw the gap, but placed the fix in T051, which is the wrong crate. T047a owns it |
| NPC turns | **Neither** raised it. Decision 7 |
| Capture watermark | **Neither** raised it. Without step 5, play stops at 4,096 actions |
| Errors | **B**, reduced to groups. A's `SessionExpired` and `NotSnapshotPeer` are not `HostError` variants |
| `unsafe` | **B's layering**, corrected: "no hand-written `unsafe`" is impossible, because `ExtensionLibrary` is an `unsafe trait` |
| Panics | **B**: wrap regardless. The pinned source shows the crate does catch; the guard stays so the session fences instead of limping on |
| `engine` feature | **B**. Corrected: `rlib`, explicit `api-4-7`, no `GODOT4_BIN` need |
| `cargo deny` claim | **Against B.** "Reads the graph whether or not a feature is on" is unproved, so C5 tests it |
| `incarnation` at the FFI, A's `new_peer(grants)` | **Neither.** GDScript picks no authority facts |
| Licence question | **B** raised it; narrowed here to per-crate exceptions |
| Open questions | Resolved here, except the licence: seam placement, `now_ms`, option source, engine/crate match, proxy identity, panic catch |

## Acceptance and evidence

Not yet run. Results go in each task's completion record. BR-1–BR-20 run under default
features on both native targets; BR-21 runs in CI; BR-22 runs on Windows.

| Item | Proof |
|---|---|
| BR-1 | `open` on `combat_basic` yields every participant once, ascending `NetId`, non-zero `active` |
| BR-2 | Each code 20–25 from a bad input; no host built; a later valid `open` succeeds |
| BR-3 | Same seed and action script: `Host::authority_hash` after the bridge equals an oracle `HistoryWorld` given the same actions and acknowledgements |
| BR-4 | NPC chooser against fixed option lists: first attack on another side, else first option |
| BR-5 | Scripted clock: no NPC intent before `NPC_ACTION_DELAY_MS`, one at it, at most one per `poll` |
| BR-6 | Idle `poll`: `authority_hash`, `executions` and `revision` unchanged |
| BR-7 | Full link: hand-off stops, the acknowledged ordinal equals the last accepted, the remainder arrives later, in order, once |
| BR-8 | A `Transfer` entry routes to fatal `54` |
| BR-9 | After each `poll`, `capture_acknowledged == last_capture` |
| BR-10 | Source scan: `core/client.rs` contains no `crpg_server` or `crpg_sim` |
| BR-11 | `submit_option` returns 1, 4, 6 and 5, in that precedence, and sends nothing |
| BR-12 | A crafted intent naming an unknown `NetId` yields a receipt with code 17, `poll` returning 0, and the session live |
| BR-13 | A corrupt frame on the link gives fatal `52`; the host is closed; every later `poll` repeats it |
| BR-14 | All per-entity arrays are equal length in `net_ids` order; under partial grants undisclosed health is `health_known = 0` |
| BR-15 | `u32::MAX` health survives into the 64-bit array |
| BR-16 | Events and receipts appear in frame order, once, in the revision that applied them |
| BR-17 | Each `net_ids` row names the placement `replica_id` maps to it |
| BR-18 | Negative or oversized `int` arguments give `3` with no core call |
| BR-19 | An injected core panic returns `55`, later calls repeat it, the core is gone |
| BR-20 | Source scan: `shell.rs` has exactly one `unsafe` token; `core/mod.rs` has the `forbid` |
| BR-21 | The default-feature graph on both targets has no `godot` package |
| BR-22 | `Godot4 --headless --path apps/client -s tests/smoke.gd` spawns one proxy per participant, plays `combat_basic` to `ended()` choosing option 0, exits 0 |
| L1–L6 | `crates/crpg-server/tests/host_legal.rs`, on the checked-in fixture |

## Supersession and corrections

None.

## Agent log

- 2026-11-01 (UTC) · claude-code/opus-5.5 (architect) + T045 · Adjudicated the two blind bridge drafts against the source into one Proposed ADR, adding the host read API, NPC driver, in-memory link and capture watermark that the thin slice needs and neither draft fully covered; numbered 0033 because ADR-0030 is reserved for T024q, and no ADR index exists to update.

# T151 — `crpg-godot` bridge move to protocol v4 (`crpg-godot`, `apps/client`)

**Status: draft (blind), 2026-11-15 (UTC).** Not approved. A Claude architect
writes the final contract; the user approves it. Bound by ADR-0033, ADR-0036,
the approved T026p contract (`tasks/T026p.md` §Out of scope `:1477–1479`) and
the approved T037 contract (`tasks/T037.md` §Out of scope `:1356–1358`, and the
Interface items T037b–T037d). Citation convention (T037's): a bare `:N` or
`:N–M` is a line of the file cited earlier in the same list item or sentence.
Every `path:line` was checked on `2aae6c9` (branch `task/T151-draft-a-d1`).

## Task

Move the `crpg-godot` bridge from protocol v2 to protocol v4, so a Godot client
sees positions and sends moves through the embedded single-player server. The
three `crpg-net` client types replace the v2 ones: `ReassemblyPoolV4`
(`crates/crpg-net/src/pool_v4.rs:102`) gates and reassembles delivery,
`ReplicaWorldV4` (`crates/crpg-net/src/replica_v4.rs:55`) holds rows, positions
and samples, and `MoveOutbox` (`crates/crpg-net/src/outbox_v4.rs:54`) is the
lane-1 sender. The session is created choosing v4 with T037's `bind_v4_peer`
(`tasks/T037.md:439–441`); positions and `Moved` reach GDScript as polled view
state; a move request leaves GDScript as a lane-1 intent and its
accept/refuse/supersede receipt is polled back. T049 interpolation (G1), T026
prediction (G2) and T050 click-to-move (G3) are later tasks; this one leaves
room for them without doing them.

Today the bridge opens every session with `HostConfig { protocol:
ProtocolSelection::V2, incarnation: 1 }` (`crates/crpg-godot/src/core/mod.rs:302`),
decodes whole v2 frames itself into a v2 `ReplicaWorld`
(`crates/crpg-godot/src/core/client.rs:18`, `:292`, `:321–329`), submits with
`codec_v2::encode_intent` (`:523`), and treats a `DeliveryKind::Transfer` entry
as a fatal fence (`crates/crpg-godot/src/core/authority.rs:61–75`). None of
those survive the move. The GDExtension boundary is S-set
(`docs/IMPLEMENTATION_PLAN.md:95`); only the part that adds FFI members touches
it.

## Parts

Two parts, one crate (`crpg-godot`), order **serial**: a → b. a is the
Godot-free core; b is the gdext shell. `apps/client` GDScript is **not** changed
(Reading 4), so no third part.

| Part | Crate | Scope | S-set | Review |
|---|---|---|---|---|
| T151a | `crpg-godot` | v4 session creation, the pool + replica + outbox swap, positions in the internal view | no | Claude reviewer mutant run, one fix round |
| T151b | `crpg-godot` | position and move FFI members on `CrpgView`/`CrpgSession` | **yes**: touches the GDExtension boundary (`docs/IMPLEMENTATION_PLAN.md:95`) | human-reviewed (S-set) |

### T151a owns
- `crates/crpg-godot/src/core/mod.rs`, `crates/crpg-godot/src/core/client.rs`,
  `crates/crpg-godot/src/core/authority.rs`, `crates/crpg-godot/src/core/errors.rs`
- `crates/crpg-godot/src/core/tests.rs`,
  `crates/crpg-godot/src/core/{authority,client,errors,link}/tests.rs`
- `crates/crpg-godot/tests/bridge.rs`, `crates/crpg-godot/tests/source_rules.rs`
- `docs/architecture/crpg-godot.md`, `crates/crpg-godot/AGENTS.md`
- goldens: none new, none moved.

### T151b owns
- `crates/crpg-godot/src/shell.rs`
- `docs/architecture/crpg-godot.md`, `crates/crpg-godot/AGENTS.md`
- goldens: none new, none moved.

`core/link.rs` is owned by neither part unless T151a finds it must change; the
link stays two byte FIFOs (`crates/crpg-godot/src/core/link.rs:19–26`) and only
its callers learn to route more frame kinds (Reading 2). `Cargo.toml`,
`Cargo.lock`, `apps/client/**`, `ci.yml`, `deny.toml` and `rust-toolchain.toml`
are not in either part's owned list.

## Readings

Design decisions this contract makes. Each needs approval with the contract.

1. **A v4 session is the player's session; the NPC stays a v2 `bind_peer`.**
   The client half reads only the player's delivery; the NPC peer exists only
   to drive `legal_intents`, `ingest` and `replica_id`
   (`crates/crpg-godot/src/core/authority.rs:522–548`, `:702–736`), which are
   protocol-agnostic host calls. `HostConfig.protocol` has no v3/v4 variant
   (`crates/crpg-server/src/host.rs:106–111`), so it stays
   `ProtocolSelection::V2` and the NPC's `bind_peer` keeps yielding v2; the
   player binds with `bind_v4_peer` (T037 `:439–441`). *Rejected:* binding both
   peers v4 (the NPC session is never read, and `bind_v4_peer` needs a
   `FactGrants` + `PauseRights` the NPC driver never uses).

2. **The link now carries `0x82` transfer frames, v4 deltas and `0xA1` lane-1
   frames; the client routes by first byte.** ADR-0033 decision 8
   (`docs/adr/0033-godot-bridge.md:108`) says the link carries lane-0 only
   because "a `DeliveryKind::Transfer` entry is a fatal fence in this slice" —
   and that holds only because "nothing sends `Resync`, so a `Transfer` entry is
   unreachable" (`:229–230`). A v4 session's first delivery *is* a `0x82`
   transfer (T037 B1, `tasks/T037.md:967–971`), and samples/receipts are lane 1
   (`crates/crpg-net/src/movement_v4.rs:4–8`). So `hand_off` forwards `Transfer`
   and `Delta` alike (both have bytes), the authority also hands
   `take_move_receipts` and `take_samples` to the link, and the client routes
   each frame by its first byte — the same subprotocol tag the codecs already
   multiplex on (`0x82` transfer, `4` v4 delta, `0xA1` movement). *Rejected:*
   keeping the fence (the v4 replica can never start) and adding an in-band
   channel tag to the FIFO (the first byte is already the tag; a second tag
   would be dead data). This amends ADR-0033 decision 8 — see Open question 1.

3. **The client half applies through the pool, not directly.** Replace
   `ReplicaWorld::new(epoch)` + `decode_delta`/`apply_delta`
   (`crates/crpg-godot/src/core/client.rs:292`, `:321–329`) with
   `ReassemblyPoolV4::new(TransferLimitsV3::default())`, one
   `add_receiver(epoch, GateMode::AwaitSnapshot)`, `offer_transfer` /
   `offer_delta`, `pop_update`, and `ReplicaWorldV4::{apply_snapshot,
   apply_delta}` (`crates/crpg-net/src/pool_v4.rs:113`, `:131`, `:227`, `:534`,
   `:656`; `crates/crpg-net/src/replica_v4.rs:96`, `:135`). A
   `ReplicaUpdateV4::Snapshot` pops from the pool and applies as
   `apply_snapshot`, a `Delta` as `apply_delta`. *Rejected:* decoding v4 deltas
   directly (the pool gates `event_seq` and reassembles the `0x82` snapshot; a
   bare decoder would reimplement both, and T026p V-1 says the pool is the one
   gate).

4. **Positions reach GDScript as polled parallel arrays; no `apps/client`
   change.** ADR-0033 forbids signals (`docs/adr/0033-godot-bridge.md:256–257`),
   so positions are new view members, polled like `health()`. `SessionView`
   gains `positions` (3 raw `Fx16_16` ints per row, `net_ids()` order, `0,0,0`
   where undisclosed), `position_known`, and `moving`. Nothing in
   `apps/client` reads them yet: `SceneSync.gd` keeps its client-only proxy
   layout (`crates/crpg-godot/AGENTS.md` and `docs/architecture/crpg-godot.md:191–205`)
   and T049/T050 are the first consumers. *Rejected:* exposing positions as
   signals (forbidden) and changing `SceneSync.gd` to place by position (that is
   T050's presentation). See Open question 2 for the ADR §:519 overlap.

5. **`Moved` reaches GDScript as a `ViewEvent` in `events()`.** The replica
   stores nothing from `Moved` (`crates/crpg-net/src/replica_v4.rs:186`), but
   the client half already walks `frame.ops` into `ViewEvent`s
   (`crates/crpg-godot/src/core/client.rs:330–390`). For v4 those ops are
   `DeltaOp::V3` (unwrap and match as today) and `DeltaOp::Moved`/`Position`
   (`crates/crpg-net/src/protocol_v4.rs:155–174`). `Moved { entity, points }`
   becomes a new `ViewEvent::Moved` whose `entries()` carries `kind`, `entity`
   and the raw `points`; `Position` is state only (it updates the replica, not
   the event stream). `ViewValue` gains an `Ints(&[i64])` arm so points ride
   the existing `entries()`/shell path unchanged. *Rejected:* a separate
   `moved()` getter (the events stream is the one mechanism the HUD already
   consumes and the fixture produces no `Moved`, so the HUD cases are
   unaffected).

6. **Moves leave GDScript as lane-1 intents through a `MoveOutbox`, keyed by
   the acting `NetId`.** A move needs an actor and a destination, which ADR-0033
   §10's "submission is by option index only" does not cover (that rule is the
   combat `submit_option` surface, `docs/adr/0033-godot-bridge.md:304`; ADR-0033
   has no v4 or lane-1 wording, `:27`). So `CrpgSession` gains two methods,
   `submit_move(actor, x, y, z) -> i64` and `halt_move(actor) -> i64`, where
   `actor` is a `net_ids()` entry (a `NetId`, never an `EntityId`, ADR §:236).
   The client encodes `MoveIntent { epoch, seq, actor, body }` with
   `movement_v4::encode_intent` (`crates/crpg-net/src/movement_v4.rs:207`) and
   sends it up the link; the authority `ingest_move`s it
   (`tasks/T037.md:483–484`). Accept/refuse/supersede returns as a
   `MoveReceipt` polled in `move_receipts()`. *Rejected:* an option-index move
   (there is no `legal_intents` move option; `legal_intents` keeps T047a's v1
   vocabulary, T037 `:1366`) and naming the actor by `EntityId` (ADR §:236).

7. **The lane-0 combat submit moves to `codec_v4`.** The bridge still submits
   combat options (`DeclareAction`/`EndTurn`) through `submit_option`, but the
   frame is now a v4 intent: `protocol_v4::IntentFrame` with `body:
   protocol_v4::IntentBody`, encoded by `codec_v4::encode_intent`
   (`crates/crpg-net/src/codec_v4.rs:181`). `Offered.body` becomes
   `protocol_v4::IntentBody`; the `legal_intents` → `Offered` mapping converts
   each v1 `protocol::IntentBody` to its v4 twin (the six v4 bodies re-declare
   v3's, `crates/crpg-net/src/protocol_v4.rs:95–136`). *Rejected:* keeping
   `codec_v2::encode_intent` (a v4 session refuses v2 intent bytes,
   `crates/crpg-net/src/pool_v4.rs:15–17`).

8. **Error mapping is unchanged: no new `BridgeCode`, no new `ReplicaError`
   code.** T026p Errors says `ReplicaError` is reused unchanged, so the
   bridge's fatal-53 mapping and every `Display` text stay
   (`tasks/T026p.md:1042`). A v4 decode failure is `FrameUndecodable` (52), a
   pool or replica refusal `ReplicaRejected` (53), an `ingest_move`/`take_*`
   host error maps through the wildcard `host_fault`
   (`crates/crpg-godot/src/core/errors.rs:106–110`). `OutboxError::Full` on a
   move submit maps to `IntentRefused` (30), not a new code. *Rejected:* a new
   code (the table is closed, `crates/crpg-godot/src/core/errors.rs:9`).

## Interface

## Behaviour

## Errors

## Tests

## What must not change

## Dependency edges

## Out of scope

## Open questions

## Agent log

- 2026-11-15 (UTC) · opencode-go/deepseek-v4-pro + T151 draft a · Blind draft of the `crpg-godot` bridge move to protocol v4 (`ReassemblyPoolV4`, `ReplicaWorldV4`, `MoveOutbox`), so the bridge opens a v4 session, sees positions and `Moved`, and sends lane-1 moves through the embedded server, leaving T049/T026/T050 room without doing them.

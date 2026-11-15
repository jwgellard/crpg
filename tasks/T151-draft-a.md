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

Additions and replacements only. Every public item keeps a doc comment
(`#![warn(missing_docs)]`, `crates/crpg-godot/src/lib.rs:12`). `core` stays
`#![forbid(unsafe_code)]`; no float, `HashMap`/`HashSet`, clock or thread enters
the core (crate invariants, `crates/crpg-godot/AGENTS.md:1–8`). Types from
`crpg_net` are used by path, never re-exported; the one `unsafe` stays in
`shell.rs` (`crates/crpg-godot/AGENTS.md:14`).

### T151a — `core/mod.rs`, `core/client.rs`, `core/authority.rs`

```rust
// mod.rs (open path, Reading 1)
// HostConfig stays { protocol: ProtocolSelection::V2, incarnation: 1 }.
// The player binds bind_v4_peer(ControlGrant { actors }, disclosure.clone(),
//     FACTS, R1, 0); the NPC keeps bind_peer(ControlGrant { actors }, disclosure, 0).
// FACTS and R1 are the bridge's full fact grants and default pause rights
// (Reading 1; see Open question 3 for the exact FactGrants/PauseRights).

// client.rs
pub struct SessionView { /* existing 16 fields, unchanged, plus: */ }
impl SessionView {
    // existing getters unchanged, plus:
    pub fn positions(&self) -> &[i64];       // 3 per row, net_ids() order, raw Fx16_16; 0,0,0 undisclosed
    pub fn position_known(&self) -> &[u8];   // 1 per row
    pub fn moving(&self) -> &[u8];           // 1 per row
    pub fn move_receipts(&self) -> &[ViewMoveReceipt];
}
pub enum ViewEvent {
    // existing four variants unchanged, plus:
    Moved { entity: i64, points: Vec<i64> }, // raw Fx16_16, 3 per point, in order
}
pub enum ViewValue<'a> { Int(i64), Text(&'a str), Bool(bool), Ints(&'a [i64]) }
pub struct ViewMoveReceipt { pub seq: i64, pub accepted: bool, pub superseded: bool, pub code: i64 }
impl ViewMoveReceipt { pub fn entries(&self) -> Vec<(&'static str, ViewValue<'_>)>; }
// entries() keys, "kind" first: Moved -> kind, entity, points(Ints);
// move_receipt -> seq, accepted, superseded, code.

pub(crate) struct Client { /* replica: ReplicaWorldV4, pool, receiver, outbox, … */ }
impl Client {
    pub(crate) fn new(epoch: SessionEpoch, table: PresentationTable) -> Self;
    pub(crate) fn step(&mut self, link: &mut impl Transport, options: &[Offered],
        refused: bool) -> Result<(), Fault>;
    pub(crate) fn submit(&mut self, revision: i64, index: i64, link: &mut impl Transport)
        -> Result<(), Fault>;
    pub(crate) fn submit_move(&mut self, actor: NetId, body: MoveBody,
        link: &mut impl Transport) -> Result<(), Fault>;
    pub(crate) fn view(&self) -> Rc<SessionView>;
}
pub(crate) struct Offered { pub(crate) actor: NetId, pub(crate) body: protocol_v4::IntentBody }

// authority.rs
pub(crate) fn hand_off(entries: &[DeliveryEntry], link: &mut impl Transport)
    -> Result<Option<u64>, Fault>;   // now forwards Transfer and Delta (Reading 2)
impl Authority {
    // step() gains, after hand_off and before the NPC delivery ack:
    //   take_move_receipts(player) -> hand_off_bytes; take_samples(player) -> hand_off_bytes
}
```

`Client::new` builds `ReplicaWorldV4::new(epoch)`
(`crates/crpg-net/src/replica_v4.rs:63`), a `ReassemblyPoolV4::new(TransferLimitsV3::default())`
with one `add_receiver(epoch.0, GateMode::AwaitSnapshot)` (`crates/crpg-net/src/pool_v4.rs:131`),
and a `MoveOutbox::new(epoch.0)` (`crates/crpg-net/src/outbox_v4.rs:63`). The
pool keeps one receiver for the session's lifetime; a session never rebinds or
removes it (the bridge never resumes, `docs/adr/0033-godot-bridge.md:229–230`).

`Client::step` drains the link and routes each frame by first byte
(Reading 2): `0x82` → `pool.offer_transfer`, `4` → `pool.offer_delta`,
`0xA1` → `movement_v4::decode_down` and either `MoveDown::Receipt` →
`outbox.receive` (its error is fatal 53) or `MoveDown::Sample` →
`replica.apply_sample` (never fails, `replica_v4.rs:148`). Then it pops every
`pool.pop_update`: `Snapshot(p)` → `replica.apply_snapshot(&p)` (`:135`),
`Delta(d)` → `replica.apply_delta(&d)` (`:96`), a refusal fatal 53. The
event/receipt walk (`client.rs:330–390`) matches `DeltaOp::V3` (recurse as
today) and `DeltaOp::Moved` (push `ViewEvent::Moved`); `DeltaOp::Position` is
state only. The view rebuild (`:396–481`) gains the three position columns and
`move_receipts` from the outbox's newly completed receipts.

### T151b — `shell.rs`

```rust
#[godot_api]
impl CrpgSession {
    // existing six #[func]s unchanged, plus:
    #[func] fn submit_move(&mut self, actor: i64, x: i64, y: i64, z: i64) -> i64;
    #[func] fn halt_move(&mut self, actor: i64) -> i64;
}
#[godot_api]
impl CrpgView {
    // existing 16 getters unchanged, plus:
    #[func] fn positions(&self) -> PackedInt64Array;
    #[func] fn position_known(&self) -> PackedByteArray;
    #[func] fn moving(&self) -> PackedByteArray;
    #[func] fn move_receipts(&self) -> Array<VarDictionary>;
}
```

Each `#[func]` makes exactly one core call (`crates/crpg-godot/AGENTS.md:15`).
`submit_move`/`halt_move` convert `actor` to a `NetId` via
`NetId::new(actor as u64)` (zero/negative → `BadArgument` 3), build
`MoveBody::MoveTo { destination: Point::new([x,y,z]) }` (a `None` →
`BadArgument` 3) or `MoveBody::Halt`, and return the core `submit_move`
code. `positions` packs `3 × net_ids().len()` ints; `move_receipts` builds
dictionaries from `ViewMoveReceipt::entries()` through the same `dictionary`
helper (`shell.rs:217`).

## Behaviour

Normative. Each item names the tests that prove it. "Refused" means the stated
error and no state change. **First failure wins** in every list.

1. **B1. Session creation chooses v4.** `open` builds the host with
   `HostConfig { protocol: ProtocolSelection::V2, incarnation: 1 }`
   (unchanged, `crates/crpg-godot/src/core/mod.rs:301–304`) and binds the player
   with `bind_v4_peer(ControlGrant { actors }, disclosure.clone(), FACTS, R1, 0)`
   instead of `bind_snapshot_peer` (`:325–334`); the NPC keeps `bind_peer`
   (`:335–338`). After open, `session_wire(player) == 4`
   (`crates/crpg-server/src/host.rs:1195–1202`, T037 `:447–448`), the player's
   epoch feeds `Client::new` as today (`crates/crpg-godot/src/core/mod.rs:345–349`),
   and `replica_id` still resolves every bound entity (Reading 1). A `NotV4Peer`
   or any bind failure is `StartFailed` (25). *Tests:* K1, K2.

2. **B2. The first delivery is a snapshot then anchors, applied through the
   pool.** The player's `take_delivery_entries` on a fresh v4 session is
   `Transfer` (Begin), `Transfer` (Chunk), then `Delta` (T037 B1,
   `tasks/T037.md:969–971`); `hand_off` forwards all three (Reading 2). The
   client's first `step` offers the two `Transfer` bytes to
   `pool.offer_transfer`, then the `Delta` to `pool.offer_delta`, and pops
   `Snapshot` then `Delta`, applying each; the replica's `v3()` rows equal the
   v2 session's rows (T037 B-2 `tasks/T037.md:981–990`), and `positions()` is
   the three anchored `PositionState`s. *Tests:* K3.

3. **B3. Positions reach GDScript as polled state.** After B2, `view.positions()`
   is `3 × net_ids().len()` ints in `net_ids()` order: the raw `Fx16_16` point
   of each disclosed position, `[0,0,0]` where none is disclosed;
   `position_known()` is `1` per disclosed row; `moving()` is the row's route
   flag (`crates/crpg-net/src/replica_v4.rs:30–39`). A `Position { Some }` op
   updates the row, `Position { None }` clears it (known 0), a sample updates
   the point and `moving` only when strictly newer (`:148–173`). No signal
   fires; the view is read through `view()`. *Tests:* K4, K5.

4. **B4. `Moved` reaches GDScript as an event.** A v4 `DeltaOp::Moved { entity,
   points }` pushes `ViewEvent::Moved` into that revision's `events()`, with
   `entries()` keys `kind="moved"`, `entity` (the `NetId` as `i64`), and
   `points` (raw ints, 3 per point, in route order); the replica stores nothing
   from it (`crates/crpg-net/src/replica_v4.rs:186`). A `DeltaOp::Position`
   pushes no event. `DeltaOp::V3` recurses exactly as today, so
   `ActionResolved`/`TurnStarted`/`Died`/`EncounterEnded` and v3 receipts keep
   working. *Tests:* K6.

5. **B5. A move request leaves GDScript.** `submit_move(actor, x, y, z)`
   returns 0 and enqueues one lane-1 `MoveTo` intent: the core builds
   `MoveOutbox::send(actor, MoveBody::MoveTo { destination: Point::new([x,y,z])
   })`, encodes `movement_v4::encode_intent`, and sends it up the link; the next
   authority `step` calls `ingest_move(player, bytes, now_ms)` (T037
   `:483–484`). `halt_move(actor)` does the same for `MoveBody::Halt`. A `None`
   from `Point::new` (a coordinate outside `COORD_LIMIT_RAW`,
   `crates/crpg-net/src/protocol_v4.rs:38`) or a zero/negative `actor` returns 3
   and sends nothing. `MoveOutbox::send` `Full` (8 in flight,
   `crates/crpg-net/src/outbox_v4.rs:97–99`) returns 30. *Tests:* K7, K8.

6. **B6. Accept, refuse, supersede reach GDScript.** After B5, the authority
   `step` hands `take_move_receipts(player)` bytes to the link (Reading 2); the
   client decodes `MoveDown::Receipt` and calls `outbox.receive`
   (`crates/crpg-net/src/outbox_v4.rs:113`), which completes only the oldest
   pending intent. Each completed receipt becomes a `ViewMoveReceipt` in that
   revision's `move_receipts()` with `seq`, `accepted` (`MoveOutcome::Accepted`),
   `superseded` (`Superseded`), and `code` (the `RejectionCode::as_u8()` for
   `Rejected`, else 0). The view bumps at most once per poll (crate invariant
   6). *Tests:* K9.

7. **B7. Samples update positions, never fail.** The authority hands
   `take_samples(player)` datagrams to the link; the client decodes
   `MoveDown::Sample` and calls `replica.apply_sample` (never fails,
   `crates/crpg-net/src/replica_v4.rs:148`), which moves `sample_clock` only to
   a strictly newer `server_tick` and updates only disclosed, strictly-newer
   positions. A stale, duplicated or unknown-entry datagram is ignored, not
   fatal. *Tests:* K5.

8. **B8. The lane-0 combat path keeps working on v4.** `submit_option`
   still checks 3 → `Fenced` → 1 → 4 → 6 → 5 → send, but encodes
   `protocol_v4::IntentFrame` with `codec_v4::encode_intent`
   (`crates/crpg-net/src/codec_v4.rs:181`) and an `Offered.body` of
   `protocol_v4::IntentBody` (Reading 7). A v2/v3 intent byte is never produced;
   the v4 first byte is `4`. *Tests:* K10.

9. **B9. v2/v3 and the HUD keep doing exactly what they did.** A host with the
   NPC still on `bind_peer` (v2) and the player on v4 produces, for every
   lane-0 script of combat intents, the same rows, names, sides, health, turn,
   options, events and receipts in the view as today's all-v2 bridge (T037 B-2,
   `tasks/T037.md:981–990`, restated for the bridge's own fixture). The
   `apps/client` suite — `load.gd` E1 (`ClassDB.class_exists`, revision 1,
   `net_ids == 2`, names), `scene_sync.gd` S1–S19, `hud.gd` H1–H19 and
   `smoke.gd` E2/H20 — passes unchanged, because `SceneSync.gd` and
   `CombatHud.gd` read only the existing 14 view getters
   (`apps/client/tests/hud_cases.gd:25–33`) and the fixture never produces a
   `Moved` or a lane-1 receipt. *Tests:* K11, K12, and every existing GDScript
   suite re-run under the engine gates.

10. **B10. Error mapping.** A frame that `codec_v4::decode_delta` refuses is
    52; a `pool.offer_*`/`apply_*` refusal is 53; a bad `submit_move`
    argument is 3; `MoveOutbox::send` `Full` or a full link is 30; an
    `ingest_move`/`take_*` `HostError` maps through `host_fault`
    (`crates/crpg-godot/src/core/errors.rs:106–110`) to 50/51. No new
    `BridgeCode` value, `name()` text or `is_fatal` bound changes
    (`crates/crpg-godot/src/core/errors.rs:12–91`). *Tests:* K13.

## Errors

## Tests

## What must not change

## Dependency edges

## Out of scope

## Open questions

## Agent log

- 2026-11-15 (UTC) · opencode-go/deepseek-v4-pro + T151 draft a · Blind draft of the `crpg-godot` bridge move to protocol v4 (`ReassemblyPoolV4`, `ReplicaWorldV4`, `MoveOutbox`), so the bridge opens a v4 session, sees positions and `Moved`, and sends lane-1 moves through the embedded server, leaving T049/T026/T050 room without doing them.

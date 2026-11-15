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
   has no v4 or lane-1 wording, `tasks/T151-facts.md:27`). So `CrpgSession` gains two methods,
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
the core (crate invariant 8, `crates/crpg-godot/AGENTS.md:34`). Types from
`crpg_net` are used by path, never re-exported; the one `unsafe` stays in
`shell.rs` (`crates/crpg-godot/AGENTS.md:52`).

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

Each `#[func]` makes exactly one core call (`crates/crpg-godot/AGENTS.md:57`).
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

No `BridgeCode` variant is added, renamed, renumbered or retexted
(`crates/crpg-godot/src/core/errors.rs:12–91`); `is_fatal` stays exactly
`50..=55` (`:88–90`). No new `ReplicaError` variant (T026p Errors,
`tasks/T026p.md:1042`). New mapping table only:

| Refusal | Result |
|---|---|
| `submit_move`/`halt_move` with zero or negative `actor` | 3 (`BAD_ARGUMENT`), nothing sent |
| `Point::new` `None` (a coordinate outside `COORD_LIMIT_RAW`) | 3, nothing sent |
| `MoveOutbox::send` `Full` (8 in flight) or the link is full | 30 (`INTENT_REFUSED`) |
| v4 delta does not decode (`codec_v4::decode_delta`) | 52 (`FRAME_UNDECODABLE`) |
| `pool.offer_transfer`/`offer_delta` or `apply_snapshot`/`apply_delta` refusal | 53 (`REPLICA_REJECTED`) |
| `movement_v4::decode_down` failure or `outbox.receive` `Err` | 53 |
| `ingest_move`/`take_move_receipts`/`take_samples` `HostError` | 50/51 through `host_fault` |

`last_error` is `NAME` or `NAME: detail` and is never control flow
(`crates/crpg-godot/src/core/errors.rs:4`).

## Tests

Public API first, then unit cases in the named file's `#[cfg(test)]` module.
The fixture is `campaigns/fixtures/combat_basic` (first-blood, 1 v 1:
`Hero`/`Goblin`), opened with `("", "", 1)` unless stated; both placements stand
at `[0,0,0]` (`campaigns/fixtures/combat_basic/areas/start/area.json`), so the
v4 first delivery anchors both rows at raw `[0,0,0]`. v4 cases run after T037's
host surface (`bind_v4_peer`, `ingest_move`, `take_move_receipts`,
`take_samples`) is available; each is cited against it. `n(1)`/`n(2)` are the
player's `NetId`s for `Hero`/`Goblin`; `P(x,y,z)` = `crpg_net::protocol_v4::Point::new([x,y,z])`
raw.

- **K1 `session_chooses_v4` (B1).** Open the fixture; through the test
  accessors (`crates/crpg-godot/src/core/authority.rs:618–644`):
  `session_wire(player) == Ok(4)` and `session_wire(npc) == Ok(2)`. Control: a
  second bridge built the old way (player `bind_snapshot_peer`, NPC
  `bind_peer`) reports `Ok(2)` for both, so the `4` is not a default. *Fails
  if:* the player is not bound v4 or the NPC is dragged to v4.

- **K2 `bind_failure_is_start_failed` (B1).** On a host whose `bind_v4_peer`
  returns `Err(NotV4Peer)`-shaped error (a regressed `now_ms` or a ninth
  binding), `open` returns 25 and leaves no `Live`. Control: the unmodified
  fixture returns 0. *Fails if:* a v4 bind failure is not 25 or half-opens.

- **K3 `first_delivery_snapshot_then_anchors` (B2).** After open, the player's
  first `take_delivery_entries` is `[Transfer, Transfer, Delta]` in ordinal
  order (T037 B1, `tasks/T037.md:969–971`); the client's first `step` applies
  them and `view.net_ids() == [1, 2]` with `names() == ["Hero", "Goblin"]`,
  both health `10/10`, and `position_known() == [1, 1]`. Control: the old
  v2 bridge's first delivery is `[Delta]` only and has no positions. *Fails
  if:* a `Transfer` entry is fenced (54) or the snapshot is not the first
  update.

- **K4 `positions_are_polled` (B3).** After K3: `positions() == [0, 0, 0, 0, 0,
  0]` (2 rows × 3, both `[0,0,0]`), `position_known() == [1, 1]`,
  `moving() == [0, 0]`. A crafted `DeltaOp::Position { n(2), None }` applied
  through the pool makes `position_known() == [1, 0]` and `positions()`'s
  second triple `[0,0,0]`. Control: `DeltaOp::Position { n(2),
  Some(PositionFact { P(65536,0,0), true }) }` makes the second triple
  `[65536, 0, 0]` and `moving() == [0, 1]`, so `[0,0,0]` is not a constant.
  *Fails if:* a position is mis-sliced, withheld, or a withdrawn row still
  reports `known 1`.

- **K5 `samples_update_only_newer` (B7).** From K3, apply a `MoveSample {
  server_tick: 1, processed_seq: 0, entries: [{ n(1), P(65536,0,0), true }] }`:
  the first triple becomes `[65536,0,0]`, `moving() == [1,0]`, and
  `replica.sample_clock()` is `Some({1, 0})`. Apply the same sample again
  (duplicate, same tick): no change. Control: a sample with `server_tick: 2`
  then one with `server_tick: 1` leaves the `2` values (strictly-newer wins).
  *Fails if:* a sample updates an undisclosed row, moves the clock backward, or
  fails.

- **K6 `moved_is_an_event` (B4).** A crafted v4 delta `[Moved { n(1), [P(0,0,0),
  P(32768,0,0)] }]` at the next `event_seq` produces `view.events() == [Moved {
  entity: 1, points: [0,0,0,32768,0,0] }]` and `entries()` key `kind ==
  "moved"`; `positions()` are unchanged (the replica stores nothing). Control:
  a `DeltaOp::Position { n(1), Some(..) }` in the same frame pushes no event,
  so the event is from `Moved`, not the position. *Fails if:* `Moved` is
  dropped, mis-decoded, or stored as a position.

- **K7 `submit_move_enqueues` (B5).** `submit_move(1, 1, 0, 1)` returns 0 and
  the up-queue holds one frame whose first byte is `0xA1` and which
  `movement_v4::decode_intent` reads as `MoveIntent { epoch, seq: 1, actor:
  n(1), body: MoveTo { P(65536,0,65536) } }` (raw 1 m). Control:
  `submit_move(1, COORD_LIMIT_RAW + 1, 0, 0)` returns 3 and enqueues nothing,
  so a well-formed send is what returns 0. *Fails if:* the actor or
  destination is mistyped, or a refused point still sends.

- **K8 `halt_and_window` (B5).** `halt_move(1)` returns 0 and enqueues
  `MoveBody::Halt` for `n(1)`. After eight `submit_move`s with no receipt
  taken, the ninth returns 30 (`MoveOutbox::send` `Full`). Control: taking
  eight receipts first (K9) lets the ninth return 0, so 30 is the window, not a
  permanent error. *Fails if:* `Halt` encodes as `MoveTo`, or the window is not
  8.

- **K9 `receipts_reach_gdscript` (B6).** After K7, feed a `movement_v4::encode_down(MoveDown::Receipt(MoveReceipt { epoch, seq: 1, actor: n(1), processed_tick: 0, outcome: Accepted { start_tick: 1 } }))`
  frame to the client: `move_receipts() == [{ seq: 1, accepted: true,
  superseded: false, code: 0 }]` and the outbox's `processed_seq() == 1`.
  Control: a `Rejected(RejectionCode::IllegalAction)` receipt gives
  `accepted: false, superseded: false, code: 18`, and a `Superseded` gives
  `superseded: true, code: 0`; each completes only the oldest pending intent.
  *Fails if:* a receipt completes out of order, or its outcome/code is
  mis-read.

- **K10 `lane0_submits_v4` (B8).** `submit_option` of a valid option sends a
  frame whose first byte is `4` and which `codec_v4::decode_intent` reads back
  with `body: protocol_v4::IntentBody::EndTurn` (or `DeclareAction`). Control:
  the pre-move bridge sent first byte `2` (`codec_v2`); a v4 session refuses
  that byte, so the old encoder is exercised as the negative. *Fails if:* the
  bridge still emits v2 bytes, or maps the option body to the wrong v4 variant.

- **K11 `view_equals_v2_beside_v4` (B9).** For the fixture's lane-0 script
  (open, poll, `EndTurn`, NPC strike, end), the v4 bridge's view — `net_ids`,
  `names`, `sides`, `health`, `max_health`, `health_known`, `dead`, `active`,
  `round`, `ended`, `option_labels`, `option_targets`, `events`, `receipts` —
  equals the old all-v2 bridge's view field for field at every poll. Control:
  the two bridges differ only in the three new position columns and in
  `move_receipts`, which the v2 bridge does not have. *Fails if:* a v4 change
  leaks into a v3/row/turn/option value.

- **K12 `existing_gdscript_passes` (B9).** Under the engine gates
  (`crates/crpg-godot/AGENTS.md:73–85`), `load.gd` exits 0, `scene_sync.gd`
  exits 0, `hud.gd` exits 0, and `smoke.gd` exits 0 with its control exiting 3.
  Control: the pre-move build runs the same suite green, so a red run is this
  change's regression. *Fails if:* any existing engine test exits non-zero.

- **K13 `error_mapping` (B10).** A `[0xff]` frame through the client link
  fences 52; a `pool.offer_delta`/`apply_delta` refusal fences 53; a
  `submit_move` with `actor 0` returns 3; the ninth unreceipted move returns 30;
  an `ingest_move` `Err(HostError::Closed)` maps to 50. `BridgeCode::name()` and
  `is_fatal()` are byte-identical to today (K1 of the existing errors suite).
  Control: each bad input's otherwise-valid twin returns 0. *Fails if:* a new
  code appears, a refusal changes state, or 52/53 are mis-assigned.

## What must not change

- **No golden is added, moved or re-baselined** (this crate is outside
  `determinism.py`, `docs/adr/0033-godot-bridge.md:371–372`). The
  `crates/crpg-testkit` and `crates/crpg-sim` goldens listed in T037
  (`tasks/T037.md:1319–1330`) are untouched.
- **Every existing `crates/crpg-godot` test file**, byte for byte and passing,
  except the specific case files in T151a's owned list that this contract
  renames/repoints — and no existing assertion is weakened to make the build
  pass (`AGENTS.md:54`).
- **The closed code table and host-fault mapping** (`crates/crpg-godot/src/core/errors.rs:12–115`):
  no variant, number, `name()` text, `is_fatal` bound or `host_fault` arm
  changes.
- **The link shape** (`crates/crpg-godot/src/core/link.rs`): two bounded
  byte FIFOs, `LINK_FRAMES = POLICY_EGRESS_FRAMES_PER_PEER` (`:17`), `LinkFull`
  changes nothing. Only its callers route more frame kinds.
- **The one `unsafe` and the engine pin.** `shell.rs` keeps exactly one
  hand-written `unsafe`; `godot = "=0.5.5"`, `engine` off by default
  (`crates/crpg-godot/AGENTS.md:52`).
- **The `apps/client` scripts and tests**, and `Main.tscn`/`project.godot`/
  `crpg.gdextension`: unchanged (Reading 4). `THIRD_PARTY_NOTICES` unchanged.
- **`crpg-net` and `crpg-server` are frozen here**: no wire byte, codec,
  replica, pool, `Host` surface, `World` field, checkpoint version or tick
  order is changed by this task (they are T026p/T037's). `cargo check
  --workspace` proves `crpg-server` and `crpg-net` compile unchanged.
- **`tools/lint/deps.py` `ALLOWED`, `determinism.py`'s crate list, `deny.toml`,
  `Cargo.toml` (workspace and crate), `Cargo.lock`, `rust-toolchain.toml`,
  `crpg-contracts` and `.github/workflows/`.**

## Dependency edges

No new crate or external dependency. Every type used is already a dependency of
`crpg-godot`:

| Edge | Section | Part | `ALLOWED` change |
|---|---|---|---|
| `crpg-godot` → `crpg-net` (`pool_v4`, `replica_v4`, `outbox_v4`, `movement_v4`, `protocol_v4`, `codec_v4`, `protocol_v3`) | `[dependencies]` (normal, path, already present) | T151a | none: the `crpg-godot` row allows every workspace crate (`tools/lint/deps.py:71`) |
| `crpg-godot` → `crpg-server` (`Host`, `bind_v4_peer`, `ingest_move`, `take_move_receipts`, `take_samples`, `FactGrants`, `PauseRights`) | `[dependencies]` (normal, path, already present) | T151a | none (same row) |

`crpg-godot` already depends on `crpg-net` and `crpg-server`
(`crates/crpg-godot/Cargo.toml:16–17`); the v4 modules are inside `crpg-net`
and the v4 host surface is inside `crpg-server`, so no manifest, lock or
`ALLOWED` edit is made. `deps.py`'s godot rule (`tools/lint/deps.py:12`) and
unsafe rule (`:16–17`) are untouched; the crate stays the only godot/unsafe
holder.

## Out of scope

- **T049 interpolation (G1)** — `previous_positions()`, the per-entity sample
  buffer, the U7 render delay and linear blend
  (`docs/adr/0036-movement-and-navigation.md:446–451`).
- **T026 prediction (G2)** — client-side route prediction for the controlled
  actor, the mesh-revision gate (`:453–458`).
- **T050 click-to-move (G3)** — camera, picking, ground snap, the `crpg-godot` →
  `crpg-nav` edge, and any `SceneSync.gd`/`CombatHud.gd` change that consumes
  `positions()` (`:460–463`).
- **The `crpg-net-quic` lane-1 transport (T150)** (`tasks/T026p.md:1471–1475`);
  the nav revision on the wire; any change to `crpg-net` or the T037 host
  surface (`bind_v4_peer`, `ingest_move`, `take_*`, `DeclareMove`, samples).
- **`legal_intents` gaining `DeclareMove`** (it keeps T047a's v1 vocabulary,
  `tasks/T037.md:1366`); real-time or exploration play; a second human peer.
- **Any code, any other file** beyond the two parts' owned lists.

## Open questions

1. **Does this draft amend ADR-0033 decision 8 ("the link carries lane-0 frames
   only", `docs/adr/0033-godot-bridge.md:108`)?** Reading 2 has the link carry
   `0x82` transfer and `0xA1` lane-1 frames too, routed by first byte. The ADR's
   "lane-0 only" sentence rests on "nothing sends Resync, so a Transfer entry is
   unreachable" (`:229–230`), which v4 falsifies. *Recommend:* approve Reading 2
   and record a dated supersession note on ADR-0033 decision 8 (the link
   multiplexes by subprotocol first byte; the authority boundary is unchanged —
   the client still sees only bytes).

2. **Does T151 own `positions()` on `CrpgView`, or does T049?** ADR-0033
   `:519` says T049 "later adds `positions()` and `previous_positions()` to
   `CrpgView`"; this brief requires T151 to say how positions reach GDScript.
   Reading 4 gives T151 the newest-position columns (named `positions()`,
   `position_known()`, `moving()`) and leaves T049 `previous_positions()` + the
   buffer. *Recommend:* T151 owns the newest-position surface; T049's ADR note
   is superseded to say T049 adds `previous_positions()` and interpolation only.

3. **Which `FactGrants` and `PauseRights` does the bridge pass to
   `bind_v4_peer`?** T037's signature (`tasks/T037.md:439–441`) needs both; the
   bridge's `grants()` today builds only `DisclosureGrants`
   (`crates/crpg-godot/src/core/authority.rs:647–675`). *Recommend:* full fact
   grants mirroring the full `DisclosureGrants` (every bound entity, encounter,
   host facts) and `PauseRights { pause: false, unpause: false }` — the
   single-player slice never pauses from the wire — but the exact
   `FactGrants`/`PauseRights` construction is a detail the final contract must
   pin from T037d's as-built surface.

4. **Is `submit_move(actor, x, y, z)` the right lane-1 surface, or should the
   actor be implicit until T050?** A move needs an actor; ADR §:236 only bans
   `EntityId`, not `NetId`. *Recommend:* keep the explicit `NetId` actor, so
   T050's click-to-move maps a picked proxy to it without a new bridge method.

5. **Does `Moved` belong in `events()` (Reading 5) or a dedicated polled
   getter?** `ViewValue` gains an `Ints` arm either way. *Recommend:* `events()`
   (the HUD already consumes it; the fixture produces no `Moved`, so no HUD case
   changes).

## Agent log

- 2026-11-15 (UTC) · opencode-go/deepseek-v4-pro + T151 draft a · Blind draft of the `crpg-godot` bridge move to protocol v4 (`ReassemblyPoolV4`, `ReplicaWorldV4`, `MoveOutbox`), so the bridge opens a v4 session, sees positions and `Moved`, and sends lane-1 moves through the embedded server, leaving T049/T026/T050 room without doing them.

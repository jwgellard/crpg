# T151-draft-b — blind contract draft: `crpg-godot` bridge move to protocol v4

> Blind draft. Nothing here is approved: a Claude architect writes the final
> contract and the user approves it. Built after T037d from
> `tasks/T151-facts.md`; every `path:line` was re-read in this slot. A bare
> `:N` is a line of the file cited earlier in the same item.

## Task

Move the `crpg-godot` bridge from its v2 session to a protocol v4 session so
a Godot client sees positions and sends moves through the embedded server.
Today `BridgeCore::open` builds the host with
`HostConfig { protocol: ProtocolSelection::V2, incarnation: 1 }`
(`crates/crpg-godot/src/core/mod.rs:302`), the client half holds one v2
`ReplicaWorld` (`crates/crpg-godot/src/core/client.rs:292`) decoding with
`codec_v2::decode_delta` (`crates/crpg-godot/src/core/client.rs:322`), and the
view has no position member (`crates/crpg-godot/src/core/client.rs:28–45`)
because the v1/v2 wire has "no transform op"
(`crates/crpg-net/src/protocol.rs:305`). After this task the bridge binds
with the T037 future `Host::bind_v4_peer` (`tasks/T037.md:439–441`), the
client half feeds `ReassemblyPoolV4::pop_update`
(`crates/crpg-net/src/pool_v4.rs:656`) into `ReplicaWorldV4`
(`crates/crpg-net/src/replica_v4.rs:55`) plus `MoveOutbox`
(`crates/crpg-net/src/outbox_v4.rs:54`), and GDScript reads granted positions
and sends `MoveTo`/`Halt` through new polled getters and one new submit call.
T049 interpolation (G1), T026 prediction (G2) and T050 click-to-move (G3) are
later tasks: this one exposes the data and the send path without doing them
(`tasks/T026p.md:1477–1479`; `tasks/T037.md:1356–1358`).

## Parts

Order: **serial** A → B → C → D. Each part is one crate (or client) and one
implementer run. Parts A and B stay below the GDExtension boundary and are
not S-set (no wire byte, version or tick order is fixed here; v4 bytes are
T026p's). Part C touches the GDExtension boundary
(`docs/IMPLEMENTATION_PLAN.md:86–98`, "the GDExtension boundary in
`crpg-godot`" at `:89–95`) and is S-set: one at a time and human-reviewed.
Part D is GDScript only, a separate part per the brief, not S-set.

| Part | Scope | Size (source + tests) | S-set | Review |
|---|---|---|---|---|
| T151a | `crpg-godot` core client half: pool, replica, outbox, view, submit | ~200 + ~400 | no | reviewer, one fix round |
| T151b | `crpg-godot` core authority half: v4 bind, samples, receipts, hand-off | ~150 + ~350 | no | same |
| T151c | gdext shell: new `#[func]`s and view getters | ~80 + ~120 | **yes** (boundary) | same + human |
| T151d | `apps/client` GDScript: read positions, send moves, runner cases | GDScript + cases only | no | same |

### T151a owns

- `crates/crpg-godot/src/core/client.rs`
- `crates/crpg-godot/src/core/client/tests.rs`
- `crates/crpg-godot/src/core/tests.rs` (new v4 cases only)
- `docs/architecture/crpg-godot.md`, `crates/crpg-godot/AGENTS.md`
- goldens: none new, none moved.

### T151b owns

- `crates/crpg-godot/src/core/authority.rs`
- `crates/crpg-godot/src/core/mod.rs` (bind + wiring only)
- `crates/crpg-godot/src/core/authority/tests.rs`
- `docs/architecture/crpg-godot.md`, `crates/crpg-godot/AGENTS.md`
- goldens: none new, none moved.

### T151c owns

- `crates/crpg-godot/src/shell.rs` (new `#[func]`s and getters only)
- `crates/crpg-godot/tests/bridge.rs` (public-API cases only)
- `docs/architecture/crpg-godot.md`
- goldens: none new, none moved.

### T151d owns

- `apps/client/scripts/Main.gd`, `apps/client/scripts/SceneSync.gd`
- `apps/client/tests/scene_sync_cases.gd`, `apps/client/tests/smoke.gd`
- `docs/architecture/crpg-godot.md`
- goldens: none new, none moved.

## Readings

1. **The bridge opens a v4 session, not a translated v2 one.** `open` binds
   the player with the T037 future `bind_v4_peer` (`tasks/T037.md:439–441`)
   and builds the client replica from `host.epoch(player)` as today
   (`tasks/T151-facts.md:164`). Rejected: keeping the `ProtocolSelection::V2`
   session (`crates/crpg-godot/src/core/mod.rs:302`) and translating v4 pages
   in the bridge. The v4 delta codec and `Position` ops do not exist in v2, so
   translation would re-implement `codec_v4` in the wrong crate.
2. **Positions reach GDScript as polled view getters, not signals.** ADR-0033
   fixes "no signals in the slice" (`tasks/T151-facts.md:22`) and `Main.gd`
   already polls once per `_process` (`tasks/T151-facts.md:185`), so the view
   gains `positions()`-style getters read after `poll`. Rejected: emitting a
   signal per sample. Signals would break the once-per-poll revision gate and
   contradict the approved FFI surface.
3. **`Moved` rides the existing `ViewEvent` pipeline.** The replica stores
   nothing from `Moved` (`crates/crpg-net/src/replica_v4.rs:186`); a presenter
   pairs the id with `v3().entity()` and treats a miss as not visible
   (`tasks/T026p.md:260`). The view already carries events oldest-first with
   `kind`-keyed entries (`docs/architecture/crpg-godot.md:104–108`), so `Moved`
   becomes one more `kind`. Rejected: a separate `moved()` getter. A second
   channel would duplicate the revision gate and could show a move whose anchor
   has not arrived yet.
4. **Real-time moves leave GDScript through a new `submit_move`, not through
   options.** Lane 1 intents are `MoveTo { seq, actor, destination }` and
   `Halt { seq, actor }` with no other fields (`tasks/T151-facts.md:35`), and
   the bridge's lane-0 `submit` encodes one intent per call
   (`crates/crpg-godot/src/core/client.rs:523`). A parallel `submit_move`
   owns the lane-1 `seq` in `MoveOutbox` (`crates/crpg-net/src/outbox_v4.rs:93`).
   Rejected: smuggling destinations into `submit_option`. Options are the
   host's `legal_intents` vocabulary; destinations are client-chosen integers
   and would corrupt the pending/seq rules of lane 0.
5. **Turn-based `DeclareMove` keeps riding lane 0 through `submit_option`.**
   TB moves are the combat verb `DeclareMove { actor, destination }` whose
   result is a `Moved` projection (`tasks/T151-facts.md:39`), decided by the
   host like any action. Rejected: sending `DeclareMove` over lane 1. Lane 1
   is the real-time subprotocol; TB legality (pools, turn) is checked in the
   action pipeline, and T037c's K2 proves it there
   (`tasks/T037.md:1300–1306`).
6. **Accept, refuse and supersede are visible as polled move receipts, in
   `seq` order.** Every admitted intent gets exactly one receipt, in `seq`
   order (`tasks/T151-facts.md:36`), and `MoveOutbox::receive` completes only
   the oldest pending intent (`crates/crpg-net/src/outbox_v4.rs:113–131`). The
   view exposes them oldest-first like lane-0 receipts. Rejected: callbacks or
   overwriting a "last outcome" cell. Ordering is the contract; a cell would
   lose the supersede-then-accept sequence the tests pin.
7. **Lane-1 bytes travel the unchanged `LocalLink`.** The link is two FIFOs of
   whole `Vec<u8>` frames (`crates/crpg-godot/src/core/link.rs:22–33`) capped
   at `LINK_FRAMES` (`crates/crpg-godot/src/core/link.rs:17`); it stores
   bytes, never decodes. The authority hands `take_samples`/`take_move_receipts`
   bytes down and `ingest_move` bytes up through the same queues. Rejected: a
   second link or direct host access from the client half. A second channel
   would split the FIFO order the tests assert, and direct access would break
   "the client cannot mutate authoritative state"
   (`tasks/T151-facts.md:16`).
8. **The client step demuxes by source, not by sniffing bytes.** Down-queue
   frames from `take_delivery_entries` go to `ReassemblyPoolV4::offer_delta`
   (`crates/crpg-net/src/pool_v4.rs:534`); sample datagrams go to
   `ReplicaWorldV4::apply_sample`
   (`crates/crpg-net/src/replica_v4.rs:148`); lane-1 receipts go to
   `MoveOutbox::receive`. Rejected: one pool for everything. The pool refuses
   non-delta bytes by design, and `apply_sample` never fails by contract, so
   mixing them would turn routine samples into fatal fences.
9. **No new `BridgeCode`: `NotV4Peer` maps through the wildcard.** `host_fault`
   maps `Closed` to 50 and everything else to 51
   (`crates/crpg-godot/src/core/errors.rs:106–110`), and T037 keeps that shape
   compiling (`tasks/T037.md:1412–1416`). Rejected: adding codes 56+. The
   table is closed at 20 stable numbers (`tasks/T151-facts.md:83`); T026p adds
   no `ReplicaError` variant for the same reason (`tasks/T026p.md:1042`).
10. **The NPC keeps its option driver; it does not move.** The NPC reads
    `legal_intents` and encodes the same intent a player would
    (`docs/architecture/crpg-godot.md:88–91`); no T037 item gives the bridge an
    NPC movement policy. Rejected: driving NPC `MoveTo` from the pacer. That
    is AI policy, owed elsewhere, and would pin numbers this contract must
    leave open.

## Interface

New and changed items only. Everything else keeps its shape, including the
`BridgeCode` table (`crates/crpg-godot/src/core/errors.rs:12–53`) and the
`CrpgSession::open/poll/view/submit_option/close` set
(`tasks/T151-facts.md:109`).

```rust
// core/client.rs (T151a): the v4 client half. `client.rs` still names
// neither `crpg_server` nor `crpg_sim` (tasks/T151-facts.md:16).
pub(crate) struct Client {
    pool: crpg_net::pool_v4::ReassemblyPoolV4,
    replica: crpg_net::replica_v4::ReplicaWorldV4,
    outbox: crpg_net::outbox_v4::MoveOutbox,
    view: Rc<SessionView>,
    offered: Vec<OfferedOption>,   // as today
    pending: Option<u64>,          // lane-0 pending, as today
    next_seq: u64,                 // lane-0 seq, as today
}
impl Client {
    pub(crate) fn new(epoch: crpg_net::SessionEpoch, table: PresentationTable) -> Self;
    /// Drain down queue: delivery entries -> pool -> replica; samples ->
    /// `apply_sample`; lane-1 receipts -> `outbox.receive`. Rebuild view.
    pub(crate) fn step(&mut self, down: &mut VecDeque<Vec<u8>>) -> Result<(), Fault>;
    /// Lane-0 submit, unchanged rules.
    pub(crate) fn submit(&mut self, revision: u64, index: usize, up: &mut VecDeque<Vec<u8>>) -> Result<(), Fault>;
    /// Lane-1 submit: `outbox.send(actor, body)`, queue canonical bytes up.
    /// `Full` maps to code 30 and changes nothing; success advances
    /// `outbox.next_seq`.
    pub(crate) fn submit_move(&mut self, actor: crpg_net::NetId, body: crpg_net::movement_v4::MoveBody,
        up: &mut VecDeque<Vec<u8>>) -> Result<crpg_net::movement_v4::MoveIntent, Fault>;
}

// core/client.rs (T151a): view additions. Slices stay parallel by `NetId`.
pub(crate) struct ViewMoveReceipt { pub seq: u64, pub outcome: MoveOutcomeView, pub processed_tick: u64 }
pub(crate) enum MoveOutcomeView { Accepted, Superseded, Rejected { code: u8 } }
impl SessionView {
    /// Granted positions, ascending `NetId`: `(id, [x, y, z] raw ints, moving)`.
    pub(crate) fn positions(&self) -> &[(crpg_net::NetId, [i32; 3], bool)];
    /// This step's lane-1 receipts, oldest first.
    pub(crate) fn move_receipts(&self) -> &[ViewMoveReceipt];
}
```

```rust
// core/authority.rs + core/mod.rs (T151b)
impl Authority {
    /// Player binds with `bind_v4_peer` (tasks/T037.md:439–441); NPC keeps
    /// `bind_peer` (tasks/T151-facts.md:163).
    pub(crate) fn new_v4(/* as Authority::new today, tasks/T151-facts.md:165 */) -> Self;
    /// Each poll, after `pump`: `take_samples(player)` and
    /// `take_move_receipts(player)` bytes are handed down the link;
    /// up-queue bytes go to `ingest_move` (tasks/T037.md:483–487).
    /// `DeliveryKind` needs no new arm: v4 pages arrive as `Delta`
    /// (tasks/T037.md:447–448).
}
```

```rust
// shell.rs (T151c, S-set): one core call per #[func], as today
// (tasks/T151-facts.md:246).
#[func] fn submit_move(&mut self, actor: i64, x: i32, y: i32, z: i32) -> i64;
#[func] fn submit_halt(&mut self, actor: i64) -> i64;
// CrpgView getters, parallel arrays in ascending NetId order:
#[func] fn position_ids(&self) -> PackedInt64Array;
#[func] fn position_points(&self) -> PackedInt64Array;  // x, y, z per id
#[func] fn position_moving(&self) -> PackedInt64Array; // 0 / 1 per id
#[func] fn move_receipts(&self) -> Array<VarDictionary>; // seq, outcome, code, processed_tick
```

## Behaviour

Normative. "Refused" means the stated code and no state change. Tests are in
§Tests; every item names its cases.

B1. **Session creation chooses v4.** `open` on the fixture binds the player
    with `bind_v4_peer` and the NPC with `bind_peer`; `session_wire` is 4 for
    the player session. The client replica is `ReplicaWorldV4` built from
    `host.epoch(player)`, and the pool is `ReassemblyPoolV4` built from the v3
    limits. *Tests:* K1, K2.
B2. **Positions reach GDScript through polled getters, with exact names.**
    After a poll that applied a page carrying a `Position` anchor, `view()`
    exposes it in `position_ids` / `position_points` / `position_moving`
    (ascending `NetId`, raw integers, never floats per
    `tasks/T151-facts.md:37`). No signal is emitted. With no granted position
    all three arrays are empty. *Tests:* K3, K4.
B3. **`Moved` reaches GDScript as a `ViewEvent` of kind `"moved"`.** The
    replica stores nothing (`crates/crpg-net/src/replica_v4.rs:186`); the view
    pairs the id with `v3().entity()` and drops ids with no row. The event
    joins the same oldest-first `events()` list and the same once-per-poll
    revision bump as every other event. *Tests:* K5.
B4. **A move request leaves GDScript through `submit_move` / `submit_halt`.**
    `submit_move(actor, x, y, z)` encodes one lane-1 `MoveTo` with the outbox's
    next `seq` (starting at 1, `crates/crpg-net/src/outbox_v4.rs:63`) and
    queues its canonical bytes up; only success advances the outbox. Negative
    actor or unknown `NetId` is refused without touching the outbox, matching
    the `submit` prefix rules (`crates/crpg-godot/src/core/client.rs:491`).
    *Tests:* K6, K7.
B5. **Accept is visible as a move receipt.** When the host releases an
    `Accepted` receipt for the oldest pending intent, the next poll shows
    `{seq, Accepted, processed_tick}` in `move_receipts()` oldest-first and
    the outbox drops that intent (`processed_seq` advances). *Tests:* K8.
B6. **Refuse is visible and keeps the window.** A `Rejected(code)` receipt
    completes its intent and shows `{seq, Rejected, code}`; a `Superseded`
    receipt shows `{seq, Superseded}`. Neither changes any other pending
    intent. *Tests:* K9.
B7. **Supersede ordering is exact.** Receipts complete only the oldest pending
    intent whatever the outcome
    (`crates/crpg-net/src/outbox_v4.rs:113–131`); an out-of-order or
    foreign-epoch receipt changes nothing and the view shows no receipt for
    it. *Tests:* K10.
B8. **Samples move the exposed position without touching the anchor rules.**
    `apply_sample` stores an entry only for an entity with a disclosed
    position and only when strictly newer
    (`crates/crpg-net/src/replica_v4.rs:144–147`); the getters then report the
    sampled point. A sample for an undisclosed entity changes nothing.
    *Tests:* K11.
B9. **Existing v2/v3 behaviour and the HUD tests keep doing what they do.**
    Lane-0 options, events, receipts, labels, the 14 HUD getters
    (`tasks/T151-facts.md:187`), the `load.gd` / `hud.gd` / `scene_sync.gd` /
    `smoke.gd` exit codes (`tasks/T151-facts.md:190–198`), and the proxy
    layout derived from `net_ids()`/`sides()` only
    (`tasks/T151-facts.md:186`) are unchanged. T049/G1 (interpolation), T026/G2
    (prediction) and T050/G3 (click-to-move) are not implemented: proxies do
    not interpolate, the client does not predict, and no picking is added.
    *Tests:* K12 plus every existing suite.
B10. **Error mapping is closed.** `HostError::Closed` → 50, any other
    `HostError` (including `NotV4Peer`) → 51, through the existing wildcard
    (`crates/crpg-godot/src/core/errors.rs:106–110`). A corrupt lane-0 frame
    fences 52/53 as today; an undecodable lane-1 receipt for the oldest
    pending intent fences the same way a refused lane-0 frame does and keeps
    the last revision. `is_fatal` stays exactly 50–55
    (`tasks/T151-facts.md:83`). *Tests:* K13.

## Errors

No new `BridgeCode`, no new `Display` text (Reading 9). The closed table and
`name()`/`is_fatal()` are untouched
(`crates/crpg-godot/src/core/errors.rs:12–53`,
`crates/crpg-godot/src/core/errors.rs:62`,
`crates/crpg-godot/src/core/errors.rs:88–90`).

| Source | Mapping | Detail |
|---|---|---|
| `HostError::Closed` anywhere | 50 `HostClosed` | as today |
| `HostError::NotV4Peer` and every other `HostError` | 51 `HostFailed` | wildcard arm, `error.to_string()` |
| `MoveOutbox::send` → `Full` (8 pending, `tasks/T151-facts.md:140`) | 30 (notice, processing continues) | link-full precedent, queue unchanged |
| `MoveOutbox::send` → `Exhausted` | 51 `HostFailed`, fence | `seq` space is worn out; unrecoverable |
| undecodable lane-0 frame | 52, fence | as today |
| replica refusal (`UnknownEntity`, version) | 53, fence | as today |
| `DeliveryKind::Transfer` on a v4 session | 54 `UnexpectedTransfer`, fence | same arm as today (`crates/crpg-godot/src/core/authority.rs:62–67`); unreachable because the player binds with `bind_snapshot_peer` semantics (`tasks/T151-facts.md:20`) |
| `submit_move` negative/unknown actor | 3 / 6 | same argument/index rules as `submit` |
| panic at any boundary | 55 `PANICKED` | same guard as today |

Lane-1 receipt bytes that fail to decode for the oldest pending intent fence
like a refused lane-0 frame (B10); receipts for a non-oldest `seq` or a
foreign epoch are ignored without a code (B7).

## Tests

In `core/tests.rs` / `client/tests.rs` style (21 + 12 cases today,
`tasks/T151-facts.md:208–214`): fixture opens, crafted v4 bytes where the
host cannot produce them yet, `open_and_poll`-shaped helpers. Each case
states its control and what failure looks like.

- **K1 `v4_session_binds`.** Open the fixture, poll once. Expected: the
  player session reports `session_wire == 4`, the pool holds the v3 limits,
  and the replica epoch equals `host.epoch(player)`. Control: the same open
  on master reports wire 2. *Fails if:* wire is not 4.
- **K2 `transfer_is_still_a_fence`.** Hand a `Transfer` entry to `hand_off`.
  Expected: code 54 with the ordinal in the detail, state `Fenced(54)`.
  Control: a `Delta` entry is sent. *Fails if:* the code is not 54 or a later
  `poll` does not repeat 54.
- **K3 `anchor_reaches_the_view`.** Feed one v4 delta page carrying
  `Position { hero NetId 1: Some(P(32768,0,32768), moving true) }`, poll.
  Expected: `position_ids == [1]`, `position_points == [32768, 0, 32768]`,
  `position_moving == [1]`, revision bumped by exactly 1. Control: a page with
  no `Position` op leaves all three arrays empty. *Fails if:* any array
  differs or the revision moved twice.
- **K4 `no_position_means_empty_arrays`.** Fresh open plus one poll on the
  fixture (whose pages carry no anchors). Expected: all three arrays empty and
  the same `Rc` kept when nothing else changed. Control: K3. *Fails if:* a
  non-empty array or a rebuilt view.
- **K5 `moved_is_an_unchecked_notice`.** Feed a delta with `Moved { NetId 99 }`
  (no such row): no event. Then `Moved { hero }`: exactly one event with
  `entries()["kind"] == "moved"`, oldest-first, in the same poll's revision.
  Control: the unknown id. *Fails if:* the unknown id produces an event or the
  known id produces zero or two.
- **K6 `submit_move_sends_seq_1`.** `submit_move(hero, 65536, 0, 65536)` on a
  fresh session. Expected: intent `seq == 1`, one frame queued up, `pending`
  length 1, `next_seq == 2`; a second call sends `seq == 2`. Control: the
  second call. *Fails if:* the first `seq` is not 1 or the queue gained zero
  or two frames.
- **K7 `submit_move_refusals_change_nothing`.** `submit_move(-1, …)` → code 3;
  `submit_move(NetId 99, …)` → code 6; both leave `pending` empty and
  `next_seq == 1`. Control: K6. *Fails if:* a refusal returns 0 or moves the
  outbox.
- **K8 `accept_advances_the_outbox`.** After K6, feed a lane-1 receipt
  `Accepted { seq 1, processed_tick 101 }`, poll. Expected:
  `move_receipts == [{seq 1, Accepted, tick 101}]`, `pending` empty,
  `processed_seq == 1`. Control: no receipt leaves `move_receipts` empty.
  *Fails if:* the receipt is missing or the intent is still pending.
- **K9 `refuse_and_supersede_show`.** Two intents (`seq` 1, 2), then receipts
  `Rejected(code)` for 1 and `Superseded` for 2. Expected: both shown
  oldest-first with the exact code byte fed in, `pending` empty. Control: K8.
  *Fails if:* order flips or the code byte differs.
- **K10 `out_of_order_receipt_is_ignored`.** One pending intent (`seq` 1);
  feed a receipt for `seq` 2. Expected: no view receipt, `pending` still length
  1, `processed_seq == 0`. Control: the receipt for `seq` 1 completes (K8).
  *Fails if:* anything completes or appears.
- **K11 `sample_moves_the_exposed_point`.** Anchor hero at `P(0,0,0)`,
  `moving == false`; apply sample `{server_tick 7, point P(46340,0,46340)}`
  (the T037 K3 tick-1 shape, `tasks/T037.md:1310–1312`); then a sample for
  undisclosed `NetId 2` at tick 8. Expected: getters show
  `[46340, 0, 46340]`, clock 7; the second sample changes nothing. Control:
  pre-sample getters show the anchor. *Fails if:* the point or clock differs.
- **K12 `hud_and_proxies_keep_working`.** Full fixture playthrough to the ended
  view: `proxy_count() == net_ids().size() == 2` (the smoke exit-0 condition,
  `tasks/T151-facts.md:196`), the 14 HUD getters read, `hud.gd` and
  `scene_sync.gd` pass. Control: the same run on master. *Fails if:* any
  runner exits non-zero.
- **K13 `corrupt_lane1_receipt_fences`.** Garbage receipt bytes for the oldest
  pending intent. Expected: fence 52, view keeps its last revision, the next
  `poll` repeats 52, `close` resets to `Closed`. Control: K8's valid receipt.
  *Fails if:* the code is not 52 or a later `poll` returns 0.

## What must not change

- **Goldens**, byte for byte (none added), as T037 pins them
  (`tasks/T037.md:1319–1330`):
  `crpg-testkit/goldens/combat_basic_…`, `replay_basic_…`, `srd_lite_…` (both
  targets each), `crpg-sim/tests/goldens/history_v1/v2/v3`, `movement_v1`,
  `surface_v1` (both targets each).
- **Every existing `crpg-godot` test file and case**, byte for byte and
  passing: the 21 cases in `core/tests.rs`, 7 in `authority/tests.rs`, 12 in
  `client/tests.rs`, 2 in `errors/tests.rs`, 3 in `link/tests.rs`
  (`tasks/T151-facts.md:208–221`), plus `tests/bridge.rs` and
  `tests/source_rules.rs` (`tasks/T151-facts.md:222–226`).
- **Bytes and versions:** every v1, v2, v3, `0x81`, `0x82` byte a session
  receives; checkpoint versions; `HISTORY_VERSION`; `World` and tick order
  (`tasks/T037.md:1342–1344`).
- **Public shapes:** every existing public item of `crpg-godot` and
  `crpg-net`, every `Display` text, the 20-code `BridgeCode` table and
  `is_fatal() == 50..=55`.
- **The HUD contract:** the 14 getters the HUD may read
  (`tasks/T151-facts.md:187`), the `load.gd` / `hud.gd` / `scene_sync.gd` /
  `smoke.gd` exit codes (`tasks/T151-facts.md:190–198`), and the client-only
  proxy layout (`tasks/T151-facts.md:25`).
- **Existing-test edits: none**, in any part.

## Dependency edges

No new edge in any part, and `tools/lint/deps.py` / `ALLOWED` are not edited
by any part.

| Edge used | Section | Part | `ALLOWED` change |
|---|---|---|---|
| `crpg-godot` → `crpg-net` (`pool_v4`, `replica_v4`, `outbox_v4`, `movement_v4`) | `[dependencies]` (path, exists) | T151a | none: the row permits every workspace crate (`tasks/T151-facts.md:243`) |
| `crpg-godot` → `crpg-server` (`bind_v4_peer`, `take_samples`, `ingest_move`, …) | `[dependencies]` (path, exists) | T151b | none (same row); `crates/crpg-godot/Cargo.toml:13–19` already lists all five |
| `crpg-godot` → `godot` (optional, `engine`) | `[dependencies]` (exists) | T151c | none; no bump, no default change |
| `apps/client` GDScript → `CrpgSession`/`CrpgView` | GDExtension runtime only | T151d | none (not a Cargo edge) |

No external crate and no new lockfile package. `crpg-godot` already depends
on `crpg-core`, `crpg-data`, `crpg-net`, `crpg-server` and `crpg-sim`
(`crates/crpg-godot/Cargo.toml:13–19`), so the v4 types arrive over existing
edges. No lower crate imports the bridge back: `crpg-testkit` and
`crpg-server` both exclude `crpg-godot` (`tasks/T151-facts.md:244`).

## Out of scope

- **T049 interpolation (G1).** Each entity's sample buffer, rendering at the
  newest tick minus the U7 delay, interpolating after `to_f32`
  (`tasks/T151-facts.md:41`). Proxies show the newest granted point as-is.
- **T026 prediction (G2).** Own-movement prediction and reconciliation, gated
  on the announced baked mesh revision (`tasks/T151-facts.md:42`); only for
  actors the peer controls, never TB.
- **T050 click-to-move UI (G3).** Camera, picking a ground point, the `snap`
  preview and sending `MoveTo`/`DeclareMove` by mode
  (`tasks/T151-facts.md:43`). T151d exposes the calls; no picking is added.
- **The `crpg-net-quic` lane-1 transport (T150).** The lane-1 reliable stream
  pair, DATAGRAM extension, and the `quic.rs` route binding v4 sessions
  (`tasks/T037.md:1351–1355`). The bridge stays in-process over `LocalLink`.
- **The nav revision on the wire.** Reading 4's `crpg-net` task, owed before
  T026 (`tasks/T037.md:1359`).
- **Any change to `crpg-net`.** No new op, code, limit or `Display` text; the
  v4 types are used as T026p built them.
- **Any change to the T037 host surface.** No new `Host` method, error,
  policy constant or byte; `NotV4Peer` arrives with T037d.
- **`legal_intents` gaining `DeclareMove`** (`tasks/T037.md:1366`); grid
  snapping (S2–S5); terrain, facing; egress/ingress cap changes.

## Open questions

## Agent log

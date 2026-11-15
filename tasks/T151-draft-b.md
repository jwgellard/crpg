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

## Errors

## Tests

## What must not change

## Dependency edges

## Out of scope

## Open questions

## Agent log

# T024h draft — server sender/host requirements and seamless narrowing

Status: **blind contract draft** for the architect's comparison; not approved,
no code. One crate: `crpg-server`. It answers T024 S§11 (`tasks/T024.md:885-909`)
and S§10.7 as a host contract, with no `crpg-net` wire change, no dependency
and no golden. Below, `S§N` cites `tasks/T024.md`, `H§N` this draft, and
`path:line` the branch as measured.

## H§0. Inputs read

Root `AGENTS.md`; `tasks/T024h-facts.md`; `tasks/T024.md` (S§10, S§11, S§15,
S§16, approval); `tasks/T025b.md`; ADR-0020/0021/0022/0028;
`docs/architecture/crpg-server.md`; `crates/crpg-server/AGENTS.md`;
`crates/crpg-server/src/{host,quic,checkpoint}.rs`;
`crates/crpg-server/tests/{host_capture,host_quic}.rs`;
`crates/crpg-net/src/{protocol_transfer,codec_transfer}.rs`.

## H§1. What the code does today (measured)

1. One delivery queue per session: `Session.log: VecDeque<LoggedFrame>`,
   `log_bytes`, `next_event_seq`, `delivered`, `generation`, `resync_pending`
   (`crates/crpg-server/src/host.rs:373-378`); `LoggedFrame` is
   `{ event_seq, bytes }` (`host.rs:348-353`).
2. Narrowing in `Host::set_grants` (`host.rs:699-712`) clears the log, bumps
   `generation` (`:706`), **resets `next_event_seq = 1`** (`:707`), resets
   `delivered = 0` (`:708`), sets `resync_pending` (`:709`), calls
   `try_resync` (`:710`) and returns `GrantUpdate::RebindRequired` (`:711`).
   The QUIC adapter does not call it on narrowing; it fences
   (`crates/crpg-server/src/quic.rs:1110-1112`,
   `crates/crpg-server/AGENTS.md:156-159`), because bytes already handed to
   QUIC cannot be recalled (ADR-0022 §Decision 2).
3. A `resync_pending` session is **skipped** for deltas and receipts
   (`host.rs:1215`, `:1075`, `:1356`), so the fresh state repairs the gap.
4. Ack is by `event_seq`: `acknowledge_delivery(peer, through)` pops
   `event_seq <= through` (`host.rs:1449-1461`); the adapter acks
   `acked + accepted`, one frame per sequence (`quic.rs:899`).
5. The filtered facts exist only as ops: `PeerView::full_state_ops`
   (`host.rs:2032-2054`), `health` (`:1994-2003`), `turn` (`:2017-2028`), map
   (`plan_map`, `:1796-1822`). `SnapshotBody` is never built here (fact sheet
   §8).
6. Sessions are not checkpointed (`crates/crpg-server/src/checkpoint.rs:3-10`);
   a load makes fresh bindings and epochs.
7. The T022 narrowing test asserts `frames[0].event_seq == 1` and a later
   delta at 2 (`tests/host_capture.rs:1508`, `:1532`); the QUIC test asserts
   `FenceCause::Narrowed` (`tests/host_quic.rs:1654-1671`).

## H§2. Decisions (options, consequences, recommendation)

D1 **Trigger set.** (a) Snapshot on narrowing and on upstream `Resync` only;
widening stays incremental and bind keeps `state_frames` — recommended.
(b) Every grant change: wastes a transfer on widening and breaks ADR-0022's
widening continuity. (c) Also at bind: a fresh epoch has no prior replica and
would make every bind depend on T024q's channel; that is T025a's
cross-connection install. Recommend (a).

D2 **Transfer ids.** (a) One `next_transfer` per session (epoch), starting at
1, strictly increasing, never rewritten — recommended; a rebind is a new epoch
with a new receiver, so restarting at 1 is safe (S§11.1 is per epoch).
(b) Host-global: legal but grows across peers and differs after a load for no
benefit. (c) Derived from `event_seq`: two captures in one tick collide.
Recommend (a); ids are allocated at staging, so a coalesced request burns none.

D3 **Ledger shape.** (a)
`DeliveryFrame { ordinal: u64, event_seq: Option<u64>, bytes }` — recommended;
`Some` is a delta, `None` a transfer frame, and the ordinal is the total-order
position. (b) Keep `Vec<Vec<u8>>` plus a side position: the adapter would have
to decode to count deltas, i.e. sniffing, forbidden by ADR-0019. (c) A second
queue plus an index: two queues can interleave wrongly and still need a
positional ack. Recommend (a); it is also the channel fact T024q needs without
inspecting payloads.

D4 **Ack unit.** (a) An ordinal watermark: `acknowledge_delivery(peer,
through)` retires staged entries with `ordinal <= through` — recommended;
`event_seq` cannot serve (H§5.4 counterexample). (b) An entry count: breaks
when the host clears the log (unacked entries vanish unacked), which this
contract does. (c) A second `acknowledge_frames(n)`: two watermarks on one
queue. Recommend (a); the adapter acks `frames[accepted - 1].ordinal`.

D5 **Staging point.** (a) The head of `Host::pump`, before admission —
recommended. (b) Inside `set_grants`/`ingest_transfer`: also sound (no delta
can be appended in between) but splits the capture instant and the room check.
(c) In `take_delivery` like `try_resync` today: unsound, because a later pump
can append a delta above the cursor before the `Begin` is staged. Recommend
(a); the two triggers only record a request.

D6 **Unacked log at staging.** (a) Clear all of it, then append the transfer
— recommended: one rule, matches T022 narrowing, and frees the frame room the
two new entries need. Narrowing also clears immediately in `set_grants`, so no
old frame can be taken before the pump. (b) Keep `<= cursor` on `Resync`:
preserves unacked events but a full 128-entry log can then fail `room`, and
adds a second ordering case. (c) Keep all: same room problem and stale frames.
Cost of (a): the peer's unacked **events** for that moment are not re-sent
(never retractable); unacked receipts recover via the retry cache at a new
ordinal. Q4 offers (b) if event preservation is preferred.

D7 **Segments per chunk.** (a) 8 (one 32,768-byte chunk) — recommended: a
valid v1 body is at most 22,552 bytes (S§1.4), so `chunk_count == 1` for every
body this sender can build; a transfer is `Begin` + one chunk (two entries)
and no delta can interleave inside it. (b) 1 (4,096 bytes): 6 chunks for a
maximal body, 7 entries and an interleave window for no benefit here; the
multi-chunk path is proved in `crpg-net` S§12 cases 10 and 13. (c) Adaptive:
an extra branch with no measured reason. Recommend (a); H§9 T3 pins it.

D8 **Cancel.** (a) `Aborted` when narrowing supersedes a transfer captured
under wider grants; **no Cancel** when an upstream `Resync` retires it, because
the receiver says it already failed it (S§10.3-10.4) — recommended. This
sender emits no `Superseded`: it has no supersession without a grant change
(a pending request coalesces, a `Resync` is the receiver's own failure), so
`Superseded` stays reserved for T025a's resume; the pool treats both reasons
identically (`Cancelled`). (b) Always `Aborted` even on `Resync` would claim a
revocation that did not happen. (c) No Cancel, rely on the higher id: the
receiver keeps the old transfer in flight and refuses the new `Begin` (S§10.3
step 3). Recommend (a); a Cancel is staged even if no old chunk was acked
(21 bytes, always legal).

D9 **Overflow (S§11.7).** (a) Admission backpressure — recommended: the host
never appends past the caps; `accepted_room` already leaves the command staged
(`host.rs:1341-1367`) and a pending snapshot that cannot be staged stops the
pump before any delta (H§5.1, H§7). (b) Cancel at a threshold: extra policy,
unreachable before the frame cap at two transfer entries. (c) Fence: the
adapter's `STALL_TIMEOUT_MS` path (`quic.rs:912-918`) is the terminal case.
Recommend (a), never a skipped delta.

D10 **Facts into the body.** (a) New
`PeerView::snapshot_body(&self, world: &World) -> SnapshotBody` — recommended,
built from the same `map`/`health`/`turn` predicates as `full_state_ops`.
(b) Convert `full_state_ops` back to facts: a second vocabulary and a
re-check of `active`. Recommend (a).

D11 **Surface and `generation`.** (a) Replace `RebindRequired` with
`SnapshotScheduled`; add `DeliveryFrame`, `TransferDisposition`,
`TransferStatus`, `PumpSummary.snapshots_staged`, and
`HostError::{WrongEpoch, UnknownTransfer, MalformedTransfer}`; delete the
private `generation` (`host.rs:377`, `:706`, `:1071`, `:1260`) — recommended
(needs an ADR per `crates/crpg-server/AGENTS.md:17-19`). (b) Overload
`RebindRequired`: its documented meaning is "fence", which would undo the
feature. (c) A private sender with no status: the boundary cannot be tested.
Recommend (a).

D12 **Upstream bytes.** (a) `Host::ingest_transfer(peer, bytes, now_ms)`
decodes `TransferUp` in `host.rs` — recommended. (b) Decode in `quic.rs` and
pass a typed message: transfer knowledge in the adapter, against "policy stays
in the host". (c) Overload `Host::ingest`: lane-0 admission must not sniff.
Recommend (a).

## H§3. Public surface (exact)

```rust
pub struct DeliveryFrame {
    pub ordinal: u64,           // 1..u64::MAX-1, strictly increasing per log
    pub event_seq: Option<u64>, // Some = delta frame, None = transfer frame
    pub bytes: Vec<u8>,
}
pub enum GrantUpdate { Unchanged, SnapshotScheduled } // RebindRequired removed
pub enum TransferDisposition {
    Published { transfer: u64 },
    AlreadyRetired,
    ResyncScheduled,
}
pub struct TransferStatus {
    pub next_transfer: u64, pub in_flight: Option<u64>,
    pub in_flight_cursor: u64, pub retired_through: u64,
    pub snapshot_pending: bool,
}
pub struct PumpSummary { /* existing fields */ pub snapshots_staged: usize }

impl Host {
    pub fn take_delivery(&mut self, peer: PeerHandle) -> Result<Vec<DeliveryFrame>, HostError>;
    pub fn acknowledge_delivery(&mut self, peer: PeerHandle, through: u64)
        -> Result<(), HostError>;
    pub fn ingest_transfer(&mut self, peer: PeerHandle, bytes: &[u8], now_ms: u64)
        -> Result<TransferDisposition, HostError>;
    pub fn transfer_status(&self, peer: PeerHandle) -> Result<TransferStatus, HostError>;
}
```

Private state (not exposed): `DeliveryEntry { ordinal, event_seq: Option<u64>,
bytes }`; `InFlightTransfer { id: TransferId, cursor, base_tick, chunk_count }`;
`enum SnapshotCause { Narrowed, Resync }`; `Session` loses `generation` and
`resync_pending`, gains `log: VecDeque<DeliveryEntry>`, `next_delivery`,
`next_transfer`, `retired_transfer`, `transfer: Option<InFlightTransfer>` and
`snapshot_pending`, and keeps `delivered` as the highest acked ordinal.
`next_event_seq` stays. New types derive `Debug, Clone, PartialEq, Eq`; no
serde; `TransferId` stays internal and the public surface carries `u64`, so no
`crpg-net` type is re-exported.

## H§4. S§11 as server requirements (each with its proof)

R1 (**S§11.1**) Ids strictly increasing per epoch from 1, never reused. Met by
private `Host::stage_snapshot` (`session.next_transfer` → `TransferId::new`,
increment after staging, `CounterExhausted` at `u64::MAX`); reported by
`Host::transfer_status`. Test T1.

R2 (**S§11.2**) `event_seq` never resets; a snapshot's `cursor` is the last
sequence issued before capture and later deltas are above it. Met by
`Host::set_grants` leaving `next_event_seq`/`delivered` untouched and by
`stage_snapshot` using `cursor = next_event_seq - 1`; the transfer push path
never advances `next_event_seq`. Tests T4, T5, T12.

R3 (**S§11.3**) A `Begin` precedes every delta above its cursor, and chunks
follow the `Begin`. Met by staging pending snapshots at the head of
`Host::pump`, before `admit` (the only delta appender), after clearing the log;
transfer entries are appended contiguously. Tests T6, T7, T10.

R4 (**S§11.4**) A new transfer starts only after `Published`/`Resync` for the
previous one, or after sending `Cancel`. Met by
`Host::ingest_transfer` (retires on both) and `stage_snapshot` (stages
`Cancel` first when an unretired transfer exists). Tests T7, T8.

R5 (**S§11.5**) Revocation touching an in-flight transfer sends
`Cancel(Aborted)`, stops that id's chunks and captures under the new grants.
Met by `Host::set_grants`' narrowing branch (clear log, pending `Narrowed`) and
`stage_snapshot` (Cancel then the new Begin in one commit; the old id is
retired in that commit, so no old chunk can follow it). Tests T7, T9.

R6 (**S§11.6**) A body holds only granted facts: presence makes an entity, the
`health` grant adds `health`, `turn_state` adds `turn`, `active` is `None`
unless present. Met by `PeerView::snapshot_body` (H§5.3): entities are exactly
the `map` keys (built by `plan_map` from present, live entities), health is
`PeerView::health`, turn is `PeerView::turn` with `active` mapped through the
same `map`. Tests T2, T3, T9, T12.

R7 (**S§11.7**) Deltas after capture wait in egress under the existing caps;
overflow means Cancel and a fresh snapshot, or a fence; never a skipped delta.
Met by deleting every `resync_pending` skip (`host.rs:1075`, `:1215`, `:1356`),
by `room`/`accepted_room` counting transfer entries like any other, and by
`pump` stopping before admission when a pending snapshot has no room. Tests
T10, T11.

R8 (**S§11.8**) `plan_transfer` produces canonical frames; a host that
encodes itself must match. Met by `stage_snapshot` calling
`crpg_net::codec_transfer::plan_transfer` (`codec_transfer.rs:488-497`) and
never encoding a transfer frame by hand. Test T3.

## H§5. Sender algorithms

### H§5.1 Staging

At `Host::pump`'s head, in ascending `PeerHandle` order, for each session with
`snapshot_pending`, check first and commit only on success:

1. Plan (no mutation): if `transfer == Some(old)` and the cause is `Narrowed`,
   the entries start with `encode_down(Cancel { epoch, old.id, Aborted })`;
   for `Resync` there is no Cancel (the receiver already failed it, D8).
2. Candidate `id = TransferId::new(next_transfer)`; `next_transfer == u64::MAX`
   → `CounterExhausted`.
3. `cursor = next_event_seq - 1`; `base_tick = server_tick()`;
   `body = session.view().snapshot_body(self.history.world())`.
4. `frames = plan_transfer(epoch, id, base_tick, cursor, 8, &body)?`; assert
   `frames.len() == 1 + begin.chunk_count`.
5. Build the entries (Cancel if planned, then frames), each with
   `event_seq: None`. Check room against the **cleared** ledger (the old
   entries are dropped in the same commit): if it does not fit, return "not
   staged" changing nothing (no retirement, no id burned, no clearing); the
   request stays and the pump stops.
6. Commit: clear `log` and subtract `log_bytes` from `egress_bytes`; push the
   entries (`push_transfer_frames` assigns ordinals and
   `log_bytes`/`egress_bytes`; no `next_event_seq` change); if a Cancel was
   planned, set `retired_transfer = max(retired_transfer, old.id)`; set
   `transfer = Some(InFlightTransfer { .. })`, `next_transfer += 1`,
   `retired_transfer = max(retired_transfer, id - 1)`,
   `snapshot_pending = None`.

`SNAPSHOT_SEGMENTS_PER_CHUNK: u8 = 8` is a private documented constant (D7).
`plan_transfer` fills `total_bytes`/`entity_count`/`chunk_count`; the 1,024
entity bound makes `chunk_count == 1` (`debug_assert`, but the push loop
handles any `n`).

### H§5.2 Ids, retirement, cancel-before-supersede

- `next_transfer` is per session, starts at 1, is not checkpointed (H§7); a
  new binding is a new epoch and its receiver is per epoch (S§10.1).
- `retired_transfer` is the highest id that may never begin: retired on
  publish, cancel, supersession or failure. Only `Aborted` is emitted (D8);
  `Superseded` is T025a's.
- `Published` for the in-flight id sets `retired_transfer =
  max(retired_transfer, id)`, clears the transfer and returns `Published`;
  a repeat at or below `retired_transfer` is `AlreadyRetired`. A `Resync` with
  `after` above the highest issued id is `UnknownTransfer` and changes
  nothing; otherwise every issued id is retired (`retired_transfer =
  next_transfer - 1`), an in-flight transfer is cleared, and
  `snapshot_pending = Some(Resync)` (coalescing). `ingest_transfer` never
  stages frames.
- While `snapshot_pending` is set, `set_grants` coalesces **both** directions
  into the pending snapshot: install grants/map, keep the request, return
  `SnapshotScheduled`; staging widening ops first would put a delta above the
  cursor before the `Begin` (R3). Widening with a transfer *in flight* is
  safe: its ops are above the cursor, the receiver holds them, and `plan_map`
  mints higher ids.

### H§5.3 `PeerView::snapshot_body`

```rust
fn snapshot_body(&self, world: &World) -> SnapshotBody {
    let mut mapped: Vec<(NetId, EntityId)> =
        self.map.iter().map(|(e, n)| (*n, *e)).collect();
    mapped.sort();
    SnapshotBody {
        entities: mapped.into_iter().map(|(net, entity)| SnapshotEntity {
            entity: net,
            health: self.health(world, entity).map(|h| HealthFact {
                health: h.health, max_health: h.max_health, dead: h.dead,
            }),
        }).collect(),
        turn: self.turn(world).map(|t| TurnFact { active: t.active, round: t.round }),
    }
}
```

Fact for fact the same as `full_state_ops`: presence is `map` membership (so
`present: false` has no entry; health also needs the `health` flag), turn needs
`turn_state`, and `active` names a body entity or is `None`. Ids ascend, as
`encode_body` requires. No World/save bytes, events, receipts, rolls, grants or
raw `EntityId`s can enter.

### H§5.4 Ledger, ack and the cutover order

- `ordinal = next_delivery; next_delivery += 1` (sentinel `u64::MAX`). Deltas
  also take `event_seq = next_event_seq` and advance it; transfer entries take
  `None` and do not.
- `take_delivery` returns every unacked entry in order.
- `acknowledge_delivery(peer, through)` is a no-op at or below `delivered`,
  refuses `through > last ordinal` (`FutureDeliveryCursor`), pops front entries
  while `ordinal <= through`, and sets `delivered = through`.
- Why not `event_seq` (D4): log `[D4, C, D5, C2, D6]`. The adapter hands off
  `D4, C, D5` and stops; an ack "through 5" would pop `C2` too, and a
  delta-only rule would leave `C2` behind the watermark. The ordinal ack is
  exact after a clear; an old stale ack pops nothing until the next real one.
- `CacheEntry.receipt` becomes `Option<u64>` (the staged ordinal);
  `cache_hit` re-stages only while `ordinal > delivered` (`host.rs:1071-1073`).
  A receipt dropped by a snapshot is re-issued on the next retry above the
  cursor.

## H§6. Seamless narrowing (what replaces the reset)

`Host::set_grants` narrowing, exactly: keep the existing closed/unknown/invalid
precedence and `plan_map` (`host.rs:688-698`, `:693-698`); install grants and
map; subtract `log_bytes` from `egress_bytes` and clear `log`; set
`snapshot_pending = Some(Narrowed)`; leave `next_event_seq`, `next_delivery`,
`delivered` untouched; remove `generation`/`resync_pending`; return
`GrantUpdate::SnapshotScheduled`. This path never returns `QueueFull`: room is
checked at staging, and failure shows up as pump backpressure.

At the next `pump` the host stages `Begin` (cursor = last pre-narrow sequence)
and the chunk, then admits commands; their deltas are `cursor + 1`, … . The
client receives `Begin`, chunks and later deltas above `cursor`; no
`SessionFenced`, no `event_seq` 1 against a live session, no generation change.

Why one total order holds: frames already handed to the transport have
`event_seq <= cursor`; the pool applies them to the pre-narrow base, which the
snapshot replaces, or drops them as `Superseded` after publication
(S§10.6-10.7, `tasks/T024.md:805-829`). Frames the host still held are cleared
and never sent. Frames after `Begin` are `> cursor` and held until publication.
So no delta is applied to the wrong base, twice or skipped, and `event_seq`
never regresses. That is why the T022 fence can go once a channel exists: the
fence stood in for "old deltas look new after a reset"
(`tasks/T024.md:196-206`). The narrowed client's state is replaced; its
unacked pre-narrow **events** are lost (D6), the price of a seamless install.

## H§7. Pump, capture, retirement, boundary

- **`pump`** gains one step before the `while let Some(staged)` loop: stage
  every pending snapshot in ascending handle order; if one cannot be staged,
  set `backpressured = 1` and return without admitting; `snapshots_staged`
  counts successes. The admission loop is unchanged.
- **Capture (ADR-0021/0022).** A snapshot is not a `CapturedRecord` and does
  not read the journal. It is built synchronously from the current `World` at
  staging, so it sees exactly the mutations reflected at or below its cursor.
  `commit_accepted`'s views, bounds and capture order are untouched.
- **Retirement.** Transfer state is session state: `unbind_peer`/`fence`
  removes the session and its `log_bytes` (`host.rs:662-670`), dropping the
  transfer, id and request. Save/load creates fresh sessions (H§1.6), so no
  transfer id/cursor/body survives; the new epoch's receiver starts clean. No
  new cap: one transfer per session, at most two entries, retired on
  Published/Resync/unbind; existing egress caps bound the rest.
- **`quic.rs`** changes only mechanically: `frame.bytes`, and
  `frames[accepted - 1].ordinal` instead of `acked + accepted`
  (`quic.rs:899`). **Narrowing still fences** (`quic.rs:1110-1112` and the
  defensive `SnapshotScheduled` arm of the widening match): no channel selector
  exists until T024q, and a `0x81` frame on the lane-0 stream would be refused.
  T024q replaces `narrowed()` with `host.set_grants` plus a new non-fencing
  `GrantChange` variant and deletes T14.
- **No policy in `crpg-net`:** the host uses only `plan_transfer`,
  `encode_down`, `decode_up`, `TransferId`; trigger, cursor, id, cancel and
  filtering stay in `host.rs`. Tests use `ReassemblyPool` only as a test-local
  receiver.

## H§8. `ingest_transfer` precedence

First failure wins, nothing changes on refusal: (1) `Closed`; (2)
`TimeRegression`; (3) `UnknownPeer`; (4) `decode_up` failure →
`MalformedTransfer`; (5) epoch ≠ `session.epoch` → `WrongEpoch`; (6)
`Published` → retire in-flight / `AlreadyRetired` at or below
`retired_transfer` / else `UnknownTransfer`; `Resync` → `after` above the
highest issued id is `UnknownTransfer`, otherwise retire issued ids, clear an
in-flight transfer, set `snapshot_pending`, `ResyncScheduled`. The three new
errors are client-input refusals and must not poison (`poisons` keeps
`Invariant`/`Closed`/`TimeRegression`, `quic.rs:374-379`); `Display` stays the
uniform `<Variant> at host/admission` with no payload bytes.

## H§9. Tests (numbered; each with controls)

New `tests/host_transfer.rs` (public API + receiver pool), one unit case in
`host.rs`, and two edits. Commands in H§12.

T1 `transfer_ids_strictly_increase_and_never_reuse`. Two narrows with a
command between give ids 1 then 2; `Published(1)` then a narrow gives 2, never
1; `transfer_status` fields match. A rebind (new epoch) starts at 1 while the
old epoch's id 1 is still meaningful. Control: two requests before one `pump`
coalesce into one id. Unit control: `next_transfer = u64::MAX` →
`CounterExhausted` (pinned-dispatch residual).

T2 `body_holds_only_granted_facts`. Hand-author expected bodies for the
three-combatant fixture: full grants; presence only; one health; `turn_state`
with the active entity's presence revoked (`active == None`, `round`
present). Decode `Begin`+chunk (`decode_down`/`decode_body`) and compare.
Negative control: the hidden health value (7) and hidden `NetId` appear
nowhere; compare entity lists by `NetId`, not by count.

T3 `frames_are_canonical_plan_transfer_bytes`. Host bytes equal
`plan_transfer(epoch, id, base_tick, cursor, 8, &body)` byte for byte; `Begin`
first, `segments_per_chunk == 8`, `chunk_count == 1`, every frame ≤ 65,536,
first byte `0x81`, lane 2. Control: `segments_per_chunk = 1` differs; the body
re-encodes identically.

T4 `narrowing_never_resets_event_seq` (replaces
`narrowing_grants_resynchronize_without_leak`, `tests/host_capture.rs:1476`).
Two deltas (1, 2), narrow → `SnapshotScheduled`; log is `Begin` (cursor 2) +
chunk; after publication a command gives `event_seq == 3`, widening continues
at 4. Negative control: no frame `<= 2` after the `Begin`; the sequence is
1,2,3,4 with no repeat.

T5 `seamless_narrowing_hides_revoked_facts`. Narrow health: the body has the
entity without health and a hidden entity absent; later deltas carry neither.
Control: the same command without narrowing does deliver health.

T6 `begin_precedes_every_delta_above_cursor`. Ingest a command, narrow, one
`pump`: log is `Begin`, chunk, delta at `cursor + 1`. Resync variant with 128
unacked entries: cleared, `Begin` first, the staged delta follows. Control: a
second pump with nothing pending adds no transfer frames.

T7 `cancel_before_supersede`. Transfer 1 in flight, then narrow before
`pump`: `Cancel(Aborted, 1)` then `Begin(2)`; the pool retires 1 and a late
chunk is `StaleTransfer`. A second narrow gives `Cancel(Aborted, 2)` +
`Begin(3)`. Control: `Published(1)` then narrow → `Begin(2)` first, no Cancel;
`Resync` while 1 is in flight → no Cancel, `Begin(2)`.

T8 `published_and_resync_retire_ids`. `Published(in flight)` → retires;
repeat → `AlreadyRetired`; `Published(999)` → `UnknownTransfer` with the whole
`TransferStatus` unchanged. `Resync(after Some(0))` → `ResyncScheduled`;
`Resync(after Some(99))` → `UnknownTransfer`; wrong epoch → `WrongEpoch`;
garbage → `MalformedTransfer`; each changes nothing. Control: the correct epoch
and id are accepted.

T9 `revocation_cancels_in_flight_and_rebuilds`. Transfer 1 in flight under
wide grants; narrow health → `Cancel(Aborted, 1)`; transfer 2 lacks health; the
pool publishes 2; the oracle sees none. Control: without the narrow, transfer
1 publishes with health; the old id's chunks are absent after the cancel.

T10 `deltas_wait_in_egress_and_are_never_skipped`. While transfer 1 is in
flight, an accepted command stages a delta at `cursor + 1` after the chunk; the
pool holds it until publication then releases it; the oracle is monotone.
Control: with no snapshot the delta releases immediately. Negative control: the
old `resync_pending` skip is gone.

T11 `pending_snapshot_blocks_admission_instead_of_skipping` (unit, `host.rs`).
With a pending snapshot, raise `self.egress_bytes` to the host cap (private,
same module) to fail `room`: `pump` → `snapshots_staged == 0`,
`backpressured == 1`, `next_event_seq`/`next_delivery`/log unchanged; restore
and pump → staged, delta at `cursor + 1`. Control: with room the same sequence
stages and admits. (Defence in depth: two entries cannot exhaust the caps with
real frames; the test pins dispatch order, not a product path.)

T12 `independent_filtered_replica_oracle`. Scripted fight over two peers with
different grants (widen, narrow, `Resync`, commands, receipts). Feed every
`DeliveryFrame` into a fresh `ReassemblyPool` (`DeltaWire::V2`,
`GateMode::Live`), pop updates, apply to a test-local `BTreeMap`. After every
update: (1) compare each fact by hand with the expected permitted facts from
the fixture and grant table (never `PeerView`, `full_state_ops` or
`project_events`, never a World hash); (2) assert `event_seq` strictly
increases and never lands at or below the last applied sequence or the
published cursor; (3) assert no `NetId` outside the hand-authored expected set
appears. Negative controls: (a) drop a chunk → nothing publishes, earlier
replica intact, oracle reports the view missing; (b) corrupt one health byte in
all duplicates → a decodable wrong body publishes and the oracle mismatches;
(c) apply the same delta twice to the oracle replica → it flags a sequence
regression; (d) feed a delta above the cursor before publication → held, never
applied early.

T13 `snapshot_ledger_is_bounded_and_session_scoped`. Body ≤ 22,552 bytes,
entities ascending; at most two entries per transfer; `log_bytes` returns to 0
after ack; `unbind_peer` drops the transfer and `egress_bytes` returns;
checkpoint → load gives a new epoch, `next_transfer == 1`, no in-flight
transfer. Control: a normal bind/deliver/ack round keeps the same accounting.

T14 `direct_quic_narrowing_still_fences` (existing
`fencing_closes_with_session_fenced`, `tests/host_quic.rs:1591`, assertions
unchanged). Pins H§7: the adapter still returns `GrantChange::Fenced(Narrowed)`
and never calls `set_grants` on narrowing until T024q. This is the documented
transitional state, not a weakening.

## H§10. Boundary

- **T024q (`crpg-net-quic`).** The transfer channel on the ordered stream
  under ALPN `crpg-lane0/2` with a framing selector (T024 Q3, approved
  2026-10-24); routing upstream bytes to `ingest_transfer` and tracing that
  call; the client `ReassemblyPool`; switching `set_invitation_grants` to
  `set_grants` plus a new non-fencing `GrantChange` variant, deleting T14 and
  the trap; real-transport order/flow-control tests (S§1.5).
- **T025a (`crpg-net`).** Proof-bearing resume, fencing generations, snapshot
  install across connections/epochs, receiver lifecycle; no resume claim here.
- **T025b (`crpg-server`).** 30 s grace, frozen input with continuing
  authority, retained epoch/sequences/cache/receipts, expiry, full filtered
  snapshot on resume (reusing this sender), no combat reset
  (`tasks/T025b.md:19-31`).
- **Outside both:** T027b production interest facts (new facts need transfer
  version 0x82), T046 applies `ReplicaUpdate`s, lane-1 movement, compression,
  World/save serialization, any `crpg-net` wire change.

## H§11. S-set, golden, dependency, invariant impact

- **S-set: none.** No `World`/tick order, data schema, workspace manifest,
  `deny.toml`, workflow or GDExtension boundary. `Session` is private,
  uncheckpointed and unhashed; `authority_hash`/`history_hash` are unchanged
  because a snapshot only reads `World` and staging mutates no history, RNG or
  journal.
- **Goldens/replays: none.** Delivery bytes and session order are not hashed;
  no replay file changes; the host-call trace gains a variant only in T024q.
- **Dependencies: none.** The `crpg-net` edge already exists
  (`crates/crpg-server/AGENTS.md:65-79`); `deps.py`'s row
  (`tools/lint/deps.py:69`) is untouched; no manifest/lock/feature edit.
- **Determinism:** `Vec`/`VecDeque`/`BTreeMap` only, sorted by `NetId`; no
  `HashMap`, float or clock beyond injected `now_ms`.
- **Invariants:** host is the only mutator (1); room before mutation, failed
  staging changes nothing (2); the body is built at staging before later
  mutation (3); per entity, per field, deny by default (4); ids never repeat
  (5); the ledger is bounded by the existing caps and retires (6); injected
  time, no I/O (7); no `unsafe` (8). The public-surface change needs an ADR.

## H§12. Docs, ADR, DoD

- `docs/adr/0030-host-snapshot-sender.md` (Accepted with the contract): the
  sender boundary, ordinal ledger, ids/retirement, no-reset rule, T024q
  hand-off; link `tasks/T024h.md` as authoritative.
- `crates/crpg-server/AGENTS.md`: surface (`ingest_transfer`, `take_delivery`
  type), invariants (ordinals, snapshot-pending), replace the two narrowing
  traps (`:117-119`, `:156-159`), add "never reset `event_seq`", "never skip a
  delta for a pending snapshot", "clear before you stage", the T024q hand-off;
  signed entry.
- `docs/architecture/crpg-server.md`: replace "Narrowing grants restart a
  peer's delivery at 1" (`:167`) and "planned: snapshot transfer"
  (`:30`, `:208-209`); signed entry. BACKLOG row by the implementer.
- DoD: `crates/crpg-server/AGENTS.md:81-95` plus
  `cargo test -p crpg-server --test host_transfer --locked`; the existing
  suites change only where H§9 says, plus mechanical `DeliveryFrame` updates
  in their helpers (`host_quic.rs`/`host_save.rs` decode `frame.bytes`; no
  assertion changes), and no test is weakened otherwise.

## Open questions for the user (each with a recommendation)

Q1 **Unfence QUIC in T024h?** Recommend **no**: land and prove the host sender;
`quic.rs` keeps `FenceCause::Narrowed` because no channel selector exists until
T024q (H§7). The alternative sends `0x81` on the lane-0 stream before T024q,
breaking the client's selection and ADR-0019's no-sniffing rule.

Q2 **Accept the surface change?** Recommend **yes** as H§3 (with an ADR). The
alternative keeps `Vec<Vec<u8>>` and makes the adapter count deltas, which
needs decoding and makes the ack wrong after a log clear.

Q3 **8 segments or 1?** Recommend **8** (one chunk, two entries, no
interleaved chunk window; D7). Choose 1 only to exercise the multi-chunk loop
with real fixtures — impossible here: a valid body is at most 22,552 bytes and
this crate has no large-world fixture (no `crpg-data` edge).

Q4 **Clear all unacked frames, or keep `<= cursor` on `Resync`?** Recommend
**clear both** (D6): one rule, frees the room the new entries need, matches
T022. The cost is the peer's unacked events for that moment; receipts recover
via the retry cache. (b) preserves events but can fail `room` and adds a
second ordering case.

Q5 **Trigger set?** Recommend narrowing + `Resync` only; widening stays
incremental, bind keeps `state_frames` (D1). Snapshotting every grant change
breaks ADR-0022's widening continuity; snapshotting at bind is T025a.

Q6 **Overflow policy?** Recommend admission backpressure, pump blocking
before any delta, the adapter stall fence as terminal (D9/R7). Host-side
cancel-at-a-threshold is extra policy with no reachable trigger at two entries
per peer.

Q7 **Carry `ResyncReason` into host state?** Recommend **no**: `after` is all
the host needs; exposing `crpg-net`'s enum in `TransferStatus` would widen the
surface for observability T024q's trace can own.

## Agent log

- 2026-10-30 (UTC) · opencode-go/deepseek-v4.1-flash + T024h draft · Drafted the host sender contract: per-session transfer ids, a filtered `SnapshotBody`, an ordinal delivery ledger with no `event_seq` reset, cancel-before-supersede, pump-head staging, and a 14-case test plan with an independent filtered-replica oracle.

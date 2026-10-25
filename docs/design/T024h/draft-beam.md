# Exact contract — T024h (draft, beam): `crpg-server` sender/host requirements and seamless narrowing

Date: 2026-10-25 (UTC)
Status: **Proposed (draft by opencode-go/beam-free)** — blind contract draft for
comparison by the Claude architect; not approved, no implementation.

Crate: `crpg-server` only. This draft turns T024 S§11 (the eight sender
rules) plus seamless narrowing into a server-side contract. It consumes the
already-approved `crpg-net` transfer subprotocol (ADR-0028, T024 S§0–S§16)
without changing its wire shapes, and it does not move any policy into
`crpg-net`: the net crate stays a codec/reassembly library, the host stays
the only mutator and the only producer of bodies.

## S§0. Inputs read

- `tasks/T024h-facts.md` (fact sheet; every code citation below is from it).
- `tasks/T024.md` S§10, S§11, S§16 and the approval paragraph (D35).
- `tasks/T025b.md` (boundary).
- `docs/adr/0020-authoritative-area-membership.md`,
  `docs/adr/0021-authoritative-host-capture.md`,
  `docs/adr/0022-host-session-capture-retirement.md`,
  `docs/adr/0028-snapshot-transfer-subprotocol.md`.
- `docs/architecture/crpg-server.md`; `crates/crpg-server/AGENTS.md`.
- `crates/crpg-server/src/host.rs`, `crates/crpg-server/src/quic.rs`,
  `crates/crpg-net/src/protocol_transfer.rs`,
  `crates/crpg-net/src/codec_transfer.rs`.

## S§1. Scope and crate

One crate: `crpg-server`. No change to `crpg-net` wire shapes, no new
dependency, no change to `crpg-contracts` or `rust-toolchain.toml`. The
host keeps its five modules (`host`, `capture`, `checkpoint`, `save`,
`quic`); all additions land in `host` (and the `quic` adapter's narrowing
branch). `crpg-server/AGENTS.md` "Public surface" and trap list are amended
by the implementation, not by this draft.

## S§2. Summary of decisions

1. **Transfer ids live on the session, not the host.** `Session.next_transfer_id`
   starts at 1 in `Host::bind_peer` (`host.rs:641` neighbourhood) and is
   never reset, including across narrowing; unbind retires the session.
2. **`event_seq` is never reset.** The narrowing branch of `Host::set_grants`
   (`host.rs:707`) stops resetting `next_event_seq = 1`; it keeps the
   current counter and uses `Session::last_event_seq()` (`host.rs:395-397`)
   as the snapshot cursor.
3. **A narrowed peer receives a snapshot transfer, not a rebind.** The
   unacknowledged log is still discarded and delivery restarts as a
   filtered snapshot, but through a `Begin`/chunks on the same total order
   rather than `RebindRequired` + `SessionFenced`.
4. **`GrantUpdate` gains `NarrowedSnapshot(TransferId)`.** `RebindRequired`
   is retained for T025b's true revocation path; narrowing no longer fences.
5. **One in-flight transfer per peer.** `begin_transfer` is refused with a
   new `HostError::TransferInFlight` unless the previous id is retired via
   `Published`/`Resync`/`Cancel`.
6. **Bodies are built by the host from granted facts only**, reusing
   `PeerView::health`/`turn` and `session.map` (the already-filtered
   presence set). No new disclosure logic.
7. **Transfer frames are staged in the same ordered delivery log as lane-0
   frames**, via a new `DeliveryItem` enum, so `take_delivery`
   (`host.rs:1423-1431`) returns them in capture order.
8. **Canonical bytes come from `crpg_net::codec_transfer::plan_transfer`**
   (`codec_transfer.rs:488`); the host never hand-encodes.

## S§3. Files

Changed by the implementation (not by this draft):
- `crates/crpg-server/src/host.rs` — session fields, `bind_peer`, `set_grants`,
  new transfer methods, `take_delivery`, `acknowledge_delivery`,
  `commit_accepted`/`commit_rejection` accounting, `accepted_room`.
- `crates/crpg-server/src/quic.rs` — narrowing branch of
  `set_invitation_grants` (`quic.rs:1110-1132`) stops fencing and lets the
  transfer flow; `pump` phase 5 already hands off delivery bytes.
- `crates/crpg-server/AGENTS.md` — surface/trap amendments.
- `crates/crpg-server/tests/` — new suites listed in S§12.

No change to `crates/crpg-net/*`, no manifest/lock change.

## S§4. Public surface changes

`host` module additions (all need an ADR per `crates/crpg-server/AGENTS.md:15-25`):

```rust
pub enum GrantUpdate {
    Unchanged,
    RebindRequired,                 // retained: true revocation (T025b)
    NarrowedSnapshot(TransferId),    // new: seamless narrowing
}

pub enum HostError {
    // ... existing variants ...
    TransferInFlight,               // new
    UnknownTransfer,                // new
    StaleTransfer,                  // new
}

pub struct TransferId; // re-export crpg_net::protocol_transfer::TransferId

impl Host {
    pub fn transfer_published(&mut self, peer: PeerHandle, transfer: TransferId)
        -> Result<(), HostError>;
    pub fn transfer_resync(&mut self, peer: PeerHandle,
        after: Option<TransferId>, reason: ResyncReason)
        -> Result<(), HostError>;
    pub fn begin_transfer(&mut self, peer: PeerHandle,
        segments_per_chunk: u8) -> Result<TransferId, HostError>;
}
```

`TransferId` and `ResyncReason` are imported from `crpg_net::protocol_transfer`
(`protocol_transfer.rs:65`, `:205`); no new dependency. `crpg-net`'s
`TransferId::new`/`get` (`protocol_transfer.rs:71`, `:79`) are the only
allocator helpers; the host keeps its own monotonic counter and wraps via
`TransferId::new`.

Session additions (crate-private):

```rust
struct InFlightTransfer {
    id: TransferId,
    cursor: u64,            // last event_seq issued before capture
    base_tick: u64,         // host server_tick() at capture
    segments_per_chunk: u8,
    chunk_count: u8,
}
struct Session {
    // ... existing fields (host.rs:355-379) ...
    next_transfer_id: u64,           // new, starts 1
    in_flight: Option<InFlightTransfer>, // new
    retired_transfer_through: u64,    // new, max retired id
}
```

Delivery log becomes an ordered enum (replacing the bare `LoggedFrame` in
`Session.log`):

```rust
enum DeliveryItem {
    Lane0(LoggedFrame),          // existing LoggedFrame (host.rs:348-353)
    Transfer(Vec<u8>),           // pre-encoded transfer frame bytes
}
```

`Session.log: VecDeque<DeliveryItem>`, `log_bytes` accounts both. `push_frames`
(`host.rs:1676`) is split into `push_lane0` (keeps the `event_seq` assert
and `next_event_seq` increment at `host.rs:1682-1686`) and `push_transfer`
(no seq). `take_delivery` (`host.rs:1423`) flattens `DeliveryItem` to bytes.
`acknowledge_delivery` (`host.rs:1436`) pops `Lane0` items with
`event_seq <= through` and all contiguous `Transfer` items that precede a
popped boundary; transfer completion is separately reported by
`transfer_published`/`transfer_resync`, so a transfer frame is never
acknowledged by a lane-0 watermark alone.

## S§5. TransferId allocation

- Stored as `Session.next_transfer_id: u64`, initialised to 1 in
  `Host::bind_peer` (alongside `next_event_seq = 1`, `host.rs:641`) and
  never reinitialised by narrowing, widening, or resync.
- `Session::allocate_transfer_id` (new, crate-private):

```rust
fn allocate_transfer_id(&mut self) -> Result<TransferId, HostError> {
    if self.next_transfer_id == 0 || self.next_transfer_id == u64::MAX {
        return Err(HostError::CounterExhausted);
    }
    let id = TransferId::new(self.next_transfer_id)
        .ok_or(HostError::CounterExhausted)?;
    self.next_transfer_id = self.next_transfer_id.saturating_add(1);
    Ok(id)
}
```

- An id is retired (and `retired_transfer_through` raised) on `Published`,
  `Resync`, or `Cancel`; retired ids are never reused even if the transfer
  fails. `begin_transfer` refuses an id `<= retired_transfer_through` by
  allocating the next one, so monotonicity is structural.
- Epoch scoping: the counter lives on the session, and the session is dropped
  by `unbind_peer`/`fence` (`host.rs:662-670`); a new binding mints a fresh
  counter. This matches "per epoch, from 1" (`T024.md:890`).

## S§6. Filtered body builder (S§11 item 6)

New crate-private method:

```rust
fn capture_transfer_body(&self, session: &Session) -> SnapshotBody {
    // presence makes an entity (session.map is already the filtered,
    // never-reused NetId set from plan_map, host.rs:1796-1822)
    let mut entities: Vec<SnapshotEntity> = session
        .map
        .iter()
        .map(|(_entity, net)| SnapshotEntity {
            entity: *net,
            health: None,   // filled below when granted
        })
        .collect();
    // health grant adds health
    if session.grants.health {
        for (entity, net) in &session.map {
            if let Some(h) = self.health_fact(world, *entity) { // PeerView::health
                if let Some(slot) = entities.iter_mut().find(|e| e.entity == *net) {
                    slot.health = Some(h);
                }
            }
        }
    }
    // turn_state grant adds turn; active is None unless that entity is present
    let turn = if session.grants.turn_state {
        session.view().turn(world).map(|t| TurnFact {
            active: t.active.and_then(|a| session.map.get(&a).copied()),
            round: t.round,
        })
    } else {
        None
    };
    SnapshotBody { entities, turn }
}
```

Rules enforced by construction (T024 S§10.9, S§11 item 6):
- presence makes an entity: only `session.map` entries (already filtered by
  `plan_map`, `host.rs:1796-1822`) appear.
- health grant adds `health`: `health` is `Some` only when
  `session.grants.health` and the entity is a combatant.
- turn_state grant adds `turn`: `turn` is `Some` only when
  `session.grants.turn_state`.
- `active` is `None` unless that entity is present: the `and_then` on
  `session.map` (`host.rs:2024-2026` pattern) drops undisclosed actives.
- ungranted facts never reach bytes: the builder never reads a hidden
  entity's health; `PeerView::health` (`host.rs:1994-2003`) already gates on
  the grant.

`base_tick` is `self.server_tick()` (`host.rs:527`); `cursor` is
`session.last_event_seq()` (`host.rs:395-397`).

## S§7. Sender staging and the delivery-cursor cutover

New method `Host::stage_transfer(peer, segments_per_chunk) -> Result<TransferId, HostError>`:

1. Validate `segments_per_chunk` in `1..=MAX_SEGMENTS_PER_CHUNK` (8); else
   `HostError::InvalidGrants`-style `Malformed` mapping. (The net codec also
   validates; host validates before calling.)
2. If `session.in_flight.is_some()`, return `Err(HostError::TransferInFlight)`.
3. Allocate `id = session.allocate_transfer_id()`.
4. `cursor = session.last_event_seq()`; `base_tick = self.server_tick()`.
5. `body = self.capture_transfer_body(session)`.
6. `frames = crpg_net::codec_transfer::plan_transfer(session.epoch, id, base_tick, cursor, segments_per_chunk, &body)` (`codec_transfer.rs:488`). This yields
   `Begin` first, then one `Chunk` per chunk, in index order — canonical bytes.
7. Room check: `self.room(session, &transfer_frames_as_lane0_sized?)`. Transfer
   frames are large; use their byte lengths against the same caps
   (`room`, `host.rs:1370`). If no room, return `Err(HostError::QueueFull)`
   with nothing staged and the id retired (`retired_transfer_through = max(..., id)`).
8. Stage `Cancel(Aborted)` first if an in-flight transfer is being replaced
   (see S§8); then stage the `Begin` and chunk frames in order, followed by
   any deltas already waiting. Concretely: push the transfer frames into
   `session.log` via `push_transfer` in capture order, so they land ahead of
   deltas that later commits append.
9. Set `session.in_flight = Some(InFlightTransfer { id, cursor, base_tick, segments_per_chunk, chunk_count })`.
10. Return `Ok(id)`.

Cutover order (T024 S§10.7, S§11 items 2–3): because transfer frames and
lane-0 deltas share one `VecDeque<DeliveryItem>` and `take_delivery`
flattens in order, the on-wire order is:
`... deltas with event_seq <= cursor ... , Begin, Chunk* , ... deltas with event_seq > cursor ...`.
`Begin` therefore precedes every delta above its cursor, and chunks follow
`Begin`. Deltas at or below the cursor that were already queued stay ahead
of `Begin` and are applied first by the receiver, then replaced by the
snapshot; deltas above the cursor are never queued before `Begin` because
they are produced by commits that happen after `stage_transfer` returns (or
are held by the receiver's cutover gate). No delta is skipped: an overflow
during steps 6–8 triggers `Cancel` + fresh snapshot (S§8) or a fence, never
dropping a delta silently.

`segments_per_chunk` is chosen by the caller (host policy / adapter) in
`1..=8` (ADR-0028 item (b)); the host does not mandate a fixed value, so
real multi-chunk publication is exercised. Default recommendation: 4 (a
middle value that keeps frames small when sharing a stream with deltas).

## S§8. Cancel-before-supersede (S§11 item 4, item 5)

- `Host::begin_transfer` refuses a second in-flight transfer
  (`HostError::TransferInFlight`).
- On revocation that touches an in-flight transfer, the host, inside
  `Host::set_grants`'s narrowing branch (or a helper
  `cancel_in_flight(peer, CancelReason::Aborted)`):
  1. Stage `Cancel { epoch, transfer: id, reason: Aborted }` into
     `session.log` (so it reaches the peer before the new `Begin`).
  2. Set `session.in_flight = None` and raise `retired_transfer_through`.
  3. Stop staging chunks for the old id (any later chunk of that id is
     `StaleTransfer` at the receiver, T024 S§10.9).
  4. Capture a new body under the new grants and call `stage_transfer` for
     the new id (which may itself fail with `QueueFull`; then the reset stays
     pending and `try_resync` stages it later, matching `host.rs:709-710`).
- `CancelReason` and `ResyncReason` come from `crpg_net::protocol_transfer`
  (`protocol_transfer.rs:180`, `:205`).
- The host never sends a new `Begin` for a lower-or-equal id, and never
  reuses an id (`retired_transfer_through` guard).

## S§9. Seamless narrowing (what replaces the reset)

Today (`host.rs:699-712`): narrowing sets grants/map, subtracts `log_bytes`,
clears `log`, increments `generation`, resets `next_event_seq = 1`,
`delivered = 0`, sets `resync_pending = true`, calls `try_resync`, returns
`RebindRequired`.

New narrowing branch:

```rust
if is_narrowing(&session.grants, &grants) {
    let session = self.sessions.get_mut(&peer).expect("checked above");
    // cancel any in-flight transfer touched by the revocation
    if let Some(tf) = session.in_flight.take() {
        // stage Cancel(Aborted) for tf.id
        session.retired_transfer_through =
            session.retired_transfer_through.max(tf.id.get());
    }
    session.grants = grants;
    session.install_map(map, next_net);
    self.egress_bytes -= session.log_bytes;
    session.log.clear();          // unacknowledged lane-0 deltas dropped
    session.log_bytes = 0;
    session.generation += 1;      // epoch unchanged; generation bumps
    // NO next_event_seq reset; NO delivered reset to 0 as a seq reset.
    session.delivered = session.last_event_seq(); // watermark tracks cursor
    // capture + stage a fresh filtered snapshot under new grants
    let id = self.stage_transfer(peer, segments_per_chunk)?; // may QueueFull
    session.resync_pending = id.is_none(); // pending only if staging failed
    return Ok(GrantUpdate::NarrowedSnapshot(id));
}
```

(If `stage_transfer` returns `QueueFull`, the grants are already installed,
`resync_pending = true`, and `try_resync` stages the snapshot later — the
existing trap at `AGENTS.md:117-120`.)

How the client keeps one total order (T024 S§10.7, S§11 items 2–3):
- `event_seq` never resets within the epoch, so the snapshot's `cursor` is
  the last `event_seq` issued before capture and later deltas are above it.
- The peer receives `Begin(cursor)` followed by chunks, then deltas above
  `cursor`; the receiver holds deltas above `cursor` until publication and
  then releases them in order, so no `event_seq` regresses and no delta is
  applied to the wrong base.
- Widening and normal commits continue to increment `next_event_seq` from
  where it is; narrowing no longer creates a seq-1 epoch.

What it receives instead of a reset: a `NarrowedSnapshot(TransferId)` plus,
on the wire, `Cancel(Aborted)` (if an in-flight transfer was touched) then
`Begin`/chunks for the fresh body. The connection is not fenced, so bytes
already handed to the transport are not recalled; the snapshot replaces the
view in place (T025b will build resume on this).

## S§10. Hook into `Host::pump` and capture/retirement

- `Host::pump` (`host.rs:807`) is unchanged in structure: it still drains
  the staged ingress FIFO through `admit`. Transfer frames are produced
  inside `commit_accepted`/`commit_rejection`? No — they are produced by
  `set_grants`/`begin_transfer`/`transfer_resync`, which are out-of-pump
  operator calls (`AGENTS.md:181-184` already states such calls are not in
  the trace). `pump` does not need a new phase; the existing phase 3
  (`host.pump`) runs sim admission, and transfer staging happens when the
  operator or the commit path calls `begin_transfer`.
- `commit_accepted` (`host.rs:1181`) and `commit_rejection` (`host.rs:1143`)
  continue to append deltas at each peer's `next_event_seq`. They do not
  stage transfer frames. The only interaction with transfers is accounting:
  `accepted_room` (`host.rs:1341`) and `room` (`host.rs:1370`) must include
  `session.log_bytes` for in-flight transfer frames and a worst-case
  snapshot, so a commit that would overflow egress returns `QueueFull` at
  admission (head stays staged) rather than silently dropping a delta.
- When egress cannot hold a post-capture delta and a transfer is in flight,
  the host sends `Cancel(Aborted)` for that transfer, schedules a fresh
  snapshot (S§8), and retries the commit on the next pump; it never skips
  the delta.
- Capture/retirement (ADR-0021, ADR-0022): `CapturedRecord` (`capture.rs`)
  and `read_captures`/`acknowledge_captures` (`host.rs:1483`, `:1517`) are
  untouched. Transfer state is delivery/transport state, not captured
  evidence: it is not written to captures, not checkpointed, and resets on
  restart (ADR-0022: delivery logs and grace state reset to fresh). A
  restarted host re-establishes transfers through T025a resume, not through
  checkpoint.
- `save_checkpoint` (`host.rs:1564`) and `load_checkpoint` are unchanged;
  no `CHECKPOINT_VERSION` bump, because no checkpoint field changes.

QUIC adapter (`quic.rs`):
- `set_invitation_grants` narrowing branch (`quic.rs:1110-1112`) stops
  calling `narrowed()`/`fence(... Narrowed ...)` and instead stores grants
  and calls `host.set_grants`, mapping `NarrowedSnapshot(id)` to a new
  `GrantChange::NarrowedSnapshot(id)` (no fence). Widening path unchanged.
- `pump` phase 5 (`host.rs`-adjacent, `quic.rs` phase 5) already calls
  `take_delivery` and acknowledges; transfer frames flow through it without
  a new phase.

## S§11. Numbered server requirements (T024 S§11)

Each item restated as a server requirement, with where it is met and the
test that proves it.

1. **Transfer ids strictly increasing per epoch, from 1, never reused.**
   Met by: `Session::allocate_transfer_id` (S§5) called from `Host::begin_transfer`
   and the narrowing branch; `Session.next_transfer_id` init in
   `Host::bind_peer` (`host.rs:641`). Proven by:
   `transfer_id_allocator_monotonic_per_epoch` (S§12.1).

2. **`event_seq` never resets within an epoch; a snapshot's cursor is the
   last `event_seq` issued before capture; later deltas are above it.**
   Met by: removal of `session.next_event_seq = 1` from `Host::set_grants`
   (`host.rs:707`); cursor = `Session::last_event_seq()` (`host.rs:395-397`).
   Proven by: `event_seq_does_not_reset_on_narrowing` (S§12.2).

3. **In the peer's single total order, a `Begin` precedes every delta above
   its cursor, and a transfer's chunks follow its `Begin`.**
   Met by: `Host::stage_transfer` pushes `Begin` then chunks into the ordered
   `Session.log` ahead of later deltas (S§7). Proven by:
   `begin_precedes_deltas_above_cursor` (S§12.3).

4. **The host starts a new transfer only after `Published` or `Resync` for
   the previous one, or after sending `Cancel` for it.**
   Met by: `Host::begin_transfer` refuses with `HostError::TransferInFlight`
   when `session.in_flight.is_some()`; `transfer_published`/`transfer_resync`/
   `cancel_in_flight` retire the id and clear `in_flight`. Proven by:
   `second_transfer_refused_while_in_flight` (S§12.4).

5. **On revocation touching an in-flight transfer, the host sends
   `Cancel(Aborted)`, stops sending that id's chunks, and captures a new body
   under the new grants.**
   Met by: `Host::set_grants` narrowing branch (S§8, S§9) and
   `cancel_in_flight`. Proven by:
   `revocation_cancels_in_flight_and_recaptures` (S§12.5).

6. **A body holds only facts granted at capture: presence makes an entity,
   the health grant adds `health`, the `turn_state` grant adds `turn`, and
   `active` is `None` unless that entity is present.**
   Met by: `Host::capture_transfer_body` (S§6) over `session.map` and
   `PeerView::health`/`turn`. Proven by:
   `filtered_body_holds_granted_facts_only` and
   `body_active_none_unless_present` (S§12.6, S§12.7).

7. **Deltas after capture wait in egress under the existing caps; overflow
   means `Cancel` and a fresh snapshot, or a fence; never a skipped delta.**
   Met by: `accepted_room`/`room` accounting including transfer bytes
   (`host.rs:1341`, `:1370`) and the cancel-and-resnapshot path in
   `commit_accepted`/`commit_rejection` (S§10). Proven by:
   `delta_overflow_cancels_or_fences_no_skip` (S§12.8).

8. **`plan_transfer` produces canonical frames; a host that encodes frames
   itself must produce the same bytes.**
   Met by: `Host::stage_transfer` calls
   `crpg_net::codec_transfer::plan_transfer` (`codec_transfer.rs:488`) and
   stores its bytes; no second encoder. Proven by:
   `plan_transfer_canonical_bytes` (S§12.9).

## S§12. Test list

All under `crates/crpg-server/tests/` (new file `host_transfer.rs` plus
additions to `host_capture.rs`/`host_quic.rs`). Valid controls required for
every denial (root rules). Counts are targets; the implementation may split
files but must keep names.

1. `transfer_id_allocator_monotonic_per_epoch` — bind, begin_transfer twice
   returns 1 then 2; after cancel/resync, next is 3; unbind+rebind starts at 1.
2. `event_seq_does_not_reset_on_narrowing` — accepted commands bring
   `next_event_seq` to N; narrowing keeps N; `last_event_seq()` is cursor.
3. `begin_precedes_deltas_above_cursor` — stage transfer at cursor C, then
   accept a command; delivery bytes order is `Begin` before the new delta.
4. `second_transfer_refused_while_in_flight` — `begin_transfer` twice returns
   `TransferInFlight` with state unchanged.
5. `revocation_cancels_in_flight_and_recaptures` — narrow while in flight;
   `Cancel(Aborted)` is staged, old chunks become `StaleTransfer` at the
   receiver, new transfer publishes under new grants.
6. `filtered_body_holds_granted_facts_only` — grant matrix (presence/health/
   turn_state) produces bodies with exactly the granted facts; ungranted
   entity absent.
7. `body_active_none_unless_present` — undisclosed active reads as `None`.
8. `delta_overflow_cancels_or_fences_no_skip` — fill egress, then accept;
   either a `Cancel`+fresh snapshot is staged or `QueueFull` with the delta
   retained; no delta's bytes disappear.
9. `plan_transfer_canonical_bytes` — host-staged bytes equal
   `plan_transfer` output byte-for-byte.
10. `seamless_narrowing_keeps_total_order` — end-to-end: narrowing no longer
    resets `event_seq`; deltas above cursor arrive after `Begin`; receiver
    publishes and releases them; no `SessionFenced`.
11. `narrowed_client_never_sees_ungranted_fact_or_seq_regress` — independent
    filtered-replica oracle: apply popped updates to a `BTreeMap` replica,
    compare per fact against hand-authored expected facts; assert no ungranted
    fact and no `event_seq` regression across narrowing. Negative control:
    applying a delta above the cursor before the snapshot, or an ungranted
    health byte, makes the oracle fail.
12. `published_clears_in_flight_and_retires_id` — `transfer_published(id)`
    retires id, clears `in_flight`, leaves snapshot queued.
13. `resync_retries_with_fresh_snapshot` — `transfer_resync(Some(id), Expired)`
    retires id and allows a new `begin_transfer`.
14. `cancel_retires_id_and_supersedes` — `Cancel` of in-flight retires id;
    `Begin(id)` later is `StaleTransfer`; `Begin(id+1)` publishes.
15. `host_pump_stages_transfer_frames_in_order` — `QuicPumpReport.calls`
    records the host calls; replay reproduces transitions (existing trap).
16. `capture_retirement_unaffected_by_transfer` — captures/acknowledgements
    unchanged by transfer staging; no `CHECKPOINT_VERSION` bump.

Oracle rules (T024 S§12 style): the oracle is test-local and independent, applies
popped updates to a `BTreeMap` replica, compares per fact, never uses a
production projection, and has negative controls that fail loudly.

## S§13. Boundary list (what stays out)

- **T024q (`crpg-net-quic`)**: carrying transfer frames on the existing
  ordered stream with an explicit channel selector under ALPN
  `crpg-lane0/2`; dispatch by first byte/channel, not payload inspection.
  The host remains byte producer/consumer via `take_delivery`; no socket code
  here.
- **T025a (resume)**: proof-bearing resume across connections, fencing
  generations, installing a snapshot across connections. T024h keeps transfer
  state in-process and resets on restart; cross-connection install is T025a.
- **T025b (grace)**: 30 s same-process grace, frozen input with continuing
  authority, retry returns original receipt, full filtered replacement after
  revocation, no combat reset. T024h supplies the seamless snapshot primitive
  but does not implement grace, fencing epochs, or restart policy.
- **T027b (interest)**: producing bodies from production interest facts and
  private-field grants; new facts need transfer version `0x82`.
- **T046 (replica storage)**: applying `ReplicaUpdate`s and read queries;
  the oracle here is test-local.

## S§14. S-set, golden and replay impact

- **S-set (`docs/IMPLEMENTATION_PLAN.md` §1.4): not touched.** No change to
  `World` or tick order, no schema version/migration change in `crpg-data`,
  no `crpg-contracts` signature change, no GDExtension boundary change. The
  narrowing control flow changes, but `World`/tick order do not.
- **Golden/replay: no file change and no re-baseline.** No new golden; no
  cross-target hash comparison. Existing narrowing/QUIC tests that assert
  fencing on narrowing (`fencing_closes_with_session_fenced`,
  `host_capture.rs:1476`) are re-scoped by the implementation: true
  revocation still fences, narrowing no longer does. That is a behaviour
  change in tests, not a golden change, and requires H-review of the test
  edits.
- **Checkpoint: no `CHECKPOINT_VERSION` bump.** Transfer state is delivery/
  transport state and resets on restart (ADR-0022); it is not captured or
  persisted.
- **Dependencies: no new edge.** Uses existing `crpg-net` transfer modules;
  `crpg-server`'s allowed set (`tools/lint/deps.py:69`) unchanged.
- **Invariants:** no `HashMap`/`HashSet` introduced (use `BTreeMap`); no
  floats; `#![forbid(unsafe_code)]` preserved; host remains sole mutator;
  determine/reserve/commit and capture-before-acknowledgement preserved.

## S§15. Open questions for the user

1. **Should narrowing keep `GrantUpdate::RebindRequired` as a deprecated
   alias, or replace it outright with `NarrowedSnapshot(TransferId)`?**
   Recommendation: replace the narrowing path's return with
   `NarrowedSnapshot(TransferId)` and keep `RebindRequired` only for true
   revocation (T025b), so adapters can distinguish "snapshot coming" from
   "rebind needed". Avoids a silent behaviour change in adapters.

2. **Should `delivered` be reset to the cursor on narrowing, or left at its
   previous value?**
   Recommendation: set `delivered = cursor` (S§9), so the delivery watermark
   matches the snapshot base and `take_delivery`/`acknowledge_delivery`
   (`host.rs:1423`, `:1436`) stay consistent; leaving it stale would re-send
   frames below the cursor.

3. **Who chooses `segments_per_chunk` (1..=8), and what is the default?**
   Recommendation: host policy chooses per transfer; default 4; the adapter
   may pass a smaller value under egress pressure. Fixed 8 would make every
   valid body one chunk and never exercise reorder/missing with a real body.

4. **On egress overflow during a post-capture delta, cancel+resnapshot or
   fence?**
   Recommendation: cancel the in-flight transfer and schedule a fresh
   snapshot when a transfer is in flight (S§11 item 7, D11); fence only
   when no transfer is in flight or the fresh snapshot cannot be staged.
   Failing closed at the receiver (T024 S§16 Q4) already requires a fresh
   snapshot, so cancel+resnapshot matches.

5. **Should transfer frames be acknowledged by lane-0 watermark or by
   `Published`/`Resync` only?**
   Recommendation: only by `Published`/`Resync` (S§4). A lane-0 watermark
   must not retire a transfer frame, otherwise a re-sent chunk could be
   dropped before publication.

6. **Does seamless narrowing require a generation bump even though `event_seq`
   no longer resets?**
   Recommendation: yes — bump `generation` (S§9) so cached receipts and
   capture records keyed by `(generation, event_seq)` (`host.rs:1260`)
   cannot be confused with pre-narrowing frames; `event_seq` continuity is
   for the peer's total order, generation is for host identity of receipts.

## Agent log

- 2026-10-25 (UTC) · opencode-go/beam-free + T024h draft · Drafted the server sender/host and seamless-narrowing contract (TransferId allocation, filtered body builder, cutover order, cancel-before-supersede, narrowed snapshot replacing the event_seq reset, pump/capture hooks, eight numbered requirements, test list with independent oracle, and boundary list), citing host.rs/quic.rs/net signatures.
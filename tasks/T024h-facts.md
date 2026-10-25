# T024h facts — `crpg-server` sender/host requirements (T024 S§11)

Fact sheet only. Every `path:line` below exists on this branch. No
recommendations, proposed types, or opinions.

## 1. T024 S§11 sender rules and host requirements

Section heading `tasks/T024.md:885` ("Sender rules and host requirements");
intro `tasks/T024.md:887-888` ("requirements on the later host wiring (S§16 Q5);
T024 can check only what its pool sees"). N = 8.

1. `tasks/T024.md:890-891` Transfer ids are strictly increasing per epoch,
   from 1, and never reused; they need not be contiguous.
2. `tasks/T024.md:892-895` `event_seq` never resets within an epoch; a
   snapshot's `cursor` is the last `event_seq` issued before capture, and
   later deltas are above it; T022's narrowing reset to 1 must change when
   narrowing becomes a seamless snapshot.
3. `tasks/T024.md:896-897` In the peer's single total order, a `Begin`
   precedes every delta above its cursor, and a transfer's chunks follow its
   `Begin`.
4. `tasks/T024.md:898-899` The host starts a new transfer only after
   `Published` or `Resync` for the previous one, or after sending `Cancel`
   for it.
5. `tasks/T024.md:900-902` On revocation that touches an in-flight transfer,
   the host sends `Cancel(Aborted)`, stops sending that id's chunks and
   captures a new body under the new grants (D11, D14).
6. `tasks/T024.md:903-905` A body holds only facts granted at capture:
   presence makes an entity, the health grant adds `health`, the
   `turn_state` grant adds `turn`, and `active` is `None` unless that entity
   is present.
7. `tasks/T024.md:906-907` Deltas after capture wait in egress under the
   existing caps; overflow means `Cancel` and a fresh snapshot, or a fence;
   never a skipped delta.
8. `tasks/T024.md:908-909` `plan_transfer` produces canonical frames; a host
   that encodes frames itself must produce the same bytes.

"Seamless narrowing" mentions (all occurrences found):

- `tasks/T024.md:894-895` (item 2): "T022's narrowing reset to 1 must
  change when narrowing becomes a seamless snapshot".
- `tasks/T024.md:1145` (S§15 out of scope): "`crpg-server` wiring: … making
  narrowing seamless in place of `SessionFenced` (T023b's note)".
- `tasks/T024.md:1208` (S§16 Q5): "T024h (`crpg-server`): S§11 and seamless
  narrowing."
- `tasks/T024.md:1217` (approval paragraph): "T024h (`crpg-server`) — T024
  S§11 sender/host requirements and seamless narrowing."
- `tasks/T023.md:1847-1848` (R4): "close now, seamless later … make
  narrowing seamless through snapshot resync rather than an in-band reset."
- `tasks/T023b.md:807`: "Making narrowing seamless through snapshot resync is
  T024/T025a (R4)."
- `tasks/T023b.md:1367`: "Snapshot transfer and seamless narrowing (T024,
  D11); narrowing closes the connection here."
- `tasks/T023b.md:2381`: "A snapshot resync can make it seamless; until then,
  `set_invitation_grants` must keep fencing."
- `docs/architecture/crpg-server.md:209`: "T024 adds snapshot resync, which
  can make narrowing seamless."
- `docs/architecture/crpg-net-quic.md:42`: "T025a | Resume/reconnect hello;
  seamless narrowing instead of a `SessionFenced` close".
- `docs/architecture/crpg-server.md:30`: "snapshot transfer and seamless
  narrowing (T024)".
- `tasks/BACKLOG.md:115`: "T024h … `crpg-server` T024 S§11 sender/host
  requirements and seamless narrowing".

Meaning in those sources: today narrowing (`Host::set_grants` on a removed
permission) discards the unacknowledged delivery log, bumps `generation`,
resets `next_event_seq` to 1, sets `resync_pending`, returns
`GrantUpdate::RebindRequired`; the QUIC adapter then fences the connection
with `SessionFenced` (Narrowed) so bytes already handed to QUIC cannot be
recalled (`crates/crpg-server/src/quic.rs:1110-1132`,
`crates/crpg-server/AGENTS.md:156-159`). "Seamless narrowing" means using a
snapshot transfer instead: capture a fresh filtered body under the new grants,
keep `event_seq` across the snapshot (no reset to 1), send `Cancel(Aborted)`
if an in-flight transfer is affected, and install the snapshot on the client
without closing/fencing, with later deltas above the cursor.

## 2. Where `crpg-server` sends lane-0 deltas and snapshots today

Delivery state lives in `Session` (`crates/crpg-server/src/host.rs:355-379`):
`log: VecDeque<LoggedFrame>` (373), `log_bytes` (374), `next_event_seq: u64`
(375), `delivered: u64` (376), `generation: u64` (377),
`resync_pending: bool` (378). `LoggedFrame` is `{ event_seq: u64,
bytes: Vec<u8> }` (`crates/crpg-server/src/host.rs:348-353`). `last_event_seq()` returns
`next_event_seq - 1` (`crates/crpg-server/src/host.rs:395-397`).

Functions and call order (host authority only; no snapshot transfer exists in
the crate today — see section 8):

- `Host::bind_peer(control, grants, now_ms) -> Result<PeerHandle, HostError>`
  (`crates/crpg-server/src/host.rs:588-656`): mints epoch as incarnation LE bytes + handle LE bytes
  (`crates/crpg-server/src/host.rs:607-610`); creates a session with `next_event_seq = 1`
  (`crates/crpg-server/src/host.rs:641`), `generation = 0` (`crates/crpg-server/src/host.rs:643`), `resync_pending = false`
  (`crates/crpg-server/src/host.rs:644`); calls `state_frames` (`crates/crpg-server/src/host.rs:647`) then `room`
  (`crates/crpg-server/src/host.rs:648`) and `push_frames` (`crates/crpg-server/src/host.rs:651`); returns
  `HostError::QueueFull` if the fresh state has no room.
- `Host::state_frames(session) -> Result<Vec<LoggedFrame>, HostError>`
  (`crates/crpg-server/src/host.rs:1397-1400`): `full_state_ops` (`crates/crpg-server/src/host.rs:1398`) then
  `chunk_frames` (`crates/crpg-server/src/host.rs:1399`).
- `PeerView::full_state_ops(world) -> Vec<protocol_v2::DeltaOp>`
  (`crates/crpg-server/src/host.rs:2032-2054`): `EntityEnter` per mapped entity, then `Health`
  where granted, then `Turn` if `turn_state` granted.
- `Host::chunk_frames(session, ops) -> Result<Vec<LoggedFrame>, HostError>`
  (`crates/crpg-server/src/host.rs:1379-1394`): chunks `ops` into frames of at most
  `MAX_DELTA_OPS` (256) ops, each frame at an incremented `event_seq`
  starting at `session.next_event_seq` (`crates/crpg-server/src/host.rs:1385`, `crates/crpg-server/src/host.rs:1389-1391`).
- `push_frames(host_total, session, frames) -> Result<(), HostError>`
  (`crates/crpg-server/src/host.rs:1676-1692`): asserts `frame.event_seq == session.next_event_seq`
  (`crates/crpg-server/src/host.rs:1682`), then increments `next_event_seq` (`crates/crpg-server/src/host.rs:1683-1686`),
  adds bytes to `log_bytes`/host total (`crates/crpg-server/src/host.rs:1687-1688`) and pushes to
  `session.log` (`crates/crpg-server/src/host.rs:1689`).
- `Host::set_grants(peer, grants) -> Result<GrantUpdate, HostError>`
  (`crates/crpg-server/src/host.rs:683-732`):
  - narrowing branch (`crates/crpg-server/src/host.rs:699-712`): sets grants, installs map, subtracts
    `log_bytes` from `egress_bytes`, clears `log` and `log_bytes`, increments
    `generation` (`crates/crpg-server/src/host.rs:706`), resets `next_event_seq = 1`
    (`crates/crpg-server/src/host.rs:707`), `delivered = 0` (`crates/crpg-server/src/host.rs:708`), sets
    `resync_pending = true` (`crates/crpg-server/src/host.rs:709`), calls `try_resync`
    (`crates/crpg-server/src/host.rs:710`), returns `GrantUpdate::RebindRequired` (`crates/crpg-server/src/host.rs:711`).
  - widening branch (`crates/crpg-server/src/host.rs:713-731`): builds `widening_ops` unless
    `resync_pending` (`crates/crpg-server/src/host.rs:713-722`), `chunk_frames` (`crates/crpg-server/src/host.rs:723`),
    `room` check (`crates/crpg-server/src/host.rs:724`), installs grants/map, `push_frames`
    (`crates/crpg-server/src/host.rs:730`), returns `GrantUpdate::Unchanged` (`crates/crpg-server/src/host.rs:731`).
- `Host::try_resync(peer) -> Result<(), HostError>` (`crates/crpg-server/src/host.rs:1403-1416`):
  if `resync_pending`, builds `state_frames`, checks `room`, `push_frames`,
  clears `resync_pending`; returns `HostError::QueueFull` if no room.
- `Host::commit_accepted(...)` (`crates/crpg-server/src/host.rs:1181-1326`): for every session not
  `resync_pending` (`crates/crpg-server/src/host.rs:1215`), builds receipt op, projected events
  (`crates/crpg-server/src/host.rs:1228-1235`), health ops (`crates/crpg-server/src/host.rs:1237-1241`) and turn op
  (`crates/crpg-server/src/host.rs:1242-1244`), encodes a frame at `session.next_event_seq`
  (`crates/crpg-server/src/host.rs:1253`), records `receipt_ref = Some((generation, next_event_seq))`
  (`crates/crpg-server/src/host.rs:1260`), then after capture appends deliveries via `push_frames`
  (`crates/crpg-server/src/host.rs:1305-1308`), advances `session.next_new_seq = frame.seq + 1`
  (`crates/crpg-server/src/host.rs:1310`), inserts the cache entry (`crates/crpg-server/src/host.rs:1311-1321`), and
  acknowledges the sim journal (`crates/crpg-server/src/host.rs:1322-1324`).
- `Host::commit_rejection(...)` (`crates/crpg-server/src/host.rs:1143-1177`): sets
  `next_new_seq = frame.seq + 1` (`crates/crpg-server/src/host.rs:1155`); if not `resync_pending`
  and the failure budget allows, stages the receipt via `push_frames`
  (`crates/crpg-server/src/host.rs:1170`); inserts cache entry (`crates/crpg-server/src/host.rs:1175`).
- `Host::cache_hit(...)` (`crates/crpg-server/src/host.rs:1061-1117`): may stage a receipt frame at
  `session.next_event_seq` (`crates/crpg-server/src/host.rs:1077-1081`) when room and budget allow.
- `Host::receipt_frame(session, seq, processed_tick, status) -> Result<LoggedFrame, HostError>`
  (`crates/crpg-server/src/host.rs:1120-1138`): builds a frame at `session.next_event_seq`.
- `Host::take_delivery(peer) -> Result<Vec<Vec<u8>>, HostError>`
  (`crates/crpg-server/src/host.rs:1423-1431`): calls `try_resync` (`crates/crpg-server/src/host.rs:1424`), then clones
  `session.log` bytes (`crates/crpg-server/src/host.rs:1426-1430`).
- `Host::acknowledge_delivery(peer, through) -> Result<(), HostError>`
  (`crates/crpg-server/src/host.rs:1436-1463`): pops `session.log` frames with
  `event_seq <= through` (`crates/crpg-server/src/host.rs:1452-1460`) and sets `delivered = through`
  (`crates/crpg-server/src/host.rs:1461`); idempotent at/below watermark (`crates/crpg-server/src/host.rs:1449-1451`).
- `Host::unbind_peer(peer)` (`crates/crpg-server/src/host.rs:662-664`) calls `fence`
  (`crates/crpg-server/src/host.rs:666-670`), removing the session and subtracting its `log_bytes`
  from `egress_bytes`.

Epoch and fencing (ADR-0021, ADR-0022):

- `Host::epoch(peer) -> Result<[u8; 16], HostError>` (`crates/crpg-server/src/host.rs:519-524`)
  returns the binding's epoch.
- Epoch bytes are incarnation (LE, first 8) + handle (LE, last 8)
  (`crates/crpg-server/src/host.rs:607-610`); handle starts at 1 and never reuses (`next_handle`,
  `crates/crpg-server/src/host.rs:426`, `crates/crpg-server/src/host.rs:603-606`).
- `Host::unbind_peer` / `fence` remove the binding; staged commands for it
  are later refused as `SessionExpired` (`crates/crpg-server/src/host.rs:904-917`).
- QUIC adapter fencing (`crates/crpg-server/src/quic.rs`):
  - `fence(conn, cause, report)` (`crates/crpg-server/src/quic.rs:952-991`) records a `Fence`.
  - Epoch closure: if `host.epoch(peer) != Ok(epoch)` fence with
    `FenceCause::EpochClosed` (`crates/crpg-server/src/quic.rs:513-518`).
  - Supersession: newer connection fences the old with
    `FenceCause::Superseded` (`crates/crpg-server/src/quic.rs:587`).
  - Narrowing: `set_invitation_grants` when `is_narrowing` stores grants and
    calls `narrowed` -> `fence(conn, FenceCause::Narrowed, None)`
    (`crates/crpg-server/src/quic.rs:1110-1132`); comment "The connection is closed now … Making
    narrowing seamless through snapshot resync is T024/T025a (R4)."
  - Revocation: `revoke_invitation` fences live binding with
    `FenceCause::Revoked` (`crates/crpg-server/src/quic.rs:1074-1085`).
- `crates/crpg-server/AGENTS.md:156-159`: "Narrowing fences; it never calls
  `set_grants`. A narrowing `set_invitation_grants` closes the connection
  with `SessionFenced` … Seamless narrowing is T024/T025a."

No snapshot transfer call exists in `crpg-server` today (section 8).

## 3. Interest/visibility filtering in the server today

Filtering is host-side, per peer, deny by default. Grant shapes:
`EntityDisclosure` (10 flags) and `DisclosureGrants` (`crates/crpg-server/src/host.rs:122-171`);
`is_narrowing(old, new)` compares flags/abilities/encounters/global flags
(`crates/crpg-server/src/host.rs:1773-1791`); `normalize_grants` validates bounds/duplicates
(`crates/crpg-server/src/host.rs:1726-1747`); `plan_map` maps only present, live entities and mints
never-reused `NetId`s (`crates/crpg-server/src/host.rs:1796-1822`).

Where filtering happens during an accepted command
(`commit_accepted`, `crates/crpg-server/src/host.rs:1181-1326`):

- Events: `view.candidate(&envelope.payload)` for each captured envelope
  (`crates/crpg-server/src/host.rs:1231`), then `project_events` (`crates/crpg-server/src/host.rs:1233`) from
  `crates/crpg-net/src/projection_v2.rs`; candidates carry only granted
  fields.
- Health: `view.health_entries(world, &involved)` (`crates/crpg-server/src/host.rs:1237`), which
  calls `health()` and filters by `present` + `health` grant.
- Turn: `view.turn(world)` (`crates/crpg-server/src/host.rs:1238`), gated on `grants.turn_state`;
  undisclosed active reads as `None` (`crates/crpg-server/src/host.rs:2016-2028`).
- Ops encoded only if `ops` non-empty (`crates/crpg-server/src/host.rs:1245`); frames per peer at
  that peer's `next_event_seq` (`crates/crpg-server/src/host.rs:1253`).
- `PeerView` methods (`crates/crpg-server/src/host.rs:1911-2059`): `net_if`, `grant`, `health`,
  `health_entries`, `turn`, `candidate`, `full_state_ops`.

Fresh-state / widening paths:

- `full_state_ops` (`crates/crpg-server/src/host.rs:2032-2054`): complete filtered state
  (EntityEnter + Health + Turn) used by `state_frames`.
- `widening_ops` (`crates/crpg-server/src/host.rs:2060-2090`): ops added when grants widen (new
  entities, newly visible health, turn_state).
- `set_grants` narrowing path clears log and resets delivery
  (`crates/crpg-server/src/host.rs:699-712`); widening path stages new ops (`crates/crpg-server/src/host.rs:713-731`).
- QUIC: `set_invitation_grants` stores grants and, if narrowing, fences
  (`crates/crpg-server/src/quic.rs:1090-1125`); widening calls `host.set_grants` then stores
  (`crates/crpg-server/src/quic.rs:1114-1124`).

Existing tests of filtering/narrowing:

- `crates/crpg-server/tests/host_capture.rs:1027`
  `hidden_fields_absent_from_snapshots_and_replies`: per-field suppression of
  events/state; missing presence/ability/outcome/damage/actor/target omits the
  whole event, state ops reflect grants.
- `crates/crpg-server/tests/host_capture.rs:1209`
  `two_peers_share_lane_sequences_safely`: per-peer event_seq safety.
- `crates/crpg-server/tests/host_capture.rs:1476`
  `narrowing_grants_resynchronize_without_leak`: `set_grants` narrowing returns
  `RebindRequired`, delivery restarts at seq 1 with fresh filtered state,
  hidden entity's old NetId (2) never appears in later frames, widening mints
  a fresh id (4), `set_control` does not resync.
- `crates/crpg-server/tests/host_quic.rs:478`
  `quic_filtered_replica_converges_per_peer`: per-peer replicas from QUIC bytes;
  B holds A's presence but never A's health; events/receipts filtered per peer.
- `crates/crpg-server/tests/host_quic.rs:600`
  `quic_desync_oracle_fails_on_drop_and_corruption`: oracle detects dropped/
  corrupt frames.
- `crates/crpg-server/tests/host_quic.rs:1739`
  `fencing_closes_with_session_fenced`: narrowing closes connection.

## 4. The `crpg-net` transfer API the server would call

Constants (`crates/crpg-net/src/protocol_transfer.rs:19-54`):
`TRANSFER_VERSION = 0x81` (19), `LANE_SNAPSHOT = 2` (22),
`TRANSFER_TAG_BEGIN = 0x10` (25), `TRANSFER_TAG_CHUNK = 0x11` (27),
`TRANSFER_TAG_CANCEL = 0x12` (29), `TRANSFER_TAG_PUBLISHED = 0x20` (31),
`TRANSFER_TAG_RESYNC = 0x21` (33), `MAX_SEGMENT_BYTES = 4096` (40),
`MAX_SEGMENTS_PER_CHUNK = 8` (42), `MAX_CHUNK_BYTES = 32768` (44),
`MAX_DOWN_FRAME_BYTES = 65536` (46), `MAX_UP_FRAME_BYTES = 4096` (48),
`MAX_ENCODED_BODY_BYTES = 22552` (50),
`POLICY_SNAPSHOT_STAGING_BYTES_PER_PEER = 1048576` (52),
`POLICY_SNAPSHOT_STAGING_BYTES_HOST = 8388608` (54).

Identity (`crates/crpg-net/src/protocol_transfer.rs:65-82`): `TransferId(u64)` with
`new(raw: u64) -> Option<Self>` (71) and `get(self) -> u64` (79); derives
`Copy, PartialOrd, Ord, Hash`.

Messages (`crates/crpg-net/src/protocol_transfer.rs:88-270`):
`BeginTransfer` fields epoch/base_tick/cursor/total_bytes/entity_count/
segments_per_chunk/chunk_count with `validate(&self) -> Result<(), CodecError>`
(111) and `chunk_len(&self, index: u8) -> Option<usize>` (148).
`TransferChunk { epoch, transfer, index, bytes: Vec<u8> }` (169-178).
`CancelReason` (Superseded=1, Aborted=2) with `as_u8`/`from_u8` and `Copy`
(180-203). `ResyncReason` (Expired=1, Invalid=2, Overflow=3) (205-231).
`TransferDown` (Begin/Chunk/Cancel) (233-249). `TransferUp` (Published/Resync)
(251-270).

Body (`crates/crpg-net/src/protocol_transfer.rs:276-312`): `SnapshotBody { entities, turn }`,
`SnapshotEntity { entity: NetId, health: Option<HealthFact> }`,
`HealthFact { health, max_health, dead }`, `TurnFact { active, round }`.

Codec (`crates/crpg-net/src/codec_transfer.rs`):
- `encode_down(msg: &TransferDown) -> Result<Vec<u8>, CodecError>` (160)
- `decode_down(bytes: &[u8]) -> Result<TransferDown, CodecError>` (246)
- `encode_up(msg: &TransferUp) -> Result<Vec<u8>, CodecError>` (393)
- `decode_up(bytes: &[u8]) -> Result<TransferUp, CodecError>` (432)
- `encode_body(body: &SnapshotBody) -> Result<Vec<u8>, CodecError>` (32)
- `decode_body(bytes: &[u8]) -> Result<SnapshotBody, CodecError>` (97)
- `plan_transfer(epoch, transfer, base_tick, cursor, segments_per_chunk, body)
  -> Result<Vec<Vec<u8>>, CodecError>` (488-497)

Reassembler (`crates/crpg-net/src/reassembly.rs`):
Byte layer:
- `TransferAssembly::new(begin: &BeginTransfer) -> Result<Self, CodecError>` (60)
- `begin(&self) -> &BeginTransfer` (70)
- `accept(&mut self, chunk: &TransferChunk) -> Result<ChunkOutcome, AssemblyError>` (73)
- `received_chunks(&self) -> u8` (109)
- `is_complete(&self) -> bool` (112)
- `into_bytes(self) -> Result<Vec<u8>, Self>` (115)
- `ChunkOutcome { Stored, Duplicate, Complete }` (58-62)
- `AssemblyError { WrongTransfer, OutOfRange, WrongLength, Conflict }` (17-24)

Limits:
- `TransferLimits` fields max_receivers/deadline_ms/staging_bytes_per_receiver/
  staging_bytes_total/delta_frames_per_receiver/delta_bytes_per_receiver/
  delta_bytes_total (127-135)
- `TransferLimits::v1() -> Self` (139)
- `validate(&self) -> Result<(), LimitsError>` (150)
- `LimitsError { InvalidLimits { field }, LoosensPolicy { field } }` (225-229)

Receiver pool:
- `ReassemblyPool::new(limits: TransferLimits) -> Result<Self, LimitsError>` (477)
- `limits(&self) -> TransferLimits` (488)
- `add_receiver(&mut self, epoch, wire: DeltaWire, start: GateMode)
  -> Result<ReceiverId, TransferError>` (491-493)
- `remove_receiver(&mut self, id: ReceiverId)` (524)
- `offer_transfer(&mut self, id: ReceiverId, bytes: &[u8], now_ms: u64)
  -> Result<TransferOutcome, TransferError>` (581-583)
- `offer_delta(&mut self, id: ReceiverId, bytes: &[u8], now_ms: u64)
  -> Result<DeltaOutcome, TransferError>` (884-886)
- `poll(&mut self, now_ms: u64) -> Vec<ReceiverId>` (1000)
- `pop_update(&mut self, id: ReceiverId) -> Option<ReplicaUpdate>` (1007)
- `status(&self, id: ReceiverId) -> Result<ReceiverStatus, TransferError>` (1022)
- `staging_bytes(&self) -> usize` (1055)
- `held_delta_bytes(&self) -> usize` (1058)

Supporting types in reassembly.rs:
`DeltaWire { V1, V2 }` (281-282), `GateMode { Live, AwaitSnapshot }` (283-284),
`GatedDelta { V1(DeltaFrame), V2(DeltaFrame) }` with `event_seq(&self) -> u64`
(285-289), `Publication { transfer, base_tick, cursor, body }` (589-594),
`ReplicaUpdate { Snapshot(Publication), Delta(GatedDelta) }` (595),
`ReceiverId(u32)` with `get` (597-598), `TransferOutcome` (600-606),
`DeltaOutcome` (607), `FailReason` (608-610), `TransferError` (612-625),
`InFlightStatus` (627-635), `PublishedBase` (636), `ReceiverStatus` (637-649).

## 5. Host session capture/retirement and the T022 host loop

Host methods a sender would call (all in `crates/crpg-server/src/host.rs`):

- `Host::new(seed, config, now_ms) -> Result<Self, HostError>` (446)
- `Host::from_history(history, config, now_ms) -> Result<Self, HostError>` (453)
- `Host::restore(config, history, captures, capture_acknowledged, now_ms) -> Self`
  (489, crate-private)
- `Host::epoch(peer) -> Result<[u8; 16], HostError>` (519)
- `Host::server_tick(&self) -> u64` (527)
- `Host::ingest(peer, bytes, now_ms) -> Result<IngestDisposition, HostError>` (757)
- `Host::pump(now_ms) -> Result<PumpSummary, HostError>` (807)
- `Host::tick(&mut self) -> Result<(), HostError>` (1553)
- `Host::take_delivery(peer) -> Result<Vec<Vec<u8>>, HostError>` (1423)
- `Host::acknowledge_delivery(peer, through) -> Result<(), HostError>` (1436)
- `Host::set_grants(peer, grants) -> Result<GrantUpdate, HostError>` (683)
- `Host::set_control(peer, control) -> Result<(), HostError>` (736)
- `Host::bind_peer(control, grants, now_ms) -> Result<PeerHandle, HostError>` (588)
- `Host::unbind_peer(peer)` (662)
- `Host::save_checkpoint(&self) -> Result<Vec<u8>, CheckpointError>` (1564)
- `Host::read_captures(after, limit) -> Result<Vec<CapturedRecord>, HostError>` (1483)
- `Host::acknowledge_captures(through) -> Result<(), HostError>` (1517)
- `Host::capture_acknowledged(&self) -> u64` (1541)
- `Host::last_capture(&self) -> u64` (1546)
- `Host::shutdown(&mut self) -> Vec<AdmissionResult>` (1572)

Checkpoint load (`crates/crpg-server/src/checkpoint.rs`):
- `load_checkpoint(config, bytes, now_ms) -> Result<Host, CheckpointError>` (128)
- `load_checkpoint_from_reader<R: Read>(config, reader, now_ms, max_bytes)
  -> Result<Host, CheckpointError>` (182)

Save adapter (`crates/crpg-server/src/save.rs`):
- `encode_host_save(identity, checkpoint) -> Result<Vec<u8>, SaveError>` (165)
- `decode_host_save(bytes) -> Result<(SaveIdentity, Vec<u8>), SaveError>` (196)
- `save_host_file(path, host, identity) -> Result<(), SaveError>` (272)
- `load_host_file(path, config, identity, now_ms) -> Result<LoadedSave, SaveError>`
  (287)

QUIC adapter hook-in points (`crates/crpg-server/src/quic.rs`):
- `QuicHost::set_invitation_grants(id, grants) -> Result<GrantChange, QuicHostError>`
  (1090)
- `QuicHost::pump(now_ms)` phase list in `docs/architecture/crpg-server.md:113-123`
- `QuicHost::host(&self) -> &Host` (1211)
- `QuicHost::shutdown(self, checkpoint) -> QuicHostShutdown` (1251)

Capture records (`crates/crpg-server/src/capture.rs`): `CapturedRecord`
(118), `CapturedView` (105), `CapturedHealth` (79), `CapturedTurn` (94),
`CapturedOutcome` (62). One record per accepted command carries
`(history_start, history_end]`, cloned envelopes and per-peer views.

No host-side transfer sender, transfer id allocator, reassembly receiver, or
`SnapshotBody` builder exists in `crpg-server` today (section 8).

## 6. Existing `crpg-server` tests touching snapshots, deltas, epochs or filtering

`crates/crpg-server/tests/host_capture.rs`:

- `valid_action_applied_through_bytes` (687): accepted command produces frames
  starting at event_seq 1 and full filtered state; authority unchanged on
  rejection.
- `hidden_fields_absent_from_snapshots_and_replies` (1027): per-field
  suppression in events and state ops.
- `two_peers_share_lane_sequences_safely` (1211): per-peer delivery sequences
  stay gapless under concurrency.
- `narrowing_grants_resynchronize_without_leak` (1476): narrowing returns
  `RebindRequired`, restarts delivery at seq 1 with fresh filtered state,
  hidden entity's old NetId (2) never appears in later frames, widening mints
  a fresh id (4), `set_control` does not resync.
- `old_epoch_refused_after_restart` (2107): old epoch refused after restart.

`crates/crpg-server/tests/host_quic.rs`:

- `quic_filtered_replica_converges_per_peer` (478): per-peer replicas from
  transport bytes, filtered state/events/receipts.
- `quic_desync_oracle_fails_on_drop_and_corruption` (600): oracle detects
  dropped/corrupt frames.
- `admission_binds_invitation_and_hands_off_epoch` (1306): epoch handoff on
  bind.
- `fencing_closes_with_session_fenced` (1739): narrowing closes connection.

## 7. Constraints that bind T024h

`crates/crpg-server/AGENTS.md`:

- Public surface is five modules `host`, `capture`, `checkpoint`, `save`,
  `quic` with no glob re-export; changing any public type/field/variant/
  signature/cap/`Display` needs an ADR (`crates/crpg-server/AGENTS.md:15-25`).
- Invariants: host is only mutator; determine/reserve/commit; capture before
  anything else moves; disclosure per entity per field deny by default; identity
  never repeats; every ledger bounded; time injected, I/O fenced; no unsafe
  (`crates/crpg-server/AGENTS.md:27-64`).
- Allowed normal deps: `crpg-core`, `crpg-sim`, `crpg-net`, workspace
  `serde`/`serde_json`, `crpg-persist`, `crpg-net-quic`; no dev-deps; no
  `crpg-data`/`crpg-rules` edges (`crates/crpg-server/AGENTS.md:65-79`).
- DoD commands (`crates/crpg-server/AGENTS.md:81-95`).
- Trap: "Narrowing fences; it never calls `set_grants`. A narrowing
  `set_invitation_grants` closes the connection with `SessionFenced` …
  Seamless narrowing is T024/T025a." (`crates/crpg-server/AGENTS.md:156-159`).

`docs/architecture/crpg-server.md`:

- Planned and not here includes "snapshot transfer and seamless narrowing
  (T024)" (`docs/architecture/crpg-server.md:30`).
- Narrowing "restarts a peer's delivery at 1 with a fresh filtered state"
  (`docs/architecture/crpg-server.md:167`).
- "T024 adds snapshot resync, which can make narrowing seamless"
  (`docs/architecture/crpg-server.md:208-209`).

`tools/lint/deps.py`:

- `crpg-server` row: `ALLOWED["crpg-server"] = (None, {"crpg-godot", "crpg-edit"})`
  (`tools/lint/deps.py:69`): may depend on any workspace crate except `crpg-godot` and
  `crpg-edit`; a missing table row is itself a failure (`tools/lint/deps.py:186-193`).
- `crpg-server` is not in `IO_FREE_CRATES` (`tools/lint/deps.py:81`), so I/O-free
  enforcement does not apply to it; AGENTS.md still fences I/O to `save.rs`/
  `quic.rs`.

`tasks/T025b.md` (boundary):

- Blocked on T022/T023b/T025a; 30 s same-process grace, frozen player input
  with continuing authority, fresh session on restart (`tasks/T025b.md:5-15`).
- During grace, retain epoch, lane sequences, canonical request cache and
  terminal receipts; retry returns original receipt (`tasks/T025b.md:19-21`).
- Resume requires authenticated proof and excludes simultaneous authority from
  the old connection (`tasks/T025b.md:22`).
- Full current filtered snapshot replaces old view; old cached payloads cannot
  redisclose revoked facts (`tasks/T025b.md:23-24`).
- No combat escape/reset of turns, resources, effects or deadlines
  (`tasks/T025b.md:24`).
- Out of scope: host migration, account service, prediction, new combat rules,
  generic persistence backend (`tasks/T025b.md:66-69`).
- `T026` receives explicit resume/snapshot reset notifications and fencing
  epochs (`tasks/T025b.md:78`).

## 8. Open facts

Searched `crates/crpg-server/src/` and `crates/crpg-server/tests/` for
`SnapshotBody`, `TransferId`, `ReassemblyPool`, `offer_transfer`,
`plan_transfer`, `ReplicaUpdate`, `TransferDown`, `TransferUp`,
`TransferAssembly`. Result: no matches. Therefore, on this branch:

- There is no `crpg-server` sender for transfer frames, no transfer id
  allocator, no snapshot capture into `SnapshotBody`, and no reassembly
  receiver wiring.
- There is no host API to stage transfer frames in the delivery log or to
  carry `cursor`/`base_tick`/transfer id alongside `event_seq`.
- The delivery log (`Session.log`) is a single `VecDeque<LoggedFrame>` with no
  separate held/queued split; post-cutover held deltas are not represented.
- `Host::set_grants` still resets `next_event_seq` to 1 on narrowing
  (`crates/crpg-server/src/host.rs:707`); no path preserves `event_seq` across a snapshot.
- No `crpg-server` test exercises snapshot transfer, transfer ids, or
  `Published`/`Resync`; the transfer suite lives in
  `crates/crpg-net/tests/snapshot_transfer.rs` (T024).
- No documentation in `docs/architecture/crpg-server.md` describes a sender
  hook or transfer staging area; it lists snapshot transfer as planned
  (`docs/architecture/crpg-server.md:30`).
- `Host::pump` is the host loop referenced as "T022 host loop"; no separate
  loop type exists.

## Agent log

- 2026-10-25 (UTC) · opencode-go/beam-free + T024h-facts · Gathered T024 S§11 sender rules, host delta/snapshot send paths, filtering, transfer API signatures, capture/retirement hooks, existing tests, and binding constraints into a contract-author fact sheet.

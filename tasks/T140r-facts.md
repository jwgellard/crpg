I'll gather the facts from each file.

## 1) `crates/crpg-net/src/reassembly.rs` — `ReassemblyPool` public surface

Pool methods (all `impl ReassemblyPool`, `reassembly.rs:485`):
- `pub fn new(limits: TransferLimits) -> Result<Self, LimitsError>` — `:486`
- `pub fn limits(&self) -> TransferLimits` — `:497`
- `pub fn add_receiver(&mut self, epoch: [u8;16], wire: DeltaWire, start: GateMode) -> Result<ReceiverId, TransferError>` — `:500-505`
- `pub fn remove_receiver(&mut self, id: ReceiverId)` — `:533`
- `pub fn offer_transfer(&mut self, id: ReceiverId, bytes: &[u8], now_ms: u64) -> Result<TransferOutcome, TransferError>` — `:590-595`
- `pub fn offer_delta(&mut self, id: ReceiverId, bytes: &[u8], now_ms: u64) -> Result<DeltaOutcome, TransferError>` — `:893-898`
- `pub fn poll(&mut self, now_ms: u64) -> Vec<ReceiverId>` — `:1009`
- `pub fn pop_update(&mut self, id: ReceiverId) -> Option<ReplicaUpdate>` — `:1016`
- `pub fn status(&self, id: ReceiverId) -> Result<ReceiverStatus, TransferError>` — `:1031`
- `pub fn staging_bytes(&self) -> usize` — `:1064`; `pub fn held_delta_bytes(&self) -> usize` — `:1067`
- `pub fn rebind_receiver(&mut self, id: ReceiverId, now_ms: u64) -> Result<Rebound, TransferError>` — `:1077-1081`

Public types alongside: `AssemblyError` `:20`, `ChunkOutcome` `:41`, `TransferAssembly` `:52` (methods `:60,70,73,109,112,115`), `TransferLimits` `:128` (`v1` `:139`, `validate` `:150`), `LimitsError` `:226`, `DeltaWire` `:250`, `GateMode` `:256`, `GatedDelta` `:262` (`event_seq` `:268`), `Publication` `:277`, `ReplicaUpdate` `:285`, `ReceiverId` `:291`, `TransferOutcome` `:300`, `Rebound` `:318`, `DeltaOutcome` `:326`, `FailReason` `:334`, `TransferError` `:365`, `InFlightStatus` `:402`, `PublishedBase` `:413`, `ReceiverStatus` `:420`.

Version-specific points:
- `DeltaWire` has exactly `V1 | V2` — no v3 arm (`:249-253`); stored per receiver (`Receiver.wire :457`) and reported (`ReceiverStatus.wire :422`, `:1052`).
- Delta codec choice is per-receiver wire: `DeltaWire::V1 => codec::decode_delta`, `DeltaWire::V2 => codec_v2::decode_delta` (`:905-908`); `GatedDelta::{V1,V2}` (`:261-265`) is the only version-carrying payload.
- Transfer path is `0x81`-only: `codec_transfer::decode_down` (`:602`), `TransferDown::{Begin,Chunk,Cancel}` (`:604-616`), `codec_transfer::decode_body` on publish (`:773`), and `Publication.body: SnapshotBody` (`:281`) is the v1/v2 snapshot body.
- `ReplicaUpdate::{Snapshot, Delta}` (`:284-288`) — Snapshot carries `Publication`, Delta carries `GatedDelta`.
- `GateMode::{Live, AwaitSnapshot}` (`:255-259`); set at `:580, :779, :883, :1099`, tested at `:957-961, :974-978`.
- 1 MiB / 32-chunk sizing is v1: `TransferLimits::v1` uses `protocol::MAX_SNAPSHOT_BYTES` (`:143, :176`) and `8` receivers/`128` frames/`2 MiB` (`:139-148`); `TransferAssembly` uses `u32` bitmap (`:55, :87`).
- Delta seq ceiling is shared v1 constant `SEQ_LAST` (`:915`, import `:9`).

## 2) `crates/crpg-net/src/reassembly_v3.rs` — v3 assembly + limits

- Re-exports `AssemblyError, ChunkOutcome, LimitsError` (`:20`); policy consts `POLICY_SNAPSHOT_REASSEMBLY_MS_V3` `:24`, `..._STAGING_BYTES_PER_PEER_V3` `:26`, `..._STAGING_BYTES_HOST_V3` `:28`.
- `TransferAssemblyV3` `:41`; `pub fn new(begin: &BeginTransfer) -> Result<Self, CodecError>` `:50`; `pub fn begin(&self) -> &BeginTransfer` `:61`; `pub fn accept(&mut self, chunk: &TransferChunk) -> Result<ChunkOutcome, AssemblyError>` `:66`; `pub fn received_chunks(&self) -> u8` `:99`; `pub fn is_complete(&self) -> bool` `:104`; `pub fn into_bytes(self) -> Result<Vec<u8>, Self>` `:109`; `u128` bitmap `:44`.
- `TransferLimitsV3` `:120` with fields `:122-134` (same order as v1); `pub fn v3() -> Self` `:139`; `pub fn validate(&self) -> Result<(), LimitsError>` `:154`.

## 3) `crates/crpg-net/src/replica.rs` — `ReplicaWorld`

State (`:121-128`): `epoch: SessionEpoch`, `entities: Vec<ReplicaEntity>` (strictly ascending `NetId`), `turn: Option<TurnFact>`, `next_event_seq`, `server_tick`, `high_water`. Row type `ReplicaEntity { net_id, health: Option<HealthFact> }` `:24-29`; bound `MAX_REPLICA_ENTITIES` `:20`.
API: `new(epoch)` `:132`, `epoch()` `:144`, `entities()` `:149`, `entity(NetId)` `:154`, `len()` `:159`, `is_empty()` `:164`, `turn()` `:169`, `next_event_seq()` `:175`, `server_tick()` `:180`, `apply_delta(&DeltaFrame)` `:188`, `apply_snapshot(&Publication)` `:220`. Error enum `ReplicaError` `:34`.
Delta application: uses `crate::protocol_v2::{DeltaFrame, DeltaOp}` (`:16`); lane → op-count → `event_seq` order (`:189-203`); copy-validate-commit (`:206-212`). Op dispatch `apply_op` `:272-282`; legacy v1 ops `apply_legacy` `:285-347`; v2 ops handled: `ActionResolved` `:275`, `TurnStarted` `:279`, `EncounterEnded` `:280`; all others are `DeltaOp::Legacy` v1 ops. No v3 branch (`Encounter`/`Timeline`/`EntityFacts`/`RoundStarted`/`ModeChanged`/`OrderEnded`/`Joined`/`Left`/`Clock`/`HostFacts` are absent).
Snapshot application: reads `publication.body` (`SnapshotBody`, entities + turn) `:228-262`, checks cursor `:221-227`, row count `:229`, ascending order `:233-244`, id reuse `:240`, active membership `:245-249`; commits replacement `:252-267`. No fact sets, encounter or timeline are stored.

## 4) v3 modules — tags, `0x82` body, decode entry points

Fact op tags: `DELTA_TAG_ENCOUNTER = 11` `protocol_v3.rs:59`, `DELTA_TAG_TIMELINE = 12` `:61`, `DELTA_TAG_ENTITY_FACTS = 13` `:63`; also `ROUND_STARTED 14` `:65`, `MODE_CHANGED 15` `:67`, `ORDER_ENDED 16` `:69`, `JOINED 17` `:71`, `LEFT 18` `:73`, `CLOCK 19` `:75`, `HOST_FACTS 20` `:77`; legacy tags 0–10 are the `DeltaOp` variants at `:753-837`.
v3 delta frame/op types: `DeltaFrame { lane, server_tick, event_seq, ops }` `:740-749`; `DeltaOp` `:753-891` (`Encounter(EncounterFacts)` `:839`, `Timeline { entries }` `:842`, `EntityFacts { entity, facts }` `:849`).
`0x82` body: `TRANSFER_VERSION_V3 = 0x82` `transfer_v3.rs:39`; `SnapshotEntityV3` `:155`, `SnapshotBodyV3 { entities, turn, encounter, timeline }` `:166-175`; `pub fn decode_body(bytes: &[u8]) -> Result<SnapshotBodyV3, CodecError>` `:258` (encode `:187`); caps `MAX_SNAPSHOT_BYTES_V3` `:43`, `MAX_SNAPSHOT_CHUNKS_V3` `:47`, `MAX_ENCODED_BODY_BYTES_V3` `:52`.
Receiver-callable decoders: `codec_v3::decode_delta(bytes) -> Result<DeltaFrame, CodecError>` `codec_v3.rs:1103` (encode `:1077`, `frame_spans` `:1049`); `transfer_v3::decode_down` `:426` (encode `:349`), `decode_up` `:610`, `check_begin` `:94`, `chunk_len` `:132`, `plan_transfer` `:674`. Fact projection: `facts_v3::project_page` `facts_v3.rs:206`, `project_snapshot_v3` `:441`, `clock_op` `:403`, `host_facts_op` `:413`, `PageV3` `:187`, `SnapshotProjectionV3` `:423`; `projection_v3::project_events(candidates: &[EventCandidate]) -> Result<Vec<DeltaOp>, ProjectionError>` `projection_v3.rs:70`, `EventCandidate` `:23-59`.

## 5) Caller usage

`crates/crpg-godot/src/core/client.rs`: `use crpg_net::replica::ReplicaWorld` `:18`; field `replica: ReplicaWorld` `:279`; `ReplicaWorld::new(epoch)` `:292`; each delivered frame is `codec_v2::decode_delta` (`:12, :322`) then `replica.apply_delta(&frame)` with a fatal 53 on refusal (`:326-329`); reads `entities()` `:409`, `entity()` `:433`, `epoch()` `:516`, `server_tick()` `:519`. No `ReassemblyPool`, no `apply_snapshot` in this file.
`crates/crpg-net/tests/replica.rs`: `use crpg_net::replica::{ReplicaEntity, ReplicaError, ReplicaWorld, MAX_REPLICA_ENTITIES}` `:24`; builds `Publication` locally `:163-171` (and `:272-299`) rather than via a pool; drives `ReplicaWorld::new`/`apply_delta`/`apply_snapshot` throughout (`:173-175, :317, :913, :1956`). It does not reference `ReassemblyPool`.

## 6) Tests exercising `ReassemblyPool`

- `crates/crpg-net/tests/snapshot_transfer.rs` (helpers `:537-622`): `wire_identifiers_and_caps_are_pinned` `:725`, `limits_validate_tightening_only` `:2265`, `max_valid_body_publishes_atomically` `:2466`, `second_in_flight_refused` `:2574`, `staging_caps_per_receiver_and_aggregate` `:2618`, `duplicates_idempotent_no_deadline_extension` `:2765`, `conflicts_discard_retire_and_flag` `:2869`, `reorder_and_missing_chunks` `:2969`, `deadline_boundary_4999_5000` `:3064`, `expiry_frees_and_retry_publishes` `:3151`, `old_mixed_epoch_and_id_refusals` `:3258`, `invalid_body_never_publishes` `:3357`, `cutover_releases_in_fifo_order` `:3476`, `await_snapshot_holds_until_publication` `:3644`, `delta_hold_caps_and_overflow` `:3769`, `delta_sequence_and_wire_refusals` `:3982`, `cancel_retires_and_supersedes` `:4081`, `revocation_never_republishes_hidden_facts` `:4247`, `hidden_values_never_reach_bytes` `:4355`, `oracle_catches_dropped_and_corrupt_chunks` `:4506`, `fabric_transfer_with_faults_and_resync` `:4731`.
- `crates/crpg-net/tests/snapshot_transfer_contract.rs`: `limits_validate_tightening_only` `:1268`, `assembly_duplicate_conflict_and_shape` `:1355`, `max_valid_body_publishes_atomically` `:1495`, `second_in_flight_refused` `:1604`, `staging_caps_per_receiver_and_aggregate` `:1640`, `refusals_leave_the_whole_pool_unchanged` `:1737`, `failures_discard_retire_and_resync` `:1825`, `invalid_body_never_publishes` `:1938`, `wrong_epoch_and_behind_replica_refusals` `:2042`, `queued_snapshot_is_replaced_by_next_publication` `:2109`, `cancel_retires_and_supersedes` `:2141`, `expiry_sweep_frees_and_retry_publishes` `:2212`, `cutover_releases_in_fifo_order` `:2269`, `await_snapshot_holds_until_publication` `:2401`, `delta_hold_caps_and_overflow` `:2458`, `delta_byte_and_pool_caps_overflow` `:2505`, `delta_sequence_and_wire_refusals` `:2556`, `duplicates_idempotent_no_deadline_extension` `:2775`, `publication_over_a_queued_snapshot_clears_the_whole_queue` `:2906`, `queued_deltas_count_toward_every_delta_cap` `:3004`, `overflow_drops_held_deltas_but_keeps_queued_updates` `:3103`, `held_deltas_survive_expired_conflict_and_invalid_body_failures` `:3235`, `drain_stops_at_the_inflight_cursor_and_publication_drops_covered_deltas` `:3460`, `identical_held_delta_is_a_duplicate_and_a_different_one_conflicts` `:3531`, `remove_receiver_releases_held_and_queued_delta_bytes` `:3547`, `a_stale_clock_is_clamped_when_a_deadline_is_set` `:3623`.
- `crates/crpg-net/tests/reconnect_wire.rs` (helpers `:1496-1524, :1633`): `rebind_keeps_floors_and_fails_closed` `:2166`, `resume_cutover_installs_snapshot_before_post_cutover_deltas` `:2469`, `retry_identity_across_resume` `:2729`, `obsolete_transfer_never_overwrites_the_resume_snapshot` `:2950`, `resume_snapshot_oracle_and_corruption` `:3319`.
## Agent log
- 2026-11-11 (UTC) · opencode/deepseek-v4.1-flash-free (scout) + T140r fact sheet · Gathered the receiver, replica and v3 decoder facts with line citations so the T140r contract can be written without re-reading the code; pointers only, not verified evidence.

# T025a facts — `crpg-net` reconnect/resync mechanisms (D12)

Fact sheet only. Every `path:line` below exists on this branch. No
recommendations, proposed types, or opinions.

## 1. Lane-0 v1/v2 codecs

- `PROTOCOL_VERSION` v1 = 1 (`crates/crpg-net/src/protocol.rs:25`); v2 = 2
  (`crates/crpg-net/src/protocol_v2.rs:34`); transfer subprotocol first byte
  `TRANSFER_VERSION = 0x81` (`crates/crpg-net/src/protocol_transfer.rs:19`).
- `LANE_COMBAT = 0` (`crates/crpg-net/src/protocol.rs:34`);
  `LANE_SNAPSHOT = 2` (`crates/crpg-net/src/protocol_transfer.rs:22`).
- `SEQ_FIRST = 1`, `SEQ_LAST = u64::MAX - 1`
  (`crates/crpg-net/src/protocol.rs:41,48`).
- Wire hard maxima: `MAX_INTENT_FRAME_BYTES = 4096` (`protocol.rs:58`),
  `MAX_DELTA_FRAME_BYTES = 65536` (`protocol.rs:64`),
  `MAX_DELTA_OPS = 256` (`protocol.rs:70`),
  `MAX_WIRE_COLLECTION = 256` (`protocol.rs:73`),
  `MAX_WIRE_STRING_BYTES = 256` (`protocol.rs:80`),
  `MAX_WIRE_BYTES_FIELD = 4096` (`protocol.rs:83`),
  `MAX_SNAPSHOT_BYTES = 1_048_576` (`protocol.rs:89`),
  `MAX_VISIBLE_ENTITIES = 1024` (`protocol.rs:92`),
  `MAX_SNAPSHOT_CHUNKS = 32` (`protocol.rs:95`).
- Transfer caps: `MAX_SEGMENT_BYTES = 4096` (`protocol_transfer.rs:40`),
  `MAX_SEGMENTS_PER_CHUNK = 8` (`protocol_transfer.rs:42`),
  `MAX_CHUNK_BYTES = 32768` (`protocol_transfer.rs:44`),
  `MAX_DOWN_FRAME_BYTES = 65536` (`protocol_transfer.rs:46`),
  `MAX_UP_FRAME_BYTES = 4096` (`protocol_transfer.rs:48`),
  `MAX_ENCODED_BODY_BYTES = 22552` (`protocol_transfer.rs:50`).
- Operational policy (tightening-only, host-supplied, never wire):
  `POLICY_*` constants (`protocol.rs:107-165`); fabric defaults
  `QueueCaps::v1()/v1_egress()` and `RateCaps::v1()`
  (`crates/crpg-net/src/sim.rs:83-147`).
- Frozen v1 tags: intent 0/1 (`protocol.rs:172-174`), delta 0..7
  (`protocol.rs:176-191`), receipt 0/1 (`protocol.rs:193-196`).
- v2 new tags: 8/9/10 (`protocol_v2.rs:37-43`).
- Transfer tags: downstream 0x10/0x11/0x12, upstream 0x20/0x21
  (`protocol_transfer.rs:25-33`).
- Identity types: `SessionEpoch(pub [u8; 16])` (`protocol.rs:206-207`),
  `NetId(u64)` with `new`/`get` (`protocol.rs:216-233`),
  `TransferId(u64)` with `new`/`get` (`protocol_transfer.rs:66-82`).
- v1 message enums: `IntentFrame`/`IntentBody` (`protocol.rs:244-279`),
  `DeltaFrame`/`DeltaOp` (`protocol.rs:291-366`), `ReceiptStatus`
  (`protocol.rs:377-383`), `RejectionCode` (`protocol.rs:391-435`).
- v2 message enums: `DeltaFrame`/`DeltaOp` with `Legacy` plus
  `ActionResolved`/`TurnStarted`/`EncounterEnded`
  (`protocol_v2.rs:67-119`); re-exports v1 `IntentFrame`/`IntentBody`/
  `NetId`/`SessionEpoch`/`ReceiptStatus`/`RejectionCode`
  (`protocol_v2.rs:49-51`).
- Transfer message enums: `TransferDown`/`TransferUp`
  (`protocol_transfer.rs:233-270`), `BeginTransfer`, `TransferChunk`,
  `CancelReason`, `ResyncReason` (`protocol_transfer.rs:88-231`),
  `SnapshotBody`/`SnapshotEntity`/`HealthFact`/`TurnFact`
  (`protocol_transfer.rs:276-312`).
- Projection: `EventCandidate`, `ProjectionError`, `project_events`
  (`crates/crpg-net/src/projection_v2.rs:53-219`).
- Error enums: `CodecError` (`FrameTooLarge`, `UnsupportedVersion`,
  `WrongDirection`, `UnknownMessage`, `Malformed`, `LimitExceeded`)
  (`crates/crpg-net/src/codec.rs:51-66`), re-exported by `codec_v2`
  (`codec_v2.rs:68`) and `codec_transfer` (`codec_transfer.rs:9`).
- Assembly errors: `AssemblyError` (`WrongTransfer`/`OutOfRange`/
  `WrongLength`/`Conflict`) (`reassembly.rs:19-37`); `LimitsError`
  (`InvalidLimits`/`LoosensPolicy`) (`reassembly.rs:225-229`); `TransferError`
  (`reassembly.rs:356-369`); `FailReason` (`Conflict`/`InvalidBody`/
  `Expired`/`Overflow`) (`reassembly.rs:325-341`).
- Encode/decode signatures: `encode_intent`/`decode_intent`/
  `encode_delta`/`decode_delta` (`codec.rs:149,185,386,412`); v2 same
  names (`codec_v2.rs:242,277,567,595`); transfer `encode_down`/
  `decode_down`/`encode_up`/`decode_up`/`encode_body`/`decode_body`/
  `plan_transfer` (`codec_transfer.rs:160,246,393,432,32,97,488`).
- Reassembly entry points: `TransferAssembly::new`/`accept`/`into_bytes`
  (`reassembly.rs:60-120`); `ReassemblyPool::new`/`add_receiver`/
  `remove_receiver`/`offer_transfer`/`offer_delta`/`pop_update`/`status`/
  `poll` (`reassembly.rs:476-490,524-533,581-619,884-998,1007-1021,1022-1054,1000-1006`).
- ADR: `docs/adr/0019-event-protocol-v2.md`; transfer ADR:
  `docs/adr/0028-snapshot-transfer-subprotocol.md`.

## 2. Session epoch and sequence/dedup state

- `SessionEpoch` is a host-issued 16-byte session generation; a new epoch
  resets per-lane numbering; session reattachment not supported in v1
  (`protocol.rs:202-205`).
- `IntentFrame.epoch/lane/seq` carry command identity
  (`protocol.rs:244-258`); `DeltaOp::Receipt.epoch/lane/seq/processed_tick`
  echo it (`protocol.rs:353-366`).
- Per-peer lane state lives in `PeerState` (`sim.rs:235-251`):
  `epoch: [u8; 16]`, `lane: u8`, `next_new_seq: u64`,
  `cache: Vec<(u64, Vec<u8>, ReceiptStatus)>`,
  `fail_budget`/`byte_budget: TokenBucket` (`sim.rs:247-250`).
- `PeerState::new` binds epoch with empty seq/cache state and full failure
  budgets (`sim.rs:253-271`); `is_exhausted` reports `next_new_seq > SEQ_LAST`
  (`sim.rs:283-285`); `try_consume_failure` spends one failure response
  against both budgets (`sim.rs:297-301`).
- Cache depth/size policy: `POLICY_RETRY_CACHE_PER_PEER_LANE = 256`,
  `POLICY_RETRY_CACHE_BYTES_PER_PEER_LANE = 2 * 1024 * 1024`
  (`protocol.rs:145-149`).
- Driver admission keeps `states: BTreeMap<PeerId, PeerState>` and
  `closed: BTreeSet<[u8; 16]>` (closed epochs) (`tests/support/driver.rs:227-228`).
- Seq/cache decision logic: `SeqDecision` enum
  (`tests/support/driver.rs:191-208`); cache lookup in `admit_status`
  (`tests/support/driver.rs:433-445`): cached retry if canonical bytes match,
  `SeqConflict` if same seq different bytes, `SeqExhausted`/`SeqGap`/
  `StaleSeq` otherwise.
- Terminal outcomes cache canonical bytes and advance `next_new_seq`
  (`tests/support/driver.rs:550-570`); `finalize` pushes
  `(seq, bytes, status)` and evicts oldest over 256 entries or 2 MiB
  (`tests/support/driver.rs:561-567`).
- Observed-tick field: `IntentFrame.observed_tick: u64`
  (`protocol.rs:252-253`); freshness window `POLICY_OBSERVED_TICK_WINDOW = 200`
  (`protocol.rs:165`); checks in `admit_new` (`tests/support/driver.rs:467-482`).
- Reset/reattach: `NetDriver.bind_peer` replaces a peer's `PeerState` with a
  fresh one (`tests/support/driver.rs:277-287`); `InMemoryTransport.add_peer`
  allocates a new monotonic `PeerId` (`sim.rs:430-465`); `remove_peer`
  releases staging (`sim.rs:467-473`).
- No resume request/result type exists in `crpg-net` today (searched
  `crates/crpg-net`: no `Resume`/`reconnect` symbol).

## 3. Snapshot transfer (T024)

- Separate subprotocol: first byte `0x81`, lane 2
  (`protocol_transfer.rs:19,22`); ADR `docs/adr/0028-snapshot-transfer-subprotocol.md`.
- Downstream/upstream tags `0x10`/`0x11`/`0x12` and `0x20`/`0x21`
  (`protocol_transfer.rs:25-33`).
- Segment/chunk caps: `MAX_SEGMENT_BYTES = 4096` (`protocol_transfer.rs:40`),
  `MAX_SEGMENTS_PER_CHUNK = 8` (`protocol_transfer.rs:42`),
  `MAX_CHUNK_BYTES = 32768` (`protocol_transfer.rs:44`),
  `MAX_SNAPSHOT_CHUNKS = 32` (`protocol.rs:95`),
  `MAX_SNAPSHOT_BYTES = 1_048_576` (`protocol.rs:89`).
- `BeginTransfer` fields and validation
  (`protocol_transfer.rs:88-145`); `chunk_len` helper
  (`protocol_transfer.rs:147-165`).
- Receiver pool: `ReassemblyPool` with `TransferLimits` (max 8 receivers,
  5000 ms deadline, 1 MiB per receiver, 8 MiB aggregate, 128 delta frames,
  2 MiB per receiver, 16 MiB total) (`reassembly.rs:127-149`,
  `reassembly.rs:466-490`).
- `TransferLimits::validate` enforces tightening-only (`LoosensPolicy`)
  (`reassembly.rs:150-222`).
- Atomic publication: `ReassemblyPool::publish` decodes body, frees
  reservation, retires transfer id, clears queued snapshots, drops held deltas
  with `event_seq <= cursor`, sets `published`/`next_event_seq = cursor + 1`,
  queues `ReplicaUpdate::Snapshot`, then moves contiguous held deltas
  (`reassembly.rs:739-851`).
- Delivery-cursor cutover gate: `offer_delta` compares `eseq` to
  `next_event_seq`; superseded if `eseq < next_event_seq`; buffers otherwise;
  releases contiguous sequence starting at `next_event_seq` only in `Live`
  mode and when `eseq <= in_flight cursor` (`reassembly.rs:884-998`).
- Transfer id retirement: `retired_through` high-water mark;
  `handle_begin` rejects `transfer <= retired_through` as `StaleTransfer`
  (`reassembly.rs:641-644`); `handle_chunk` same (`reassembly.rs:733-736`);
  `fail_in_flight` sets `retired_through = max(...)` (`reassembly.rs:563-580`);
  `handle_cancel` retires on cancel (`reassembly.rs:852-883`).
- Obsolete partial transfer cannot overwrite later resync: `handle_begin`
  rejects `BehindReplica` when cursor+1 < next_event_seq or base_tick behind
  published (`reassembly.rs:649-660`); `InFlight` when another transfer in
  flight (`reassembly.rs:646-648`).
- Revocation/cancellation: `handle_cancel` discards in-flight reservation and
  retires id (`reassembly.rs:852-883`); `remove_receiver` frees staging
  (`reassembly.rs:524-533`); overflow drops held deltas
  (`reassembly.rs:573-578`).
- Expiry: `sweep` fails in-flight transfers with `Expired` when
  `now_ms >= deadline_ms` (`reassembly.rs:534-562`); `poll` returns expired
  receiver ids (`reassembly.rs:1000-1006`).
- `TransferOutcome`/`DeltaOutcome`/`ReceiverStatus` definitions
  (`reassembly.rs:300-322,411-423`).

## 4. Command identity and retry

- Command identity fields are epoch/lane/seq/observed_tick/actor/body in
  `IntentFrame` (`protocol.rs:244-258`).
- Canonical bytes are the raw encoded intent bytes retained in
  `PeerState.cache: Vec<(u64, Vec<u8>, ReceiptStatus)>`
  (`sim.rs:244-246`).
- Retry comparison uses byte equality of canonical bytes
  (`tests/support/driver.rs:435-439`).
- Retry returns original `ReceiptStatus` without re-execution
  (`tests/support/driver.rs:447`); duplicate delivery of same canonical bytes
  is `Cached` (`tests/support/driver.rs:436-437`).
- `seq` handling: `next_new_seq` starts at 1 and advances only on terminal
  outcomes (`sim.rs:241-243`, `tests/support/driver.rs:568`).
- `observed_tick` is retained in the frame and checked against
  `server_tick` with `POLICY_OBSERVED_TICK_WINDOW`
  (`protocol.rs:252-253,165`; `tests/support/driver.rs:467-482`).
- Receipt carries `processed_tick` set to `server_tick`
  (`tests/support/driver.rs:594-604`).
- No rewritten observed tick on retry: admit reuses the decoded frame's
  `observed_tick` (`tests/support/driver.rs:419-422`).
- No `observed_tick` mutation in codec: encode/decode pass through
  (`codec.rs:163,205`; `codec_v2.rs:251,292`).

## 5. Connection identity and fencing

- `SessionEpoch` is the only session-generation identity in net
  (`protocol.rs:202-207`); documented as replay isolation, not authentication
  credentials (`protocol.rs:204`).
- `PeerState.epoch` binds a peer to one epoch (`sim.rs:237-238`).
- Driver rejects mismatched/closed epochs with `SessionExpired`
  (`tests/support/driver.rs:423-432`); closed epochs stored in `closed`
  (`tests/support/driver.rs:228,448-451`).
- `InMemoryTransport.add_peer` takes `_epoch` but ignores it; binding lives in
  drivers (`sim.rs:430-436`).
- `PeerId` is a monotonic, never-reused `u32` handle (`sim.rs:214-226`).
- No connection-generation token, fence type, or stale-connection rejection
  beyond epoch match/closed set found in `crates/crpg-net`.

## 6. Credentials and redaction

- No token, credential, opaque-blob, or session-proof type exists in
  `crates/crpg-net/src` (searched all modules).
- `SessionEpoch` is documented as not authentication credentials
  (`protocol.rs:204`); projection candidate values are not credentials
  (`projection_v2.rs:7`).
- Types derive `Debug` without redaction (e.g. `SessionEpoch`
  `protocol.rs:206`, `PeerState` `sim.rs:235`, `TransferDown`
  `protocol_transfer.rs:234`); no `Debug` redaction implementation found.

## 7. Per-lane identity and policy limits

- B3 (per-lane identity): per-epoch per-lane seq stream
  (`protocol.rs:36-48`); `PeerState.lane` and `next_new_seq` per peer
  (`sim.rs:239-243`); `IntentFrame.lane` (`protocol.rs:249`);
  `DeltaFrame.lane` (`protocol.rs:293`); cache is per peer per lane by policy
  (`protocol.rs:145-149`); `POLICY_RETRY_CACHE_PER_PEER_LANE`
  (`protocol.rs:147`).
- B5 (two-tier limits): wire hard maxima `MAX_*` in `protocol.rs:51-95`
  (versioned; raising requires protocol review); operational policy `POLICY_*`
  in `protocol.rs:97-165` (tightening-only, never wire).
- Fabric: `QueueCaps::v1()`/`v1_egress()` from policy
  (`sim.rs:83-105`); `RateCaps::v1()` (`sim.rs:132-147`); comments state
  tightening-only overrides (`sim.rs:84-85,95-96,133-134`).
- Transfer: `TransferLimits::v1()` and `validate()` enforce tightening-only
  with `LoosensPolicy` (`reassembly.rs:138-222`).
- Sources: E017 Appendix B §B3 (`tasks/E017-t018-interface-debt.md:484-515`),
  §B5 (`tasks/E017-t018-interface-debt.md:527-544`).

## 8. Simulated transport and conformance

- Simulated transport types: `SimClock` (`sim.rs:43-61`),
  `QueueCaps` (`sim.rs:72-105`), `RateCaps` (`sim.rs:113-147`),
  `TokenBucket` (`sim.rs:157-208`), `PeerId` (`sim.rs:219-226`),
  `PeerState` (`sim.rs:235-302`), `FaultSchedule` (`sim.rs:321-342`,
  `clean` at `sim.rs:333-342`), `InMemoryTransport`
  (`sim.rs:396-607`).
- `InMemoryTransport` API: `new` (`sim.rs:411-423`), `clock_mut`
  (`sim.rs:426-428`), `add_peer` (`sim.rs:430-465`), `remove_peer`
  (`sim.rs:467-473`), `send_to` (`sim.rs:486-531`), `recv_from`
  (`sim.rs:535-561`), `tick` (`sim.rs:572-606`).
- Local transport trait: `Transport` with `Error`/`send`/`recv`
  (`crates/crpg-net/src/transport.rs:14-23`).
- Conformance suite: `tests/conformance.rs` (imports `InMemoryTransport`,
  `NetDriver`, `Fixture`; module wiring at `conformance.rs:13-16`).
- Driver hooks: `NetDriver` (`tests/support/driver.rs:217`),
  `bind_peer` (`driver.rs:277`), `admit`/`admit_status`
  (`driver.rs:397,418`), `deltas` (`driver.rs:723`), `redeliver`
  (`driver.rs:851`), `project` (`driver.rs:875`), `assign`/`revoke`
  (`driver.rs:305,317`).
- No `reconnect_wire` test file exists in `crates/crpg-net/tests` today.

## Open seams

- No resume/reconnect request or result type exists in `crpg-net`.
- No connection-generation or fencing token beyond `SessionEpoch` and the
  driver's closed-epoch set.
- No credential/token/opaque-blob type and no `Debug` redaction.
- No command-in-flight "observed tick" rewrite field beyond the existing
  advisory `observed_tick` (no separate "original tick on retry" type).
- No cross-epoch snapshot guard in transfer code beyond `WrongEpoch` on
  epoch mismatch (`reassembly.rs`/`offer_transfer`).
- No resume-specific codec functions or tags; transfer codec has no resume
  message.
- `crpg-net` has no grace timer or disconnect state.

## Agent log

- 2026-10-25 (UTC) · opencode-go/beam-free + T025a facts · Gathered existing crpg-net shapes, limits, state, transfer, and test hooks with citations for the T025a contract.
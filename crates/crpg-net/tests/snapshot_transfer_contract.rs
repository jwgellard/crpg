//! Independent contract tests for the T024 snapshot transfer subprotocol.
//!
//! Derived from `tasks/T024.md` S§4–S§10 only, through the public `crpg-net`
//! API. Each test names the contract section it proves.

use std::collections::BTreeMap;

use crpg_net::codec::{self, CodecError};
use crpg_net::codec_transfer::{
    decode_body, decode_down, decode_up, encode_body, encode_down, encode_up, plan_transfer,
};
use crpg_net::codec_v2;
use crpg_net::protocol::{
    DeltaFrame as DeltaV1, DeltaOp as OpV1, IntentBody, IntentFrame, LANE_COMBAT,
    MAX_DELTA_FRAME_BYTES, MAX_INTENT_FRAME_BYTES, MAX_SNAPSHOT_BYTES, MAX_SNAPSHOT_CHUNKS,
    MAX_VISIBLE_ENTITIES, MAX_WIRE_BYTES_FIELD, PROTOCOL_VERSION, SEQ_LAST,
};
use crpg_net::protocol_transfer::{
    BeginTransfer, CancelReason, HealthFact, NetId, ResyncReason, SnapshotBody, SnapshotEntity,
    TransferChunk, TransferDown, TransferId, TransferUp, TurnFact, LANE_SNAPSHOT, MAX_CHUNK_BYTES,
    MAX_DOWN_FRAME_BYTES, MAX_ENCODED_BODY_BYTES, MAX_SEGMENTS_PER_CHUNK, MAX_SEGMENT_BYTES,
    MAX_UP_FRAME_BYTES, POLICY_SNAPSHOT_STAGING_BYTES_HOST, POLICY_SNAPSHOT_STAGING_BYTES_PER_PEER,
    TRANSFER_TAG_BEGIN, TRANSFER_TAG_CANCEL, TRANSFER_TAG_CHUNK, TRANSFER_TAG_PUBLISHED,
    TRANSFER_TAG_RESYNC, TRANSFER_VERSION,
};
use crpg_net::protocol_v2::{DeltaFrame as DeltaV2, DeltaOp as OpV2, PROTOCOL_VERSION as V2};
use crpg_net::reassembly::{
    AssemblyError, ChunkOutcome, DeltaOutcome, DeltaWire, FailReason, GateMode, GatedDelta,
    LimitsError, PublishedBase, ReassemblyPool, ReceiverId, ReceiverStatus, ReplicaUpdate,
    TransferAssembly, TransferError, TransferLimits, TransferOutcome,
};

const EPOCH: [u8; 16] = [0u8; 16];

type HealthRow = (u64, Option<(u32, u32, bool)>);
type LimitMutation = (fn(&mut TransferLimits), &'static str);

fn ep(byte: u8) -> [u8; 16] {
    [byte; 16]
}

fn varint(mut value: u64) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let low = (value & 0x7f) as u8;
        value >>= 7;
        if value == 0 {
            out.push(low);
            break;
        }
        out.push(low | 0x80);
    }
    out
}

fn tid(raw: u64) -> TransferId {
    TransferId::new(raw).expect("nonzero transfer id")
}

fn net(raw: u64) -> NetId {
    NetId::new(raw).expect("nonzero net id")
}

fn begin(
    id: u64,
    base_tick: u64,
    cursor: u64,
    total_bytes: u32,
    entity_count: u16,
    segments_per_chunk: u8,
    chunk_count: u8,
) -> BeginTransfer {
    BeginTransfer {
        epoch: EPOCH,
        transfer: tid(id),
        base_tick,
        cursor,
        total_bytes,
        entity_count,
        segments_per_chunk,
        chunk_count,
    }
}

fn begin_frame(value: &BeginTransfer) -> Vec<u8> {
    encode_down(&TransferDown::Begin(value.clone())).expect("begin encodes")
}

fn with_epoch(mut value: BeginTransfer, epoch: [u8; 16]) -> BeginTransfer {
    value.epoch = epoch;
    value
}

fn two_chunk_body() -> Vec<u8> {
    let facts: Vec<HealthRow> = (1..=800).map(|id| (id, Some((10, 10, false)))).collect();
    encode_body(&body_of(&facts, None)).expect("two-chunk body")
}

fn chunk_of(value: &BeginTransfer, body: &[u8], index: u8) -> TransferChunk {
    let chunk_size = value.segments_per_chunk as usize * MAX_SEGMENT_BYTES;
    let start = index as usize * chunk_size;
    let len = value.chunk_len(index).expect("chunk in range");
    TransferChunk {
        epoch: value.epoch,
        transfer: value.transfer,
        index,
        bytes: body[start..start + len].to_vec(),
    }
}

fn chunk_frame(value: &BeginTransfer, body: &[u8], index: u8) -> Vec<u8> {
    encode_down(&TransferDown::Chunk(chunk_of(value, body, index))).expect("chunk encodes")
}

fn frames_of(value: &BeginTransfer, body: &[u8]) -> Vec<Vec<u8>> {
    (0..value.chunk_count)
        .map(|index| chunk_frame(value, body, index))
        .collect()
}

fn body_of(entities: &[HealthRow], turn: Option<TurnFact>) -> SnapshotBody {
    SnapshotBody {
        entities: entities
            .iter()
            .map(|(id, health)| SnapshotEntity {
                entity: net(*id),
                health: health.map(|(health, max_health, dead)| HealthFact {
                    health,
                    max_health,
                    dead,
                }),
            })
            .collect(),
        turn,
    }
}

fn maximal_body() -> SnapshotBody {
    let first = u64::MAX - 1_023;
    SnapshotBody {
        entities: (0..MAX_VISIBLE_ENTITIES)
            .map(|i| SnapshotEntity {
                entity: net(first + i as u64),
                health: Some(HealthFact {
                    health: u32::MAX,
                    max_health: u32::MAX,
                    dead: true,
                }),
            })
            .collect(),
        turn: Some(TurnFact {
            active: Some(net(u64::MAX)),
            round: u64::MAX,
        }),
    }
}

fn narrowed(body: &SnapshotBody) -> SnapshotBody {
    let mut copy = body.clone();
    copy.entities[0].health = None;
    copy
}

fn plan(
    epoch: [u8; 16],
    id: u64,
    base_tick: u64,
    cursor: u64,
    segments_per_chunk: u8,
    body: &SnapshotBody,
) -> Result<Vec<Vec<u8>>, CodecError> {
    plan_transfer(epoch, tid(id), base_tick, cursor, segments_per_chunk, body)
}

fn v1_delta(seq: u64, ops: Vec<OpV1>) -> Vec<u8> {
    codec::encode_delta(&DeltaV1 {
        lane: LANE_COMBAT,
        server_tick: 0,
        event_seq: seq,
        ops,
    })
    .expect("v1 delta encodes")
}

fn v1_delta_of_len(seq: u64, len: usize) -> Vec<u8> {
    // version, lane, server_tick 0, event_seq; op-count 0 adds one byte
    let base = 4 + varint(seq).len();
    for three in [1usize, 0] {
        let Some(rest) = len.checked_sub(base) else {
            continue;
        };
        for two in 0..=256usize {
            let count = two + three;
            if count > 256 {
                break;
            }
            if 3 * three + 2 * two + varint(count as u64).len() != rest + 1 {
                continue;
            }
            let mut ops = Vec::new();
            if three == 1 {
                ops.push(OpV1::Died { entity: net(200) });
            }
            for _ in 0..two {
                ops.push(OpV1::Died { entity: net(1) });
            }
            let bytes = v1_delta(seq, ops);
            assert_eq!(
                bytes.len(),
                len,
                "padded delta length seq={seq} two={two} three={three}"
            );
            return bytes;
        }
    }
    panic!("cannot build a v1 delta of {len} bytes");
}

/// A v1 delta of exactly `len` bytes for any `len` up to the 256-op ceiling,
/// padded with maximal `Health` ops and then small `Died` ops.
fn v1_delta_padded(seq: u64, len: usize) -> Vec<u8> {
    let big = OpV1::Health {
        entity: net(u64::MAX),
        health: u32::MAX,
        max_health: u32::MAX,
        dead: true,
    };
    for bigs in (0..=256usize.min(len / 20)).rev() {
        let base = v1_delta(seq, vec![big.clone(); bigs]).len();
        if base > len {
            continue;
        }
        for three in 0..=1usize {
            for two in 0..=(len - base) / 2 + 1 {
                if bigs + two + three > 256 {
                    break;
                }
                let mut ops = vec![big.clone(); bigs];
                if three == 1 {
                    ops.push(OpV1::Died { entity: net(200) });
                }
                ops.extend((0..two).map(|_| OpV1::Died { entity: net(1) }));
                let bytes = v1_delta(seq, ops);
                if bytes.len() == len {
                    return bytes;
                }
            }
        }
    }
    panic!("cannot build a v1 delta of {len} bytes");
}

fn v2_delta(seq: u64, ops: Vec<OpV2>) -> Vec<u8> {
    codec_v2::encode_delta(&DeltaV2 {
        lane: LANE_COMBAT,
        server_tick: 0,
        event_seq: seq,
        ops,
    })
    .expect("v2 delta encodes")
}

fn health_op(entity: u64, health: u32) -> OpV1 {
    OpV1::Health {
        entity: net(entity),
        health,
        max_health: 10,
        dead: false,
    }
}

fn raw_down(tag: u8, fields: &[u8]) -> Vec<u8> {
    let mut bytes = vec![TRANSFER_VERSION, LANE_SNAPSHOT, tag];
    bytes.extend_from_slice(fields);
    bytes
}

fn raw_up(tag: u8, fields: &[u8]) -> Vec<u8> {
    let mut bytes = vec![TRANSFER_VERSION, LANE_SNAPSHOT, tag];
    bytes.extend_from_slice(fields);
    bytes
}

fn raw_begin(
    transfer: u64,
    base_tick: u64,
    cursor: u64,
    total: &[u8],
    entity_count: &[u8],
    segments: u8,
    chunks: u8,
) -> Vec<u8> {
    let mut fields = vec![0u8; 16];
    fields.extend(varint(transfer));
    fields.extend(varint(base_tick));
    fields.extend(varint(cursor));
    fields.extend_from_slice(total);
    fields.extend_from_slice(entity_count);
    fields.push(segments);
    fields.push(chunks);
    raw_down(TRANSFER_TAG_BEGIN, &fields)
}

fn raw_chunk(transfer: u64, index: u8, segments: &[(Vec<u8>, Vec<u8>)]) -> Vec<u8> {
    let mut fields = vec![0u8; 16];
    fields.extend(varint(transfer));
    fields.push(index);
    fields.push(segments.len() as u8);
    for (len, body) in segments {
        fields.extend_from_slice(len);
        fields.extend_from_slice(body);
    }
    raw_down(TRANSFER_TAG_CHUNK, &fields)
}

fn raw_cancel(transfer: u64, reason: u8) -> Vec<u8> {
    let mut fields = vec![0u8; 16];
    fields.extend(varint(transfer));
    fields.push(reason);
    raw_down(TRANSFER_TAG_CANCEL, &fields)
}

fn raw_resync(after: u64, reason: u8) -> Vec<u8> {
    let mut fields = vec![0u8; 16];
    fields.extend(varint(after));
    fields.push(reason);
    raw_up(TRANSFER_TAG_RESYNC, &fields)
}

fn receiver(
    pool: &mut ReassemblyPool,
    epoch: [u8; 16],
    wire: DeltaWire,
    mode: GateMode,
) -> ReceiverId {
    pool.add_receiver(epoch, wire, mode)
        .expect("receiver added")
}

fn status(pool: &ReassemblyPool, id: ReceiverId) -> ReceiverStatus {
    pool.status(id).expect("known receiver")
}

fn offer_all(
    pool: &mut ReassemblyPool,
    id: ReceiverId,
    frames: &[Vec<u8>],
    now_ms: u64,
) -> Result<TransferOutcome, TransferError> {
    let mut last = Ok(TransferOutcome::Begun);
    for frame in frames {
        last = pool.offer_transfer(id, frame, now_ms);
    }
    last
}

fn limits(per_receiver: usize, total: usize) -> TransferLimits {
    let mut value = TransferLimits::v1();
    value.staging_bytes_per_receiver = per_receiver;
    value.staging_bytes_total = total;
    value
}

fn delta_limits(per_receiver: usize, total: usize) -> TransferLimits {
    let mut value = TransferLimits::v1();
    value.delta_bytes_per_receiver = per_receiver;
    value.delta_bytes_total = total;
    value
}

#[allow(clippy::too_many_arguments)]
fn publish(
    pool: &mut ReassemblyPool,
    id: ReceiverId,
    epoch: [u8; 16],
    transfer: u64,
    base_tick: u64,
    cursor: u64,
    segments_per_chunk: u8,
    body: &SnapshotBody,
    now_ms: u64,
) -> Result<TransferOutcome, TransferError> {
    let frames =
        plan(epoch, transfer, base_tick, cursor, segments_per_chunk, body).expect("canonical plan");
    offer_all(pool, id, &frames, now_ms)
}

/// Test-local, independent oracle: the only consumer of popped updates.
#[derive(Debug, Default)]
struct Oracle {
    health: BTreeMap<u64, u32>,
    cursor: u64,
}

impl Oracle {
    fn apply(&mut self, update: ReplicaUpdate) -> Result<(), String> {
        match update {
            ReplicaUpdate::Snapshot(publication) => {
                for entity in &publication.body.entities {
                    if let Some(fact) = &entity.health {
                        self.health.insert(entity.entity.get(), fact.health);
                    }
                }
                self.cursor = publication.cursor;
                Ok(())
            }
            ReplicaUpdate::Delta(delta) => {
                let seq = delta.event_seq();
                if seq <= self.cursor {
                    return Err(format!("delta {seq} at or below cursor {}", self.cursor));
                }
                match delta {
                    GatedDelta::V1(frame) => {
                        for op in &frame.ops {
                            if let OpV1::Health { entity, health, .. } = op {
                                self.health.insert(entity.get(), *health);
                            }
                        }
                    }
                    GatedDelta::V2(frame) => {
                        for op in &frame.ops {
                            if let OpV2::Legacy(OpV1::Health { entity, health, .. }) = op {
                                self.health.insert(entity.get(), *health);
                            }
                        }
                    }
                }
                self.cursor = seq;
                Ok(())
            }
        }
    }

    fn matches(&self, expected: &BTreeMap<u64, u32>) -> bool {
        &self.health == expected
    }
}

fn expect_unchanged<T: PartialEq + std::fmt::Debug>(
    pool: &ReassemblyPool,
    call: impl FnOnce(&mut ReassemblyPool) -> T,
) -> T {
    let mut scratch = pool.clone();
    let before = scratch.clone();
    let result = call(&mut scratch);
    assert_eq!(scratch, before, "a refused call changed pool state");
    result
}

// ---------------------------------------------------------------- S§2/S§4/S§7

#[test]
fn wire_identifiers_and_caps_are_pinned() {
    assert_eq!(TRANSFER_VERSION, 0x81);
    assert_eq!(LANE_SNAPSHOT, 2);
    assert_eq!(TRANSFER_TAG_BEGIN, 0x10);
    assert_eq!(TRANSFER_TAG_CHUNK, 0x11);
    assert_eq!(TRANSFER_TAG_CANCEL, 0x12);
    assert_eq!(TRANSFER_TAG_PUBLISHED, 0x20);
    assert_eq!(TRANSFER_TAG_RESYNC, 0x21);
    assert_ne!(TRANSFER_VERSION, PROTOCOL_VERSION);
    assert_ne!(TRANSFER_VERSION, V2);
    for tag in [
        TRANSFER_TAG_BEGIN,
        TRANSFER_TAG_CHUNK,
        TRANSFER_TAG_CANCEL,
        TRANSFER_TAG_PUBLISHED,
        TRANSFER_TAG_RESYNC,
    ] {
        assert!(!(0..=10).contains(&tag), "tag {tag:#04x} collides");
    }

    assert_eq!(MAX_SEGMENT_BYTES, MAX_WIRE_BYTES_FIELD);
    assert_eq!(MAX_SEGMENTS_PER_CHUNK, 8);
    assert_eq!(MAX_CHUNK_BYTES, 32_768);
    assert_eq!(MAX_CHUNK_BYTES * MAX_SNAPSHOT_CHUNKS, MAX_SNAPSHOT_BYTES);
    assert_eq!(
        MAX_SEGMENT_BYTES * MAX_SEGMENTS_PER_CHUNK as usize,
        MAX_CHUNK_BYTES
    );
    assert_eq!(MAX_DOWN_FRAME_BYTES, MAX_DELTA_FRAME_BYTES);
    assert_eq!(MAX_UP_FRAME_BYTES, MAX_INTENT_FRAME_BYTES);
    assert_eq!(MAX_ENCODED_BODY_BYTES, 22_552);
    assert_eq!(POLICY_SNAPSHOT_STAGING_BYTES_PER_PEER, 1_048_576);
    assert_eq!(POLICY_SNAPSHOT_STAGING_BYTES_HOST, 8_388_608);
    assert_eq!(MAX_SNAPSHOT_BYTES, 1_048_576);
    assert_eq!(MAX_VISIBLE_ENTITIES, 1_024);

    let v1 = TransferLimits::v1();
    assert_eq!(v1.max_receivers, 8);
    assert_eq!(v1.deadline_ms, 5_000);
    assert_eq!(v1.staging_bytes_per_receiver, 1_048_576);
    assert_eq!(v1.staging_bytes_total, 8_388_608);
    assert_eq!(v1.delta_frames_per_receiver, 128);
    assert_eq!(v1.delta_bytes_per_receiver, 2_097_152);
    assert_eq!(v1.delta_bytes_total, 16_777_216);
    assert_eq!(v1.validate(), Ok(()));

    assert_eq!(TransferId::new(0), None);
    assert_eq!(TransferId::new(1).map(TransferId::get), Some(1));
    assert_eq!(CancelReason::from_u8(1), Some(CancelReason::Superseded));
    assert_eq!(CancelReason::from_u8(2), Some(CancelReason::Aborted));
    assert_eq!(CancelReason::from_u8(0), None);
    assert_eq!(CancelReason::Aborted.as_u8(), 2);
    assert_eq!(ResyncReason::from_u8(1), Some(ResyncReason::Expired));
    assert_eq!(ResyncReason::from_u8(2), Some(ResyncReason::Invalid));
    assert_eq!(ResyncReason::from_u8(3), Some(ResyncReason::Overflow));
    assert_eq!(ResyncReason::Expired.as_u8(), 1);

    assert_eq!(FailReason::Expired.resync_reason(), ResyncReason::Expired);
    assert_eq!(FailReason::Conflict.resync_reason(), ResyncReason::Invalid);
    assert_eq!(
        FailReason::InvalidBody(CodecError::Malformed).resync_reason(),
        ResyncReason::Invalid
    );
    assert_eq!(FailReason::Overflow.resync_reason(), ResyncReason::Overflow);
}

// ---------------------------------------------------------------- S§5.5

#[test]
fn literal_byte_oracles_round_trip() {
    let mut minimal_begin = vec![TRANSFER_VERSION, LANE_SNAPSHOT, TRANSFER_TAG_BEGIN];
    minimal_begin.extend_from_slice(&EPOCH);
    minimal_begin.extend_from_slice(&[1, 0, 0, 2, 0, 1, 1]);
    assert_eq!(minimal_begin.len(), 26);
    let begin_value = begin(1, 0, 0, 2, 0, 1, 1);
    assert_eq!(begin_frame(&begin_value), minimal_begin);
    assert_eq!(
        decode_down(&minimal_begin),
        Ok(TransferDown::Begin(begin_value.clone()))
    );

    let mut chunk = vec![TRANSFER_VERSION, LANE_SNAPSHOT, TRANSFER_TAG_CHUNK];
    chunk.extend_from_slice(&EPOCH);
    chunk.extend_from_slice(&[1, 0, 1, 2, 0, 0]);
    assert_eq!(chunk.len(), 25);
    let chunk_value = TransferChunk {
        epoch: EPOCH,
        transfer: tid(1),
        index: 0,
        bytes: vec![0, 0],
    };
    assert_eq!(
        encode_down(&TransferDown::Chunk(chunk_value.clone())),
        Ok(chunk.clone())
    );
    assert_eq!(
        decode_down(&chunk),
        Ok(TransferDown::Chunk(chunk_value.clone()))
    );

    let mut cancel = vec![TRANSFER_VERSION, LANE_SNAPSHOT, TRANSFER_TAG_CANCEL];
    cancel.extend_from_slice(&EPOCH);
    cancel.extend_from_slice(&[1, 1]);
    assert_eq!(cancel.len(), 21);
    assert_eq!(
        encode_down(&TransferDown::Cancel {
            epoch: EPOCH,
            transfer: tid(1),
            reason: CancelReason::Superseded,
        }),
        Ok(cancel.clone())
    );
    assert_eq!(
        decode_down(&cancel),
        Ok(TransferDown::Cancel {
            epoch: EPOCH,
            transfer: tid(1),
            reason: CancelReason::Superseded,
        })
    );

    let mut published = vec![TRANSFER_VERSION, LANE_SNAPSHOT, TRANSFER_TAG_PUBLISHED];
    published.extend_from_slice(&EPOCH);
    published.push(1);
    assert_eq!(published.len(), 20);
    assert_eq!(
        encode_up(&TransferUp::Published {
            epoch: EPOCH,
            transfer: tid(1),
        }),
        Ok(published.clone())
    );
    assert_eq!(
        decode_up(&published),
        Ok(TransferUp::Published {
            epoch: EPOCH,
            transfer: tid(1),
        })
    );

    let mut resync = vec![TRANSFER_VERSION, LANE_SNAPSHOT, TRANSFER_TAG_RESYNC];
    resync.extend_from_slice(&EPOCH);
    resync.extend_from_slice(&[0, 1]);
    assert_eq!(resync.len(), 21);
    assert_eq!(
        encode_up(&TransferUp::Resync {
            epoch: EPOCH,
            after: None,
            reason: ResyncReason::Expired,
        }),
        Ok(resync.clone())
    );
    assert_eq!(
        decode_up(&resync),
        Ok(TransferUp::Resync {
            epoch: EPOCH,
            after: None,
            reason: ResyncReason::Expired,
        })
    );

    let alone = body_of(&[(1, Some((10, 10, false)))], None);
    let mut alone_bytes = vec![0x01, 0x01, 0x01, 0x0A, 0x0A, 0x00, 0x00];
    assert_eq!(encode_body(&alone), Ok(alone_bytes.clone()));
    assert_eq!(decode_body(&alone_bytes), Ok(alone.clone()));
    assert_eq!(alone_bytes.len(), 7);

    let with_turn = body_of(
        &[(1, Some((10, 10, false)))],
        Some(TurnFact {
            active: Some(net(1)),
            round: 3,
        }),
    );
    alone_bytes.truncate(6);
    alone_bytes.extend_from_slice(&[0x01, 0x01, 0x01, 0x03]);
    assert_eq!(encode_body(&with_turn), Ok(alone_bytes.clone()));
    assert_eq!(decode_body(&alone_bytes), Ok(with_turn));

    let mut wide = vec![0u8; 4_097];
    for (i, byte) in wide.iter_mut().enumerate() {
        *byte = (i % 251) as u8;
    }
    let wide_frame = encode_down(&TransferDown::Chunk(TransferChunk {
        epoch: EPOCH,
        transfer: tid(1),
        index: 0,
        bytes: wide.clone(),
    }))
    .expect("4,097-byte chunk");
    let after_header = 3 + 16 + 1 + 1;
    assert_eq!(wide_frame[after_header], 2, "two segments");
    assert_eq!(
        &wide_frame[after_header + 1..after_header + 3],
        &[0x80, 0x20]
    );
    assert_eq!(wide_frame.len(), 3 + 16 + 1 + 1 + 1 + 2 + 4_096 + 1 + 1);
}

// ---------------------------------------------------------------- S§1.3/S§6

#[test]
fn max_frames_fit_the_frame_ceilings() {
    let begin_at_max = BeginTransfer {
        epoch: [0xFF; 16],
        transfer: tid(u64::MAX),
        base_tick: u64::MAX,
        cursor: SEQ_LAST,
        total_bytes: 1_048_576,
        entity_count: 1_024,
        segments_per_chunk: 8,
        chunk_count: 32,
    };
    let bytes = begin_frame(&begin_at_max);
    assert_eq!(bytes.len(), 56);
    assert_eq!(decode_down(&bytes), Ok(TransferDown::Begin(begin_at_max)));

    let mut payload = vec![0u8; 32_768];
    for (i, byte) in payload.iter_mut().enumerate() {
        *byte = (i % 251) as u8;
    }
    let full = TransferChunk {
        epoch: [0xFF; 16],
        transfer: tid(u64::MAX),
        index: 0,
        bytes: payload.clone(),
    };
    let full_bytes = encode_down(&TransferDown::Chunk(full.clone())).expect("full chunk encodes");
    assert_eq!(full_bytes.len(), 32_815);
    assert!(full_bytes.len() <= MAX_DELTA_FRAME_BYTES);
    assert_eq!(decode_down(&full_bytes), Ok(TransferDown::Chunk(full)));

    let mut too_big = payload;
    too_big.push(0);
    assert_eq!(
        encode_down(&TransferDown::Chunk(TransferChunk {
            epoch: EPOCH,
            transfer: tid(1),
            index: 0,
            bytes: too_big,
        })),
        Err(CodecError::LimitExceeded)
    );
    assert_eq!(
        encode_down(&TransferDown::Chunk(TransferChunk {
            epoch: EPOCH,
            transfer: tid(1),
            index: 0,
            bytes: Vec::new(),
        })),
        Err(CodecError::Malformed)
    );

    assert_eq!(
        encode_down(&TransferDown::Cancel {
            epoch: [0xFF; 16],
            transfer: tid(u64::MAX),
            reason: CancelReason::Aborted,
        })
        .map(|bytes| bytes.len()),
        Ok(30)
    );
    assert_eq!(
        encode_up(&TransferUp::Published {
            epoch: [0xFF; 16],
            transfer: tid(u64::MAX),
        })
        .map(|bytes| bytes.len()),
        Ok(29)
    );
    assert_eq!(
        encode_up(&TransferUp::Resync {
            epoch: [0xFF; 16],
            after: Some(tid(u64::MAX)),
            reason: ResyncReason::Overflow,
        })
        .map(|bytes| bytes.len()),
        Ok(30)
    );
}

// ---------------------------------------------------------------- S§6

#[test]
fn down_decode_stage_precedence() {
    assert_eq!(decode_down(&[]), Err(CodecError::Malformed));

    let mut overhead = vec![0u8; MAX_DOWN_FRAME_BYTES + 1];
    overhead[0] = 0x01;
    assert_eq!(decode_down(&overhead), Err(CodecError::FrameTooLarge));
    overhead[0] = TRANSFER_VERSION;
    assert_eq!(decode_down(&overhead), Err(CodecError::FrameTooLarge));

    let at_cap = vec![0u8; MAX_DOWN_FRAME_BYTES];
    assert_eq!(decode_down(&at_cap), Err(CodecError::UnsupportedVersion));
    for version in [0x00, 0x01, 0x02, 0x80, 0x82] {
        assert_eq!(
            decode_down(&[version]),
            Err(CodecError::UnsupportedVersion),
            "version {version:#04x}"
        );
    }

    assert_eq!(
        decode_down(&[TRANSFER_VERSION, PROTOCOL_VERSION, TRANSFER_TAG_BEGIN]),
        Err(CodecError::WrongDirection)
    );
    assert_eq!(
        decode_down(&[TRANSFER_VERSION, 1, TRANSFER_TAG_BEGIN]),
        Err(CodecError::WrongDirection)
    );
    assert_eq!(
        decode_down(&[TRANSFER_VERSION, 3, TRANSFER_TAG_CHUNK]),
        Err(CodecError::WrongDirection)
    );
    for tag in [TRANSFER_TAG_PUBLISHED, TRANSFER_TAG_RESYNC] {
        assert_eq!(
            decode_down(&[TRANSFER_VERSION, LANE_SNAPSHOT, tag]),
            Err(CodecError::WrongDirection)
        );
    }
    for tag in [0x00, 0x08, 0x13, 0xFF] {
        assert_eq!(
            decode_down(&[TRANSFER_VERSION, LANE_SNAPSHOT, tag]),
            Err(CodecError::UnknownMessage)
        );
    }
    for reason in [0, 3, 4] {
        assert_eq!(
            decode_down(&raw_cancel(1, reason)),
            Err(CodecError::UnknownMessage)
        );
    }

    // Earliest stage wins on combined faults.
    assert_eq!(
        decode_down(&[TRANSFER_VERSION, 0, TRANSFER_TAG_PUBLISHED]),
        Err(CodecError::WrongDirection)
    );
    let mut both = vec![0u8; MAX_DOWN_FRAME_BYTES + 1];
    both[0] = TRANSFER_VERSION;
    both[1] = 0;
    both[2] = TRANSFER_TAG_PUBLISHED;
    assert_eq!(decode_down(&both), Err(CodecError::FrameTooLarge));
}

#[test]
fn down_fields_checked_left_to_right() {
    let two = varint(2);
    assert_eq!(
        decode_down(&raw_begin(0, 0, 0, &two, &[0], 1, 1)),
        Err(CodecError::Malformed)
    );
    assert_eq!(
        decode_down(&raw_begin(1, 0, u64::MAX, &two, &[0], 1, 1)),
        Err(CodecError::Malformed)
    );
    assert_eq!(
        decode_down(&raw_begin(1, 0, 0, &[0], &[0], 1, 1)),
        Err(CodecError::Malformed)
    );
    assert_eq!(
        decode_down(&raw_begin(1, 0, 0, &varint(1_048_577), &[0], 1, 1)),
        Err(CodecError::LimitExceeded)
    );
    assert_eq!(
        decode_down(&raw_begin(1, 0, 0, &two, &varint(1_025), 1, 1)),
        Err(CodecError::LimitExceeded)
    );
    assert_eq!(
        decode_down(&raw_begin(1, 0, 0, &two, &[0], 0, 1)),
        Err(CodecError::Malformed)
    );
    assert_eq!(
        decode_down(&raw_begin(1, 0, 0, &two, &[0], 9, 1)),
        Err(CodecError::LimitExceeded)
    );
    assert_eq!(
        decode_down(&raw_begin(1, 0, 0, &two, &[0], 1, 0)),
        Err(CodecError::Malformed)
    );
    assert_eq!(
        decode_down(&raw_begin(1, 0, 0, &two, &[0], 1, 2)),
        Err(CodecError::Malformed)
    );
    assert_eq!(
        decode_down(&raw_begin(
            1,
            0,
            0,
            &varint(1_048_576),
            &[0x80, 0x08],
            8,
            33
        )),
        Err(CodecError::LimitExceeded)
    );

    // A hostile 10-byte total is a cap failure, never a wrap; a real varint
    // overflow is Malformed.
    assert_eq!(
        decode_down(&raw_begin(1, 0, 0, &varint(u64::MAX), &[0], 1, 1)),
        Err(CodecError::LimitExceeded)
    );
    let mut overflow = vec![0xFF; 9];
    overflow.push(0x02);
    assert_eq!(
        decode_down(&raw_begin(1, 0, 0, &overflow, &[0], 1, 1)),
        Err(CodecError::Malformed)
    );

    // encode_down and TransferAssembly::new use the same Begin routine.
    for broken in [
        begin(1, 0, 0, 0, 0, 1, 1),
        begin(1, 0, 0, 1_048_577, 0, 1, 1),
        begin(1, 0, 0, 2, 1_025, 1, 1),
        begin(1, 0, 0, 2, 0, 9, 1),
        begin(1, 0, 0, 2, 0, 1, 2),
    ] {
        let expected = broken.validate().expect_err("invalid begin");
        assert_eq!(
            encode_down(&TransferDown::Begin(broken.clone())),
            Err(expected)
        );
        assert_eq!(TransferAssembly::new(&broken).err(), Some(expected));
    }
}

#[test]
fn chunk_segment_length_checked_before_body() {
    assert_eq!(
        decode_down(&raw_chunk(1, 32, &[(varint(1), vec![0])])),
        Err(CodecError::LimitExceeded)
    );
    assert_eq!(
        decode_down(&raw_chunk(1, 31, &[(varint(1), vec![0])])),
        Ok(TransferDown::Chunk(TransferChunk {
            epoch: EPOCH,
            transfer: tid(1),
            index: 31,
            bytes: vec![0],
        }))
    );
    assert_eq!(
        decode_down(&raw_chunk(1, 0, &[])),
        Err(CodecError::Malformed)
    );
    let nine: Vec<(Vec<u8>, Vec<u8>)> = (0..9).map(|_| (varint(0), Vec::new())).collect();
    assert_eq!(
        decode_down(&raw_chunk(1, 0, &nine)),
        Err(CodecError::LimitExceeded)
    );
    assert_eq!(
        decode_down(&raw_chunk(1, 0, &[(varint(0), Vec::new())])),
        Err(CodecError::Malformed)
    );
    assert_eq!(
        decode_down(&raw_chunk(1, 0, &[(varint(4_097), Vec::new())])),
        Err(CodecError::LimitExceeded)
    );
    assert_eq!(
        decode_down(&raw_chunk(1, 0, &[(varint(u64::MAX), Vec::new())])),
        Err(CodecError::LimitExceeded)
    );
    let mut overflow = vec![0xFF; 9];
    overflow.push(0x02);
    assert_eq!(
        decode_down(&raw_chunk(1, 0, &[(overflow, Vec::new())])),
        Err(CodecError::Malformed)
    );
    assert_eq!(
        decode_down(&raw_chunk(
            1,
            0,
            &[(varint(4_095), vec![0; 4_095]), (varint(1), vec![0])]
        )),
        Err(CodecError::Malformed)
    );
    assert_eq!(
        decode_down(&raw_chunk(1, 0, &[(varint(2), vec![0])])),
        Err(CodecError::Malformed)
    );

    let mut valid = raw_chunk(1, 0, &[(varint(1), vec![0])]);
    valid.push(0);
    assert_eq!(decode_down(&valid), Err(CodecError::Malformed));

    assert_eq!(
        encode_down(&TransferDown::Chunk(TransferChunk {
            epoch: EPOCH,
            transfer: tid(1),
            index: 32,
            bytes: Vec::new(),
        })),
        Err(CodecError::LimitExceeded)
    );
}

#[test]
fn up_decode_precedence() {
    assert_eq!(decode_up(&[]), Err(CodecError::Malformed));
    let too_big = vec![0u8; MAX_UP_FRAME_BYTES + 1];
    assert_eq!(decode_up(&too_big), Err(CodecError::FrameTooLarge));
    let at_cap = vec![0u8; MAX_UP_FRAME_BYTES];
    assert_eq!(decode_up(&at_cap), Err(CodecError::UnsupportedVersion));

    for tag in [TRANSFER_TAG_BEGIN, TRANSFER_TAG_CHUNK, TRANSFER_TAG_CANCEL] {
        assert_eq!(
            decode_up(&[TRANSFER_VERSION, LANE_SNAPSHOT, tag]),
            Err(CodecError::WrongDirection)
        );
    }
    assert_eq!(
        decode_up(&[TRANSFER_VERSION, 0, TRANSFER_TAG_BEGIN]),
        Err(CodecError::WrongDirection)
    );
    for tag in [0x00, 0x13, 0xFF] {
        assert_eq!(
            decode_up(&[TRANSFER_VERSION, LANE_SNAPSHOT, tag]),
            Err(CodecError::UnknownMessage)
        );
    }
    assert_eq!(
        decode_up(&[TRANSFER_VERSION, LANE_SNAPSHOT, TRANSFER_TAG_PUBLISHED]),
        Err(CodecError::Malformed)
    );
    for reason in [0, 4] {
        assert_eq!(
            decode_up(&raw_resync(0, reason)),
            Err(CodecError::UnknownMessage)
        );
    }
    assert_eq!(
        decode_up(&raw_resync(0, 1)),
        Ok(TransferUp::Resync {
            epoch: EPOCH,
            after: None,
            reason: ResyncReason::Expired,
        })
    );
    let mut trailing = raw_resync(0, 1);
    trailing.push(0);
    assert_eq!(decode_up(&trailing), Err(CodecError::Malformed));
}

#[test]
fn transfer_and_lane_zero_refuse_each_other() {
    let transfer_frames = [
        begin_frame(&begin(1, 0, 0, 2, 0, 1, 1)),
        raw_chunk(1, 0, &[(varint(1), vec![0])]),
        raw_cancel(1, 1),
        encode_up(&TransferUp::Published {
            epoch: EPOCH,
            transfer: tid(1),
        })
        .expect("published"),
        raw_resync(0, 1),
    ];
    for frame in &transfer_frames {
        assert_eq!(
            codec::decode_intent(frame),
            Err(CodecError::UnsupportedVersion)
        );
        assert_eq!(
            codec::decode_delta(frame),
            Err(CodecError::UnsupportedVersion)
        );
        assert_eq!(
            codec_v2::decode_intent(frame),
            Err(CodecError::UnsupportedVersion)
        );
        assert_eq!(
            codec_v2::decode_delta(frame),
            Err(CodecError::UnsupportedVersion)
        );
    }

    let v1_delta_frame = v1_delta(1, vec![health_op(1, 5)]);
    let v2_delta_frame = v2_delta(1, vec![OpV2::Legacy(health_op(1, 5))]);
    assert_eq!(
        decode_down(&v1_delta_frame),
        Err(CodecError::UnsupportedVersion)
    );
    assert_eq!(
        decode_up(&v1_delta_frame),
        Err(CodecError::UnsupportedVersion)
    );
    assert_eq!(
        decode_down(&v2_delta_frame),
        Err(CodecError::UnsupportedVersion)
    );
    assert_eq!(
        decode_up(&v2_delta_frame),
        Err(CodecError::UnsupportedVersion)
    );

    for tag in [0x10, 0x11, 0x12, 0x20, 0x21] {
        let v1_op = vec![PROTOCOL_VERSION, LANE_COMBAT, 0, 1, 1, tag];
        let v2_op = vec![V2, LANE_COMBAT, 0, 1, 1, tag];
        assert_eq!(codec::decode_delta(&v1_op), Err(CodecError::UnknownMessage));
        assert_eq!(
            codec_v2::decode_delta(&v2_op),
            Err(CodecError::UnknownMessage)
        );
    }

    let intent = IntentFrame {
        epoch: EPOCH,
        lane: LANE_COMBAT,
        seq: 1,
        observed_tick: 0,
        actor: net(1),
        body: IntentBody::EndTurn,
    };
    let mut intent_bytes = codec::encode_intent(&intent).expect("intent encodes");
    let last = intent_bytes.len() - 1;
    intent_bytes[last] = TRANSFER_TAG_BEGIN;
    assert_eq!(
        codec::decode_intent(&intent_bytes),
        Err(CodecError::UnknownMessage)
    );
}

// ---------------------------------------------------------------- S§5.4/S§6

#[test]
fn body_codec_bounds() {
    let maximal = maximal_body();
    let encoded = encode_body(&maximal).expect("maximal body encodes");
    assert_eq!(encoded.len(), MAX_ENCODED_BODY_BYTES);
    assert_eq!(decode_body(&encoded), Ok(maximal.clone()));
    assert_eq!(encode_body(&decode_body(&encoded).unwrap()), Ok(encoded));

    let mut too_many = maximal.clone();
    too_many.entities.insert(
        0,
        SnapshotEntity {
            entity: net(u64::MAX - 1_024),
            health: None,
        },
    );
    assert_eq!(too_many.entities.len(), 1_025);
    assert_eq!(encode_body(&too_many), Err(CodecError::LimitExceeded));
    assert_eq!(decode_body(&[0x81, 0x08]), Err(CodecError::LimitExceeded));

    assert_eq!(
        decode_body(&[0x02, 0x01, 0x00, 0x01, 0x00]),
        Err(CodecError::Malformed)
    );
    assert_eq!(
        decode_body(&[0x02, 0x02, 0x00, 0x01, 0x00]),
        Err(CodecError::Malformed)
    );
    assert_eq!(
        decode_body(&[0x01, 0x00, 0x00, 0x00]),
        Err(CodecError::Malformed)
    );
    assert_eq!(decode_body(&[0x01, 0x01, 0x02]), Err(CodecError::Malformed));
    assert_eq!(
        decode_body(&[0x01, 0x01, 0x00, 0x01, 0x01, 0x09, 0x03]),
        Err(CodecError::Malformed)
    );
    assert_eq!(decode_body(&[0x01, 0x01, 0x01]), Err(CodecError::Malformed));
    assert_eq!(
        decode_body(&[0x01, 0x01, 0x01, 0x0A, 0x0A, 0x00, 0x00, 0x00]),
        Err(CodecError::Malformed)
    );
    assert_eq!(
        decode_body(&[0x01, 0x01, 0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0x1F]),
        Err(CodecError::Malformed)
    );
    assert_eq!(
        decode_body(&vec![0u8; MAX_SNAPSHOT_BYTES + 1]),
        Err(CodecError::LimitExceeded)
    );

    let mut descending = body_of(&[(2, None), (1, None)], None);
    assert_eq!(encode_body(&descending), Err(CodecError::Malformed));
    descending.entities.reverse();
    assert_eq!(
        encode_body(&descending),
        Ok(vec![0x02, 0x01, 0x00, 0x02, 0x00, 0x00])
    );

    let absent_active = body_of(
        &[(1, None)],
        Some(TurnFact {
            active: Some(net(9)),
            round: 3,
        }),
    );
    assert_eq!(encode_body(&absent_active), Err(CodecError::Malformed));
    assert_eq!(encode_body(&body_of(&[], None)), Ok(vec![0x00, 0x00]));
    assert_eq!(decode_body(&[0x00, 0x00]), Ok(body_of(&[], None)));
}

#[test]
fn canonical_chunk_len_boundaries() {
    let one = begin(1, 0, 0, 1, 0, 1, 1);
    assert_eq!(one.chunk_len(0), Some(1));
    assert_eq!(one.chunk_len(1), None);

    let two = begin(1, 0, 0, 2, 0, 1, 1);
    assert_eq!(two.chunk_len(0), Some(2));
    assert_eq!(two.chunk_len(1), None);

    let full_chunk = begin(1, 0, 0, 32_768, 0, 8, 1);
    assert_eq!(full_chunk.chunk_len(0), Some(32_768));
    assert_eq!(full_chunk.chunk_len(1), None);

    let spill = begin(1, 0, 0, 32_769, 0, 8, 2);
    assert_eq!(spill.chunk_len(0), Some(32_768));
    assert_eq!(spill.chunk_len(1), Some(1));
    assert_eq!(spill.chunk_len(2), None);

    let eight = begin(1, 0, 0, 1_048_576, 0, 8, 32);
    assert_eq!(eight.chunk_len(31), Some(32_768));
    assert_eq!(eight.chunk_len(32), None);

    let narrow = begin(1, 0, 0, 131_072, 0, 1, 32);
    assert_eq!(narrow.chunk_len(0), Some(4_096));
    assert_eq!(narrow.chunk_len(31), Some(4_096));

    assert_eq!(begin(1, 0, 0, 2, 0, 0, 1).chunk_len(0), None);
    assert_eq!(begin(1, 0, 0, 2, 0, 1, 2).chunk_len(0), None);
    assert_eq!(begin(1, 0, 0, 2, 0, 1, 1).chunk_len(0), Some(2));
}

#[test]
fn plan_transfer_is_canonical() {
    let empty = body_of(&[], None);
    let frames = plan(EPOCH, 1, 0, 0, 1, &empty).expect("plans");
    assert_eq!(frames.len(), 2);
    assert_eq!(frames[0], begin_frame(&begin(1, 0, 0, 2, 0, 1, 1)));
    assert_eq!(
        frames[1],
        raw_down(TRANSFER_TAG_CHUNK, &{
            let mut fields = vec![0u8; 16];
            fields.extend_from_slice(&[1, 0, 1, 2, 0, 0]);
            fields
        })
    );

    assert_eq!(
        plan(EPOCH, 1, 0, u64::MAX, 1, &empty),
        Err(CodecError::Malformed)
    );
    assert_eq!(
        plan(EPOCH, 1, 0, SEQ_LAST, 0, &empty),
        Err(CodecError::Malformed)
    );
    assert_eq!(
        plan(EPOCH, 1, 0, 0, 9, &empty),
        Err(CodecError::LimitExceeded)
    );

    let maximal = maximal_body();
    let body_bytes = encode_body(&maximal).expect("body encodes");
    let narrow = plan(EPOCH, 2, 7, 5, 1, &maximal).expect("plans");
    assert_eq!(narrow.len(), 1 + 6);
    let begin_narrow = decode_down(&narrow[0]).expect("begin");
    match begin_narrow {
        TransferDown::Begin(value) => {
            assert_eq!(value.total_bytes as usize, body_bytes.len());
            assert_eq!(value.entity_count as usize, maximal.entities.len());
            assert_eq!(value.chunk_count, 6);
            assert_eq!(value.base_tick, 7);
            assert_eq!(value.cursor, 5);
        }
        other => panic!("expected begin, got {other:?}"),
    }
    let mut joined = Vec::new();
    for frame in &narrow[1..] {
        match decode_down(frame).expect("chunk") {
            TransferDown::Chunk(chunk) => joined.extend_from_slice(&chunk.bytes),
            other => panic!("expected chunk, got {other:?}"),
        }
    }
    assert_eq!(joined, body_bytes);

    let wide = plan(EPOCH, 3, 7, 5, 8, &maximal).expect("plans");
    assert_eq!(wide.len(), 2);

    let mut too_many = narrow_body_entities();
    too_many.push(SnapshotEntity {
        entity: net(u64::MAX),
        health: None,
    });
    assert_eq!(too_many.len(), 1_025);
    assert_eq!(
        plan(
            EPOCH,
            4,
            0,
            0,
            1,
            &SnapshotBody {
                entities: too_many,
                turn: None
            }
        ),
        Err(CodecError::LimitExceeded)
    );
}

fn narrow_body_entities() -> Vec<SnapshotEntity> {
    (0..1_024)
        .map(|i| SnapshotEntity {
            entity: net(i + 1),
            health: None,
        })
        .collect()
}

#[test]
fn limits_validate_tightening_only() {
    assert_eq!(TransferLimits::v1().validate(), Ok(()));

    let loosened: [LimitMutation; 7] = [
        (|l| l.max_receivers = 9, "max_receivers"),
        (|l| l.deadline_ms = 5_001, "deadline_ms"),
        (
            |l| l.staging_bytes_per_receiver = 1_048_577,
            "staging_bytes_per_receiver",
        ),
        (|l| l.staging_bytes_total = 8_388_609, "staging_bytes_total"),
        (
            |l| l.delta_frames_per_receiver = 129,
            "delta_frames_per_receiver",
        ),
        (
            |l| l.delta_bytes_per_receiver = 2_097_153,
            "delta_bytes_per_receiver",
        ),
        (|l| l.delta_bytes_total = 16_777_217, "delta_bytes_total"),
    ];
    for (change, field) in loosened {
        let mut value = TransferLimits::v1();
        change(&mut value);
        assert_eq!(
            value.validate(),
            Err(LimitsError::LoosensPolicy { field }),
            "{field}"
        );
        assert_eq!(
            ReassemblyPool::new(value).err(),
            Some(LimitsError::LoosensPolicy { field })
        );
    }

    let invalid: [LimitMutation; 7] = [
        (|l| l.max_receivers = 0, "max_receivers"),
        (|l| l.deadline_ms = 0, "deadline_ms"),
        (
            |l| l.staging_bytes_per_receiver = 22_551,
            "staging_bytes_per_receiver",
        ),
        (|l| l.staging_bytes_total = 1_048_575, "staging_bytes_total"),
        (
            |l| l.delta_frames_per_receiver = 0,
            "delta_frames_per_receiver",
        ),
        (
            |l| l.delta_bytes_per_receiver = 65_535,
            "delta_bytes_per_receiver",
        ),
        (|l| l.delta_bytes_total = 2_097_151, "delta_bytes_total"),
    ];
    for (change, field) in invalid {
        let mut value = TransferLimits::v1();
        change(&mut value);
        assert_eq!(
            value.validate(),
            Err(LimitsError::InvalidLimits { field }),
            "{field}"
        );
    }

    let mut two_bad = TransferLimits::v1();
    two_bad.max_receivers = 0;
    two_bad.deadline_ms = 0;
    assert_eq!(
        two_bad.validate(),
        Err(LimitsError::InvalidLimits {
            field: "max_receivers"
        })
    );

    assert_eq!(
        format!(
            "{}",
            LimitsError::InvalidLimits {
                field: "deadline_ms"
            }
        ),
        "InvalidLimits at transfer/limits: deadline_ms"
    );
}

// ---------------------------------------------------------------- S§9

#[test]
fn assembly_duplicate_conflict_and_shape() {
    let begin_value = begin(1, 0, 0, 4_097, 0, 1, 2);
    let mut assembly = TransferAssembly::new(&begin_value).expect("assembly");
    assert_eq!(assembly.received_chunks(), 0);
    assert!(!assembly.is_complete());

    let first = chunk_of(&begin_value, &vec![7u8; 4_097], 0);
    let second = chunk_of(&begin_value, &vec![7u8; 4_097], 1);
    assert_eq!(assembly.accept(&first), Ok(ChunkOutcome::Stored));
    assert_eq!(assembly.accept(&first), Ok(ChunkOutcome::Duplicate));

    let conflicting = TransferChunk {
        bytes: vec![9u8; 4_096],
        ..first.clone()
    };
    let before = assembly.clone();
    assert_eq!(assembly.accept(&conflicting), Err(AssemblyError::Conflict));
    assert_eq!(assembly, before);

    let out_of_range = TransferChunk {
        index: 2,
        bytes: vec![0],
        ..first.clone()
    };
    let before = assembly.clone();
    assert_eq!(
        assembly.accept(&out_of_range),
        Err(AssemblyError::OutOfRange)
    );
    assert_eq!(assembly, before);

    let mut short = first.clone();
    short.bytes.truncate(4_095);
    let before = assembly.clone();
    assert_eq!(assembly.accept(&short), Err(AssemblyError::WrongLength));
    assert_eq!(assembly, before);

    let mut long = second.clone();
    long.bytes.push(0);
    let before = assembly.clone();
    assert_eq!(assembly.accept(&long), Err(AssemblyError::WrongLength));
    assert_eq!(assembly, before);

    let wrong_epoch = TransferChunk {
        epoch: ep(1),
        ..first.clone()
    };
    let wrong_id = TransferChunk {
        transfer: tid(2),
        ..first.clone()
    };
    for wrong in [wrong_epoch, wrong_id] {
        let before = assembly.clone();
        assert_eq!(assembly.accept(&wrong), Err(AssemblyError::WrongTransfer));
        assert_eq!(assembly, before);
    }

    // WrongTransfer is checked before OutOfRange and WrongLength.
    let combined = TransferChunk {
        epoch: ep(2),
        index: 9,
        bytes: Vec::new(),
        ..first.clone()
    };
    assert_eq!(
        assembly.accept(&combined),
        Err(AssemblyError::WrongTransfer)
    );

    let before = assembly.clone();
    match assembly.clone().into_bytes() {
        Ok(_) => panic!("incomplete assembly produced bytes"),
        Err(back) => assert_eq!(back, before),
    }
    assert_eq!(assembly.accept(&second), Ok(ChunkOutcome::Complete));
    assert!(assembly.is_complete());
    assert_eq!(assembly.received_chunks(), 2);
    let payload: Vec<u8> = [7u8; 4_097].to_vec();
    assert_eq!(assembly.into_bytes(), Ok(payload));

    assert_eq!(
        TransferAssembly::new(&begin(1, 0, 0, 2, 0, 0, 1)).err(),
        Some(CodecError::Malformed)
    );
    assert_eq!(
        TransferAssembly::new(&begin(1, 0, 0, 1_048_577, 0, 1, 32)).err(),
        Some(CodecError::LimitExceeded)
    );
}

#[test]
fn full_size_byte_assembly() {
    let total = 1_048_576usize;
    let mut payload = vec![0u8; total];
    for (i, byte) in payload.iter_mut().enumerate() {
        *byte = ((i * 31 + 7) % 251) as u8;
    }
    let eight = begin(1, 0, 0, total as u32, 0, 8, 32);
    let mut assembly = TransferAssembly::new(&eight).expect("assembly");
    for index in (0..eight.chunk_count).rev() {
        let frame = chunk_frame(&eight, &payload, index);
        assert!(frame.len() <= MAX_DELTA_FRAME_BYTES);
        let expected = if index == 0 {
            ChunkOutcome::Complete
        } else {
            ChunkOutcome::Stored
        };
        assert_eq!(
            assembly.accept(&chunk_of(&eight, &payload, index)),
            Ok(expected)
        );
    }
    assert_eq!(assembly.received_chunks(), 32);
    assert_eq!(assembly.into_bytes(), Ok(payload));

    let narrow_len = 131_072usize;
    let mut narrow_payload = vec![0u8; narrow_len];
    for (i, byte) in narrow_payload.iter_mut().enumerate() {
        *byte = ((i * 31 + 7) % 251) as u8;
    }
    let one = begin(2, 0, 0, narrow_len as u32, 0, 1, 32);
    let mut narrow = TransferAssembly::new(&one).expect("assembly");
    for index in 0..one.chunk_count {
        assert_eq!(one.chunk_len(index), Some(4_096));
        let expected = if index == 31 {
            ChunkOutcome::Complete
        } else {
            ChunkOutcome::Stored
        };
        assert_eq!(
            narrow.accept(&chunk_of(&one, &narrow_payload, index)),
            Ok(expected)
        );
    }
    assert_eq!(narrow.into_bytes(), Ok(narrow_payload));
}

// ---------------------------------------------------------------- S§10.6

#[test]
fn max_valid_body_publishes_atomically() {
    let mut pool = ReassemblyPool::new(TransferLimits::v1()).expect("pool");
    let id = receiver(&mut pool, EPOCH, DeltaWire::V1, GateMode::Live);
    let body = maximal_body();
    let encoded = encode_body(&body).expect("body");
    assert_eq!(encoded.len(), 22_552);
    let value = begin(
        1,
        0,
        0,
        encoded.len() as u32,
        body.entities.len() as u16,
        1,
        6,
    );

    assert_eq!(
        pool.offer_transfer(id, &begin_frame(&value), 0),
        Ok(TransferOutcome::Begun)
    );
    assert_eq!(
        status(&pool, id)
            .in_flight
            .as_ref()
            .map(|f| f.reserved_bytes),
        Some(22_552)
    );
    for (received, index) in [2u8, 0, 4, 1, 5].into_iter().enumerate() {
        assert_eq!(
            pool.offer_transfer(id, &chunk_frame(&value, &encoded, index), 0),
            Ok(TransferOutcome::Stored {
                received: received as u8 + 1,
                chunk_count: 6,
            })
        );
    }
    let mid = status(&pool, id);
    assert_eq!(mid.in_flight.as_ref().map(|f| f.received), Some(5));
    assert_eq!(mid.queued_updates, 0);
    assert_eq!(pool.staging_bytes(), 22_552);

    assert_eq!(
        pool.offer_transfer(id, &chunk_frame(&value, &encoded, 3), 0),
        Ok(TransferOutcome::Published {
            transfer: tid(1),
            released: 0,
        })
    );
    assert_eq!(pool.staging_bytes(), 0);
    let after = status(&pool, id);
    assert_eq!(after.retired_through, 1);
    assert_eq!(after.in_flight, None);
    assert_eq!(after.mode, GateMode::Live);
    assert_eq!(after.next_event_seq, 1);
    assert_eq!(after.resync, None);
    assert_eq!(
        after.published,
        Some(PublishedBase {
            transfer: tid(1),
            base_tick: 0,
            cursor: 0,
        })
    );
    match pool.pop_update(id) {
        Some(ReplicaUpdate::Snapshot(publication)) => {
            assert_eq!(publication.transfer, tid(1));
            assert_eq!(publication.body, body);
        }
        other => panic!("expected snapshot, got {other:?}"),
    }
    assert_eq!(pool.pop_update(id), None);

    let published = TransferUp::Published {
        epoch: EPOCH,
        transfer: tid(1),
    };
    assert_eq!(
        decode_up(&encode_up(&published).expect("published encodes")),
        Ok(published)
    );

    let wide_id = receiver(&mut pool, ep(1), DeltaWire::V1, GateMode::Live);
    let wide = with_epoch(
        begin(
            2,
            0,
            0,
            encoded.len() as u32,
            body.entities.len() as u16,
            8,
            1,
        ),
        ep(1),
    );
    let mut wide_frames = vec![begin_frame(&wide)];
    wide_frames.extend(frames_of(&wide, &encoded));
    assert_eq!(wide_frames.len(), 2);
    assert_eq!(
        offer_all(&mut pool, wide_id, &wide_frames, 0),
        Ok(TransferOutcome::Published {
            transfer: tid(2),
            released: 0,
        })
    );
}

// ---------------------------------------------------------------- S§10.1/10.3

#[test]
fn second_in_flight_refused() {
    let mut pool = ReassemblyPool::new(TransferLimits::v1()).expect("pool");
    let id = receiver(&mut pool, EPOCH, DeltaWire::V1, GateMode::Live);
    let first = begin(1, 0, 0, 2, 0, 1, 1);
    assert_eq!(
        pool.offer_transfer(id, &begin_frame(&first), 0),
        Ok(TransferOutcome::Begun)
    );
    let second = begin(2, 0, 0, 2, 0, 1, 1);
    let before = pool.clone();
    assert_eq!(
        pool.offer_transfer(id, &begin_frame(&second), 0),
        Err(TransferError::InFlight)
    );
    assert_eq!(pool, before);

    assert_eq!(
        pool.offer_transfer(id, &chunk_frame(&first, &[0, 0], 0), 0),
        Ok(TransferOutcome::Published {
            transfer: tid(1),
            released: 0,
        })
    );
    assert_eq!(
        pool.offer_transfer(id, &begin_frame(&second), 0),
        Ok(TransferOutcome::Begun)
    );
    let before = pool.clone();
    assert_eq!(
        pool.offer_transfer(id, &begin_frame(&first), 0),
        Err(TransferError::StaleTransfer)
    );
    assert_eq!(pool, before);
}

#[test]
fn staging_caps_per_receiver_and_aggregate() {
    let mut full = ReassemblyPool::new(TransferLimits::v1()).expect("pool");
    for index in 0..8u8 {
        let id = receiver(&mut full, ep(index), DeltaWire::V1, GateMode::Live);
        assert_eq!(
            full.offer_transfer(
                id,
                &begin_frame(&with_epoch(begin(1, 0, 0, 1_048_576, 0, 8, 32), ep(index))),
                0
            ),
            Ok(TransferOutcome::Begun)
        );
    }
    assert_eq!(full.staging_bytes(), 8_388_608);
    let before = full.clone();
    assert_eq!(
        full.add_receiver(ep(9), DeltaWire::V1, GateMode::Live),
        Err(TransferError::ReceiversFull)
    );
    assert_eq!(full, before);

    let mut pool = ReassemblyPool::new(limits(22_552, 45_104)).expect("pool");
    let a = receiver(&mut pool, EPOCH, DeltaWire::V1, GateMode::Live);
    let b = receiver(&mut pool, ep(1), DeltaWire::V1, GateMode::Live);
    let c = receiver(&mut pool, ep(2), DeltaWire::V1, GateMode::Live);
    let d = receiver(&mut pool, ep(3), DeltaWire::V1, GateMode::Live);
    let fit = begin(1, 0, 0, 22_552, 0, 1, 6);
    assert_eq!(
        pool.offer_transfer(a, &begin_frame(&fit), 0),
        Ok(TransferOutcome::Begun)
    );
    assert_eq!(
        pool.offer_transfer(b, &begin_frame(&with_epoch(fit.clone(), ep(1))), 0),
        Ok(TransferOutcome::Begun)
    );
    let before = pool.clone();
    let three = with_epoch(begin(3, 0, 0, 22_552, 0, 1, 6), ep(2));
    assert_eq!(
        pool.offer_transfer(c, &begin_frame(&three), 0),
        Err(TransferError::AggregateFull)
    );
    assert_eq!(pool, before);

    let over = with_epoch(begin(4, 0, 0, 22_553, 0, 1, 6), ep(3));
    assert_eq!(
        pool.offer_transfer(d, &begin_frame(&over), 0),
        Err(TransferError::StagingFull)
    );

    pool.remove_receiver(a);
    assert_eq!(pool.staging_bytes(), 22_552);
    assert_eq!(
        pool.offer_transfer(c, &begin_frame(&three), 0),
        Ok(TransferOutcome::Begun)
    );
    assert_eq!(
        pool.offer_transfer(
            c,
            &encode_down(&TransferDown::Cancel {
                epoch: ep(2),
                transfer: tid(3),
                reason: CancelReason::Aborted,
            })
            .expect("cancel"),
            0
        ),
        Ok(TransferOutcome::Cancelled { discarded: true })
    );
    assert_eq!(pool.staging_bytes(), 22_552);
    assert_eq!(
        pool.offer_transfer(d, &begin_frame(&with_epoch(fit.clone(), ep(3))), 0),
        Ok(TransferOutcome::Begun)
    );
    assert_eq!(pool.staging_bytes(), 45_104);
    assert_eq!(pool.poll(5_000), vec![b, d]);
    assert_eq!(pool.staging_bytes(), 0);

    let e = receiver(&mut pool, ep(4), DeltaWire::V1, GateMode::Live);
    assert_eq!(
        pool.offer_transfer(e, &begin_frame(&with_epoch(fit, ep(4))), 5_000),
        Ok(TransferOutcome::Begun)
    );
}

// ---------------------------------------------------------------- S§10.10

fn assert_failed(pool: &ReassemblyPool, id: ReceiverId, retired: u64, reason: ResyncReason) {
    let value = status(pool, id);
    assert_eq!(value.in_flight, None, "in-flight transfer survived failure");
    assert_eq!(value.retired_through, retired, "failed id was not retired");
    assert_eq!(value.resync, Some(reason));
    assert_eq!(value.mode, GateMode::AwaitSnapshot);
    assert_eq!(value.queued_updates, 0);
    assert_eq!(pool.staging_bytes(), 0, "reservation was not freed");
}

#[test]
fn refusals_leave_the_whole_pool_unchanged() {
    let mut pool = ReassemblyPool::new(TransferLimits::v1()).expect("pool");
    let a = receiver(&mut pool, EPOCH, DeltaWire::V1, GateMode::Live);
    let b = receiver(&mut pool, ep(1), DeltaWire::V1, GateMode::Live);
    let value = begin(1, 0, 0, 2, 0, 1, 1);
    assert_eq!(
        pool.offer_transfer(a, &begin_frame(&value), 0),
        Ok(TransferOutcome::Begun)
    );
    let wrong_epoch = with_epoch(begin(2, 0, 0, 2, 0, 1, 1), ep(9));
    let wrong_chunk = TransferChunk {
        epoch: ep(9),
        transfer: tid(1),
        index: 0,
        bytes: vec![0, 0],
    };
    let wrong_cancel = raw_down(TRANSFER_TAG_CANCEL, &{
        let mut fields = vec![9u8; 16];
        fields.extend_from_slice(&[1, 2]);
        fields
    });

    let result = expect_unchanged(&pool, |p| {
        p.offer_transfer(a, &begin_frame(&wrong_epoch), 0)
    });
    assert_eq!(result, Err(TransferError::WrongEpoch));
    let result = expect_unchanged(&pool, |p| {
        p.offer_transfer(
            a,
            &encode_down(&TransferDown::Chunk(wrong_chunk.clone())).expect("chunk"),
            0,
        )
    });
    assert_eq!(result, Err(TransferError::WrongEpoch));
    let result = expect_unchanged(&pool, |p| p.offer_transfer(a, &wrong_cancel, 0));
    assert_eq!(result, Err(TransferError::WrongEpoch));
    let result = expect_unchanged(&pool, |p| {
        p.offer_transfer(a, &begin_frame(&begin(2, 0, 0, 2, 0, 1, 1)), 0)
    });
    assert_eq!(result, Err(TransferError::InFlight));
    let result = expect_unchanged(&pool, |p| {
        p.offer_transfer(a, &chunk_frame(&begin(9, 0, 0, 2, 0, 1, 1), &[0, 0], 0), 0)
    });
    assert_eq!(result, Err(TransferError::UnknownTransfer));
    let result = expect_unchanged(&pool, |p| p.offer_transfer(a, &[TRANSFER_VERSION], 0));
    assert_eq!(result, Err(TransferError::Codec(CodecError::Malformed)));

    // A real cancellation retires an id; the next frame for it is refused.
    assert_eq!(
        pool.offer_transfer(
            b,
            &encode_down(&TransferDown::Cancel {
                epoch: ep(1),
                transfer: tid(5),
                reason: CancelReason::Aborted,
            })
            .expect("cancel"),
            0
        ),
        Ok(TransferOutcome::Cancelled { discarded: false })
    );
    let result = expect_unchanged(&pool, |p| {
        p.offer_transfer(
            b,
            &begin_frame(&with_epoch(begin(5, 0, 0, 2, 0, 1, 1), ep(1))),
            0,
        )
    });
    assert_eq!(result, Err(TransferError::StaleTransfer));

    let removed = receiver(&mut pool, ep(2), DeltaWire::V1, GateMode::Live);
    pool.remove_receiver(removed);
    let result = expect_unchanged(&pool, |p| {
        p.offer_transfer(removed, &begin_frame(&begin(1, 0, 0, 2, 0, 1, 1)), 0)
    });
    assert_eq!(result, Err(TransferError::UnknownReceiver));

    // A refused call with a different epoch leaves the in-flight transfer live.
    assert_eq!(
        pool.offer_transfer(a, &chunk_frame(&value, &[0, 0], 0), 0),
        Ok(TransferOutcome::Published {
            transfer: tid(1),
            released: 0,
        })
    );
}

#[test]
fn failures_discard_retire_and_resync() {
    let body = vec![7u8; 4_097];
    let value = begin(1, 0, 0, 4_097, 0, 1, 2);

    let mut conflict = ReassemblyPool::new(TransferLimits::v1()).expect("pool");
    let id = receiver(&mut conflict, EPOCH, DeltaWire::V1, GateMode::Live);
    assert_eq!(
        conflict.offer_transfer(id, &begin_frame(&value), 0),
        Ok(TransferOutcome::Begun)
    );
    let mut short = chunk_of(&value, &body, 0);
    short.bytes.truncate(4_095);
    assert_eq!(
        conflict.offer_transfer(
            id,
            &encode_down(&TransferDown::Chunk(short)).expect("chunk"),
            0
        ),
        Err(TransferError::Failed(FailReason::Conflict))
    );
    assert_failed(&conflict, id, 1, ResyncReason::Invalid);
    let before = conflict.clone();
    assert_eq!(
        conflict.offer_transfer(id, &chunk_frame(&value, &body, 0), 0),
        Err(TransferError::StaleTransfer)
    );
    assert_eq!(conflict, before);
    assert_eq!(
        conflict.offer_transfer(id, &chunk_frame(&begin(2, 0, 0, 2, 0, 1, 1), &[0, 0], 0), 0),
        Err(TransferError::UnknownTransfer)
    );
    assert_eq!(
        conflict.offer_transfer(id, &begin_frame(&value), 0),
        Err(TransferError::StaleTransfer)
    );
    let higher = begin(2, 0, 0, 2, 0, 1, 1);
    assert_eq!(
        conflict.offer_transfer(id, &begin_frame(&higher), 0),
        Ok(TransferOutcome::Begun)
    );
    assert_eq!(status(&conflict, id).resync, None);

    let mut duplicate_begin = ReassemblyPool::new(TransferLimits::v1()).expect("pool");
    let id = receiver(&mut duplicate_begin, EPOCH, DeltaWire::V1, GateMode::Live);
    assert_eq!(
        duplicate_begin.offer_transfer(id, &begin_frame(&value), 0),
        Ok(TransferOutcome::Begun)
    );
    let other = begin(1, 0, 3, 4_097, 0, 1, 2);
    assert_eq!(
        duplicate_begin.offer_transfer(id, &begin_frame(&other), 0),
        Err(TransferError::Failed(FailReason::Conflict))
    );
    assert_failed(&duplicate_begin, id, 1, ResyncReason::Invalid);

    for bad in [
        TransferChunk {
            index: 2,
            bytes: vec![0],
            ..chunk_of(&value, &body, 0)
        },
        TransferChunk {
            bytes: vec![1u8; 4_095],
            ..chunk_of(&value, &body, 0)
        },
    ] {
        let mut pool = ReassemblyPool::new(TransferLimits::v1()).expect("pool");
        let id = receiver(&mut pool, EPOCH, DeltaWire::V1, GateMode::Live);
        assert_eq!(
            pool.offer_transfer(id, &begin_frame(&value), 0),
            Ok(TransferOutcome::Begun)
        );
        assert_eq!(
            pool.offer_transfer(
                id,
                &encode_down(&TransferDown::Chunk(bad)).expect("chunk"),
                0
            ),
            Err(TransferError::Failed(FailReason::Conflict))
        );
        assert_failed(&pool, id, 1, ResyncReason::Invalid);
    }

    // A stored chunk followed by different bytes at the same index conflicts.
    let mut different = ReassemblyPool::new(TransferLimits::v1()).expect("pool");
    let id = receiver(&mut different, EPOCH, DeltaWire::V1, GateMode::Live);
    assert_eq!(
        different.offer_transfer(id, &begin_frame(&value), 0),
        Ok(TransferOutcome::Begun)
    );
    assert_eq!(
        different.offer_transfer(id, &chunk_frame(&value, &body, 0), 0),
        Ok(TransferOutcome::Stored {
            received: 1,
            chunk_count: 2,
        })
    );
    let mut changed = chunk_of(&value, &body, 0);
    changed.bytes[0] ^= 0xFF;
    assert_eq!(
        different.offer_transfer(
            id,
            &encode_down(&TransferDown::Chunk(changed)).expect("chunk"),
            0
        ),
        Err(TransferError::Failed(FailReason::Conflict))
    );
    assert_failed(&different, id, 1, ResyncReason::Invalid);
}

// ---------------------------------------------------------------- S§10.6/10.4

#[test]
fn invalid_body_never_publishes() {
    let mut pool = ReassemblyPool::new(TransferLimits::v1()).expect("pool");
    let id = receiver(&mut pool, EPOCH, DeltaWire::V1, GateMode::Live);
    let good = body_of(&[(1, Some((500, 1_000, false)))], None);
    assert_eq!(
        publish(&mut pool, id, EPOCH, 1, 0, 5, 1, &good, 0),
        Ok(TransferOutcome::Published {
            transfer: tid(1),
            released: 0,
        })
    );

    let bad_count = begin(2, 0, 5, 7, 0, 1, 1);
    assert_eq!(
        pool.offer_transfer(id, &begin_frame(&bad_count), 0),
        Ok(TransferOutcome::Begun)
    );
    assert_eq!(
        pool.offer_transfer(
            id,
            &encode_down(&TransferDown::Chunk(TransferChunk {
                epoch: EPOCH,
                transfer: tid(2),
                index: 0,
                bytes: vec![0x01, 0x01, 0x01, 0x0A, 0x0A, 0x00, 0x00],
            }))
            .expect("chunk"),
            0
        ),
        Err(TransferError::Failed(FailReason::InvalidBody(
            CodecError::Malformed
        )))
    );
    let after = status(&pool, id);
    assert_eq!(after.in_flight, None);
    assert_eq!(after.retired_through, 2);
    assert_eq!(after.resync, Some(ResyncReason::Invalid));
    assert_eq!(after.mode, GateMode::AwaitSnapshot);
    assert_eq!(after.next_event_seq, 6);
    assert_eq!(pool.staging_bytes(), 0);
    match pool.pop_update(id) {
        Some(ReplicaUpdate::Snapshot(publication)) => {
            assert_eq!(publication.transfer, tid(1));
            assert_eq!(publication.body, good);
        }
        other => panic!("expected the earlier snapshot, got {other:?}"),
    }
    assert_eq!(pool.pop_update(id), None);

    let mut trailing = ReassemblyPool::new(TransferLimits::v1()).expect("pool");
    let id = receiver(&mut trailing, EPOCH, DeltaWire::V1, GateMode::Live);
    let value = begin(1, 0, 0, 3, 0, 1, 1);
    assert_eq!(
        trailing.offer_transfer(id, &begin_frame(&value), 0),
        Ok(TransferOutcome::Begun)
    );
    assert_eq!(
        trailing.offer_transfer(
            id,
            &encode_down(&TransferDown::Chunk(TransferChunk {
                epoch: EPOCH,
                transfer: tid(1),
                index: 0,
                bytes: vec![0x00, 0x00, 0x2A],
            }))
            .expect("chunk"),
            0
        ),
        Err(TransferError::Failed(FailReason::InvalidBody(
            CodecError::Malformed
        )))
    );
    assert_failed(&trailing, id, 1, ResyncReason::Invalid);

    let mut opaque = ReassemblyPool::new(TransferLimits::v1()).expect("pool");
    let id = receiver(&mut opaque, EPOCH, DeltaWire::V1, GateMode::Live);
    let value = begin(1, 0, 0, 1_048_576, 0, 8, 32);
    let mut frames = vec![begin_frame(&value)];
    frames.extend(frames_of(&value, &vec![0u8; 1_048_576]));
    assert_eq!(
        offer_all(&mut opaque, id, &frames, 0),
        Err(TransferError::Failed(FailReason::InvalidBody(
            CodecError::Malformed
        )))
    );
    assert_failed(&opaque, id, 1, ResyncReason::Invalid);

    let mut counted = ReassemblyPool::new(TransferLimits::v1()).expect("pool");
    let id = receiver(&mut counted, EPOCH, DeltaWire::V1, GateMode::Live);
    let maximal = encode_body(&maximal_body()).expect("body");
    let value = begin(1, 0, 0, maximal.len() as u32, 1_023, 8, 1);
    assert_eq!(
        counted.offer_transfer(id, &begin_frame(&value), 0),
        Ok(TransferOutcome::Begun)
    );
    assert_eq!(
        counted.offer_transfer(id, &chunk_frame(&value, &maximal, 0), 0),
        Err(TransferError::Failed(FailReason::InvalidBody(
            CodecError::Malformed
        )))
    );
}

#[test]
fn wrong_epoch_and_behind_replica_refusals() {
    let mut pool = ReassemblyPool::new(TransferLimits::v1()).expect("pool");
    let id = receiver(&mut pool, EPOCH, DeltaWire::V1, GateMode::Live);
    let first = body_of(&[(1, Some((100, 1_000, false)))], None);
    assert_eq!(
        publish(&mut pool, id, EPOCH, 1, 100, 5, 1, &first, 0),
        Ok(TransferOutcome::Published {
            transfer: tid(1),
            released: 0,
        })
    );
    assert!(pool.pop_update(id).is_some());

    let behind_cursor = begin(2, 100, 3, 2, 0, 1, 1);
    let result = expect_unchanged(&pool, |p| {
        p.offer_transfer(id, &begin_frame(&behind_cursor), 0)
    });
    assert_eq!(result, Err(TransferError::BehindReplica));
    let behind_base = begin(2, 99, 5, 2, 0, 1, 1);
    let result = expect_unchanged(&pool, |p| {
        p.offer_transfer(id, &begin_frame(&behind_base), 0)
    });
    assert_eq!(result, Err(TransferError::BehindReplica));

    let accepted = begin(2, 100, 5, 2, 0, 1, 1);
    assert_eq!(
        pool.offer_transfer(id, &begin_frame(&accepted), 0),
        Ok(TransferOutcome::Begun)
    );

    let wrong_begin = with_epoch(begin(3, 100, 5, 2, 0, 1, 1), ep(9));
    let result = expect_unchanged(&pool, |p| {
        p.offer_transfer(id, &begin_frame(&wrong_begin), 0)
    });
    assert_eq!(result, Err(TransferError::WrongEpoch));
    let wrong_chunk = TransferChunk {
        epoch: ep(9),
        transfer: tid(2),
        index: 0,
        bytes: vec![0, 0],
    };
    let result = expect_unchanged(&pool, |p| {
        p.offer_transfer(
            id,
            &encode_down(&TransferDown::Chunk(wrong_chunk.clone())).expect("chunk"),
            0,
        )
    });
    assert_eq!(result, Err(TransferError::WrongEpoch));
    let wrong_cancel = raw_down(TRANSFER_TAG_CANCEL, &{
        let mut fields = vec![9u8; 16];
        fields.extend_from_slice(&[2, 1]);
        fields
    });
    let result = expect_unchanged(&pool, |p| p.offer_transfer(id, &wrong_cancel, 0));
    assert_eq!(result, Err(TransferError::WrongEpoch));

    assert_eq!(
        pool.offer_transfer(id, &chunk_frame(&accepted, &[0, 0], 0), 0),
        Ok(TransferOutcome::Published {
            transfer: tid(2),
            released: 0,
        })
    );
}

#[test]
fn queued_snapshot_is_replaced_by_next_publication() {
    let mut pool = ReassemblyPool::new(TransferLimits::v1()).expect("pool");
    let id = receiver(&mut pool, EPOCH, DeltaWire::V1, GateMode::Live);
    let first = body_of(&[(1, Some((1, 10, false)))], None);
    let second = body_of(&[(1, Some((2, 10, false)))], None);
    assert_eq!(
        publish(&mut pool, id, EPOCH, 1, 0, 5, 1, &first, 0),
        Ok(TransferOutcome::Published {
            transfer: tid(1),
            released: 0,
        })
    );
    assert_eq!(
        publish(&mut pool, id, EPOCH, 2, 0, 6, 1, &second, 0),
        Ok(TransferOutcome::Published {
            transfer: tid(2),
            released: 0,
        })
    );
    match pool.pop_update(id) {
        Some(ReplicaUpdate::Snapshot(publication)) => {
            assert_eq!(publication.transfer, tid(2));
            assert_eq!(publication.body, second);
        }
        other => panic!("expected the newer snapshot, got {other:?}"),
    }
    assert_eq!(pool.pop_update(id), None);
}

// ---------------------------------------------------------------- S§10.3/10.5

#[test]
fn cancel_retires_and_supersedes() {
    let mut pool = ReassemblyPool::new(TransferLimits::v1()).expect("pool");
    let id = receiver(&mut pool, EPOCH, DeltaWire::V1, GateMode::Live);
    let value = begin(1, 0, 0, 4_097, 0, 1, 2);
    assert_eq!(
        pool.offer_transfer(id, &begin_frame(&value), 0),
        Ok(TransferOutcome::Begun)
    );
    let cancel = |transfer: u64| {
        encode_down(&TransferDown::Cancel {
            epoch: EPOCH,
            transfer: tid(transfer),
            reason: CancelReason::Superseded,
        })
        .expect("cancel")
    };
    assert_eq!(
        pool.offer_transfer(id, &cancel(1), 0),
        Ok(TransferOutcome::Cancelled { discarded: true })
    );
    let after = status(&pool, id);
    assert_eq!(after.in_flight, None);
    assert_eq!(after.mode, GateMode::AwaitSnapshot);
    assert_eq!(after.resync, None);
    assert_eq!(after.retired_through, 1);
    assert_eq!(pool.staging_bytes(), 0);
    let before = pool.clone();
    assert_eq!(
        pool.offer_transfer(id, &chunk_frame(&value, &[0u8; 4_097], 0), 0),
        Err(TransferError::StaleTransfer)
    );
    assert_eq!(pool, before);
    assert_eq!(
        pool.offer_transfer(id, &cancel(1), 0),
        Ok(TransferOutcome::Cancelled { discarded: false })
    );

    assert_eq!(
        pool.offer_transfer(id, &cancel(9), 0),
        Ok(TransferOutcome::Cancelled { discarded: false })
    );
    assert_eq!(status(&pool, id).retired_through, 9);
    let stale = begin(9, 0, 0, 2, 0, 1, 1);
    let result = expect_unchanged(&pool, |p| p.offer_transfer(id, &begin_frame(&stale), 0));
    assert_eq!(result, Err(TransferError::StaleTransfer));

    let ten = begin(10, 0, 0, 2, 0, 1, 1);
    assert_eq!(
        pool.offer_transfer(id, &begin_frame(&ten), 0),
        Ok(TransferOutcome::Begun)
    );
    assert_eq!(
        pool.offer_transfer(id, &chunk_frame(&ten, &[0, 0], 0), 0),
        Ok(TransferOutcome::Published {
            transfer: tid(10),
            released: 0,
        })
    );
    assert_eq!(
        pool.offer_transfer(id, &cancel(10), 0),
        Ok(TransferOutcome::Cancelled { discarded: false })
    );
    match pool.pop_update(id) {
        Some(ReplicaUpdate::Snapshot(publication)) => {
            assert_eq!(publication.transfer, tid(10));
        }
        other => panic!("published snapshot was retracted: {other:?}"),
    }
}

#[test]
fn expiry_sweep_frees_and_retry_publishes() {
    let mut pool = ReassemblyPool::new(limits(22_552, 22_552)).expect("pool");
    let a = receiver(&mut pool, EPOCH, DeltaWire::V1, GateMode::Live);
    let b = receiver(&mut pool, ep(1), DeltaWire::V1, GateMode::AwaitSnapshot);
    let fit = begin(1, 0, 0, 22_552, 0, 1, 6);
    assert_eq!(
        pool.offer_transfer(a, &begin_frame(&fit), 0),
        Ok(TransferOutcome::Begun)
    );
    assert_eq!(pool.staging_bytes(), 22_552);
    assert_eq!(
        pool.offer_delta(b, &v1_delta(1, vec![]), 5_000),
        Ok(DeltaOutcome::Buffered)
    );
    let expired = status(&pool, a);
    assert_eq!(expired.in_flight, None);
    assert_eq!(expired.resync, Some(ResyncReason::Expired));
    assert_eq!(expired.retired_through, 1);
    assert_eq!(pool.staging_bytes(), 0);

    let resync = TransferUp::Resync {
        epoch: EPOCH,
        after: Some(tid(1)),
        reason: ResyncReason::Expired,
    };
    assert_eq!(
        decode_up(&encode_up(&resync).expect("resync encodes")),
        Ok(resync)
    );

    assert_eq!(
        publish(&mut pool, a, EPOCH, 2, 0, 0, 8, &maximal_body(), 5_000),
        Ok(TransferOutcome::Published {
            transfer: tid(2),
            released: 0,
        })
    );

    let mut ordered = ReassemblyPool::new(limits(22_552, 45_104)).expect("pool");
    let first = receiver(&mut ordered, EPOCH, DeltaWire::V1, GateMode::Live);
    let second = receiver(&mut ordered, ep(1), DeltaWire::V1, GateMode::Live);
    let fit = begin(1, 0, 0, 22_552, 0, 1, 6);
    assert_eq!(
        ordered.offer_transfer(first, &begin_frame(&fit), 0),
        Ok(TransferOutcome::Begun)
    );
    assert_eq!(
        ordered.offer_transfer(second, &begin_frame(&with_epoch(fit, ep(1))), 0),
        Ok(TransferOutcome::Begun)
    );
    assert_eq!(ordered.poll(5_000), vec![first, second]);
    assert_eq!(ordered.staging_bytes(), 0);
}

// ---------------------------------------------------------------- S§10.7

#[test]
fn cutover_releases_in_fifo_order() {
    let mut pool = ReassemblyPool::new(TransferLimits::v1()).expect("pool");
    let id = receiver(&mut pool, EPOCH, DeltaWire::V2, GateMode::Live);
    for seq in 1..=3u64 {
        assert_eq!(
            pool.offer_delta(
                id,
                &v2_delta(seq, vec![OpV2::Legacy(health_op(seq, seq as u32))]),
                0
            ),
            Ok(DeltaOutcome::Released { count: 1 })
        );
    }
    let snapshot = body_of(
        &[
            (1, Some((501, 1_000, false))),
            (2, Some((502, 1_000, false))),
            (3, Some((503, 1_000, false))),
            (4, Some((504, 1_000, false))),
            (5, Some((505, 1_000, false))),
        ],
        None,
    );
    let frames = plan(EPOCH, 1, 0, 5, 1, &snapshot).expect("plan");
    assert_eq!(
        pool.offer_transfer(id, &frames[0], 0),
        Ok(TransferOutcome::Begun)
    );
    let health = |seq: u64| v2_delta(seq, vec![OpV2::Legacy(health_op(seq, seq as u32))]);
    assert_eq!(
        pool.offer_delta(id, &health(5), 0),
        Ok(DeltaOutcome::Buffered)
    );
    assert_eq!(
        pool.offer_delta(id, &health(4), 0),
        Ok(DeltaOutcome::Released { count: 2 })
    );
    assert_eq!(
        pool.offer_delta(id, &health(6), 0),
        Ok(DeltaOutcome::Buffered)
    );
    assert_eq!(
        pool.offer_delta(id, &health(7), 0),
        Ok(DeltaOutcome::Buffered)
    );

    let mut pre = Vec::new();
    for _ in 0..5 {
        pre.push(pool.pop_update(id).expect("queued delta"));
    }
    assert_eq!(
        pool.pop_update(id),
        None,
        "delta above cursor released early"
    );
    for (index, update) in pre.iter().enumerate() {
        match update {
            ReplicaUpdate::Delta(delta) => assert_eq!(delta.event_seq(), index as u64 + 1),
            other => panic!("expected delta, got {other:?}"),
        }
    }

    assert_eq!(
        pool.offer_transfer(id, &frames[frames.len() - 1], 0),
        Ok(TransferOutcome::Published {
            transfer: tid(1),
            released: 2,
        })
    );
    let mut post = Vec::new();
    for _ in 0..3 {
        post.push(pool.pop_update(id).expect("post-cutover update"));
    }
    match &post[0] {
        ReplicaUpdate::Snapshot(publication) => assert_eq!(publication.body, snapshot),
        other => panic!("expected snapshot, got {other:?}"),
    }
    match &post[1] {
        ReplicaUpdate::Delta(delta) => assert_eq!(delta.event_seq(), 6),
        other => panic!("expected delta 6, got {other:?}"),
    }
    match &post[2] {
        ReplicaUpdate::Delta(delta) => assert_eq!(delta.event_seq(), 7),
        other => panic!("expected delta 7, got {other:?}"),
    }
    assert_eq!(pool.pop_update(id), None);

    let mut all = pre;
    all.extend(post);
    let mut oracle = Oracle::default();
    for update in all.clone() {
        oracle.apply(update).expect("ordered application");
    }
    let mut expected = BTreeMap::new();
    for entity in 1..=5u64 {
        expected.insert(entity, 500 + entity as u32);
    }
    expected.insert(6, 6);
    expected.insert(7, 7);
    assert!(oracle.matches(&expected), "pop order produced {oracle:?}");

    let mut out_of_order = Oracle::default();
    for update in &all[..5] {
        out_of_order.apply(update.clone()).expect("old base");
    }
    out_of_order.apply(all[5].clone()).expect("snapshot");
    assert!(
        out_of_order.apply(all[4].clone()).is_err(),
        "oracle must reject a delta at or below the snapshot cursor"
    );
    let mut dropped = Oracle::default();
    for (index, update) in all.clone().into_iter().enumerate() {
        if index != 6 {
            dropped.apply(update).expect("without delta 6");
        }
    }
    assert!(
        !dropped.matches(&expected),
        "oracle must notice the missing delta 6"
    );

    assert_eq!(
        pool.offer_delta(id, &health(5), 0),
        Ok(DeltaOutcome::Superseded)
    );
    assert_eq!(
        pool.offer_delta(id, &health(6), 0),
        Ok(DeltaOutcome::Superseded)
    );
}

#[test]
fn await_snapshot_holds_until_publication() {
    let mut pool = ReassemblyPool::new(TransferLimits::v1()).expect("pool");
    let id = receiver(&mut pool, EPOCH, DeltaWire::V1, GateMode::AwaitSnapshot);
    for seq in 1..=3u64 {
        assert_eq!(
            pool.offer_delta(id, &v1_delta(seq, vec![health_op(seq, seq as u32)]), 0),
            Ok(DeltaOutcome::Buffered)
        );
    }
    assert_eq!(pool.pop_update(id), None);
    assert_eq!(status(&pool, id).held_frames, 3);

    let snapshot = body_of(
        &[
            (1, Some((101, 1_000, false))),
            (2, Some((102, 1_000, false))),
        ],
        None,
    );
    assert_eq!(
        publish(&mut pool, id, EPOCH, 1, 0, 2, 1, &snapshot, 0),
        Ok(TransferOutcome::Published {
            transfer: tid(1),
            released: 1,
        })
    );
    match pool.pop_update(id) {
        Some(ReplicaUpdate::Snapshot(publication)) => assert_eq!(publication.body, snapshot),
        other => panic!("expected snapshot, got {other:?}"),
    }
    match pool.pop_update(id) {
        Some(ReplicaUpdate::Delta(delta)) => assert_eq!(delta.event_seq(), 3),
        other => panic!("expected delta 3, got {other:?}"),
    }

    assert_eq!(
        pool.offer_delta(id, &v1_delta(5, vec![health_op(5, 5)]), 0),
        Ok(DeltaOutcome::Buffered)
    );
    assert_eq!(
        pool.offer_delta(id, &v1_delta(4, vec![health_op(4, 4)]), 0),
        Ok(DeltaOutcome::Released { count: 2 })
    );
    match pool.pop_update(id) {
        Some(ReplicaUpdate::Delta(delta)) => assert_eq!(delta.event_seq(), 4),
        other => panic!("expected delta 4, got {other:?}"),
    }
    match pool.pop_update(id) {
        Some(ReplicaUpdate::Delta(delta)) => assert_eq!(delta.event_seq(), 5),
        other => panic!("expected delta 5, got {other:?}"),
    }
    assert_eq!(pool.pop_update(id), None);
}

// ---------------------------------------------------------------- S§10.8

#[test]
fn delta_hold_caps_and_overflow() {
    let mut pool = ReassemblyPool::new(TransferLimits::v1()).expect("pool");
    let id = receiver(&mut pool, EPOCH, DeltaWire::V1, GateMode::Live);
    let value = begin(1, 0, 0, 2, 0, 1, 1);
    assert_eq!(
        pool.offer_transfer(id, &begin_frame(&value), 0),
        Ok(TransferOutcome::Begun)
    );
    for seq in 1..=128u64 {
        assert_eq!(
            pool.offer_delta(id, &v1_delta(seq, vec![]), 0),
            Ok(DeltaOutcome::Buffered)
        );
    }
    assert_eq!(status(&pool, id).held_frames, 128);
    assert_eq!(
        pool.offer_delta(id, &v1_delta(129, vec![]), 0),
        Err(TransferError::Failed(FailReason::Overflow))
    );
    let after = status(&pool, id);
    assert_eq!(after.in_flight, None);
    assert_eq!(after.retired_through, 1);
    assert_eq!(after.resync, Some(ResyncReason::Overflow));
    assert_eq!(after.mode, GateMode::AwaitSnapshot);
    assert_eq!(after.held_frames, 0);
    assert_eq!(pool.staging_bytes(), 0);
    assert_eq!(pool.pop_update(id), None);

    let mut conflict = ReassemblyPool::new(TransferLimits::v1()).expect("pool");
    let id = receiver(&mut conflict, EPOCH, DeltaWire::V1, GateMode::AwaitSnapshot);
    assert_eq!(
        conflict.offer_delta(id, &v1_delta(5, vec![health_op(1, 50)]), 0),
        Ok(DeltaOutcome::Buffered)
    );
    assert_eq!(
        conflict.offer_delta(id, &v1_delta(5, vec![health_op(1, 60)]), 0),
        Err(TransferError::Failed(FailReason::Conflict))
    );
    let after = status(&conflict, id);
    assert_eq!(after.resync, Some(ResyncReason::Invalid));
    assert_eq!(
        after.held_frames, 1,
        "only Overflow drops held deltas (S§10.4 step 4)"
    );
}

#[test]
fn delta_byte_and_pool_caps_overflow() {
    let mut exact = ReassemblyPool::new(delta_limits(65_536, 16_777_216)).expect("pool");
    let id = receiver(&mut exact, EPOCH, DeltaWire::V1, GateMode::AwaitSnapshot);
    for seq in 1..=128u64 {
        assert_eq!(
            exact.offer_delta(id, &v1_delta_of_len(seq, 512), 0),
            Ok(DeltaOutcome::Buffered)
        );
    }
    assert_eq!(status(&exact, id).held_bytes, 65_536);
    assert_eq!(status(&exact, id).held_frames, 128);

    let mut over = ReassemblyPool::new(delta_limits(65_536, 16_777_216)).expect("pool");
    let id = receiver(&mut over, EPOCH, DeltaWire::V1, GateMode::AwaitSnapshot);
    for seq in 1..=127u64 {
        assert_eq!(
            over.offer_delta(id, &v1_delta_of_len(seq, 512), 0),
            Ok(DeltaOutcome::Buffered)
        );
    }
    assert_eq!(
        over.offer_delta(id, &v1_delta_of_len(128, 513), 0),
        Err(TransferError::Failed(FailReason::Overflow))
    );
    assert_eq!(status(&over, id).held_frames, 0);
    assert_eq!(over.pop_update(id), None);

    let mut total = ReassemblyPool::new(delta_limits(65_536, 131_072)).expect("pool");
    let a = receiver(&mut total, EPOCH, DeltaWire::V1, GateMode::AwaitSnapshot);
    let b = receiver(&mut total, ep(1), DeltaWire::V1, GateMode::AwaitSnapshot);
    let c = receiver(&mut total, ep(2), DeltaWire::V1, GateMode::AwaitSnapshot);
    for seq in 1..=128u64 {
        assert_eq!(
            total.offer_delta(a, &v1_delta_of_len(seq, 512), 0),
            Ok(DeltaOutcome::Buffered)
        );
        assert_eq!(
            total.offer_delta(b, &v1_delta_of_len(seq, 512), 0),
            Ok(DeltaOutcome::Buffered)
        );
    }
    assert_eq!(total.held_delta_bytes(), 131_072);
    assert_eq!(
        total.offer_delta(c, &v1_delta_of_len(1, 512), 0),
        Err(TransferError::Failed(FailReason::Overflow))
    );
    assert_eq!(status(&total, a).held_bytes, 65_536);
    assert_eq!(status(&total, b).held_bytes, 65_536);
}

#[test]
fn delta_sequence_and_wire_refusals() {
    let mut pool = ReassemblyPool::new(TransferLimits::v1()).expect("pool");
    let v1 = receiver(&mut pool, EPOCH, DeltaWire::V1, GateMode::Live);
    let v2 = receiver(&mut pool, ep(1), DeltaWire::V2, GateMode::Live);

    let zero = vec![PROTOCOL_VERSION, LANE_COMBAT, 0, 0, 0];
    let result = expect_unchanged(&pool, |p| p.offer_delta(v1, &zero, 0));
    assert_eq!(result, Err(TransferError::DeltaSequence));
    let mut last = vec![PROTOCOL_VERSION, LANE_COMBAT, 0];
    last.extend(varint(u64::MAX));
    last.push(0);
    let result = expect_unchanged(&pool, |p| p.offer_delta(v1, &last, 0));
    assert_eq!(result, Err(TransferError::DeltaSequence));

    assert_eq!(
        pool.offer_delta(v1, &v1_delta(SEQ_LAST, vec![]), 0),
        Ok(DeltaOutcome::Buffered)
    );

    let v1_bytes = v1_delta(1, vec![health_op(1, 1)]);
    let v2_bytes = v2_delta(1, vec![OpV2::Legacy(health_op(1, 1))]);
    let result = expect_unchanged(&pool, |p| p.offer_delta(v2, &v1_bytes, 0));
    assert_eq!(
        result,
        Err(TransferError::Codec(CodecError::UnsupportedVersion))
    );
    let result = expect_unchanged(&pool, |p| p.offer_delta(v1, &v2_bytes, 0));
    assert_eq!(
        result,
        Err(TransferError::Codec(CodecError::UnsupportedVersion))
    );
    let result = expect_unchanged(&pool, |p| p.offer_delta(v1, &[], 0));
    assert_eq!(result, Err(TransferError::Codec(CodecError::Malformed)));

    pool.remove_receiver(v2);
    let result = expect_unchanged(&pool, |p| p.offer_delta(v2, &v2_bytes, 0));
    assert_eq!(result, Err(TransferError::UnknownReceiver));
}

// ---------------------------------------------------------------- S§10.9

#[test]
fn revocation_never_republishes_hidden_facts() {
    let wide = maximal_body();
    let narrow = narrowed(&wide);
    let hidden = wide.entities[0].entity.get();
    assert_eq!(narrow.entities[0].health, None);
    assert_eq!(&narrow.entities[1..], &wide.entities[1..]);

    let mut pool = ReassemblyPool::new(TransferLimits::v1()).expect("pool");
    let id = receiver(&mut pool, EPOCH, DeltaWire::V1, GateMode::Live);
    let frames = plan(EPOCH, 1, 0, 0, 1, &wide).expect("plan");
    assert_eq!(frames.len(), 7);
    assert_eq!(
        pool.offer_transfer(id, &frames[0], 0),
        Ok(TransferOutcome::Begun)
    );
    for index in 0..5usize {
        assert_eq!(
            pool.offer_transfer(id, &frames[index + 1], 0),
            Ok(TransferOutcome::Stored {
                received: index as u8 + 1,
                chunk_count: 6,
            })
        );
    }
    assert_eq!(
        pool.offer_transfer(
            id,
            &encode_down(&TransferDown::Cancel {
                epoch: EPOCH,
                transfer: tid(1),
                reason: CancelReason::Aborted,
            })
            .expect("cancel"),
            0
        ),
        Ok(TransferOutcome::Cancelled { discarded: true })
    );
    assert_eq!(pool.staging_bytes(), 0);
    for index in 0..6usize {
        let result = expect_unchanged(&pool, |p| p.offer_transfer(id, &frames[index + 1], 0));
        assert_eq!(result, Err(TransferError::StaleTransfer));
    }

    assert_eq!(
        publish(&mut pool, id, EPOCH, 2, 0, 0, 1, &narrow, 0),
        Ok(TransferOutcome::Published {
            transfer: tid(2),
            released: 0,
        })
    );
    match pool.pop_update(id) {
        Some(ReplicaUpdate::Snapshot(publication)) => {
            assert_eq!(publication.body, narrow);
            assert_eq!(publication.body.entities[0].health, None);
            assert_eq!(
                publication
                    .body
                    .entities
                    .iter()
                    .find(|entity| entity.entity.get() == hidden),
                Some(&SnapshotEntity {
                    entity: net(hidden),
                    health: None,
                })
            );
        }
        other => panic!("expected narrowed snapshot, got {other:?}"),
    }

    let mut joined = Vec::new();
    for frame in &frames[1..] {
        match decode_down(frame).expect("chunk") {
            TransferDown::Chunk(chunk) => joined.extend_from_slice(&chunk.bytes),
            other => panic!("expected chunk, got {other:?}"),
        }
    }
    let disclosed = decode_body(&joined).expect("body");
    assert_eq!(
        disclosed.entities[0].health,
        Some(HealthFact {
            health: u32::MAX,
            max_health: u32::MAX,
            dead: true,
        })
    );

    let mut leak = ReassemblyPool::new(TransferLimits::v1()).expect("pool");
    let id = receiver(&mut leak, EPOCH, DeltaWire::V1, GateMode::Live);
    assert_eq!(
        publish(&mut leak, id, EPOCH, 1, 0, 0, 1, &wide, 0),
        Ok(TransferOutcome::Published {
            transfer: tid(1),
            released: 0,
        })
    );
    let leaked = match leak.pop_update(id) {
        Some(ReplicaUpdate::Snapshot(publication)) => publication.body,
        other => panic!("expected snapshot, got {other:?}"),
    };
    assert!(
        leaked.entities[0].health.is_some(),
        "without the cancel the hidden fact is disclosed"
    );
    assert_ne!(leaked, narrow);
}

#[test]
fn hidden_values_never_reach_bytes() {
    let hidden = body_of(&[(7, None)], None);
    assert_eq!(encode_body(&hidden), Ok(vec![0x01, 0x07, 0x00, 0x00]));

    let first = Fixture {
        entities: vec![(1, (10, 10, false)), (2, (90, 90, true))],
        turn: (1, 3),
    };
    let second = Fixture {
        entities: vec![(1, (20, 20, true)), (2, (1, 1, false))],
        turn: (2, 99),
    };
    let a = build(&first, 1, false, false);
    let b = build(&second, 1, false, false);
    assert_eq!(a, b, "hidden values changed the body");
    assert_eq!(
        encode_body(&a),
        encode_body(&b),
        "hidden values reached the bytes"
    );
    assert_eq!(
        plan(EPOCH, 1, 0, 0, 1, &a),
        plan(EPOCH, 1, 0, 0, 1, &b),
        "hidden values reached the plan"
    );

    let granted = build(&first, 1, true, false);
    let hidden_granted = build(&second, 1, true, false);
    assert_ne!(encode_body(&granted), encode_body(&hidden_granted));
    assert_ne!(a, granted, "a granted change must change the bytes");

    assert_eq!(a.turn, None);
    assert_eq!(
        build(&first, 1, false, true).turn.and_then(|t| t.active),
        Some(net(1))
    );
    assert_eq!(
        build(&first, 2, false, true).turn.and_then(|t| t.active),
        None,
        "active cannot name a hidden entity"
    );
}

struct Fixture {
    entities: Vec<(u64, (u32, u32, bool))>,
    turn: (u64, u64),
}

fn build(fixture: &Fixture, visible: u64, health_grant: bool, turn_grant: bool) -> SnapshotBody {
    let entities = fixture
        .entities
        .iter()
        .filter(|(id, _)| *id == visible)
        .map(|(id, (health, max_health, dead))| SnapshotEntity {
            entity: net(*id),
            health: health_grant.then_some(HealthFact {
                health: *health,
                max_health: *max_health,
                dead: *dead,
            }),
        })
        .collect();
    let turn = turn_grant.then_some(TurnFact {
        active: (fixture.turn.0 == visible).then_some(net(visible)),
        round: fixture.turn.1,
    });
    SnapshotBody { entities, turn }
}

#[test]
fn duplicates_idempotent_no_deadline_extension() {
    let mut pool = ReassemblyPool::new(TransferLimits::v1()).expect("pool");
    let id = receiver(&mut pool, EPOCH, DeltaWire::V1, GateMode::Live);
    let value = begin(1, 0, 0, 4_097, 0, 1, 2);
    assert_eq!(
        pool.offer_transfer(id, &begin_frame(&value), 10_000),
        Ok(TransferOutcome::Begun)
    );
    assert_eq!(
        status(&pool, id).in_flight.map(|f| f.deadline_ms),
        Some(15_000)
    );
    assert_eq!(
        pool.offer_transfer(id, &begin_frame(&value), 14_000),
        Ok(TransferOutcome::Duplicate)
    );
    assert_eq!(
        status(&pool, id).in_flight.map(|f| f.deadline_ms),
        Some(15_000)
    );
    let body = vec![7u8; 4_097];
    assert_eq!(
        pool.offer_transfer(id, &chunk_frame(&value, &body, 0), 14_000),
        Ok(TransferOutcome::Stored {
            received: 1,
            chunk_count: 2,
        })
    );
    assert_eq!(
        pool.offer_transfer(id, &chunk_frame(&value, &body, 0), 14_000),
        Ok(TransferOutcome::Duplicate)
    );
    assert_eq!(
        status(&pool, id).in_flight.map(|f| f.deadline_ms),
        Some(15_000)
    );
    assert_eq!(
        pool.offer_transfer(id, &chunk_frame(&value, &body, 1), 15_000),
        Err(TransferError::Failed(FailReason::Expired))
    );
    let after = status(&pool, id);
    assert_eq!(after.in_flight, None);
    assert_eq!(after.resync, Some(ResyncReason::Expired));
    assert_eq!(after.queued_updates, 0);
    assert_eq!(pool.staging_bytes(), 0);
}

#[test]
fn deadline_boundary_4999_5000() {
    let body = two_chunk_body();
    let value = begin(1, 0, 0, body.len() as u32, 800, 1, 2);

    let mut live = ReassemblyPool::new(TransferLimits::v1()).expect("pool");
    let id = receiver(&mut live, EPOCH, DeltaWire::V1, GateMode::Live);
    assert_eq!(
        live.offer_transfer(id, &begin_frame(&value), 10_000),
        Ok(TransferOutcome::Begun)
    );
    assert_eq!(
        live.offer_transfer(id, &chunk_frame(&value, &body, 0), 10_001),
        Ok(TransferOutcome::Stored {
            received: 1,
            chunk_count: 2,
        })
    );
    assert_eq!(
        live.offer_transfer(id, &chunk_frame(&value, &body, 1), 14_999),
        Ok(TransferOutcome::Published {
            transfer: tid(1),
            released: 0,
        })
    );

    let mut late = ReassemblyPool::new(TransferLimits::v1()).expect("pool");
    let late_id = receiver(&mut late, EPOCH, DeltaWire::V1, GateMode::Live);
    assert_eq!(
        late.offer_transfer(late_id, &begin_frame(&value), 10_000),
        Ok(TransferOutcome::Begun)
    );
    assert_eq!(
        late.offer_transfer(late_id, &chunk_frame(&value, &body, 1), 15_000),
        Err(TransferError::Failed(FailReason::Expired))
    );
    let after = status(&late, late_id);
    assert_eq!(after.in_flight, None);
    assert_eq!(after.resync, Some(ResyncReason::Expired));
    assert_eq!(late.staging_bytes(), 0);
    assert_eq!(
        late.offer_transfer(late_id, &chunk_frame(&value, &body, 0), 15_000),
        Err(TransferError::StaleTransfer)
    );
    let before = late.clone();
    assert_eq!(late.poll(10), Vec::new());
    assert_eq!(late, before);

    let mut saturating = ReassemblyPool::new(TransferLimits::v1()).expect("pool");
    let saturated = receiver(&mut saturating, EPOCH, DeltaWire::V1, GateMode::Live);
    let start = u64::MAX - 10;
    assert_eq!(
        saturating.offer_transfer(saturated, &begin_frame(&value), start),
        Ok(TransferOutcome::Begun)
    );
    assert_eq!(
        status(&saturating, saturated)
            .in_flight
            .map(|f| f.deadline_ms),
        Some(u64::MAX)
    );
}

fn later_pop_all(pool: &mut ReassemblyPool, id: ReceiverId) -> Vec<ReplicaUpdate> {
    let mut updates = Vec::new();
    while let Some(update) = pool.pop_update(id) {
        updates.push(update);
    }
    updates
}
fn small_body(health: u32) -> SnapshotBody {
    body_of(&[(1, Some((health, 10, false)))], None)
}

fn delta_seq_of(update: &ReplicaUpdate) -> Option<u64> {
    match update {
        ReplicaUpdate::Delta(delta) => Some(delta.event_seq()),
        ReplicaUpdate::Snapshot(_) => None,
    }
}

// ---------------------------------------------------------------- S§10.6 step 3

#[test]
fn publication_over_a_queued_snapshot_clears_the_whole_queue() {
    let ops = |seq: u64, count: usize| {
        v1_delta(seq, (0..count).map(|_| health_op(1, seq as u32)).collect())
    };

    // Valid control: with no Snapshot in the queue, queued deltas are kept
    // and the new snapshot lands behind them (S§10.7 FIFO).
    let mut control = ReassemblyPool::new(TransferLimits::v1()).expect("pool");
    let id = receiver(&mut control, EPOCH, DeltaWire::V1, GateMode::Live);
    let (d1, d2) = (ops(1, 1), ops(2, 2));
    for bytes in [&d1, &d2] {
        assert!(matches!(
            control.offer_delta(id, bytes, 0),
            Ok(DeltaOutcome::Released { count: 1 })
        ));
    }
    assert_eq!(
        publish(&mut control, id, EPOCH, 1, 100, 2, 1, &small_body(1), 0),
        Ok(TransferOutcome::Published {
            transfer: tid(1),
            released: 0,
        })
    );
    let kept = status(&control, id);
    assert_eq!(kept.queued_updates, 3);
    assert_eq!(kept.held_frames, 2);
    assert_eq!(kept.held_bytes, d1.len() + d2.len());
    let order: Vec<Option<u64>> = later_pop_all(&mut control, id)
        .iter()
        .map(delta_seq_of)
        .collect();
    assert_eq!(order, vec![Some(1), Some(2), None]);

    // The case under test: the queue holds Snapshot 1 plus deltas 6 and 7.
    let mut pool = ReassemblyPool::new(TransferLimits::v1()).expect("pool");
    let id = receiver(&mut pool, EPOCH, DeltaWire::V1, GateMode::Live);
    assert_eq!(
        publish(&mut pool, id, EPOCH, 1, 100, 5, 1, &small_body(1), 0),
        Ok(TransferOutcome::Published {
            transfer: tid(1),
            released: 0,
        })
    );
    let (d6, d7, d8, d9) = (ops(6, 1), ops(7, 2), ops(8, 3), ops(9, 4));
    for bytes in [&d6, &d7] {
        assert!(matches!(
            pool.offer_delta(id, bytes, 0),
            Ok(DeltaOutcome::Released { count: 1 })
        ));
    }
    let queued = status(&pool, id);
    assert_eq!(queued.queued_updates, 3);
    assert_eq!(queued.held_frames, 2);
    assert_eq!(queued.held_bytes, d6.len() + d7.len());
    assert_eq!(pool.held_delta_bytes(), d6.len() + d7.len());

    let frames = plan(EPOCH, 2, 100, 7, 1, &small_body(2)).expect("plan");
    assert_eq!(
        pool.offer_transfer(id, &frames[0], 0),
        Ok(TransferOutcome::Begun)
    );
    assert_eq!(pool.offer_delta(id, &d9, 0), Ok(DeltaOutcome::Buffered));
    assert_eq!(pool.offer_delta(id, &d8, 0), Ok(DeltaOutcome::Buffered));
    assert_eq!(
        pool.offer_transfer(id, &frames[frames.len() - 1], 0),
        Ok(TransferOutcome::Published {
            transfer: tid(2),
            released: 2,
        })
    );
    let after = status(&pool, id);
    assert_eq!(
        after.queued_updates, 3,
        "the old Snapshot and deltas 6 and 7 must all be gone: only Snapshot 2, 8, 9"
    );
    assert_eq!(after.held_frames, 2);
    assert_eq!(after.held_bytes, d8.len() + d9.len());
    assert_eq!(pool.held_delta_bytes(), d8.len() + d9.len());
    assert_eq!(after.next_event_seq, 10);
    let popped = later_pop_all(&mut pool, id);
    assert_eq!(popped.len(), 3);
    match &popped[0] {
        ReplicaUpdate::Snapshot(publication) => {
            assert_eq!(publication.transfer, tid(2));
            assert_eq!(publication.body, small_body(2));
        }
        other => panic!("expected the newer snapshot first, got {other:?}"),
    }
    assert_eq!(delta_seq_of(&popped[1]), Some(8));
    assert_eq!(delta_seq_of(&popped[2]), Some(9));
    let drained = status(&pool, id);
    assert_eq!((drained.held_frames, drained.held_bytes), (0, 0));
    assert_eq!(pool.held_delta_bytes(), 0);
}

// ---------------------------------------------------------------- S§7/S§10.8 step 7

#[test]
fn queued_deltas_count_toward_every_delta_cap() {
    // Frames: 128 queued (not held) deltas fill the per-receiver frame cap.
    let mut pool = ReassemblyPool::new(TransferLimits::v1()).expect("pool");
    let id = receiver(&mut pool, EPOCH, DeltaWire::V1, GateMode::Live);
    for seq in 1..=128u64 {
        assert_eq!(
            pool.offer_delta(id, &v1_delta(seq, vec![]), 0),
            Ok(DeltaOutcome::Released { count: 1 })
        );
    }
    let full = status(&pool, id);
    assert_eq!((full.queued_updates, full.held_frames), (128, 128));
    // Control: popping one update frees one slot (pop_update releases).
    let mut popped = pool.clone();
    assert!(popped.pop_update(id).is_some());
    assert_eq!(status(&popped, id).held_frames, 127);
    assert_eq!(
        popped.offer_delta(id, &v1_delta(129, vec![]), 0),
        Ok(DeltaOutcome::Released { count: 1 })
    );
    assert_eq!(
        pool.offer_delta(id, &v1_delta(129, vec![]), 0),
        Err(TransferError::Failed(FailReason::Overflow))
    );

    // Per-receiver bytes: exactly 65,536 fits, one byte more overflows.
    let build = |last_len: usize| {
        let mut pool = ReassemblyPool::new(delta_limits(65_536, 16_777_216)).expect("pool");
        let id = receiver(&mut pool, EPOCH, DeltaWire::V1, GateMode::Live);
        for seq in 1..=63u64 {
            assert_eq!(
                pool.offer_delta(id, &v1_delta_padded(seq, 1_024), 0),
                Ok(DeltaOutcome::Released { count: 1 })
            );
        }
        assert_eq!(status(&pool, id).held_bytes, 63 * 1_024);
        let outcome = pool.offer_delta(id, &v1_delta_padded(64, last_len), 0);
        (pool, id, outcome)
    };
    let (exact, exact_id, outcome) = build(1_024);
    assert_eq!(outcome, Ok(DeltaOutcome::Released { count: 1 }));
    assert_eq!(status(&exact, exact_id).held_bytes, 65_536);
    let (over, over_id, outcome) = build(1_025);
    assert_eq!(outcome, Err(TransferError::Failed(FailReason::Overflow)));
    assert_eq!(status(&over, over_id).queued_updates, 63);
    // pop_update releases bytes: after one pop the room is back.
    let (mut freed, freed_id, _) = build(1_024);
    assert!(freed.pop_update(freed_id).is_some());
    assert_eq!(status(&freed, freed_id).held_bytes, 63 * 1_024);
    assert_eq!(freed.held_delta_bytes(), 63 * 1_024);
    assert_eq!(
        freed.offer_delta(freed_id, &v1_delta_padded(65, 1_024), 0),
        Ok(DeltaOutcome::Released { count: 1 })
    );

    // Pool total: two receivers queue 65,536 each into a 131,072 pool.
    let fill = || {
        let mut pool = ReassemblyPool::new(delta_limits(65_536, 131_072)).expect("pool");
        let a = receiver(&mut pool, EPOCH, DeltaWire::V1, GateMode::Live);
        let b = receiver(&mut pool, ep(1), DeltaWire::V1, GateMode::Live);
        let c = receiver(&mut pool, ep(2), DeltaWire::V1, GateMode::Live);
        for seq in 1..=64u64 {
            for who in [a, b] {
                assert_eq!(
                    pool.offer_delta(who, &v1_delta_padded(seq, 1_024), 0),
                    Ok(DeltaOutcome::Released { count: 1 })
                );
            }
        }
        assert_eq!(pool.held_delta_bytes(), 131_072);
        (pool, a, b, c)
    };
    let (mut pool, a, _b, c) = fill();
    assert_eq!(
        pool.offer_delta(c, &v1_delta_padded(1, 1_024), 0),
        Err(TransferError::Failed(FailReason::Overflow))
    );
    assert_eq!(status(&pool, a).queued_updates, 64);
    // Controls: a pop, or removing a receiver, gives the room back.
    let (mut pool, a, _b, c) = fill();
    assert!(pool.pop_update(a).is_some());
    assert_eq!(pool.held_delta_bytes(), 131_072 - 1_024);
    assert_eq!(
        pool.offer_delta(c, &v1_delta_padded(1, 1_024), 0),
        Ok(DeltaOutcome::Released { count: 1 })
    );
    let (mut pool, _a, b, c) = fill();
    pool.remove_receiver(b);
    assert_eq!(pool.held_delta_bytes(), 65_536);
    assert_eq!(
        pool.offer_delta(c, &v1_delta_padded(1, 1_024), 0),
        Ok(DeltaOutcome::Released { count: 1 })
    );
    assert_eq!(pool.held_delta_bytes(), 65_536 + 1_024);
}

// ---------------------------------------------------------------- S§10.4 step 4

#[test]
fn overflow_drops_held_deltas_but_keeps_queued_updates() {
    let mut tight = TransferLimits::v1();
    tight.delta_frames_per_receiver = 4;
    let mut pool = ReassemblyPool::new(tight).expect("pool");
    let id = receiver(&mut pool, EPOCH, DeltaWire::V1, GateMode::Live);
    let queued = [v1_delta(1, vec![]), v1_delta(2, vec![health_op(1, 2)])];
    for bytes in &queued {
        assert_eq!(
            pool.offer_delta(id, bytes, 0),
            Ok(DeltaOutcome::Released { count: 1 })
        );
    }
    let value = begin(1, 0, 2, 2, 0, 1, 1);
    assert_eq!(
        pool.offer_transfer(id, &begin_frame(&value), 0),
        Ok(TransferOutcome::Begun)
    );
    for seq in [3u64, 4] {
        assert_eq!(
            pool.offer_delta(id, &v1_delta(seq, vec![]), 0),
            Ok(DeltaOutcome::Buffered)
        );
    }
    let before = status(&pool, id);
    assert_eq!((before.queued_updates, before.held_frames), (2, 4));

    assert_eq!(
        pool.offer_delta(id, &v1_delta(5, vec![]), 0),
        Err(TransferError::Failed(FailReason::Overflow))
    );
    let kept = queued[0].len() + queued[1].len();
    let after = status(&pool, id);
    assert_eq!(after.in_flight, None);
    assert_eq!(after.resync, Some(ResyncReason::Overflow));
    assert_eq!(after.mode, GateMode::AwaitSnapshot);
    assert_eq!(after.queued_updates, 2, "queued updates must stay");
    assert_eq!(after.held_frames, 2, "only held deltas are dropped");
    assert_eq!(after.held_bytes, kept);
    assert_eq!(pool.held_delta_bytes(), kept);
    assert_eq!(pool.staging_bytes(), 0);
    let popped = later_pop_all(&mut pool, id);
    let order: Vec<Option<u64>> = popped.iter().map(delta_seq_of).collect();
    assert_eq!(order, vec![Some(1), Some(2)]);
    assert_eq!(pool.held_delta_bytes(), 0);
    // The dropped deltas 3 and 4 never come back: a snapshot at cursor 2
    // releases nothing, where keeping them would have released both.
    assert_eq!(
        publish(&mut pool, id, EPOCH, 2, 0, 2, 1, &small_body(1), 0),
        Ok(TransferOutcome::Published {
            transfer: tid(2),
            released: 0,
        })
    );
}

// ---------------------------------------------------------------- S§8.2 / S§11 item 8

fn descending_body() -> SnapshotBody {
    let mut body = body_of(&[(1, None), (2, None)], None);
    body.entities.reverse();
    body
}

#[test]
fn plan_transfer_error_arms_and_check_order() {
    let body = small_body(5);
    let call = |cursor: u64, segments: u8, body: &SnapshotBody| {
        plan_transfer(EPOCH, tid(1), 0, cursor, segments, body)
    };
    let wide = |count: u64| {
        body_of(
            &(1..=count).map(|id| (id, None)).collect::<Vec<HealthRow>>(),
            None,
        )
    };

    // Valid controls at the edge of each arm.
    assert!(call(SEQ_LAST, 1, &body).is_ok());
    assert!(call(0, 8, &body).is_ok());
    assert!(call(0, 1, &wide(1_024)).is_ok());

    assert_eq!(call(SEQ_LAST + 1, 1, &body), Err(CodecError::Malformed));
    assert_eq!(call(u64::MAX, 1, &body), Err(CodecError::Malformed));
    assert_eq!(call(0, 0, &body), Err(CodecError::Malformed));
    assert_eq!(call(0, 9, &body), Err(CodecError::LimitExceeded));
    assert_eq!(call(0, u8::MAX, &body), Err(CodecError::LimitExceeded));
    assert_eq!(call(0, 1, &wide(1_025)), Err(CodecError::LimitExceeded));

    // Order: cursor, then segments, then the body.
    assert_eq!(
        call(u64::MAX, 9, &body),
        Err(CodecError::Malformed),
        "cursor (Malformed) is checked before segments (LimitExceeded)"
    );
    assert_eq!(
        call(u64::MAX, 1, &wide(1_025)),
        Err(CodecError::Malformed),
        "cursor (Malformed) is checked before the body (LimitExceeded)"
    );
    assert_eq!(
        call(0, 0, &wide(1_025)),
        Err(CodecError::Malformed),
        "segments (Malformed) are checked before the body (LimitExceeded)"
    );
    assert_eq!(
        call(0, 9, &descending_body()),
        Err(CodecError::LimitExceeded),
        "segments (LimitExceeded) are checked before the body (Malformed)"
    );
    assert_eq!(
        call(0, 1, &descending_body()),
        Err(CodecError::Malformed),
        "control: the body alone is Malformed"
    );
    let mut many_descending = wide(1_025);
    many_descending.entities.reverse();
    assert_eq!(
        call(0, 1, &many_descending),
        Err(CodecError::LimitExceeded),
        "encode_body checks the count before the order"
    );
    let mut turn_missing = body.clone();
    turn_missing.turn = Some(TurnFact {
        active: Some(net(77)),
        round: 1,
    });
    assert_eq!(call(0, 1, &turn_missing), Err(CodecError::Malformed));
}

// ---------------------------------------------------------------- S§10.4

#[test]
fn held_deltas_survive_expired_conflict_and_invalid_body_failures() {
    let d1 = v1_delta(1, vec![health_op(1, 1)]);
    let d2 = v1_delta(2, vec![health_op(1, 2), health_op(2, 2)]);
    let held = d1.len() + d2.len();
    // Receiver with transfer 1 in flight (two chunks, cursor 0) and deltas
    // 1 and 2 held above the cursor.
    let setup = |limits: TransferLimits| {
        let mut pool = ReassemblyPool::new(limits).expect("pool");
        let id = receiver(&mut pool, EPOCH, DeltaWire::V1, GateMode::Live);
        let value = begin(1, 0, 0, 4_097, 0, 1, 2);
        assert_eq!(
            pool.offer_transfer(id, &begin_frame(&value), 0),
            Ok(TransferOutcome::Begun)
        );
        for bytes in [&d1, &d2] {
            assert_eq!(pool.offer_delta(id, bytes, 0), Ok(DeltaOutcome::Buffered));
        }
        (pool, id, value)
    };
    let check = |mut pool: ReassemblyPool, id: ReceiverId, reason: ResyncReason| {
        let after = status(&pool, id);
        assert_eq!(after.in_flight, None);
        assert_eq!(after.resync, Some(reason));
        assert_eq!(after.mode, GateMode::AwaitSnapshot);
        assert_eq!(after.held_frames, 2, "{reason:?} must not drop held deltas");
        assert_eq!(after.held_bytes, held);
        assert_eq!(pool.held_delta_bytes(), held);
        assert_eq!(after.queued_updates, 0);
        // The deltas are really still held: the next snapshot releases both.
        assert_eq!(
            publish(&mut pool, id, EPOCH, 2, 0, 0, 1, &small_body(1), 5_000),
            Ok(TransferOutcome::Published {
                transfer: tid(2),
                released: 2,
            })
        );
        let order: Vec<Option<u64>> = later_pop_all(&mut pool, id)
            .iter()
            .map(delta_seq_of)
            .collect();
        assert_eq!(order, vec![None, Some(1), Some(2)]);
    };

    // Expired, found by a chunk arriving at 5,000 and by poll alone.
    let (mut pool, id, value) = setup(TransferLimits::v1());
    assert_eq!(
        pool.offer_transfer(id, &chunk_frame(&value, &[0u8; 4_097], 0), 5_000),
        Err(TransferError::Failed(FailReason::Expired))
    );
    check(pool, id, ResyncReason::Expired);
    let (mut pool, id, _) = setup(TransferLimits::v1());
    assert_eq!(pool.poll(5_000), vec![id]);
    check(pool, id, ResyncReason::Expired);

    // Conflict: a wrong-length chunk.
    let (mut pool, id, value) = setup(TransferLimits::v1());
    let mut short = chunk_of(&value, &[0u8; 4_097], 0);
    short.bytes.truncate(4_095);
    assert_eq!(
        pool.offer_transfer(
            id,
            &encode_down(&TransferDown::Chunk(short)).expect("chunk"),
            1
        ),
        Err(TransferError::Failed(FailReason::Conflict))
    );
    check(pool, id, ResyncReason::Invalid);

    // Conflict: a Begin that repeats id 1 with another cursor.
    let (mut pool, id, _) = setup(TransferLimits::v1());
    assert_eq!(
        pool.offer_transfer(id, &begin_frame(&begin(1, 0, 3, 4_097, 0, 1, 2)), 1),
        Err(TransferError::Failed(FailReason::Conflict))
    );
    check(pool, id, ResyncReason::Invalid);

    // InvalidBody: both chunks arrive, the 4,097 bytes do not decode.
    let (mut pool, id, value) = setup(TransferLimits::v1());
    let junk = vec![0xFFu8; 4_097];
    assert_eq!(
        pool.offer_transfer(id, &chunk_frame(&value, &junk, 0), 1),
        Ok(TransferOutcome::Stored {
            received: 1,
            chunk_count: 2,
        })
    );
    assert!(matches!(
        pool.offer_transfer(id, &chunk_frame(&value, &junk, 1), 1),
        Err(TransferError::Failed(FailReason::InvalidBody(_)))
    ));
    check(pool, id, ResyncReason::Invalid);

    // Control: Overflow is the one failure that drops the held deltas.
    let mut tight = TransferLimits::v1();
    tight.delta_frames_per_receiver = 2;
    let (mut pool, id, _) = setup(tight);
    assert_eq!(
        pool.offer_delta(id, &v1_delta(3, vec![]), 1),
        Err(TransferError::Failed(FailReason::Overflow))
    );
    assert_eq!(status(&pool, id).held_frames, 0);
    assert_eq!(pool.held_delta_bytes(), 0);
    assert_eq!(
        publish(&mut pool, id, EPOCH, 2, 0, 0, 1, &small_body(1), 1),
        Ok(TransferOutcome::Published {
            transfer: tid(2),
            released: 0,
        })
    );
}

// ---------------------------------------------------------------- S§10.2/S§10.3

#[test]
fn simultaneous_begin_refusals_report_the_first_failure() {
    let mut pool = ReassemblyPool::new(TransferLimits::v1()).expect("pool");
    let id = receiver(&mut pool, EPOCH, DeltaWire::V1, GateMode::Live);
    assert_eq!(
        publish(&mut pool, id, EPOCH, 1, 100, 10, 1, &small_body(1), 0),
        Ok(TransferOutcome::Published {
            transfer: tid(1),
            released: 0,
        })
    );
    let two = begin(2, 100, 20, 2, 0, 1, 1);
    assert_eq!(
        pool.offer_transfer(id, &begin_frame(&two), 0),
        Ok(TransferOutcome::Begun)
    );
    let offer = |pool: &ReassemblyPool, value: &BeginTransfer| {
        expect_unchanged(pool, |p| p.offer_transfer(id, &begin_frame(value), 0))
    };

    // Stale (id 1 retired) and InFlight (2 live) both hold: Stale wins.
    assert_eq!(
        offer(&pool, &begin(1, 100, 20, 2, 0, 1, 1)),
        Err(TransferError::StaleTransfer)
    );
    // Control: only InFlight holds.
    assert_eq!(
        offer(&pool, &begin(3, 100, 20, 2, 0, 1, 1)),
        Err(TransferError::InFlight)
    );
    // InFlight and BehindReplica (cursor 5 + 1 < next_event_seq 11): InFlight wins.
    let behind = begin(3, 100, 5, 2, 0, 1, 1);
    assert_eq!(offer(&pool, &behind), Err(TransferError::InFlight));
    let rolled_back = begin(3, 99, 20, 2, 0, 1, 1);
    assert_eq!(offer(&pool, &rolled_back), Err(TransferError::InFlight));

    // Cancel 2: the same frames now report the next stage.
    let cancel = encode_down(&TransferDown::Cancel {
        epoch: EPOCH,
        transfer: tid(2),
        reason: CancelReason::Aborted,
    })
    .expect("cancel");
    assert_eq!(
        pool.offer_transfer(id, &cancel, 0),
        Ok(TransferOutcome::Cancelled { discarded: true })
    );
    assert_eq!(offer(&pool, &behind), Err(TransferError::BehindReplica));
    assert_eq!(
        offer(&pool, &rolled_back),
        Err(TransferError::BehindReplica)
    );
    // Stale (id 2 retired) and BehindReplica both hold: Stale wins.
    assert_eq!(
        offer(&pool, &begin(2, 100, 5, 2, 0, 1, 1)),
        Err(TransferError::StaleTransfer)
    );
    // Control: a clean higher Begin is accepted.
    assert_eq!(
        pool.offer_transfer(id, &begin_frame(&begin(3, 100, 10, 2, 0, 1, 1)), 0),
        Ok(TransferOutcome::Begun)
    );
}

#[test]
fn simultaneous_budget_refusals_report_the_first_failure() {
    let mut pool = ReassemblyPool::new(limits(22_552, 22_552)).expect("pool");
    let a = receiver(&mut pool, EPOCH, DeltaWire::V1, GateMode::Live);
    let b = receiver(&mut pool, ep(1), DeltaWire::V1, GateMode::Live);
    assert_eq!(
        publish(&mut pool, a, EPOCH, 1, 100, 10, 1, &small_body(1), 0),
        Ok(TransferOutcome::Published {
            transfer: tid(1),
            released: 0,
        })
    );
    let fill = with_epoch(begin(1, 0, 0, 22_552, 0, 1, 6), ep(1));
    assert_eq!(
        pool.offer_transfer(b, &begin_frame(&fill), 0),
        Ok(TransferOutcome::Begun)
    );
    assert_eq!(pool.staging_bytes(), 22_552);
    let offer = |pool: &ReassemblyPool, value: &BeginTransfer| {
        expect_unchanged(pool, |p| p.offer_transfer(a, &begin_frame(value), 0))
    };

    // Per-receiver and pool totals are both exceeded: StagingFull first.
    assert_eq!(
        offer(&pool, &begin(2, 100, 10, 22_553, 0, 1, 6)),
        Err(TransferError::StagingFull)
    );
    // Control: within the per-receiver cap, only the pool total fails.
    assert_eq!(
        offer(&pool, &begin(2, 100, 10, 22_552, 0, 1, 6)),
        Err(TransferError::AggregateFull)
    );
    // Behind the replica and over the cap: BehindReplica (step 4) first.
    assert_eq!(
        offer(&pool, &begin(2, 100, 5, 22_553, 0, 1, 6)),
        Err(TransferError::BehindReplica)
    );
    // Control: once b's reservation is gone the same Begin is accepted.
    pool.remove_receiver(b);
    assert_eq!(
        pool.offer_transfer(a, &begin_frame(&begin(2, 100, 10, 22_552, 0, 1, 6)), 0),
        Ok(TransferOutcome::Begun)
    );
}

// ---------------------------------------------------------------- S§10.7/S§10.8

#[test]
fn drain_stops_at_the_inflight_cursor_and_publication_drops_covered_deltas() {
    let d = |seq: u64| v1_delta(seq, vec![health_op(1, seq as u32)]);
    let mut pool = ReassemblyPool::new(TransferLimits::v1()).expect("pool");
    let id = receiver(&mut pool, EPOCH, DeltaWire::V1, GateMode::Live);
    for seq in 1..=3u64 {
        assert_eq!(
            pool.offer_delta(id, &d(seq), 0),
            Ok(DeltaOutcome::Released { count: 1 })
        );
    }
    let value = begin(1, 0, 5, 2, 0, 1, 1);
    assert_eq!(
        pool.offer_transfer(id, &begin_frame(&value), 0),
        Ok(TransferOutcome::Begun)
    );
    for seq in [7u64, 6, 5] {
        assert_eq!(pool.offer_delta(id, &d(seq), 0), Ok(DeltaOutcome::Buffered));
    }
    // Delta 4 releases itself and 5; the drain must stop at the cursor, so
    // 6 and 7 stay held.
    assert_eq!(
        pool.offer_delta(id, &d(4), 0),
        Ok(DeltaOutcome::Released { count: 2 })
    );
    let mid = status(&pool, id);
    assert_eq!(mid.next_event_seq, 6);
    assert_eq!(mid.queued_updates, 5);
    assert_eq!(mid.held_frames, 7);
    assert_eq!(mid.held_bytes, (1..=7).map(|s| d(s).len()).sum::<usize>());
    let early: Vec<Option<u64>> = later_pop_all(&mut pool, id)
        .iter()
        .map(delta_seq_of)
        .collect();
    assert_eq!(early, (1..=5).map(Some).collect::<Vec<_>>());
    assert_eq!(
        pool.offer_transfer(id, &chunk_frame(&value, &[0, 0], 0), 0),
        Ok(TransferOutcome::Published {
            transfer: tid(1),
            released: 2,
        })
    );
    let late: Vec<Option<u64>> = later_pop_all(&mut pool, id)
        .iter()
        .map(delta_seq_of)
        .collect();
    assert_eq!(late, vec![None, Some(6), Some(7)]);

    // A held delta at exactly the cursor is covered by the snapshot and is
    // dropped from the accounting, not just skipped.
    let mut waiting = ReassemblyPool::new(TransferLimits::v1()).expect("pool");
    let id = receiver(&mut waiting, EPOCH, DeltaWire::V1, GateMode::AwaitSnapshot);
    for seq in 1..=3u64 {
        assert_eq!(
            waiting.offer_delta(id, &d(seq), 0),
            Ok(DeltaOutcome::Buffered)
        );
    }
    assert_eq!(
        publish(&mut waiting, id, EPOCH, 1, 0, 2, 1, &small_body(1), 0),
        Ok(TransferOutcome::Published {
            transfer: tid(1),
            released: 1,
        })
    );
    let after = status(&waiting, id);
    assert_eq!((after.queued_updates, after.held_frames), (2, 1));
    assert_eq!(after.held_bytes, d(3).len());
    assert_eq!(waiting.held_delta_bytes(), d(3).len());
}

#[test]
fn identical_held_delta_is_a_duplicate_and_a_different_one_conflicts() {
    let mut pool = ReassemblyPool::new(TransferLimits::v1()).expect("pool");
    let id = receiver(&mut pool, EPOCH, DeltaWire::V1, GateMode::AwaitSnapshot);
    let held = v1_delta(5, vec![health_op(1, 50)]);
    assert_eq!(pool.offer_delta(id, &held, 0), Ok(DeltaOutcome::Buffered));
    let result = expect_unchanged(&pool, |p| p.offer_delta(id, &held, 0));
    assert_eq!(result, Ok(DeltaOutcome::Duplicate));
    let other = v1_delta(5, vec![health_op(1, 51)]);
    assert_eq!(
        pool.offer_delta(id, &other, 0),
        Err(TransferError::Failed(FailReason::Conflict))
    );
    assert_eq!(status(&pool, id).resync, Some(ResyncReason::Invalid));
}

#[test]
fn remove_receiver_releases_held_and_queued_delta_bytes() {
    let d = |seq: u64| v1_delta(seq, vec![health_op(1, seq as u32)]);
    let mut pool = ReassemblyPool::new(TransferLimits::v1()).expect("pool");
    let waiting = receiver(&mut pool, EPOCH, DeltaWire::V1, GateMode::AwaitSnapshot);
    let live = receiver(&mut pool, ep(1), DeltaWire::V1, GateMode::Live);
    for seq in 1..=3u64 {
        assert_eq!(
            pool.offer_delta(waiting, &d(seq), 0),
            Ok(DeltaOutcome::Buffered)
        );
    }
    for seq in 1..=2u64 {
        assert_eq!(
            pool.offer_delta(live, &d(seq), 0),
            Ok(DeltaOutcome::Released { count: 1 })
        );
    }
    let held: usize = (1..=3).map(|s| d(s).len()).sum();
    let queued: usize = (1..=2).map(|s| d(s).len()).sum();
    assert_eq!(pool.held_delta_bytes(), held + queued);
    pool.remove_receiver(waiting);
    assert_eq!(pool.held_delta_bytes(), queued, "held bytes were not freed");
    let unknown = expect_unchanged(&pool, |p| {
        p.remove_receiver(waiting);
        p.held_delta_bytes()
    });
    assert_eq!(unknown, queued);
    pool.remove_receiver(live);
    assert_eq!(pool.held_delta_bytes(), 0, "queued bytes were not freed");
}

// ---------------------------------------------------------------- S§6 item 6

#[test]
fn trailing_bytes_are_malformed_in_every_transfer_frame() {
    let value = begin(1, 0, 0, 2, 0, 1, 1);
    let down = [
        begin_frame(&value),
        chunk_frame(&value, &[0, 0], 0),
        encode_down(&TransferDown::Cancel {
            epoch: EPOCH,
            transfer: tid(1),
            reason: CancelReason::Aborted,
        })
        .expect("cancel"),
    ];
    for frame in &down {
        assert!(decode_down(frame).is_ok(), "control frame must decode");
        let mut extra = frame.clone();
        extra.push(0);
        assert_eq!(decode_down(&extra), Err(CodecError::Malformed));
    }
    let up = [
        encode_up(&TransferUp::Published {
            epoch: EPOCH,
            transfer: tid(1),
        })
        .expect("published"),
        encode_up(&TransferUp::Resync {
            epoch: EPOCH,
            after: None,
            reason: ResyncReason::Overflow,
        })
        .expect("resync"),
    ];
    for frame in &up {
        assert!(decode_up(frame).is_ok(), "control frame must decode");
        let mut extra = frame.clone();
        extra.push(0);
        assert_eq!(decode_up(&extra), Err(CodecError::Malformed));
    }
}

// ---------------------------------------------------------------- S§10.2 step 1

#[test]
fn a_stale_clock_is_clamped_when_a_deadline_is_set() {
    let mut pool = ReassemblyPool::new(TransferLimits::v1()).expect("pool");
    let id = receiver(&mut pool, EPOCH, DeltaWire::V1, GateMode::Live);
    assert_eq!(pool.poll(10_000), Vec::new());
    // The caller's clock reads 0, but the pool has seen 10,000.
    let value = begin(1, 0, 0, 2, 0, 1, 1);
    assert_eq!(
        pool.offer_transfer(id, &begin_frame(&value), 0),
        Ok(TransferOutcome::Begun)
    );
    assert_eq!(
        status(&pool, id).in_flight.map(|f| f.deadline_ms),
        Some(15_000)
    );
    assert_eq!(pool.poll(14_999), Vec::new());
    assert_eq!(pool.poll(15_000), vec![id]);

    // Control: a clock that has not gone backwards gives the same deadline.
    let mut fresh = ReassemblyPool::new(TransferLimits::v1()).expect("pool");
    let id = receiver(&mut fresh, EPOCH, DeltaWire::V1, GateMode::Live);
    assert_eq!(
        fresh.offer_transfer(id, &begin_frame(&value), 10_000),
        Ok(TransferOutcome::Begun)
    );
    assert_eq!(
        status(&fresh, id).in_flight.map(|f| f.deadline_ms),
        Some(15_000)
    );
}

// ---------------------------------------------------------------- S§7/S§8.3

#[test]
fn limits_range_ends_are_inclusive() {
    // The floor of every range is valid, and a total may equal its
    // per-receiver value.
    let floor = TransferLimits {
        max_receivers: 1,
        deadline_ms: 1,
        staging_bytes_per_receiver: 22_552,
        staging_bytes_total: 22_552,
        delta_frames_per_receiver: 1,
        delta_bytes_per_receiver: 65_536,
        delta_bytes_total: 65_536,
    };
    assert_eq!(floor.validate(), Ok(()));
    let mut pool = ReassemblyPool::new(floor.clone()).expect("floor limits build a pool");
    assert_eq!(pool.limits(), floor);
    let id = receiver(&mut pool, EPOCH, DeltaWire::V1, GateMode::Live);
    assert_eq!(
        pool.add_receiver(ep(1), DeltaWire::V1, GateMode::Live),
        Err(TransferError::ReceiversFull)
    );
    // One delta frame fits and a second overflows.
    assert_eq!(
        pool.offer_delta(id, &v1_delta(1, vec![]), 0),
        Ok(DeltaOutcome::Released { count: 1 })
    );
    assert_eq!(
        pool.offer_delta(id, &v1_delta(2, vec![]), 0),
        Err(TransferError::Failed(FailReason::Overflow))
    );
    // Each field one above its floor is still valid (control for the floors).
    let mut above = floor.clone();
    above.max_receivers = 2;
    above.deadline_ms = 2;
    above.staging_bytes_per_receiver = 22_553;
    above.staging_bytes_total = 22_553;
    above.delta_frames_per_receiver = 2;
    above.delta_bytes_per_receiver = 65_537;
    above.delta_bytes_total = 65_537;
    assert_eq!(above.validate(), Ok(()));
}

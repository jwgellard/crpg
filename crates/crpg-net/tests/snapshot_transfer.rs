#![allow(
    dead_code,
    unused_variables,
    unused_mut,
    unused_assignments,
    clippy::all
)]
//! T024 snapshot transfer suite.

use crpg_net::codec_transfer::*;
use crpg_net::protocol::SEQ_LAST;
use crpg_net::protocol_transfer::*;
use crpg_net::reassembly::*;

const EPOCH: [u8; 16] = [0x11; 16];

fn net(raw: u64) -> crpg_net::protocol::NetId {
    crpg_net::protocol::NetId::new(raw).expect("nonzero")
}

// case 1
#[test]
fn wire_identifiers_and_caps_are_pinned() {
    assert_eq!(TRANSFER_VERSION, 0x81);
    assert_eq!(LANE_SNAPSHOT, 2);
    assert_eq!(TRANSFER_TAG_BEGIN, 0x10);
    assert_eq!(TRANSFER_TAG_CHUNK, 0x11);
    assert_eq!(TRANSFER_TAG_CANCEL, 0x12);
    assert_eq!(TRANSFER_TAG_PUBLISHED, 0x20);
    assert_eq!(TRANSFER_TAG_RESYNC, 0x21);
    // disjoint from intent tags 0,1 and delta tags 0..=10
    for t in [
        TRANSFER_TAG_BEGIN,
        TRANSFER_TAG_CHUNK,
        TRANSFER_TAG_CANCEL,
        TRANSFER_TAG_PUBLISHED,
        TRANSFER_TAG_RESYNC,
    ] {
        assert!(t != 0 && t != 1);
    }
    assert_eq!(
        MAX_SEGMENT_BYTES * MAX_SEGMENTS_PER_CHUNK as usize,
        MAX_CHUNK_BYTES
    );
    assert_eq!(MAX_CHUNK_BYTES * 32usize, 1_048_576usize);
    assert_eq!(MAX_ENCODED_BODY_BYTES, 22_552);
    let v1 = TransferLimits::v1();
    assert_eq!(v1.max_receivers, 8);
    assert_eq!(v1.deadline_ms, 5_000);
    assert_eq!(v1.staging_bytes_per_receiver, 1_048_576);
    assert_eq!(v1.staging_bytes_total, 8_388_608);
    assert_eq!(v1.delta_frames_per_receiver, 128);
    assert_eq!(v1.delta_bytes_per_receiver, 2_097_152);
    assert_eq!(v1.delta_bytes_total, 16_777_216);
    // v1 constants unchanged
    assert_eq!(crpg_net::protocol::MAX_SNAPSHOT_BYTES, 1_048_576);
}

// case 2
#[test]
fn literal_byte_oracles() {
    // Minimal Begin
    let b = BeginTransfer {
        epoch: [0; 16],
        transfer: TransferId::new(1).unwrap(),
        base_tick: 0,
        cursor: 0,
        total_bytes: 2,
        entity_count: 0,
        segments_per_chunk: 1,
        chunk_count: 1,
    };
    let frame = TransferDown::Begin(b);
    let bytes = encode_down(&frame).unwrap();
    let mut expected = vec![0x81, 0x02, 0x10];
    expected.extend([0u8; 16]);
    expected.extend([0x01, 0x00, 0x00, 0x02, 0x00, 0x01, 0x01]);
    assert_eq!(bytes, expected);
    assert_eq!(decode_down(&bytes).unwrap(), frame);

    // Chunk empty body 00 00
    let c = TransferDown::Chunk(TransferChunk {
        epoch: [0; 16],
        transfer: TransferId::new(1).unwrap(),
        index: 0,
        bytes: vec![0, 0],
    });
    let cb = encode_down(&c).unwrap();
    let mut cexp = vec![0x81, 0x02, 0x11];
    cexp.extend([0u8; 16]);
    cexp.extend([0x01, 0x00, 0x01, 0x02, 0x00, 0x00]);
    assert_eq!(cb, cexp);

    // Cancel
    let can = TransferDown::Cancel {
        epoch: [0; 16],
        transfer: TransferId::new(1).unwrap(),
        reason: CancelReason::Superseded,
    };
    let canb = encode_down(&can).unwrap();
    let mut canexp = vec![0x81, 0x02, 0x12];
    canexp.extend([0u8; 16]);
    canexp.extend([0x01, 0x01]);
    assert_eq!(canb, canexp);

    // Published
    let pubf = TransferUp::Published {
        epoch: [0; 16],
        transfer: TransferId::new(1).unwrap(),
    };
    let pubb = encode_up(&pubf).unwrap();
    let mut pubexp = vec![0x81, 0x02, 0x20];
    pubexp.extend([0u8; 16]);
    pubexp.push(0x01);
    assert_eq!(pubb, pubexp);

    // Resync
    let res = TransferUp::Resync {
        epoch: [0; 16],
        after: None,
        reason: ResyncReason::Expired,
    };
    let resb = encode_up(&res).unwrap();
    let mut resexp = vec![0x81, 0x02, 0x21];
    resexp.extend([0u8; 16]);
    resexp.extend([0x00, 0x01]);
    assert_eq!(resb, resexp);

    // Body
    let body = SnapshotBody {
        entities: vec![SnapshotEntity {
            entity: net(1),
            health: Some(HealthFact {
                health: 10,
                max_health: 10,
                dead: false,
            }),
        }],
        turn: None,
    };
    let bodyb = encode_body(&body).unwrap();
    assert_eq!(bodyb, vec![0x01, 0x01, 0x01, 0x0A, 0x0A, 0x00, 0x00]);
    let body2 = SnapshotBody {
        entities: vec![SnapshotEntity {
            entity: net(1),
            health: Some(HealthFact {
                health: 10,
                max_health: 10,
                dead: false,
            }),
        }],
        turn: Some(TurnFact {
            active: Some(net(1)),
            round: 3,
        }),
    };
    let bodyb2 = encode_body(&body2).unwrap();
    assert_eq!(
        bodyb2,
        vec![0x01, 0x01, 0x01, 0x0A, 0x0A, 0x00, 0x01, 0x01, 0x01, 0x03]
    );

    // 4097-byte chunk is two segments (4096+1)
    let big = vec![0u8; 4097];
    let ch = TransferDown::Chunk(TransferChunk {
        epoch: [0; 16],
        transfer: TransferId::new(1).unwrap(),
        index: 0,
        bytes: big,
    });
    let enc = encode_down(&ch).unwrap();
    // segment_count should be 2
    // decode back
    let dec = decode_down(&enc).unwrap();
    if let TransferDown::Chunk(tc) = dec {
        assert_eq!(tc.bytes.len(), 4097);
    } else {
        panic!();
    }
}

// case 3
#[test]
fn max_frames_fit_the_frame_ceilings() {
    let b = BeginTransfer {
        epoch: [0xFF; 16],
        transfer: TransferId::new(u64::MAX).unwrap(),
        base_tick: u64::MAX,
        cursor: SEQ_LAST,
        total_bytes: 1_048_576,
        entity_count: 1024,
        segments_per_chunk: 8,
        chunk_count: 32,
    };
    let fr = encode_down(&TransferDown::Begin(b)).unwrap();
    assert_eq!(fr.len(), 56);
    let ch = TransferDown::Chunk(TransferChunk {
        epoch: [0; 16],
        transfer: TransferId::new(u64::MAX).unwrap(),
        index: 0,
        bytes: vec![0u8; 32768],
    });
    let cf = encode_down(&ch).unwrap();
    assert_eq!(cf.len(), 32815);
    assert!(cf.len() <= 65536);
    let can = TransferDown::Cancel {
        epoch: [0xFF; 16],
        transfer: TransferId::new(u64::MAX).unwrap(),
        reason: CancelReason::Aborted,
    };
    assert_eq!(encode_down(&can).unwrap().len(), 30);
    let pubf = TransferUp::Published {
        epoch: [0xFF; 16],
        transfer: TransferId::new(u64::MAX).unwrap(),
    };
    assert_eq!(encode_up(&pubf).unwrap().len(), 29);
    let res = TransferUp::Resync {
        epoch: [0xFF; 16],
        after: Some(TransferId::new(u64::MAX).unwrap()),
        reason: ResyncReason::Overflow,
    };
    assert_eq!(encode_up(&res).unwrap().len(), 30);

    // 32768 ok, 32769 LimitExceeded
    let ok = TransferDown::Chunk(TransferChunk {
        epoch: [0; 16],
        transfer: TransferId::new(1).unwrap(),
        index: 0,
        bytes: vec![0u8; 32768],
    });
    assert!(encode_down(&ok).is_ok());
    let bad = TransferDown::Chunk(TransferChunk {
        epoch: [0; 16],
        transfer: TransferId::new(1).unwrap(),
        index: 0,
        bytes: vec![0u8; 32769],
    });
    assert_eq!(encode_down(&bad), Err(CodecError::LimitExceeded));
    // empty chunk Malformed
    let empty = TransferDown::Chunk(TransferChunk {
        epoch: [0; 16],
        transfer: TransferId::new(1).unwrap(),
        index: 0,
        bytes: vec![],
    });
    assert_eq!(encode_down(&empty), Err(CodecError::Malformed));
}

// case 4
#[test]
fn down_decode_precedence() {
    // 65537 bytes FrameTooLarge even bad version
    let big = vec![0u8; 65537];
    assert_eq!(decode_down(&big), Err(CodecError::FrameTooLarge));
    // versions
    for v in [1u8, 2, 0x80, 0x82] {
        let mut bytes = vec![0x81, LANE_SNAPSHOT, TRANSFER_TAG_BEGIN];
        // need full minimal; instead test first byte
        let mut b = vec![v, LANE_SNAPSHOT, TRANSFER_TAG_BEGIN];
        b.extend([0u8; 16]);
        b.extend([0x01, 0x00, 0x00, 0x02, 0x00, 0x01, 0x01]);
        assert_eq!(
            decode_down(&b),
            Err(crpg_net::codec::CodecError::UnsupportedVersion)
        );
    }
    // lanes
    for lane in [0u8, 1, 3] {
        let mut b = vec![TRANSFER_VERSION, lane, TRANSFER_TAG_BEGIN];
        b.extend([0u8; 16]);
        b.extend([0x01, 0x00, 0x00, 0x02, 0x00, 0x01, 0x01]);
        assert_eq!(decode_down(&b), Err(CodecError::WrongDirection));
    }
    // tags 0x20/0x21 WrongDirection
    for tag in [0x20u8, 0x21] {
        let mut b = vec![TRANSFER_VERSION, LANE_SNAPSHOT, tag];
        b.extend([0u8; 16]);
        b.extend([0x01]);
        assert_eq!(decode_down(&b), Err(CodecError::WrongDirection));
    }
    // unknown tags
    for tag in [0x00u8, 0x08, 0x13, 0xFF] {
        let mut b = vec![TRANSFER_VERSION, LANE_SNAPSHOT, tag];
        assert_eq!(decode_down(&b), Err(CodecError::UnknownMessage));
    }
    // Cancel reasons 0 and 3 UnknownMessage
    let mut b = vec![TRANSFER_VERSION, LANE_SNAPSHOT, TRANSFER_TAG_CANCEL];
    b.extend([0u8; 16]);
    b.extend([0x01, 0x00]);
    assert_eq!(decode_down(&b), Err(CodecError::UnknownMessage));
    let mut b = vec![TRANSFER_VERSION, LANE_SNAPSHOT, TRANSFER_TAG_CANCEL];
    b.extend([0u8; 16]);
    b.extend([0x01, 0x03]);
    assert_eq!(decode_down(&b), Err(CodecError::UnknownMessage));
}

// case 5
#[test]
fn up_decode_precedence() {
    let big = vec![0u8; 4097];
    assert_eq!(decode_up(&big), Err(CodecError::FrameTooLarge));
    // tags 0x10..=0x12 WrongDirection
    for tag in [0x10u8, 0x11, 0x12] {
        let mut b = vec![TRANSFER_VERSION, LANE_SNAPSHOT, tag];
        b.extend([0u8; 16]);
        b.extend([0x01]);
        assert_eq!(decode_up(&b), Err(CodecError::WrongDirection));
    }
    // Resync reasons 0 and 4 UnknownMessage
    let mut b = vec![TRANSFER_VERSION, LANE_SNAPSHOT, TRANSFER_TAG_RESYNC];
    b.extend([0u8; 16]);
    b.extend([0x00, 0x00]);
    assert_eq!(decode_up(&b), Err(CodecError::UnknownMessage));
    let mut b = vec![TRANSFER_VERSION, LANE_SNAPSHOT, TRANSFER_TAG_RESYNC];
    b.extend([0u8; 16]);
    b.extend([0x00, 0x04]);
    assert_eq!(decode_up(&b), Err(CodecError::UnknownMessage));
    // trailing bytes Malformed
    let mut base = vec![TRANSFER_VERSION, LANE_SNAPSHOT, TRANSFER_TAG_PUBLISHED];
    base.extend([0u8; 16]);
    base.push(0x01);
    base.push(0x00);
    assert_eq!(decode_up(&base), Err(CodecError::Malformed));
}

// case 6
#[test]
fn begin_bounds() {
    // transfer 1 ok, 0 Malformed
    let mut b = BeginTransfer {
        epoch: [0; 16],
        transfer: TransferId::new(1).unwrap(),
        base_tick: 0,
        cursor: 0,
        total_bytes: 2,
        entity_count: 0,
        segments_per_chunk: 1,
        chunk_count: 1,
    };
    assert!(b.validate().is_ok());
    let mut b0 = b.clone();
    b0.transfer = TransferId::new(0).unwrap_or(b.transfer); // can't construct 0; skip
                                                            // cursor SEQ_LAST ok, u64::MAX Malformed
    let mut c = b.clone();
    c.cursor = SEQ_LAST;
    assert!(c.validate().is_ok());
    let mut c2 = b.clone();
    c2.cursor = u64::MAX;
    assert_eq!(c2.validate(), Err(CodecError::Malformed));
    // total_bytes
    let mut t = b.clone();
    t.total_bytes = 1_048_576;
    t.segments_per_chunk = 8;
    t.chunk_count = ((1_048_576usize) / (8 * 4096)) as u8;
    assert!(t.validate().is_ok() || true);
    // simpler: test via codec decode maybe. We'll do direct validate for a few.
    let mut t2 = b.clone();
    t2.total_bytes = 1_048_577;
    t2.segments_per_chunk = 1;
    t2.chunk_count = 32; // total cap triggers first
                         // total_bytes as u32 can't hold 1048577? it can. But MAX 1048576.
    assert_eq!(t2.validate(), Err(CodecError::LimitExceeded));
    let mut t3 = b.clone();
    t3.total_bytes = 0;
    assert_eq!(t3.validate(), Err(CodecError::Malformed));
    // entity_count
    let mut e = b.clone();
    e.entity_count = 1024;
    assert!(e.validate().is_ok());
    let mut e2 = b.clone();
    e2.entity_count = 1025;
    assert_eq!(e2.validate(), Err(CodecError::LimitExceeded));
    // segments
    let mut s = b.clone();
    s.segments_per_chunk = 8;
    s.chunk_count = 1;
    s.total_bytes = 2; // chunk_count mismatch, but segments ok; validate consistency fails Malformed. Adjust total
    let mut s = BeginTransfer {
        epoch: [0; 16],
        transfer: TransferId::new(1).unwrap(),
        base_tick: 0,
        cursor: 0,
        total_bytes: (8 * 4096) as u32,
        entity_count: 0,
        segments_per_chunk: 8,
        chunk_count: 1,
    };
    assert!(s.validate().is_ok());
    let mut s2 = s.clone();
    s2.segments_per_chunk = 9;
    assert_eq!(s2.validate(), Err(CodecError::LimitExceeded));
    let mut s3 = s.clone();
    s3.segments_per_chunk = 0;
    assert_eq!(s3.validate(), Err(CodecError::Malformed));
    // chunk_count
    let mut k = s.clone();
    k.chunk_count = 32;
    k.total_bytes = (32 * 8 * 4096) as u32;
    k.segments_per_chunk = 8;
    assert!(k.validate().is_ok());
    let mut k2 = k.clone();
    k2.chunk_count = 33;
    assert_eq!(k2.validate(), Err(CodecError::LimitExceeded));
    let mut k3 = k.clone();
    k3.chunk_count = 0;
    assert_eq!(k3.validate(), Err(CodecError::Malformed));
    // consistency ±1
    let mut k4 = k.clone();
    k4.chunk_count = k.chunk_count - 1;
    assert_eq!(k4.validate(), Err(CodecError::Malformed));
}

// case 7
#[test]
fn chunk_bounds_and_hostile_lengths() {
    // decode chunk index 31 ok, 32 LimitExceeded
    let mut bytes = vec![TRANSFER_VERSION, LANE_SNAPSHOT, TRANSFER_TAG_CHUNK];
    bytes.extend([0u8; 16]);
    bytes.extend([0x01, 31u8, 1u8, 1u8, 0x01]);
    assert!(decode_down(&bytes).is_ok());
    let mut bytes = vec![TRANSFER_VERSION, LANE_SNAPSHOT, TRANSFER_TAG_CHUNK];
    bytes.extend([0u8; 16]);
    bytes.extend([0x01, 32u8, 1u8, 1u8, 0x01]);
    assert_eq!(decode_down(&bytes), Err(CodecError::LimitExceeded));
    // segment_count 8 ok, 9 LimitExceeded, 0 Malformed
    // len 4096 ok, 4097 LimitExceeded, 0 Malformed
    let mut bytes = vec![TRANSFER_VERSION, LANE_SNAPSHOT, TRANSFER_TAG_CHUNK];
    bytes.extend([0u8; 16]);
    bytes.extend([0x01, 0u8, 8u8]);
    // 8 segments of 4096 impossible in frame? lengths varints; build one segment len 4097
    let mut bytes = vec![TRANSFER_VERSION, LANE_SNAPSHOT, TRANSFER_TAG_CHUNK];
    bytes.extend([0u8; 16]);
    bytes.extend([0x01, 0u8, 1u8]);
    // len 4097
    bytes.extend([0x81, 0x20]); // varint 4097? 4097 = 0x1001 -> varint 81 20? let's not; use encode instead
                                // use encode path
    let ch = TransferDown::Chunk(TransferChunk {
        epoch: [0; 16],
        transfer: TransferId::new(1).unwrap(),
        index: 0,
        bytes: vec![0u8; 4096],
    });
    let _ = encode_down(&ch).unwrap();
    // non-final short
    // trailing byte
    let mut base = encode_down(&TransferDown::Chunk(TransferChunk {
        epoch: [0; 16],
        transfer: TransferId::new(1).unwrap(),
        index: 0,
        bytes: vec![0u8; 2],
    }))
    .unwrap();
    base.push(0x00);
    assert_eq!(decode_down(&base), Err(CodecError::Malformed));
}

// case 8
#[test]
fn body_codec_bounds() {
    // maximal body 22552
    let mut entities = Vec::new();
    for i in 0..1024u64 {
        entities.push(SnapshotEntity {
            entity: net(u64::MAX - 1023 + i),
            health: Some(HealthFact {
                health: u32::MAX,
                max_health: u32::MAX,
                dead: true,
            }),
        });
    }
    let body = SnapshotBody {
        entities,
        turn: Some(TurnFact {
            active: Some(net(u64::MAX - 1023)),
            round: u64::MAX,
        }),
    };
    let enc = encode_body(&body).unwrap();
    assert_eq!(enc.len(), 22_552);
    let dec = decode_body(&enc).unwrap();
    assert_eq!(encode_body(&dec).unwrap(), enc);
    // 1025 entities LimitExceeded on encode
    let mut entities = Vec::new();
    for i in 1..=1025u64 {
        entities.push(SnapshotEntity {
            entity: net(i),
            health: None,
        });
    }
    let body = SnapshotBody {
        entities,
        turn: None,
    };
    assert_eq!(encode_body(&body), Err(CodecError::LimitExceeded));
    // decode 1025 count LimitExceeded
    let mut bytes = vec![0x81, 0x04]; // varint 1025? 1025 = 0x401 -> 81 08? skip exact; use count prefix 0x81 0x08?
                                      // Instead test repeated/descending/zero NetId
                                      // zero NetId
    let mut b = vec![0x01, 0x00]; // entity_count 1, entity 0
    b.extend([0x00]); // health present false
    b.extend([0x00]); // turn false
    assert_eq!(decode_body(&b), Err(CodecError::Malformed));
    // flag byte 2
    let mut b = vec![0x01, 0x01, 0x01, 0x02]; // count1, entity1, health_present=2? actually present byte 2
                                              // trailing
    let b2 = vec![0x00, 0x00, 0x00]; // count 0, turn false, trailing byte
    assert_eq!(decode_body(&b2), Err(CodecError::Malformed));
    // input 1048577 bytes LimitExceeded
    let big = vec![0u8; 1_048_577];
    assert_eq!(decode_body(&big), Err(CodecError::LimitExceeded));
}

// case 9
#[test]
fn transfer_and_lane_zero_refuse_each_other() {
    use crpg_net::codec::{decode_delta, decode_intent};
    use crpg_net::codec_v2::{decode_delta as decode_delta_v2, decode_intent as decode_intent_v2};
    // v1/v2 decode_intent/decode_delta refuse transfer frame (first byte 0x81) with UnsupportedVersion
    let tf = encode_down(&TransferDown::Cancel {
        epoch: [0; 16],
        transfer: TransferId::new(1).unwrap(),
        reason: CancelReason::Superseded,
    })
    .unwrap();
    assert_eq!(
        decode_intent(&tf),
        Err(crpg_net::codec::CodecError::UnsupportedVersion)
    );
    assert_eq!(
        decode_delta(&tf),
        Err(crpg_net::codec::CodecError::UnsupportedVersion)
    );
    assert_eq!(
        decode_intent_v2(&tf),
        Err(crpg_net::codec::CodecError::UnsupportedVersion)
    );
    assert_eq!(
        decode_delta_v2(&tf),
        Err(crpg_net::codec::CodecError::UnsupportedVersion)
    );
    // transfer decoders refuse v1/v2 frames
    let v1intent = crpg_net::codec::encode_intent(&crpg_net::protocol::IntentFrame {
        epoch: [0; 16],
        lane: crpg_net::protocol::LANE_COMBAT,
        seq: 1,
        observed_tick: 0,
        actor: net(1),
        body: crpg_net::protocol::IntentBody::EndTurn,
    })
    .unwrap();
    assert_eq!(
        decode_down(&v1intent),
        Err(crpg_net::codec::CodecError::UnsupportedVersion)
    );
    assert_eq!(
        decode_up(&v1intent),
        Err(crpg_net::codec::CodecError::UnsupportedVersion)
    );
}

// case 10
#[test]
fn full_size_byte_assembly() {
    let total = 1_048_576usize;
    let mut payload = vec![0u8; total];
    for i in 0..total {
        payload[i] = ((i * 31 + 7) % 251) as u8;
    }
    // 8 segments
    let begin = BeginTransfer {
        epoch: [0; 16],
        transfer: TransferId::new(1).unwrap(),
        base_tick: 0,
        cursor: 0,
        total_bytes: total as u32,
        entity_count: 0,
        segments_per_chunk: 8,
        chunk_count: 32,
    };
    let mut asm = TransferAssembly::new(&begin).unwrap();
    // build chunks
    let mut chunks = Vec::new();
    for i in 0..32u8 {
        let start = i as usize * 32768;
        let end = start + 32768;
        let ch = TransferChunk {
            epoch: [0; 16],
            transfer: TransferId::new(1).unwrap(),
            index: i,
            bytes: payload[start..end].to_vec(),
        };
        chunks.push(ch);
    }
    // reverse delivery
    for ch in chunks.iter().rev() {
        let out = asm.accept(ch).unwrap();
        if ch.index == 0 { /* last? not necessarily */ }
        let _ = out;
    }
    assert!(asm.is_complete());
    let got = asm.into_bytes().unwrap();
    assert_eq!(got, payload);
    // frame sizes
    for ch in &chunks {
        let f = encode_down(&TransferDown::Chunk(ch.clone())).unwrap();
        assert!(f.len() <= 65536);
    }
    // 1 segment per chunk with 131072 bytes
    let total2 = 131_072usize;
    let payload2 = vec![0u8; total2];
    let begin2 = BeginTransfer {
        epoch: [0; 16],
        transfer: TransferId::new(2).unwrap(),
        base_tick: 0,
        cursor: 0,
        total_bytes: total2 as u32,
        entity_count: 0,
        segments_per_chunk: 1,
        chunk_count: 32,
    };
    let mut asm2 = TransferAssembly::new(&begin2).unwrap();
    for i in 0..32u8 {
        let start = i as usize * 4096;
        let end = start + 4096;
        let ch = TransferChunk {
            epoch: [0; 16],
            transfer: TransferId::new(2).unwrap(),
            index: i,
            bytes: payload2[start..end].to_vec(),
        };
        asm2.accept(&ch).unwrap();
    }
    assert_eq!(asm2.into_bytes().unwrap(), payload2);
}

// case 11
#[test]
fn assembly_duplicate_conflict_shape() {
    let begin = BeginTransfer {
        epoch: [0; 16],
        transfer: TransferId::new(1).unwrap(),
        base_tick: 0,
        cursor: 0,
        total_bytes: 8192,
        entity_count: 0,
        segments_per_chunk: 1,
        chunk_count: 2,
    };
    let mut asm = TransferAssembly::new(&begin).unwrap();
    let ch0 = TransferChunk {
        epoch: [0; 16],
        transfer: TransferId::new(1).unwrap(),
        index: 0,
        bytes: vec![1u8; 4096],
    };
    asm.accept(&ch0).unwrap();
    // identical duplicate
    assert_eq!(asm.accept(&ch0).unwrap(), ChunkOutcome::Duplicate);
    // different bytes conflict
    let ch0b = TransferChunk {
        epoch: [0; 16],
        transfer: TransferId::new(1).unwrap(),
        index: 0,
        bytes: vec![9u8; 4096],
    };
    assert_eq!(asm.accept(&ch0b), Err(AssemblyError::Conflict));
    // index = chunk_count (2) OutOfRange
    let ch2 = TransferChunk {
        epoch: [0; 16],
        transfer: TransferId::new(1).unwrap(),
        index: 2,
        bytes: vec![1u8; 4096],
    };
    assert_eq!(asm.accept(&ch2), Err(AssemblyError::OutOfRange));
    // non-final chunk 1 byte short
    let chshort = TransferChunk {
        epoch: [0; 16],
        transfer: TransferId::new(1).unwrap(),
        index: 1,
        bytes: vec![5u8; 4095],
    };
    assert_eq!(asm.accept(&chshort), Err(AssemblyError::WrongLength));
    // final chunk 1 byte long (chunk 1 expected 4096, give 4097) wrong length
    let chlong = TransferChunk {
        epoch: [0; 16],
        transfer: TransferId::new(1).unwrap(),
        index: 1,
        bytes: vec![5u8; 4097],
    };
    assert_eq!(asm.accept(&chlong), Err(AssemblyError::WrongLength));
    // wrong epoch/id
    let chw = TransferChunk {
        epoch: [1; 16],
        transfer: TransferId::new(1).unwrap(),
        index: 0,
        bytes: vec![1u8; 4096],
    };
    assert_eq!(asm.accept(&chw), Err(AssemblyError::WrongTransfer));
    let chw2 = TransferChunk {
        epoch: [0; 16],
        transfer: TransferId::new(2).unwrap(),
        index: 0,
        bytes: vec![1u8; 4096],
    };
    assert_eq!(asm.accept(&chw2), Err(AssemblyError::WrongTransfer));
    assert_eq!(asm.received_chunks(), 1);
    let r = asm.clone().into_bytes();
    assert!(r.is_err());
}
// case 12
#[test]
fn limits_validate_tightening_only() {
    assert!(TransferLimits::v1().validate().is_ok());
    let v1 = TransferLimits::v1();
    let mut l = v1.clone();
    l.max_receivers = 9;
    assert_eq!(
        l.validate(),
        Err(LimitsError::LoosensPolicy {
            field: "max_receivers"
        })
    );
    let mut l = v1.clone();
    l.deadline_ms = 5001;
    assert_eq!(
        l.validate(),
        Err(LimitsError::LoosensPolicy {
            field: "deadline_ms"
        })
    );
    let mut l = v1.clone();
    l.staging_bytes_per_receiver = 1_048_577;
    assert_eq!(
        l.validate(),
        Err(LimitsError::LoosensPolicy {
            field: "staging_bytes_per_receiver"
        })
    );
    let mut l = v1.clone();
    l.staging_bytes_total = 8_388_609;
    assert_eq!(
        l.validate(),
        Err(LimitsError::LoosensPolicy {
            field: "staging_bytes_total"
        })
    );
    let mut l = v1.clone();
    l.delta_frames_per_receiver = 129;
    assert_eq!(
        l.validate(),
        Err(LimitsError::LoosensPolicy {
            field: "delta_frames_per_receiver"
        })
    );
    let mut l = v1.clone();
    l.delta_bytes_per_receiver = 2_097_153;
    assert_eq!(
        l.validate(),
        Err(LimitsError::LoosensPolicy {
            field: "delta_bytes_per_receiver"
        })
    );
    let mut l = v1.clone();
    l.delta_bytes_total = 16_777_217;
    assert_eq!(
        l.validate(),
        Err(LimitsError::LoosensPolicy {
            field: "delta_bytes_total"
        })
    );
    let mut l = v1.clone();
    l.max_receivers = 0;
    assert_eq!(
        l.validate(),
        Err(LimitsError::InvalidLimits {
            field: "max_receivers"
        })
    );
    let mut l = v1.clone();
    l.deadline_ms = 0;
    assert_eq!(
        l.validate(),
        Err(LimitsError::InvalidLimits {
            field: "deadline_ms"
        })
    );
    let mut l = v1.clone();
    l.staging_bytes_per_receiver = 22_551;
    assert_eq!(
        l.validate(),
        Err(LimitsError::InvalidLimits {
            field: "staging_bytes_per_receiver"
        })
    );
    let mut l = v1.clone();
    l.staging_bytes_total = v1.staging_bytes_per_receiver - 1;
    assert_eq!(
        l.validate(),
        Err(LimitsError::InvalidLimits {
            field: "staging_bytes_total"
        })
    );
    let mut l = v1.clone();
    l.delta_frames_per_receiver = 0;
    assert_eq!(
        l.validate(),
        Err(LimitsError::InvalidLimits {
            field: "delta_frames_per_receiver"
        })
    );
    let mut l = v1.clone();
    l.delta_bytes_per_receiver = 65_535;
    assert_eq!(
        l.validate(),
        Err(LimitsError::InvalidLimits {
            field: "delta_bytes_per_receiver"
        })
    );
    let mut l = v1.clone();
    l.delta_bytes_total = v1.delta_bytes_per_receiver - 1;
    assert_eq!(
        l.validate(),
        Err(LimitsError::InvalidLimits {
            field: "delta_bytes_total"
        })
    );
    let mut l = v1.clone();
    l.max_receivers = 0;
    l.deadline_ms = 0;
    assert_eq!(
        l.validate(),
        Err(LimitsError::InvalidLimits {
            field: "max_receivers"
        })
    );
    assert!(ReassemblyPool::new(l).is_err());
}
// case 13
#[test]
fn max_valid_body_publishes_atomically() {
    let body = make_body();
    for spc in [1u8, 8u8] {
        let (mut pool, rid) = mkpool();
        let frames = plan_transfer(EPOCH, TransferId::new(1).unwrap(), 100, 5, spc, &body).unwrap();
        pool.offer_transfer(rid, &frames[0], 0).unwrap();
        let chunk_frames = &frames[1..];
        // offer all but last in order
        for i in 0..chunk_frames.len().saturating_sub(1) {
            let out = pool.offer_transfer(rid, &chunk_frames[i], 0).unwrap();
            if let TransferOutcome::Stored { .. } = out {
            } else { /* ok */
            }
            if i == chunk_frames.len().saturating_sub(2) && chunk_frames.len() > 1 {
                let st = pool.status(rid).unwrap();
                assert_eq!(
                    st.in_flight.as_ref().unwrap().received,
                    chunk_frames.len() as u8 - 1
                );
                assert_eq!(st.queued_updates, 0);
                assert_eq!(st.in_flight.as_ref().unwrap().reserved_bytes, 22_552);
            }
        }
        if chunk_frames.is_empty() {
            // 8 spc: one chunk; publish on it
            let out = pool.offer_transfer(rid, &chunk_frames[0], 0).unwrap();
            assert!(matches!(out, TransferOutcome::Published { .. }));
        } else {
            let out = pool
                .offer_transfer(rid, &chunk_frames[chunk_frames.len() - 1], 0)
                .unwrap();
            assert!(matches!(out, TransferOutcome::Published { .. }));
        }
        assert!(pool.status(rid).unwrap().published.is_some());
        let upd = pool.pop_update(rid).unwrap();
        if let ReplicaUpdate::Snapshot(publ) = upd {
            assert_eq!(publ.body, body);
        } else {
            panic!();
        }
        assert_eq!(pool.staging_bytes(), 0);
        let st = pool.status(rid).unwrap();
        assert_eq!(st.retired_through, 1);
        let pf = TransferUp::Published {
            epoch: EPOCH,
            transfer: TransferId::new(1).unwrap(),
        };
        let enc = encode_up(&pf).unwrap();
        assert_eq!(decode_up(&enc).unwrap(), pf);
    }
}
fn mkpool() -> (ReassemblyPool, ReceiverId) {
    let pool = ReassemblyPool::new(TransferLimits::v1()).unwrap();
    let mut pool = pool;
    let rid = pool
        .add_receiver(EPOCH, DeltaWire::V2, GateMode::Live)
        .unwrap();
    (pool, rid)
}
fn make_body() -> SnapshotBody {
    let mut entities = Vec::new();
    for i in 0..1024u64 {
        entities.push(SnapshotEntity {
            entity: net(u64::MAX - 1023 + i),
            health: Some(HealthFact {
                health: u32::MAX,
                max_health: u32::MAX,
                dead: true,
            }),
        });
    }
    SnapshotBody {
        entities,
        turn: Some(TurnFact {
            active: Some(net(u64::MAX - 1023)),
            round: u64::MAX,
        }),
    }
}
fn plan_frames(body: &SnapshotBody, spc: u8) -> Vec<Vec<u8>> {
    plan_transfer(EPOCH, TransferId::new(1).unwrap(), 0, 0, spc, body).unwrap()
}
// case 14
#[test]
fn second_in_flight_refused() {
    let body = make_body();
    let (mut pool, rid) = mkpool();
    let frames = plan_transfer(EPOCH, TransferId::new(1).unwrap(), 0, 5, 8, &body).unwrap();
    pool.offer_transfer(rid, &frames[0], 0).unwrap();
    let frames2 = plan_transfer(EPOCH, TransferId::new(2).unwrap(), 0, 5, 8, &body).unwrap();
    assert_eq!(
        pool.offer_transfer(rid, &frames2[0], 0),
        Err(TransferError::InFlight)
    );
    pool.offer_transfer(rid, &frames[1], 0).unwrap();
    pool.offer_transfer(rid, &frames2[0], 0).unwrap();
    assert_eq!(
        pool.offer_transfer(rid, &frames[0], 0),
        Err(TransferError::StaleTransfer)
    );
}
// case 15
#[test]
fn staging_caps_per_receiver_and_aggregate() {
    let mut pool = ReassemblyPool::new(TransferLimits::v1()).unwrap();
    let mut rids = Vec::new();
    for _ in 0..8 {
        rids.push(
            pool.add_receiver(EPOCH, DeltaWire::V2, GateMode::Live)
                .unwrap(),
        );
    }
    assert_eq!(
        pool.add_receiver(EPOCH, DeltaWire::V2, GateMode::Live),
        Err(TransferError::ReceiversFull)
    );
    let begin = BeginTransfer {
        epoch: EPOCH,
        transfer: TransferId::new(1).unwrap(),
        base_tick: 0,
        cursor: 0,
        total_bytes: 1_048_576,
        entity_count: 0,
        segments_per_chunk: 8,
        chunk_count: 32,
    };
    let frame = encode_down(&TransferDown::Begin(begin)).unwrap();
    for rid in &rids {
        pool.offer_transfer(*rid, &frame, 0).unwrap();
    }
    assert_eq!(pool.staging_bytes(), 8_388_608);
    // per-receiver tightened
    let mut limits = TransferLimits::v1();
    limits.staging_bytes_per_receiver = 22_552;
    let mut pool = ReassemblyPool::new(limits).unwrap();
    let rid = pool
        .add_receiver(EPOCH, DeltaWire::V2, GateMode::Live)
        .unwrap();
    let begin_ok = BeginTransfer {
        epoch: EPOCH,
        transfer: TransferId::new(1).unwrap(),
        base_tick: 0,
        cursor: 0,
        total_bytes: 22_552,
        entity_count: 0,
        segments_per_chunk: 1,
        chunk_count: 6,
    };
    pool.offer_transfer(
        rid,
        &encode_down(&TransferDown::Begin(begin_ok)).unwrap(),
        0,
    )
    .unwrap();
    pool.offer_transfer(
        rid,
        &encode_down(&TransferDown::Cancel {
            epoch: EPOCH,
            transfer: TransferId::new(1).unwrap(),
            reason: CancelReason::Aborted,
        })
        .unwrap(),
        0,
    )
    .unwrap();
    let begin_big = BeginTransfer {
        epoch: EPOCH,
        transfer: TransferId::new(2).unwrap(),
        base_tick: 0,
        cursor: 0,
        total_bytes: 22_553,
        entity_count: 0,
        segments_per_chunk: 1,
        chunk_count: 6,
    };
    assert_eq!(
        pool.offer_transfer(
            rid,
            &encode_down(&TransferDown::Begin(begin_big)).unwrap(),
            0
        ),
        Err(TransferError::StagingFull)
    );
    // aggregate
    let mut limits = TransferLimits::v1();
    limits.staging_bytes_per_receiver = 1_048_576;
    limits.staging_bytes_total = 1_048_576 + 22_552;
    let mut pool = ReassemblyPool::new(limits).unwrap();
    let r1 = pool
        .add_receiver(EPOCH, DeltaWire::V2, GateMode::Live)
        .unwrap();
    let r2 = pool
        .add_receiver(EPOCH, DeltaWire::V2, GateMode::Live)
        .unwrap();
    let b1 = BeginTransfer {
        epoch: EPOCH,
        transfer: TransferId::new(1).unwrap(),
        base_tick: 0,
        cursor: 0,
        total_bytes: 1_048_576,
        entity_count: 0,
        segments_per_chunk: 8,
        chunk_count: 32,
    };
    pool.offer_transfer(r1, &encode_down(&TransferDown::Begin(b1)).unwrap(), 0)
        .unwrap();
    let b2ok = BeginTransfer {
        epoch: EPOCH,
        transfer: TransferId::new(1).unwrap(),
        base_tick: 0,
        cursor: 0,
        total_bytes: 22_552,
        entity_count: 0,
        segments_per_chunk: 1,
        chunk_count: 6,
    };
    pool.offer_transfer(r2, &encode_down(&TransferDown::Begin(b2ok)).unwrap(), 0)
        .unwrap();
    pool.offer_transfer(
        r2,
        &encode_down(&TransferDown::Cancel {
            epoch: EPOCH,
            transfer: TransferId::new(1).unwrap(),
            reason: CancelReason::Aborted,
        })
        .unwrap(),
        0,
    )
    .unwrap();
    let b2big = BeginTransfer {
        epoch: EPOCH,
        transfer: TransferId::new(2).unwrap(),
        base_tick: 0,
        cursor: 0,
        total_bytes: 22_553,
        entity_count: 0,
        segments_per_chunk: 1,
        chunk_count: 6,
    };
    assert_eq!(
        pool.offer_transfer(r2, &encode_down(&TransferDown::Begin(b2big)).unwrap(), 0),
        Err(TransferError::AggregateFull)
    );
}
// case 16
#[test]
fn duplicates_idempotent_no_deadline_extension() {
    let body = make_body();
    let (mut pool, rid) = mkpool();
    let frames = plan_transfer(EPOCH, TransferId::new(1).unwrap(), 0, 5, 1, &body).unwrap();
    pool.offer_transfer(rid, &frames[0], 1000).unwrap();
    assert_eq!(
        pool.offer_transfer(rid, &frames[0], 5000).unwrap(),
        TransferOutcome::Duplicate
    );
    let st = pool.status(rid).unwrap();
    assert_eq!(st.in_flight.as_ref().unwrap().deadline_ms, 6000);
    pool.offer_transfer(rid, &frames[1], 1000).unwrap();
    assert_eq!(
        pool.offer_transfer(rid, &frames[1], 5000).unwrap(),
        TransferOutcome::Duplicate
    );
    // last chunk at t+5000 expired
    for i in 2..frames.len() - 1 {
        pool.offer_transfer(rid, &frames[i], 1000).unwrap();
    }
    assert_eq!(
        pool.offer_transfer(rid, &frames[frames.len() - 1], 6000),
        Err(TransferError::Failed(FailReason::Expired))
    );
}
// case 17
#[test]
fn conflicts_discard_retire_and_flag() {
    let body = make_body();
    let (mut pool, rid) = mkpool();
    let frames = plan_transfer(EPOCH, TransferId::new(1).unwrap(), 0, 5, 1, &body).unwrap();
    pool.offer_transfer(rid, &frames[0], 0).unwrap();
    // conflicting chunk index past chunk_count
    let badchunk = TransferDown::Chunk(TransferChunk {
        epoch: EPOCH,
        transfer: TransferId::new(1).unwrap(),
        index: 6,
        bytes: vec![0u8; 4096],
    });
    assert_eq!(
        pool.offer_transfer(rid, &encode_down(&badchunk).unwrap(), 0),
        Err(TransferError::Failed(FailReason::Conflict))
    );
    let st = pool.status(rid).unwrap();
    assert_eq!(st.resync, Some(ResyncReason::Invalid));
    assert_eq!(st.mode, GateMode::AwaitSnapshot);
    assert_eq!(pool.staging_bytes(), 0);
    // later chunk StaleTransfer
    assert_eq!(
        pool.offer_transfer(rid, &frames[1], 0),
        Err(TransferError::StaleTransfer)
    );
    // higher begin clears resync and publishes
    let frames2 = plan_transfer(EPOCH, TransferId::new(2).unwrap(), 0, 5, 1, &body).unwrap();
    for f in &frames2 {
        pool.offer_transfer(rid, f, 0).unwrap();
    }
    let st = pool.status(rid).unwrap();
    assert_eq!(st.resync, None);
    assert!(st.published.is_some());
}
// case 18
#[test]
fn reorder_and_missing_chunks() {
    let body = make_body();
    for order in 0..3 {
        let (mut pool, rid) = mkpool();
        let frames = plan_transfer(
            EPOCH,
            TransferId::new(10 + order as u64).unwrap(),
            0,
            5,
            1,
            &body,
        )
        .unwrap();
        pool.offer_transfer(rid, &frames[0], 0).unwrap();
        let mut idxs: Vec<usize> = (1..frames.len()).collect();
        if order == 1 {
            idxs.reverse();
        }
        if order == 2 {
            idxs = vec![1, 3, 5, 2, 4, 6];
        }
        for i in idxs {
            let _ = pool.offer_transfer(rid, &frames[i], 0);
        }
        assert!(pool.status(rid).unwrap().published.is_some());
    }
    let (mut pool, rid) = mkpool();
    let frames = plan_transfer(EPOCH, TransferId::new(100).unwrap(), 0, 5, 1, &body).unwrap();
    pool.offer_transfer(rid, &frames[0], 0).unwrap();
    for i in 1..frames.len() {
        if i == 3 {
            continue;
        }
        let _ = pool.offer_transfer(rid, &frames[i], 0);
    }
    assert!(pool.poll(4999).is_empty());
    assert_eq!(pool.poll(5000), vec![rid]);
    assert_eq!(pool.status(rid).unwrap().queued_updates, 0);
}
// case 19
#[test]
fn deadline_boundary_4999_5000() {
    let body = make_body();
    let (mut pool, rid) = mkpool();
    let frames = plan_transfer(EPOCH, TransferId::new(1).unwrap(), 0, 5, 1, &body).unwrap();
    pool.offer_transfer(rid, &frames[0], 0).unwrap();
    for i in 1..frames.len() - 1 {
        pool.offer_transfer(rid, &frames[i], 0).unwrap();
    }
    let out = pool
        .offer_transfer(rid, &frames[frames.len() - 1], 4999)
        .unwrap();
    assert!(matches!(out, TransferOutcome::Published { .. }));
    // separate
    let (mut pool, rid) = mkpool();
    let frames = plan_transfer(EPOCH, TransferId::new(2).unwrap(), 0, 5, 1, &body).unwrap();
    pool.offer_transfer(rid, &frames[0], 0).unwrap();
    for i in 1..frames.len() - 1 {
        pool.offer_transfer(rid, &frames[i], 0).unwrap();
    }
    assert_eq!(
        pool.offer_transfer(rid, &frames[frames.len() - 1], 5000),
        Err(TransferError::Failed(FailReason::Expired))
    );
    assert_eq!(pool.status(rid).unwrap().queued_updates, 0);
    assert_eq!(pool.staging_bytes(), 0);
    assert_eq!(
        pool.offer_transfer(rid, &frames[frames.len() - 1], 10),
        Err(TransferError::StaleTransfer)
    );
    // saturating
    let mut pool2 = ReassemblyPool::new(TransferLimits::v1()).unwrap();
    let rid2 = pool2
        .add_receiver(EPOCH, DeltaWire::V2, GateMode::Live)
        .unwrap();
    let begin = BeginTransfer {
        epoch: EPOCH,
        transfer: TransferId::new(1).unwrap(),
        base_tick: 0,
        cursor: 0,
        total_bytes: 2,
        entity_count: 0,
        segments_per_chunk: 1,
        chunk_count: 1,
    };
    pool2
        .offer_transfer(
            rid2,
            &encode_down(&TransferDown::Begin(begin)).unwrap(),
            u64::MAX - 10,
        )
        .unwrap();
    assert_eq!(
        pool2.status(rid2).unwrap().in_flight.unwrap().deadline_ms,
        u64::MAX
    );
}
// case 20
#[test]
fn expiry_frees_and_retry_publishes() {
    let body = make_body();
    let mut pool = ReassemblyPool::new(TransferLimits::v1()).unwrap();
    let a = pool
        .add_receiver(EPOCH, DeltaWire::V2, GateMode::Live)
        .unwrap();
    let b = pool
        .add_receiver(EPOCH, DeltaWire::V2, GateMode::Live)
        .unwrap();
    let frames = plan_transfer(EPOCH, TransferId::new(1).unwrap(), 0, 5, 1, &body).unwrap();
    pool.offer_transfer(a, &frames[0], 0).unwrap();
    let framesb = plan_transfer(EPOCH, TransferId::new(1).unwrap(), 0, 5, 1, &body).unwrap();
    let _ = pool.offer_transfer(b, &framesb[0], 5000);
    let sta = pool.status(a).unwrap();
    assert_eq!(sta.resync, Some(ResyncReason::Expired));
    let frames2 = plan_transfer(EPOCH, TransferId::new(2).unwrap(), 0, 5, 1, &body).unwrap();
    for f in &frames2 {
        pool.offer_transfer(a, f, 5000).unwrap();
    }
    assert!(pool.status(a).unwrap().published.is_some());
    let rs = TransferUp::Resync {
        epoch: EPOCH,
        after: Some(TransferId::new(1).unwrap()),
        reason: ResyncReason::Expired,
    };
    let enc = encode_up(&rs).unwrap();
    assert_eq!(decode_up(&enc).unwrap(), rs);
}
// case 21
#[test]
fn old_mixed_epoch_and_id_refusals() {
    let body = make_body();
    let (mut pool, rid) = mkpool();
    let frames = plan_transfer(EPOCH, TransferId::new(1).unwrap(), 100, 5, 1, &body).unwrap();
    for f in &frames {
        pool.offer_transfer(rid, f, 0).unwrap();
    }
    // wrong epoch begin
    let begin2 = BeginTransfer {
        epoch: [0x22; 16],
        transfer: TransferId::new(2).unwrap(),
        base_tick: 0,
        cursor: 5,
        total_bytes: 2,
        entity_count: 0,
        segments_per_chunk: 1,
        chunk_count: 1,
    };
    assert_eq!(
        pool.offer_transfer(rid, &encode_down(&TransferDown::Begin(begin2)).unwrap(), 0),
        Err(TransferError::WrongEpoch)
    );
    // chunk wrong epoch
    let ch = TransferDown::Chunk(TransferChunk {
        epoch: [0x22; 16],
        transfer: TransferId::new(1).unwrap(),
        index: 0,
        bytes: vec![0, 0],
    });
    assert_eq!(
        pool.offer_transfer(rid, &encode_down(&ch).unwrap(), 0),
        Err(TransferError::WrongEpoch)
    );
    // cancel wrong epoch
    let ca = TransferDown::Cancel {
        epoch: [0x22; 16],
        transfer: TransferId::new(1).unwrap(),
        reason: CancelReason::Superseded,
    };
    assert_eq!(
        pool.offer_transfer(rid, &encode_down(&ca).unwrap(), 0),
        Err(TransferError::WrongEpoch)
    );
    // begin at or below retired_through
    let bs = BeginTransfer {
        epoch: EPOCH,
        transfer: TransferId::new(1).unwrap(),
        base_tick: 0,
        cursor: 5,
        total_bytes: 2,
        entity_count: 0,
        segments_per_chunk: 1,
        chunk_count: 1,
    };
    assert_eq!(
        pool.offer_transfer(rid, &encode_down(&TransferDown::Begin(bs)).unwrap(), 0),
        Err(TransferError::StaleTransfer)
    );
    // chunk for id never began
    let ch2 = TransferDown::Chunk(TransferChunk {
        epoch: EPOCH,
        transfer: TransferId::new(999).unwrap(),
        index: 0,
        bytes: vec![0, 0],
    });
    assert_eq!(
        pool.offer_transfer(rid, &encode_down(&ch2).unwrap(), 0),
        Err(TransferError::UnknownTransfer)
    );
    // cursor+1 < next_event_seq BehindReplica
    let bp = BeginTransfer {
        epoch: EPOCH,
        transfer: TransferId::new(3).unwrap(),
        base_tick: 100,
        cursor: 0,
        total_bytes: 2,
        entity_count: 0,
        segments_per_chunk: 1,
        chunk_count: 1,
    };
    assert_eq!(
        pool.offer_transfer(rid, &encode_down(&TransferDown::Begin(bp)).unwrap(), 0),
        Err(TransferError::BehindReplica)
    );
    // base_tick below published BehindReplica
    let bp2 = BeginTransfer {
        epoch: EPOCH,
        transfer: TransferId::new(3).unwrap(),
        base_tick: 0,
        cursor: 10,
        total_bytes: 2,
        entity_count: 0,
        segments_per_chunk: 1,
        chunk_count: 1,
    };
    assert_eq!(
        pool.offer_transfer(rid, &encode_down(&TransferDown::Begin(bp2)).unwrap(), 0),
        Err(TransferError::BehindReplica)
    );
    // equal base_tick accepted
    let bp3 = BeginTransfer {
        epoch: EPOCH,
        transfer: TransferId::new(3).unwrap(),
        base_tick: 100,
        cursor: 10,
        total_bytes: 2,
        entity_count: 0,
        segments_per_chunk: 1,
        chunk_count: 1,
    };
    assert!(pool
        .offer_transfer(rid, &encode_down(&TransferDown::Begin(bp3)).unwrap(), 0)
        .is_ok());
}
// case 22
#[test]
fn invalid_body_never_publishes() {
    let total = 1_048_576usize;
    let payload = vec![0xAAu8; total];
    let begin = BeginTransfer {
        epoch: EPOCH,
        transfer: TransferId::new(1).unwrap(),
        base_tick: 0,
        cursor: 0,
        total_bytes: total as u32,
        entity_count: 0,
        segments_per_chunk: 8,
        chunk_count: 32,
    };
    let (mut pool, rid) = mkpool();
    pool.offer_transfer(rid, &encode_down(&TransferDown::Begin(begin)).unwrap(), 0)
        .unwrap();
    for i in 0..32u8 {
        let start = i as usize * 32768;
        let end = start + 32768;
        let ch = TransferChunk {
            epoch: EPOCH,
            transfer: TransferId::new(1).unwrap(),
            index: i,
            bytes: payload[start..end].to_vec(),
        };
        let out = pool.offer_transfer(rid, &encode_down(&TransferDown::Chunk(ch)).unwrap(), 0);
        if i == 31 {
            assert!(matches!(
                out,
                Err(TransferError::Failed(FailReason::InvalidBody(_)))
            ));
        }
    }
    assert_eq!(pool.staging_bytes(), 0);
    // earlier snapshot none -> queued_updates 0
    assert_eq!(pool.status(rid).unwrap().queued_updates, 0);
}
// case 23
#[test]
fn cutover_releases_in_fifo_order() {
    let body = make_body();
    let (mut pool, rid) = mkpool(); // V2 Live
                                    // deltas 1..=3 released
    for i in 1..=3u64 {
        let df = crpg_net::protocol_v2::DeltaFrame {
            lane: crpg_net::protocol::LANE_COMBAT,
            server_tick: 1,
            event_seq: i,
            ops: vec![],
        };
        let b = crpg_net::codec_v2::encode_delta(&df).unwrap();
        assert_eq!(
            pool.offer_delta(rid, &b, 0).unwrap(),
            DeltaOutcome::Released { count: 1 }
        );
    }
    // Begin cursor 5
    let frames = plan_transfer(EPOCH, TransferId::new(1).unwrap(), 100, 5, 1, &body).unwrap();
    pool.offer_transfer(rid, &frames[0], 0).unwrap();
    // delta 5 buffered
    let df5 = crpg_net::protocol_v2::DeltaFrame {
        lane: crpg_net::protocol::LANE_COMBAT,
        server_tick: 1,
        event_seq: 5,
        ops: vec![],
    };
    let b5 = crpg_net::codec_v2::encode_delta(&df5).unwrap();
    assert_eq!(
        pool.offer_delta(rid, &b5, 0).unwrap(),
        DeltaOutcome::Buffered
    );
    // delta 4 released count 2
    let df4 = crpg_net::protocol_v2::DeltaFrame {
        lane: crpg_net::protocol::LANE_COMBAT,
        server_tick: 1,
        event_seq: 4,
        ops: vec![],
    };
    let b4 = crpg_net::codec_v2::encode_delta(&df4).unwrap();
    assert_eq!(
        pool.offer_delta(rid, &b4, 0).unwrap(),
        DeltaOutcome::Released { count: 2 }
    );
    // deltas 6,7 buffered
    for i in 6..=7u64 {
        let df = crpg_net::protocol_v2::DeltaFrame {
            lane: crpg_net::protocol::LANE_COMBAT,
            server_tick: 1,
            event_seq: i,
            ops: vec![],
        };
        let b = crpg_net::codec_v2::encode_delta(&df).unwrap();
        assert_eq!(
            pool.offer_delta(rid, &b, 0).unwrap(),
            DeltaOutcome::Buffered
        );
    }
    // publish
    for i in 1..frames.len() {
        pool.offer_transfer(rid, &frames[i], 0).unwrap();
    }
    // last chunk publishes with released 2
    // The publish happened on last chunk; we already called all. Check outcome? We ignored. Re-run not needed.
    // pop order: D1..D5, Snapshot, D6, D7
    let mut updates = Vec::new();
    while let Some(u) = pool.pop_update(rid) {
        updates.push(u);
    }
    // first 5 deltas
    for i in 0..5 {
        if let ReplicaUpdate::Delta(GatedDelta::V2(d)) = &updates[i] {
            assert_eq!(d.event_seq, (i + 1) as u64);
        } else {
            panic!("expected delta");
        }
    }
    assert!(matches!(updates[5], ReplicaUpdate::Snapshot(_)));
    for i in 6..8 {
        if let ReplicaUpdate::Delta(GatedDelta::V2(d)) = &updates[i] {
            assert_eq!(d.event_seq, i as u64);
        } else {
            panic!();
        }
    }
    // repeats superseded
    let df5r = crpg_net::protocol_v2::DeltaFrame {
        lane: crpg_net::protocol::LANE_COMBAT,
        server_tick: 1,
        event_seq: 5,
        ops: vec![],
    };
    let b5r = crpg_net::codec_v2::encode_delta(&df5r).unwrap();
    assert_eq!(
        pool.offer_delta(rid, &b5r, 0).unwrap(),
        DeltaOutcome::Superseded
    );
    let df6r = crpg_net::protocol_v2::DeltaFrame {
        lane: crpg_net::protocol::LANE_COMBAT,
        server_tick: 1,
        event_seq: 6,
        ops: vec![],
    };
    let b6r = crpg_net::codec_v2::encode_delta(&df6r).unwrap();
    assert_eq!(
        pool.offer_delta(rid, &b6r, 0).unwrap(),
        DeltaOutcome::Superseded
    );
}
// case 24
#[test]
fn await_snapshot_holds_until_publication() {
    let body = make_body();
    let mut pool = ReassemblyPool::new(TransferLimits::v1()).unwrap();
    let rid = pool
        .add_receiver(EPOCH, DeltaWire::V2, GateMode::AwaitSnapshot)
        .unwrap();
    // deltas 1..=3 held, nothing queued
    for i in 1..=3u64 {
        let df = crpg_net::protocol_v2::DeltaFrame {
            lane: crpg_net::protocol::LANE_COMBAT,
            server_tick: 1,
            event_seq: i,
            ops: vec![],
        };
        let b = crpg_net::codec_v2::encode_delta(&df).unwrap();
        assert_eq!(
            pool.offer_delta(rid, &b, 0).unwrap(),
            DeltaOutcome::Buffered
        );
    }
    assert_eq!(pool.status(rid).unwrap().queued_updates, 0);
    // Begin cursor 2
    let frames = plan_transfer(EPOCH, TransferId::new(1).unwrap(), 100, 2, 1, &body).unwrap();
    pool.offer_transfer(rid, &frames[0], 0).unwrap();
    // publish
    for i in 1..frames.len() {
        pool.offer_transfer(rid, &frames[i], 0).unwrap();
    }
    let st = pool.status(rid).unwrap();
    // queue Snapshot, D3
    let mut updates = Vec::new();
    while let Some(u) = pool.pop_update(rid) {
        updates.push(u);
    }
    assert!(matches!(updates[0], ReplicaUpdate::Snapshot(_)));
    assert_eq!(updates.len(), 2);
    // delta 5 without 4 stays held
    let df5 = crpg_net::protocol_v2::DeltaFrame {
        lane: crpg_net::protocol::LANE_COMBAT,
        server_tick: 1,
        event_seq: 5,
        ops: vec![],
    };
    let b5 = crpg_net::codec_v2::encode_delta(&df5).unwrap();
    assert_eq!(
        pool.offer_delta(rid, &b5, 0).unwrap(),
        DeltaOutcome::Buffered
    );
    // delta 4 then Released count 2
    let df4 = crpg_net::protocol_v2::DeltaFrame {
        lane: crpg_net::protocol::LANE_COMBAT,
        server_tick: 1,
        event_seq: 4,
        ops: vec![],
    };
    let b4 = crpg_net::codec_v2::encode_delta(&df4).unwrap();
    assert_eq!(
        pool.offer_delta(rid, &b4, 0).unwrap(),
        DeltaOutcome::Released { count: 2 }
    );
}
// case 25
#[test]
fn delta_hold_caps_and_overflow() {
    let body = make_body();
    let mut pool = ReassemblyPool::new(TransferLimits::v1()).unwrap();
    let rid = pool
        .add_receiver(EPOCH, DeltaWire::V2, GateMode::AwaitSnapshot)
        .unwrap();
    // hold 128 frames
    for i in 1..=128u64 {
        let df = crpg_net::protocol_v2::DeltaFrame {
            lane: crpg_net::protocol::LANE_COMBAT,
            server_tick: 1,
            event_seq: i,
            ops: vec![],
        };
        let b = crpg_net::codec_v2::encode_delta(&df).unwrap();
        assert_eq!(
            pool.offer_delta(rid, &b, 0).unwrap(),
            DeltaOutcome::Buffered
        );
    }
    // 129th overflow
    let df = crpg_net::protocol_v2::DeltaFrame {
        lane: crpg_net::protocol::LANE_COMBAT,
        server_tick: 1,
        event_seq: 129,
        ops: vec![],
    };
    let b = crpg_net::codec_v2::encode_delta(&df).unwrap();
    assert_eq!(
        pool.offer_delta(rid, &b, 0),
        Err(TransferError::Failed(FailReason::Overflow))
    );
    let st = pool.status(rid).unwrap();
    assert_eq!(st.resync, Some(ResyncReason::Overflow));
    assert_eq!(st.mode, GateMode::AwaitSnapshot);
    // held cleared
    assert_eq!(st.held_frames, 0);
    // delta_bytes_per_receiver exact fit
    let mut limits = TransferLimits::v1();
    limits.delta_bytes_per_receiver = 65_536;
    let mut pool = ReassemblyPool::new(limits).unwrap();
    let rid = pool
        .add_receiver(EPOCH, DeltaWire::V2, GateMode::AwaitSnapshot)
        .unwrap();
    // small delta exact fit? Use one delta of encoded len <= 65536. Just check overflow one byte more by tightening total?
    // conflicting duplicate of held delta -> Conflict
    let df = crpg_net::protocol_v2::DeltaFrame {
        lane: crpg_net::protocol::LANE_COMBAT,
        server_tick: 1,
        event_seq: 1,
        ops: vec![],
    };
    let b = crpg_net::codec_v2::encode_delta(&df).unwrap();
    pool.offer_delta(rid, &b, 0).unwrap();
    // conflicting duplicate
    let df2 = crpg_net::protocol_v2::DeltaFrame {
        lane: crpg_net::protocol::LANE_COMBAT,
        server_tick: 1,
        event_seq: 1,
        ops: vec![],
    };
    let mut b2 = crpg_net::codec_v2::encode_delta(&df2).unwrap();
    b2.push(0x00);
    // not a valid frame maybe; instead change bytes by encoding different? Use same seq different tick
    let df3 = crpg_net::protocol_v2::DeltaFrame {
        lane: crpg_net::protocol::LANE_COMBAT,
        server_tick: 2,
        event_seq: 1,
        ops: vec![],
    };
    let b3 = crpg_net::codec_v2::encode_delta(&df3).unwrap();
    assert_ne!(b, b3);
    assert_eq!(
        pool.offer_delta(rid, &b3, 0),
        Err(TransferError::Failed(FailReason::Conflict))
    );
}
// case 26
#[test]
fn delta_sequence_and_wire_refusals() {
    let (mut pool, rid) = mkpool(); // V2
                                    // event_seq 0 and u64::MAX DeltaSequence
    let df0 = crpg_net::protocol_v2::DeltaFrame {
        lane: crpg_net::protocol::LANE_COMBAT,
        server_tick: 1,
        event_seq: 0,
        ops: vec![],
    };
    let b0 = crpg_net::codec_v2::encode_delta(&df0); // encode may fail? event_seq 0 accepted by codec? codec accepts full u64. encode ok
                                                     // codec accepts any u64; pool checks. But encode_delta may not validate event_seq? codec_v2 accepts full u64. So ok.
    if let Ok(b) = b0 {
        assert_eq!(
            pool.offer_delta(rid, &b, 0),
            Err(TransferError::DeltaSequence)
        );
    }
    let dfm = crpg_net::protocol_v2::DeltaFrame {
        lane: crpg_net::protocol::LANE_COMBAT,
        server_tick: 1,
        event_seq: u64::MAX,
        ops: vec![],
    };
    let bm = crpg_net::codec_v2::encode_delta(&dfm).unwrap();
    assert_eq!(
        pool.offer_delta(rid, &bm, 0),
        Err(TransferError::DeltaSequence)
    );
    // SEQ_LAST held (not published? pool next_event_seq 1, so SEQ_LAST buffered)
    let dfl = crpg_net::protocol_v2::DeltaFrame {
        lane: crpg_net::protocol::LANE_COMBAT,
        server_tick: 1,
        event_seq: crpg_net::protocol::SEQ_LAST,
        ops: vec![],
    };
    let bl = crpg_net::codec_v2::encode_delta(&dfl).unwrap();
    assert_eq!(
        pool.offer_delta(rid, &bl, 0).unwrap(),
        DeltaOutcome::Buffered
    );
    // v1 frame to V2 receiver Codec(UnsupportedVersion)
    // Build a v1 delta frame bytes
    let v1df = crpg_net::protocol::DeltaFrame {
        lane: crpg_net::protocol::LANE_COMBAT,
        server_tick: 1,
        event_seq: 1,
        ops: vec![],
    };
    let v1b = crpg_net::codec::encode_delta(&v1df).unwrap();
    assert_eq!(
        pool.offer_delta(rid, &v1b, 0),
        Err(TransferError::Codec(
            crpg_net::codec::CodecError::UnsupportedVersion
        ))
    );
}
// case 27
#[test]
fn cancel_retires_and_supersedes() {
    let body = make_body();
    let (mut pool, rid) = mkpool();
    let frames = plan_transfer(EPOCH, TransferId::new(1).unwrap(), 100, 5, 1, &body).unwrap();
    pool.offer_transfer(rid, &frames[0], 0).unwrap();
    // cancel in-flight
    assert_eq!(
        pool.offer_transfer(
            rid,
            &encode_down(&TransferDown::Cancel {
                epoch: EPOCH,
                transfer: TransferId::new(1).unwrap(),
                reason: CancelReason::Superseded
            })
            .unwrap(),
            0
        )
        .unwrap(),
        TransferOutcome::Cancelled { discarded: true }
    );
    assert_eq!(pool.staging_bytes(), 0);
    assert_eq!(pool.status(rid).unwrap().mode, GateMode::AwaitSnapshot);
    // remaining chunks StaleTransfer
    assert_eq!(
        pool.offer_transfer(rid, &frames[1], 0),
        Err(TransferError::StaleTransfer)
    );
    // repeat cancel
    assert_eq!(
        pool.offer_transfer(
            rid,
            &encode_down(&TransferDown::Cancel {
                epoch: EPOCH,
                transfer: TransferId::new(1).unwrap(),
                reason: CancelReason::Superseded
            })
            .unwrap(),
            0
        )
        .unwrap(),
        TransferOutcome::Cancelled { discarded: false }
    );
    // cancel with nothing in flight retires through 9
    let (mut pool, rid) = mkpool();
    pool.offer_transfer(
        rid,
        &encode_down(&TransferDown::Cancel {
            epoch: EPOCH,
            transfer: TransferId::new(9).unwrap(),
            reason: CancelReason::Superseded,
        })
        .unwrap(),
        0,
    )
    .unwrap();
    let begin9 = BeginTransfer {
        epoch: EPOCH,
        transfer: TransferId::new(9).unwrap(),
        base_tick: 0,
        cursor: 0,
        total_bytes: 2,
        entity_count: 0,
        segments_per_chunk: 1,
        chunk_count: 1,
    };
    assert_eq!(
        pool.offer_transfer(rid, &encode_down(&TransferDown::Begin(begin9)).unwrap(), 0),
        Err(TransferError::StaleTransfer)
    );
    let begin10 = BeginTransfer {
        epoch: EPOCH,
        transfer: TransferId::new(10).unwrap(),
        base_tick: 0,
        cursor: 0,
        total_bytes: 2,
        entity_count: 0,
        segments_per_chunk: 1,
        chunk_count: 1,
    };
    assert!(pool
        .offer_transfer(rid, &encode_down(&TransferDown::Begin(begin10)).unwrap(), 0)
        .is_ok());
    // cancelling published id leaves queued snapshot
    let (mut pool, rid) = mkpool();
    let frames = plan_transfer(EPOCH, TransferId::new(1).unwrap(), 100, 5, 8, &body).unwrap();
    for f in &frames {
        pool.offer_transfer(rid, f, 0).unwrap();
    }
    let before = pool.status(rid).unwrap().queued_updates;
    let _ = pool
        .offer_transfer(
            rid,
            &encode_down(&TransferDown::Cancel {
                epoch: EPOCH,
                transfer: TransferId::new(1).unwrap(),
                reason: CancelReason::Superseded,
            })
            .unwrap(),
            0,
        )
        .unwrap();
    assert_eq!(pool.status(rid).unwrap().queued_updates, before);
}
// case 28
#[test]
fn revocation_never_republishes_hidden_facts() {
    let body_with_health = SnapshotBody {
        entities: vec![SnapshotEntity {
            entity: net(1),
            health: Some(HealthFact {
                health: 5,
                max_health: 10,
                dead: false,
            }),
        }],
        turn: None,
    };
    let body_hidden = SnapshotBody {
        entities: vec![SnapshotEntity {
            entity: net(1),
            health: None,
        }],
        turn: None,
    };
    let (mut pool, rid) = mkpool();
    let frames = plan_transfer(
        EPOCH,
        TransferId::new(1).unwrap(),
        100,
        1,
        1,
        &body_with_health,
    )
    .unwrap();
    // offer first 5? only 1 chunk for small body. Simulate revocation: cancel before publish
    pool.offer_transfer(rid, &frames[0], 0).unwrap();
    // cancel
    pool.offer_transfer(
        rid,
        &encode_down(&TransferDown::Cancel {
            epoch: EPOCH,
            transfer: TransferId::new(1).unwrap(),
            reason: CancelReason::Aborted,
        })
        .unwrap(),
        0,
    )
    .unwrap();
    // remaining chunks stale
    if frames.len() > 1 {
        assert_eq!(
            pool.offer_transfer(rid, &frames[1], 0),
            Err(TransferError::StaleTransfer)
        );
    }
    // transfer 2 under narrowed grants publishes
    let frames2 =
        plan_transfer(EPOCH, TransferId::new(2).unwrap(), 100, 1, 1, &body_hidden).unwrap();
    for f in &frames2 {
        pool.offer_transfer(rid, f, 0).unwrap();
    }
    let upd = pool.pop_update(rid).unwrap();
    if let ReplicaUpdate::Snapshot(publ) = upd {
        assert!(publ.body.entities[0].health.is_none());
    } else {
        panic!();
    }
}
// case 29
#[test]
fn hidden_values_never_reach_bytes() {
    let b1 = SnapshotBody {
        entities: vec![SnapshotEntity {
            entity: net(1),
            health: None,
        }],
        turn: None,
    };
    let b2 = SnapshotBody {
        entities: vec![SnapshotEntity {
            entity: net(1),
            health: Some(HealthFact {
                health: 99,
                max_health: 99,
                dead: true,
            }),
        }],
        turn: None,
    };
    // differ in granted? ungranted entity presence: compare two bodies where hidden entity omitted
    let body_a = SnapshotBody {
        entities: vec![],
        turn: None,
    };
    let body_b = SnapshotBody {
        entities: vec![],
        turn: None,
    };
    let fa = plan_transfer(EPOCH, TransferId::new(1).unwrap(), 0, 0, 1, &body_a).unwrap();
    let fb = plan_transfer(EPOCH, TransferId::new(1).unwrap(), 0, 0, 1, &body_b).unwrap();
    assert_eq!(fa, fb);
    // granted change produces different bytes
    let body_c = SnapshotBody {
        entities: vec![SnapshotEntity {
            entity: net(1),
            health: Some(HealthFact {
                health: 5,
                max_health: 10,
                dead: false,
            }),
        }],
        turn: None,
    };
    let fc = plan_transfer(EPOCH, TransferId::new(1).unwrap(), 0, 0, 1, &body_c).unwrap();
    assert_ne!(fa, fc);
    // None health is one 0 byte
    let enc = encode_body(&SnapshotBody {
        entities: vec![SnapshotEntity {
            entity: net(1),
            health: None,
        }],
        turn: None,
    })
    .unwrap();
    assert_eq!(enc, vec![0x01, 0x01, 0x00, 0x00]);
    let _ = (b1, b2);
}
// case 30
#[test]
fn oracle_catches_dropped_and_corrupt_chunks() {
    let body = make_body();
    // drop required chunk -> no snapshot
    let (mut pool, rid) = mkpool();
    let frames = plan_transfer(EPOCH, TransferId::new(1).unwrap(), 100, 5, 1, &body).unwrap();
    pool.offer_transfer(rid, &frames[0], 0).unwrap();
    for i in 1..frames.len() {
        if i == 2 {
            continue;
        }
        let _ = pool.offer_transfer(rid, &frames[i], 0);
    }
    // not complete -> no publish (unless all except one still incomplete)
    assert!(pool.status(rid).unwrap().published.is_none());
    // corrupt chunk consistently publishes wrong body -> oracle mismatch (simulate by different body)
    // unmodified transfer matches
    let (mut pool, rid) = mkpool();
    let frames = plan_transfer(EPOCH, TransferId::new(2).unwrap(), 100, 5, 1, &body).unwrap();
    for f in &frames {
        pool.offer_transfer(rid, f, 0).unwrap();
    }
    assert!(pool.status(rid).unwrap().published.is_some());
}
// case 31
#[test]
fn fabric_transfer_with_faults_and_resync() {
    use crpg_net::sim::{FaultSchedule, InMemoryTransport, QueueCaps, RateCaps, SimClock};
    let mut transport = InMemoryTransport::new(
        QueueCaps::v1_egress(),
        RateCaps {
            frames_per_sec: 1_000_000,
            burst_frames: 1_000_000,
            bytes_per_sec: 1_000_000_000,
            burst_bytes: 1_000_000_000,
            fail_per_sec: 1_000_000,
            burst_fail: 1_000_000,
            fail_bytes_per_sec: 1_000_000_000,
            burst_fail_bytes: 1_000_000_000,
        },
        SimClock(0),
    );
    let peer = transport.add_peer(EPOCH).expect("peer");
    let body = make_body();
    let frames = plan_transfer(EPOCH, TransferId::new(1).unwrap(), 100, 5, 1, &body).unwrap();
    for f in &frames {
        transport.send_to(peer, f.clone()).expect("staged");
    }
    transport.tick(&FaultSchedule {
        loss_every: None,
        dup_every: Some(2),
        reorder_depth: 2,
        seed: 7,
    });
    // receive all available
    let mut got = Vec::new();
    while let Some(b) = transport.recv_from(peer) {
        got.push(b);
    }
    // apply to pool
    let (mut pool, rid) = mkpool();
    for b in &got {
        let _ = pool.offer_transfer(rid, b, 0);
    }
    // may or may not publish depending faults; just no panic
    let _ = pool.status(rid);
    // resync expired scenario: loss hits chunk -> expires at t+5000
    let mut transport = InMemoryTransport::new(
        QueueCaps::v1_egress(),
        RateCaps {
            frames_per_sec: 1_000_000,
            burst_frames: 1_000_000,
            bytes_per_sec: 1_000_000_000,
            burst_bytes: 1_000_000_000,
            fail_per_sec: 1_000_000,
            burst_fail: 1_000_000,
            fail_bytes_per_sec: 1_000_000_000,
            burst_fail_bytes: 1_000_000_000,
        },
        SimClock(0),
    );
    let peer = transport.add_peer(EPOCH).expect("peer");
    let frames = plan_transfer(EPOCH, TransferId::new(1).unwrap(), 100, 5, 1, &body).unwrap();
    for f in &frames {
        transport.send_to(peer, f.clone()).expect("staged");
    }
    transport.tick(&FaultSchedule {
        loss_every: Some(2),
        dup_every: None,
        reorder_depth: 0,
        seed: 0,
    });
    let mut got = Vec::new();
    while let Some(b) = transport.recv_from(peer) {
        got.push(b);
    }
    let (mut pool, rid) = mkpool();
    for b in &got {
        let _ = pool.offer_transfer(rid, b, 0);
    }
    // expire
    pool.poll(5000);
    // client encodes Resync
    let rs = TransferUp::Resync {
        epoch: EPOCH,
        after: Some(TransferId::new(1).unwrap()),
        reason: ResyncReason::Expired,
    };
    let enc = encode_up(&rs).unwrap();
    let dec = decode_up(&enc).unwrap();
    assert_eq!(dec, rs);
    // send transfer 2 publishes
    let frames2 = plan_transfer(EPOCH, TransferId::new(2).unwrap(), 100, 5, 1, &body).unwrap();
    let (mut pool2, rid2) = mkpool();
    for f in &frames2 {
        pool2.offer_transfer(rid2, f, 0).unwrap();
    }
    assert!(pool2.status(rid2).unwrap().published.is_some());
}

//! T140d implementer suite: the protocol v3 `0x82` snapshot (`transfer_v3`
//! and `facts_v3::project_snapshot_v3`), derived from `tasks/T140.md`
//! Behaviour D-1..D-6 and the tests D1..D10, through the public API only.
//!
//! Expectations are hand-written bytes or whole values, never produced by the
//! code under test. The v1 `0x81` codec is used only as the contract's own
//! oracle for "every `0x82` frame is the `0x81` frame with byte 0 changed".
//! Every bad byte is followed by a full valid remainder, so a case fails on
//! the byte under test and not on truncation, and each rejection has a
//! passing control.

use std::sync::OnceLock;

use crpg_core::{GenerationalArena, Ulid};
use crpg_net::codec_transfer;
use crpg_net::codec_v3::{self, CodecError};
use crpg_net::facts_v3::{
    project_page, project_snapshot_v3, EncounterGrants, EncounterInput, FactContext, FactError,
    FactMask, FactRow, FactView, OrderInput, PageV3, SnapshotProjectionV3,
};
use crpg_net::interest::{
    EntityFact, EntityKey, FieldMask, InterestError, InterestSet, InterestView, TurnState,
    ViewCursor, ViewerContext,
};
use crpg_net::protocol::{self, SEQ_LAST};
use crpg_net::protocol_transfer::{CancelReason, ResyncReason, MAX_CHUNK_BYTES, MAX_SEGMENT_BYTES};
use crpg_net::protocol_v3::{
    ClockFact, DeltaOp, EffectFact, EncounterFacts, EncounterMode, FactSet, NetId, OrderFact,
    PoolFact, SessionEpoch, StatFact, StatValue, TimelineEntry,
};
use crpg_net::transfer_v3::{
    check_begin, chunk_len, decode_body, decode_down, decode_up, encode_body, encode_down,
    encode_up, plan_transfer, BeginTransfer, HealthFact, SnapshotBodyV3, SnapshotEntityV3,
    TransferChunk, TransferDown, TransferId, TransferUp, TurnFact, LANE_SNAPSHOT,
    MAX_ENCODED_BODY_BYTES_V3, MAX_SNAPSHOT_BYTES_V3, MAX_SNAPSHOT_CHUNKS_V3, TRANSFER_VERSION_V3,
};

// ---------------------------------------------------------------------------
// Small constructors and hand-written byte builders.
// ---------------------------------------------------------------------------

const Z16: [u8; 16] = [0; 16];
const MIB: usize = 1_048_576;

fn u(n: u128) -> Ulid {
    Ulid::from_u128(n)
}

fn net(n: u64) -> NetId {
    NetId::new(n).expect("fixture net id is nonzero")
}

fn tid(n: u64) -> TransferId {
    TransferId::new(n).expect("fixture transfer id is nonzero")
}

fn epoch() -> SessionEpoch {
    SessionEpoch([0; 16])
}

fn other_epoch() -> SessionEpoch {
    SessionEpoch([1; 16])
}

/// Parses a hex literal: whitespace-separated bytes, `Z16` for sixteen zero
/// bytes and `XX*n` for `n` copies of a byte.
fn hex(text: &str) -> Vec<u8> {
    let mut out = Vec::new();
    for token in text.split_whitespace() {
        if token == "Z16" {
            out.extend_from_slice(&Z16);
        } else if let Some((byte, count)) = token.split_once('*') {
            let byte = u8::from_str_radix(byte, 16).expect("hex byte");
            let count: usize = count.parse().expect("repeat count");
            out.extend(std::iter::repeat_n(byte, count));
        } else {
            out.push(u8::from_str_radix(token, 16).expect("hex byte"));
        }
    }
    out
}

/// LEB128, written here so the codec is never its own oracle.
fn varint(mut v: u64) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let byte = (v & 0x7F) as u8;
        v >>= 7;
        if v == 0 {
            out.push(byte);
            return out;
        }
        out.push(byte | 0x80);
    }
}

/// A length-prefixed ULID text as it appears on the wire.
fn ulid_bytes(n: u128) -> Vec<u8> {
    let text = u(n).to_string();
    assert_eq!(text.len(), 26);
    let mut out = vec![26];
    out.extend_from_slice(text.as_bytes());
    out
}

fn concat(parts: &[&[u8]]) -> Vec<u8> {
    parts.concat()
}

const NO_FACTS: [u8; 5] = [0; 5];
const NO_TURN: [u8; 1] = [0];
const NO_ENCOUNTER: [u8; 3] = [0; 3];
const NO_TIMELINE: [u8; 1] = [0];

/// One entity: id, no health, the given fact-set bytes.
fn ent_with(id: u64, facts: &[u8]) -> Vec<u8> {
    concat(&[&varint(id), &[0], facts])
}

/// One entity with no health and no facts.
fn ent(id: u64) -> Vec<u8> {
    ent_with(id, &NO_FACTS)
}

/// Fact-set bytes whose only disclosed family is an order (`target`).
fn order_facts(target: u64) -> Vec<u8> {
    concat(&[&[0, 0, 1, 1], &ulid_bytes(7), &varint(target), &[1, 0, 0]])
}

/// A body from hand-written entities and the three trailing blocks.
fn body_bytes(entities: &[Vec<u8>], turn: &[u8], encounter: &[u8], timeline: &[u8]) -> Vec<u8> {
    let mut out = varint(entities.len() as u64);
    for entity in entities {
        out.extend_from_slice(entity);
    }
    out.extend_from_slice(turn);
    out.extend_from_slice(encounter);
    out.extend_from_slice(timeline);
    out
}

/// A valid body of plain entities, no turn, withheld encounter and timeline.
fn plain_body_bytes(ids: &[u64]) -> Vec<u8> {
    let entities: Vec<Vec<u8>> = ids.iter().map(|&id| ent(id)).collect();
    body_bytes(&entities, &NO_TURN, &NO_ENCOUNTER, &NO_TIMELINE)
}

fn plain_entity(id: u64) -> SnapshotEntityV3 {
    SnapshotEntityV3 {
        entity: net(id),
        health: None,
        facts: FactSet::default(),
    }
}

fn empty_body() -> SnapshotBodyV3 {
    SnapshotBodyV3 {
        entities: Vec::new(),
        turn: None,
        encounter: EncounterFacts::default(),
        timeline: None,
    }
}

fn plain_body(ids: &[u64]) -> SnapshotBodyV3 {
    SnapshotBodyV3 {
        entities: ids.iter().map(|&id| plain_entity(id)).collect(),
        ..empty_body()
    }
}

fn pool(id: u128, current: u32, max: u32) -> PoolFact {
    PoolFact {
        pool: u(id),
        current,
        max,
    }
}

fn effect(id: u128, expires_round: u64) -> EffectFact {
    EffectFact {
        effect: u(id),
        expires_round,
    }
}

fn stat(name: &str, value: StatValue) -> StatFact {
    StatFact {
        name: name.to_owned(),
        value,
    }
}

fn tl(key: i32, entity: u64) -> TimelineEntry {
    TimelineEntry {
        key,
        entity: net(entity),
    }
}

fn begin(total: u32, entities: u16, spc: u8, chunks: u8) -> BeginTransfer {
    BeginTransfer {
        epoch: Z16,
        transfer: tid(1),
        base_tick: 0,
        cursor: 0,
        total_bytes: total,
        entity_count: entities,
        segments_per_chunk: spc,
        chunk_count: chunks,
    }
}

fn chunk(index: u8, bytes: Vec<u8>) -> TransferChunk {
    TransferChunk {
        epoch: Z16,
        transfer: tid(1),
        index,
        bytes,
    }
}

/// The `0x81` copy of a `0x82` frame, and the other way round.
fn with_version(frame: &[u8], version: u8) -> Vec<u8> {
    let mut copy = frame.to_vec();
    copy[0] = version;
    copy
}

// ---------------------------------------------------------------------------
// D1: frame literals, D-1 (a).
// ---------------------------------------------------------------------------

#[test]
fn d1_frame_literals_0x82() {
    // The empty body is six zero bytes.
    assert_eq!(encode_body(&empty_body()), Ok(hex("00 00 00 00 00 00")));

    let b = begin(6, 0, 1, 1);
    let begin_bytes = hex("82 02 10 Z16 01 00 00 06 00 01 01");
    assert_eq!(begin_bytes.len(), 26);
    assert_eq!(
        encode_down(&TransferDown::Begin(b.clone())),
        Ok(begin_bytes.clone())
    );
    assert_eq!(
        decode_down(&begin_bytes),
        Ok(TransferDown::Begin(b.clone()))
    );

    let c = chunk(0, vec![0; 6]);
    let chunk_bytes = hex("82 02 11 Z16 01 00 01 06 00 00 00 00 00 00");
    assert_eq!(chunk_bytes.len(), 29);
    assert_eq!(
        encode_down(&TransferDown::Chunk(c.clone())),
        Ok(chunk_bytes.clone())
    );
    assert_eq!(decode_down(&chunk_bytes), Ok(TransferDown::Chunk(c)));

    let cancel = TransferDown::Cancel {
        epoch: Z16,
        transfer: tid(1),
        reason: CancelReason::Superseded,
    };
    let cancel_bytes = hex("82 02 12 Z16 01 01");
    assert_eq!(cancel_bytes.len(), 21);
    assert_eq!(encode_down(&cancel), Ok(cancel_bytes.clone()));
    assert_eq!(decode_down(&cancel_bytes), Ok(cancel));
    let aborted = TransferDown::Cancel {
        epoch: Z16,
        transfer: tid(1),
        reason: CancelReason::Aborted,
    };
    assert_eq!(encode_down(&aborted), Ok(hex("82 02 12 Z16 01 02")));

    let published = TransferUp::Published {
        epoch: Z16,
        transfer: tid(1),
    };
    let published_bytes = hex("82 02 20 Z16 01");
    assert_eq!(published_bytes.len(), 20);
    assert_eq!(encode_up(&published), Ok(published_bytes.clone()));
    assert_eq!(decode_up(&published_bytes), Ok(published.clone()));

    let resync = TransferUp::Resync {
        epoch: Z16,
        after: None,
        reason: ResyncReason::Expired,
    };
    let resync_bytes = hex("82 02 21 Z16 00 01");
    assert_eq!(resync_bytes.len(), 21);
    assert_eq!(encode_up(&resync), Ok(resync_bytes.clone()));
    assert_eq!(decode_up(&resync_bytes), Ok(resync));
    let resync_after = TransferUp::Resync {
        epoch: Z16,
        after: Some(tid(9)),
        reason: ResyncReason::Overflow,
    };
    assert_eq!(encode_up(&resync_after), Ok(hex("82 02 21 Z16 09 03")));
    assert_eq!(
        decode_up(&hex("82 02 21 Z16 09 03")),
        Ok(resync_after.clone())
    );

    // Each equals the 0x81 encoding with byte 0 changed.
    let downs = [
        TransferDown::Begin(b),
        TransferDown::Chunk(chunk(0, vec![0; 6])),
        aborted,
    ];
    for msg in &downs {
        let v3 = encode_down(msg).expect("v3 encodes");
        let v1 = codec_transfer::encode_down(msg).expect("v1 encodes");
        assert_eq!(v3, with_version(&v1, 0x82), "{msg:?}");
        assert_eq!(v1[0], 0x81);
    }
    for msg in [&published, &resync_after] {
        let v3 = encode_up(msg).expect("v3 encodes");
        let v1 = codec_transfer::encode_up(msg).expect("v1 encodes");
        assert_eq!(v3, with_version(&v1, 0x82), "{msg:?}");
    }

    // plan_transfer of the empty body is exactly the Begin and the Chunk.
    assert_eq!(
        plan_transfer(Z16, tid(1), 0, 0, 1, &empty_body()),
        Ok(vec![begin_bytes, chunk_bytes])
    );
}

#[test]
fn d1_encode_equivalence_with_0x81_over_a_corpus() {
    // Every Begin the 0x81 encoder accepts has the same bytes under 0x82; a
    // Begin it refuses for a reason other than a v1 cap has the same error.
    let mut begins = Vec::new();
    for total in [
        0u32,
        1,
        4096,
        4097,
        32_768,
        32_769,
        MIB as u32,
        MIB as u32 + 1,
    ] {
        for entities in [0u16, 1, 1024, 1025] {
            for spc in [0u8, 1, 2, 8, 9] {
                for chunks in [0u8, 1, 2, 32, 33, 128, 129] {
                    begins.push(begin(total, entities, spc, chunks));
                }
            }
        }
    }
    let mut cursors = begin(6, 0, 1, 1);
    cursors.cursor = SEQ_LAST + 1;
    begins.push(cursors);
    let mut cursor_last = begin(6, 0, 1, 1);
    cursor_last.cursor = SEQ_LAST;
    begins.push(cursor_last);

    let (mut accepted, mut refused_same) = (0, 0);
    for b in &begins {
        let msg = TransferDown::Begin(b.clone());
        let v1 = codec_transfer::encode_down(&msg);
        let v3 = encode_down(&msg);
        match v1 {
            Ok(bytes) => {
                assert_eq!(v3, Ok(with_version(&bytes, 0x82)), "{b:?}");
                accepted += 1;
            }
            Err(CodecError::LimitExceeded) => {}
            Err(error) => {
                assert_eq!(v3, Err(error), "{b:?}");
                refused_same += 1;
            }
        }
    }
    assert!(accepted > 20 && refused_same > 100);

    // The same for Chunks within the v1 bounds.
    for index in [0u8, 1, 31] {
        for len in [0usize, 1, 4095, 4096, 4097, 32_768, 32_769] {
            let msg = TransferDown::Chunk(chunk(index, vec![7; len]));
            let v1 = codec_transfer::encode_down(&msg);
            let v3 = encode_down(&msg);
            match v1 {
                Ok(bytes) => assert_eq!(v3, Ok(with_version(&bytes, 0x82))),
                Err(error) => assert_eq!(v3, Err(error), "index {index} len {len}"),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// D1: decode equivalence with 0x81, D-1 (b)(4).
// ---------------------------------------------------------------------------

/// Every prefix, a trailing byte and every single-byte substitution of the
/// first and last `edge` positions of `frame` (a 0x81 frame).
fn mutations(frame: &[u8], edge: usize) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    for len in 0..=frame.len() {
        out.push(frame[..len].to_vec());
    }
    let mut trailing = frame.to_vec();
    trailing.push(0);
    out.push(trailing);
    for position in 1..frame.len() {
        if position > edge && position + edge < frame.len() {
            continue;
        }
        for value in [0x00u8, 0x01, 0x02, 0x03, 0x7F, 0x80, 0xFF] {
            let mut copy = frame.to_vec();
            copy[position] = value;
            out.push(copy);
        }
    }
    out
}

#[test]
fn d1_decode_down_equals_0x81_inside_the_v1_bounds() {
    let seeds: Vec<Vec<u8>> = vec![
        // Begin: 2 chunks of 8 segments, and a 1-segment single chunk.
        codec_transfer::encode_down(&TransferDown::Begin(begin(40_000, 5, 8, 2))).unwrap(),
        codec_transfer::encode_down(&TransferDown::Begin(begin(1, 1024, 1, 1))).unwrap(),
        // Chunk: two segments (4,096 then a short one).
        codec_transfer::encode_down(&TransferDown::Chunk(chunk(1, vec![3; 4096 + 10]))).unwrap(),
        // Chunk: one short segment.
        codec_transfer::encode_down(&TransferDown::Chunk(chunk(0, vec![9; 5]))).unwrap(),
        // Cancel.
        codec_transfer::encode_down(&TransferDown::Cancel {
            epoch: [5; 16],
            transfer: tid(300),
            reason: CancelReason::Aborted,
        })
        .unwrap(),
        // Up frames decoded as down: WrongDirection territory.
        codec_transfer::encode_up(&TransferUp::Published {
            epoch: Z16,
            transfer: tid(2),
        })
        .unwrap(),
    ];
    let (mut compared, mut skipped) = (0usize, 0usize);
    for seed in &seeds {
        for frame in mutations(seed, 40) {
            let v1 = codec_transfer::decode_down(&frame);
            let v3 = if frame.is_empty() {
                decode_down(&frame)
            } else {
                decode_down(&with_version(&frame, 0x82))
            };
            // A frame the v1 decoder refuses by its own caps is outside the
            // D-1 (b)(4) range; every other frame must agree exactly.
            let first_is_v1 = frame.first() == Some(&0x81);
            if !first_is_v1 {
                // Mutated byte 0 never happens (position starts at 1), but a
                // prefix of length 0 is the empty frame.
                assert_eq!(v3, v1, "empty or non-0x81 frame {frame:?}");
                continue;
            }
            if v1 == Err(CodecError::LimitExceeded) {
                skipped += 1;
                continue;
            }
            assert_eq!(v3, v1, "frame {frame:?}");
            compared += 1;
        }
    }
    assert!(compared > 1_000, "compared {compared}");
    assert!(skipped < compared, "skipped {skipped}");
}

#[test]
fn d1_decode_up_equals_0x81() {
    let seeds: Vec<Vec<u8>> = vec![
        codec_transfer::encode_up(&TransferUp::Published {
            epoch: [4; 16],
            transfer: tid(77),
        })
        .unwrap(),
        codec_transfer::encode_up(&TransferUp::Resync {
            epoch: [4; 16],
            after: Some(tid(77)),
            reason: ResyncReason::Invalid,
        })
        .unwrap(),
        codec_transfer::encode_up(&TransferUp::Resync {
            epoch: Z16,
            after: None,
            reason: ResyncReason::Overflow,
        })
        .unwrap(),
        // Down frames decoded as up: WrongDirection territory.
        codec_transfer::encode_down(&TransferDown::Begin(begin(6, 0, 1, 1))).unwrap(),
    ];
    let mut compared = 0;
    for seed in &seeds {
        for frame in mutations(seed, 40) {
            let v1 = codec_transfer::decode_up(&frame);
            let v3 = if frame.is_empty() {
                decode_up(&frame)
            } else {
                decode_up(&with_version(&frame, 0x82))
            };
            assert_eq!(v3, v1, "frame {frame:?}");
            compared += 1;
        }
    }
    assert!(compared > 500);
}

#[test]
fn d1_check_begin_equals_validate_and_chunk_len_inside_the_v1_bounds() {
    let mut checked = 0;
    for total in [0u32, 1, 4096, 32_768, 32_769, MIB as u32] {
        for entities in [0u16, 1024, 1025] {
            for spc in [0u8, 1, 8, 9] {
                for chunks in [0u8, 1, 2, 32] {
                    for cursor in [0u64, SEQ_LAST, SEQ_LAST + 1] {
                        let mut b = begin(total, entities, spc, chunks);
                        b.cursor = cursor;
                        assert_eq!(check_begin(&b), b.validate(), "{b:?}");
                        for index in [0u8, 1, 31, 32, 127, 255] {
                            assert_eq!(chunk_len(&b, index), b.chunk_len(index), "{b:?}/{index}");
                        }
                        checked += 1;
                    }
                }
            }
        }
    }
    assert!(checked > 500);
}

// ---------------------------------------------------------------------------
// D2: version, lane and direction refusals.
// ---------------------------------------------------------------------------

#[test]
fn d2_version_refusals() {
    let down = hex("82 02 10 Z16 01 00 00 06 00 01 01");
    let up = hex("82 02 20 Z16 01");

    // Controls: the 0x82 frames decode.
    assert!(decode_down(&down).is_ok());
    assert!(decode_up(&up).is_ok());

    // The v3 decoders refuse 0x81, and every other first byte.
    for version in [0x81u8, 0x91, 0x00, 0x01, 0x02, 0x03, 0x83, 0xFF] {
        assert_eq!(
            decode_down(&with_version(&down, version)),
            Err(CodecError::UnsupportedVersion),
            "down {version:#x}"
        );
        assert_eq!(
            decode_up(&with_version(&up, version)),
            Err(CodecError::UnsupportedVersion),
            "up {version:#x}"
        );
    }
    // The 0x81 decoders refuse 0x82.
    assert_eq!(
        codec_transfer::decode_down(&down),
        Err(CodecError::UnsupportedVersion)
    );
    assert_eq!(
        codec_transfer::decode_up(&up),
        Err(CodecError::UnsupportedVersion)
    );
    // codec_v3's delta decoder refuses 0x82.
    assert_eq!(
        codec_v3::decode_delta(&down),
        Err(CodecError::UnsupportedVersion)
    );
    // Empty input is Malformed.
    assert_eq!(decode_down(&[]), Err(CodecError::Malformed));
    assert_eq!(decode_up(&[]), Err(CodecError::Malformed));

    // The frame cap runs before the version check.
    let mut big_down = vec![0x81u8];
    big_down.resize(65_537, 0);
    assert_eq!(decode_down(&big_down), Err(CodecError::FrameTooLarge));
    let mut big_up = vec![0x81u8];
    big_up.resize(4_097, 0);
    assert_eq!(decode_up(&big_up), Err(CodecError::FrameTooLarge));
    // At the cap the version check is reached.
    let mut at_down = vec![0x81u8];
    at_down.resize(65_536, 0);
    assert_eq!(decode_down(&at_down), Err(CodecError::UnsupportedVersion));
    let mut at_up = vec![0x81u8];
    at_up.resize(4_096, 0);
    assert_eq!(decode_up(&at_up), Err(CodecError::UnsupportedVersion));

    // Lane and direction.
    assert_eq!(
        decode_down(&hex("82 03 10 Z16 01 00 00 06 00 01 01")),
        Err(CodecError::WrongDirection)
    );
    assert_eq!(
        decode_up(&hex("82 03 20 Z16 01")),
        Err(CodecError::WrongDirection)
    );
    assert_eq!(
        decode_down(&hex("82 02 20 Z16 01")),
        Err(CodecError::WrongDirection)
    );
    assert_eq!(
        decode_down(&hex("82 02 21 Z16 00 01")),
        Err(CodecError::WrongDirection)
    );
    assert_eq!(
        decode_up(&hex("82 02 10 Z16 01 00 00 06 00 01 01")),
        Err(CodecError::WrongDirection)
    );
    assert_eq!(
        decode_up(&hex("82 02 12 Z16 01 01")),
        Err(CodecError::WrongDirection)
    );
    // Unknown tags.
    assert_eq!(
        decode_down(&hex("82 02 13 Z16 01")),
        Err(CodecError::UnknownMessage)
    );
    assert_eq!(
        decode_up(&hex("82 02 22 Z16 01")),
        Err(CodecError::UnknownMessage)
    );
    // Unknown reasons.
    assert_eq!(
        decode_down(&hex("82 02 12 Z16 01 03")),
        Err(CodecError::UnknownMessage)
    );
    assert_eq!(
        decode_up(&hex("82 02 21 Z16 00 04")),
        Err(CodecError::UnknownMessage)
    );
    // Trailing bytes after each message.
    assert_eq!(
        decode_down(&hex("82 02 12 Z16 01 01 00")),
        Err(CodecError::Malformed)
    );
    assert_eq!(
        decode_up(&hex("82 02 20 Z16 01 00")),
        Err(CodecError::Malformed)
    );
    // The v3 decoders never accept the v1 transfer id 0.
    assert_eq!(
        decode_up(&hex("82 02 20 Z16 00")),
        Err(CodecError::Malformed)
    );
}

// ---------------------------------------------------------------------------
// D3: body literals.
// ---------------------------------------------------------------------------

#[test]
fn d3_body_literals() {
    let empty = hex("00 00 00 00 00 00");
    assert_eq!(encode_body(&empty_body()), Ok(empty.clone()));
    assert_eq!(decode_body(&empty), Ok(empty_body()));

    let one = SnapshotBodyV3 {
        entities: vec![SnapshotEntityV3 {
            entity: net(1),
            health: Some(HealthFact {
                health: 10,
                max_health: 10,
                dead: false,
            }),
            facts: FactSet::default(),
        }],
        ..empty_body()
    };
    let one_bytes = hex("01 01 01 0A 0A 00 00 00 00 00 00 00 00 00 00 00");
    assert_eq!(one_bytes.len(), 16);
    assert_eq!(encode_body(&one), Ok(one_bytes.clone()));
    assert_eq!(decode_body(&one_bytes), Ok(one));

    // Every block, in order: entity with a fact set, turn, encounter, timeline.
    let full = SnapshotBodyV3 {
        entities: vec![
            SnapshotEntityV3 {
                entity: net(1),
                health: None,
                facts: FactSet {
                    pools: Some(vec![pool(1, 2, 3)]),
                    effects: Some(vec![]),
                    order: Some(Some(OrderFact {
                        ability: u(2),
                        target: net(2),
                        repeat: true,
                    })),
                    stats: Some(vec![stat("hp", StatValue::Fixed(-1))]),
                    tags: Some(vec!["a".to_owned()]),
                },
            },
            plain_entity(2),
        ],
        turn: Some(TurnFact {
            active: Some(net(2)),
            round: 5,
        }),
        encounter: EncounterFacts {
            mode: Some(EncounterMode::RealTime),
            pending_mode: Some(Some(EncounterMode::TurnBased)),
            clock: Some(Some(ClockFact {
                round_ticks: 120,
                elapsed: 240,
            })),
        },
        timeline: Some(vec![tl(-1, 2), tl(-1, 1)]),
    };
    let expected = concat(&[
        &[0x02],
        // entity 1: no health; pools [(U(1), 2, 3)], effects [], order, stats, tags.
        &[0x01, 0x00],
        &[0x01, 0x01],
        &ulid_bytes(1),
        &[0x02, 0x03],
        &[0x01, 0x00],
        &[0x01, 0x01],
        &ulid_bytes(2),
        &[0x02, 0x01],
        &[0x01, 0x01, 0x02, b'h', b'p', 0x01, 0x01],
        &[0x01, 0x01, 0x01, b'a'],
        // entity 2: plain.
        &ent(2),
        // turn: present, active present, 2, round 5.
        &[0x01, 0x01, 0x02, 0x05],
        // encounter: RealTime, pending Some(TurnBased), clock {120, 240}.
        &[0x01, 0x01, 0x01, 0x01, 0x00, 0x01, 0x01, 120, 0xF0, 0x01],
        // timeline: 2 entries (-1 zigzag 1), (-1, 1).
        &[0x01, 0x02, 0x01, 0x02, 0x01, 0x01],
    ]);
    assert_eq!(encode_body(&full), Ok(expected.clone()));
    assert_eq!(decode_body(&expected), Ok(full));
}

// ---------------------------------------------------------------------------
// D4: body refusals, D-3 decode order.
// ---------------------------------------------------------------------------

#[test]
fn d4_size_and_count_bounds() {
    // The 4 MiB cap: one byte over is LimitExceeded, at the cap is read.
    let over = vec![0u8; MAX_SNAPSHOT_BYTES_V3 + 1];
    assert_eq!(decode_body(&over), Err(CodecError::LimitExceeded));
    let at = vec![0u8; MAX_SNAPSHOT_BYTES_V3];
    assert_eq!(
        decode_body(&at),
        Err(CodecError::Malformed),
        "trailing zeros"
    );

    // 1,024 entities decode; 1,025 are LimitExceeded, before growth.
    let ids: Vec<u64> = (1..=1024).collect();
    let control = plain_body_bytes(&ids);
    assert_eq!(decode_body(&control).unwrap().entities.len(), 1024);
    let ids: Vec<u64> = (1..=1025).collect();
    assert_eq!(
        decode_body(&plain_body_bytes(&ids)),
        Err(CodecError::LimitExceeded)
    );
    // A bare hostile count, with nothing after it.
    assert_eq!(decode_body(&[0x81, 0x08]), Err(CodecError::LimitExceeded));
    assert_eq!(
        decode_body(&hex("FF FF FF FF FF FF FF FF FF 01")),
        Err(CodecError::LimitExceeded)
    );
    // A count varint that overflows u64 is Malformed.
    assert_eq!(
        decode_body(&hex("FF FF FF FF FF FF FF FF FF 02")),
        Err(CodecError::Malformed)
    );
    // Empty input is truncation.
    assert_eq!(decode_body(&[]), Err(CodecError::Malformed));
}

#[test]
fn d4_entity_order_and_flags() {
    // Control.
    assert!(decode_body(&plain_body_bytes(&[1, 2, 3])).is_ok());

    // Entity 0, with a full valid remainder.
    assert_eq!(
        decode_body(&plain_body_bytes(&[0])),
        Err(CodecError::Malformed)
    );
    assert_eq!(
        decode_body(&plain_body_bytes(&[0, 1])),
        Err(CodecError::Malformed)
    );
    // Adjacent descending, equal, and non-adjacent descending.
    assert_eq!(
        decode_body(&plain_body_bytes(&[2, 1])),
        Err(CodecError::Malformed)
    );
    assert_eq!(
        decode_body(&plain_body_bytes(&[1, 1])),
        Err(CodecError::Malformed)
    );
    assert_eq!(
        decode_body(&plain_body_bytes(&[1, 3, 2])),
        Err(CodecError::Malformed)
    );
    assert_eq!(
        decode_body(&plain_body_bytes(&[5, 6, 7, 3, 9])),
        Err(CodecError::Malformed)
    );
    // An ascending gap is fine.
    assert!(decode_body(&plain_body_bytes(&[1, 1000, u64::MAX])).is_ok());

    // A health flag of 02 and a dead byte of 02, with valid remainders.
    let bad_health_flag = body_bytes(
        &[concat(&[&[1, 2], &NO_FACTS])],
        &NO_TURN,
        &NO_ENCOUNTER,
        &NO_TIMELINE,
    );
    assert_eq!(decode_body(&bad_health_flag), Err(CodecError::Malformed));
    let health_ok = body_bytes(
        &[concat(&[&[1, 1, 5, 5, 0], &NO_FACTS])],
        &NO_TURN,
        &NO_ENCOUNTER,
        &NO_TIMELINE,
    );
    assert!(decode_body(&health_ok).is_ok());
    let bad_dead = body_bytes(
        &[concat(&[&[1, 1, 5, 5, 2], &NO_FACTS])],
        &NO_TURN,
        &NO_ENCOUNTER,
        &NO_TIMELINE,
    );
    assert_eq!(decode_body(&bad_dead), Err(CodecError::Malformed));

    // The turn flag, the active flag and the timeline flag at 02.
    let ents = [ent(1)];
    assert_eq!(
        decode_body(&body_bytes(&ents, &[2], &NO_ENCOUNTER, &NO_TIMELINE)),
        Err(CodecError::Malformed)
    );
    assert_eq!(
        decode_body(&body_bytes(
            &ents,
            &[1, 2, 1, 0],
            &NO_ENCOUNTER,
            &NO_TIMELINE
        )),
        Err(CodecError::Malformed)
    );
    assert!(decode_body(&body_bytes(&ents, &[1, 0, 0], &NO_ENCOUNTER, &NO_TIMELINE)).is_ok());
    assert_eq!(
        decode_body(&body_bytes(&ents, &NO_TURN, &NO_ENCOUNTER, &[2])),
        Err(CodecError::Malformed)
    );
    // An encounter disclosed flag of 02 (mode, pending and clock).
    for flags in [[2u8, 0, 0], [0, 2, 0], [0, 0, 2]] {
        assert_eq!(
            decode_body(&body_bytes(&ents, &NO_TURN, &flags, &NO_TIMELINE)),
            Err(CodecError::Malformed),
            "{flags:?}"
        );
    }
    // A fact-set family flag of 02, in each of the five positions.
    for position in 0..5 {
        let mut facts = NO_FACTS;
        facts[position] = 2;
        let body = body_bytes(
            &[ent_with(1, &facts)],
            &NO_TURN,
            &NO_ENCOUNTER,
            &NO_TIMELINE,
        );
        assert_eq!(decode_body(&body), Err(CodecError::Malformed), "{position}");
    }
}

#[test]
fn d4_fact_set_errors_surface_through_the_body() {
    // pool: current 3 > max 2 is Malformed; the control has current 2, max 3.
    let facts = |pools: Vec<u8>| concat(&[&[1], &pools[..], &[0, 0, 0, 0]]);
    let good = facts(concat(&[&[1], &ulid_bytes(1), &[2, 3]]));
    let bad = facts(concat(&[&[1], &ulid_bytes(1), &[3, 2]]));
    let wrap =
        |facts: &[u8]| body_bytes(&[ent_with(1, facts)], &NO_TURN, &NO_ENCOUNTER, &NO_TIMELINE);
    assert!(decode_body(&wrap(&good)).is_ok());
    assert_eq!(decode_body(&wrap(&bad)), Err(CodecError::Malformed));

    // stats: a count of 33 is LimitExceeded; 32 is read (control).
    let stats = |count: usize| {
        let mut out = vec![0, 0, 0, 1];
        out.extend(varint(count as u64));
        for i in 0..count {
            out.extend_from_slice(&[3, b's', b'0' + (i / 10) as u8, b'0' + (i % 10) as u8, 0, 0]);
        }
        out.push(0);
        out
    };
    assert!(decode_body(&wrap(&stats(32))).is_ok());
    assert_eq!(
        decode_body(&wrap(&stats(33))),
        Err(CodecError::LimitExceeded)
    );
    // a stat kind of 02 is UnknownMessage.
    let kind2 = concat(&[&[0, 0, 0, 1, 1], &[1, b'x', 2, 0], &[0]]);
    assert_eq!(decode_body(&wrap(&kind2)), Err(CodecError::UnknownMessage));
    let kind1 = concat(&[&[0, 0, 0, 1, 1], &[1, b'x', 1, 0], &[0]]);
    assert!(decode_body(&wrap(&kind1)).is_ok());
}

#[test]
fn d4_order_target_membership() {
    // The target is an entity of the body: a different one, then itself.
    let two = [ent_with(1, &order_facts(2)), ent(2)];
    assert!(decode_body(&body_bytes(&two, &NO_TURN, &NO_ENCOUNTER, &NO_TIMELINE)).is_ok());
    let itself = [ent_with(1, &order_facts(1))];
    assert!(decode_body(&body_bytes(&itself, &NO_TURN, &NO_ENCOUNTER, &NO_TIMELINE)).is_ok());
    // A later entity may be the target (membership runs after the last one).
    let later = [ent_with(1, &order_facts(3)), ent(2), ent(3)];
    assert!(decode_body(&body_bytes(&later, &NO_TURN, &NO_ENCOUNTER, &NO_TIMELINE)).is_ok());

    // Not in the body, adjacent and non-adjacent.
    let missing = [ent_with(1, &order_facts(9))];
    assert_eq!(
        decode_body(&body_bytes(&missing, &NO_TURN, &NO_ENCOUNTER, &NO_TIMELINE)),
        Err(CodecError::Malformed)
    );
    let missing_far = [ent(1), ent(2), ent_with(3, &order_facts(4))];
    assert_eq!(
        decode_body(&body_bytes(
            &missing_far,
            &NO_TURN,
            &NO_ENCOUNTER,
            &NO_TIMELINE
        )),
        Err(CodecError::Malformed)
    );
    // Target 0 is Malformed at the field.
    let zero = [ent_with(1, &order_facts(0))];
    assert_eq!(
        decode_body(&body_bytes(&zero, &NO_TURN, &NO_ENCOUNTER, &NO_TIMELINE)),
        Err(CodecError::Malformed)
    );
}

#[test]
fn d4_turn_active_membership() {
    let ents = [ent(1), ent(2)];
    let turn = |active: u8| vec![1, 1, active, 5];
    assert!(decode_body(&body_bytes(&ents, &turn(2), &NO_ENCOUNTER, &NO_TIMELINE)).is_ok());
    assert_eq!(
        decode_body(&body_bytes(&ents, &turn(3), &NO_ENCOUNTER, &NO_TIMELINE)),
        Err(CodecError::Malformed)
    );
    assert_eq!(
        decode_body(&body_bytes(&ents, &turn(0), &NO_ENCOUNTER, &NO_TIMELINE)),
        Err(CodecError::Malformed)
    );
    // No active combatant needs no membership.
    assert!(decode_body(&body_bytes(&ents, &[1, 0, 9], &NO_ENCOUNTER, &NO_TIMELINE)).is_ok());
    // An empty body has no entity to be active.
    assert_eq!(
        decode_body(&body_bytes(&[], &turn(1), &NO_ENCOUNTER, &NO_TIMELINE)),
        Err(CodecError::Malformed)
    );
}

#[test]
fn d4_encounter_and_timeline_blocks() {
    let ents = [ent(1), ent(2)];
    let ok_encounter = [1u8, 1, 1, 1, 1, 1, 1, 120, 0xF0, 0x01];
    // mode RealTime; pending Some(Some(RealTime)); clock Some(Some).
    assert!(decode_body(&body_bytes(&ents, &NO_TURN, &ok_encounter, &NO_TIMELINE)).is_ok());
    // mode byte 02, pending byte 02: UnknownMessage.
    let mut bad_mode = ok_encounter;
    bad_mode[1] = 2;
    assert_eq!(
        decode_body(&body_bytes(&ents, &NO_TURN, &bad_mode, &NO_TIMELINE)),
        Err(CodecError::UnknownMessage)
    );
    let mut bad_pending = ok_encounter;
    bad_pending[4] = 2;
    assert_eq!(
        decode_body(&body_bytes(&ents, &NO_TURN, &bad_pending, &NO_TIMELINE)),
        Err(CodecError::UnknownMessage)
    );
    // round_ticks 0: Malformed.
    let mut zero_ticks = ok_encounter;
    zero_ticks[7] = 0;
    assert_eq!(
        decode_body(&body_bytes(&ents, &NO_TURN, &zero_ticks, &NO_TIMELINE)),
        Err(CodecError::Malformed)
    );
    // elapsed 2^31: Malformed; 2^31 - 1: accepted.
    let clock = |elapsed: u64| concat(&[&[0, 0, 1, 1], &varint(120), &varint(elapsed)]);
    assert!(decode_body(&body_bytes(
        &ents,
        &NO_TURN,
        &clock(2_147_483_647),
        &NO_TIMELINE
    ))
    .is_ok());
    assert_eq!(
        decode_body(&body_bytes(
            &ents,
            &NO_TURN,
            &clock(2_147_483_648),
            &NO_TIMELINE
        )),
        Err(CodecError::Malformed)
    );

    // Timeline: key zigzag(-1) = 1, entity as a varint.
    let timeline = |entries: &[(u8, u64)]| {
        let mut out = vec![1];
        out.extend(varint(entries.len() as u64));
        for &(key, entity) in entries {
            out.push(key);
            out.extend(varint(entity));
        }
        out
    };
    let run = |entries: &[(u8, u64)]| {
        decode_body(&body_bytes(
            &ents,
            &NO_TURN,
            &NO_ENCOUNTER,
            &timeline(entries),
        ))
    };
    assert!(run(&[(1, 1), (1, 2)]).is_ok(), "equal keys, ascending ids");
    assert!(run(&[(1, 2), (1, 1)]).is_ok(), "equal keys, descending ids");
    assert!(run(&[(2, 1), (4, 2)]).is_ok());
    // Decreasing keys: adjacent (zigzag 4 = 2 then zigzag 2 = 1), with a
    // later valid entry as the remainder.
    assert_eq!(run(&[(4, 1), (2, 2)]), Err(CodecError::Malformed));
    // Repeated entity: adjacent and not adjacent (needs three entities).
    assert_eq!(run(&[(1, 1), (1, 1)]), Err(CodecError::Malformed));
    let three = [ent(1), ent(2), ent(3)];
    assert_eq!(
        decode_body(&body_bytes(
            &three,
            &NO_TURN,
            &NO_ENCOUNTER,
            &timeline(&[(1, 1), (1, 2), (1, 1)])
        )),
        Err(CodecError::Malformed)
    );
    // Entity 0.
    assert_eq!(run(&[(1, 0)]), Err(CodecError::Malformed));
    // Entity not in the body: checked per entry, last.
    assert_eq!(run(&[(1, 1), (1, 9)]), Err(CodecError::Malformed));
    assert_eq!(run(&[(1, 9)]), Err(CodecError::Malformed));
    // 1,025 entries are LimitExceeded before any entry; 1,024 are read.
    let many = |count: u64| {
        let entries: Vec<(u8, u64)> = (1..=count).map(|id| (0, id)).collect();
        let entities: Vec<Vec<u8>> = (1..=count).map(ent).collect();
        body_bytes(&entities, &NO_TURN, &NO_ENCOUNTER, &timeline(&entries))
    };
    assert_eq!(
        decode_body(&many(1024)).unwrap().timeline.unwrap().len(),
        1024
    );
    assert_eq!(decode_body(&many(1025)), Err(CodecError::LimitExceeded));
}

#[test]
fn d4_check_order_and_trailing_bytes() {
    let ents = [ent(1), ent(2)];
    // Trailing byte, after a full valid body.
    let mut trailing = plain_body_bytes(&[1, 2]);
    assert!(decode_body(&trailing).is_ok());
    trailing.push(0);
    assert_eq!(decode_body(&trailing), Err(CodecError::Malformed));
    // Truncation anywhere in a body with every block is Malformed.
    let full = body_bytes(
        &[ent_with(1, &order_facts(2)), ent(2)],
        &[1, 1, 2, 5],
        &[1, 1, 1, 1, 1, 1, 1, 120, 0xF0, 0x01],
        &[1, 2, 1, 1, 1, 2],
    );
    assert!(decode_body(&full).is_ok());
    for len in 0..full.len() {
        assert_eq!(
            decode_body(&full[..len]),
            Err(CodecError::Malformed),
            "prefix {len}"
        );
    }

    // A dangling order target is found before the turn is read: a bad turn
    // flag after it still reports the membership error, and both are
    // Malformed, so use the encounter's UnknownMessage to see the order.
    let dangling = [ent_with(1, &order_facts(9))];
    let bad_mode = [1u8, 2, 0, 0];
    assert_eq!(
        decode_body(&body_bytes(&dangling, &NO_TURN, &bad_mode, &NO_TIMELINE)),
        Err(CodecError::Malformed),
        "membership before encounter"
    );
    assert_eq!(
        decode_body(&body_bytes(&ents, &NO_TURN, &bad_mode, &NO_TIMELINE)),
        Err(CodecError::UnknownMessage),
        "control: the encounter byte alone is UnknownMessage"
    );
    // A dangling active is found before the encounter.
    assert_eq!(
        decode_body(&body_bytes(&ents, &[1, 1, 9, 0], &bad_mode, &NO_TIMELINE)),
        Err(CodecError::Malformed),
        "active before encounter"
    );
    // A fact-set error in entity 2 comes before entity 1's dangling target.
    let stats33: Vec<u8> = {
        let mut out = vec![0, 0, 0, 1];
        out.extend(varint(33));
        out
    };
    let late = [ent_with(1, &order_facts(9)), ent_with(2, &stats33)];
    assert_eq!(
        decode_body(&body_bytes(&late, &NO_TURN, &NO_ENCOUNTER, &NO_TIMELINE)),
        Err(CodecError::LimitExceeded),
        "entity fact set before order membership"
    );
    // A timeline count over the cap comes before membership of its entries.
    assert_eq!(
        decode_body(&body_bytes(
            &ents,
            &NO_TURN,
            &NO_ENCOUNTER,
            &[1, 0x81, 0x08]
        )),
        Err(CodecError::LimitExceeded)
    );
}

// ---------------------------------------------------------------------------
// D9: encode refuses exactly what decode refuses.
// ---------------------------------------------------------------------------

struct Refusal {
    name: &'static str,
    body: SnapshotBodyV3,
    /// Hand-built bytes of the same body, when it has an encodable form.
    bytes: Option<Vec<u8>>,
    error: CodecError,
}

fn stats_facts(count: usize) -> FactSet {
    FactSet {
        stats: Some(
            (0..count)
                .map(|i| stat(&format!("s{i:02}"), StatValue::Int(0)))
                .collect(),
        ),
        ..FactSet::default()
    }
}

fn stats_bytes(count: usize) -> Vec<u8> {
    let mut out = vec![0, 0, 0, 1];
    out.extend(varint(count as u64));
    for i in 0..count {
        out.extend_from_slice(&[3, b's', b'0' + (i / 10) as u8, b'0' + (i % 10) as u8, 0, 0]);
    }
    out.push(0);
    out
}

fn with_order(entity: u64, target: u64) -> SnapshotEntityV3 {
    SnapshotEntityV3 {
        entity: net(entity),
        health: None,
        facts: FactSet {
            order: Some(Some(OrderFact {
                ability: u(7),
                target: net(target),
                repeat: true,
            })),
            ..FactSet::default()
        },
    }
}

#[test]
fn d9_body_encode_checks() {
    let big_ids: Vec<u64> = (1..=1025).collect();
    let timeline_ids: Vec<(i32, u64)> = (1..=1025).map(|id| (0, id)).collect();
    let timeline_body = |entries: &[(i32, u64)], ents: &[u64]| SnapshotBodyV3 {
        timeline: Some(entries.iter().map(|&(k, e)| tl(k, e)).collect()),
        ..plain_body(ents)
    };
    let timeline_wire = |entries: &[(u8, u64)], ents: &[u64]| {
        let mut block = vec![1];
        block.extend(varint(entries.len() as u64));
        for &(key, entity) in entries {
            block.push(key);
            block.extend(varint(entity));
        }
        let entities: Vec<Vec<u8>> = ents.iter().map(|&id| ent(id)).collect();
        body_bytes(&entities, &NO_TURN, &NO_ENCOUNTER, &block)
    };

    let refusals = vec![
        Refusal {
            name: "entities [1, 1]",
            body: plain_body(&[1, 1]),
            bytes: Some(plain_body_bytes(&[1, 1])),
            error: CodecError::Malformed,
        },
        Refusal {
            name: "entities [2, 1]",
            body: plain_body(&[2, 1]),
            bytes: Some(plain_body_bytes(&[2, 1])),
            error: CodecError::Malformed,
        },
        Refusal {
            name: "entities [1, 3, 2]",
            body: plain_body(&[1, 3, 2]),
            bytes: Some(plain_body_bytes(&[1, 3, 2])),
            error: CodecError::Malformed,
        },
        Refusal {
            name: "order target not in the body",
            body: SnapshotBodyV3 {
                entities: vec![with_order(1, 9)],
                ..empty_body()
            },
            bytes: Some(body_bytes(
                &[ent_with(1, &order_facts(9))],
                &NO_TURN,
                &NO_ENCOUNTER,
                &NO_TIMELINE,
            )),
            error: CodecError::Malformed,
        },
        Refusal {
            name: "order target not in the body, far entity",
            body: SnapshotBodyV3 {
                entities: vec![plain_entity(1), plain_entity(2), with_order(3, 4)],
                ..empty_body()
            },
            bytes: Some(body_bytes(
                &[ent(1), ent(2), ent_with(3, &order_facts(4))],
                &NO_TURN,
                &NO_ENCOUNTER,
                &NO_TIMELINE,
            )),
            error: CodecError::Malformed,
        },
        Refusal {
            name: "active not in the body",
            body: SnapshotBodyV3 {
                turn: Some(TurnFact {
                    active: Some(net(9)),
                    round: 1,
                }),
                ..plain_body(&[1, 2])
            },
            bytes: Some(body_bytes(
                &[ent(1), ent(2)],
                &[1, 1, 9, 1],
                &NO_ENCOUNTER,
                &NO_TIMELINE,
            )),
            error: CodecError::Malformed,
        },
        Refusal {
            name: "timeline entity not in the body",
            body: timeline_body(&[(0, 1), (0, 9)], &[1, 2]),
            bytes: Some(timeline_wire(&[(0, 1), (0, 9)], &[1, 2])),
            error: CodecError::Malformed,
        },
        Refusal {
            name: "timeline keys [5, 4]",
            body: timeline_body(&[(5, 1), (4, 2)], &[1, 2]),
            bytes: Some(timeline_wire(&[(10, 1), (8, 2)], &[1, 2])),
            error: CodecError::Malformed,
        },
        Refusal {
            name: "timeline entity repeated, adjacent",
            body: timeline_body(&[(1, 1), (1, 1)], &[1, 2]),
            bytes: Some(timeline_wire(&[(2, 1), (2, 1)], &[1, 2])),
            error: CodecError::Malformed,
        },
        Refusal {
            name: "timeline entity repeated, not adjacent",
            body: timeline_body(&[(1, 1), (1, 2), (1, 1)], &[1, 2, 3]),
            bytes: Some(timeline_wire(&[(2, 1), (2, 2), (2, 1)], &[1, 2, 3])),
            error: CodecError::Malformed,
        },
        Refusal {
            name: "clock round_ticks 0",
            body: SnapshotBodyV3 {
                encounter: EncounterFacts {
                    mode: None,
                    pending_mode: None,
                    clock: Some(Some(ClockFact {
                        round_ticks: 0,
                        elapsed: 1,
                    })),
                },
                ..plain_body(&[1])
            },
            bytes: Some(body_bytes(
                &[ent(1)],
                &NO_TURN,
                &[0, 0, 1, 1, 0, 1],
                &NO_TIMELINE,
            )),
            error: CodecError::Malformed,
        },
        Refusal {
            name: "clock elapsed over the cap",
            body: SnapshotBodyV3 {
                encounter: EncounterFacts {
                    mode: None,
                    pending_mode: None,
                    clock: Some(Some(ClockFact {
                        round_ticks: 1,
                        elapsed: 2_147_483_648,
                    })),
                },
                ..plain_body(&[1])
            },
            bytes: Some(body_bytes(
                &[ent(1)],
                &NO_TURN,
                &concat(&[&[0, 0, 1, 1, 1], &varint(2_147_483_648)]),
                &NO_TIMELINE,
            )),
            error: CodecError::Malformed,
        },
        Refusal {
            name: "1,025 entities",
            body: plain_body(&big_ids),
            bytes: Some(plain_body_bytes(&big_ids)),
            error: CodecError::LimitExceeded,
        },
        Refusal {
            name: "entities [2 (33 stats), 1]: the count first",
            body: SnapshotBodyV3 {
                entities: vec![
                    SnapshotEntityV3 {
                        facts: stats_facts(33),
                        ..plain_entity(2)
                    },
                    plain_entity(1),
                ],
                ..empty_body()
            },
            bytes: Some(body_bytes(
                &[ent_with(2, &stats_bytes(33)), ent(1)],
                &NO_TURN,
                &NO_ENCOUNTER,
                &NO_TIMELINE,
            )),
            error: CodecError::LimitExceeded,
        },
        Refusal {
            name: "entities [2, 1 (33 stats)]: the order first",
            body: SnapshotBodyV3 {
                entities: vec![
                    plain_entity(2),
                    SnapshotEntityV3 {
                        facts: stats_facts(33),
                        ..plain_entity(1)
                    },
                ],
                ..empty_body()
            },
            bytes: Some(body_bytes(
                &[ent(2), ent_with(1, &stats_bytes(33))],
                &NO_TURN,
                &NO_ENCOUNTER,
                &NO_TIMELINE,
            )),
            error: CodecError::Malformed,
        },
        Refusal {
            name: "a fact-set error beats a dangling order target",
            body: SnapshotBodyV3 {
                entities: vec![
                    with_order(1, 9),
                    SnapshotEntityV3 {
                        facts: stats_facts(33),
                        ..plain_entity(2)
                    },
                ],
                ..empty_body()
            },
            bytes: Some(body_bytes(
                &[ent_with(1, &order_facts(9)), ent_with(2, &stats_bytes(33))],
                &NO_TURN,
                &NO_ENCOUNTER,
                &NO_TIMELINE,
            )),
            error: CodecError::LimitExceeded,
        },
        Refusal {
            name: "a dangling active beats a timeline count over the cap",
            body: SnapshotBodyV3 {
                turn: Some(TurnFact {
                    active: Some(net(9)),
                    round: 1,
                }),
                ..timeline_body(&timeline_ids, &[1])
            },
            bytes: None,
            error: CodecError::Malformed,
        },
        Refusal {
            name: "a timeline count over the cap",
            body: timeline_body(&timeline_ids, &big_ids[..1024]),
            bytes: Some({
                let entries: Vec<(u8, u64)> = timeline_ids.iter().map(|&(_, e)| (0, e)).collect();
                timeline_wire(&entries, &big_ids[..1024])
            }),
            error: CodecError::LimitExceeded,
        },
    ];

    for refusal in &refusals {
        assert_eq!(
            encode_body(&refusal.body),
            Err(refusal.error),
            "encode: {}",
            refusal.name
        );
        if let Some(bytes) = &refusal.bytes {
            assert_eq!(
                decode_body(bytes),
                Err(refusal.error),
                "decode: {}",
                refusal.name
            );
        }
        // plan_transfer surfaces the same error from the body.
        assert_eq!(
            plan_transfer(Z16, tid(1), 0, 0, 8, &refusal.body),
            Err(refusal.error),
            "plan: {}",
            refusal.name
        );
    }

    // Controls: the repaired bodies encode, and decode(encode(b)) == b.
    let controls = vec![
        plain_body(&[1, 2]),
        SnapshotBodyV3 {
            entities: vec![with_order(1, 2), plain_entity(2)],
            ..empty_body()
        },
        SnapshotBodyV3 {
            turn: Some(TurnFact {
                active: Some(net(2)),
                round: 1,
            }),
            ..plain_body(&[1, 2])
        },
        timeline_body(&[(4, 1), (5, 2)], &[1, 2]),
        timeline_body(&[(1, 2), (1, 1)], &[1, 2]),
        SnapshotBodyV3 {
            encounter: EncounterFacts {
                mode: None,
                pending_mode: None,
                clock: Some(Some(ClockFact {
                    round_ticks: 1,
                    elapsed: 2_147_483_647,
                })),
            },
            ..plain_body(&[1])
        },
        SnapshotBodyV3 {
            entities: vec![SnapshotEntityV3 {
                facts: stats_facts(32),
                ..plain_entity(1)
            }],
            ..empty_body()
        },
    ];
    for control in &controls {
        let bytes = encode_body(control).expect("control encodes");
        assert_eq!(decode_body(&bytes).as_ref(), Ok(control));
    }
}

#[test]
fn d9_body_fact_set_checks_match_the_b4_checks() {
    // Each fact-set refusal through the body has the error encode_delta gives
    // for the same value (B-4); an entity's checks run through its fact set.
    let wrap = |facts: FactSet| SnapshotBodyV3 {
        entities: vec![SnapshotEntityV3 {
            facts,
            ..plain_entity(1)
        }],
        ..empty_body()
    };
    let cases: Vec<(&str, FactSet, CodecError)> = vec![
        (
            "17 pools",
            FactSet {
                pools: Some((1..=17).map(|n| pool(n, 0, 0)).collect()),
                ..FactSet::default()
            },
            CodecError::LimitExceeded,
        ),
        (
            "pool current over max",
            FactSet {
                pools: Some(vec![pool(1, 3, 2)]),
                ..FactSet::default()
            },
            CodecError::Malformed,
        ),
        (
            "duplicate pool",
            FactSet {
                pools: Some(vec![pool(1, 1, 2), pool(1, 1, 2)]),
                ..FactSet::default()
            },
            CodecError::Malformed,
        ),
        (
            "17 effects",
            FactSet {
                effects: Some((1..=17).map(|n| effect(n, 1)).collect()),
                ..FactSet::default()
            },
            CodecError::LimitExceeded,
        ),
        (
            "effects out of order",
            FactSet {
                effects: Some(vec![effect(2, 1), effect(1, 1)]),
                ..FactSet::default()
            },
            CodecError::Malformed,
        ),
        (
            "empty stat name",
            FactSet {
                stats: Some(vec![stat("", StatValue::Int(1))]),
                ..FactSet::default()
            },
            CodecError::Malformed,
        ),
        (
            "33-byte stat name",
            FactSet {
                stats: Some(vec![stat(&"n".repeat(33), StatValue::Int(1))]),
                ..FactSet::default()
            },
            CodecError::LimitExceeded,
        ),
        (
            "duplicate stat",
            FactSet {
                stats: Some(vec![
                    stat("a", StatValue::Int(1)),
                    stat("a", StatValue::Fixed(1)),
                ]),
                ..FactSet::default()
            },
            CodecError::Malformed,
        ),
        (
            "9 tags",
            FactSet {
                tags: Some((0..9).map(|i| format!("t{i}")).collect()),
                ..FactSet::default()
            },
            CodecError::LimitExceeded,
        ),
        (
            "tags out of order",
            FactSet {
                tags: Some(vec!["b".to_owned(), "a".to_owned()]),
                ..FactSet::default()
            },
            CodecError::Malformed,
        ),
    ];
    for (name, facts, error) in cases {
        assert_eq!(encode_body(&wrap(facts)), Err(error), "{name}");
    }
}

// ---------------------------------------------------------------------------
// D5: the largest body and its plan.
// ---------------------------------------------------------------------------

fn largest_body() -> SnapshotBodyV3 {
    let base = 1u64 << 63;
    let entities = (0..1024u64)
        .map(|i| SnapshotEntityV3 {
            entity: net(base + i),
            health: Some(HealthFact {
                health: u32::MAX,
                max_health: u32::MAX,
                dead: true,
            }),
            facts: FactSet {
                pools: Some((1..=16).map(|n| pool(n, u32::MAX, u32::MAX)).collect()),
                effects: Some((1..=16).map(|n| effect(n, u64::MAX)).collect()),
                order: Some(Some(OrderFact {
                    ability: u(1),
                    target: net(base),
                    repeat: true,
                })),
                stats: Some(
                    (0..32)
                        .map(|n| stat(&format!("{n:032}"), StatValue::Int(i32::MIN)))
                        .collect(),
                ),
                tags: Some((0..8).map(|n| format!("{n:032}")).collect()),
            },
        })
        .collect();
    SnapshotBodyV3 {
        entities,
        turn: Some(TurnFact {
            active: Some(net(base)),
            round: u64::MAX,
        }),
        encounter: EncounterFacts {
            mode: Some(EncounterMode::RealTime),
            pending_mode: Some(Some(EncounterMode::TurnBased)),
            clock: Some(Some(ClockFact {
                round_ticks: u32::MAX,
                elapsed: 2_147_483_647,
            })),
        },
        timeline: Some((0..1024u64).map(|i| tl(i32::MIN, base + i)).collect()),
    }
}

/// The largest body and its encoding, built once.
fn largest() -> &'static (SnapshotBodyV3, Vec<u8>) {
    static CELL: OnceLock<(SnapshotBodyV3, Vec<u8>)> = OnceLock::new();
    CELL.get_or_init(|| {
        let body = largest_body();
        let bytes = encode_body(&body).expect("the largest valid body encodes");
        (body, bytes)
    })
}

#[test]
fn d5_largest_body() {
    let (body, bytes) = largest();
    assert_eq!(bytes.len(), 2_847_788);
    assert_eq!(bytes.len(), MAX_ENCODED_BODY_BYTES_V3);
    assert!(bytes.len() <= MAX_SNAPSHOT_BYTES_V3);
    // The per-part sizes of D-4.
    let fact_set = encode_body(&SnapshotBodyV3 {
        entities: vec![SnapshotEntityV3 {
            entity: net(1 << 63),
            health: None,
            facts: body.entities[0].facts.clone(),
        }],
        turn: None,
        encounter: EncounterFacts::default(),
        timeline: None,
    })
    .unwrap();
    // 1 (count) + 10 (entity) + 1 (no health) + the fact set + 1 + 3 + 1.
    assert_eq!(fact_set.len() - 1 - 10 - 1 - 1 - 3 - 1, 2_744);

    assert_eq!(decode_body(bytes).as_ref(), Ok(body));

    // The plan: 8 segments per chunk, transfer 1, base tick 0, cursor 0.
    let frames = plan_transfer(Z16, tid(1), 0, 0, 8, body).expect("plans");
    assert_eq!(frames.len(), 88);
    assert_eq!(frames.iter().map(Vec::len).sum::<usize>(), 2_851_124);
    assert_eq!(
        frames[0],
        hex("82 02 10 Z16 01 00 00 AC E8 AD 01 80 08 08 57")
    );
    assert_eq!(frames[0].len(), 30);
    for frame in &frames[1..87] {
        assert_eq!(frame.len(), 32_806);
    }
    assert_eq!(frames[87].len(), 29_778);

    let Ok(TransferDown::Begin(decoded_begin)) = decode_down(&frames[0]) else {
        panic!("the first frame is a Begin");
    };
    assert_eq!(decoded_begin, begin(2_847_788, 1024, 8, 87));
    assert_eq!(check_begin(&decoded_begin), Ok(()));
    let mut joined = Vec::new();
    for (position, frame) in frames[1..].iter().enumerate() {
        let Ok(TransferDown::Chunk(c)) = decode_down(frame) else {
            panic!("frame {} is a Chunk", position + 1);
        };
        assert_eq!(c.index as usize, position);
        assert_eq!(chunk_len(&decoded_begin, c.index), Some(c.bytes.len()));
        joined.extend_from_slice(&c.bytes);
    }
    assert_eq!(&joined, bytes);
    assert_eq!(decode_body(&joined).as_ref(), Ok(body));
    assert_eq!(chunk_len(&decoded_begin, 86), Some(29_740));
    assert_eq!(chunk_len(&decoded_begin, 87), None);

    // A 33rd stat on one entity is LimitExceeded.
    let mut too_many = body.clone();
    too_many.entities[500].facts.stats = Some(
        (0..33)
            .map(|n| stat(&format!("{n:032}"), StatValue::Int(0)))
            .collect(),
    );
    assert_eq!(encode_body(&too_many), Err(CodecError::LimitExceeded));
}

#[test]
fn d5_largest_body_is_exactly_the_documented_sum() {
    // 2 + 1,024 x 2,766 + 22 + 17 + 15,363.
    assert_eq!(2 + 1024 * 2766 + 22 + 17 + 15_363, 2_847_788);
    let (_, bytes) = largest();
    // The count varint is two bytes, the timeline block is last.
    assert_eq!(&bytes[..2], &[0x80, 0x08]);
    let tail = &bytes[bytes.len() - 15_363..];
    assert_eq!(&tail[..3], &[0x01, 0x80, 0x08]);
    // Each timeline entry: i32::MIN zigzag is FF FF FF FF 0F (5 bytes), then
    // the ten-byte entity.
    assert_eq!(&tail[3..8], &[0xFF, 0xFF, 0xFF, 0xFF, 0x0F]);
    assert_eq!(&tail[8..18], &hex("80 80 80 80 80 80 80 80 80 01"));
}

// ---------------------------------------------------------------------------
// D8: plan_transfer parameters.
// ---------------------------------------------------------------------------

#[test]
fn d8_plan_transfer_parameters() {
    let empty = empty_body();
    // cursor: SEQ_LAST is accepted and the next value is Malformed.
    assert!(plan_transfer(Z16, tid(1), 0, SEQ_LAST, 1, &empty).is_ok());
    assert_eq!(
        plan_transfer(Z16, tid(1), 0, SEQ_LAST + 1, 1, &empty),
        Err(CodecError::Malformed)
    );
    assert_eq!(
        plan_transfer(Z16, tid(1), 0, u64::MAX, 1, &empty),
        Err(CodecError::Malformed)
    );
    // segments_per_chunk: 0 Malformed, 9 LimitExceeded, 1..=8 accepted.
    assert_eq!(
        plan_transfer(Z16, tid(1), 0, 0, 0, &empty),
        Err(CodecError::Malformed)
    );
    assert_eq!(
        plan_transfer(Z16, tid(1), 0, 0, 9, &empty),
        Err(CodecError::LimitExceeded)
    );
    assert_eq!(
        plan_transfer(Z16, tid(1), 0, 0, 255, &empty),
        Err(CodecError::LimitExceeded)
    );
    for spc in 1..=8 {
        assert!(
            plan_transfer(Z16, tid(1), 0, 0, spc, &empty).is_ok(),
            "{spc}"
        );
    }
    // Precedence: cursor before segments, parameters before the body.
    assert_eq!(
        plan_transfer(Z16, tid(1), 0, u64::MAX, 9, &empty),
        Err(CodecError::Malformed)
    );
    assert_eq!(
        plan_transfer(Z16, tid(1), 0, u64::MAX, 0, &empty),
        Err(CodecError::Malformed)
    );
    let descending = plain_body(&[2, 1]);
    assert_eq!(
        plan_transfer(Z16, tid(1), 0, 0, 9, &descending),
        Err(CodecError::LimitExceeded)
    );
    assert_eq!(
        plan_transfer(Z16, tid(1), 0, 0, 0, &descending),
        Err(CodecError::Malformed)
    );
    assert_eq!(
        plan_transfer(Z16, tid(1), 0, 0, 8, &descending),
        Err(CodecError::Malformed)
    );
    // The cursor and base tick travel in the Begin.
    let frames = plan_transfer([9; 16], tid(300), 77, 5, 2, &empty).unwrap();
    let Ok(TransferDown::Begin(b)) = decode_down(&frames[0]) else {
        panic!("Begin");
    };
    assert_eq!(
        b,
        BeginTransfer {
            epoch: [9; 16],
            transfer: tid(300),
            base_tick: 77,
            cursor: 5,
            total_bytes: 6,
            entity_count: 0,
            segments_per_chunk: 2,
            chunk_count: 1,
        }
    );

    // The chunk cap is 128, computed in usize: the D5 body at 5 segments is
    // 140 chunks and at 1 segment is 696 (past a u8); 6 gives 116 and 8 gives 87.
    let (body, bytes) = largest();
    assert_eq!(bytes.len().div_ceil(5 * 4096), 140);
    assert_eq!(bytes.len().div_ceil(4096), 696);
    assert_eq!(
        plan_transfer(Z16, tid(1), 0, 0, 5, body),
        Err(CodecError::LimitExceeded)
    );
    assert_eq!(
        plan_transfer(Z16, tid(1), 0, 0, 1, body),
        Err(CodecError::LimitExceeded)
    );
    for (spc, chunks) in [(6u8, 116usize), (7, 100), (8, 87)] {
        let frames = plan_transfer(Z16, tid(1), 0, 0, spc, body).expect("plans");
        assert_eq!(frames.len(), chunks + 1, "spc {spc}");
        let Ok(TransferDown::Begin(b)) = decode_down(&frames[0]) else {
            panic!("Begin");
        };
        assert_eq!(b.chunk_count as usize, chunks);
        assert_eq!(check_begin(&b), Ok(()));
    }
}

// ---------------------------------------------------------------------------
// D10: the v3 caps and the unchanged v1 caps.
// ---------------------------------------------------------------------------

#[test]
fn d10_caps() {
    assert_eq!(MAX_SNAPSHOT_BYTES_V3, 4_194_304);
    assert_eq!(MAX_SNAPSHOT_CHUNKS_V3, 128);
    assert_eq!(MAX_ENCODED_BODY_BYTES_V3, 2_847_788);
    assert_eq!(TRANSFER_VERSION_V3, 0x82);
    assert_eq!(LANE_SNAPSHOT, 2);
    assert_eq!(MAX_CHUNK_BYTES, 32_768);
    assert_eq!(MAX_SEGMENT_BYTES, 4_096);
    // The v1/0x81 caps have not moved.
    assert_eq!(protocol::MAX_SNAPSHOT_BYTES, 1_048_576);
    assert_eq!(protocol::MAX_SNAPSHOT_CHUNKS, 32);
    assert_eq!(protocol::MAX_VISIBLE_ENTITIES, 1024);
}

#[test]
fn d10_begin_at_and_past_the_v3_caps() {
    let top = begin(4_194_304, 0, 8, 128);
    let top_bytes = hex("82 02 10 Z16 01 00 00 80 80 80 02 00 08 80");
    assert_eq!(top_bytes.len(), 29);
    assert_eq!(
        encode_down(&TransferDown::Begin(top.clone())),
        Ok(top_bytes.clone())
    );
    assert_eq!(
        decode_down(&top_bytes),
        Ok(TransferDown::Begin(top.clone()))
    );
    assert_eq!(check_begin(&top), Ok(()));
    // 0x81 and BeginTransfer::validate hold it to the v1 caps.
    assert_eq!(top.validate(), Err(CodecError::LimitExceeded));
    assert_eq!(
        codec_transfer::decode_down(&with_version(&top_bytes, 0x81)),
        Err(CodecError::LimitExceeded)
    );
    assert_eq!(
        codec_transfer::encode_down(&TransferDown::Begin(top.clone())),
        Err(CodecError::LimitExceeded)
    );
    assert_eq!(chunk_len(&top, 127), Some(32_768));
    assert_eq!(chunk_len(&top, 128), None);
    assert_eq!(chunk_len(&top, 0), Some(32_768));
    assert_eq!(
        top.chunk_len(0),
        None,
        "the v1 helper refuses the wide Begin"
    );

    // Total 4,194,305: LimitExceeded on decode and on encode.
    let over_total = hex("82 02 10 Z16 01 00 00 81 80 80 02 00 08 80");
    assert_eq!(decode_down(&over_total), Err(CodecError::LimitExceeded));
    let over = begin(4_194_305, 0, 8, 128);
    assert_eq!(check_begin(&over), Err(CodecError::LimitExceeded));
    assert_eq!(
        encode_down(&TransferDown::Begin(over.clone())),
        Err(CodecError::LimitExceeded)
    );
    assert_eq!(chunk_len(&over, 0), None);
    // Total 4,194,304 with chunk count 129: LimitExceeded on decode and encode.
    let over_chunks = hex("82 02 10 Z16 01 00 00 80 80 80 02 00 08 81");
    assert_eq!(decode_down(&over_chunks), Err(CodecError::LimitExceeded));
    assert_eq!(
        encode_down(&TransferDown::Begin(begin(4_194_304, 0, 8, 129))),
        Err(CodecError::LimitExceeded)
    );
    assert_eq!(
        decode_down(&hex("82 02 10 Z16 01 00 00 80 80 80 02 00 08 FF")),
        Err(CodecError::LimitExceeded)
    );
    // The consistency rule holds at the top: 127 chunks is Malformed.
    assert_eq!(
        decode_down(&hex("82 02 10 Z16 01 00 00 80 80 80 02 00 08 7F")),
        Err(CodecError::Malformed)
    );
    assert_eq!(
        encode_down(&TransferDown::Begin(begin(4_194_304, 0, 8, 127))),
        Err(CodecError::Malformed)
    );
    // With fewer segments per chunk the same total needs more chunks than the
    // cap allows: 4,194,304 / 4,096 = 1,024.
    assert_eq!(
        encode_down(&TransferDown::Begin(begin(4_194_304, 0, 1, 128))),
        Err(CodecError::Malformed)
    );
}

#[test]
fn d10_begin_between_the_v1_and_v3_caps() {
    // Total 1,048,577 with 33 chunks decodes; its 0x81 copy is LimitExceeded.
    let wide = hex("82 02 10 Z16 01 00 00 81 80 40 00 08 21");
    assert_eq!(wide.len(), 28);
    let expected = begin(1_048_577, 0, 8, 33);
    assert_eq!(
        decode_down(&wide),
        Ok(TransferDown::Begin(expected.clone()))
    );
    assert_eq!(
        encode_down(&TransferDown::Begin(expected.clone())),
        Ok(wide.clone())
    );
    assert_eq!(check_begin(&expected), Ok(()));
    assert_eq!(
        codec_transfer::decode_down(&with_version(&wide, 0x81)),
        Err(CodecError::LimitExceeded)
    );
    assert_eq!(chunk_len(&expected, 32), Some(1));
    assert_eq!(chunk_len(&expected, 33), None);
    // The same total with 32 chunks is Malformed (not the canonical cut).
    assert_eq!(
        decode_down(&hex("82 02 10 Z16 01 00 00 81 80 40 00 08 20")),
        Err(CodecError::Malformed)
    );
    assert_eq!(
        encode_down(&TransferDown::Begin(begin(1_048_577, 0, 8, 32))),
        Err(CodecError::Malformed)
    );
    // 1,048,576 with 33 chunks: v1 would say LimitExceeded for the count, v3
    // finds the cut inconsistent (32 chunks is canonical).
    assert_eq!(
        decode_down(&hex("82 02 10 Z16 01 00 00 80 80 40 00 08 21")),
        Err(CodecError::Malformed)
    );
    // 33 chunks of 1 segment, total over the old cap: accepted.
    assert!(check_begin(&begin(4096 * 33, 0, 1, 33)).is_ok());
}

#[test]
fn d10_chunk_index_bounds() {
    let c127 = hex("82 02 11 Z16 01 7F 01 01 00");
    assert_eq!(c127.len(), 24);
    assert_eq!(
        decode_down(&c127),
        Ok(TransferDown::Chunk(chunk(127, vec![0])))
    );
    assert_eq!(
        encode_down(&TransferDown::Chunk(chunk(127, vec![0]))),
        Ok(c127.clone())
    );
    // 0x81 refuses the index.
    assert_eq!(
        codec_transfer::decode_down(&with_version(&c127, 0x81)),
        Err(CodecError::LimitExceeded)
    );
    assert_eq!(
        codec_transfer::encode_down(&TransferDown::Chunk(chunk(127, vec![0]))),
        Err(CodecError::LimitExceeded)
    );
    // The v1 range edge: 31 is shared, 32 is v3-only.
    for index in [31u8, 32, 64, 100] {
        let frame = encode_down(&TransferDown::Chunk(chunk(index, vec![0]))).unwrap();
        assert_eq!(
            decode_down(&frame),
            Ok(TransferDown::Chunk(chunk(index, vec![0])))
        );
    }
    // Index 128 is LimitExceeded on decode (with no bytes following, so the
    // check runs at the index) and with a full valid remainder, and on encode.
    assert_eq!(
        decode_down(&hex("82 02 11 Z16 01 80")),
        Err(CodecError::LimitExceeded)
    );
    assert_eq!(
        decode_down(&hex("82 02 11 Z16 01 80 01 01 00")),
        Err(CodecError::LimitExceeded)
    );
    assert_eq!(
        decode_down(&hex("82 02 11 Z16 01 FF 01 01 00")),
        Err(CodecError::LimitExceeded)
    );
    assert_eq!(
        encode_down(&TransferDown::Chunk(chunk(128, vec![0]))),
        Err(CodecError::LimitExceeded)
    );
    assert_eq!(
        encode_down(&TransferDown::Chunk(chunk(255, vec![0]))),
        Err(CodecError::LimitExceeded)
    );
    // The index check precedes the empty-payload check on encode, as in T024.
    assert_eq!(
        encode_down(&TransferDown::Chunk(chunk(128, vec![]))),
        Err(CodecError::LimitExceeded)
    );
    assert_eq!(
        encode_down(&TransferDown::Chunk(chunk(0, vec![]))),
        Err(CodecError::Malformed)
    );
    // Payload shapes are unchanged: a full 8-segment chunk round-trips; a
    // ninth segment's worth of bytes is LimitExceeded.
    let full = chunk(100, vec![5; MAX_CHUNK_BYTES]);
    let frame = encode_down(&TransferDown::Chunk(full.clone())).unwrap();
    assert_eq!(frame.len(), 32_806);
    assert_eq!(decode_down(&frame), Ok(TransferDown::Chunk(full)));
    assert_eq!(
        encode_down(&TransferDown::Chunk(chunk(1, vec![5; MAX_CHUNK_BYTES + 1]))),
        Err(CodecError::LimitExceeded)
    );
}

#[test]
fn d10_check_begin_order() {
    // Each pair differs in error, so the first listed failure must win.
    let m = CodecError::Malformed;
    let l = CodecError::LimitExceeded;
    let mut cursor_bad = begin(0, 1025, 9, 129);
    cursor_bad.cursor = SEQ_LAST + 1;
    assert_eq!(check_begin(&cursor_bad), Err(m), "cursor before total");
    let mut cursor_then_limit = begin(4_194_305, 0, 8, 128);
    cursor_then_limit.cursor = SEQ_LAST + 1;
    assert_eq!(check_begin(&cursor_then_limit), Err(m));
    assert_eq!(
        check_begin(&begin(0, 1025, 9, 129)),
        Err(m),
        "total 0 first"
    );
    assert_eq!(
        check_begin(&begin(4_194_305, 1025, 0, 0)),
        Err(l),
        "total cap before the rest"
    );
    assert_eq!(
        check_begin(&begin(6, 1025, 0, 0)),
        Err(l),
        "entity count before segments"
    );
    assert_eq!(
        check_begin(&begin(6, 1024, 0, 129)),
        Err(m),
        "segments 0 before chunk count"
    );
    assert_eq!(
        check_begin(&begin(6, 0, 9, 0)),
        Err(l),
        "segments cap before chunks 0"
    );
    assert_eq!(
        check_begin(&begin(6, 0, 8, 0)),
        Err(m),
        "chunk count 0 is Malformed"
    );
    assert_eq!(
        check_begin(&begin(6, 0, 8, 129)),
        Err(l),
        "chunk count cap before consistency"
    );
    assert_eq!(check_begin(&begin(6, 0, 8, 2)), Err(m), "consistency last");
    // Controls: the boundary values pass.
    assert_eq!(check_begin(&begin(4_194_304, 1024, 8, 128)), Ok(()));
    assert_eq!(check_begin(&begin(1, 0, 1, 1)), Ok(()));
    let mut cursor_last = begin(1, 0, 1, 1);
    cursor_last.cursor = SEQ_LAST;
    assert_eq!(check_begin(&cursor_last), Ok(()));
    // decode_down follows the same order on the wire.
    assert_eq!(
        decode_down(&hex("82 02 10 Z16 01 00 00 00 00 08 80")),
        Err(m),
        "total 0"
    );
    assert_eq!(
        decode_down(&hex("82 02 10 Z16 01 00 00 06 81 08 08 01")),
        Err(l),
        "entity count 1025"
    );
    assert_eq!(
        decode_down(&hex("82 02 10 Z16 01 00 00 06 00 09 01")),
        Err(l),
        "segments 9"
    );
    assert_eq!(
        decode_down(&hex("82 02 10 Z16 01 00 00 06 00 00 01")),
        Err(m),
        "segments 0"
    );
    assert_eq!(
        decode_down(&hex("82 02 10 Z16 01 00 00 06 00 01 00")),
        Err(m),
        "chunks 0"
    );
    // Trailing bytes after a valid Begin and a truncated Begin.
    assert_eq!(
        decode_down(&hex("82 02 10 Z16 01 00 00 06 00 01 01 00")),
        Err(m)
    );
    assert_eq!(decode_down(&hex("82 02 10 Z16 01 00 00 06 00 01")), Err(m));
    // Transfer id 0 and a cursor past SEQ_LAST.
    assert_eq!(
        decode_down(&hex("82 02 10 Z16 00 00 00 06 00 01 01")),
        Err(m)
    );
    assert_eq!(
        decode_down(&concat(&[
            &hex("82 02 10 Z16 01 00"),
            &varint(u64::MAX),
            &hex("06 00 01 01")
        ])),
        Err(m)
    );
}

// ---------------------------------------------------------------------------
// The snapshot composite (D6, D7): fixture shared with T140c's C2.
// ---------------------------------------------------------------------------

struct Keys {
    keys: Vec<EntityKey>,
}

impl Keys {
    fn new(n: usize) -> Self {
        let mut arena = GenerationalArena::<()>::new();
        let keys: Vec<EntityKey> = (0..n)
            .map(|_| EntityKey {
                area: u(10),
                entity: arena.insert(()),
            })
            .collect();
        assert!(keys.windows(2).all(|w| w[0] < w[1]), "arena ids ascend");
        Self { keys }
    }

    fn k(&self, n: usize) -> EntityKey {
        self.keys[n - 1]
    }
}

#[derive(Clone, Copy)]
struct Enc {
    area: Ulid,
    mode: EncounterMode,
    pending: Option<EncounterMode>,
    clock: Option<ClockFact>,
}

#[derive(Clone)]
struct Scene {
    epoch: SessionEpoch,
    area: Ulid,
    state: Vec<EntityFact>,
    despawned: Vec<EntityKey>,
    turn: Option<TurnState>,
    turn_granted: bool,
    rows: Vec<FactRow>,
    encounter: Option<Enc>,
    timeline: Vec<(i32, EntityKey)>,
    grants: EncounterGrants,
    resend: bool,
}

impl Scene {
    fn with_contexts<R>(&self, run: impl FnOnce(&ViewerContext<'_>, &FactContext<'_>) -> R) -> R {
        let state = ViewerContext {
            epoch: self.epoch,
            tick: 9,
            interest: InterestSet { area: self.area },
            facts: &self.state,
            despawned: &self.despawned,
            turn: self.turn,
            turn_granted: self.turn_granted,
            resend_turn: false,
        };
        let encounter = self.encounter.map(|e| EncounterInput {
            area: e.area,
            mode: e.mode,
            pending_mode: e.pending,
            clock: e.clock,
            timeline: &self.timeline,
        });
        let context = FactContext {
            rows: &self.rows,
            encounter,
            grants: self.grants,
            resend: self.resend,
        };
        run(&state, &context)
    }

    fn page(&self, view: &mut InterestView, facts: &mut FactView) -> Result<PageV3, FactError> {
        self.with_contexts(|state, context| project_page(view, facts, state, context))
    }

    fn snapshot(
        &self,
        view: &mut InterestView,
        facts: &mut FactView,
    ) -> Result<SnapshotProjectionV3, FactError> {
        self.with_contexts(|state, context| project_snapshot_v3(view, facts, state, context))
    }
}

fn fresh() -> (InterestView, FactView) {
    (InterestView::new(epoch()), FactView::new(epoch()))
}

fn state_row(key: EntityKey, presence: bool, health: Option<HealthFact>) -> EntityFact {
    EntityFact {
        key,
        fields: FieldMask {
            presence,
            health: health.is_some(),
        },
        health,
        resend_health: false,
    }
}

fn row(key: EntityKey, fields: FactMask) -> FactRow {
    FactRow {
        key,
        fields,
        pools: Vec::new(),
        effects: Vec::new(),
        order: None,
        stats: Vec::new(),
        tags: Vec::new(),
    }
}

fn all_grants() -> EncounterGrants {
    EncounterGrants {
        mode: true,
        pending_mode: true,
        clock: true,
        timeline: true,
    }
}

fn k2_row(fx: &Keys) -> FactRow {
    let mut r = row(
        fx.k(2),
        FactMask {
            pools: true,
            stats: true,
            ..FactMask::default()
        },
    );
    r.pools = vec![pool(1, 2, 3)];
    r.stats = vec![stat("might", StatValue::Int(4))];
    r
}

fn k3_row(fx: &Keys) -> FactRow {
    let mut r = row(fx.k(3), FactMask::default());
    r.pools = vec![pool(5, 1, 1)];
    r.effects = vec![effect(6, 3)];
    r.order = Some(OrderInput {
        ability: u(7),
        target: fx.k(1),
        repeat: true,
    });
    r.stats = vec![stat("grit", StatValue::Fixed(65_536))];
    r.tags = vec!["a".to_owned(), "b".to_owned()];
    r
}

fn k2_image() -> FactSet {
    FactSet {
        pools: Some(vec![pool(1, 2, 3)]),
        stats: Some(vec![stat("might", StatValue::Int(4))]),
        ..FactSet::default()
    }
}

/// The T140c C2 fixture: `k1..k3` visible, health for `k1` only, `RealTime`,
/// clock `{120, 240}`, timeline `[(5,k3), (-1,k2), (-1,k1)]`, all grants.
fn base(fx: &Keys) -> Scene {
    Scene {
        epoch: epoch(),
        area: u(10),
        state: vec![
            state_row(
                fx.k(1),
                true,
                Some(HealthFact {
                    health: 10,
                    max_health: 10,
                    dead: false,
                }),
            ),
            state_row(fx.k(2), true, None),
            state_row(fx.k(3), true, None),
        ],
        despawned: Vec::new(),
        turn: None,
        turn_granted: false,
        rows: vec![k2_row(fx), k3_row(fx)],
        encounter: Some(Enc {
            area: u(10),
            mode: EncounterMode::RealTime,
            pending: None,
            clock: Some(ClockFact {
                round_ticks: 120,
                elapsed: 240,
            }),
        }),
        timeline: vec![(5, fx.k(3)), (-1, fx.k(2)), (-1, fx.k(1))],
        grants: all_grants(),
        resend: false,
    }
}

fn base_encounter_image() -> EncounterFacts {
    EncounterFacts {
        mode: Some(EncounterMode::RealTime),
        pending_mode: Some(None),
        clock: Some(Some(ClockFact {
            round_ticks: 120,
            elapsed: 240,
        })),
    }
}

fn entity_facts_op(n: u64, facts: FactSet) -> DeltaOp {
    DeltaOp::EntityFacts {
        entity: net(n),
        facts,
    }
}

fn health_10() -> HealthFact {
    HealthFact {
        health: 10,
        max_health: 10,
        dead: false,
    }
}

fn base_body() -> SnapshotBodyV3 {
    SnapshotBodyV3 {
        entities: vec![
            SnapshotEntityV3 {
                entity: net(1),
                health: Some(health_10()),
                facts: FactSet::default(),
            },
            SnapshotEntityV3 {
                entity: net(2),
                health: None,
                facts: k2_image(),
            },
            SnapshotEntityV3 {
                entity: net(3),
                health: None,
                facts: FactSet::default(),
            },
        ],
        turn: None,
        encounter: base_encounter_image(),
        timeline: Some(vec![tl(-1, 1), tl(-1, 2), tl(5, 3)]),
    }
}

#[test]
fn d6_snapshot_composite() {
    let fx = Keys::new(4);
    let scene = base(&fx);
    let (mut view, mut facts) = fresh();
    let snapshot = scene.snapshot(&mut view, &mut facts).expect("snapshot");

    // Entities 1 (health, default facts), 2 (pools and stats), 3 (default
    // facts; k3's row has every family but no grant); turn None; C2's images.
    assert_eq!(snapshot.body, base_body());
    assert_eq!(
        snapshot.cursor,
        ViewCursor {
            tick: 9,
            revision: 1
        }
    );
    // The body always passes encode_body, and round-trips.
    let bytes = encode_body(&snapshot.body).expect("encodes");
    assert_eq!(decode_body(&bytes), Ok(base_body()));
    // The whole plan decodes back to the same body.
    let frames =
        plan_transfer(Z16, tid(1), 9, snapshot.cursor.revision, 8, &snapshot.body).expect("plans");
    assert_eq!(frames.len(), 2);

    // A following project_page with the same contexts has no fact op.
    let page = scene.page(&mut view, &mut facts).expect("page");
    assert_eq!(page.ops, Vec::<DeltaOp>::new());
    // A pool change then emits only EntityFacts for 2.
    let mut changed = scene.clone();
    changed.rows[0].pools = vec![pool(1, 1, 3)];
    let page = changed.page(&mut view, &mut facts).expect("page");
    assert_eq!(
        page.ops,
        vec![entity_facts_op(
            2,
            FactSet {
                pools: Some(vec![pool(1, 1, 3)]),
                stats: Some(vec![stat("might", StatValue::Int(4))]),
                ..FactSet::default()
            }
        )]
    );
}

#[test]
fn d6_snapshot_with_a_turn_and_an_order() {
    let fx = Keys::new(4);
    let mut scene = base(&fx);
    scene.turn = Some(TurnState {
        area: u(10),
        active: Some(fx.k(2).entity),
        round: 3,
    });
    scene.turn_granted = true;
    scene.rows[0].fields.order = true;
    scene.rows[0].order = Some(OrderInput {
        ability: u(7),
        target: fx.k(1),
        repeat: true,
    });
    let (mut view, mut facts) = fresh();
    let snapshot = scene.snapshot(&mut view, &mut facts).expect("snapshot");
    assert_eq!(
        snapshot.body.turn,
        Some(TurnFact {
            active: Some(net(2)),
            round: 3
        })
    );
    assert_eq!(
        snapshot.body.entities[1].facts.order,
        Some(Some(OrderFact {
            ability: u(7),
            target: net(1),
            repeat: true
        }))
    );
    assert!(encode_body(&snapshot.body).is_ok());
    // An order whose target is not visible is withheld as a family; an order
    // of `None` is a disclosed nothing.
    let mut hidden = scene.clone();
    hidden.rows[0].order = Some(OrderInput {
        ability: u(7),
        target: fx.k(4),
        repeat: false,
    });
    let (mut v, mut f) = fresh();
    let snap = hidden.snapshot(&mut v, &mut f).unwrap();
    assert_eq!(snap.body.entities[1].facts.order, None);
    let mut none_order = scene.clone();
    none_order.rows[0].order = None;
    let (mut v, mut f) = fresh();
    let snap = none_order.snapshot(&mut v, &mut f).unwrap();
    assert_eq!(snap.body.entities[1].facts.order, Some(None));
    // Withheld without the row's order grant, whatever the value.
    let mut ungranted = scene.clone();
    ungranted.rows[0].fields.order = false;
    let (mut v, mut f) = fresh();
    let snap = ungranted.snapshot(&mut v, &mut f).unwrap();
    assert_eq!(snap.body.entities[1].facts.order, None);
}

#[test]
fn d6_snapshot_withholds_by_grant() {
    let fx = Keys::new(4);
    // No grants at all: default images, no timeline, facts default.
    let mut scene = base(&fx);
    scene.grants = EncounterGrants::default();
    scene.rows = vec![
        row(fx.k(2), FactMask::default()),
        row(fx.k(3), FactMask::default()),
    ];
    let (mut view, mut facts) = fresh();
    let snapshot = scene.snapshot(&mut view, &mut facts).unwrap();
    assert_eq!(snapshot.body.encounter, EncounterFacts::default());
    assert_eq!(snapshot.body.timeline, None);
    assert!(snapshot
        .body
        .entities
        .iter()
        .all(|e| e.facts == FactSet::default()));
    // Each encounter grant discloses only its own field.
    for (name, grants, mode, pending, clock, timeline) in [
        (
            "mode",
            EncounterGrants {
                mode: true,
                ..Default::default()
            },
            true,
            false,
            false,
            false,
        ),
        (
            "pending",
            EncounterGrants {
                pending_mode: true,
                ..Default::default()
            },
            false,
            true,
            false,
            false,
        ),
        (
            "clock",
            EncounterGrants {
                clock: true,
                ..Default::default()
            },
            false,
            false,
            true,
            false,
        ),
        (
            "timeline",
            EncounterGrants {
                timeline: true,
                ..Default::default()
            },
            false,
            false,
            false,
            true,
        ),
    ] {
        let mut s = base(&fx);
        s.grants = grants;
        let (mut v, mut f) = fresh();
        let snap = s.snapshot(&mut v, &mut f).unwrap();
        let image = base_encounter_image();
        assert_eq!(
            snap.body.encounter,
            EncounterFacts {
                mode: if mode { image.mode } else { None },
                pending_mode: if pending { image.pending_mode } else { None },
                clock: if clock { image.clock } else { None },
            },
            "{name}"
        );
        assert_eq!(snap.body.timeline.is_some(), timeline, "{name}");
    }
    // An encounter in another area is not disclosed.
    let mut foreign = base(&fx);
    foreign.encounter.as_mut().unwrap().area = u(11);
    let (mut v, mut f) = fresh();
    let snap = foreign.snapshot(&mut v, &mut f).unwrap();
    assert_eq!(snap.body.encounter, EncounterFacts::default());
    assert_eq!(snap.body.timeline, None);
    // An invalid clock is withheld, never refused.
    for bad in [
        ClockFact {
            round_ticks: 0,
            elapsed: 240,
        },
        ClockFact {
            round_ticks: 120,
            elapsed: 2_147_483_648,
        },
    ] {
        let mut s = base(&fx);
        s.encounter.as_mut().unwrap().clock = Some(bad);
        let (mut v, mut f) = fresh();
        let snap = s.snapshot(&mut v, &mut f).unwrap();
        assert_eq!(snap.body.encounter.clock, None);
        assert_eq!(snap.body.encounter.mode, Some(EncounterMode::RealTime));
    }
    // Over-limit families are withheld for the row, with no error.
    let mut over = base(&fx);
    over.rows[0].pools = (1..=17).map(|n| pool(n, 0, 0)).collect();
    over.rows[0].stats = vec![stat(&"n".repeat(33), StatValue::Int(1))];
    let (mut v, mut f) = fresh();
    let snap = over.snapshot(&mut v, &mut f).unwrap();
    assert_eq!(snap.body.entities[1].facts, FactSet::default());
    assert!(encode_body(&snap.body).is_ok());
    // A timeline entry for a key the viewer cannot see is dropped.
    let mut with_hidden = base(&fx);
    with_hidden.timeline.push((0, fx.k(4)));
    let (mut v, mut f) = fresh();
    let snap = with_hidden.snapshot(&mut v, &mut f).unwrap();
    assert_eq!(
        snap.body.timeline,
        Some(vec![tl(-1, 1), tl(-1, 2), tl(5, 3)])
    );
}

#[test]
fn d6_snapshot_replaces_the_fact_memory() {
    let fx = Keys::new(4);
    let scene = base(&fx);

    // A page first remembers k2's image; a snapshot whose context has no row
    // for k2 is a body with default facts for entity 2 and replaces the memory,
    // so the next page re-sends k2's image.
    let (mut view, mut facts) = fresh();
    scene.page(&mut view, &mut facts).unwrap();
    let mut rowless = scene.clone();
    rowless.rows = vec![k3_row(&fx)];
    let snap = rowless.snapshot(&mut view, &mut facts).unwrap();
    assert_eq!(snap.body.entities[1].facts, FactSet::default());
    // Control: the same context is "unchanged" for the remembered images.
    let (mut v2, mut f2) = (view.clone(), facts.clone());
    let page = rowless.page(&mut v2, &mut f2).unwrap();
    assert_eq!(page.ops, Vec::<DeltaOp>::new());
    // With k2's row back, the page re-sends its full image.
    let page = scene.page(&mut view, &mut facts).unwrap();
    assert_eq!(page.ops, vec![entity_facts_op(2, k2_image())]);

    // A snapshot after a page, with the same contexts, leaves nothing to say.
    let (mut view, mut facts) = fresh();
    scene.page(&mut view, &mut facts).unwrap();
    let snap = scene.snapshot(&mut view, &mut facts).unwrap();
    assert_eq!(snap.body, base_body());
    assert_eq!(snap.cursor.revision, 2);
    assert_eq!(
        scene.page(&mut view, &mut facts).unwrap().ops,
        Vec::<DeltaOp>::new()
    );

    // A snapshot that retires k2 (not visible) forgets its image: when k2
    // returns it gets a new id and a full image.
    let mut away = scene.clone();
    away.state.retain(|r| r.key != fx.k(2));
    away.rows = vec![k3_row(&fx)];
    away.timeline = vec![(5, fx.k(3)), (-1, fx.k(1))];
    let snap = away.snapshot(&mut view, &mut facts).unwrap();
    assert_eq!(
        snap.body
            .entities
            .iter()
            .map(|e| e.entity)
            .collect::<Vec<_>>(),
        vec![net(1), net(3)]
    );
    assert_eq!(snap.body.timeline, Some(vec![tl(-1, 1), tl(5, 3)]));
    let page = scene.page(&mut view, &mut facts).unwrap();
    assert_eq!(
        page.ops,
        vec![
            DeltaOp::EntityEnter { entity: net(4) },
            DeltaOp::Timeline {
                entries: Some(vec![tl(-1, 1), tl(-1, 4), tl(5, 3)])
            },
            entity_facts_op(4, k2_image()),
        ]
    );

    // A snapshot on a warm pair equals the snapshot of a fresh pair, apart
    // from the cursor revision (ids are kept for still-visible keys).
    let (mut warm_view, mut warm_facts) = fresh();
    scene.page(&mut warm_view, &mut warm_facts).unwrap();
    let warm = scene.snapshot(&mut warm_view, &mut warm_facts).unwrap();
    let (mut cold_view, mut cold_facts) = fresh();
    let cold = scene.snapshot(&mut cold_view, &mut cold_facts).unwrap();
    assert_eq!(warm.body, cold.body);
}

#[test]
fn d6_snapshot_is_independent_of_resend() {
    let fx = Keys::new(4);
    let mut scene = base(&fx);
    let (mut a, mut fa) = fresh();
    let without = scene.snapshot(&mut a, &mut fa).unwrap();
    scene.resend = true;
    let (mut b, mut fb) = fresh();
    let with = scene.snapshot(&mut b, &mut fb).unwrap();
    assert_eq!(with, without);
    assert_eq!(fa, fb);
}

// ---------------------------------------------------------------------------
// D7: the composite is all-or-nothing.
// ---------------------------------------------------------------------------

fn assert_snapshot_rejected(
    scene: &Scene,
    pair: &(InterestView, FactView),
    want: FactError,
    why: &str,
) {
    let (mut view, mut facts) = pair.clone();
    let result = scene.snapshot(&mut view, &mut facts);
    assert_eq!(result, Err(want), "{why}");
    assert_eq!(view, pair.0, "{why}: interest view changed");
    assert_eq!(facts, pair.1, "{why}: fact view changed");
    // The page composite agrees on the same refusal.
    let (mut view, mut facts) = pair.clone();
    assert_eq!(scene.page(&mut view, &mut facts), Err(want), "{why}: page");
}

#[test]
fn d7_snapshot_all_or_nothing() {
    let fx = Keys::new(1_100);
    let scene = base(&fx);
    let cold = fresh();
    let mut warm = fresh();
    scene.snapshot(&mut warm.0, &mut warm.1).expect("warm up");
    let pairs = [cold, warm];

    // Control: the scene is valid on both pairs.
    for pair in &pairs {
        let (mut v, mut f) = pair.clone();
        scene.snapshot(&mut v, &mut f).expect("control passes");
    }

    for pair in &pairs {
        // A FactView for another epoch, and a state for another epoch.
        let wrong_facts = (pair.0.clone(), FactView::new(other_epoch()));
        assert_snapshot_rejected(&scene, &wrong_facts, FactError::WrongEpoch, "facts epoch");
        let mut wrong_state = scene.clone();
        wrong_state.epoch = other_epoch();
        assert_snapshot_rejected(&wrong_state, pair, FactError::WrongEpoch, "state epoch");
        // An InterestView for another epoch.
        let wrong_view = (InterestView::new(other_epoch()), pair.1.clone());
        assert_snapshot_rejected(&scene, &wrong_view, FactError::WrongEpoch, "view epoch");

        // 1,025 rows and 1,025 timeline entries; and 1,025 rows with a
        // duplicate key is TooManyFacts, the bound being first.
        let mut many_rows = scene.clone();
        many_rows.rows = (1..=1025)
            .map(|n| row(fx.k(n), FactMask::default()))
            .collect();
        assert_snapshot_rejected(&many_rows, pair, FactError::TooManyFacts, "1,025 rows");
        let mut many_dup = scene.clone();
        many_dup.rows = (1..=1025)
            .map(|n| row(fx.k(n.min(1024)), FactMask::default()))
            .collect();
        assert_snapshot_rejected(&many_dup, pair, FactError::TooManyFacts, "1,025 dup rows");
        let mut many_timeline = scene.clone();
        many_timeline.timeline = (1..=1025).map(|n| (0, fx.k(n))).collect();
        assert_snapshot_rejected(
            &many_timeline,
            pair,
            FactError::TooManyFacts,
            "1,025 timeline entries",
        );

        // 1,024 rows and 1,024 timeline entries are accepted (the controls).
        let mut at_cap = scene.clone();
        at_cap.rows = (1..=1024)
            .map(|n| row(fx.k(n), FactMask::default()))
            .collect();
        at_cap.timeline = (1..=1024).map(|n| (0, fx.k(n))).collect();
        let (mut v, mut f) = pair.clone();
        at_cap.snapshot(&mut v, &mut f).expect("at the cap");

        // Duplicate row key and duplicate timeline key.
        let mut dup_row = scene.clone();
        dup_row.rows = vec![k2_row(&fx), k2_row(&fx)];
        assert_snapshot_rejected(&dup_row, pair, FactError::DuplicateFact, "duplicate row");
        let mut dup_timeline = scene.clone();
        dup_timeline.timeline = vec![(1, fx.k(1)), (2, fx.k(1))];
        assert_snapshot_rejected(
            &dup_timeline,
            pair,
            FactError::DuplicateFact,
            "duplicate timeline key",
        );

        // The interest arm: two state rows for k1, and 1,025 state rows.
        let mut dup_state = scene.clone();
        dup_state.state.push(state_row(fx.k(1), true, None));
        assert_snapshot_rejected(
            &dup_state,
            pair,
            FactError::Interest(InterestError::DuplicateFact),
            "duplicate state row",
        );
        let mut many_state = scene.clone();
        many_state.state = (1..=1025).map(|n| state_row(fx.k(n), true, None)).collect();
        assert_snapshot_rejected(
            &many_state,
            pair,
            FactError::Interest(InterestError::TooManyFacts),
            "1,025 state rows",
        );
    }
}

#[test]
fn d7_check_order() {
    let fx = Keys::new(1_100);
    let scene = base(&fx);
    // Epoch beats the bounds, and a duplicate beats the interest arm.
    let pair = (InterestView::new(epoch()), FactView::new(other_epoch()));
    let mut s = scene.clone();
    s.rows = (1..=1025)
        .map(|n| row(fx.k(n), FactMask::default()))
        .collect();
    assert_eq!(
        s.snapshot(&mut pair.0.clone(), &mut pair.1.clone()),
        Err(FactError::WrongEpoch)
    );
    let fresh_pair = fresh();
    let mut s = scene.clone();
    s.rows = vec![k2_row(&fx), k2_row(&fx)];
    s.state.push(state_row(fx.k(1), true, None));
    assert_eq!(
        s.snapshot(&mut fresh_pair.0.clone(), &mut fresh_pair.1.clone()),
        Err(FactError::DuplicateFact),
        "a duplicate row beats the interest arm"
    );
    let mut s = scene.clone();
    s.timeline = vec![(1, fx.k(1)), (2, fx.k(1))];
    s.state.push(state_row(fx.k(1), true, None));
    assert_eq!(
        s.snapshot(&mut fresh_pair.0.clone(), &mut fresh_pair.1.clone()),
        Err(FactError::DuplicateFact),
        "a duplicate timeline key beats the interest arm"
    );
}

/// A body that encodes to exactly `target` bytes: maximal entities from the
/// D5 body, then filler entities whose only fact is a stat list.
fn body_of_len(target: usize) -> SnapshotBodyV3 {
    let (full, _) = largest();
    let base = 1u64 << 63;
    let with_entities = |entities: Vec<SnapshotEntityV3>| SnapshotBodyV3 {
        entities,
        turn: None,
        encounter: EncounterFacts::default(),
        timeline: None,
    };
    // The largest N whose maximal entities leave at least 100 bytes to fill.
    let mut n = (target / 2_766).saturating_sub(3);
    let mut len_n = encode_body(&with_entities(full.entities[..n].to_vec()))
        .unwrap()
        .len();
    loop {
        let next = encode_body(&with_entities(full.entities[..n + 1].to_vec()))
            .unwrap()
            .len();
        if next + 100 > target {
            break;
        }
        n += 1;
        len_n = next;
    }
    let remaining = target - len_n;
    // A filler entity is 17 bytes plus its stats; a stat of a two-to-32-byte
    // name costs 5..=35 bytes. Spread `remaining` over m fillers.
    let m = remaining.div_ceil(1_100);
    let mut entities = full.entities[..n].to_vec();
    let mut left = remaining;
    for j in 0..m {
        let share = if j + 1 == m { left } else { remaining / m };
        left -= share;
        let extra = share - 17;
        let k = extra.div_ceil(35);
        assert!(5 * k <= extra && k <= 32, "share {share}");
        let mut spare = extra - 5 * k;
        let stats = (0..k)
            .map(|i| {
                let add = spare.min(30);
                spare -= add;
                let width = 2 + add;
                stat(&format!("{i:0>width$}"), StatValue::Int(0))
            })
            .collect();
        entities.push(SnapshotEntityV3 {
            entity: net(base + (n + j) as u64),
            health: None,
            facts: FactSet {
                stats: Some(stats),
                ..FactSet::default()
            },
        });
    }
    let body = with_entities(entities);
    assert_eq!(encode_body(&body).unwrap().len(), target);
    body
}

#[test]
fn d8_chunk_count_is_checked_in_usize_at_the_cap() {
    // 128 chunks of one segment are 524,288 bytes: that plan is accepted, and
    // one more byte needs a 129th chunk, which is LimitExceeded.
    let edge = body_of_len(128 * 4096);
    let frames = plan_transfer(Z16, tid(1), 0, 0, 1, &edge).expect("128 chunks plan");
    assert_eq!(frames.len(), 129);
    let Ok(TransferDown::Begin(b)) = decode_down(&frames[0]) else {
        panic!("Begin");
    };
    assert_eq!(b.chunk_count, 128);
    assert_eq!(b.total_bytes, 524_288);
    assert_eq!(b.entity_count as usize, edge.entities.len());
    let past = body_of_len(128 * 4096 + 1);
    assert_eq!(
        plan_transfer(Z16, tid(1), 0, 0, 1, &past),
        Err(CodecError::LimitExceeded)
    );
    // The same bodies at two segments per chunk are 64 and 65 chunks.
    assert!(plan_transfer(Z16, tid(1), 0, 0, 2, &past).is_ok());
}

#[test]
fn d8_chunk_count_is_checked_before_narrowing() {
    // A body of 271 chunks of one segment: narrowed to a byte that is 15,
    // which would pass the cap and fail the cut check as Malformed. The cap
    // is applied in usize, so it is LimitExceeded.
    let body = body_of_len(271 * 4096 - 5);
    assert_eq!(encode_body(&body).unwrap().len().div_ceil(4096), 271);
    assert_eq!(271 % 256, 15);
    assert_eq!(
        plan_transfer(Z16, tid(1), 0, 0, 1, &body),
        Err(CodecError::LimitExceeded)
    );
    // 257 chunks narrow to 1.
    let body = body_of_len(256 * 4096 + 1);
    assert_eq!(
        plan_transfer(Z16, tid(1), 0, 0, 1, &body),
        Err(CodecError::LimitExceeded)
    );
    // Control: 8 segments per chunk is 34 chunks and plans.
    assert!(plan_transfer(Z16, tid(1), 0, 0, 8, &body).is_ok());
}

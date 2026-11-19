//! T042: the wire-level malicious-client suite (spec §16.2, §19.2 #21).
//! Every server-facing decoder, every hostile class, one table. No clock,
//! no thread, no I/O, no unseeded RNG, no `HashMap`/`HashSet`, no float.

use crpg_core::Ulid;
use crpg_net::codec::{self, CodecError};
use crpg_net::codec_resume;
use crpg_net::codec_transfer;
use crpg_net::codec_v2;
use crpg_net::codec_v3;
use crpg_net::codec_v4;
use crpg_net::movement_v4::{self, MoveBody, MoveDown, MoveIntent, MoveOutcome, MoveReceipt};
use crpg_net::protocol::{self, NetId, LANE_COMBAT};
use crpg_net::protocol_resume::{
    self, ConnectionFence, LaneClaim, ResumeDown, ResumeRejected, ResumeRejection, ResumeRequest,
    ResumeResult,
};
use crpg_net::protocol_transfer::{
    self, CancelReason, ResyncReason, TransferDown, TransferId, TransferUp,
};
use crpg_net::protocol_v2;
use crpg_net::protocol_v3::{self, EncounterMode};
use crpg_net::protocol_v4::{self, Point};
use crpg_net::transfer_v3;

/// `[0x01, 0x02, …, 0x10]`, as `tests/audit_gaps.rs:20–22`.
const EPOCH: [u8; 16] = [
    0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F, 0x10,
];

/// The pinned ability ULID of W-1.
fn ability() -> Ulid {
    Ulid::from_parts(0x0123_4567_89AB, 0x0CDE_F012_3456_789A)
}

fn net(n: u64) -> NetId {
    NetId::new(n).unwrap()
}

fn tid(n: u64) -> TransferId {
    TransferId::new(n).unwrap()
}

/// One server-facing decoder with its own encoder (Reading 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Wire {
    V1,
    V2,
    V3,
    V4,
    T81,
    T82,
    R91,
    A1,
}

/// Table order; every "for each wire" loop uses it.
const WIRES: [Wire; 8] = [
    Wire::V1,
    Wire::V2,
    Wire::V3,
    Wire::V4,
    Wire::T81,
    Wire::T82,
    Wire::R91,
    Wire::A1,
];

impl Wire {
    /// 1, 2, 3, 4, 0x81, 0x82, 0x91, 0xA1.
    fn first_byte(self) -> u8 {
        match self {
            Wire::V1 => protocol::PROTOCOL_VERSION,
            Wire::V2 => protocol_v2::PROTOCOL_VERSION,
            Wire::V3 => protocol_v3::PROTOCOL_VERSION,
            Wire::V4 => protocol_v4::PROTOCOL_VERSION,
            Wire::T81 => protocol_transfer::TRANSFER_VERSION,
            Wire::T82 => transfer_v3::TRANSFER_VERSION_V3,
            Wire::R91 => protocol_resume::RESUME_VERSION,
            Wire::A1 => movement_v4::MOVEMENT_VERSION,
        }
    }

    /// Index of the lane byte: 17, 17, 17, 17, 1, 1, 1, 17.
    fn lane_at(self) -> usize {
        match self {
            Wire::V1 | Wire::V2 | Wire::V3 | Wire::V4 | Wire::A1 => 17,
            Wire::T81 | Wire::T82 | Wire::R91 => 1,
        }
    }

    /// The lane byte: 0, 0, 0, 0, 2, 2, 3, 1.
    fn lane(self) -> u8 {
        match self {
            Wire::V1 | Wire::V2 | Wire::V3 | Wire::V4 => LANE_COMBAT,
            Wire::T81 | Wire::T82 => protocol_transfer::LANE_SNAPSHOT,
            Wire::R91 => protocol_resume::LANE_SESSION,
            Wire::A1 => movement_v4::LANE_MOVEMENT,
        }
    }

    /// Decode with this wire's decoder; on `Ok(v)`, re-encode `v` with this
    /// wire's encoder. An encoder error on a decoded value panics with the
    /// wire, the input bytes and the error (the test fails: W-12, W-13).
    fn verdict(self, bytes: &[u8]) -> Verdict {
        match self {
            Wire::V1 => reencoded(
                self,
                bytes,
                codec::decode_intent(bytes),
                codec::encode_intent,
            ),
            Wire::V2 => reencoded(
                self,
                bytes,
                codec_v2::decode_intent(bytes),
                codec_v2::encode_intent,
            ),
            Wire::V3 => reencoded(
                self,
                bytes,
                codec_v3::decode_intent(bytes),
                codec_v3::encode_intent,
            ),
            Wire::V4 => reencoded(
                self,
                bytes,
                codec_v4::decode_intent(bytes),
                codec_v4::encode_intent,
            ),
            Wire::T81 => reencoded(
                self,
                bytes,
                codec_transfer::decode_up(bytes),
                codec_transfer::encode_up,
            ),
            Wire::T82 => reencoded(
                self,
                bytes,
                transfer_v3::decode_up(bytes),
                transfer_v3::encode_up,
            ),
            Wire::R91 => reencoded(
                self,
                bytes,
                codec_resume::decode_request(bytes),
                codec_resume::encode_request,
            ),
            Wire::A1 => reencoded(
                self,
                bytes,
                movement_v4::decode_intent(bytes),
                movement_v4::encode_intent,
            ),
        }
    }
}

fn reencoded<T>(
    wire: Wire,
    bytes: &[u8],
    decoded: Result<T, CodecError>,
    encode: fn(&T) -> Result<Vec<u8>, CodecError>,
) -> Verdict {
    match decoded {
        Err(error) => Verdict::Refused(error),
        Ok(value) => match encode(&value) {
            Ok(canonical) => Verdict::Accepted(canonical),
            Err(error) => panic!(
                "{wire:?}: encoder refused a decoded value for input {:02X?}: {error:?}",
                bytes
            ),
        },
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Verdict {
    /// The decoder refused the bytes with this error.
    Refused(CodecError),
    /// The decoder accepted them; the canonical re-encoding of the value.
    Accepted(Vec<u8>),
}

/// One pinned valid frame (the table in W-1).
struct Fixture {
    name: &'static str,
    wire: Wire,
    bytes: Vec<u8>,
    tag_at: usize,
}

/// The 17 fixtures of W-1, in its table order, each built by its wire's
/// encoder from the value in that table (never from hand-written bytes).
fn fixtures() -> Vec<Fixture> {
    let ability = ability();
    let mut out = Vec::new();

    let v1_declare = protocol::IntentFrame {
        epoch: EPOCH,
        lane: LANE_COMBAT,
        seq: 1,
        observed_tick: 41,
        actor: net(7),
        body: protocol::IntentBody::DeclareAction {
            ability,
            target: net(9),
        },
    };
    out.push(Fixture {
        name: "D1",
        wire: Wire::V1,
        bytes: codec::encode_intent(&v1_declare).unwrap(),
        tag_at: 21,
    });
    let v1_end = protocol::IntentFrame {
        epoch: EPOCH,
        lane: LANE_COMBAT,
        seq: 1,
        observed_tick: 41,
        actor: net(7),
        body: protocol::IntentBody::EndTurn,
    };
    out.push(Fixture {
        name: "E1",
        wire: Wire::V1,
        bytes: codec::encode_intent(&v1_end).unwrap(),
        tag_at: 21,
    });

    let v2_declare = protocol::IntentFrame {
        epoch: EPOCH,
        lane: LANE_COMBAT,
        seq: 1,
        observed_tick: 41,
        actor: net(7),
        body: protocol::IntentBody::DeclareAction {
            ability,
            target: net(9),
        },
    };
    out.push(Fixture {
        name: "D2",
        wire: Wire::V2,
        bytes: codec_v2::encode_intent(&v2_declare).unwrap(),
        tag_at: 21,
    });
    let v2_end = protocol::IntentFrame {
        epoch: EPOCH,
        lane: LANE_COMBAT,
        seq: 1,
        observed_tick: 41,
        actor: net(7),
        body: protocol::IntentBody::EndTurn,
    };
    out.push(Fixture {
        name: "E2",
        wire: Wire::V2,
        bytes: codec_v2::encode_intent(&v2_end).unwrap(),
        tag_at: 21,
    });

    let v3_declare = protocol_v3::IntentFrame {
        epoch: EPOCH,
        lane: LANE_COMBAT,
        seq: 1,
        observed_tick: 41,
        actor: net(7),
        body: protocol_v3::IntentBody::DeclareAction {
            ability,
            target: net(9),
        },
    };
    out.push(Fixture {
        name: "D3",
        wire: Wire::V3,
        bytes: codec_v3::encode_intent(&v3_declare).unwrap(),
        tag_at: 21,
    });
    let v3_end = protocol_v3::IntentFrame {
        epoch: EPOCH,
        lane: LANE_COMBAT,
        seq: 1,
        observed_tick: 41,
        actor: net(7),
        body: protocol_v3::IntentBody::EndTurn,
    };
    out.push(Fixture {
        name: "E3",
        wire: Wire::V3,
        bytes: codec_v3::encode_intent(&v3_end).unwrap(),
        tag_at: 21,
    });
    let v3_mode = protocol_v3::IntentFrame {
        epoch: EPOCH,
        lane: LANE_COMBAT,
        seq: 1,
        observed_tick: 41,
        actor: net(7),
        body: protocol_v3::IntentBody::RequestMode {
            mode: EncounterMode::RealTime,
        },
    };
    out.push(Fixture {
        name: "M3",
        wire: Wire::V3,
        bytes: codec_v3::encode_intent(&v3_mode).unwrap(),
        tag_at: 21,
    });

    let v4_declare = protocol_v4::IntentFrame {
        epoch: EPOCH,
        lane: LANE_COMBAT,
        seq: 1,
        observed_tick: 41,
        actor: net(7),
        body: protocol_v4::IntentBody::DeclareAction {
            ability,
            target: net(9),
        },
    };
    out.push(Fixture {
        name: "D4",
        wire: Wire::V4,
        bytes: codec_v4::encode_intent(&v4_declare).unwrap(),
        tag_at: 21,
    });
    let v4_end = protocol_v4::IntentFrame {
        epoch: EPOCH,
        lane: LANE_COMBAT,
        seq: 1,
        observed_tick: 41,
        actor: net(7),
        body: protocol_v4::IntentBody::EndTurn,
    };
    out.push(Fixture {
        name: "E4",
        wire: Wire::V4,
        bytes: codec_v4::encode_intent(&v4_end).unwrap(),
        tag_at: 21,
    });
    let v4_move = protocol_v4::IntentFrame {
        epoch: EPOCH,
        lane: LANE_COMBAT,
        seq: 1,
        observed_tick: 41,
        actor: net(7),
        body: protocol_v4::IntentBody::DeclareMove {
            destination: Point::new([1, 2, 3]).unwrap(),
        },
    };
    out.push(Fixture {
        name: "V4",
        wire: Wire::V4,
        bytes: codec_v4::encode_intent(&v4_move).unwrap(),
        tag_at: 21,
    });

    let published = TransferUp::Published {
        epoch: EPOCH,
        transfer: tid(1),
    };
    out.push(Fixture {
        name: "P81",
        wire: Wire::T81,
        bytes: codec_transfer::encode_up(&published).unwrap(),
        tag_at: 2,
    });
    let resync = TransferUp::Resync {
        epoch: EPOCH,
        after: None,
        reason: ResyncReason::Expired,
    };
    out.push(Fixture {
        name: "R81",
        wire: Wire::T81,
        bytes: codec_transfer::encode_up(&resync).unwrap(),
        tag_at: 2,
    });
    out.push(Fixture {
        name: "P82",
        wire: Wire::T82,
        bytes: transfer_v3::encode_up(&published).unwrap(),
        tag_at: 2,
    });
    out.push(Fixture {
        name: "R82",
        wire: Wire::T82,
        bytes: transfer_v3::encode_up(&resync).unwrap(),
        tag_at: 2,
    });

    let request = ResumeRequest {
        epoch: EPOCH,
        fence: ConnectionFence::FIRST,
        proof: protocol_resume::ResumeProof::new(vec![0xA5; 16]).unwrap(),
        claims: vec![LaneClaim {
            lane: LANE_COMBAT,
            receipted_through: 0,
        }],
    };
    out.push(Fixture {
        name: "Q91",
        wire: Wire::R91,
        bytes: codec_resume::encode_request(&request).unwrap(),
        tag_at: 2,
    });

    let move_to = MoveIntent {
        epoch: EPOCH,
        seq: 1,
        actor: net(7),
        body: MoveBody::MoveTo {
            destination: Point::new([1, 2, 3]).unwrap(),
        },
    };
    out.push(Fixture {
        name: "MT",
        wire: Wire::A1,
        bytes: movement_v4::encode_intent(&move_to).unwrap(),
        tag_at: 20,
    });
    let halt = MoveIntent {
        epoch: EPOCH,
        seq: 1,
        actor: net(7),
        body: MoveBody::Halt,
    };
    out.push(Fixture {
        name: "MH",
        wire: Wire::A1,
        bytes: movement_v4::encode_intent(&halt).unwrap(),
        tag_at: 20,
    });

    out
}

// ---------------------------------------------------------------------------
// Literal W-1 bytes (the independent side of X1's comparison).
// ---------------------------------------------------------------------------

fn h(version: u8) -> Vec<u8> {
    let mut out = vec![version];
    out.extend_from_slice(&EPOCH);
    out.extend_from_slice(&[0x00, 0x01, 0x29, 0x07]);
    out
}

fn declare(version: u8) -> Vec<u8> {
    let mut out = h(version);
    out.extend_from_slice(&[0x00, 0x1A]);
    out.extend_from_slice(ability().to_string().as_bytes());
    out.push(0x09);
    out
}

fn end_turn(version: u8) -> Vec<u8> {
    let mut out = h(version);
    out.push(0x01);
    out
}

fn transfer_up(version: u8, tag: u8, tail: &[u8]) -> Vec<u8> {
    let mut out = vec![version, protocol_transfer::LANE_SNAPSHOT, tag];
    out.extend_from_slice(&EPOCH);
    out.extend_from_slice(tail);
    out
}

fn pinned() -> Vec<Fixture> {
    let mut q91 = vec![0x91, protocol_resume::LANE_SESSION, 0x40];
    q91.extend_from_slice(&EPOCH);
    q91.extend_from_slice(&[0x01, 0x10]);
    q91.extend_from_slice(&[0xA5; 16]);
    q91.extend_from_slice(&[0x01, 0x00, 0x00]);

    let mut mt = vec![0xA1];
    mt.extend_from_slice(&EPOCH);
    mt.extend_from_slice(&[0x01, 0x01, 0x07, 0x00, 0x02, 0x04, 0x06]);
    let mut mh = vec![0xA1];
    mh.extend_from_slice(&EPOCH);
    mh.extend_from_slice(&[0x01, 0x01, 0x07, 0x01]);

    let out = vec![
        Fixture {
            name: "D1",
            wire: Wire::V1,
            bytes: declare(1),
            tag_at: 21,
        },
        Fixture {
            name: "E1",
            wire: Wire::V1,
            bytes: end_turn(1),
            tag_at: 21,
        },
        Fixture {
            name: "D2",
            wire: Wire::V2,
            bytes: declare(2),
            tag_at: 21,
        },
        Fixture {
            name: "E2",
            wire: Wire::V2,
            bytes: end_turn(2),
            tag_at: 21,
        },
        Fixture {
            name: "D3",
            wire: Wire::V3,
            bytes: declare(3),
            tag_at: 21,
        },
        Fixture {
            name: "E3",
            wire: Wire::V3,
            bytes: end_turn(3),
            tag_at: 21,
        },
        Fixture {
            name: "M3",
            wire: Wire::V3,
            bytes: {
                let mut b = h(3);
                b.extend_from_slice(&[0x05, 0x01]);
                b
            },
            tag_at: 21,
        },
        Fixture {
            name: "D4",
            wire: Wire::V4,
            bytes: declare(4),
            tag_at: 21,
        },
        Fixture {
            name: "E4",
            wire: Wire::V4,
            bytes: end_turn(4),
            tag_at: 21,
        },
        Fixture {
            name: "V4",
            wire: Wire::V4,
            bytes: {
                let mut b = h(4);
                b.extend_from_slice(&[0x06, 0x02, 0x04, 0x06]);
                b
            },
            tag_at: 21,
        },
        Fixture {
            name: "P81",
            wire: Wire::T81,
            bytes: transfer_up(0x81, 0x20, &[0x01]),
            tag_at: 2,
        },
        Fixture {
            name: "R81",
            wire: Wire::T81,
            bytes: transfer_up(0x81, 0x21, &[0x00, 0x01]),
            tag_at: 2,
        },
        Fixture {
            name: "P82",
            wire: Wire::T82,
            bytes: transfer_up(0x82, 0x20, &[0x01]),
            tag_at: 2,
        },
        Fixture {
            name: "R82",
            wire: Wire::T82,
            bytes: transfer_up(0x82, 0x21, &[0x00, 0x01]),
            tag_at: 2,
        },
        Fixture {
            name: "Q91",
            wire: Wire::R91,
            bytes: q91,
            tag_at: 2,
        },
        Fixture {
            name: "MT",
            wire: Wire::A1,
            bytes: mt,
            tag_at: 20,
        },
        Fixture {
            name: "MH",
            wire: Wire::A1,
            bytes: mh,
            tag_at: 20,
        },
    ];
    out
}

/// `bytes` with `bytes[at]` replaced by `value`; every other byte, the valid
/// remainder after `at` included, unchanged.
fn with_byte(bytes: &[u8], at: usize, value: u8) -> Vec<u8> {
    let mut out = bytes.to_vec();
    out[at] = value;
    out
}

/// `bytes` followed by `0x00` up to `len` bytes (`len >= bytes.len()`).
fn padded(bytes: &[u8], len: usize) -> Vec<u8> {
    assert!(len >= bytes.len());
    let mut out = bytes.to_vec();
    out.resize(len, 0);
    out
}

/// W-13's rule: `Refused(_)`, or `Accepted(c)` with `verdict(&c) == Accepted(c)`.
fn stable(wire: Wire, bytes: &[u8]) -> bool {
    match wire.verdict(bytes) {
        Verdict::Refused(_) => true,
        Verdict::Accepted(canonical) => {
            let again = wire.verdict(&canonical);
            again == Verdict::Accepted(canonical)
        }
    }
}

/// Marsaglia xorshift64 (shifts 13, 7, 17), wrapping `u64` arithmetic.
struct XorShift64(u64);

impl XorShift64 {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

const SEED: u64 = 0x5EED_0000_0000_0042;

fn fixture<'a>(all: &'a [Fixture], name: &str) -> &'a Fixture {
    all.iter()
        .find(|f| f.name == name)
        .unwrap_or_else(|| panic!("no fixture {name}"))
}

// ---------------------------------------------------------------------------
// X1..X14
// ---------------------------------------------------------------------------

#[test]
fn fixtures_are_pinned() {
    let fx = fixtures();
    let literal = pinned();
    assert_eq!(fx.len(), 17, "fixture count");

    for (f, p) in fx.iter().zip(literal.iter()) {
        assert_eq!(f.name, p.name);
        assert_eq!(f.wire, p.wire, "fixture {} wire", f.name);
        assert_eq!(f.bytes.len(), p.bytes.len(), "fixture {} length", f.name);
        assert_eq!(f.tag_at, p.tag_at, "fixture {} tag_at", f.name);
        assert_eq!(
            f.bytes, p.bytes,
            "fixture {} encoder bytes {:02X?} != literal {:02X?}",
            f.name, f.bytes, p.bytes
        );
        assert_eq!(
            f.wire.verdict(&f.bytes),
            Verdict::Accepted(f.bytes.clone()),
            "fixture {} is not canonical: {:02X?}",
            f.name,
            f.bytes
        );
    }

    assert_eq!(Wire::V1.first_byte(), protocol::PROTOCOL_VERSION);
    assert_eq!(Wire::V2.first_byte(), protocol_v2::PROTOCOL_VERSION);
    assert_eq!(Wire::V3.first_byte(), protocol_v3::PROTOCOL_VERSION);
    assert_eq!(Wire::V4.first_byte(), protocol_v4::PROTOCOL_VERSION);
    assert_eq!(Wire::T81.first_byte(), protocol_transfer::TRANSFER_VERSION);
    assert_eq!(Wire::T82.first_byte(), transfer_v3::TRANSFER_VERSION_V3);
    assert_eq!(Wire::R91.first_byte(), protocol_resume::RESUME_VERSION);
    assert_eq!(Wire::A1.first_byte(), movement_v4::MOVEMENT_VERSION);

    for wire in [Wire::V1, Wire::V2, Wire::V3, Wire::V4] {
        assert_eq!(wire.lane_at(), 17, "{wire:?} lane index");
        assert_eq!(wire.lane(), LANE_COMBAT, "{wire:?} lane");
    }
    for wire in [Wire::T81, Wire::T82] {
        assert_eq!(wire.lane_at(), 1, "{wire:?} lane index");
        assert_eq!(wire.lane(), protocol_transfer::LANE_SNAPSHOT);
    }
    assert_eq!(Wire::R91.lane_at(), 1);
    assert_eq!(Wire::R91.lane(), protocol_resume::LANE_SESSION);
    assert_eq!(Wire::A1.lane_at(), 17);
    assert_eq!(Wire::A1.lane(), movement_v4::LANE_MOVEMENT);

    let e1 = fixture(&fx, "E1");
    assert_ne!(
        with_byte(&e1.bytes, 21, 0x00),
        e1.bytes,
        "control: E1 with tag 0 keeps E1's encoding"
    );
}

#[test]
fn short_inputs_are_refused_exactly() {
    for wire in WIRES {
        assert_eq!(
            wire.verdict(&[]),
            Verdict::Refused(CodecError::Malformed),
            "{wire:?} empty input"
        );
        for b in 0..=255u8 {
            let expected = if b == wire.first_byte() {
                CodecError::Malformed
            } else {
                CodecError::UnsupportedVersion
            };
            assert_eq!(
                wire.verdict(&[b]),
                Verdict::Refused(expected),
                "{wire:?} one-byte input {:02X?}",
                [b]
            );
            for c in 0..=255u8 {
                let expected = if b != wire.first_byte() {
                    CodecError::UnsupportedVersion
                } else if wire.lane_at() == 17 {
                    CodecError::Malformed
                } else if c != wire.lane() {
                    CodecError::WrongDirection
                } else {
                    CodecError::Malformed
                };
                assert_eq!(
                    wire.verdict(&[b, c]),
                    Verdict::Refused(expected),
                    "{wire:?} two-byte input {:02X?}",
                    [b, c]
                );
            }
        }
    }

    assert_eq!(
        Wire::V1.verdict(&[1]),
        Verdict::Refused(CodecError::Malformed)
    );
    assert_eq!(
        Wire::V1.verdict(&[2]),
        Verdict::Refused(CodecError::UnsupportedVersion)
    );
    assert_eq!(
        Wire::T81.verdict(&[0x81, 2]),
        Verdict::Refused(CodecError::Malformed)
    );
    assert_eq!(
        Wire::T81.verdict(&[0x81, 3]),
        Verdict::Refused(CodecError::WrongDirection)
    );
}

#[test]
fn oversize_is_refused_first() {
    assert_eq!(protocol::MAX_INTENT_FRAME_BYTES, 4096);
    assert_eq!(protocol_transfer::MAX_UP_FRAME_BYTES, 4096);
    assert_eq!(protocol_resume::MAX_RESUME_FRAME_BYTES, 4096);

    let fx = fixtures();
    for wire in WIRES {
        assert_eq!(
            wire.verdict(&[0xFF; 4097]),
            Verdict::Refused(CodecError::FrameTooLarge),
            "{wire:?} 4097 bytes"
        );
        assert_eq!(
            wire.verdict(&[0xFF; 4096]),
            Verdict::Refused(CodecError::UnsupportedVersion),
            "{wire:?} 4096 bytes"
        );
        for f in fx.iter().filter(|f| f.wire == wire) {
            assert_eq!(
                wire.verdict(&padded(&f.bytes, 4096)),
                Verdict::Refused(CodecError::Malformed),
                "{wire:?} fixture {} padded to 4096: {:02X?}",
                f.name,
                f.bytes
            );
            assert_eq!(
                wire.verdict(&padded(&f.bytes, 4097)),
                Verdict::Refused(CodecError::FrameTooLarge),
                "{wire:?} fixture {} padded to 4097: {:02X?}",
                f.name,
                f.bytes
            );
        }
    }

    let e1 = fixture(&fx, "E1");
    let at_cap = padded(&e1.bytes, 4096);
    let over_cap = padded(&e1.bytes, 4097);
    assert_eq!(over_cap.len(), at_cap.len() + 1);
    assert_eq!(
        Wire::V1.verdict(&at_cap),
        Verdict::Refused(CodecError::Malformed)
    );
    assert_eq!(
        Wire::V1.verdict(&over_cap),
        Verdict::Refused(CodecError::FrameTooLarge)
    );
}

#[test]
fn every_truncation_is_malformed() {
    let fx = fixtures();
    let mut count = 0usize;
    for f in &fx {
        for n in 0..f.bytes.len() {
            assert_eq!(
                f.wire.verdict(&f.bytes[..n]),
                Verdict::Refused(CodecError::Malformed),
                "{:?} fixture {} prefix of {n} bytes: {:02X?}",
                f.wire,
                f.name,
                &f.bytes[..n]
            );
            count += 1;
        }
        assert_eq!(
            f.wire.verdict(&f.bytes),
            Verdict::Accepted(f.bytes.clone()),
            "control: {:?} fixture {} full bytes",
            f.wire,
            f.name
        );
    }
    assert_eq!(count, 503);
}

#[test]
fn trailing_bytes_are_malformed() {
    let fx = fixtures();
    let mut count = 0usize;
    for f in &fx {
        for extra in [0x00u8, 0x01, 0xFF] {
            let mut input = f.bytes.clone();
            input.push(extra);
            assert_eq!(
                f.wire.verdict(&input),
                Verdict::Refused(CodecError::Malformed),
                "{:?} fixture {} plus {:02X?}: {:02X?}",
                f.wire,
                f.name,
                extra,
                input
            );
            count += 1;
        }
        assert_eq!(
            f.wire.verdict(&f.bytes),
            Verdict::Accepted(f.bytes.clone()),
            "control: {:?} fixture {} without the extra byte",
            f.wire,
            f.name
        );
    }
    assert_eq!(count, 51);
}

#[test]
fn foreign_first_bytes_are_unsupported() {
    let fx = fixtures();
    let mut cross = 0usize;
    for f in &fx {
        for wire in WIRES {
            if wire == f.wire {
                continue;
            }
            assert_eq!(
                wire.verdict(&f.bytes),
                Verdict::Refused(CodecError::UnsupportedVersion),
                "{wire:?} reading fixture {} of {:?}: {:02X?}",
                f.name,
                f.wire,
                f.bytes
            );
            cross += 1;
        }
        for b in 0..=255u8 {
            if b == f.bytes[0] {
                continue;
            }
            let input = with_byte(&f.bytes, 0, b);
            assert_eq!(
                f.wire.verdict(&input),
                Verdict::Refused(CodecError::UnsupportedVersion),
                "{:?} fixture {} with version byte {b:02X}: {:02X?}",
                f.wire,
                f.name,
                input
            );
        }
        assert_eq!(
            f.wire.verdict(&f.bytes),
            Verdict::Accepted(f.bytes.clone()),
            "control: {:?} fixture {} on its own wire",
            f.wire,
            f.name
        );
    }
    assert_eq!(cross, 119);
}

#[test]
fn foreign_lanes_are_wrong_direction() {
    let fx = fixtures();
    for f in &fx {
        let at = f.wire.lane_at();
        for v in 0..=255u8 {
            if v == f.wire.lane() {
                continue;
            }
            let input = with_byte(&f.bytes, at, v);
            assert_eq!(
                f.wire.verdict(&input),
                Verdict::Refused(CodecError::WrongDirection),
                "{:?} fixture {} with lane byte {v:02X} at {at}: {:02X?}",
                f.wire,
                f.name,
                input
            );
        }
        assert_eq!(
            f.wire.verdict(&with_byte(&f.bytes, at, f.wire.lane())),
            Verdict::Accepted(f.bytes.clone()),
            "control: {:?} fixture {} with its own lane byte",
            f.wire,
            f.name
        );
    }
}

fn tag_expected(name: &str, t: u8, original: &[u8], mutated: &[u8]) -> Verdict {
    match name {
        "E1" | "E2" => match t {
            0 => Verdict::Refused(CodecError::Malformed),
            1 => Verdict::Accepted(original.to_vec()),
            _ => Verdict::Refused(CodecError::UnknownMessage),
        },
        "E3" => match t {
            0 => Verdict::Refused(CodecError::Malformed),
            1 => Verdict::Accepted(original.to_vec()),
            2..=4 => Verdict::Accepted(mutated.to_vec()),
            5 => Verdict::Refused(CodecError::Malformed),
            _ => Verdict::Refused(CodecError::UnknownMessage),
        },
        "E4" => match t {
            0 => Verdict::Refused(CodecError::Malformed),
            1 => Verdict::Accepted(original.to_vec()),
            2..=4 => Verdict::Accepted(mutated.to_vec()),
            5 | 6 => Verdict::Refused(CodecError::Malformed),
            _ => Verdict::Refused(CodecError::UnknownMessage),
        },
        "MH" => match t {
            0 => Verdict::Refused(CodecError::Malformed),
            1 => Verdict::Accepted(original.to_vec()),
            _ => Verdict::Refused(CodecError::UnknownMessage),
        },
        "P81" | "P82" => match t {
            0x10..=0x12 => Verdict::Refused(CodecError::WrongDirection),
            0x20 => Verdict::Accepted(original.to_vec()),
            0x21 => Verdict::Refused(CodecError::Malformed),
            _ => Verdict::Refused(CodecError::UnknownMessage),
        },
        "Q91" => match t {
            0x30..=0x32 => Verdict::Refused(CodecError::WrongDirection),
            0x40 => Verdict::Accepted(original.to_vec()),
            _ => Verdict::Refused(CodecError::UnknownMessage),
        },
        other => panic!("no W-8 tag row for {other}"),
    }
}

#[test]
fn tag_matrix() {
    let fx = fixtures();
    let names = ["E1", "E2", "E3", "E4", "MH", "P81", "P82", "Q91"];
    for name in names {
        let f = fixture(&fx, name);
        for t in 0..=255u8 {
            let input = with_byte(&f.bytes, f.tag_at, t);
            let expected = tag_expected(name, t, &f.bytes, &input);
            assert_eq!(
                f.wire.verdict(&input),
                expected,
                "{:?} fixture {} with tag {t:02X}: {:02X?}",
                f.wire,
                f.name,
                input
            );
        }
        let control = with_byte(&f.bytes, f.tag_at, f.bytes[f.tag_at]);
        assert_eq!(
            f.wire.verdict(&control),
            Verdict::Accepted(f.bytes.clone()),
            "control: {:?} fixture {} with its own tag",
            f.wire,
            f.name
        );
    }
}

#[test]
fn zero_ids_and_field_order() {
    let fx = fixtures();

    for name in ["E1", "E2", "E3", "E4", "MH"] {
        let f = fixture(&fx, name);
        let seq_at = 18;
        let actor_at = if name == "MH" { 19 } else { 20 };
        let tag_at = if name == "MH" { 20 } else { 21 };

        assert_eq!(
            f.wire.verdict(&with_byte(&f.bytes, seq_at, 0)),
            Verdict::Refused(CodecError::Malformed),
            "{name} seq 0: {:02X?}",
            with_byte(&f.bytes, seq_at, 0)
        );
        assert_eq!(
            f.wire.verdict(&with_byte(&f.bytes, actor_at, 0)),
            Verdict::Refused(CodecError::Malformed),
            "{name} actor 0: {:02X?}",
            with_byte(&f.bytes, actor_at, 0)
        );

        let unknown_tag = with_byte(&f.bytes, tag_at, 0xFF);
        assert_eq!(
            f.wire.verdict(&unknown_tag),
            Verdict::Refused(CodecError::UnknownMessage),
            "{name} tag FF alone: {:02X?}",
            unknown_tag
        );
        assert_eq!(
            f.wire.verdict(&with_byte(&unknown_tag, seq_at, 0)),
            Verdict::Refused(CodecError::Malformed),
            "{name} tag FF with seq 0: {:02X?}",
            with_byte(&unknown_tag, seq_at, 0)
        );
        assert_eq!(
            f.wire.verdict(&with_byte(&unknown_tag, actor_at, 0)),
            Verdict::Refused(CodecError::Malformed),
            "{name} tag FF with actor 0: {:02X?}",
            with_byte(&unknown_tag, actor_at, 0)
        );

        assert_eq!(
            f.wire.verdict(&with_byte(&f.bytes, seq_at, 1)),
            Verdict::Accepted(f.bytes.clone()),
            "control: {name} seq 1"
        );
        assert_eq!(
            f.wire.verdict(&with_byte(&f.bytes, actor_at, 7)),
            Verdict::Accepted(f.bytes.clone()),
            "control: {name} actor 7"
        );
    }

    for name in ["D1", "D2", "D3", "D4"] {
        let f = fixture(&fx, name);
        assert_eq!(
            f.wire.verdict(&with_byte(&f.bytes, 49, 0)),
            Verdict::Refused(CodecError::Malformed),
            "{name} target 0: {:02X?}",
            with_byte(&f.bytes, 49, 0)
        );
        assert_eq!(
            f.wire.verdict(&with_byte(&f.bytes, 49, 9)),
            Verdict::Accepted(f.bytes.clone()),
            "control: {name} target 9"
        );
    }

    for name in ["P81", "P82"] {
        let f = fixture(&fx, name);
        assert_eq!(
            f.wire.verdict(&with_byte(&f.bytes, 19, 0)),
            Verdict::Refused(CodecError::Malformed),
            "{name} transfer 0: {:02X?}",
            with_byte(&f.bytes, 19, 0)
        );
        assert_eq!(
            f.wire.verdict(&with_byte(&f.bytes, 19, 1)),
            Verdict::Accepted(f.bytes.clone()),
            "control: {name} transfer 1"
        );
    }

    let mt = fixture(&fx, "MT");
    assert_eq!(
        mt.wire.verdict(&with_byte(&mt.bytes, 19, 0)),
        Verdict::Refused(CodecError::Malformed),
        "MT actor 0: {:02X?}",
        with_byte(&mt.bytes, 19, 0)
    );
    assert_eq!(
        mt.wire.verdict(&with_byte(&mt.bytes, 19, 7)),
        Verdict::Accepted(mt.bytes.clone()),
        "control: MT actor 7"
    );

    let q91 = fixture(&fx, "Q91");
    assert_eq!(
        q91.wire.verdict(&with_byte(&q91.bytes, 19, 0)),
        Verdict::Refused(CodecError::Malformed),
        "Q91 fence 0: {:02X?}",
        with_byte(&q91.bytes, 19, 0)
    );
    assert_eq!(
        q91.wire.verdict(&with_byte(&q91.bytes, 19, 1)),
        Verdict::Accepted(q91.bytes.clone()),
        "control: Q91 fence 1"
    );
}

#[test]
fn hostile_lengths_are_bounded() {
    let fx = fixtures();

    for (name, version) in [("D1", 1u8), ("D2", 2), ("D3", 3), ("D4", 4)] {
        let f = fixture(&fx, name);
        let head = &f.bytes[..22];
        let over_bound = [0x81u8, 0x02];
        let in_bound = [0x80u8, 0x02];

        let mut absent_257 = head.to_vec();
        absent_257.extend_from_slice(&over_bound);
        let expected = if version == 1 {
            Verdict::Refused(CodecError::Malformed)
        } else {
            Verdict::Refused(CodecError::LimitExceeded)
        };
        assert_eq!(
            f.wire.verdict(&absent_257),
            expected,
            "{name} length 257 with no body: {:02X?}",
            absent_257
        );

        let mut absent_256 = head.to_vec();
        absent_256.extend_from_slice(&in_bound);
        assert_eq!(
            f.wire.verdict(&absent_256),
            Verdict::Refused(CodecError::Malformed),
            "{name} length 256 with no body: {:02X?}",
            absent_256
        );

        let mut body_257 = head.to_vec();
        body_257.extend_from_slice(&over_bound);
        body_257.extend_from_slice(&[b'A'; 257]);
        body_257.push(0x09);
        assert_eq!(
            f.wire.verdict(&body_257),
            Verdict::Refused(CodecError::LimitExceeded),
            "{name} length 257 with a body: {:02X?}",
            body_257
        );

        let mut body_256 = head.to_vec();
        body_256.extend_from_slice(&in_bound);
        body_256.extend_from_slice(&[b'A'; 256]);
        body_256.push(0x09);
        assert_eq!(
            f.wire.verdict(&body_256),
            Verdict::Refused(CodecError::Malformed),
            "{name} length 256 with a body: {:02X?}",
            body_256
        );
    }

    let q91 = fixture(&fx, "Q91");
    let cases = [
        (20usize, 129u8, CodecError::LimitExceeded),
        (20, 128, CodecError::Malformed),
        (20, 15, CodecError::Malformed),
        (37, 5, CodecError::LimitExceeded),
        (37, 0, CodecError::Malformed),
        (38, 1, CodecError::UnknownMessage),
    ];
    for (at, value, expected) in cases {
        let input = with_byte(&q91.bytes, at, value);
        assert_eq!(
            q91.wire.verdict(&input),
            Verdict::Refused(expected),
            "Q91 byte {at} := {value}: {:02X?}",
            input
        );
    }
    assert_eq!(
        q91.wire.verdict(&q91.bytes),
        Verdict::Accepted(q91.bytes.clone()),
        "control: Q91 proof length 16"
    );
}

#[test]
fn downstream_frames_sent_up_are_refused() {
    let cancel = TransferDown::Cancel {
        epoch: EPOCH,
        transfer: tid(1),
        reason: CancelReason::Aborted,
    };
    let c81 = codec_transfer::encode_down(&cancel).unwrap();
    assert_eq!(c81, transfer_up(0x81, 0x12, &[0x01, 0x02]));
    assert_eq!(
        Wire::T81.verdict(&c81),
        Verdict::Refused(CodecError::WrongDirection),
        "C81 upstream: {:02X?}",
        c81
    );
    assert_eq!(codec_transfer::decode_down(&c81), Ok(cancel.clone()));

    let c82 = transfer_v3::encode_down(&cancel).unwrap();
    assert_eq!(c82, transfer_up(0x82, 0x12, &[0x01, 0x02]));
    assert_eq!(
        Wire::T82.verdict(&c82),
        Verdict::Refused(CodecError::WrongDirection),
        "C82 upstream: {:02X?}",
        c82
    );
    assert_eq!(transfer_v3::decode_down(&c82), Ok(cancel));

    let answer = ResumeDown::Answer(ResumeResult::Rejected(ResumeRejected {
        epoch: EPOCH,
        reason: ResumeRejection::Busy,
    }));
    let j91 = codec_resume::encode_down(&answer).unwrap();
    let mut literal = vec![0x91, protocol_resume::LANE_SESSION, 0x32];
    literal.extend_from_slice(&EPOCH);
    literal.push(0x02);
    assert_eq!(j91, literal);
    assert_eq!(
        Wire::R91.verdict(&j91),
        Verdict::Refused(CodecError::WrongDirection),
        "J91 upstream: {:02X?}",
        j91
    );
    assert!(codec_resume::decode_down(&j91).is_ok());

    let v1_delta = protocol::DeltaFrame {
        lane: LANE_COMBAT,
        server_tick: 5,
        event_seq: 1,
        ops: vec![],
    };
    let z1 = codec::encode_delta(&v1_delta).unwrap();
    assert_eq!(z1, vec![1, 0x00, 0x05, 0x01, 0x00]);
    assert_eq!(
        Wire::V1.verdict(&z1),
        Verdict::Refused(CodecError::Malformed),
        "Z1 upstream: {:02X?}",
        z1
    );
    assert_eq!(codec::decode_delta(&z1), Ok(v1_delta));

    let v2_delta = protocol_v2::DeltaFrame {
        lane: LANE_COMBAT,
        server_tick: 5,
        event_seq: 1,
        ops: vec![],
    };
    let z2 = codec_v2::encode_delta(&v2_delta).unwrap();
    assert_eq!(z2, vec![2, 0x00, 0x05, 0x01, 0x00]);
    assert_eq!(
        Wire::V2.verdict(&z2),
        Verdict::Refused(CodecError::Malformed),
        "Z2 upstream: {:02X?}",
        z2
    );
    assert_eq!(codec_v2::decode_delta(&z2), Ok(v2_delta));

    let v3_delta = protocol_v3::DeltaFrame {
        lane: LANE_COMBAT,
        server_tick: 5,
        event_seq: 1,
        ops: vec![],
    };
    let z3 = codec_v3::encode_delta(&v3_delta).unwrap();
    assert_eq!(z3, vec![3, 0x00, 0x05, 0x01, 0x00]);
    assert_eq!(
        Wire::V3.verdict(&z3),
        Verdict::Refused(CodecError::Malformed),
        "Z3 upstream: {:02X?}",
        z3
    );
    assert_eq!(codec_v3::decode_delta(&z3), Ok(v3_delta));

    let v4_delta = protocol_v4::DeltaFrame {
        lane: LANE_COMBAT,
        server_tick: 5,
        event_seq: 1,
        ops: vec![],
    };
    let z4 = codec_v4::encode_delta(&v4_delta).unwrap();
    assert_eq!(z4, vec![4, 0x00, 0x05, 0x01, 0x00]);
    assert_eq!(
        Wire::V4.verdict(&z4),
        Verdict::Refused(CodecError::Malformed),
        "Z4 upstream: {:02X?}",
        z4
    );
    assert_eq!(codec_v4::decode_delta(&z4), Ok(v4_delta));

    let receipt = MoveDown::Receipt(MoveReceipt {
        epoch: EPOCH,
        seq: 1,
        actor: net(7),
        processed_tick: 41,
        outcome: MoveOutcome::Accepted { start_tick: 42 },
    });
    let k1 = movement_v4::encode_down(&receipt).unwrap();
    let mut literal = vec![0xA1, movement_v4::LANE_MOVEMENT, 0x00];
    literal.extend_from_slice(&EPOCH);
    literal.extend_from_slice(&[0x01, 0x07, 0x29, 0x00, 0x2A]);
    assert_eq!(k1, literal);
    assert_eq!(
        Wire::A1.verdict(&k1),
        Verdict::Refused(CodecError::WrongDirection),
        "K1 upstream: {:02X?}",
        k1
    );
    assert_eq!(movement_v4::decode_down(&k1), Ok(receipt));

    let frames: [(u8, &[u8]); 8] = [
        (0x81, &c81),
        (0x82, &c82),
        (0x91, &j91),
        (1, &z1),
        (2, &z2),
        (3, &z3),
        (4, &z4),
        (0xA1, &k1),
    ];
    for (first, bytes) in frames {
        for wire in WIRES {
            if wire.first_byte() == first {
                continue;
            }
            assert_eq!(
                wire.verdict(bytes),
                Verdict::Refused(CodecError::UnsupportedVersion),
                "{wire:?} reading a frame whose first byte is {first:02X}: {:02X?}",
                bytes
            );
        }
    }
}

#[test]
fn single_byte_corruption_is_refused_or_exact() {
    let fx = fixtures();
    let mut total = 0usize;
    for f in &fx {
        let mut accepted = 0usize;
        let mut refused = 0usize;
        for (i, &original) in f.bytes.iter().enumerate() {
            for v in 0..=255u8 {
                if v == original {
                    continue;
                }
                total += 1;
                let mutated = with_byte(&f.bytes, i, v);
                match f.wire.verdict(&mutated) {
                    Verdict::Refused(CodecError::FrameTooLarge) => panic!(
                        "{:?} fixture {} byte {i} := {v:02X} reported FrameTooLarge: {:02X?}",
                        f.wire, f.name, mutated
                    ),
                    Verdict::Refused(_) => refused += 1,
                    Verdict::Accepted(canonical) => {
                        if canonical == mutated {
                            accepted += 1;
                        } else {
                            let ulid_text = matches!(f.name, "D1" | "D2" | "D3" | "D4")
                                && (23..=48).contains(&i);
                            assert!(
                                ulid_text,
                                "{:?} fixture {} byte {i} := {v:02X} re-encoded to a different value: {:02X?} -> {:02X?}",
                                f.wire, f.name, mutated, canonical
                            );
                            assert_eq!(
                                canonical.len(),
                                mutated.len(),
                                "{:?} fixture {} byte {i}: alias length changed",
                                f.wire,
                                f.name
                            );
                            for (j, (&a, &b)) in canonical.iter().zip(mutated.iter()).enumerate() {
                                if j != i {
                                    assert_eq!(
                                        a, b,
                                        "{:?} fixture {} byte {i} := {v:02X}: alias differs at {j}: {:02X?} -> {:02X?}",
                                        f.wire, f.name, mutated, canonical
                                    );
                                }
                            }
                            accepted += 1;
                        }
                    }
                }
            }
        }
        assert!(
            accepted > 0,
            "{:?} fixture {} has no accepted corruption",
            f.wire,
            f.name
        );
        assert!(
            refused > 0,
            "{:?} fixture {} has no refused corruption",
            f.wire,
            f.name
        );
        assert_eq!(
            f.wire.verdict(&f.bytes),
            Verdict::Accepted(f.bytes.clone()),
            "control: {:?} fixture {} unmutated",
            f.wire,
            f.name
        );
    }

    assert_eq!(total, 128_265, "W-12 input count");

    let e3 = fixture(&fx, "E3");
    for tag in [2u8, 3, 4] {
        let mutated = with_byte(&e3.bytes, 21, tag);
        assert_eq!(
            e3.wire.verdict(&mutated),
            Verdict::Accepted(mutated.clone()),
            "E3 tag {tag}: {:02X?}",
            mutated
        );
    }
    let m3 = fixture(&fx, "M3");
    assert_eq!(
        m3.wire.verdict(&with_byte(&m3.bytes, 22, 0)),
        Verdict::Accepted(with_byte(&m3.bytes, 22, 0)),
        "M3 mode 0"
    );
    assert_eq!(
        m3.wire.verdict(&with_byte(&m3.bytes, 22, 2)),
        Verdict::Refused(CodecError::UnknownMessage),
        "M3 mode 2"
    );
}

#[test]
fn seeded_noise_is_stable() {
    let mut rng = XorShift64(SEED);
    assert_eq!(rng.next(), 0x4B10_DA10_A18C_50C2);
    assert_eq!(rng.next(), 0x8788_B83B_1805_38E3);
    assert_eq!(rng.next(), 0x18B5_5443_1CC3_6A52);

    let (first, accepted, inputs) = seeded_run(&mut rng);
    assert_eq!(inputs, 17_920, "W-13 input count");
    assert!(
        accepted >= 4_352,
        "only {accepted} accepted verdicts in (c) and (d)"
    );

    let mut again = XorShift64(SEED);
    assert_eq!(again.next(), 0x4B10_DA10_A18C_50C2);
    assert_eq!(again.next(), 0x8788_B83B_1805_38E3);
    assert_eq!(again.next(), 0x18B5_5443_1CC3_6A52);
    let (second, _, _) = seeded_run(&mut again);
    assert_eq!(first, second, "the seeded run is not deterministic");

    let fx = fixtures();
    let e3 = fixture(&fx, "E3");
    assert!(
        stable(Wire::V3, &with_byte(&e3.bytes, 21, 2)),
        "control: E3 tag 2 is stable through the accepted branch"
    );
}

fn seeded_run(rng: &mut XorShift64) -> (Vec<Verdict>, usize, usize) {
    let mut verdicts = Vec::new();
    let mut accepted = 0usize;
    let mut inputs = 0usize;

    for wire in WIRES {
        for _ in 0..1024 {
            inputs += 1;
            let len = 1 + (rng.next() % 64) as usize;
            let mut input = Vec::new();
            for _ in 0..len {
                input.push(rng.next() as u8);
            }
            input[0] = wire.first_byte();
            if len > wire.lane_at() {
                input[wire.lane_at()] = wire.lane();
            }
            assert!(
                stable(wire, &input),
                "(a) {wire:?} is not stable on {:02X?}",
                input
            );
            verdicts.push(wire.verdict(&input));
        }
    }

    for _ in 0..1024 {
        inputs += 1;
        let len = (rng.next() % 65) as usize;
        let mut input = Vec::new();
        for _ in 0..len {
            input.push(rng.next() as u8);
        }
        for wire in WIRES {
            assert!(
                stable(wire, &input),
                "(b) {wire:?} is not stable on {:02X?}",
                input
            );
            verdicts.push(wire.verdict(&input));
        }
    }

    for f in fixtures() {
        let epoch_at = if matches!(f.name, "P81" | "R81" | "P82" | "R82" | "Q91") {
            3usize
        } else {
            1usize
        };
        for _ in 0..256 {
            inputs += 1;
            let i = epoch_at + (rng.next() % 16) as usize;
            let v = rng.next() as u8;
            let mutated = with_byte(&f.bytes, i, v);
            let verdict = f.wire.verdict(&mutated);
            assert_eq!(
                verdict,
                Verdict::Accepted(mutated.clone()),
                "(c) {:?} fixture {} epoch byte {i} := {v:02X}: {:02X?}",
                f.wire,
                f.name,
                mutated
            );
            accepted += 1;
            verdicts.push(verdict);
        }
        for _ in 0..256 {
            inputs += 1;
            let span = f.bytes.len() - f.tag_at;
            let i = f.tag_at + (rng.next() % span as u64) as usize;
            let v = rng.next() as u8;
            let mutated = with_byte(&f.bytes, i, v);
            assert!(
                stable(f.wire, &mutated),
                "(d) {:?} fixture {} byte {i} := {v:02X} is not stable: {:02X?}",
                f.wire,
                f.name,
                mutated
            );
            let verdict = f.wire.verdict(&mutated);
            if matches!(verdict, Verdict::Accepted(_)) {
                accepted += 1;
            }
            verdicts.push(verdict);
        }
    }

    (verdicts, accepted, inputs)
}

#[test]
fn refusal_texts_are_fixed() {
    assert_eq!(
        CodecError::FrameTooLarge.to_string(),
        "frame exceeds its byte cap"
    );
    assert_eq!(
        CodecError::UnsupportedVersion.to_string(),
        "unsupported protocol version"
    );
    assert_eq!(
        CodecError::WrongDirection.to_string(),
        "wrong lane or direction"
    );
    assert_eq!(
        CodecError::UnknownMessage.to_string(),
        "unknown message discriminant"
    );
    assert_eq!(CodecError::Malformed.to_string(), "malformed frame");
    assert_eq!(CodecError::LimitExceeded.to_string(), "wire bound exceeded");
}

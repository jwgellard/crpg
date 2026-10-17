//! T128 audit-gap checks: deterministic tests for the unverified audit leads
//! in `docs/reviews/audit-2026-10-16/findings.md` (crpg-net rows).
//!
//! Net-local only: hand-built [`Ulid`]s and fixed tables (no `HashMap`,
//! no clock, no threads, no I/O, no RNG). Helpers are copied verbatim from
//! `tests/codec.rs`; this file edits no existing test.
//!
//! Lead #31 (token-bucket backwards-timestamp double credit) reproduced as a
//! defect: both of its tests fail against the current code and are kept out
//! of this commit per the T128 brief (see the T128 worker report).

use crpg_core::Ulid;
use crpg_net::codec::{decode_intent, encode_intent, CodecError};
use crpg_net::protocol::{IntentBody, IntentFrame, NetId, LANE_COMBAT};

// ---------------------------------------------------------------------------
// Fixtures (copied from tests/codec.rs).
// ---------------------------------------------------------------------------

const EPOCH: [u8; 16] = [
    0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F, 0x10,
];

fn net(raw: u64) -> NetId {
    NetId::new(raw).expect("fixture id is nonzero")
}

fn ability() -> Ulid {
    Ulid::from_parts(0x0123_4567_89AB, 0x0CDE_F012_3456_789A)
}

fn declare_frame() -> IntentFrame {
    IntentFrame {
        epoch: EPOCH,
        lane: LANE_COMBAT,
        seq: 1,
        observed_tick: 41,
        actor: net(7),
        body: IntentBody::DeclareAction {
            ability: ability(),
            target: net(9),
        },
    }
}

// ---------------------------------------------------------------------------
// Lead #32 (assumption): `decode_intent` with both a bad `seq` and trailing
// bytes reports the error the documented decode order puts first
// (crates/crpg-net/AGENTS.md:71: trailing-byte check → semantic value
// checks, first failure wins). Trailing bytes are `Malformed` (pinned by
// tests/codec.rs `trailing_bytes_rejected`) and out-of-range `seq` is
// `Malformed` (pinned by `seq_bounds_rejected_both_directions`), so the
// documented order's first failure and the competing failure are the same
// variant: the combined hostile input must decode as `Err(Malformed)` and
// never as success or another variant (v1 never ignores-and-continues).
// ---------------------------------------------------------------------------

#[test]
fn bad_seq_and_trailing_bytes_report_malformed() {
    let seq_pos = 1 + 16 + 1; // version(1) + epoch(16) + lane(1)

    for trailing in [0x00u8, 0xFF] {
        let mut bytes = encode_intent(&declare_frame()).expect("valid intent encodes");
        assert_eq!(bytes[seq_pos], 0x01, "fixture seq is a one-byte varint");
        bytes[seq_pos] = 0x00; // seq 0: semantic `Malformed`
        bytes.push(trailing); // trailing byte after the complete message
        assert_eq!(
            decode_intent(&bytes),
            Err(CodecError::Malformed),
            "bad seq with trailing byte {trailing:#X} is rejected as Malformed"
        );
    }

    // Control: the same bad-seq frame without a trailing byte gives the same
    // verdict, so the combined case differs from neither failure alone — the
    // first-failure ordering between the two `Malformed` checks is not
    // observable at the public surface.
    let mut bytes = encode_intent(&declare_frame()).expect("valid intent encodes");
    bytes[seq_pos] = 0x00;
    assert_eq!(decode_intent(&bytes), Err(CodecError::Malformed));
}

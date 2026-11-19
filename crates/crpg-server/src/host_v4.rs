//! Protocol v4 session state beside a v3 session (T037b; contract Reading 1).
//!
//! A v4 session is a v3 session plus [`V4State`]: the per-epoch
//! [`PositionView`] that remembers what the viewer was last told of each
//! position (T026p3) and the operator's withheld-position list (Reading 16).
//! Everything else — ids, facts, outbox, pacing, throttle — stays the v3
//! session's, so no v3 rule is forked.
//!
//! [`position_rows`] is HM-6 (B-4): one row per disclosed, present, live
//! mover, `granted` exactly when the operator has not withheld that entity's
//! position from this viewer. Nothing here reads a clock or mints an id.
//!
//! T037c adds the session's sample datagrams and the lane-1 egress they count
//! toward (Reading 23): the queue replaces, never appends (C-5), and a frame
//! or byte in it is one of the session's egress frames and bytes.
//!
//! T037d adds [`MoveLane`] (Reading 9, D-1 to D-4): the lane-1 sequence, the
//! admitted slots awaiting the next decision window, the released receipts the
//! transport has not taken, the duplicate cache, and the move token bucket.
//! Lane 1 keeps its own state (HM-10) beside lane 0's.

use std::collections::VecDeque;

use crpg_core::EntityId;
use crpg_net::movement_v4::{MoveBody, MoveOutcome};
use crpg_net::positions_v4::PositionRow;
use crpg_net::protocol::{
    POLICY_RETRY_CACHE_BYTES_PER_PEER_LANE, POLICY_RETRY_CACHE_PER_PEER_LANE,
};
use crpg_net::protocol_v4::{NetId, Point};
use crpg_net::sim::TokenBucket;
use crpg_sim::World;

use crate::host::DisclosureGrants;
use crate::host_v3::key_of;

/// Most entries one staged v4 plan holds in the outbox (Reading 18): T141's
/// 90 with the one 992-byte delta replaced by up to five frames.
pub(crate) const OUTBOX_MAX_ENTRIES_V4: usize = 94;
/// Most encoded bytes one staged v4 plan holds in the outbox (Reading 18):
/// 2,851,124 transfer bytes, a 32-byte `Cancel` and 29,760 delta bytes.
pub(crate) const OUTBOX_MAX_BYTES_V4: usize = 2_880_916;

/// The egress reservation of one unreleased lane-1 receipt (Reading 23): the
/// largest receipt a decision can produce (`tasks/T026p.md:607–609`). A slot
/// reserves it from admission until its receipt is released.
pub(crate) const MAX_MOVE_RECEIPT_BYTES: usize = 60;

/// Whether a plan of `entries` frames and `bytes` encoded bytes fits the v4
/// outbox bound (B-7); both bounds are inclusive.
pub(crate) fn outbox_fits_v4(entries: usize, bytes: usize) -> bool {
    entries <= OUTBOX_MAX_ENTRIES_V4 && bytes <= OUTBOX_MAX_BYTES_V4
}

/// One admitted lane-1 intent (D-2 step 9).
#[derive(Debug)]
pub(crate) struct MoveSlot {
    /// The intent's lane-1 sequence.
    pub(crate) seq: u64,
    /// Host-wide admission order relative to this window's other slots
    /// (Reading 13): the last one for an actor wins.
    pub(crate) order: u64,
    /// The decoded intent's canonical bytes, for duplicate detection.
    pub(crate) canonical: Vec<u8>,
    /// The actor as the client named it (a `Rejected` receipt echoes it).
    pub(crate) actor_net: NetId,
    /// The resolved actor; `None` only for a slot refused `NotAuthorized` at
    /// admission, which is decided at once.
    pub(crate) actor: Option<EntityId>,
    /// The verb.
    pub(crate) body: MoveBody,
    /// The outcome and its `processed_tick`, set by the admission refusal or
    /// the next decision window; `None` is pending.
    pub(crate) decided: Option<(MoveOutcome, u64)>,
}

/// One released receipt kept for lane-1 duplicate detection (D-3).
#[derive(Debug)]
pub(crate) struct MoveCacheEntry {
    /// The intent's lane-1 sequence.
    pub(crate) seq: u64,
    /// The decoded intent's canonical bytes.
    pub(crate) canonical: Vec<u8>,
    /// The receipt's encoded bytes, resent verbatim on a duplicate.
    pub(crate) receipt: Vec<u8>,
}

impl MoveCacheEntry {
    /// The cache's accounting unit: canonical bytes plus the receipt.
    fn bytes(&self) -> usize {
        self.canonical.len() + self.receipt.len()
    }
}

/// One session's lane-1 state (Reading 9, HM-10): its own sequence, slots,
/// duplicate cache and move token bucket, never lane 0's.
#[derive(Debug)]
pub(crate) struct MoveLane {
    /// The next `seq` a new intent must carry (consumed by every admitted
    /// intent, D-2 step 9).
    pub(crate) next_seq: u64,
    /// Admitted intents not yet released, in admission (`seq`) order.
    pub(crate) slots: VecDeque<MoveSlot>,
    /// Released receipts still inside the cache bound (D-3).
    pub(crate) cache: VecDeque<MoveCacheEntry>,
    /// Canonical plus receipt bytes of `cache`.
    pub(crate) cache_bytes: usize,
    /// Lane 1's own intent budget (D-2 step 7).
    pub(crate) bucket: TokenBucket,
    /// Released receipts the transport has not taken, in release order.
    pub(crate) receipts: VecDeque<(u64, Vec<u8>)>,
    /// The highest lane-1 `seq` receipted so far; samples echo it (D-12).
    pub(crate) processed_seq: u64,
}

impl MoveLane {
    /// A fresh lane: `seq` 1, nothing pending, a full move bucket.
    pub(crate) fn new() -> Self {
        Self {
            next_seq: crpg_net::protocol::SEQ_FIRST,
            slots: VecDeque::new(),
            cache: VecDeque::new(),
            cache_bytes: 0,
            bucket: TokenBucket::new(
                crpg_net::movement_v4::POLICY_MOVE_INTENT_BURST,
                crpg_net::movement_v4::POLICY_MOVE_INTENTS_PER_SEC,
            ),
            receipts: VecDeque::new(),
            processed_seq: 0,
        }
    }

    /// Reading 11's "unreceipted": the admitted intents not yet released plus
    /// released receipts the transport has not taken.
    pub(crate) fn unreceipted(&self) -> usize {
        self.slots.len() + self.receipts.len()
    }

    /// The lane's egress contribution: one frame and
    /// [`MAX_MOVE_RECEIPT_BYTES`] per unreleased slot, plus the released
    /// receipts at their own length (Reading 23).
    pub(crate) fn egress(&self) -> (usize, usize) {
        let frames = self.slots.len() + self.receipts.len();
        let bytes = self.slots.len() * MAX_MOVE_RECEIPT_BYTES
            + self
                .receipts
                .iter()
                .map(|(_, receipt)| receipt.len())
                .sum::<usize>();
        (frames, bytes)
    }

    /// Inserts one released receipt, evicting the oldest first to stay within
    /// the 256-entry / 2-MiB lane ledger (the cache's retirement path, D-3).
    pub(crate) fn cache_insert(&mut self, entry: MoveCacheEntry) {
        while self.cache.len() >= POLICY_RETRY_CACHE_PER_PEER_LANE
            || self.cache_bytes + entry.bytes() > POLICY_RETRY_CACHE_BYTES_PER_PEER_LANE
        {
            match self.cache.pop_front() {
                Some(old) => self.cache_bytes -= old.bytes(),
                None => break,
            }
        }
        self.cache_bytes += entry.bytes();
        self.cache.push_back(entry);
    }
}

/// The v4 state of one session (Reading 1).
#[derive(Debug)]
pub(crate) struct V4State {
    /// What the viewer was last told of each position (T026p3).
    pub(crate) positions: crpg_net::positions_v4::PositionView,
    /// Entities whose position this viewer is not told (Reading 16);
    /// ascending and unique.
    pub(crate) withheld: Vec<EntityId>,
    /// The sample datagrams of the last committed tick not yet taken (C-5);
    /// a new tick replaces them.
    pub(crate) samples: VecDeque<Vec<u8>>,
    /// The lane-1 state: sequence, slots, cache, receipts and move bucket
    /// (T037d).
    pub(crate) lane: MoveLane,
}

impl V4State {
    /// The session's lane-1 egress, `(frames, bytes)` (C-7, Reading 23): its
    /// queued samples, its released untaken receipts and one reservation per
    /// unreleased slot.
    pub(crate) fn lane1_egress(&self) -> (usize, usize) {
        let sample_frames = self.samples.len();
        let sample_bytes: usize = self.samples.iter().map(Vec::len).sum();
        let (lane_frames, lane_bytes) = self.lane.egress();
        (sample_frames + lane_frames, sample_bytes + lane_bytes)
    }
}

/// B-4 (HM-6): one row per granted, present, live mover, in the disclosure
/// grants' stored order. A position outside the wire bound is impossible (the
/// nav and v4 asserts); such a row is omitted rather than clamped.
pub(crate) fn position_rows(
    world: &World,
    grants: &DisclosureGrants,
    withheld: &[EntityId],
) -> Vec<PositionRow> {
    let mut rows = Vec::new();
    for disclosure in &grants.entities {
        let entity = disclosure.entity;
        if !disclosure.present || !world.contains(entity) {
            continue;
        }
        let Some(mover) = world.movers().get(entity) else {
            continue;
        };
        let raw = mover.position().map(|component| component.to_raw());
        let Some(point) = Point::new(raw) else {
            continue;
        };
        rows.push(PositionRow {
            key: key_of(world, entity),
            granted: withheld.binary_search(&entity).is_err(),
            point,
            moving: mover.route().is_some(),
        });
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::{
        outbox_fits_v4, MoveCacheEntry, MoveLane, MoveSlot, V4State, OUTBOX_MAX_BYTES_V4,
        OUTBOX_MAX_ENTRIES_V4,
    };
    use crate::host_v3::outbox_fits;
    use crpg_net::movement_v4::{
        encode_down, MoveBody, MoveDown, MoveOutcome, MoveReceipt, MAX_MOVE_DOWN_BYTES,
    };
    use crpg_net::positions_v4::PositionView;
    use crpg_net::protocol_v3::RejectionCode;
    use crpg_net::protocol_v4::{NetId, SessionEpoch};
    use std::collections::VecDeque;

    /// B7: both v4 bounds are inclusive and the v3 bound did not move.
    #[test]
    fn b7_outbox_bound_v4() {
        assert_eq!(OUTBOX_MAX_ENTRIES_V4, 94);
        assert_eq!(OUTBOX_MAX_BYTES_V4, 2_880_916);
        assert!(outbox_fits_v4(94, 2_880_916));
        assert!(!outbox_fits_v4(95, 1));
        assert!(!outbox_fits_v4(1, 2_880_917));
        assert!(outbox_fits(90, 2_852_148));
        assert!(!outbox_fits(91, 1));
    }

    /// The reservation constant is the largest receipt exactly: `seq`,
    /// `processed_tick` and `start_tick` at their largest, an actor of ten
    /// varint bytes (D10).
    #[test]
    fn d10_largest_receipt_fits_the_reservation() {
        let largest = MoveReceipt {
            epoch: [0xFF; 16],
            seq: u64::MAX - 1,
            actor: NetId::new(u64::MAX).expect("nonzero"),
            processed_tick: u64::MAX,
            outcome: MoveOutcome::Accepted {
                start_tick: u64::MAX,
            },
        };
        let encoded = encode_down(&MoveDown::Receipt(largest)).expect("encodes");
        assert_eq!(encoded.len(), super::MAX_MOVE_RECEIPT_BYTES);
        assert!(encoded.len() <= MAX_MOVE_DOWN_BYTES);
        assert_eq!(super::MAX_MOVE_RECEIPT_BYTES, 60);
        // Every other outcome is smaller, so the reservation always fits.
        let superseded = encode_down(&MoveDown::Receipt(MoveReceipt {
            outcome: MoveOutcome::Superseded,
            ..largest
        }))
        .expect("encodes");
        let rejected = encode_down(&MoveDown::Receipt(MoveReceipt {
            outcome: MoveOutcome::Rejected(RejectionCode::IllegalAction),
            ..largest
        }))
        .expect("encodes");
        assert!(superseded.len() < super::MAX_MOVE_RECEIPT_BYTES);
        assert!(rejected.len() < super::MAX_MOVE_RECEIPT_BYTES);
    }

    /// Gap 10 (Reading 23, mutants l10-l12): `MoveLane::egress` counts one
    /// frame and `MAX_MOVE_RECEIPT_BYTES` per unreleased slot plus released
    /// receipts at their own length, and `V4State::lane1_egress` adds the
    /// queued sample datagrams beside the lane. A lane that ignored slots,
    /// receipts or samples would report the wrong frames and bytes here.
    #[test]
    fn lane_egress_counts_slots_receipts_and_samples() {
        fn slot(seq: u64) -> MoveSlot {
            MoveSlot {
                seq,
                order: seq,
                canonical: vec![9; 10],
                actor_net: NetId::new(1).expect("nonzero"),
                actor: None,
                body: MoveBody::Halt,
                decided: None,
            }
        }
        // Control: a fresh lane contributes nothing.
        let mut lane = MoveLane::new();
        assert_eq!(lane.egress(), (0, 0));
        // One admitted slot reserves one frame and 60 bytes.
        lane.slots.push_back(slot(1));
        assert_eq!(lane.egress(), (1, super::MAX_MOVE_RECEIPT_BYTES));
        assert_eq!(lane.egress(), (1, 60));
        // A second slot reserves a second frame and 60 more bytes.
        lane.slots.push_back(slot(2));
        assert_eq!(lane.egress(), (2, 120));
        // Releasing the first slot swaps its 60-byte reservation for its own
        // 25-byte receipt: one slot plus one receipt.
        let first = lane.slots.pop_front().expect("a slot");
        lane.cache_insert(MoveCacheEntry {
            seq: first.seq,
            canonical: first.canonical,
            receipt: vec![0; 25],
        });
        lane.receipts.push_back((1, vec![0; 25]));
        assert_eq!(lane.egress(), (2, 85));
        // Releasing the second with a 30-byte receipt leaves two receipts.
        let second = lane.slots.pop_front().expect("a slot");
        lane.cache_insert(MoveCacheEntry {
            seq: second.seq,
            canonical: second.canonical,
            receipt: vec![0; 30],
        });
        lane.receipts.push_back((2, vec![0; 30]));
        assert_eq!(lane.egress(), (2, 55));

        // The session adds its queued samples beside the lane.
        let state = V4State {
            positions: PositionView::new(SessionEpoch([7; 16])),
            withheld: Vec::new(),
            samples: VecDeque::from([vec![1; 100], vec![2; 50]]),
            lane,
        };
        assert_eq!(state.lane1_egress(), (4, 205));
        // Control: samples alone count, and an empty session counts zero.
        let mut samples_only = V4State {
            positions: PositionView::new(SessionEpoch([7; 16])),
            withheld: Vec::new(),
            samples: VecDeque::from([vec![3; 100]]),
            lane: MoveLane::new(),
        };
        assert_eq!(samples_only.lane1_egress(), (1, 100));
        samples_only.samples.clear();
        assert_eq!(samples_only.lane1_egress(), (0, 0));
    }
}

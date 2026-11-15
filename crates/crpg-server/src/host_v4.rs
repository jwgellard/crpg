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

use std::collections::VecDeque;

use crpg_core::EntityId;
use crpg_net::positions_v4::PositionRow;
use crpg_net::protocol_v4::Point;
use crpg_sim::World;

use crate::host::DisclosureGrants;
use crate::host_v3::key_of;

/// Most entries one staged v4 plan holds in the outbox (Reading 18): T141's
/// 90 with the one 992-byte delta replaced by up to five frames.
pub(crate) const OUTBOX_MAX_ENTRIES_V4: usize = 94;
/// Most encoded bytes one staged v4 plan holds in the outbox (Reading 18):
/// 2,851,124 transfer bytes, a 32-byte `Cancel` and 29,760 delta bytes.
pub(crate) const OUTBOX_MAX_BYTES_V4: usize = 2_880_916;

/// Whether a plan of `entries` frames and `bytes` encoded bytes fits the v4
/// outbox bound (B-7); both bounds are inclusive.
pub(crate) fn outbox_fits_v4(entries: usize, bytes: usize) -> bool {
    entries <= OUTBOX_MAX_ENTRIES_V4 && bytes <= OUTBOX_MAX_BYTES_V4
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
    /// The session's lane-1 egress: frames and encoded bytes counted beside
    /// its delivery log (C-7, Reading 23). T037c counts samples; T037d adds
    /// receipts and reservations.
    pub(crate) egress: (usize, usize),
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
    use super::{outbox_fits_v4, OUTBOX_MAX_BYTES_V4, OUTBOX_MAX_ENTRIES_V4};
    use crate::host_v3::outbox_fits;

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
}

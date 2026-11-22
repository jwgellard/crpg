//! T049a: presentation-only interpolation of disclosed positions.
//!
//! A v4 session receives one position per entity per server tick while it
//! moves, so drawing the newest sample steps at 20 Hz. [`Smoother`] keeps a
//! short per-entity history and answers where to draw each row a fixed
//! [`RENDER_DELAY_MS`] behind the estimated server time (ADR-0036 G1/U5/U7).
//!
//! It is fed from the replica's disclosed positions and writes nothing back:
//! the replica, the outbox, the view and every frame stay exactly as they
//! were. All arithmetic is integer (time is `i128`), so it is exact on both
//! targets and never carries a coordinate or a tick past its limits.

use std::collections::{BTreeMap, VecDeque};

use crpg_net::protocol::NetId;
use crpg_net::replica_v4::PositionState;

/// Milliseconds per server tick; equals `crpg_server::run::TICK_PERIOD_MS`.
pub(crate) const TICK_MS: u64 = 50;
/// How far behind the estimated server time rows are drawn (ADR-0036 U7).
pub(crate) const RENDER_DELAY_MS: u64 = 100;
/// Samples kept per entity; the oldest is dropped beyond this.
pub(crate) const SAMPLES_PER_ENTITY: usize = 8;

/// One buffered position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Sample {
    /// The server tick the position belongs to.
    pub(crate) tick: u64,
    /// The raw `Fx16_16` point (`Point::raw()`).
    pub(crate) point: [i32; 3],
    /// Whether the entity had a route on that tick.
    pub(crate) moving: bool,
}

/// Per-entity sample buffers and the sample clock offset.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Smoother {
    buffers: BTreeMap<NetId, VecDeque<Sample>>,
    offset_ms: Option<i128>,
}

impl Smoother {
    /// An empty smoother: no buffer, no offset.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Records one sample's server tick, received at `now_ms` (B-S1).
    ///
    /// The offset is the minimum over every sample of
    /// `arrival − server_tick × TICK_MS`, so the least-delayed arrival wins
    /// and a late or old sample never moves it back.
    pub(crate) fn observe_clock(&mut self, server_tick: u64, now_ms: u64) {
        let observed = i128::from(now_ms) - ms(server_tick);
        self.offset_ms = Some(match self.offset_ms {
            None => observed,
            Some(current) => current.min(observed),
        });
    }

    /// Brings the buffers in line with the replica's disclosed positions
    /// (B-S2).
    ///
    /// Newer tick appends; the same tick replaces the newest only when the
    /// value differs; a lower tick restarts the buffer (a revoke and re-anchor
    /// inside one frame); an id with no disclosed position is dropped at once.
    pub(crate) fn observe(&mut self, positions: &BTreeMap<NetId, PositionState>) {
        for (id, state) in positions {
            let sample = Sample {
                tick: state.tick,
                point: state.point.raw(),
                moving: state.moving,
            };
            match self.buffers.get_mut(id) {
                None => {
                    self.buffers.insert(*id, VecDeque::from([sample]));
                }
                Some(buffer) => match buffer.back().copied() {
                    Some(newest) if newest.tick < sample.tick => {
                        buffer.push_back(sample);
                        while buffer.len() > SAMPLES_PER_ENTITY {
                            buffer.pop_front();
                        }
                    }
                    Some(newest) if newest.tick == sample.tick => {
                        if newest != sample {
                            if let Some(back) = buffer.back_mut() {
                                *back = sample;
                            }
                        }
                    }
                    // A lower tick (revoked and re-anchored within one frame),
                    // or an impossible empty buffer: restart from this sample.
                    _ => {
                        buffer.clear();
                        buffer.push_back(sample);
                    }
                },
            }
        }
        self.buffers.retain(|id, _| positions.contains_key(id));
    }

    /// Where to draw `entity` at `now_ms`; `None` without a buffer (B-S3).
    ///
    /// The render time is `now_ms − offset − RENDER_DELAY_MS`. At or after the
    /// newest sample the newest point is held (never extrapolated); at or
    /// before the oldest the oldest point is drawn; between two samples the
    /// point is interpolated, over the last tick only when the earlier sample
    /// was at rest.
    pub(crate) fn render(&self, entity: NetId, now_ms: u64) -> Option<[i64; 3]> {
        let buffer = self.buffers.get(&entity)?;
        let newest = buffer.back()?;
        let Some(offset_ms) = self.offset_ms else {
            return Some(to_i64(newest.point));
        };
        let render_at = i128::from(now_ms) - offset_ms - i128::from(RENDER_DELAY_MS);
        let newest_at = ms(newest.tick);
        if render_at >= newest_at {
            return Some(to_i64(newest.point));
        }
        let oldest = buffer.front()?;
        let oldest_at = ms(oldest.tick);
        if render_at <= oldest_at {
            return Some(to_i64(oldest.point));
        }
        let mut previous: Option<&Sample> = None;
        for sample in buffer {
            if let Some(a) = previous {
                let a_at = ms(a.tick);
                let c_at = ms(sample.tick);
                if a_at <= render_at && render_at < c_at {
                    return Some(interpolate(a, sample, a_at, c_at, render_at));
                }
            }
            previous = Some(sample);
        }
        // The pair always exists when `oldest_at < render_at < newest_at`;
        // holding the newest point keeps the draw bounded if it ever does not.
        Some(to_i64(newest.point))
    }

    /// Test seam: the buffer of `entity`, oldest first.
    #[cfg(test)]
    pub(crate) fn buffer(&self, entity: NetId) -> Option<&VecDeque<Sample>> {
        self.buffers.get(&entity)
    }

    /// Test seam: the sample clock offset.
    #[cfg(test)]
    pub(crate) fn offset_ms(&self) -> Option<i128> {
        self.offset_ms
    }
}

/// `tick × TICK_MS`, in the wide type every time computation uses.
fn ms(tick: u64) -> i128 {
    i128::from(tick) * i128::from(TICK_MS)
}

/// A raw point widened to the render type.
fn to_i64(point: [i32; 3]) -> [i64; 3] {
    point.map(i64::from)
}

/// One point between `a` and `c` at `render_at` (U5's formula).
///
/// The motion starts at `a`'s tick for a moving sample; a resting sample
/// moves only over its last tick, so a rest followed by a move does not creep
/// across the whole gap. The division truncates toward zero.
fn interpolate(a: &Sample, c: &Sample, a_at: i128, c_at: i128, render_at: i128) -> [i64; 3] {
    let start = if a.moving {
        a_at
    } else {
        a_at.max(c_at - i128::from(TICK_MS))
    };
    if render_at <= start {
        return to_i64(a.point);
    }
    let elapsed = render_at - start;
    let span = c_at - start;
    let mut point = [0_i64; 3];
    for (index, slot) in point.iter_mut().enumerate() {
        let from = i128::from(a.point[index]);
        let to = i128::from(c.point[index]);
        *slot = (from + (to - from) * elapsed / span) as i64;
    }
    point
}

#[cfg(test)]
mod tests;

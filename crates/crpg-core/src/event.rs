//! Generic event substrate: [`EventEnvelope`] and [`EventQueue`].
//!
//! Per ADR-0008 this crate holds the event *mechanism* and nothing else. The
//! queue knows how events are ordered, drained and serialized; it does not
//! know what any event *means*. Concrete vocabularies live above:
//! `SimEvent` in `crpg-sim`, event-IR graph types in `crpg-data`, kernel
//! hooks in `crpg-rules`.
//!
//! The payload type `P` is deliberately unconstrained. Restricting it to
//! core-closed field types (`EntityId`, `Tick`, integers / `Fx16_16`, `Ulid`,
//! `String` — never `StatId`/`TagId` handles per ADR-0006 Decision 4, never
//! `rules`/`sim` types) is enforced by review, not by the compiler: a bound
//! would take a trait, and ADR-0008 forbids new traits. Instantiating this
//! queue with a non-core payload is a layering violation, full stop.
//!
//! There is no dispatch here, no routing, no handlers, no game enum. If this
//! module wants any of those, that want is its own task (per ADR-0008).

use serde::{Deserialize, Serialize};

use crate::time::Tick;

/// One queued event: its payload plus the coordinates that order it.
///
/// Field order is the canonical serialization order: tick, then sequence,
/// then payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventEnvelope<P> {
    /// The simulation tick the event was raised at, supplied by the caller.
    pub tick: Tick,
    /// Push order within the queue. Assigned by
    /// [`EventQueue::push`](EventQueue::push), never by callers.
    pub seq: u64,
    /// The event itself. Must be a core-closed type (see module docs).
    pub payload: P,
}

/// A game-agnostic FIFO with an ordering contract: [`drain`](Self::drain)
/// yields envelopes in ascending `(tick, seq)` order, stably, and clears the
/// queue.
///
/// Ticks arrive from callers and may arrive out of order, so the drain sorts.
/// Push order alone is not the contract; `(tick, seq)` is.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventQueue<P> {
    envelopes: Vec<EventEnvelope<P>>,
    next_seq: u64,
}

impl<P> EventQueue<P> {
    /// An empty queue.
    pub fn new() -> Self {
        Self {
            envelopes: Vec::new(),
            next_seq: 0,
        }
    }

    /// Enqueues `payload` stamped with `tick`.
    ///
    /// The sequence number is assigned here, monotonically. It saturates
    /// rather than wraps on overflow: wrapping would silently break the
    /// drain-ordering contract, and `u64::MAX` pushes without a drain is not
    /// a workload that exists.
    pub fn push(&mut self, tick: Tick, payload: P) {
        let seq = self.next_seq;
        self.next_seq = self.next_seq.saturating_add(1);
        self.envelopes.push(EventEnvelope { tick, seq, payload });
    }

    /// Removes and returns every envelope in ascending `(tick, seq)` order.
    ///
    /// The sort is stable, so envelopes sharing a tick keep push order.
    /// Draining an empty queue yields an empty vector.
    pub fn drain(&mut self) -> Vec<EventEnvelope<P>> {
        self.envelopes.sort_by_key(|e| (e.tick, e.seq));
        std::mem::take(&mut self.envelopes)
    }

    /// The number of envelopes currently queued.
    pub fn len(&self) -> usize {
        self.envelopes.len()
    }

    /// Whether the queue holds no envelopes.
    pub fn is_empty(&self) -> bool {
        self.envelopes.is_empty()
    }
}

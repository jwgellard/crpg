//! Generic event substrate: [`EventEnvelope`] and [`EventQueue`].
//!
//! Per ADR-0008 this crate holds the event *mechanism* and nothing else. The
//! queue knows how events are ordered, drained and serialized; it does not
//! know what any event *means*. Concrete vocabularies live above:
//! `SimEvent` in `crpg-sim`, event-IR graph types in `crpg-data`, kernel
//! hooks in `crpg-rules`.
//!
//! The payload type `P` is deliberately unconstrained. The core-closed rule
//! (ADR-0008 Decision 1, clarified by ADR-0011) applies to the *fields
//! composing* a payload — `EntityId`, `Tick`, integers / `Fx16_16`, `Ulid`,
//! `String`, never `StatId`/`TagId` handles per ADR-0006 Decision 4 — while
//! the vocabulary enum itself lives in the crate that owns it (`SimEvent`
//! in `crpg-sim`, IR types in `crpg-data`, hooks in `crpg-rules`). It is
//! enforced by review, not by the compiler: a bound would take a trait, and
//! ADR-0008 forbids new traits. Defining game vocabulary in core, or putting
//! non-core-closed fields in any payload, is a layering violation, full
//! stop; instantiating the generic above with a field-clean enum — as
//! `World.events: EventQueue<SimEvent>` does — is the intended use.
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
///
/// Loading is validated: sequence numbers must be unique and `next_seq` must
/// exceed every queued `seq` (saturation at `u64::MAX` excepted), so a later
/// [`push`](Self::push) cannot reuse a number or reverse push order.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct EventQueue<P> {
    envelopes: Vec<EventEnvelope<P>>,
    next_seq: u64,
}

/// Serialized shape of [`EventQueue`]. Deserialization goes through this
/// shape so sequence counters from untrusted input can be checked before a
/// queue is built.
#[derive(Deserialize)]
struct EventQueueRepr<P> {
    envelopes: Vec<EventEnvelope<P>>,
    next_seq: u64,
}

impl<'de, P: Deserialize<'de>> Deserialize<'de> for EventQueue<P> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let repr = EventQueueRepr::deserialize(deserializer)?;
        let mut seen = std::collections::BTreeSet::new();
        let mut max: Option<u64> = None;
        for envelope in &repr.envelopes {
            if !seen.insert(envelope.seq) {
                return Err(serde::de::Error::custom(format!(
                    "duplicate event sequence {}",
                    envelope.seq
                )));
            }
            max = Some(max.map_or(envelope.seq, |current| current.max(envelope.seq)));
        }
        if let Some(highest) = max {
            // Saturation is unreachable-by-construction but representable: a
            // queue that pushed at `u64::MAX` holds `seq == next_seq == MAX`,
            // and must still round-trip.
            if repr.next_seq != u64::MAX && repr.next_seq <= highest {
                return Err(serde::de::Error::custom(format!(
                    "event next_seq {} must exceed queued max {}",
                    repr.next_seq, highest
                )));
            }
        }
        Ok(Self {
            envelopes: repr.envelopes,
            next_seq: repr.next_seq,
        })
    }
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

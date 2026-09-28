//! Local in-memory `Transport` shape (T018a; E003-B).
//!
//! The byte pipe between two net endpoints in one process. Bounded
//! queueing, injected time, and fault schedules arrive with the T018b
//! simulated transport, which implements this trait; this module invents no
//! contracts trait and no host API.

/// Bounded in-memory byte transport between two net endpoints.
///
/// `send` hands one framed datagram to the peer; `recv` takes the next
/// queued datagram, or [`None`] when the queue is empty. No threads, no
/// clock, no I/O: capacity, ordering, loss, and timing are the T018b
/// implementation's documented policy, exercised through this exact shape.
pub trait Transport {
    /// Send-side failure (for example a full bounded queue).
    type Error;

    /// Queues one framed datagram for the peer.
    fn send(&mut self, bytes: &[u8]) -> Result<(), Self::Error>;

    /// Takes the next queued datagram, or [`None`] when none is waiting.
    fn recv(&mut self) -> Option<Vec<u8>>;
}

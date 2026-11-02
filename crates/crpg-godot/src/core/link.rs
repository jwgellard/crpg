//! One in-process, bounded, two-ended byte link (ADR-0033 §5).
//!
//! The link is two independent FIFOs of whole frames. The authority end sends
//! down and receives up; the client end sends up and receives down. There is
//! no clock, thread, loss or reordering: the only policy is the frame cap.
//! A full queue refuses the send and changes nothing. The client never
//! acknowledges anything here.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use crpg_net::protocol::POLICY_EGRESS_FRAMES_PER_PEER;
use crpg_net::transport::Transport;

/// Frame cap for one link direction: the host's per-peer egress depth.
pub(crate) const LINK_FRAMES: usize = POLICY_EGRESS_FRAMES_PER_PEER;

type Queue = Rc<RefCell<VecDeque<Vec<u8>>>>;

/// The shared pair of queues; each end is a [`LinkEnd`].
pub(crate) struct LocalLink {
    up: Queue,
    down: Queue,
    cap: usize,
}

/// One end of a [`LocalLink`]: a bounded send queue and a receive queue.
pub(crate) struct LinkEnd {
    send: Queue,
    recv: Queue,
    cap: usize,
}

/// A send refused because that direction already holds `cap` frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LinkFull;

impl LocalLink {
    /// Builds a link whose every direction holds at most `cap` frames.
    pub(crate) fn new(cap: usize) -> Self {
        Self {
            up: Rc::new(RefCell::new(VecDeque::with_capacity(cap))),
            down: Rc::new(RefCell::new(VecDeque::with_capacity(cap))),
            cap,
        }
    }

    /// The authority's end: sends downstream, receives upstream.
    pub(crate) fn authority_end(&self) -> LinkEnd {
        LinkEnd {
            send: Rc::clone(&self.down),
            recv: Rc::clone(&self.up),
            cap: self.cap,
        }
    }

    /// The client's end: sends upstream, receives downstream.
    pub(crate) fn client_end(&self) -> LinkEnd {
        LinkEnd {
            send: Rc::clone(&self.up),
            recv: Rc::clone(&self.down),
            cap: self.cap,
        }
    }

    /// The queued frame counts, `(up, down)`.
    #[cfg(test)]
    pub(crate) fn queued(&self) -> (usize, usize) {
        (self.up.borrow().len(), self.down.borrow().len())
    }
}

impl Transport for LinkEnd {
    type Error = LinkFull;

    fn send(&mut self, bytes: &[u8]) -> Result<(), Self::Error> {
        let mut queue = self.send.borrow_mut();
        if queue.len() >= self.cap {
            return Err(LinkFull);
        }
        queue.push_back(bytes.to_vec());
        Ok(())
    }

    fn recv(&mut self) -> Option<Vec<u8>> {
        self.recv.borrow_mut().pop_front()
    }
}

#[cfg(test)]
mod tests;

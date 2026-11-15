//! The client's lane-1 sender state (T026p5, ADR-0036 L2–L3).
//!
//! [`MoveOutbox`] holds the lane-1 rules from the client side: one sequence
//! per epoch, a window of [`POLICY_MOVE_IN_FLIGHT`] unreceipted intents, and
//! exactly one receipt per intent in `seq` order. It does no I/O and reads no
//! clock; the caller encodes what [`MoveOutbox::send`] returns and, after a
//! resume, resends [`MoveOutbox::pending`].

use core::fmt;

use crate::movement_v4::{MoveBody, MoveIntent, MoveReceipt, POLICY_MOVE_IN_FLIGHT};
use crate::protocol::SEQ_LAST;
use crate::protocol_v4::NetId;

/// Why a send or a receipt was refused. A refusal changes nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutboxError {
    /// The next `seq` is above `SEQ_LAST`.
    Exhausted,
    /// [`POLICY_MOVE_IN_FLIGHT`] intents are already unreceipted.
    Full,
    /// The receipt names another epoch.
    WrongEpoch,
    /// The receipt is not for the oldest unreceipted intent.
    Unexpected {
        /// The oldest pending `seq`; `None` when nothing is pending.
        expected: Option<u64>,
        /// The `seq` the receipt carried.
        got: u64,
    },
    /// The receipt's actor is not the oldest pending intent's actor.
    WrongActor {
        /// The receipt's `seq`.
        seq: u64,
    },
}

impl fmt::Display for OutboxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Exhausted => "Exhausted at movement/outbox",
            Self::Full => "Full at movement/outbox",
            Self::WrongEpoch => "WrongEpoch at movement/outbox",
            Self::Unexpected { .. } => "Unexpected at movement/outbox",
            Self::WrongActor { .. } => "WrongActor at movement/outbox",
        })
    }
}

impl std::error::Error for OutboxError {}

/// The client's lane-1 sender state for one epoch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoveOutbox {
    epoch: [u8; 16],
    next_seq: u64,
    pending: Vec<MoveIntent>,
    processed_seq: u64,
}

impl MoveOutbox {
    /// A fresh outbox for `epoch`: the next `seq` is 1, nothing is pending.
    pub fn new(epoch: [u8; 16]) -> Self {
        Self {
            epoch,
            next_seq: 1,
            pending: Vec::new(),
            processed_seq: 0,
        }
    }

    /// The session generation this outbox belongs to.
    pub fn epoch(&self) -> [u8; 16] {
        self.epoch
    }

    /// The `seq` the next [`Self::send`] uses.
    pub fn next_seq(&self) -> u64 {
        self.next_seq
    }

    /// The `seq` of the last receipt accepted; 0 before any.
    pub fn processed_seq(&self) -> u64 {
        self.processed_seq
    }

    /// The unreceipted intents, ascending `seq`; resend these after a resume.
    pub fn pending(&self) -> &[MoveIntent] {
        &self.pending
    }

    /// Mints the next intent for `actor` and holds it until its receipt.
    pub fn send(&mut self, actor: NetId, body: MoveBody) -> Result<MoveIntent, OutboxError> {
        if self.next_seq > SEQ_LAST {
            return Err(OutboxError::Exhausted);
        }
        if self.pending.len() == POLICY_MOVE_IN_FLIGHT {
            return Err(OutboxError::Full);
        }
        let intent = MoveIntent {
            epoch: self.epoch,
            seq: self.next_seq,
            actor,
            body,
        };
        self.pending.push(intent);
        self.next_seq += 1;
        Ok(intent)
    }

    /// Completes the oldest pending intent with its receipt, whatever the
    /// outcome (`Accepted`, `Superseded` or `Rejected`).
    pub fn receive(&mut self, receipt: &MoveReceipt) -> Result<(), OutboxError> {
        if receipt.epoch != self.epoch {
            return Err(OutboxError::WrongEpoch);
        }
        let Some(first) = self.pending.first() else {
            return Err(OutboxError::Unexpected {
                expected: None,
                got: receipt.seq,
            });
        };
        if receipt.seq != first.seq {
            return Err(OutboxError::Unexpected {
                expected: Some(first.seq),
                got: receipt.seq,
            });
        }
        if receipt.actor != first.actor {
            return Err(OutboxError::WrongActor { seq: receipt.seq });
        }
        self.pending.remove(0);
        self.processed_seq = receipt.seq;
        Ok(())
    }
}

//! Resume mechanisms (T025a2): per-session connection fence, explicit host
//! decision and client-side result check.
//!
//! The wire lives in [`crate::protocol_resume`] and [`crate::codec_resume`];
//! this module never authorises anything. A [`SessionFence`] orders a
//! session's connection generations and checks only epoch, fence order and
//! shape, and an [`ResumeDecision`] built by the host after it authenticated
//! the proof is the only input that produces `Accepted`.

use core::fmt;

use crate::protocol_resume::{
    check_lanes, ConnectionFence, LaneProgress, ResumeAccepted, ResumeRejected, ResumeRejection,
    ResumeRequest, ResumeResult,
};

/// The host's verdict on one request, made by the host after it has
/// authenticated the proof and applied its own policy. Net never builds one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResumeDecision {
    /// Resume, reporting the retained ledger's per-lane progress.
    Accept {
        /// Authoritative per-lane progress the accepted result carries.
        progress: Vec<LaneProgress>,
    },
    /// Refuse with a cause-neutral reason.
    Reject(ResumeRejection),
}

/// Why a fence call refused. No bytes, no proof.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResumeError {
    /// The request names another session epoch.
    WrongEpoch,
    /// The presented fence is above the current fence.
    UnknownFence,
    /// The presented fence is below the confirmed fence.
    StaleFence,
    /// A frame arrived on a connection older than the current fence.
    Fenced,
    /// The current fence is the grant limit, so no new grant is possible.
    FenceExhausted,
    /// The decision's progress is not a valid lane list.
    InvalidProgress,
    /// The requested grant limit is below the first grant.
    InvalidLimit,
}

impl fmt::Display for ResumeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::WrongEpoch => "WrongEpoch",
            Self::UnknownFence => "UnknownFence",
            Self::StaleFence => "StaleFence",
            Self::Fenced => "Fenced",
            Self::FenceExhausted => "FenceExhausted",
            Self::InvalidProgress => "InvalidProgress",
            Self::InvalidLimit => "InvalidLimit",
        };
        write!(f, "{} at resume/fence", name)
    }
}

impl std::error::Error for ResumeError {}

/// One session's connection-fence state (S§15). The host keeps one per
/// session for the session's life; it has no restore constructor.
///
/// State is `epoch`, the current fence `c`, the confirmed fence `k` and
/// `max_fence`, with `1 ≤ k ≤ c ≤ max_fence`. `k` is the highest fence on
/// which a frame has been checked; `[k, c]` is the set of fences a request
/// may present.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionFence {
    epoch: [u8; 16],
    current: ConnectionFence,
    confirmed: ConnectionFence,
    max_fence: u64,
}

impl SessionFence {
    /// A fence whose current and confirmed generation is
    /// [`ConnectionFence::FIRST`], with `max_fence = u64::MAX`.
    pub fn new(epoch: [u8; 16]) -> SessionFence {
        SessionFence {
            epoch,
            current: ConnectionFence::FIRST,
            confirmed: ConnectionFence::FIRST,
            max_fence: u64::MAX,
        }
    }

    /// As [`new`](Self::new) with a tightening-only grant limit; a
    /// `max_fence` below 2 is [`ResumeError::InvalidLimit`].
    pub fn with_max(epoch: [u8; 16], max_fence: u64) -> Result<SessionFence, ResumeError> {
        if max_fence < 2 {
            return Err(ResumeError::InvalidLimit);
        }
        Ok(SessionFence {
            max_fence,
            ..SessionFence::new(epoch)
        })
    }

    /// The session epoch this fence belongs to.
    pub fn epoch(&self) -> [u8; 16] {
        self.epoch
    }

    /// The fence the session's live connection holds.
    pub fn current(&self) -> ConnectionFence {
        self.current
    }

    /// The highest fence on which a frame has been checked.
    pub fn confirmed(&self) -> ConnectionFence {
        self.confirmed
    }

    /// The tightening-only grant limit.
    pub fn max_fence(&self) -> u64 {
        self.max_fence
    }

    /// Every inbound frame of a bound connection, before anything else
    /// (S§15.1). `Ok` only for the current fence, which it confirms.
    pub fn check_frame(&mut self, tag: ConnectionFence) -> Result<(), ResumeError> {
        if tag == self.current {
            self.confirmed = self.current;
            Ok(())
        } else if tag < self.current {
            Err(ResumeError::Fenced)
        } else {
            Err(ResumeError::UnknownFence)
        }
    }

    /// Pure: the fence a resolve would grant, or why not (S§15.2), first
    /// failure wins.
    pub fn check_request(&self, request: &ResumeRequest) -> Result<ConnectionFence, ResumeError> {
        if request.epoch != self.epoch {
            return Err(ResumeError::WrongEpoch);
        }
        if request.fence > self.current {
            return Err(ResumeError::UnknownFence);
        }
        if request.fence < self.confirmed {
            return Err(ResumeError::StaleFence);
        }
        if self.current.get() == self.max_fence {
            return Err(ResumeError::FenceExhausted);
        }
        ConnectionFence::new(self.current.get() + 1).ok_or(ResumeError::FenceExhausted)
    }

    /// The one state-changing resume step (S§15.3).
    ///
    /// The fence check comes before the decision is looked at, so a fence
    /// error wins over a `Reject` and over invalid progress. The request's
    /// advisory claims are never read.
    pub fn resolve(
        &mut self,
        request: &ResumeRequest,
        decision: &ResumeDecision,
    ) -> Result<ResumeResult, ResumeError> {
        let granted = self.check_request(request)?;
        match decision {
            ResumeDecision::Reject(reason) => Ok(ResumeResult::Rejected(ResumeRejected {
                epoch: self.epoch,
                reason: *reason,
            })),
            ResumeDecision::Accept { progress } => {
                check_lanes(progress.iter().map(|p| (p.lane, p.finalized_through)))
                    .map_err(|_| ResumeError::InvalidProgress)?;
                self.current = granted;
                Ok(ResumeResult::Accepted(ResumeAccepted {
                    epoch: self.epoch,
                    granted,
                    progress: progress.clone(),
                }))
            }
        }
    }
}

/// Client side: whether `result` answers a resume of `epoch` presented at
/// `presented` (S§15.4). Pure.
pub fn check_result(
    epoch: [u8; 16],
    presented: ConnectionFence,
    result: &ResumeResult,
) -> Result<(), ResumeError> {
    if result.epoch() != epoch {
        return Err(ResumeError::WrongEpoch);
    }
    if let ResumeResult::Accepted(accepted) = result {
        if accepted.granted <= presented {
            return Err(ResumeError::StaleFence);
        }
    }
    Ok(())
}

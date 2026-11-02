//! The bridge's one closed error table (ADR-0033 §11).
//!
//! Codes are stable integers: GDScript switches on the number and shows
//! [`Fault::detail`] as the text. Fatal means the session is fenced and the
//! only way back is `close` followed by a new `open`.

use crpg_server::host::HostError;

/// One bridge result code. `Ok` is success; every other value is a failure.
#[repr(i64)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeCode {
    /// Success.
    Ok = 0,
    /// A call arrived before `open`.
    NotOpen = 1,
    /// A second `open` on a live or fenced core.
    AlreadyOpen = 2,
    /// A negative number, an oversized index or bad ULID text.
    BadArgument = 3,
    /// `revision` is not the view's current revision.
    StaleOptions = 4,
    /// An intent is still outstanding.
    IntentPending = 5,
    /// `index` is past the offered options.
    NoSuchOption = 6,
    /// Reading the campaign directory failed.
    CampaignIo = 20,
    /// The campaign did not load or did not validate.
    CampaignInvalid = 21,
    /// The campaign holds no encounter.
    NoEncounter = 22,
    /// The requested encounter id is unknown.
    UnknownEncounter = 23,
    /// The requested player side is unknown.
    UnknownSide = 24,
    /// Starting, wrapping or binding the authority failed.
    StartFailed = 25,
    /// The host refused the intent, or the link had no room.
    IntentRefused = 30,
    /// The host was already shut down.
    HostClosed = 50,
    /// Any other host failure, or a rejected NPC intent.
    HostFailed = 51,
    /// A delivered frame did not decode.
    FrameUndecodable = 52,
    /// The replica refused a decoded frame.
    ReplicaRejected = 53,
    /// A transfer entry appeared in a delta hand-off.
    UnexpectedTransfer = 54,
    /// A panic was caught at the core boundary.
    Panicked = 55,
}

impl BridgeCode {
    /// The stable wire number GDScript sees.
    pub fn as_i64(self) -> i64 {
        self as i64
    }

    /// The upper-case ADR name used as the `last_error` prefix.
    pub fn name(self) -> &'static str {
        match self {
            Self::Ok => "OK",
            Self::NotOpen => "NOT_OPEN",
            Self::AlreadyOpen => "ALREADY_OPEN",
            Self::BadArgument => "BAD_ARGUMENT",
            Self::StaleOptions => "STALE_OPTIONS",
            Self::IntentPending => "INTENT_PENDING",
            Self::NoSuchOption => "NO_SUCH_OPTION",
            Self::CampaignIo => "CAMPAIGN_IO",
            Self::CampaignInvalid => "CAMPAIGN_INVALID",
            Self::NoEncounter => "NO_ENCOUNTER",
            Self::UnknownEncounter => "UNKNOWN_ENCOUNTER",
            Self::UnknownSide => "UNKNOWN_SIDE",
            Self::StartFailed => "START_FAILED",
            Self::IntentRefused => "INTENT_REFUSED",
            Self::HostClosed => "HOST_CLOSED",
            Self::HostFailed => "HOST_FAILED",
            Self::FrameUndecodable => "FRAME_UNDECODABLE",
            Self::ReplicaRejected => "REPLICA_REJECTED",
            Self::UnexpectedTransfer => "UNEXPECTED_TRANSFER",
            Self::Panicked => "PANICKED",
        }
    }

    /// Whether this code fences the session (`50..=55`, ADR §11).
    pub fn is_fatal(self) -> bool {
        (50..=55).contains(&self.as_i64())
    }
}

/// A bridge code plus the diagnostic text that follows its name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Fault {
    /// The code returned to GDScript.
    pub(crate) code: BridgeCode,
    /// The source `Display`, a summary, or the panic payload.
    pub(crate) detail: String,
}

/// Maps one trusted host failure to the bridge table.
///
/// `HostError::Closed` is the only code 50; every other host failure is 51
/// (`ADR-0033 §11`). `Display` becomes the diagnostic text.
pub(crate) fn host_fault(error: &HostError) -> Fault {
    let code = match error {
        HostError::Closed => BridgeCode::HostClosed,
        _ => BridgeCode::HostFailed,
    };
    Fault {
        code,
        detail: error.to_string(),
    }
}

#[cfg(test)]
mod tests;

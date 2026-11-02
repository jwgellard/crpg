//! K1-K2: the closed code table and the host-failure mapping.

use super::{host_fault, BridgeCode};
use crpg_server::host::HostError;

/// K1 / B15: every code, name and fatality matches the ADR-0033 §11 table.
///
/// The bug caught is a swapped discriminant or a wrong fatality boundary; the
/// control is `Ok`, which is 0, `"OK"` and not fatal.
#[test]
fn every_code_matches_the_adr_table() {
    let table = [
        (BridgeCode::Ok, 0, "OK", false),
        (BridgeCode::NotOpen, 1, "NOT_OPEN", false),
        (BridgeCode::AlreadyOpen, 2, "ALREADY_OPEN", false),
        (BridgeCode::BadArgument, 3, "BAD_ARGUMENT", false),
        (BridgeCode::StaleOptions, 4, "STALE_OPTIONS", false),
        (BridgeCode::IntentPending, 5, "INTENT_PENDING", false),
        (BridgeCode::NoSuchOption, 6, "NO_SUCH_OPTION", false),
        (BridgeCode::CampaignIo, 20, "CAMPAIGN_IO", false),
        (BridgeCode::CampaignInvalid, 21, "CAMPAIGN_INVALID", false),
        (BridgeCode::NoEncounter, 22, "NO_ENCOUNTER", false),
        (BridgeCode::UnknownEncounter, 23, "UNKNOWN_ENCOUNTER", false),
        (BridgeCode::UnknownSide, 24, "UNKNOWN_SIDE", false),
        (BridgeCode::StartFailed, 25, "START_FAILED", false),
        (BridgeCode::IntentRefused, 30, "INTENT_REFUSED", false),
        (BridgeCode::HostClosed, 50, "HOST_CLOSED", true),
        (BridgeCode::HostFailed, 51, "HOST_FAILED", true),
        (BridgeCode::FrameUndecodable, 52, "FRAME_UNDECODABLE", true),
        (BridgeCode::ReplicaRejected, 53, "REPLICA_REJECTED", true),
        (
            BridgeCode::UnexpectedTransfer,
            54,
            "UNEXPECTED_TRANSFER",
            true,
        ),
        (BridgeCode::Panicked, 55, "PANICKED", true),
    ];
    assert_eq!(table.len(), 20);
    for (code, number, name, fatal) in table {
        assert_eq!(code.as_i64(), number, "code number for {name}");
        assert_eq!(code.name(), name, "name for {number}");
        assert_eq!(code.is_fatal(), fatal, "fatality for {name}");
    }
    assert_eq!(BridgeCode::Ok.as_i64(), 0);
    assert_eq!(BridgeCode::Ok.name(), "OK");
    assert!(!BridgeCode::Ok.is_fatal());
    assert!(!BridgeCode::IntentRefused.is_fatal());
    assert!(BridgeCode::HostClosed.is_fatal());
    assert!(BridgeCode::Panicked.is_fatal());
}

/// K2 / B3: only `Closed` maps to 50; every other host failure maps to 51.
#[test]
fn host_failures_split_on_closed() {
    let closed = host_fault(&HostError::Closed);
    assert_eq!(closed.code, BridgeCode::HostClosed);
    assert_eq!(closed.detail, "Closed at host/admission");

    for error in [
        HostError::TimeRegression,
        HostError::Invariant("x"),
        HostError::UnknownPeer,
    ] {
        let fault = host_fault(&error);
        assert_eq!(fault.code, BridgeCode::HostFailed);
        assert_eq!(fault.detail, error.to_string());
    }
}

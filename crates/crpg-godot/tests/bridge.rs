//! K15 and K30 through the public bridge only: the fixture's first view and
//! the idempotent close. The private `replica_id` mapping is a unit-test
//! concern in `src/core/tests.rs`.

use crpg_godot::core::{BridgeCode, BridgeCore};

fn fixture() -> String {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("campaigns")
        .join("fixtures")
        .join("combat_basic")
        .to_str()
        .expect("fixture path is UTF-8")
        .to_string()
}

/// K15 / BR-1, B11, B12, B13: the first view of the checked-in fixture.
#[test]
fn fixture_opens_and_polls_to_the_expected_view() {
    let mut core = BridgeCore::new();
    assert_eq!(core.open(&fixture(), "", "", 1), BridgeCode::Ok);
    assert_eq!(core.poll(0), BridgeCode::Ok);
    let view = core.view();
    assert_eq!(view.revision(), 1);
    assert_eq!(view.net_ids(), &[1, 2]);
    assert_eq!(view.names(), &["Hero".to_string(), "Goblin".to_string()]);
    assert_eq!(view.sides(), &["Hero".to_string(), "Goblin".to_string()]);
    assert_eq!(
        view.placements(),
        &[
            "0000000000000000000000000E".to_string(),
            "0000000000000000000000000F".to_string(),
        ]
    );
    assert_eq!(view.health_known(), &[1, 1]);
    assert_eq!(view.health().len(), 2);
    assert_eq!(view.max_health().len(), 2);
    // The goblin's initiative 9 sorts before the hero's 10, so the goblin is
    // the opening actor; the default player side is the first combatant's
    // (Hero), which is not active on this poll.
    assert_eq!(view.active(), 2);
    assert!(view.events().is_empty());
    assert!(view.option_labels().is_empty());
    drop(view);

    // Control: naming the encounter and side explicitly gives an equal view.
    let mut named = BridgeCore::new();
    assert_eq!(
        named.open(&fixture(), "0000000000000000000000000N", "Hero", 1),
        BridgeCode::Ok
    );
    assert_eq!(named.poll(0), BridgeCode::Ok);
    assert_eq!(named.view(), core.view());
}

/// K30 / B21: close keeps the last error, resets the view, and the next poll
/// is `NOT_OPEN`.
#[test]
fn close_is_idempotent_through_the_public_api() {
    let mut core = BridgeCore::new();
    assert_eq!(core.open(&fixture(), "", "", 1), BridgeCode::Ok);
    assert_eq!(core.poll(0), BridgeCode::Ok);
    let revision = core.view().revision();
    assert_eq!(core.submit_option(revision, 99), BridgeCode::NoSuchOption);
    assert!(core.last_error().starts_with("NO_SUCH_OPTION"));
    core.close();
    assert!(core.last_error().starts_with("NO_SUCH_OPTION"));
    core.close();
    assert_eq!(core.view().revision(), 0);
    assert_eq!(core.poll(0), BridgeCode::NotOpen);
    assert!(core.last_error().starts_with("NOT_OPEN"));
    assert_eq!(core.open(&fixture(), "", "", 1), BridgeCode::Ok);
}

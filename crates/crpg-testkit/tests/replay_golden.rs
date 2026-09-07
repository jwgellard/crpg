//! Independent native replay gates (spec section 15.4 step 9, ADR-0012).
//!
//! Windows/MSVC owns the primary baseline; Linux/GNU owns the supported
//! server baseline. Both use Rust 1.98.0, the normal test profile and default
//! features. Target selection is compile-time only; other builds never read
//! either golden. Scope changes require reviewed, independently generated
//! native replacements, not tolerance or a permanent rebless command.

#![forbid(unsafe_code)]

mod support;

#[cfg(any(
    all(
        target_os = "windows",
        target_arch = "x86_64",
        target_env = "msvc",
        debug_assertions
    ),
    all(
        target_os = "linux",
        target_arch = "x86_64",
        target_env = "gnu",
        debug_assertions
    )
))]
#[test]
fn native_target_golden_replay() {
    let replay_path = support::fixture_replay_path();
    let hashes = crpg_testkit::play_and_verify(
        &replay_path,
        &support::target_golden_path(),
        support::test_apply(),
    )
    .expect("native target golden replay must match; missing baseline is an error");
    let replay = crpg_testkit::read_replay(&replay_path).expect("replay must parse");
    assert_eq!(hashes.len(), replay.total_ticks as usize);
}

#[cfg(not(any(
    all(
        target_os = "windows",
        target_arch = "x86_64",
        target_env = "msvc",
        debug_assertions
    ),
    all(
        target_os = "linux",
        target_arch = "x86_64",
        target_env = "gnu",
        debug_assertions
    )
)))]
#[test]
fn portable_replay_shape_and_repeatability() {
    let replay =
        crpg_testkit::read_replay(&support::fixture_replay_path()).expect("replay must parse");
    let first =
        crpg_testkit::play_replay(&replay, support::test_apply()).expect("replay must play");
    let second =
        crpg_testkit::play_replay(&replay, support::test_apply()).expect("replay must play");
    assert_eq!(first, second);
    assert_eq!(first.len(), replay.total_ticks as usize);
}

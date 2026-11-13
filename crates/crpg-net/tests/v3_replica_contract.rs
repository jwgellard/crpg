//! T140r2 two-keys suite: the v3 replica (`replica_v3`), written from
//! `tasks/T140r.md` (Behaviour R-1..R-15, Tests R1..R10) and the public API
//! only. The implementer's suite and source were not read.

#[test]
fn r1_construction() {}

#[test]
fn r2_matches_the_v2_replica() {}

#[test]
fn r3_fact_ops_replace_images() {}

#[test]
fn r4_fact_references_live_and_notices_inert() {}

#[test]
fn r5_stale_references_are_tolerated() {}

#[test]
fn r6_clock_and_host_facts() {}

#[test]
fn r7_snapshot_checks_in_order() {}

#[test]
fn r8_snapshot_replaces_everything_but_host_state() {}

#[test]
fn r9_mid_frame_failure_changes_nothing() {}

#[test]
fn r10_pool_to_replica_end_to_end() {}

//! U1-U8: `Sides`, `host_route`, `point_along` and `decide`'s error path.
//!
//! The literal values are tasks/T056.md's
//! "T056a: crates/crpg-ai/tests/mvp_unit.rs". The tests need no encounter.

use crpg_ai::{
    decide, host_route, point_along, RouteError, SideId, Sides, SidesError, AI_ROUTE_TAG,
    STANDOFF_MARGIN_RAW,
};
use crpg_core::{EntityId, Fx16_16};
use crpg_nav::{bake_box, NavMesh, NavRect};
use crpg_sim::{
    state_hash, CombatError, EntityMeta, LegalActionsError, NavRevision, RouteTag, World,
};

/// A whole-metre point `[x, 0, z]`.
fn p(x: i32, z: i32) -> [Fx16_16; 3] {
    [Fx16_16::from_int(x), Fx16_16::ZERO, Fx16_16::from_int(z)]
}

/// A raw point `[x, y, z]`, all in 1/65,536 m.
fn rp(x: i32, y: i32, z: i32) -> [Fx16_16; 3] {
    [
        Fx16_16::from_raw(x),
        Fx16_16::from_raw(y),
        Fx16_16::from_raw(z),
    ]
}

/// A whole-metre rectangle in `x`/`z`.
fn rect(x0: i32, z0: i32, x1: i32, z1: i32) -> NavRect {
    NavRect {
        min_x: Fx16_16::from_int(x0),
        min_z: Fx16_16::from_int(z0),
        max_x: Fx16_16::from_int(x1),
        max_z: Fx16_16::from_int(z1),
    }
}

/// `M`: one cell, so every path on it is a straight two-point line.
fn mesh() -> NavMesh {
    bake_box(Fx16_16::ZERO, rect(-5, -5, 25, 25), &[]).expect("bakes")
}

fn rev(mesh: &NavMesh) -> NavRevision {
    NavRevision::of_bytes(&mesh.to_canonical_bytes())
}

/// Three plain entities spawned in order, so `e0 < e1 < e2`.
fn entities(world: &mut World) -> (EntityId, EntityId, EntityId) {
    (
        world.spawn(EntityMeta {}),
        world.spawn(EntityMeta {}),
        world.spawn(EntityMeta {}),
    )
}

// ---------------------------------------------------------------------------
// U1-U3: sides.
// ---------------------------------------------------------------------------

#[test]
fn u1_sides_sort_and_read() {
    let mut world = World::new(7);
    let (e0, e1, e2) = entities(&mut world);

    let sides = Sides::new(vec![(e2, SideId(1)), (e0, SideId(2))]).expect("unique entities");
    assert_eq!(sides.entries(), &[(e0, SideId(2)), (e2, SideId(1))]);
    assert_eq!(sides.side(e0), Some(SideId(2)));
    assert_eq!(sides.side(e1), None);

    // Control: the same pairs given in the other order read identically.
    let flipped = Sides::new(vec![(e0, SideId(2)), (e2, SideId(1))]).expect("unique entities");
    assert_eq!(flipped, sides);

    // Control: listing `e1` flips its lookup from `None` to `Some`.
    let with_e1 = Sides::new(vec![(e2, SideId(1)), (e0, SideId(2)), (e1, SideId(3))])
        .expect("unique entities");
    assert_eq!(with_e1.side(e1), Some(SideId(3)));
    assert_eq!(
        with_e1.entries(),
        &[(e0, SideId(2)), (e1, SideId(3)), (e2, SideId(1))]
    );
    assert_eq!(Sides::default().entries(), &[]);
}

#[test]
fn u2_sides_duplicate() {
    let mut world = World::new(7);
    let (e0, e1, e2) = entities(&mut world);

    let error = Sides::new(vec![(e1, SideId(1)), (e0, SideId(1)), (e1, SideId(2))])
        .expect_err("e1 is listed twice");
    assert_eq!(error, SidesError::Duplicate { entity: e1 });
    assert_eq!(format!("{error}"), format!("Duplicate at sides/{e1:?}"));

    // Control: without the repeated `e1` the same input is accepted.
    assert!(Sides::new(vec![(e1, SideId(1)), (e0, SideId(1))]).is_ok());

    // Control: the least duplicate is reported, not the first one given.
    let least = Sides::new(vec![
        (e2, SideId(1)),
        (e2, SideId(1)),
        (e0, SideId(3)),
        (e0, SideId(3)),
    ])
    .expect_err("e0 and e2 are listed twice");
    assert_eq!(least, SidesError::Duplicate { entity: e0 });
}

#[test]
fn u3_hostile_cases() {
    let mut world = World::new(7);
    let (e0, _e1, e2) = entities(&mut world);
    let e1 = world.spawn(EntityMeta {});
    let e3 = world.spawn(EntityMeta {});
    let sides =
        Sides::new(vec![(e0, SideId(1)), (e1, SideId(2)), (e2, SideId(1))]).expect("unique");

    assert!(sides.hostile(e0, e1));
    assert!(sides.hostile(e1, e0));
    assert!(!sides.hostile(e0, e2));
    assert!(!sides.hostile(e0, e0));
    assert!(!sides.hostile(e0, e3));
    assert!(!sides.hostile(e3, e0));

    // Control: listing `e3` on another side flips both of its answers.
    let with_e3 = Sides::new(vec![
        (e0, SideId(1)),
        (e1, SideId(2)),
        (e2, SideId(1)),
        (e3, SideId(2)),
    ])
    .expect("unique");
    assert!(with_e3.hostile(e0, e3));
    assert!(with_e3.hostile(e3, e0));
    assert!(!with_e3.hostile(e1, e3));
}

// ---------------------------------------------------------------------------
// U4: host_route's refusals, in order.
// ---------------------------------------------------------------------------

#[test]
fn u4_host_route_refusals_in_order() {
    let mut world = World::new(7);
    let (e0, _e1, _e2) = entities(&mut world);
    let absent = world.spawn(EntityMeta {});
    assert!(world.despawn(absent));
    let mesh = mesh();

    // Unbound wins over absent.
    assert_eq!(
        host_route(&mesh, &world, absent, p(1, 1)),
        Err(RouteError::NavUnbound)
    );

    assert_eq!(
        format!("{}", RouteError::NavUnbound),
        "NavUnbound at ai/route"
    );
    assert_eq!(
        format!("{}", RouteError::AbsentEntity { entity: absent }),
        "AbsentEntity at ai/route"
    );
    assert_eq!(
        format!("{}", RouteError::NotMover { entity: e0 }),
        "NotMover at ai/route"
    );
    assert_eq!(
        format!("{}", RouteError::Nav(crpg_nav::NavError::Unreachable)),
        "Nav at ai/route"
    );

    assert!(world.bind_nav(rev(&mesh)).is_empty());

    // Bound: the next failure is the absent actor, then the missing mover.
    assert_eq!(
        host_route(&mesh, &world, absent, p(1, 1)),
        Err(RouteError::AbsentEntity { entity: absent })
    );
    assert_eq!(
        host_route(&mesh, &world, e0, p(1, 1)),
        Err(RouteError::NotMover { entity: e0 })
    );
}

// ---------------------------------------------------------------------------
// U5-U6: point_along.
// ---------------------------------------------------------------------------

#[test]
fn u5_point_along_walks_segments() {
    // 4 m along `x`, then 3 m along `z`.
    let points = [rp(0, 0, 0), rp(262_144, 0, 0), rp(262_144, 0, 196_608)];

    assert_eq!(point_along(&points, 0), Some(rp(0, 0, 0)));
    assert_eq!(point_along(&points, 131_072), Some(rp(131_072, 0, 0)));
    assert_eq!(point_along(&points, 262_144), Some(rp(262_144, 0, 0)));
    assert_eq!(point_along(&points, 360_448), Some(rp(262_144, 0, 98_304)));
    assert_eq!(point_along(&points, 458_752), Some(rp(262_144, 0, 196_608)));
    assert_eq!(
        point_along(&points, 1_000_000_000),
        Some(rp(262_144, 0, 196_608))
    );
    assert_eq!(point_along(&[], 0), None);
    assert_eq!(point_along(&[rp(5, 6, 7)], 0), Some(rp(5, 6, 7)));
    assert_eq!(point_along(&[rp(5, 6, 7)], 1_000_000), Some(rp(5, 6, 7)));

    // Control: one raw short of the corner stays on the first segment.
    assert_eq!(point_along(&points, 262_143), Some(rp(262_143, 0, 0)));
}

#[test]
fn u6_point_along_truncates_toward_the_start() {
    // A 3-4-5 line of length 327,680 raw.
    let points = [rp(0, 0, 0), rp(196_608, 0, 262_144)];
    assert_eq!(point_along(&points, 100_000), Some(rp(60_000, 0, 80_000)));
    assert_eq!(point_along(&points, 3), Some(rp(1, 0, 2)));

    // The reversed direction truncates toward zero, not away from it.
    let reversed = [rp(0, 0, 0), rp(-196_608, 0, -262_144)];
    assert_eq!(point_along(&reversed, 3), Some(rp(-1, 0, -2)));

    // Controls: one raw more crosses into the next unit (`2`, `-2`), and the
    // same distance on a longer line stays at the previous unit.
    assert_eq!(point_along(&points, 6), Some(rp(3, 0, 4)));
    assert_eq!(point_along(&reversed, 6), Some(rp(-3, 0, -4)));
}

// ---------------------------------------------------------------------------
// U7-U8: decide's error path and the constants.
// ---------------------------------------------------------------------------

#[test]
fn u7_decide_without_encounter() {
    let mut world = World::new(7);
    let (e0, _e1, _e2) = entities(&mut world);

    let before = state_hash(&world);
    let decision = decide(&world, e0, &Sides::default(), None);
    assert_eq!(
        decision,
        Err(LegalActionsError::Actor(CombatError::NoEncounter))
    );
    assert_eq!(state_hash(&world), before);

    // Control: the same actor in a world with a live entity but no encounter
    // is still `NoEncounter`, and another actor reads the same.
    let (other, _, _) = entities(&mut world);
    assert_eq!(
        decide(&world, other, &Sides::default(), None),
        Err(LegalActionsError::Actor(CombatError::NoEncounter))
    );
}

#[test]
fn u8_constants() {
    assert_eq!(AI_ROUTE_TAG, RouteTag(0));
    assert_eq!(STANDOFF_MARGIN_RAW, 64);

    // Control: a tag that differs is not the constant.
    assert_ne!(AI_ROUTE_TAG, RouteTag(1));
    assert_eq!(STANDOFF_MARGIN_RAW * 2, 128);
}

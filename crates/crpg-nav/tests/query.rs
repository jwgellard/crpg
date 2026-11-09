//! T035b implementer suite: the box bake, `locate` and `nearest_on_mesh`.

use crpg_core::DeterministicRng;
use crpg_nav::{
    bake_box, BakeError, Fx16_16, NavMesh, NavMeshError, NavRect, PolyId, COORD_LIMIT_RAW,
    MAX_BAKE_HOLES,
};

fn m(v: i32) -> Fx16_16 {
    Fx16_16::from_int(v)
}

fn p(x: i32, z: i32) -> [Fx16_16; 3] {
    [m(x), m(0), m(z)]
}

fn raw(x: i32, y: i32, z: i32) -> [Fx16_16; 3] {
    [x, y, z].map(Fx16_16::from_raw)
}

fn rect(x0: i32, z0: i32, x1: i32, z1: i32) -> NavRect {
    NavRect {
        min_x: m(x0),
        min_z: m(z0),
        max_x: m(x1),
        max_z: m(z1),
    }
}

/// A rectangle from raw `min_x, min_z, max_x, max_z`.
fn rect_raw(v: [i32; 4]) -> NavRect {
    let [min_x, min_z, max_x, max_z] = v.map(Fx16_16::from_raw);
    NavRect {
        min_x,
        min_z,
        max_x,
        max_z,
    }
}

fn xz_mesh(xz: &[(i32, i32)], polygons: Vec<Vec<u32>>) -> NavMesh {
    NavMesh::new(xz.iter().map(|&(x, z)| p(x, z)).collect(), polygons).expect("mesh builds")
}

fn mesh_w() -> NavMesh {
    bake_box(m(0), rect(0, 0, 10, 10), &[rect(4, 0, 6, 8)]).expect("W bakes")
}

fn mesh_n() -> NavMesh {
    xz_mesh(
        &[(0, 0), (4, 0), (4, 2), (0, 2), (8, 4), (6, 6), (9, 8)],
        vec![vec![0, 1, 2, 3], vec![1, 4, 5, 2], vec![5, 4, 6]],
    )
}

fn comb(k: i32) -> NavMesh {
    let holes: Vec<NavRect> = (0..k)
        .map(|i| {
            let x0 = 2 + 6 * i;
            if i % 2 == 0 {
                rect(x0, 0, x0 + 2, 16)
            } else {
                rect(x0, 4, x0 + 2, 20)
            }
        })
        .collect();
    bake_box(m(0), rect(0, 0, 200, 20), &holes).expect("comb bakes")
}

fn ids(list: &[Option<u32>]) -> Vec<Option<PolyId>> {
    list.iter().map(|o| o.map(PolyId)).collect()
}

fn neighbours_of(mesh: &NavMesh) -> Vec<Vec<Option<PolyId>>> {
    (0..mesh.polygon_count() as u32)
        .map(|i| mesh.neighbours(PolyId(i)).expect("in range").to_vec())
        .collect()
}

fn polygons_of(mesh: &NavMesh) -> Vec<Vec<u32>> {
    (0..mesh.polygon_count() as u32)
        .map(|i| mesh.polygon(PolyId(i)).expect("in range").to_vec())
        .collect()
}

#[test]
fn b1_bake_literals() {
    let w = mesh_w();
    let xz = [
        (0, 0),
        (4, 0),
        (4, 8),
        (0, 8),
        (4, 10),
        (0, 10),
        (6, 8),
        (6, 10),
        (6, 0),
        (10, 0),
        (10, 8),
        (10, 10),
    ];
    let expected: Vec<[Fx16_16; 3]> = xz.iter().map(|&(x, z)| p(x, z)).collect();
    assert_eq!(w.vertices(), expected.as_slice());
    assert_eq!(
        polygons_of(&w),
        vec![
            vec![0, 1, 2, 3],
            vec![3, 2, 4, 5],
            vec![2, 6, 7, 4],
            vec![8, 9, 10, 6],
            vec![6, 10, 11, 7]
        ]
    );
    assert_eq!(
        neighbours_of(&w),
        vec![
            ids(&[None, None, Some(1), None]),
            ids(&[Some(0), Some(2), None, None]),
            ids(&[None, Some(4), None, Some(1)]),
            ids(&[None, None, Some(4), None]),
            ids(&[Some(3), None, None, Some(2)]),
        ]
    );
    assert_eq!(w.to_canonical_bytes().len(), 242);
    // Baking twice gives an equal mesh and equal bytes.
    let again = mesh_w();
    assert_eq!(w, again);
    assert_eq!(w.to_canonical_bytes(), again.to_canonical_bytes());

    // R, the ring, equals its literal.
    let ring = bake_box(m(0), rect(0, 0, 10, 10), &[rect(4, 4, 6, 6)]).expect("R bakes");
    let literal = xz_mesh(
        &[
            (0, 0),
            (4, 0),
            (4, 4),
            (0, 4),
            (4, 6),
            (0, 6),
            (4, 10),
            (0, 10),
            (6, 0),
            (6, 4),
            (6, 6),
            (6, 10),
            (10, 0),
            (10, 4),
            (10, 6),
            (10, 10),
        ],
        vec![
            vec![0, 1, 2, 3],
            vec![3, 2, 4, 5],
            vec![5, 4, 6, 7],
            vec![1, 8, 9, 2],
            vec![4, 10, 11, 6],
            vec![8, 12, 13, 9],
            vec![9, 13, 14, 10],
            vec![10, 14, 15, 11],
        ],
    );
    assert_eq!(ring, literal);
    assert_eq!(ring.polygon_count(), 8);
    assert_eq!(ring.vertices().len(), 16);

    // No holes: one polygon, every vertex at the bake height.
    let flat = bake_box(m(2), rect(0, 0, 10, 10), &[]).expect("flat bakes");
    assert_eq!(polygons_of(&flat), vec![vec![0, 1, 2, 3]]);
    let corners = [(0, 0), (10, 0), (10, 10), (0, 10)];
    let expected: Vec<[Fx16_16; 3]> = corners.iter().map(|&(x, z)| [m(x), m(2), m(z)]).collect();
    assert_eq!(flat.vertices(), expected.as_slice());
    assert_eq!(flat.polygon_y(PolyId(0)), Some(m(2)));

    // Overlapping holes.
    let overlap = bake_box(
        m(0),
        rect(0, 0, 10, 10),
        &[rect(2, 2, 6, 6), rect(4, 4, 8, 8)],
    )
    .expect("overlap bakes");
    assert_eq!(overlap.polygon_count(), 18);
    assert_eq!(overlap.vertices().len(), 34);

    // The comb with the most holes.
    let c32 = comb(MAX_BAKE_HOLES as i32);
    assert_eq!(c32.polygon_count(), 131);
    assert_eq!(c32.vertices().len(), 264);
}

#[test]
fn b1_bake_touching_and_covering_holes() {
    // A hole on the outer edge is allowed and leaves no cell outside it.
    let edge = bake_box(m(0), rect(0, 0, 10, 10), &[rect(0, 0, 10, 4)]).expect("edge hole");
    assert_eq!(edge.polygon_count(), 1);
    assert_eq!(edge.locate(p(5, 5)), Some(PolyId(0)));
    assert_eq!(edge.locate(p(5, 3)), None);
    // Two holes that together cover the box leave nothing, though neither does alone.
    assert_eq!(
        bake_box(
            m(0),
            rect(0, 0, 10, 10),
            &[rect(0, 0, 5, 10), rect(5, 0, 10, 10)]
        )
        .unwrap_err(),
        BakeError::NoWalkableArea
    );
}

#[test]
fn b2_bake_errors() {
    let outer = rect(0, 0, 10, 10);
    let lo = COORD_LIMIT_RAW;
    let many = vec![rect(1, 1, 2, 2); 33];
    assert_eq!(
        bake_box(m(0), outer, &many).unwrap_err(),
        BakeError::TooManyHoles { count: 33 }
    );
    // Too many holes wins over out-of-bounds, empty and outside values.
    let bad = vec![rect_raw([lo + 1, 0, lo + 2, 1]); 33];
    assert_eq!(
        bake_box(m(0), rect(0, 0, 0, 0), &bad).unwrap_err(),
        BakeError::TooManyHoles { count: 33 }
    );
    // Exactly the limit of holes is accepted by this check.
    assert!(bake_box(m(0), outer, &[rect(1, 1, 2, 2); 32]).is_ok());

    // Out of bounds: the height, then each outer and hole value.
    let y_out = Fx16_16::from_raw(lo + 1);
    assert_eq!(
        bake_box(y_out, outer, &[]).unwrap_err(),
        BakeError::OutOfBounds
    );
    assert_eq!(
        bake_box(Fx16_16::from_raw(i32::MIN), outer, &[]).unwrap_err(),
        BakeError::OutOfBounds
    );
    for field in 0..4 {
        let mut v = [0, 0, 10, 10];
        v[field] = if field < 2 { -lo - 1 } else { lo + 1 };
        assert_eq!(
            bake_box(m(0), rect_raw(v), &[]).unwrap_err(),
            BakeError::OutOfBounds,
            "outer field {field}"
        );
        let mut h = [1, 1, 2, 2];
        h[field] = if field < 2 { -lo - 1 } else { lo + 1 };
        assert_eq!(
            bake_box(m(0), outer, &[rect(1, 1, 2, 2), rect_raw(h)]).unwrap_err(),
            BakeError::OutOfBounds,
            "hole field {field}"
        );
    }
    // Out of bounds is checked before the empty and outside checks.
    assert_eq!(
        bake_box(m(0), rect_raw([lo + 1, 0, lo + 1, 10]), &[]).unwrap_err(),
        BakeError::OutOfBounds
    );
    assert_eq!(
        bake_box(
            m(0),
            outer,
            &[rect(9, 0, 9, 1), rect_raw([0, 0, 0, lo + 1])]
        )
        .unwrap_err(),
        BakeError::OutOfBounds
    );
    // The limit itself is allowed, on every value.
    let full = rect_raw([-lo, -lo, lo, lo]);
    assert!(bake_box(Fx16_16::from_raw(lo), full, &[]).is_ok());
    assert!(bake_box(Fx16_16::from_raw(-lo), full, &[]).is_ok());

    // Empty outer: equal or reversed, on either axis.
    for r in [
        rect(0, 0, 0, 10),
        rect(0, 0, 10, 0),
        rect(10, 0, 0, 10),
        rect(0, 10, 10, 0),
    ] {
        assert_eq!(bake_box(m(0), r, &[]).unwrap_err(), BakeError::EmptyOuter);
    }
    // Empty outer comes before any hole check.
    assert_eq!(
        bake_box(m(0), rect(0, 0, 0, 10), &[rect(9, 0, 9, 1)]).unwrap_err(),
        BakeError::EmptyOuter
    );

    // Empty hole: equal or reversed, either axis, and the first hole is named.
    assert_eq!(
        bake_box(m(0), outer, &[rect(9, 0, 9, 1)]).unwrap_err(),
        BakeError::EmptyHole { hole: 0 }
    );
    assert_eq!(
        bake_box(m(0), outer, &[rect(1, 1, 2, 2), rect(3, 3, 5, 3)]).unwrap_err(),
        BakeError::EmptyHole { hole: 1 }
    );
    assert_eq!(
        bake_box(m(0), outer, &[rect(5, 1, 3, 2)]).unwrap_err(),
        BakeError::EmptyHole { hole: 0 }
    );
    // Per hole, empty and outside are checked in hole order: an outside hole 0
    // is reported before an empty hole 1, and an empty hole 0 that is also
    // outside is reported as empty.
    assert_eq!(
        bake_box(m(0), outer, &[rect(9, 0, 11, 1), rect(3, 3, 3, 4)]).unwrap_err(),
        BakeError::HoleOutside { hole: 0 }
    );
    assert_eq!(
        bake_box(m(0), outer, &[rect(11, 0, 11, 1)]).unwrap_err(),
        BakeError::EmptyHole { hole: 0 }
    );

    // Hole outside, on each of the four sides.
    for (i, h) in [
        rect(9, 0, 11, 1),
        rect(-1, 0, 1, 1),
        rect(0, 9, 1, 11),
        rect(0, -1, 1, 1),
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(
            bake_box(m(0), outer, &[rect(1, 1, 2, 2), h]).unwrap_err(),
            BakeError::HoleOutside { hole: 1 },
            "side {i}"
        );
    }
    assert_eq!(
        bake_box(m(0), outer, &[rect(9, 0, 11, 1)]).unwrap_err(),
        BakeError::HoleOutside { hole: 0 }
    );

    // Holes covering the box.
    assert_eq!(
        bake_box(m(0), outer, &[outer]).unwrap_err(),
        BakeError::NoWalkableArea
    );
    // Display texts.
    assert_eq!(
        BakeError::TooManyHoles { count: 33 }.to_string(),
        "bake has 33 holes, more than 32"
    );
    assert_eq!(
        BakeError::OutOfBounds.to_string(),
        "bake coordinate is outside the coordinate limit"
    );
    assert_eq!(
        BakeError::EmptyOuter.to_string(),
        "bake outer rectangle is empty"
    );
    assert_eq!(
        BakeError::EmptyHole { hole: 3 }.to_string(),
        "bake hole 3 is empty"
    );
    assert_eq!(
        BakeError::HoleOutside { hole: 4 }.to_string(),
        "bake hole 4 is not inside the outer rectangle"
    );
    assert_eq!(
        BakeError::NoWalkableArea.to_string(),
        "bake holes cover the whole outer rectangle"
    );
    assert_eq!(
        BakeError::Mesh(NavMeshError::NotConvex { poly: 2 }).to_string(),
        "navmesh polygon 2 is not convex and counter-clockwise"
    );
}

#[test]
fn b3_locate_literals() {
    let w = mesh_w();
    let cases = [
        (p(2, 2), Some(0)),
        (p(4, 8), Some(0)),
        (p(4, 4), Some(0)),
        (p(5, 9), Some(2)),
        (p(5, 8), Some(2)),
        (p(10, 10), Some(4)),
        (p(5, 4), None),
        (p(11, 5), None),
    ];
    for (q, want) in cases {
        assert_eq!(w.locate(q), want.map(PolyId), "W at {q:?}");
    }
    let n = mesh_n();
    let cases = [
        (p(4, 1), Some(0)),
        (p(4, 2), Some(0)),
        (p(6, 6), Some(1)),
        (p(10, 10), None),
    ];
    for (q, want) in cases {
        assert_eq!(n.locate(q), want.map(PolyId), "N at {q:?}");
    }
    // `y` never matters.
    assert_eq!(w.locate([m(2), m(99), m(2)]), Some(PolyId(0)));
    assert_eq!(w.locate([m(2), m(-99), m(2)]), Some(PolyId(0)));
    // Any raw input is handled without panic.
    assert_eq!(w.locate(raw(i32::MIN, 0, i32::MAX)), None);
    assert_eq!(w.locate(raw(i32::MAX, 0, i32::MAX)), None);
    assert_eq!(w.locate(raw(i32::MIN, i32::MIN, i32::MIN)), None);
    assert_eq!(n.locate(raw(i32::MAX, 0, i32::MIN)), None);
}

#[test]
fn b3_locate_returns_the_lowest_overlapping_polygon() {
    let big_then_small = xz_mesh(
        &[(0, 0), (10, 0), (0, 10), (1, 1), (3, 1), (1, 3)],
        vec![vec![0, 1, 2], vec![3, 4, 5]],
    );
    assert_eq!(big_then_small.locate(p(2, 2)), Some(PolyId(0)));
    let small_then_big = xz_mesh(
        &[(1, 1), (3, 1), (1, 3), (0, 0), (10, 0), (0, 10)],
        vec![vec![0, 1, 2], vec![3, 4, 5]],
    );
    assert_eq!(small_then_big.locate(p(2, 2)), Some(PolyId(0)));
    assert_eq!(small_then_big.locate(p(5, 2)), Some(PolyId(1)));
}

#[test]
fn b4_nearest_literals() {
    let w = mesh_w();
    let q = |x, z| [m(x), m(3), m(z)];
    // On the mesh, the input is kept and `y` becomes the polygon's.
    assert_eq!(w.nearest_on_mesh(q(7, 9)), p(7, 9));
    assert_eq!(w.nearest_on_mesh(q(5, 8)), p(5, 8));
    // Off the mesh. (5,4) ties with (6,4) in P3; the lowest polygon wins.
    assert_eq!(w.nearest_on_mesh(q(5, 4)), p(4, 4));
    assert_eq!(w.nearest_on_mesh(q(5, -3)), p(4, 0));
    assert_eq!(w.nearest_on_mesh(q(12, 12)), p(10, 10));
    assert_eq!(w.nearest_on_mesh(q(-2, 5)), p(0, 5));

    let n = mesh_n();
    assert_eq!(
        n.nearest_on_mesh(p(10, 10)),
        raw(589_824, 0, 524_288),
        "(9,8) is a vertex"
    );
    assert_eq!(n.nearest_on_mesh(p(-3, 1)), raw(0, 0, 65_536));
    assert_eq!(n.nearest_on_mesh(p(5, -1)), raw(262_144, 0, 0));
    assert_eq!(n.nearest_on_mesh(p(9, 4)), raw(528_143, 0, 277_564));
    assert_eq!(n.nearest_on_mesh(p(2, 5)), raw(131_072, 0, 131_072));
    // On the mesh in N keeps the exact input.
    assert_eq!(n.nearest_on_mesh(raw(1, 77, 3)), raw(1, 0, 3));
}

#[test]
fn b4_nearest_takes_the_polygon_height() {
    let high = bake_box(m(5), rect(0, 0, 10, 10), &[rect(4, 4, 6, 6)]).expect("bakes");
    assert_eq!(high.nearest_on_mesh([m(1), m(0), m(1)]), [m(1), m(5), m(1)]);
    assert_eq!(
        high.nearest_on_mesh([m(5), m(0), m(5)]),
        [m(4), m(5), m(5)],
        "the hole centre is 1 m from (4,5) in P1 and from (5,4) in P3; P1 wins"
    );
    // Two floors at different heights, disjoint in x: y follows the nearest.
    let two = NavMesh::new(
        vec![
            raw(0, 100, 0),
            raw(65_536, 100, 0),
            raw(0, 100, 65_536),
            raw(262_144, 200, 0),
            raw(327_680, 200, 0),
            raw(262_144, 200, 65_536),
        ],
        vec![vec![0, 1, 2], vec![3, 4, 5]],
    )
    .expect("builds");
    assert_eq!(two.nearest_on_mesh(raw(10, 0, 0)), raw(10, 100, 0));
    assert_eq!(
        two.nearest_on_mesh(raw(262_200, 0, 0)),
        raw(262_200, 200, 0)
    );
    assert_eq!(
        two.nearest_on_mesh(raw(400_000, 0, 0)),
        raw(327_680, 200, 0),
        "off the mesh, y is the nearest polygon's"
    );
}

/// B-Q5 branches on small hand-checked meshes.
#[test]
fn b5_edge_candidate_branches() {
    let square = xz_mesh(&[(0, 0), (4, 0), (4, 4), (0, 4)], vec![vec![0, 1, 2, 3]]);
    // `t <= 0` picks the edge start, `t >= l2` its end: both reach the corner.
    assert_eq!(square.nearest_on_mesh(p(-3, -3)), p(0, 0));
    assert_eq!(square.nearest_on_mesh(p(7, 7)), p(4, 4));
    assert_eq!(square.nearest_on_mesh(p(7, -3)), p(4, 0));
    assert_eq!(square.nearest_on_mesh(p(-3, 7)), p(0, 4));
    // A projection strictly inside an axis-aligned edge needs no rounding.
    assert_eq!(
        square.nearest_on_mesh(raw(100_000, 0, -50_000)),
        raw(100_000, 0, 0)
    );

    // The lattice rule on a slanted hypotenuse: x + z = 4 style edge
    // (7,0) -> (0,5) with the foot of (10,10) between lattice points. The four
    // candidates are (4,1) (4,2) (5,1) (5,2); (5,2) is outside the triangle and
    // (4,2) is the nearest of the contained ones.
    let tri = NavMesh::new(
        vec![raw(0, 0, 0), raw(7, 0, 0), raw(0, 0, 5)],
        vec![vec![0, 1, 2]],
    )
    .expect("builds");
    assert_eq!(tri.nearest_on_mesh(raw(10, 0, 10)), raw(4, 0, 2));
}

/// B-Q5 rounding cases found by search against an independent integer model;
/// each is decided by exactly one rule (ceiling, floor of a negative product,
/// candidate order and strict-less tie-break, and the `d2(a) <= d2(b)` tie).
#[test]
fn b5_rounding_rules() {
    // (triangle xz, query xz, expected xz), all raw, y = 0.
    type Case = ([(i32, i32); 3], (i32, i32), (i32, i32));
    let cases: [Case; 5] = [
        // The ceiling candidate wins; a floor-only rule gives (-7, 7).
        ([(-8, 4), (-2, 5), (-8, 8)], (-6, 8), (-6, 7)),
        // Floor of a negative product; truncation gives (-1, 4).
        ([(-7, 7), (5, -3), (-1, 4)], (3, 15), (-3, 5)),
        // Floor of a negative `z` product; truncation gives (3, 6).
        ([(-1, 4), (8, 3), (3, 6)], (-3, 15), (1, 5)),
        // Candidate (5,1) beats the equally near (6,0): fixed order, first wins.
        ([(-1, -1), (7, -1), (2, 7)], (6, 1), (5, 1)),
        // Nothing contained on the long edges; the end with the least distance.
        ([(1, -3), (-8, 8), (5, -8)], (-20, -11), (1, -3)),
    ];
    for (tri, (qx, qz), (wx, wz)) in cases {
        let mesh = NavMesh::new(
            tri.iter().map(|&(x, z)| raw(x, 0, z)).collect(),
            vec![vec![0, 1, 2]],
        )
        .expect("builds");
        assert_eq!(
            mesh.nearest_on_mesh(raw(qx, 0, qz)),
            raw(wx, 0, wz),
            "{tri:?} {qx} {qz}"
        );
    }
}

/// The none-contained fallback of B-Q5 on a one-unit-thin sliver.
#[test]
fn b5_fallback_to_edge_ends() {
    // a=(0,0) b=(100,1) c=(99,1), raw, at raw height 7: no lattice point near
    // the long edges lies inside, so the edge end is taken.
    let sliver = NavMesh::new(
        vec![raw(0, 7, 0), raw(100, 7, 1), raw(99, 7, 1)],
        vec![vec![0, 1, 2]],
    )
    .expect("builds");
    // Edge a->b: foot of (50,-5) rounds to (49|50, 0|1), none inside. The
    // nearest end over all edges is c = (99,1), by distance 2437 against 2525
    // for a and 2536 for b.
    assert_eq!(sliver.nearest_on_mesh(raw(50, 0, -5)), raw(99, 7, 1));
    // Past the tip: b-side.
    assert_eq!(sliver.nearest_on_mesh(raw(200, 0, 0)), raw(100, 7, 1));
    // Before the origin: a-side.
    assert_eq!(sliver.nearest_on_mesh(raw(-30, 0, -4)), raw(0, 7, 0));
}

#[test]
fn b5_nearest_is_on_mesh() {
    let mut rng = DeterministicRng::from_seed(5);
    let stream = rng.stream("nav");
    let mut built = 0;
    while built < 20_000 {
        let mut v = [[0i32; 2]; 3];
        for c in &mut v {
            *c = [stream.gen_range_i32(-50, 50), stream.gen_range_i32(-50, 50)];
        }
        let [[ax, az], [bx, bz], [cx, cz]] = v.map(|c| c.map(i64::from));
        let cross = (bx - ax) * (cz - az) - (bz - az) * (cx - ax);
        if cross == 0 {
            continue;
        }
        if cross < 0 {
            v.swap(1, 2);
        }
        let mesh = NavMesh::new(
            v.iter().map(|&[x, z]| raw(x, 0, z)).collect(),
            vec![vec![0, 1, 2]],
        )
        .expect("triangle builds");
        let q = raw(
            stream.gen_range_i32(-200, 200),
            0,
            stream.gen_range_i32(-200, 200),
        );
        let near = mesh.nearest_on_mesh(q);
        assert!(mesh.locate(near).is_some(), "{v:?} {q:?} -> {near:?}");
        if mesh.locate(q).is_some() {
            assert_eq!(near, q);
        }
        built += 1;
    }
    let n = mesh_n();
    for _ in 0..2_000 {
        let q = raw(
            stream.gen_range_i32(i32::MIN, i32::MAX),
            stream.gen_range_i32(i32::MIN, i32::MAX),
            stream.gen_range_i32(i32::MIN, i32::MAX),
        );
        assert!(n.locate(n.nearest_on_mesh(q)).is_some(), "{q:?}");
    }
}

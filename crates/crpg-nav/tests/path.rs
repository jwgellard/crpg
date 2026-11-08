//! T035c implementer suite: `find_path` (A* over polygons, then the funnel).

use crpg_core::DeterministicRng;
use crpg_nav::{
    bake_box, seg_len, BakeError, Fx16_16, NavError, NavMesh, NavPath, NavRect,
    MAX_PATH_LENGTH_RAW, MAX_PATH_POINTS,
};

type P3 = [Fx16_16; 3];

fn m(v: i32) -> Fx16_16 {
    Fx16_16::from_int(v)
}

fn p(x: i32, z: i32) -> P3 {
    [m(x), m(0), m(z)]
}

fn rect(x0: i32, z0: i32, x1: i32, z1: i32) -> NavRect {
    NavRect {
        min_x: m(x0),
        min_z: m(z0),
        max_x: m(x1),
        max_z: m(z1),
    }
}

fn xz_mesh(xz: &[(i32, i32)], polygons: Vec<Vec<u32>>) -> NavMesh {
    NavMesh::new(xz.iter().map(|&(x, z)| p(x, z)).collect(), polygons).expect("mesh builds")
}

fn mesh_w() -> NavMesh {
    bake_box(m(0), rect(0, 0, 10, 10), &[rect(4, 0, 6, 8)]).expect("W bakes")
}

fn mesh_r() -> NavMesh {
    bake_box(m(0), rect(0, 0, 10, 10), &[rect(4, 4, 6, 6)]).expect("R bakes")
}

fn mesh_n() -> NavMesh {
    xz_mesh(
        &[(0, 0), (4, 0), (4, 2), (0, 2), (8, 4), (6, 6), (9, 8)],
        vec![vec![0, 1, 2, 3], vec![1, 4, 5, 2], vec![5, 4, 6]],
    )
}

fn mesh_u() -> NavMesh {
    bake_box(m(0), rect(0, 0, 10, 10), &[rect(4, 0, 6, 10)]).expect("U bakes")
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

fn mesh_l() -> NavMesh {
    bake_box(m(0), rect(0, 0, 5000, 10), &[]).expect("L bakes")
}

/// `find_path` run twice on the same mesh; the two results must be equal (C-9).
fn go(mesh: &NavMesh, from: P3, to: P3) -> Result<NavPath, NavError> {
    let first = mesh.find_path(from, to);
    assert_eq!(first, mesh.find_path(from, to), "{from:?} -> {to:?}");
    first
}

fn xz_points(path: &NavPath) -> Vec<(i32, i32)> {
    path.points()
        .iter()
        .map(|v| (v[0].to_raw() >> 16, v[2].to_raw() >> 16))
        .collect()
}

fn seg_sum(points: &[P3]) -> u64 {
    points.windows(2).map(|w| seg_len(w[0], w[1])).sum()
}

#[test]
fn c1_wall_literal() {
    let path = go(&mesh_w(), p(2, 2), p(8, 2)).expect("path");
    assert_eq!(xz_points(&path), vec![(2, 2), (4, 8), (6, 8), (8, 2)]);
    assert_eq!(path.length(), 960_046);
    assert_eq!(path.points().len(), 4);
    assert_eq!(path.clone().into_points(), path.points().to_vec());
}

#[test]
fn c2_ring_tie_break() {
    let path = go(&mesh_r(), p(5, 1), p(5, 9)).expect("path");
    assert_eq!(xz_points(&path), vec![(5, 1), (4, 4), (4, 6), (5, 9)]);
    assert_eq!(path.length(), 545_560);
    // The mirror query goes the same (left) way round, by the same pinned order.
    let back = go(&mesh_r(), p(5, 9), p(5, 1)).expect("path");
    assert_eq!(back.points().len(), 4);
    assert_eq!(back.length(), 545_560);
}

#[test]
fn c3_non_axis_literals() {
    let n = mesh_n();
    let a = go(&n, p(1, 1), p(8, 7)).expect("path");
    assert_eq!(xz_points(&a), vec![(1, 1), (4, 2), (8, 7)]);
    assert_eq!(a.length(), 626_880);
    let b = go(&n, p(1, 1), p(5, 3)).expect("path");
    assert_eq!(xz_points(&b), vec![(1, 1), (4, 2), (5, 3)]);
    assert_eq!(b.length(), 299_926);
    let c = go(&n, p(0, 2), p(9, 8)).expect("path");
    assert_eq!(xz_points(&c), vec![(0, 2), (4, 2), (9, 8)]);
    assert_eq!(c.length(), 773_997);
}

#[test]
fn c4_same_polygon() {
    let path = go(&mesh_w(), p(1, 1), p(3, 3)).expect("path");
    assert_eq!(xz_points(&path), vec![(1, 1), (3, 3)]);
    assert_eq!(path.length(), seg_len(p(1, 1), p(3, 3)));
}

#[test]
fn c5_heights() {
    let mesh = bake_box(m(2), rect(0, 0, 10, 10), &[rect(4, 0, 6, 8)]).expect("W at y 2");
    let from = [m(2), m(-1), m(2)];
    let to = [m(8), m(7), m(2)];
    let path = go(&mesh, from, to).expect("path");
    let expected: Vec<P3> = vec![
        [m(2), m(-1), m(2)],
        [m(4), m(2), m(8)],
        [m(6), m(2), m(8)],
        [m(8), m(2), m(2)],
    ];
    assert_eq!(path.points(), expected.as_slice());
    // Same polygon: first point is `from` exactly, last takes the polygon's y.
    let short = go(&mesh, [m(1), m(-5), m(1)], [m(3), m(9), m(3)]).expect("path");
    assert_eq!(short.points(), &[[m(1), m(-5), m(1)], [m(3), m(2), m(3)]]);
}

#[test]
fn c6_errors_in_order() {
    let w = mesh_w();
    assert_eq!(go(&w, p(-1, 5), p(11, 5)), Err(NavError::StartOffMesh));
    assert_eq!(go(&w, p(2, 5), p(11, 5)), Err(NavError::GoalOffMesh));
    assert_eq!(
        go(&w, p(2, 5), [m(2), m(3), m(5)]),
        Err(NavError::AlreadyThere)
    );
    assert_eq!(go(&w, p(5, 4), p(2, 2)), Err(NavError::StartOffMesh));
    assert_eq!(go(&mesh_u(), p(2, 5), p(8, 5)), Err(NavError::Unreachable));
    // Same polygon in a split mesh is still fine.
    assert!(go(&mesh_u(), p(1, 1), p(3, 3)).is_ok());
    // Raw extremes never panic.
    let far = [i32::MIN, i32::MAX, i32::MAX].map(Fx16_16::from_raw);
    let far2 = [i32::MAX, i32::MIN, i32::MIN].map(Fx16_16::from_raw);
    assert_eq!(go(&w, far, far2), Err(NavError::StartOffMesh));
    assert_eq!(go(&w, p(2, 2), far), Err(NavError::GoalOffMesh));
}

#[test]
fn c7_limits() {
    let c31 = go(&comb(31), p(1, 10), p(199, 10)).expect("C(31) path");
    assert_eq!(c31.points().len(), MAX_PATH_POINTS);
    assert_eq!(c31.length(), seg_sum(c31.points()));
    let too_complex = go(&comb(32), p(1, 10), p(199, 10)).expect_err("C(32) is too complex");
    assert_eq!(too_complex, NavError::PathTooComplex { points: 66 });
    assert_eq!(
        too_complex.to_string(),
        "path needs 66 points, more than 64"
    );

    let l = mesh_l();
    let long = go(&l, p(0, 5), p(4500, 5)).expect_err("too long");
    assert_eq!(
        long,
        NavError::TooLong {
            length: 294_912_000
        }
    );
    assert_eq!(long.to_string(), "path length 294912000 exceeds 268435456");
    let edge = go(&l, p(0, 5), p(4096, 5)).expect("exactly the limit");
    assert_eq!(edge.points().len(), 2);
    assert_eq!(edge.length(), 268_435_456);
    assert_eq!(edge.length(), MAX_PATH_LENGTH_RAW);
    // One raw unit past the limit.
    let over = [268_435_457, 0, 5 << 16].map(Fx16_16::from_raw);
    assert_eq!(
        go(&l, p(0, 5), over),
        Err(NavError::TooLong {
            length: MAX_PATH_LENGTH_RAW + 1
        })
    );
}

#[test]
fn display_texts() {
    let table = [
        (NavError::StartOffMesh, "path start is off the navmesh"),
        (NavError::GoalOffMesh, "path goal is off the navmesh"),
        (
            NavError::AlreadyThere,
            "path start and goal are the same point",
        ),
        (
            NavError::Unreachable,
            "path goal cannot be reached from the start",
        ),
        (
            NavError::PathTooComplex { points: 65 },
            "path needs 65 points, more than 64",
        ),
        (
            NavError::TooLong { length: 7 },
            "path length 7 exceeds 268435456",
        ),
    ];
    for (error, text) in table {
        assert_eq!(error.to_string(), text);
        let boxed: Box<dyn std::error::Error> = Box::new(error);
        assert_eq!(boxed.to_string(), text);
    }
    // The bake error type keeps its own text.
    assert_eq!(
        BakeError::NoWalkableArea.to_string(),
        "bake holes cover the whole outer rectangle"
    );
}

/// `max(q.x, r.x) <= x0` or `min >= x1`, the same for `z`, or all four hole
/// corners on one side of the segment line (C-8's oracle, all `i128`).
fn clear_of_hole(q: P3, r: P3, hole: [i128; 4]) -> bool {
    let [x0, z0, x1, z1] = hole;
    let g = |v: P3| (i128::from(v[0].to_raw()), i128::from(v[2].to_raw()));
    let ((qx, qz), (rx, rz)) = (g(q), g(r));
    if qx.max(rx) <= x0 || qx.min(rx) >= x1 || qz.max(rz) <= z0 || qz.min(rz) >= z1 {
        return true;
    }
    let sides = [(x0, z0), (x1, z0), (x1, z1), (x0, z1)]
        .map(|(cx, cz)| (rx - qx) * (cz - qz) - (rz - qz) * (cx - qx));
    sides.iter().all(|&c| c >= 0) || sides.iter().all(|&c| c <= 0)
}

#[test]
fn c8_inside_the_mesh() {
    let mut rng = DeterministicRng::from_seed(35);
    let stream = rng.stream("nav");
    let mut ok = 0;
    let mut meshes = 0;
    while meshes < 200 {
        let unit = [1, 1000, 65_536][stream.gen_range_i32(0, 2) as usize];
        let w = stream.gen_range_i32(3, 30);
        let hole_count = stream.gen_range_i32(0, 8);
        let mut holes: Vec<[i32; 4]> = Vec::new();
        for _ in 0..hole_count {
            let x0 = stream.gen_range_i32(0, w - 1);
            let x1 = stream.gen_range_i32(x0 + 1, w);
            let z0 = stream.gen_range_i32(0, w - 1);
            let z1 = stream.gen_range_i32(z0 + 1, w);
            holes.push([x0 * unit, z0 * unit, x1 * unit, z1 * unit]);
        }
        let rect_raw = |v: [i32; 4]| {
            let [min_x, min_z, max_x, max_z] = v.map(Fx16_16::from_raw);
            NavRect {
                min_x,
                min_z,
                max_x,
                max_z,
            }
        };
        let rects: Vec<NavRect> = holes.iter().map(|&h| rect_raw(h)).collect();
        let outer = rect_raw([0, 0, w * unit, w * unit]);
        let mesh = match bake_box(m(0), outer, &rects) {
            Ok(mesh) => mesh,
            Err(BakeError::NoWalkableArea) => continue,
            Err(e) => panic!("bake failed: {e}"),
        };
        meshes += 1;
        let fresh = bake_box(m(0), outer, &rects).expect("rebake");
        for _ in 0..10 {
            let mut q = [mesh.nearest_on_mesh(p(0, 0)); 2];
            for slot in &mut q {
                let x = stream.gen_range_i32(-unit, (w + 1) * unit);
                let z = stream.gen_range_i32(-unit, (w + 1) * unit);
                *slot = mesh.nearest_on_mesh([x, 0, z].map(Fx16_16::from_raw));
            }
            let [a, b] = q;
            let result = go(&mesh, a, b);
            assert_eq!(result, fresh.find_path(a, b));
            match result {
                Err(NavError::Unreachable | NavError::AlreadyThere) => {}
                Err(e) => panic!("unexpected {e} for {a:?} -> {b:?}"),
                Ok(path) => {
                    ok += 1;
                    let pts = path.points();
                    assert_eq!(pts[0], a);
                    let last = pts[pts.len() - 1];
                    assert!(last[0] == b[0] && last[2] == b[2]);
                    assert_eq!(path.length(), seg_sum(pts));
                    let limit = i64::from(w) * i64::from(unit);
                    for v in pts {
                        for c in [v[0], v[2]] {
                            assert!((0..=limit).contains(&i64::from(c.to_raw())));
                        }
                    }
                    for pair in pts.windows(2) {
                        let (qq, rr) = (pair[0], pair[1]);
                        assert!(
                            qq[0] != rr[0] || qq[2] != rr[2],
                            "zero-length segment in {pts:?}"
                        );
                        for h in &holes {
                            assert!(
                                clear_of_hole(qq, rr, h.map(i128::from)),
                                "{qq:?} -> {rr:?} enters hole {h:?}"
                            );
                        }
                    }
                }
            }
        }
    }
    assert!(ok >= 1_000, "only {ok} of 2000 queries were Ok");
}

#[test]
fn c8_oracle_rejects_a_segment_through_a_hole() {
    // The oracle itself must fail a segment that cuts a hole's interior.
    let hole = [m(4), m(0), m(6), m(8)].map(|v| i128::from(v.to_raw()));
    assert!(!clear_of_hole(p(2, 2), p(8, 2), hole));
    assert!(clear_of_hole(p(2, 2), p(4, 8), hole));
    assert!(clear_of_hole(p(4, 8), p(6, 8), hole));
    assert!(clear_of_hole(p(2, 9), p(8, 9), hole));
}

#[test]
fn c9_deterministic() {
    type Case = (fn() -> NavMesh, P3, P3);
    let cases: Vec<Case> = vec![
        (mesh_w, p(2, 2), p(8, 2)),
        (mesh_r, p(5, 1), p(5, 9)),
        (mesh_n, p(1, 1), p(8, 7)),
        (mesh_n, p(0, 2), p(9, 8)),
        (mesh_w, p(1, 1), p(3, 3)),
        (mesh_u, p(2, 5), p(8, 5)),
        (|| comb(31), p(1, 10), p(199, 10)),
        (|| comb(32), p(1, 10), p(199, 10)),
        (mesh_l, p(0, 5), p(4500, 5)),
        (mesh_l, p(0, 5), p(4096, 5)),
    ];
    for (build, from, to) in cases {
        let (a, b) = (build(), build());
        assert_eq!(a, b);
        assert_eq!(go(&a, from, to), go(&b, from, to));
    }
}

#[test]
fn c2b_heuristic_breaks_f_ties() {
    // A 5 x 5 box with a 3 x 3 hole: an f-tie between polygons whose
    // heuristics differ is settled by the smaller `h`, not by the id alone.
    let mesh = bake_box(m(0), rect(0, 0, 5, 5), &[rect(1, 1, 4, 4)]).expect("bakes");
    let path = go(&mesh, p(2, 0), p(3, 4)).expect("path");
    assert_eq!(xz_points(&path), vec![(2, 0), (4, 1), (4, 4), (3, 4)]);
    assert_eq!(path.length(), 408_687);
}

#[test]
fn c2c_strict_improvement_only() {
    // A later, cheaper route to an open polygon replaces its parent (a strict
    // improvement), and an equal-cost route does not.
    let a = bake_box(
        m(0),
        rect(0, 0, 4, 4),
        &[rect(2, 2, 4, 4), rect(3, 1, 4, 4)],
    )
    .expect("A");
    let path = go(&a, p(0, 2), p(4, 0)).expect("path");
    assert_eq!(xz_points(&path), vec![(0, 2), (2, 1), (4, 0)]);
    assert_eq!(path.length(), 293_086);
    let b = bake_box(
        m(0),
        rect(0, 0, 4, 4),
        &[rect(3, 1, 4, 2), rect(1, 2, 2, 4)],
    )
    .expect("B");
    let path = go(&b, p(4, 0), p(0, 4)).expect("path");
    assert_eq!(xz_points(&path), vec![(4, 0), (1, 2), (0, 4)]);
    assert_eq!(path.length(), 382_837);
}

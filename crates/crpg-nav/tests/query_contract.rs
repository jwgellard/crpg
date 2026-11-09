//! T035b two-keys suite, written from `tasks/T035.md` alone (B-B1 to B-B3,
//! B-Q1 to B-Q5). Integers only. Expected values are hand-derived literals
//! or come from the independent integer model in this file, which also
//! proves its own sensitivity: every mutated model must disagree with the
//! contract model somewhere in the corpus.

use std::collections::{BTreeMap, BTreeSet};

use crpg_core::DeterministicRng;
use crpg_nav::{
    bake_box, BakeError, Fx16_16, NavMesh, NavMeshError, NavRect, PolyId, COORD_LIMIT_RAW,
    MAX_BAKE_HOLES,
};

const LIM: i32 = COORD_LIMIT_RAW;
const UNIT: i32 = 65536;

// ---------------------------------------------------------------- helpers

fn m(v: i32) -> Fx16_16 {
    Fx16_16::from_int(v)
}

fn p(x: i32, z: i32) -> [Fx16_16; 3] {
    [m(x), m(0), m(z)]
}

fn rw(x: i32, y: i32, z: i32) -> [Fx16_16; 3] {
    [
        Fx16_16::from_raw(x),
        Fx16_16::from_raw(y),
        Fx16_16::from_raw(z),
    ]
}

fn rect(x0: i32, z0: i32, x1: i32, z1: i32) -> NavRect {
    NavRect {
        min_x: m(x0),
        min_z: m(z0),
        max_x: m(x1),
        max_z: m(z1),
    }
}

/// Raw rectangle from `[min_x, min_z, max_x, max_z]`.
fn rr(a: [i32; 4]) -> NavRect {
    NavRect {
        min_x: Fx16_16::from_raw(a[0]),
        min_z: Fx16_16::from_raw(a[1]),
        max_x: Fx16_16::from_raw(a[2]),
        max_z: Fx16_16::from_raw(a[3]),
    }
}

fn bake_raw(y: i32, outer: [i32; 4], holes: &[[i32; 4]]) -> Result<NavMesh, BakeError> {
    let hs: Vec<NavRect> = holes.iter().map(|h| rr(*h)).collect();
    bake_box(Fx16_16::from_raw(y), rr(outer), &hs)
}

/// Metre rectangle `[x0, z0, x1, z1]` as a raw array.
fn mm(x0: i32, z0: i32, x1: i32, z1: i32) -> [i32; 4] {
    [x0 * UNIT, z0 * UNIT, x1 * UNIT, z1 * UNIT]
}

fn bake_m(y: i32, outer: [i32; 4], holes: &[[i32; 4]]) -> Result<NavMesh, BakeError> {
    let o = mm(outer[0], outer[1], outer[2], outer[3]);
    let hs: Vec<[i32; 4]> = holes.iter().map(|h| mm(h[0], h[1], h[2], h[3])).collect();
    bake_raw(y * UNIT, o, &hs)
}

/// Raw vertices and polygon index lists of a mesh.
type Snap = (Vec<[i32; 3]>, Vec<Vec<u32>>);

fn snap(mesh: &NavMesh) -> Snap {
    let vs = mesh
        .vertices()
        .iter()
        .map(|v| [v[0].to_raw(), v[1].to_raw(), v[2].to_raw()])
        .collect();
    let ps = (0..mesh.polygon_count())
        .map(|i| mesh.polygon(PolyId(i as u32)).unwrap().to_vec())
        .collect();
    (vs, ps)
}

/// Neighbours recomputed from the polygons alone (reversed directed pair).
fn oracle_neighbours(polys: &[Vec<u32>]) -> Vec<Vec<Option<PolyId>>> {
    let mut owner: BTreeMap<(u32, u32), u32> = BTreeMap::new();
    for (id, poly) in polys.iter().enumerate() {
        for i in 0..poly.len() {
            owner.insert((poly[i], poly[(i + 1) % poly.len()]), id as u32);
        }
    }
    polys
        .iter()
        .map(|poly| {
            (0..poly.len())
                .map(|i| {
                    owner
                        .get(&(poly[(i + 1) % poly.len()], poly[i]))
                        .map(|&q| PolyId(q))
                })
                .collect()
        })
        .collect()
}

/// Whole-mesh literal check: vertices (metres, height `y` raw), polygons,
/// per-polygon height and derived adjacency.
fn expect_mesh(mesh: &NavMesh, y: i32, vm: &[(i32, i32)], polys: &[&[u32]]) {
    let (vs, ps) = snap(mesh);
    let ev: Vec<[i32; 3]> = vm.iter().map(|&(x, z)| [x * UNIT, y, z * UNIT]).collect();
    let ep: Vec<Vec<u32>> = polys.iter().map(|q| q.to_vec()).collect();
    assert_eq!(vs, ev);
    assert_eq!(ps, ep);
    let nb = oracle_neighbours(&ps);
    for (i, expected) in nb.iter().enumerate() {
        assert_eq!(
            mesh.neighbours(PolyId(i as u32)).unwrap(),
            expected.as_slice()
        );
        assert_eq!(mesh.polygon_y(PolyId(i as u32)), Some(Fx16_16::from_raw(y)));
    }
}

const W_V: [(i32, i32); 12] = [
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
const W_P: [&[u32]; 5] = [
    &[0, 1, 2, 3],
    &[3, 2, 4, 5],
    &[2, 6, 7, 4],
    &[8, 9, 10, 6],
    &[6, 10, 11, 7],
];

fn mesh_w() -> NavMesh {
    bake_box(m(0), rect(0, 0, 10, 10), &[rect(4, 0, 6, 8)]).unwrap()
}

fn mesh_n() -> NavMesh {
    let vs: Vec<[Fx16_16; 3]> = [(0, 0), (4, 0), (4, 2), (0, 2), (8, 4), (6, 6), (9, 8)]
        .iter()
        .map(|&(x, z)| p(x, z))
        .collect();
    NavMesh::new(vs, vec![vec![0, 1, 2, 3], vec![1, 4, 5, 2], vec![5, 4, 6]]).unwrap()
}

fn locate_m(mesh: &NavMesh, x: i32, z: i32) -> Option<u32> {
    mesh.locate(p(x, z)).map(|id| id.0)
}

// ---------------------------------------------------------------- B-B1

#[test]
fn bake_too_many_holes() {
    assert_eq!(MAX_BAKE_HOLES, 32);
    let hole = mm(1, 1, 2, 2);
    // 32 identical holes are legal (overlap allowed); 33 are not.
    assert!(bake_raw(0, mm(0, 0, 10, 10), &[hole; 32]).is_ok());
    assert_eq!(
        bake_raw(0, mm(0, 0, 10, 10), &[hole; 33]).unwrap_err(),
        BakeError::TooManyHoles { count: 33 }
    );
    assert_eq!(
        bake_raw(0, mm(0, 0, 10, 10), &[hole; 100]).unwrap_err(),
        BakeError::TooManyHoles { count: 100 }
    );
    // Declared first: wins over every later failure at once.
    let bad = [[i32::MAX, 0, i32::MIN, 0]; 33];
    assert_eq!(
        bake_raw(i32::MAX, [5, 5, 5, 5], &bad).unwrap_err(),
        BakeError::TooManyHoles { count: 33 }
    );
    // The same inputs with 32 holes fall through to the next check.
    assert_eq!(
        bake_raw(i32::MAX, [5, 5, 5, 5], &bad[..32]).unwrap_err(),
        BakeError::OutOfBounds
    );
}

#[test]
fn bake_out_of_bounds_every_field() {
    let full = [-LIM, -LIM, LIM, LIM];
    // Exactly the limit passes, both signs, for y and all outer values.
    assert!(bake_raw(0, full, &[]).is_ok());
    assert!(bake_raw(LIM, full, &[]).is_ok());
    assert!(bake_raw(-LIM, full, &[]).is_ok());
    for bad in [LIM + 1, -LIM - 1, i32::MAX, i32::MIN] {
        assert_eq!(bake_raw(bad, full, &[]), Err(BakeError::OutOfBounds));
    }
    // Outer fields: sign matched, so the rectangle is otherwise sound.
    for f in 0..4 {
        let bads: [i32; 2] = if f < 2 {
            [-LIM - 1, i32::MIN]
        } else {
            [LIM + 1, i32::MAX]
        };
        for bad in bads {
            let mut o = full;
            o[f] = bad;
            assert_eq!(bake_raw(0, o, &[]), Err(BakeError::OutOfBounds), "{f}");
        }
    }
    // The wrong sign on a field is also out of bounds (and also an empty
    // rectangle, which is declared later).
    for f in 0..4 {
        let mut o = full;
        o[f] = if f < 2 { LIM + 1 } else { -LIM - 1 };
        assert_eq!(bake_raw(0, o, &[]), Err(BakeError::OutOfBounds), "{f}");
    }
    // Hole fields, first and second hole; sign matched, so each also lies
    // outside the outer, which is declared later.
    let ok_hole = [-5, -5, 5, 5];
    for f in 0..4 {
        let bads: [i32; 2] = if f < 2 {
            [-LIM - 1, i32::MIN]
        } else {
            [LIM + 1, i32::MAX]
        };
        for bad in bads {
            let mut h = ok_hole;
            h[f] = bad;
            assert_eq!(
                bake_raw(0, full, &[h]),
                Err(BakeError::OutOfBounds),
                "hole0 {f}"
            );
            assert_eq!(
                bake_raw(0, full, &[ok_hole, h]),
                Err(BakeError::OutOfBounds),
                "hole1 {f}"
            );
        }
    }
    // A hole touching the outer edge at exactly the limit is inside.
    assert!(bake_raw(0, full, &[[-LIM, -LIM, -LIM + 5, -LIM + 5]]).is_ok());
    assert!(bake_raw(0, full, &[[LIM - 5, LIM - 5, LIM, LIM]]).is_ok());
    assert_eq!(
        bake_raw(0, full, &[full]),
        Err(BakeError::NoWalkableArea),
        "hole equal to the outer at the limit"
    );
}

#[test]
fn bake_empty_outer() {
    assert!(bake_raw(0, [0, 0, 1, 1], &[]).is_ok());
    for bad in [
        [0, 0, 0, 10],  // min_x == max_x
        [0, 0, 10, 0],  // min_z == max_z
        [5, 0, 4, 10],  // min_x > max_x
        [0, 5, 10, 4],  // min_z > max_z
        [3, 3, 3, 3],   // a point
        [10, 10, 0, 0], // both reversed
    ] {
        assert_eq!(bake_raw(0, bad, &[]), Err(BakeError::EmptyOuter), "{bad:?}");
    }
    // One raw unit of width is enough.
    assert!(bake_raw(0, [0, 0, 1, 10], &[]).is_ok());
    assert!(bake_raw(0, [0, 0, 10, 1], &[]).is_ok());
}

#[test]
fn bake_empty_hole() {
    let o = mm(0, 0, 10, 10);
    assert!(bake_raw(0, o, &[mm(9, 0, 10, 1)]).is_ok());
    for (k, bad) in [
        mm(9, 0, 9, 1),  // min_x == max_x
        mm(9, 3, 10, 3), // min_z == max_z
        mm(5, 1, 3, 2),  // min_x > max_x
        mm(1, 5, 2, 3),  // min_z > max_z
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(
            bake_raw(0, o, &[bad]),
            Err(BakeError::EmptyHole { hole: 0 }),
            "case {k}"
        );
    }
    // Contract examples via the metre helper.
    assert_eq!(
        bake_box(m(0), rect(0, 0, 10, 10), &[rect(9, 0, 9, 1)]).unwrap_err(),
        BakeError::EmptyHole { hole: 0 }
    );
    // A hole one raw unit wide is not empty.
    assert!(bake_raw(0, o, &[[100, 100, 101, 200]]).is_ok());
}

#[test]
fn bake_hole_outside_each_side() {
    // Outer offset from the origin so no field can be mistaken for another.
    let o = [UNIT, 2 * UNIT, 11 * UNIT, 12 * UNIT];
    let inner = [200_000, 200_000, 300_000, 300_000];
    for f in 0..4 {
        // Touching the outer edge from inside passes; one raw unit past fails.
        let mut touch = inner;
        touch[f] = o[f];
        assert!(bake_raw(0, o, &[touch]).is_ok(), "touch {f}");
        let mut past = inner;
        past[f] = if f < 2 { o[f] - 1 } else { o[f] + 1 };
        assert_eq!(
            bake_raw(0, o, &[past]),
            Err(BakeError::HoleOutside { hole: 0 }),
            "past {f}"
        );
    }
    assert_eq!(
        bake_box(m(0), rect(0, 0, 10, 10), &[rect(9, 0, 11, 1)]).unwrap_err(),
        BakeError::HoleOutside { hole: 0 }
    );
    // A hole larger than the outer is outside, not "no walkable area".
    assert_eq!(
        bake_m(0, [0, 0, 10, 10], &[[-1, -1, 11, 11]]),
        Err(BakeError::HoleOutside { hole: 0 })
    );
}

#[test]
fn bake_no_walkable_area() {
    assert_eq!(
        bake_box(m(0), rect(0, 0, 10, 10), &[rect(0, 0, 10, 10)]).unwrap_err(),
        BakeError::NoWalkableArea
    );
    // Several holes that together cover everything.
    assert_eq!(
        bake_m(0, [0, 0, 10, 10], &[[0, 0, 5, 10], [5, 0, 10, 10]]),
        Err(BakeError::NoWalkableArea)
    );
    // The same holes with a one-raw-unit gap leave one cell.
    let mesh = bake_raw(
        0,
        mm(0, 0, 10, 10),
        &[
            [0, 0, 5 * UNIT, 10 * UNIT],
            [5 * UNIT + 1, 0, 10 * UNIT, 10 * UNIT],
        ],
    )
    .unwrap();
    assert_eq!(mesh.polygon_count(), 1);
    // A hole one unit short of covering leaves a strip.
    assert!(bake_m(0, [0, 0, 10, 10], &[[0, 0, 10, 9]]).is_ok());
}

#[test]
fn bake_error_declaration_order() {
    let o = mm(0, 0, 10, 10);
    let empty_outer = [0, 0, 0, 10];
    let ok_hole = mm(1, 1, 2, 2);
    // TooManyHoles over OutOfBounds: covered in bake_too_many_holes.
    // OutOfBounds over EmptyOuter.
    assert_eq!(
        bake_raw(LIM + 1, empty_outer, &[]),
        Err(BakeError::OutOfBounds)
    );
    assert_eq!(bake_raw(0, empty_outer, &[]), Err(BakeError::EmptyOuter));
    // A later hole out of bounds still beats an empty outer.
    assert_eq!(
        bake_raw(0, empty_outer, &[ok_hole, [0, 0, LIM + 1, 5]]),
        Err(BakeError::OutOfBounds)
    );
    // OutOfBounds over EmptyHole / HoleOutside on earlier holes.
    assert_eq!(
        bake_raw(
            0,
            o,
            &[mm(3, 3, 3, 4), mm(40, 0, 41, 1), [0, 0, LIM + 1, 5]]
        ),
        Err(BakeError::OutOfBounds)
    );
    // EmptyOuter over EmptyHole and HoleOutside.
    assert_eq!(
        bake_raw(0, empty_outer, &[mm(3, 3, 3, 4)]),
        Err(BakeError::EmptyOuter)
    );
    assert_eq!(
        bake_raw(0, empty_outer, &[mm(30, 3, 31, 4)]),
        Err(BakeError::EmptyOuter)
    );
    // EmptyHole over HoleOutside on the same hole.
    assert_eq!(
        bake_raw(0, o, &[mm(11, 0, 11, 1)]),
        Err(BakeError::EmptyHole { hole: 0 })
    );
    assert_eq!(
        bake_raw(0, o, &[mm(11, 0, 12, 1)]),
        Err(BakeError::HoleOutside { hole: 0 })
    );
    // Hole order: the first failing hole is reported, whichever kind.
    let outside = mm(11, 0, 12, 1);
    let empty = mm(3, 3, 3, 4);
    assert_eq!(
        bake_raw(0, o, &[outside, empty]),
        Err(BakeError::HoleOutside { hole: 0 })
    );
    assert_eq!(
        bake_raw(0, o, &[empty, outside]),
        Err(BakeError::EmptyHole { hole: 0 })
    );
    assert_eq!(
        bake_raw(0, o, &[ok_hole, empty]),
        Err(BakeError::EmptyHole { hole: 1 })
    );
    assert_eq!(
        bake_raw(0, o, &[ok_hole, ok_hole, outside]),
        Err(BakeError::HoleOutside { hole: 2 })
    );
    assert_eq!(
        bake_raw(0, o, &[ok_hole, outside, empty]),
        Err(BakeError::HoleOutside { hole: 1 })
    );
    // HoleOutside over NoWalkableArea.
    assert_eq!(
        bake_raw(0, o, &[mm(-1, -1, 11, 11)]),
        Err(BakeError::HoleOutside { hole: 0 })
    );
    assert_eq!(bake_raw(0, o, &[o]), Err(BakeError::NoWalkableArea));
}

#[test]
fn bake_error_display_texts() {
    fn is_error<E: std::error::Error>(_: &E) {}
    let cases = [
        (
            BakeError::TooManyHoles { count: 33 },
            "bake has 33 holes, more than 32",
        ),
        (
            BakeError::OutOfBounds,
            "bake coordinate is outside the coordinate limit",
        ),
        (BakeError::EmptyOuter, "bake outer rectangle is empty"),
        (BakeError::EmptyHole { hole: 3 }, "bake hole 3 is empty"),
        (
            BakeError::HoleOutside { hole: 4 },
            "bake hole 4 is not inside the outer rectangle",
        ),
        (
            BakeError::NoWalkableArea,
            "bake holes cover the whole outer rectangle",
        ),
        (
            BakeError::Mesh(NavMeshError::PolygonCount { count: 0 }),
            "navmesh polygon count 0 is outside 1..=16384",
        ),
    ];
    for (e, text) in cases {
        is_error(&e);
        assert_eq!(e.to_string(), text);
    }
}

// ---------------------------------------------------------------- B-B2, B-B3

#[test]
fn bake_w_and_r_literals() {
    let w = mesh_w();
    expect_mesh(&w, 0, &W_V, &W_P);
    let expected: [[Option<u32>; 4]; 5] = [
        [None, None, Some(1), None],
        [Some(0), Some(2), None, None],
        [None, Some(4), None, Some(1)],
        [None, None, Some(4), None],
        [Some(3), None, None, Some(2)],
    ];
    for (i, row) in expected.iter().enumerate() {
        let want: Vec<Option<PolyId>> = row.iter().map(|o| o.map(PolyId)).collect();
        assert_eq!(w.neighbours(PolyId(i as u32)).unwrap(), want.as_slice());
    }
    let r = bake_box(m(0), rect(0, 0, 10, 10), &[rect(4, 4, 6, 6)]).unwrap();
    expect_mesh(
        &r,
        0,
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
        &[
            &[0, 1, 2, 3],
            &[3, 2, 4, 5],
            &[5, 4, 6, 7],
            &[1, 8, 9, 2],
            &[4, 10, 11, 6],
            &[8, 12, 13, 9],
            &[9, 13, 14, 10],
            &[10, 14, 15, 11],
        ],
    );
}

#[test]
fn bake_corner_order_and_height() {
    // One cell, asymmetric, non-zero height: x/z swaps and rotations show.
    let one = bake_box(m(2), rect(0, 0, 10, 10), &[]).unwrap();
    expect_mesh(
        &one,
        2 * UNIT,
        &[(0, 0), (10, 0), (10, 10), (0, 10)],
        &[&[0, 1, 2, 3]],
    );
    let off = bake_box(m(3), rect(-2, 1, 5, 4), &[]).unwrap();
    expect_mesh(
        &off,
        3 * UNIT,
        &[(-2, 1), (5, 1), (5, 4), (-2, 4)],
        &[&[0, 1, 2, 3]],
    );
    // Negative and fractional heights.
    for y in [-7 * UNIT, 12345, -1, LIM, -LIM] {
        let mesh = bake_raw(y, mm(0, 0, 6, 6), &[mm(2, 2, 3, 3)]).unwrap();
        let (vs, _) = snap(&mesh);
        assert!(vs.iter().all(|v| v[1] == y));
        assert_eq!(mesh.polygon_count(), 8);
        for i in 0..8 {
            assert_eq!(mesh.polygon_y(PolyId(i)), Some(Fx16_16::from_raw(y)));
        }
    }
}

#[test]
fn bake_covered_cell_rules() {
    // Touching holes: each cell is covered by its own hole only.
    let touching = bake_m(0, [0, 0, 6, 2], &[[1, 0, 2, 2], [2, 0, 3, 2]]).unwrap();
    expect_mesh(
        &touching,
        0,
        &[
            (0, 0),
            (1, 0),
            (1, 2),
            (0, 2),
            (3, 0),
            (6, 0),
            (6, 2),
            (3, 2),
        ],
        &[&[0, 1, 2, 3], &[4, 5, 6, 7]],
    );
    // A hole on the outer corner; vertices shared across polygons keep the
    // index of their first appearance.
    let corner = bake_m(0, [0, 0, 10, 10], &[[0, 0, 3, 3]]).unwrap();
    expect_mesh(
        &corner,
        0,
        &[
            (0, 3),
            (3, 3),
            (3, 10),
            (0, 10),
            (3, 0),
            (10, 0),
            (10, 3),
            (10, 10),
        ],
        &[&[0, 1, 2, 3], &[4, 5, 6, 1], &[1, 6, 7, 2]],
    );
    // A duplicated hole changes nothing.
    let once = bake_m(0, [0, 0, 10, 10], &[[2, 2, 5, 5]]).unwrap();
    let twice = bake_m(0, [0, 0, 10, 10], &[[2, 2, 5, 5], [2, 2, 5, 5]]).unwrap();
    assert_eq!(once, twice);
    // Overlapping holes: 18 polygons, 34 vertices, area 100 - 28 = 72 m^2.
    let ov = bake_box(
        m(0),
        rect(0, 0, 10, 10),
        &[rect(2, 2, 6, 6), rect(4, 4, 8, 8)],
    )
    .unwrap();
    assert_eq!(ov.polygon_count(), 18);
    assert_eq!(ov.vertices().len(), 34);
    let (vs, ps) = snap(&ov);
    let mut twice_area: i128 = 0;
    for poly in &ps {
        for i in 0..poly.len() {
            let a = vs[poly[i] as usize];
            let b = vs[poly[(i + 1) % poly.len()] as usize];
            twice_area += i128::from(a[0]) * i128::from(b[2]) - i128::from(b[0]) * i128::from(a[2]);
        }
    }
    assert_eq!(twice_area, 144 * i128::from(UNIT) * i128::from(UNIT));
    // Grid vertices interior to a hole are not emitted: 16 cells, 32 vertices.
    let nested = bake_m(0, [0, 0, 10, 10], &[[2, 2, 8, 8], [4, 4, 5, 5]]).unwrap();
    assert_eq!(nested.polygon_count(), 16);
    assert_eq!(nested.vertices().len(), 32);
    let (_, nested_polys) = snap(&nested);
    let used: BTreeSet<u32> = nested_polys.into_iter().flatten().collect();
    assert_eq!(used.len(), 32);
}

#[test]
fn bake_large_and_bytes() {
    let holes: Vec<NavRect> = (0..32)
        .map(|i| {
            let x0 = 2 + 6 * i;
            if i % 2 == 0 {
                rect(x0, 0, x0 + 2, 16)
            } else {
                rect(x0, 4, x0 + 2, 20)
            }
        })
        .collect();
    let comb = bake_box(m(0), rect(0, 0, 200, 20), &holes).unwrap();
    assert_eq!(comb.polygon_count(), 131);
    assert_eq!(comb.vertices().len(), 264);
    let w = mesh_w();
    assert_eq!(w.to_canonical_bytes().len(), 242);
    let w2 = mesh_w();
    assert_eq!(w, w2);
    assert_eq!(w.to_canonical_bytes(), w2.to_canonical_bytes());
}

#[test]
fn bake_result_is_mesh_new_of_its_parts() {
    // B-B3: the baked mesh is exactly NavMesh::new of what it holds.
    for mesh in [
        mesh_w(),
        bake_m(1, [0, 0, 10, 10], &[[2, 2, 6, 6], [4, 4, 8, 8]]).unwrap(),
    ] {
        let (_, polys) = snap(&mesh);
        let rebuilt = NavMesh::new(mesh.vertices().to_vec(), polys).unwrap();
        assert_eq!(rebuilt, mesh);
    }
}

/// Independent bake model: grid of distinct edges, hole cover, emission.
fn model_bake(y: i32, outer: [i32; 4], holes: &[[i32; 4]]) -> Option<Snap> {
    let mut xs = BTreeSet::from([outer[0], outer[2]]);
    let mut zs = BTreeSet::from([outer[1], outer[3]]);
    for h in holes {
        xs.extend([h[0], h[2]]);
        zs.extend([h[1], h[3]]);
    }
    let xs: Vec<i32> = xs.into_iter().collect();
    let zs: Vec<i32> = zs.into_iter().collect();
    let mut verts: Vec<[i32; 3]> = Vec::new();
    let mut polys: Vec<Vec<u32>> = Vec::new();
    for i in 0..xs.len() - 1 {
        for j in 0..zs.len() - 1 {
            let covered = holes
                .iter()
                .any(|h| h[0] <= xs[i] && xs[i + 1] <= h[2] && h[1] <= zs[j] && zs[j + 1] <= h[3]);
            if covered {
                continue;
            }
            let corners = [
                (xs[i], zs[j]),
                (xs[i + 1], zs[j]),
                (xs[i + 1], zs[j + 1]),
                (xs[i], zs[j + 1]),
            ];
            let mut poly = Vec::new();
            for (cx, cz) in corners {
                let v = [cx, y, cz];
                let idx = match verts.iter().position(|w| *w == v) {
                    Some(k) => k,
                    None => {
                        verts.push(v);
                        verts.len() - 1
                    }
                };
                poly.push(idx as u32);
            }
            polys.push(poly);
        }
    }
    if polys.is_empty() {
        None
    } else {
        Some((verts, polys))
    }
}

fn random_bake(rng: &mut DeterministicRng) -> (i32, [i32; 4], Vec<[i32; 4]>) {
    let s = rng.stream("bake");
    let ox = s.gen_range_i32(-3, 3);
    let oz = s.gen_range_i32(-3, 3);
    let w = s.gen_range_i32(3, 9);
    let h = s.gen_range_i32(3, 9);
    let y = s.gen_range_i32(-5, 5);
    let k = s.gen_range_i32(0, 6);
    let mut holes = Vec::new();
    for _ in 0..k {
        let x0 = s.gen_range_i32(0, w - 1);
        let x1 = s.gen_range_i32(x0 + 1, w);
        let z0 = s.gen_range_i32(0, h - 1);
        let z1 = s.gen_range_i32(z0 + 1, h);
        holes.push([ox + x0, oz + z0, ox + x1, oz + z1]);
    }
    (y, [ox, oz, ox + w, oz + h], holes)
}

#[test]
fn bake_matches_independent_model() {
    let mut rng = DeterministicRng::from_seed(41);
    let (mut none, mut some) = (0, 0);
    for case in 0..1500 {
        let (y, outer, holes) = random_bake(&mut rng);
        match (model_bake(y, outer, &holes), bake_raw(y, outer, &holes)) {
            (None, got) => {
                none += 1;
                assert_eq!(got, Err(BakeError::NoWalkableArea), "case {case}");
            }
            (Some((vs, ps)), Ok(mesh)) => {
                some += 1;
                assert_eq!(snap(&mesh), (vs, ps), "case {case}");
            }
            (Some(_), Err(e)) => panic!("case {case}: unexpected {e:?}"),
        }
    }
    assert!(none > 0 && some > 0, "corpus must hit both outcomes");
}

// ---------------------------------------------------------------- query model

type V2 = [i128; 2];

struct MP {
    v: Vec<V2>,
    y: i32,
}

#[derive(Clone, Copy)]
struct Knobs {
    keep_last: bool,
    cand_last: bool,
    fb_lt: bool,
    trunc: bool,
    rev: bool,
    no_filter: bool,
    strict: bool,
}

const K0: Knobs = Knobs {
    keep_last: false,
    cand_last: false,
    fb_lt: false,
    trunc: false,
    rev: false,
    no_filter: false,
    strict: false,
};

/// Each mutant breaks exactly one rule of the contract.
const MUTANTS: [Knobs; 7] = [
    Knobs {
        keep_last: true,
        ..K0
    },
    Knobs {
        cand_last: true,
        ..K0
    },
    Knobs { fb_lt: true, ..K0 },
    Knobs { trunc: true, ..K0 },
    Knobs { rev: true, ..K0 },
    Knobs {
        no_filter: true,
        ..K0
    },
    Knobs { strict: true, ..K0 },
];
const MUTANT_NAMES: [&str; 7] = [
    "keep last on ties",
    "candidate ties keep last",
    "fallback < instead of <=",
    "truncating division",
    "candidate order reversed",
    "no contains filter",
    "strict contains",
];

fn cr(o: V2, a: V2, b: V2) -> i128 {
    (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
}

fn d2(a: V2, b: V2) -> i128 {
    (a[0] - b[0]) * (a[0] - b[0]) + (a[1] - b[1]) * (a[1] - b[1])
}

fn inside(poly: &MP, q: V2, strict: bool) -> bool {
    let n = poly.v.len();
    (0..n).all(|i| {
        let c = cr(poly.v[i], poly.v[(i + 1) % n], q);
        if strict {
            c > 0
        } else {
            c >= 0
        }
    })
}

fn edge_candidate(poly: &MP, a: V2, b: V2, q: V2, k: Knobs) -> V2 {
    let ex = b[0] - a[0];
    let ez = b[1] - a[1];
    let t = (q[0] - a[0]) * ex + (q[1] - a[1]) * ez;
    let l2 = ex * ex + ez * ez;
    if t <= 0 {
        return a;
    }
    if t >= l2 {
        return b;
    }
    let (qx, rx, qz, rz) = if k.trunc {
        ((ex * t) / l2, (ex * t) % l2, (ez * t) / l2, (ez * t) % l2)
    } else {
        (
            (ex * t).div_euclid(l2),
            (ex * t).rem_euclid(l2),
            (ez * t).div_euclid(l2),
            (ez * t).rem_euclid(l2),
        )
    };
    let fx = a[0] + qx;
    let cx = fx + i128::from(rx != 0);
    let fz = a[1] + qz;
    let cz = fz + i128::from(rz != 0);
    let mut cands = [[fx, fz], [fx, cz], [cx, fz], [cx, cz]];
    if k.rev {
        cands.reverse();
    }
    let mut best: Option<V2> = None;
    for c in cands {
        if !k.no_filter && !inside(poly, c, k.strict) {
            continue;
        }
        best = match best {
            None => Some(c),
            Some(b0) => {
                let better = if k.cand_last {
                    d2(c, q) <= d2(b0, q)
                } else {
                    d2(c, q) < d2(b0, q)
                };
                Some(if better { c } else { b0 })
            }
        };
    }
    best.unwrap_or_else(|| {
        let a_wins = if k.fb_lt {
            d2(a, q) < d2(b, q)
        } else {
            d2(a, q) <= d2(b, q)
        };
        if a_wins {
            a
        } else {
            b
        }
    })
}

/// `(locate, nearest)` of the model; nearest is `[x, y, z]`.
fn model(polys: &[MP], q: V2, k: Knobs) -> (Option<usize>, [i128; 3]) {
    let loc = polys.iter().position(|pl| inside(pl, q, k.strict));
    if let Some(id) = loc {
        return (loc, [q[0], i128::from(polys[id].y), q[1]]);
    }
    let mut best: Option<(i128, V2, i32)> = None;
    for poly in polys {
        let n = poly.v.len();
        for i in 0..n {
            let c = edge_candidate(poly, poly.v[i], poly.v[(i + 1) % n], q, k);
            let d = d2(c, q);
            let take = match best {
                None => true,
                Some((bd, _, _)) => {
                    if k.keep_last {
                        d <= bd
                    } else {
                        d < bd
                    }
                }
            };
            if take {
                best = Some((d, c, poly.y));
            }
        }
    }
    let (_, c, y) = best.unwrap();
    (None, [c[0], i128::from(y), c[1]])
}

fn polys_of(mesh: &NavMesh) -> Vec<MP> {
    (0..mesh.polygon_count())
        .map(|i| {
            let id = PolyId(i as u32);
            MP {
                v: mesh
                    .polygon(id)
                    .unwrap()
                    .iter()
                    .map(|&k| {
                        let v = mesh.vertices()[k as usize];
                        [i128::from(v[0].to_raw()), i128::from(v[2].to_raw())]
                    })
                    .collect(),
                y: mesh.polygon_y(id).unwrap().to_raw(),
            }
        })
        .collect()
}

fn raw3(v: [Fx16_16; 3]) -> [i128; 3] {
    [
        i128::from(v[0].to_raw()),
        i128::from(v[1].to_raw()),
        i128::from(v[2].to_raw()),
    ]
}

/// Compares the implementation with the contract model on one query and
/// tallies which mutated models disagree with it.
fn check(mesh: &NavMesh, mp: &[MP], q: [i32; 3], tally: &mut [u32; 7]) {
    let qv = [i128::from(q[0]), i128::from(q[2])];
    let (ml, mn) = model(mp, qv, K0);
    let pt = rw(q[0], q[1], q[2]);
    assert_eq!(mesh.locate(pt).map(|id| id.0 as usize), ml, "locate {q:?}");
    let n = mesh.nearest_on_mesh(pt);
    assert_eq!(raw3(n), mn, "nearest {q:?}");
    assert!(mesh.locate(n).is_some(), "nearest off the mesh {q:?}");
    for (i, k) in MUTANTS.iter().enumerate() {
        if model(mp, qv, *k) != (ml, mn) {
            tally[i] += 1;
        }
    }
}

/// Mutant 2 (fallback tie) is separated by the unimodular scan instead.
fn assert_all_mutants_seen(tally: &[u32; 7]) {
    for (i, n) in tally.iter().enumerate() {
        assert!(
            *n > 0 || i == 2,
            "corpus never separates mutant: {}",
            MUTANT_NAMES[i]
        );
    }
}

// ---------------------------------------------------------------- B-Q2

#[test]
fn locate_w_literals() {
    let w = mesh_w();
    let cases: [(i32, i32, Option<u32>); 22] = [
        (2, 2, Some(0)),
        (4, 8, Some(0)), // corner of P0, P1, P2
        (4, 4, Some(0)),
        (5, 9, Some(2)),
        (5, 8, Some(2)),
        (10, 10, Some(4)),
        (5, 4, None), // inside the hole
        (11, 5, None),
        (6, 8, Some(2)), // corner of P2, P3, P4
        (4, 0, Some(0)),
        (5, 0, None), // under the hole, off the mesh
        (6, 0, Some(3)),
        (6, 4, Some(3)), // hole edge
        (4, 10, Some(1)),
        (4, 9, Some(1)), // shared edge P1 / P2
        (6, 9, Some(2)), // shared edge P2 / P4
        (8, 8, Some(3)), // shared edge P3 / P4
        (8, 10, Some(4)),
        (10, 5, Some(3)),
        (0, 10, Some(1)),
        (-1, 5, None),
        (5, 11, None),
    ];
    for (x, z, want) in cases {
        assert_eq!(locate_m(&w, x, z), want, "({x},{z})");
    }
    // One raw unit off the edge flips the answer.
    assert_eq!(w.locate(rw(4 * UNIT + 1, 0, 4 * UNIT)), None);
    assert_eq!(w.locate(rw(4 * UNIT, 0, 4 * UNIT)), Some(PolyId(0)));
    assert_eq!(w.locate(rw(6 * UNIT - 1, 0, 4 * UNIT)), None);
    assert_eq!(w.locate(rw(6 * UNIT, 0, 4 * UNIT)), Some(PolyId(3)));
}

#[test]
fn locate_n_literals_and_y_ignored() {
    let n = mesh_n();
    let cases = [
        (4, 1, Some(0)),
        (4, 2, Some(0)),
        (4, 0, Some(0)),
        (6, 6, Some(1)),
        (8, 4, Some(1)), // corner of P1 and P2
        (7, 5, Some(1)), // on the shared slanted edge
        (8, 6, Some(2)),
        (9, 8, Some(2)),
        (3, 3, None),
        (10, 10, None),
    ];
    for (x, z, want) in cases {
        assert_eq!(locate_m(&n, x, z), want, "({x},{z})");
    }
    let w = mesh_w();
    assert_eq!(w.locate([m(2), m(99), m(2)]), Some(PolyId(0)));
    for y in [i32::MIN, -1, 1, 1 << 20, i32::MAX] {
        assert_eq!(w.locate(rw(2 * UNIT, y, 2 * UNIT)), Some(PolyId(0)));
        assert_eq!(w.locate(rw(5 * UNIT, y, 4 * UNIT)), None);
    }
    assert_eq!(w.locate(rw(i32::MIN, 0, i32::MAX)), None);
}

#[test]
fn locate_lowest_id_among_overlapping_polygons() {
    // Overlap is not detected; the lowest id answers (Reading 3).
    let sq = |x: i32, z: i32, s: i32| -> Vec<[Fx16_16; 3]> {
        vec![p(x, z), p(x + s, z), p(x + s, z + s), p(x, z + s)]
    };
    let mut v = sq(0, 0, 4);
    v.extend(sq(2, 2, 4));
    let a = NavMesh::new(v, vec![vec![0, 1, 2, 3], vec![4, 5, 6, 7]]).unwrap();
    assert_eq!(locate_m(&a, 3, 3), Some(0));
    assert_eq!(locate_m(&a, 5, 5), Some(1));
    assert_eq!(locate_m(&a, 2, 2), Some(0));
    assert_eq!(locate_m(&a, 6, 6), Some(1));
    assert_eq!(locate_m(&a, 5, 1), None);
    let mut v = sq(2, 2, 4);
    v.extend(sq(0, 0, 4));
    let b = NavMesh::new(v, vec![vec![0, 1, 2, 3], vec![4, 5, 6, 7]]).unwrap();
    assert_eq!(locate_m(&b, 3, 3), Some(0));
    assert_eq!(locate_m(&b, 1, 1), Some(1));
}

// ---------------------------------------------------------------- B-Q3 to B-Q5

#[test]
fn nearest_inside_keeps_x_z_and_takes_polygon_y() {
    let t = NavMesh::new(
        vec![rw(0, 131072, 0), rw(65536, 131072, 0), rw(0, 131072, 65536)],
        vec![vec![0, 1, 2]],
    )
    .unwrap();
    assert_eq!(raw3(t.nearest_on_mesh(rw(100, 5, 100))), [100, 131072, 100]);
    assert_eq!(raw3(t.nearest_on_mesh(rw(0, -9, 0))), [0, 131072, 0]);
    // Off the mesh: hypotenuse midpoint, exact (no rounding needed).
    assert_eq!(
        raw3(t.nearest_on_mesh(rw(65536, 7, 65536))),
        [32768, 131072, 32768]
    );
    // A baked mesh at height 2 m ignores the query height.
    let b = bake_box(m(2), rect(0, 0, 10, 10), &[]).unwrap();
    assert_eq!(b.nearest_on_mesh([m(3), m(99), m(4)]), [m(3), m(2), m(4)]);
    assert_eq!(
        raw3(b.nearest_on_mesh(rw(12345, i32::MIN, 54321))),
        [12345, i128::from(2 * UNIT), 54321]
    );
    // Boundary counts as inside.
    assert_eq!(
        b.nearest_on_mesh([m(10), m(0), m(10)]),
        [m(10), m(2), m(10)]
    );
}

#[test]
fn nearest_w_literals() {
    let w = mesh_w();
    let cases = [
        ((5, 4), (4, 4)), // tie with (6,4) in P3: lowest polygon wins
        ((5, -3), (4, 0)),
        ((12, 12), (10, 10)),
        ((-2, 5), (0, 5)),
        ((7, 9), (7, 9)),
    ];
    for ((x, z), (ex, ez)) in cases {
        let got = w.nearest_on_mesh([m(x), m(3), m(z)]);
        assert_eq!(got, [m(ex), m(0), m(ez)], "({x},{z})");
    }
    // Take the other side of the tie by one raw unit.
    assert_eq!(
        raw3(w.nearest_on_mesh(rw(5 * UNIT + 1, 3, 4 * UNIT))),
        [i128::from(6 * UNIT), 0, i128::from(4 * UNIT)]
    );
    assert_eq!(
        raw3(w.nearest_on_mesh(rw(5 * UNIT - 1, 3, 4 * UNIT))),
        [i128::from(4 * UNIT), 0, i128::from(4 * UNIT)]
    );
}

#[test]
fn nearest_n_literals() {
    let n = mesh_n();
    let cases = [
        ((10, 10), [589824, 0, 524288]),
        ((-3, 1), [0, 0, 65536]),
        ((5, -1), [262144, 0, 0]),
        ((9, 4), [528143, 0, 277564]), // lattice rule on a slanted edge
        ((2, 5), [131072, 0, 131072]),
    ];
    for ((x, z), want) in cases {
        assert_eq!(
            raw3(n.nearest_on_mesh([m(x), m(3), m(z)])),
            want,
            "({x},{z})"
        );
    }
}

#[test]
fn nearest_clamps_to_endpoints() {
    // t < 0 gives a, t > l2 gives b: a square 0..10.
    let sq = bake_box(m(0), rect(0, 0, 10, 10), &[]).unwrap();
    for (q, want) in [
        ((-5, -5), (0, 0)),
        ((15, -5), (10, 0)),
        ((15, 15), (10, 10)),
        ((-5, 15), (0, 10)),
        ((-5, 3), (0, 3)),
        ((5, 17), (5, 10)),
    ] {
        assert_eq!(sq.nearest_on_mesh(p(q.0, q.1)), p(want.0, want.1), "{q:?}");
    }
}

#[test]
fn nearest_across_polygons_first_strictly_smallest_and_height() {
    let sq = |x0: i32, y: i32| -> Vec<[Fx16_16; 3]> {
        vec![
            [m(x0), Fx16_16::from_raw(y), m(0)],
            [m(x0 + 2), Fx16_16::from_raw(y), m(0)],
            [m(x0 + 2), Fx16_16::from_raw(y), m(2)],
            [m(x0), Fx16_16::from_raw(y), m(2)],
        ]
    };
    let mut v = sq(0, 0);
    v.extend(sq(4, 5 * UNIT));
    let lo_first = NavMesh::new(v, vec![vec![0, 1, 2, 3], vec![4, 5, 6, 7]]).unwrap();
    let mut v = sq(4, 5 * UNIT);
    v.extend(sq(0, 0));
    let hi_first = NavMesh::new(v, vec![vec![0, 1, 2, 3], vec![4, 5, 6, 7]]).unwrap();
    // (3,1) is one metre from both squares: the lower id answers.
    assert_eq!(lo_first.nearest_on_mesh(p(3, 1)), [m(2), m(0), m(1)]);
    assert_eq!(hi_first.nearest_on_mesh(p(3, 1)), [m(4), m(5), m(1)]);
    // Clearly nearer to the second square in each mesh.
    assert_eq!(lo_first.nearest_on_mesh(p(7, 1)), [m(6), m(5), m(1)]);
    assert_eq!(hi_first.nearest_on_mesh(p(7, 1)), [m(6), m(5), m(1)]);
    assert_eq!(lo_first.nearest_on_mesh(p(-3, 1)), [m(0), m(0), m(1)]);
    assert_eq!(hi_first.nearest_on_mesh(p(-3, 1)), [m(0), m(0), m(1)]);
    // Inside the higher square: its own height.
    assert_eq!(lo_first.nearest_on_mesh(p(5, 1)), [m(5), m(5), m(1)]);
}

#[test]
fn nearest_fallback_and_edge_ties_on_unimodular_triangle() {
    // (0,0) (7,3) (2,1) holds no lattice point but its vertices, so every
    // interior edge candidate is uncontained and the fallback decides.
    let z = NavMesh::new(
        vec![rw(0, 777, 0), rw(7, 777, 3), rw(2, 777, 1)],
        vec![vec![0, 1, 2]],
    )
    .unwrap();
    // Equidistant (2465) from (0,0) and (7,3); (2,1) is at 2466. Edge 0
    // comes first and takes a on d2(a) <= d2(b); edge 1 ties with (7,3) but
    // is not strictly smaller.
    assert_eq!(raw3(z.nearest_on_mesh(rw(23, 0, -44))), [0, 777, 0]);
    // One raw unit over, (7,3) is strictly nearer.
    assert_eq!(raw3(z.nearest_on_mesh(rw(24, 0, -44))), [7, 777, 3]);
    // Inside the thin triangle only at its vertices.
    assert_eq!(raw3(z.nearest_on_mesh(rw(2, 5, 1))), [2, 777, 1]);
    // Every query on a 121 x 121 raw grid agrees with the model; the scan
    // reaches the fallback tie, so that mutant shows.
    let mut tally = [0u32; 7];
    let mp = polys_of(&z);
    for x in -60..=60 {
        for zq in -60..=60 {
            check(&z, &mp, [x, 0, zq], &mut tally);
        }
    }
    assert!(tally[2] > 0, "{tally:?}");
}

// ---------------------------------------------------------------- B-Q1, B-5

#[test]
fn nearest_random_triangles_match_model() {
    let mut rng = DeterministicRng::from_seed(5);
    let mut tally = [0u32; 7];
    for _ in 0..20_000 {
        let s = rng.stream("nav");
        let mut v: Vec<[i32; 2]> = (0..3)
            .map(|_| [s.gen_range_i32(-50, 50), s.gen_range_i32(-50, 50)])
            .collect();
        let c = (i64::from(v[1][0]) - i64::from(v[0][0]))
            * (i64::from(v[2][1]) - i64::from(v[0][1]))
            - (i64::from(v[1][1]) - i64::from(v[0][1])) * (i64::from(v[2][0]) - i64::from(v[0][0]));
        if c == 0 {
            continue;
        }
        if c < 0 {
            v.swap(1, 2);
        }
        let y = s.gen_range_i32(-1000, 1000);
        let mesh = NavMesh::new(
            v.iter().map(|q| rw(q[0], y, q[1])).collect(),
            vec![vec![0, 1, 2]],
        )
        .unwrap();
        let mp = polys_of(&mesh);
        for _ in 0..6 {
            let q = [
                s.gen_range_i32(-200, 200),
                s.gen_range_i32(-100, 100),
                s.gen_range_i32(-200, 200),
            ];
            check(&mesh, &mp, q, &mut tally);
        }
    }
    assert_all_mutants_seen(&tally);
}

#[test]
fn nearest_random_queries_on_multi_polygon_meshes_match_model() {
    let mut rng = DeterministicRng::from_seed(77);
    let mut tally = [0u32; 7];
    // Baked meshes: integer geometry makes boundary hits and ties common.
    for _ in 0..400 {
        let (y, outer, holes) = random_bake(&mut rng);
        let Ok(mesh) = bake_raw(y, outer, &holes) else {
            continue;
        };
        let mp = polys_of(&mesh);
        let s = rng.stream("q");
        for _ in 0..40 {
            let q = [
                s.gen_range_i32(outer[0] - 4, outer[2] + 4),
                s.gen_range_i32(-3, 3),
                s.gen_range_i32(outer[1] - 4, outer[3] + 4),
            ];
            check(&mesh, &mp, q, &mut tally);
        }
    }
    // Hand-built meshes at metre scale with raw-resolution queries.
    let pentagon = NavMesh::new(
        [(0, 0), (4, 0), (5, 3), (2, 5), (-1, 3)]
            .iter()
            .map(|&(x, z)| p(x, z))
            .collect(),
        vec![vec![0, 1, 2, 3, 4]],
    )
    .unwrap();
    for mesh in [mesh_n(), mesh_w(), pentagon] {
        let mp = polys_of(&mesh);
        let s = rng.stream("q2");
        for _ in 0..3000 {
            let q = [
                s.gen_range_i32(-4 * UNIT, 14 * UNIT),
                s.gen_range_i32(-100, 100),
                s.gen_range_i32(-4 * UNIT, 14 * UNIT),
            ];
            check(&mesh, &mp, q, &mut tally);
        }
        // And on whole-metre points, where ties are everywhere.
        for x in -4..=14 {
            for zq in -4..=14 {
                check(&mesh, &mp, [x * UNIT, 0, zq * UNIT], &mut tally);
            }
        }
    }
    assert_all_mutants_seen(&tally);
}

#[test]
fn extreme_raw_inputs_never_panic_and_match_model() {
    let ext = [
        i32::MIN,
        i32::MIN + 1,
        -LIM,
        -1,
        0,
        1,
        LIM,
        i32::MAX - 1,
        i32::MAX,
    ];
    let big = bake_raw(LIM, [-LIM, -LIM, LIM, LIM], &[[-1000, -1000, 1000, 1000]]).unwrap();
    let tri = NavMesh::new(
        vec![
            rw(-LIM, -LIM, -LIM),
            rw(LIM, -LIM, -LIM),
            rw(LIM, -LIM, LIM),
        ],
        vec![vec![0, 1, 2]],
    )
    .unwrap();
    let tiny = NavMesh::new(
        vec![
            rw(-LIM, 3, LIM - 1),
            rw(-LIM + 1, 3, LIM - 1),
            rw(-LIM, 3, LIM),
        ],
        vec![vec![0, 1, 2]],
    )
    .unwrap();
    let mut tally = [0u32; 7];
    for mesh in [mesh_w(), mesh_n(), big, tri, tiny] {
        let mp = polys_of(&mesh);
        for &x in &ext {
            for &z in &ext {
                for y in [i32::MIN, 0, i32::MAX] {
                    check(&mesh, &mp, [x, y, z], &mut tally);
                }
            }
        }
    }
    // The contract's second property: 2000 full-range queries on N.
    let n = mesh_n();
    let mp = polys_of(&n);
    let mut rng = DeterministicRng::from_seed(5);
    let s = rng.stream("nav");
    for _ in 0..2000 {
        let q = [
            s.gen_range_i32(i32::MIN, i32::MAX),
            s.gen_range_i32(i32::MIN, i32::MAX),
            s.gen_range_i32(i32::MIN, i32::MAX),
        ];
        check(&n, &mp, q, &mut tally);
    }
    // Literal anchors at the extremes.
    let w = mesh_w();
    assert_eq!(w.locate(rw(i32::MIN, 0, i32::MIN)), None);
    assert_eq!(
        raw3(w.nearest_on_mesh(rw(i32::MAX, 5, i32::MAX))),
        [i128::from(10 * UNIT), 0, i128::from(10 * UNIT)]
    );
    assert_eq!(
        raw3(w.nearest_on_mesh(rw(i32::MIN, 5, i32::MIN))),
        [0, 0, 0]
    );
}

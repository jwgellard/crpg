//! T035c two-keys suite, written from `tasks/T035.md` alone (B-P1 to B-P8,
//! C-1 to C-9, Errors). Integers only.
//!
//! Besides the contract's literals, this file holds an independent reference
//! model of B-P1 to B-P8 (written from the contract text, never from the
//! implementation) and compares it with `find_path` over seeded meshes. The
//! model also runs "mutant" variants (a key dropped or reversed, a non-strict
//! cost update, ...); the corpus must tell every mutant from the real rules,
//! which proves it pins each tie-break.

use std::collections::{BTreeMap, BTreeSet};

use crpg_core::DeterministicRng;
use crpg_nav::{
    bake_box, seg_len, BakeError, Fx16_16, NavError, NavMesh, NavPath, NavRect, PolyId,
    COORD_LIMIT_RAW, MAX_PATH_LENGTH_RAW, MAX_PATH_POINTS,
};

type P3 = [Fx16_16; 3];
type Pair = (i32, i32);
type Builder = fn() -> NavMesh;

// ---------------------------------------------------------------- fixtures

fn m(v: i32) -> Fx16_16 {
    Fx16_16::from_int(v)
}
fn p(x: i32, z: i32) -> P3 {
    [m(x), m(0), m(z)]
}
fn rw(x: i32, y: i32, z: i32) -> P3 {
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
fn rect_raw(x0: i32, z0: i32, x1: i32, z1: i32) -> NavRect {
    NavRect {
        min_x: Fx16_16::from_raw(x0),
        min_z: Fx16_16::from_raw(z0),
        max_x: Fx16_16::from_raw(x1),
        max_z: Fx16_16::from_raw(z1),
    }
}
fn pts(list: &[(i32, i32)]) -> Vec<P3> {
    list.iter().map(|&(x, z)| p(x, z)).collect()
}
fn wall() -> NavMesh {
    bake_box(m(0), rect(0, 0, 10, 10), &[rect(4, 0, 6, 8)]).unwrap()
}
fn ring() -> NavMesh {
    bake_box(m(0), rect(0, 0, 10, 10), &[rect(4, 4, 6, 6)]).unwrap()
}
fn split() -> NavMesh {
    bake_box(m(0), rect(0, 0, 10, 10), &[rect(4, 0, 6, 10)]).unwrap()
}
fn long_mesh() -> NavMesh {
    bake_box(m(0), rect(0, 0, 5000, 10), &[]).unwrap()
}
fn comb(k: i32, step: i32, width: i32) -> NavMesh {
    let holes: Vec<NavRect> = (0..k)
        .map(|i| {
            let x0 = 2 + step * i;
            if i % 2 == 0 {
                rect(x0, 0, x0 + 2, 16)
            } else {
                rect(x0, 4, x0 + 2, 20)
            }
        })
        .collect();
    bake_box(m(0), rect(0, 0, width, 20), &holes).unwrap()
}
fn comb_expected(k: i32, step: i32, goal_x: i32) -> Vec<P3> {
    let mut v = vec![p(1, 10)];
    for i in 0..k {
        let x0 = 2 + step * i;
        let z = if i % 2 == 0 { 16 } else { 4 };
        v.push(p(x0, z));
        v.push(p(x0 + 2, z));
    }
    v.push(p(goal_x, 10));
    v
}
fn nmesh() -> NavMesh {
    NavMesh::new(
        vec![
            p(0, 0),
            p(4, 0),
            p(4, 2),
            p(0, 2),
            p(8, 4),
            p(6, 6),
            p(9, 8),
        ],
        vec![vec![0, 1, 2, 3], vec![1, 4, 5, 2], vec![5, 4, 6]],
    )
    .unwrap()
}
fn find(mesh: &NavMesh, a: P3, b: P3) -> Result<NavPath, NavError> {
    mesh.find_path(a, b)
}
fn ok_points(mesh: &NavMesh, a: P3, b: P3) -> Vec<P3> {
    find(mesh, a, b).unwrap().into_points()
}
fn sum_len(points: &[P3]) -> u64 {
    points.windows(2).map(|w| seg_len(w[0], w[1])).sum()
}
fn flat(r: Result<NavPath, NavError>) -> Result<(Vec<P3>, u64), NavError> {
    r.map(|path| (path.points().to_vec(), path.length()))
}

// ---------------------------------------------------- reference model (B-P*)

fn same(a: P3, b: P3) -> bool {
    a[0] == b[0] && a[2] == b[2]
}
fn cr(o: P3, a: P3, b: P3) -> i128 {
    let r = |v: P3, i: usize| i128::from(v[i].to_raw());
    (r(a, 0) - r(o, 0)) * (r(b, 2) - r(o, 2)) - (r(a, 2) - r(o, 2)) * (r(b, 0) - r(o, 0))
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum V {
    Full,
    NoH,
    RevH,
    RevId,
    NonStrict,
    CentStart,
    CentGoal,
    SwapLr,
}
const MUTANTS: [V; 7] = [
    V::NoH,
    V::RevH,
    V::RevId,
    V::NonStrict,
    V::CentStart,
    V::CentGoal,
    V::SwapLr,
];

fn ref_locate(mesh: &NavMesh, q: P3) -> Option<u32> {
    let verts = mesh.vertices();
    (0..mesh.polygon_count() as u32).find(|&id| {
        let poly = mesh.polygon(PolyId(id)).unwrap();
        let n = poly.len();
        (0..n).all(|i| {
            cr(
                verts[poly[i] as usize],
                verts[poly[(i + 1) % n] as usize],
                q,
            ) >= 0
        })
    })
}

#[allow(unused_assignments)]
fn funnel(portals: &[(P3, P3)]) -> Vec<P3> {
    let (first_l, first_r) = portals[0];
    let mut apex = first_l;
    let mut left = first_l;
    let mut right = first_r;
    let (mut left_i, mut right_i, mut apex_i) = (0usize, 0usize, 0usize);
    let mut raw = vec![apex];
    let mut i = 1;
    while i < portals.len() {
        let (l, r) = portals[i];
        if cr(apex, right, r) >= 0 {
            if same(apex, right) || cr(apex, left, r) < 0 {
                right = r;
                right_i = i;
            } else {
                raw.push(left);
                apex = left;
                apex_i = left_i;
                left = apex;
                right = apex;
                left_i = apex_i;
                right_i = apex_i;
                i = apex_i + 1;
                continue;
            }
        }
        if cr(apex, left, l) <= 0 {
            if same(apex, left) || cr(apex, right, l) > 0 {
                left = l;
                left_i = i;
            } else {
                raw.push(right);
                apex = right;
                apex_i = right_i;
                left = apex;
                right = apex;
                left_i = apex_i;
                right_i = apex_i;
                i = apex_i + 1;
                continue;
            }
        }
        i += 1;
    }
    let (last_l, _) = portals[portals.len() - 1];
    raw.push(last_l);
    raw
}

fn ref_find(mesh: &NavMesh, from: P3, to: P3, v: V) -> Result<(Vec<P3>, u64), NavError> {
    let s = ref_locate(mesh, from).ok_or(NavError::StartOffMesh)?;
    let g = ref_locate(mesh, to).ok_or(NavError::GoalOffMesh)?;
    if same(from, to) {
        return Err(NavError::AlreadyThere);
    }
    let goal = [to[0], mesh.polygon_y(PolyId(g)).unwrap(), to[2]];
    let verts = mesh.vertices();
    let centroid = |id: u32| -> P3 {
        let poly = mesh.polygon(PolyId(id)).unwrap();
        let n = poly.len() as i64;
        let (mut sx, mut sz) = (0i64, 0i64);
        for &vi in poly {
            sx += i64::from(verts[vi as usize][0].to_raw());
            sz += i64::from(verts[vi as usize][2].to_raw());
        }
        rw(sx.div_euclid(n) as i32, 0, sz.div_euclid(n) as i32)
    };
    let pt = |id: u32| -> P3 {
        if id == s && v != V::CentStart {
            from
        } else if id == g && v != V::CentGoal {
            goal
        } else {
            centroid(id)
        }
    };
    let h = |id: u32| seg_len(pt(id), goal);
    let key = |c: u64, id: u32| -> ((u64, u64, u32), u32) {
        let f = c + h(id);
        let k = match v {
            V::NoH => (f, 0, id),
            V::RevH => (f, u64::MAX - h(id), id),
            V::RevId => (f, h(id), u32::MAX - id),
            _ => (f, h(id), id),
        };
        (k, id)
    };
    let corridor: Vec<u32> = if s == g {
        vec![s]
    } else {
        let mut cost: BTreeMap<u32, u64> = BTreeMap::new();
        let mut parent: BTreeMap<u32, u32> = BTreeMap::new();
        let mut closed: BTreeSet<u32> = BTreeSet::new();
        let mut open = BTreeSet::new();
        cost.insert(s, 0);
        open.insert(key(0, s));
        let mut found = false;
        while let Some((_, pid)) = open.pop_first() {
            closed.insert(pid);
            if pid == g {
                found = true;
                break;
            }
            let nb = mesh.neighbours(PolyId(pid)).unwrap();
            // neighbour visiting order is unobservable: each Q is updated once per P
            for &entry in nb {
                let Some(PolyId(q)) = entry else { continue };
                if closed.contains(&q) {
                    continue;
                }
                let c = cost[&pid] + seg_len(pt(pid), pt(q));
                let old = cost.get(&q).copied();
                let better = match old {
                    None => true,
                    Some(o) => {
                        if v == V::NonStrict {
                            c <= o
                        } else {
                            c < o
                        }
                    }
                };
                if better {
                    if let Some(o) = old {
                        open.remove(&key(o, q));
                    }
                    cost.insert(q, c);
                    parent.insert(q, pid);
                    open.insert(key(c, q));
                }
            }
        }
        if !found {
            return Err(NavError::Unreachable);
        }
        let mut c = vec![g];
        while let Some(&par) = parent.get(c.last().unwrap()) {
            c.push(par);
        }
        c.reverse();
        c
    };
    let mut portals: Vec<(P3, P3)> = vec![(from, from)];
    for w in corridor.windows(2) {
        let (a_id, b_id) = (w[0], w[1]);
        let poly = mesh.polygon(PolyId(a_id)).unwrap();
        let nb = mesh.neighbours(PolyId(a_id)).unwrap();
        let e = nb.iter().position(|&x| x == Some(PolyId(b_id))).unwrap();
        let a = verts[poly[e] as usize];
        let b = verts[poly[(e + 1) % poly.len()] as usize];
        portals.push(if v == V::SwapLr { (a, b) } else { (b, a) });
    }
    portals.push((goal, goal));
    let mut points = Vec::new();
    for q in funnel(&portals) {
        if points.last().is_none_or(|&l| !same(l, q)) {
            points.push(q);
        }
    }
    if points.len() > MAX_PATH_POINTS {
        return Err(NavError::PathTooComplex {
            points: points.len(),
        });
    }
    let length = sum_len(&points);
    if length > MAX_PATH_LENGTH_RAW {
        return Err(NavError::TooLong { length });
    }
    Ok((points, length))
}

// ------------------------------------------------------------ literal suites

#[test]
fn c1_wall_literal_both_directions() {
    let w = wall();
    let r = find(&w, p(2, 2), p(8, 2)).unwrap();
    assert_eq!(r.points(), &pts(&[(2, 2), (4, 8), (6, 8), (8, 2)])[..]);
    assert_eq!(r.length(), 960046);
    let back = find(&w, p(8, 2), p(2, 2)).unwrap();
    assert_eq!(back.points(), &pts(&[(8, 2), (6, 8), (4, 8), (2, 2)])[..]);
    assert_eq!(back.length(), 960046);
    assert_eq!(r.clone().into_points(), r.points().to_vec());
}

#[test]
fn c2_ring_tie_break() {
    let r = find(&ring(), p(5, 1), p(5, 9)).unwrap();
    assert_eq!(r.points(), &pts(&[(5, 1), (4, 4), (4, 6), (5, 9)])[..]);
    assert_eq!(r.length(), 545560);
}

#[test]
fn c3_non_axis_literals() {
    let n = nmesh();
    let cases: [(Pair, Pair, Vec<Pair>, u64); 3] = [
        ((1, 1), (8, 7), vec![(1, 1), (4, 2), (8, 7)], 626880),
        ((1, 1), (5, 3), vec![(1, 1), (4, 2), (5, 3)], 299926),
        ((0, 2), (9, 8), vec![(0, 2), (4, 2), (9, 8)], 773997),
    ];
    for (a, b, expect, len) in cases {
        let r = find(&n, p(a.0, a.1), p(b.0, b.1)).unwrap();
        assert_eq!(r.points(), &pts(&expect)[..], "{a:?} -> {b:?}");
        assert_eq!(r.length(), len, "{a:?} -> {b:?}");
    }
}

#[test]
fn c4_same_polygon_and_y_rules() {
    let w = wall();
    assert_eq!(ok_points(&w, p(1, 1), p(3, 3)), pts(&[(1, 1), (3, 3)]));
    // same polygon at y = 2: first point is `from` exactly, last takes the polygon y
    let w2 = bake_box(m(2), rect(0, 0, 10, 10), &[rect(4, 0, 6, 8)]).unwrap();
    let r = ok_points(&w2, [m(1), m(-5), m(1)], [m(3), m(9), m(3)]);
    assert_eq!(r, vec![[m(1), m(-5), m(1)], [m(3), m(2), m(3)]]);
    // one raw unit apart is not AlreadyThere
    let a = rw(2 * 65536, 0, 2 * 65536);
    let b = rw(2 * 65536 + 1, 0, 2 * 65536);
    let r = find(&w, a, b).unwrap();
    assert_eq!(r.points(), &[a, b][..]);
    assert_eq!(r.length(), 1);
    // extreme y on `from` is carried, never compared
    let a = rw(65536, i32::MIN, 65536);
    let r = ok_points(&w, a, rw(3 * 65536, i32::MAX, 3 * 65536));
    assert_eq!(r, vec![a, p(3, 3)]);
}

#[test]
fn c5_heights() {
    let w2 = bake_box(m(2), rect(0, 0, 10, 10), &[rect(4, 0, 6, 8)]).unwrap();
    let r = ok_points(&w2, [m(2), m(-1), m(2)], [m(8), m(7), m(2)]);
    let e = |x: i32, y: i32, z: i32| [m(x), m(y), m(z)];
    assert_eq!(r, vec![e(2, -1, 2), e(4, 2, 8), e(6, 2, 8), e(8, 2, 2)]);
}

#[test]
fn c6_errors_in_order() {
    let w = wall();
    assert_eq!(find(&w, p(-1, 5), p(11, 5)), Err(NavError::StartOffMesh));
    assert_eq!(find(&w, p(2, 5), p(11, 5)), Err(NavError::GoalOffMesh));
    assert_eq!(
        find(&w, p(2, 5), [m(2), m(3), m(5)]),
        Err(NavError::AlreadyThere)
    );
    assert_eq!(find(&w, p(5, 4), p(2, 2)), Err(NavError::StartOffMesh));
    // start off the mesh and equal to the goal: the start check comes first
    assert_eq!(find(&w, p(5, 4), p(5, 4)), Err(NavError::StartOffMesh));
    // goal off the mesh, start on it, goal is the hole: GoalOffMesh
    assert_eq!(find(&w, p(2, 2), p(5, 4)), Err(NavError::GoalOffMesh));
    // controls: connected parts give Ok, split parts give Unreachable
    assert!(find(&w, p(2, 5), p(8, 5)).is_ok());
    let u = split();
    assert_eq!(find(&u, p(2, 5), p(8, 5)), Err(NavError::Unreachable));
    assert_eq!(find(&u, p(4, 5), p(6, 5)), Err(NavError::Unreachable));
    assert_eq!(find(&u, p(8, 5), p(2, 5)), Err(NavError::Unreachable));
    assert!(find(&u, p(2, 5), p(3, 5)).is_ok());
    assert_eq!(find(&u, p(2, 5), p(2, 5)), Err(NavError::AlreadyThere));
    assert_eq!(find(&u, p(5, 5), p(8, 5)), Err(NavError::StartOffMesh));
    assert_eq!(find(&u, p(2, 5), p(5, 5)), Err(NavError::GoalOffMesh));
}

#[test]
fn display_texts_and_error_trait() {
    let table: [(NavError, &str); 6] = [
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
            NavError::PathTooComplex { points: 66 },
            "path needs 66 points, more than 64",
        ),
        (
            NavError::TooLong { length: 294912000 },
            "path length 294912000 exceeds 268435456",
        ),
    ];
    for (e, text) in table {
        assert_eq!(e.to_string(), text);
        let dynamic: &dyn std::error::Error = &e;
        assert_eq!(dynamic.to_string(), text);
    }
    let _ = BakeError::NoWalkableArea;
}

#[test]
fn c7_limits() {
    // 64 points exactly is Ok
    let c31 = comb(31, 6, 200);
    let r = find(&c31, p(1, 10), p(199, 10)).unwrap();
    assert_eq!(r.points().len(), MAX_PATH_POINTS);
    assert_eq!(r.points(), &comb_expected(31, 6, 199)[..]);
    assert_eq!(r.length(), sum_len(r.points()));
    // 66 points is PathTooComplex with the exact count
    let c32 = comb(32, 6, 200);
    assert_eq!(
        find(&c32, p(1, 10), p(199, 10)),
        Err(NavError::PathTooComplex { points: 66 })
    );
    let l = long_mesh();
    let r = find(&l, p(0, 5), p(4096, 5)).unwrap();
    assert_eq!(r.points().len(), 2);
    assert_eq!(r.length(), 268435456);
    assert_eq!(
        find(&l, p(0, 5), p(4500, 5)),
        Err(NavError::TooLong { length: 294912000 })
    );
    // one raw unit either side of the boundary
    let a = rw(0, 0, 5 * 65536);
    let at = rw(4096 * 65536, 0, 5 * 65536);
    let past = rw(4096 * 65536 + 1, 0, 5 * 65536);
    let before = rw(4096 * 65536 - 1, 0, 5 * 65536);
    assert_eq!(find(&l, a, at).unwrap().length(), MAX_PATH_LENGTH_RAW);
    assert_eq!(
        find(&l, a, past),
        Err(NavError::TooLong {
            length: MAX_PATH_LENGTH_RAW + 1
        })
    );
    assert_eq!(
        find(&l, a, before).unwrap().length(),
        MAX_PATH_LENGTH_RAW - 1
    );
    // PathTooComplex is checked before TooLong (66 points and over 4,096 m)
    let wide = comb(32, 200, 6404);
    assert_eq!(
        find(&wide, p(1, 10), p(6403, 10)),
        Err(NavError::PathTooComplex { points: 66 })
    );
    // 64 points and too long: TooLong carries the exact summed length
    let wide31 = comb(31, 200, 6404);
    let expect = comb_expected(31, 200, 6403);
    assert_eq!(
        find(&wide31, p(1, 10), p(6403, 10)),
        Err(NavError::TooLong {
            length: sum_len(&expect)
        })
    );
    assert!(sum_len(&expect) > MAX_PATH_LENGTH_RAW);
}

#[test]
fn shared_edges_use_the_lowest_edge_index() {
    // A has the collinear vertices (0,0) (1,0) (2,0); B below holds both edges reversed, so
    // A's edges 0 and 1 both lead to B and edge 0 is the portal.
    let verts = pts(&[(0, 0), (1, 0), (2, 0), (2, 2), (0, 2), (0, -2), (2, -2)]);
    let mesh = NavMesh::new(verts, vec![vec![0, 1, 2, 3, 4], vec![2, 1, 0, 5, 6]]).unwrap();
    let n0 = mesh.neighbours(PolyId(0)).unwrap();
    assert_eq!(n0[0], Some(PolyId(1)));
    assert_eq!(n0[1], Some(PolyId(1)));
    let r = find(&mesh, p(2, 1), p(2, -1)).unwrap();
    assert_eq!(r.points(), &pts(&[(2, 1), (1, 0), (2, -1)])[..]);
    assert_eq!(r.length(), 185364);
    // the reverse trip uses B's edge 0 (c to b): same result mirrored by the model
    assert_eq!(
        flat(find(&mesh, p(0, -1), p(0, 1))),
        ref_find(&mesh, p(0, -1), p(0, 1), V::Full)
    );
}

#[test]
fn extreme_inputs_never_panic() {
    let vals = [i32::MIN, i32::MAX, 0, 5 * 65536];
    let mut probes = Vec::new();
    for &x in &vals {
        for &y in &vals {
            for &z in &vals {
                probes.push(rw(x, y, z));
            }
        }
    }
    for mesh in [wall(), ring(), nmesh(), split(), long_mesh()] {
        for &a in &probes {
            for &b in &probes {
                assert_eq!(flat(find(&mesh, a, b)), ref_find(&mesh, a, b, V::Full));
            }
        }
    }
    // a mesh as wide as the coordinate limit
    let lim = COORD_LIMIT_RAW;
    let big = bake_box(m(0), rect_raw(-lim, -lim, lim, lim), &[]).unwrap();
    assert_eq!(
        find(&big, rw(-lim, 0, -lim), rw(lim, 0, lim)),
        Err(NavError::TooLong {
            length: 1_518_500_250
        })
    );
    let holed = bake_box(
        m(0),
        rect_raw(-lim, -lim, lim, lim),
        &[rect_raw(-lim / 2, -lim / 2, lim / 2, lim / 2)],
    )
    .unwrap();
    for (a, b) in [
        (rw(-lim, 0, -lim), rw(lim, 0, lim)),
        (rw(-lim, 0, 0), rw(lim, 0, 0)),
        (rw(0, 0, lim), rw(lim, 0, -lim)),
        (rw(lim, 0, lim), rw(-lim, 0, lim)),
    ] {
        assert_eq!(flat(find(&holed, a, b)), ref_find(&holed, a, b, V::Full));
        assert!(matches!(
            find(&holed, a, b),
            Err(NavError::TooLong { .. }) | Ok(_)
        ));
    }
}

// ---------------------------------------------------- seeded corpus (C-8, C-9)

struct Case {
    unit: i32,
    w: i32,
    holes: Vec<[i32; 4]>,
    mesh: NavMesh,
    queries: Vec<(P3, P3)>,
}

fn bake_case(unit: i32, w: i32, holes: &[[i32; 4]]) -> Result<NavMesh, BakeError> {
    let rects: Vec<NavRect> = holes
        .iter()
        .map(|h| rect_raw(h[0], h[1], h[2], h[3]))
        .collect();
    bake_box(m(0), rect_raw(0, 0, w * unit, w * unit), &rects)
}

fn corpus() -> Vec<Case> {
    let mut rng = DeterministicRng::from_seed(35);
    let st = rng.stream("nav");
    let mut cases = Vec::new();
    for _ in 0..200 {
        let unit = [1, 1000, 65536][st.gen_range_i32(0, 2) as usize];
        let w = st.gen_range_i32(3, 30);
        let nh = st.gen_range_i32(0, 8);
        let mut holes = Vec::new();
        for _ in 0..nh {
            let x0 = st.gen_range_i32(0, w - 1);
            let x1 = st.gen_range_i32(x0 + 1, w);
            let z0 = st.gen_range_i32(0, w - 1);
            let z1 = st.gen_range_i32(z0 + 1, w);
            holes.push([x0 * unit, z0 * unit, x1 * unit, z1 * unit]);
        }
        let Ok(mesh) = bake_case(unit, w, &holes) else {
            continue;
        };
        let mut queries = Vec::new();
        for _ in 0..10 {
            let mut snap = || {
                let x = st.gen_range_i32(-unit, (w + 1) * unit);
                let z = st.gen_range_i32(-unit, (w + 1) * unit);
                mesh.nearest_on_mesh(rw(x, 0, z))
            };
            let a = snap();
            let b = snap();
            queries.push((a, b));
        }
        cases.push(Case {
            unit,
            w,
            holes,
            mesh,
            queries,
        });
    }
    cases
}

/// The contract's V5 oracle for one segment against one hole (all `i128`).
fn clear_of_hole(q: P3, r: P3, h: &[i32; 4]) -> bool {
    let (x0, z0, x1, z1) = (h[0], h[1], h[2], h[3]);
    let (qx, qz) = (q[0].to_raw(), q[2].to_raw());
    let (rx, rz) = (r[0].to_raw(), r[2].to_raw());
    if qx.max(rx) <= x0 || qx.min(rx) >= x1 || qz.max(rz) <= z0 || qz.min(rz) >= z1 {
        return true;
    }
    let cs: Vec<i128> = [(x0, z0), (x1, z0), (x1, z1), (x0, z1)]
        .iter()
        .map(|&(cx, cz)| cr(q, r, rw(cx, 0, cz)))
        .collect();
    cs.iter().all(|&c| c >= 0) || cs.iter().all(|&c| c <= 0)
}

#[test]
fn c8_inside_the_mesh_and_matches_the_model() {
    let cases = corpus();
    assert!(cases.len() >= 150, "{} meshes", cases.len());
    let mut oks = 0;
    let mut kills = [false; MUTANTS.len()];
    for case in &cases {
        let hi = case.w * case.unit;
        for &(a, b) in &case.queries {
            let got = flat(find(&case.mesh, a, b));
            assert_eq!(got, ref_find(&case.mesh, a, b, V::Full), "{a:?} {b:?}");
            for (k, &mutant) in MUTANTS.iter().enumerate() {
                if ref_find(&case.mesh, a, b, mutant) != got {
                    kills[k] = true;
                }
            }
            match got {
                Err(NavError::Unreachable) | Err(NavError::AlreadyThere) => {}
                Err(other) => panic!("unexpected {other:?}"),
                Ok((points, length)) => {
                    oks += 1;
                    assert!(points.len() >= 2);
                    assert_eq!(points[0], a);
                    assert!(same(points[points.len() - 1], b));
                    assert_eq!(length, sum_len(&points));
                    for q in &points {
                        let (x, z) = (q[0].to_raw(), q[2].to_raw());
                        assert!((0..=hi).contains(&x) && (0..=hi).contains(&z));
                    }
                    for seg in points.windows(2) {
                        assert!(!same(seg[0], seg[1]));
                        for h in &case.holes {
                            assert!(
                                clear_of_hole(seg[0], seg[1], h),
                                "segment {:?}->{:?} enters hole {h:?}",
                                seg[0],
                                seg[1]
                            );
                        }
                    }
                }
            }
        }
    }
    assert!(oks >= 1000, "only {oks} Ok queries");
    for (k, &killed) in kills.iter().enumerate() {
        assert!(killed, "corpus does not separate mutant {:?}", MUTANTS[k]);
    }
}

#[test]
fn non_axis_mesh_matches_the_model() {
    let n = nmesh();
    let mut rng = DeterministicRng::from_seed(77);
    let st = rng.stream("nav");
    let mut oks = 0;
    for i in 0..600 {
        let mut pick = || {
            let q = rw(
                st.gen_range_i32(-65536, 10 * 65536),
                st.gen_range_i32(-3, 3),
                st.gen_range_i32(-65536, 9 * 65536),
            );
            if i % 2 == 0 {
                n.nearest_on_mesh(q)
            } else {
                q
            }
        };
        let (a, b) = (pick(), pick());
        let got = flat(find(&n, a, b));
        assert_eq!(got, ref_find(&n, a, b, V::Full), "{a:?} {b:?}");
        if let Ok((points, _)) = got {
            oks += 1;
            assert_eq!(points[0], a);
        }
    }
    assert!(oks > 100);
}

#[test]
fn c9_deterministic() {
    // every literal query: twice on the same mesh, once on a fresh equal mesh
    let queries: Vec<(Builder, P3, P3)> = vec![
        (wall, p(2, 2), p(8, 2)),
        (wall, p(8, 2), p(2, 2)),
        (wall, p(1, 1), p(3, 3)),
        (wall, p(-1, 5), p(11, 5)),
        (wall, p(2, 5), p(11, 5)),
        (ring, p(5, 1), p(5, 9)),
        (nmesh, p(1, 1), p(8, 7)),
        (nmesh, p(0, 2), p(9, 8)),
        (split, p(2, 5), p(8, 5)),
        (long_mesh, p(0, 5), p(4500, 5)),
        (long_mesh, p(0, 5), p(4096, 5)),
    ];
    for (build, a, b) in queries {
        let mesh = build();
        let first = find(&mesh, a, b);
        assert_eq!(first, find(&mesh, a, b));
        assert_eq!(first, find(&build(), a, b));
    }
    for (build, a, b) in [
        (
            (|| comb(31, 6, 200)) as fn() -> NavMesh,
            p(1, 10),
            p(199, 10),
        ),
        (
            (|| comb(32, 6, 200)) as fn() -> NavMesh,
            p(1, 10),
            p(199, 10),
        ),
    ] {
        let mesh = build();
        assert_eq!(find(&mesh, a, b), find(&mesh, a, b));
        assert_eq!(find(&mesh, a, b), find(&build(), a, b));
    }
    for case in corpus().iter().take(60) {
        let fresh = bake_case(case.unit, case.w, &case.holes).unwrap();
        for &(a, b) in &case.queries {
            let first = find(&case.mesh, a, b);
            assert_eq!(first, find(&case.mesh, a, b));
            assert_eq!(first, find(&fresh, a, b));
        }
    }
}

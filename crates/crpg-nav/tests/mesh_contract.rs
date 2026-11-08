//! T035a two-keys suite, written from `tasks/T035.md` alone (B-G1, B-M1 to
//! B-M10, B-C1, B-C2, Errors). Integers only.

use crpg_nav::{
    ceil_sqrt, seg_len, Fx16_16, NavDecodeError, NavMesh, NavMeshError, PolyId, COORD_LIMIT_RAW,
    MAX_NAV_POLYGONS, MAX_NAV_VERTICES, MAX_PATH_LENGTH_RAW, MAX_PATH_POINTS, MAX_POLYGON_VERTICES,
    NAV_FORMAT_VERSION, NAV_MAGIC,
};

const LIM: i32 = 1 << 29;

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

fn pv(points: &[(i32, i32)]) -> Vec<[Fx16_16; 3]> {
    points.iter().map(|&(x, z)| p(x, z)).collect()
}

fn build(vertices: Vec<[Fx16_16; 3]>, polygons: Vec<Vec<u32>>) -> Result<NavMesh, NavMeshError> {
    NavMesh::new(vertices, polygons)
}

fn err(vertices: Vec<[Fx16_16; 3]>, polygons: Vec<Vec<u32>>) -> NavMeshError {
    build(vertices, polygons).unwrap_err()
}

fn ok(vertices: Vec<[Fx16_16; 3]>, polygons: Vec<Vec<u32>>) -> NavMesh {
    build(vertices, polygons).unwrap()
}

/// The unit right triangle `p(0,0) p(1,0) p(0,1)`.
fn tri_vertices() -> Vec<[Fx16_16; 3]> {
    pv(&[(0, 0), (1, 0), (0, 1)])
}

/// Mesh T of the contract: a raw-coordinate triangle at height 131072.
fn mesh_t() -> NavMesh {
    ok(
        vec![rw(0, 131072, 0), rw(65536, 131072, 0), rw(0, 131072, 65536)],
        vec![vec![0, 1, 2]],
    )
}

/// Mesh N of the contract.
fn mesh_n() -> NavMesh {
    ok(
        pv(&[(0, 0), (4, 0), (4, 2), (0, 2), (8, 4), (6, 6), (9, 8)]),
        vec![vec![0, 1, 2, 3], vec![1, 4, 5, 2], vec![5, 4, 6]],
    )
}

/// Mesh W of the contract, built by hand.
fn mesh_w() -> NavMesh {
    ok(
        pv(&[
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
        ]),
        vec![
            vec![0, 1, 2, 3],
            vec![3, 2, 4, 5],
            vec![2, 6, 7, 4],
            vec![8, 9, 10, 6],
            vec![6, 10, 11, 7],
        ],
    )
}

/// Mesh R of the contract, built by hand.
fn mesh_r() -> NavMesh {
    ok(
        pv(&[
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
        ]),
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
    )
}

fn nb(mesh: &NavMesh, id: u32) -> Vec<Option<u32>> {
    mesh.neighbours(PolyId(id))
        .unwrap()
        .iter()
        .map(|o| o.map(|q| q.0))
        .collect()
}

/// `n` vertices on a 256-wide raw grid, vertex `i` at `[i % 256, 0, i / 256]`.
fn grid_vertices(n: usize) -> Vec<[Fx16_16; 3]> {
    (0..n)
        .map(|i| rw((i % 256) as i32, 0, (i / 256) as i32))
        .collect()
}

/// `k` disjoint raw triangles, triangle `i` at `x = 4 i`.
fn tri_soup(k: usize) -> (Vec<[Fx16_16; 3]>, Vec<Vec<u32>>) {
    let mut vertices = Vec::new();
    let mut polygons = Vec::new();
    for i in 0..k {
        let x = 4 * i as i32;
        vertices.push(rw(x, 0, 0));
        vertices.push(rw(x + 1, 0, 0));
        vertices.push(rw(x, 0, 1));
        let b = 3 * i as u32;
        polygons.push(vec![b, b + 1, b + 2]);
    }
    (vertices, polygons)
}

/// Test-side model of B-C1, for building inputs.
fn enc(vertices: &[[i32; 3]], polygons: &[Vec<u32>]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"CNAV");
    out.push(1);
    out.extend_from_slice(&(vertices.len() as u32).to_le_bytes());
    out.extend_from_slice(&(polygons.len() as u32).to_le_bytes());
    for v in vertices {
        for c in v {
            out.extend_from_slice(&c.to_le_bytes());
        }
    }
    for poly in polygons {
        out.push(poly.len() as u8);
        for i in poly {
            out.extend_from_slice(&i.to_le_bytes());
        }
    }
    out
}

fn header(vertex_count: u32, polygon_count: u32) -> Vec<u8> {
    let mut out = b"CNAV".to_vec();
    out.push(1);
    out.extend_from_slice(&vertex_count.to_le_bytes());
    out.extend_from_slice(&polygon_count.to_le_bytes());
    out
}

/// T's canonical bytes, written out byte for byte from the contract.
fn t_bytes() -> Vec<u8> {
    vec![
        67, 78, 65, 86, // magic
        1,  // version
        3, 0, 0, 0, // vertex count
        1, 0, 0, 0, // polygon count
        0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, // vertex 0: 0, 131072, 0
        0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 0, 0, // vertex 1: 65536, 131072, 0
        0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 1, 0, // vertex 2: 0, 131072, 65536
        3, // polygon vertex count
        0, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, // indices 0, 1, 2
    ]
}

fn decode(bytes: &[u8]) -> Result<NavMesh, NavDecodeError> {
    NavMesh::from_canonical_bytes(bytes)
}

fn round_trips(mesh: &NavMesh) {
    let bytes = mesh.to_canonical_bytes();
    assert_eq!(decode(&bytes), Ok(mesh.clone()));
}

// ------------------------------------------------------------ constants

#[test]
fn constants_have_the_contract_values() {
    assert_eq!(COORD_LIMIT_RAW, 536_870_912);
    assert_eq!(MAX_NAV_POLYGONS, 16_384);
    assert_eq!(MAX_NAV_VERTICES, 65_536);
    assert_eq!(MAX_POLYGON_VERTICES, 8);
    assert_eq!(NAV_MAGIC, *b"CNAV");
    assert_eq!(NAV_FORMAT_VERSION, 1);
    assert_eq!(MAX_PATH_POINTS, 64);
    assert_eq!(MAX_PATH_LENGTH_RAW, 1 << 28);
}

// ----------------------------------------------------------------- B-G1

#[test]
fn ceil_sqrt_small_values() {
    let cases: [(u128, u128); 14] = [
        (0, 0),
        (1, 1),
        (2, 2),
        (3, 2),
        (4, 2),
        (5, 3),
        (8, 3),
        (9, 3),
        (10, 4),
        (15, 4),
        (16, 4),
        (17, 5),
        (99, 10),
        (101, 11),
    ];
    for (n, want) in cases {
        assert_eq!(ceil_sqrt(n), want, "ceil_sqrt({n})");
    }
}

#[test]
fn ceil_sqrt_large_values() {
    assert_eq!(ceil_sqrt(1 << 60), 1 << 30);
    assert_eq!(ceil_sqrt(1 << 61), 1_518_500_250);
    assert_eq!(ceil_sqrt(u128::MAX), 1 << 64);
    assert_eq!(ceil_sqrt(1 << 64), 1 << 32);
    assert_eq!(ceil_sqrt((1 << 64) + 1), (1 << 32) + 1);
    assert_eq!(ceil_sqrt(u64::MAX as u128), 1 << 32);
    let k: u128 = (1 << 64) - 1;
    assert_eq!(ceil_sqrt(k * k), k);
    assert_eq!(ceil_sqrt(k * k - 1), k);
    assert_eq!(ceil_sqrt(k * k + 1), k + 1);
    assert_eq!(ceil_sqrt(10u128.pow(20)), 10u128.pow(10));
    assert_eq!(ceil_sqrt(10u128.pow(20) + 1), 10u128.pow(10) + 1);
}

#[test]
fn seg_len_contract_values() {
    assert_eq!(seg_len(p(0, 0), p(3, 4)), 327_680);
    assert_eq!(seg_len(rw(0, 0, 0), rw(1, 0, 1)), 2);
    assert_eq!(seg_len(rw(0, 0, 0), rw(3, 0, 4)), 5);
    assert_eq!(seg_len(rw(-LIM, 0, -LIM), rw(LIM, 0, LIM)), 1_518_500_250);
}

#[test]
fn seg_len_ignores_y() {
    assert_eq!(seg_len(rw(7, 0, 7), rw(7, 1_000_000, 7)), 0);
    assert_eq!(seg_len(rw(7, -LIM, 7), rw(7, LIM, 7)), 0);
    assert_eq!(seg_len(rw(0, 5, 0), rw(3, -90, 4)), 5);
}

#[test]
fn seg_len_is_symmetric_and_handles_negative_differences() {
    let a = rw(-100, 3, 250);
    let b = rw(300, 9, -50);
    assert_eq!(seg_len(a, b), seg_len(b, a));
    // dx = 400, dz = 300.
    assert_eq!(seg_len(a, b), 500);
    assert_eq!(seg_len(a, a), 0);
    assert_eq!(seg_len(rw(-3, 0, 0), rw(0, 0, -4)), 5);
}

#[test]
fn seg_len_rounds_up_per_axis_distances() {
    assert_eq!(seg_len(rw(0, 0, 0), rw(1, 0, 0)), 1);
    assert_eq!(seg_len(rw(0, 0, 0), rw(0, 0, -1)), 1);
    // dx = 1, dz = 2: sqrt(5) rounds up to 3.
    assert_eq!(seg_len(rw(0, 0, 0), rw(1, 0, 2)), 3);
    // dx = 2, dz = 2: sqrt(8) rounds up to 3.
    assert_eq!(seg_len(rw(0, 0, 0), rw(2, 0, 2)), 3);
}

#[test]
fn seg_len_spans_the_whole_coordinate_square_without_overflow() {
    assert_eq!(seg_len(rw(-LIM, 0, 0), rw(LIM, 0, 0)), 1 << 30);
    assert_eq!(seg_len(rw(0, 0, LIM), rw(0, 0, -LIM)), 1 << 30);
}

#[test]
fn seg_len_outside_the_limit_does_not_overflow() {
    assert_eq!(
        seg_len(rw(i32::MIN, 0, 0), rw(i32::MAX, 0, 0)),
        4_294_967_295
    );
    assert_eq!(
        seg_len(rw(0, 0, i32::MAX), rw(0, 0, i32::MIN)),
        4_294_967_295
    );
    assert_eq!(
        seg_len(
            rw(i32::MIN, i32::MIN, i32::MIN),
            rw(i32::MAX, i32::MAX, i32::MAX)
        ),
        6_074_000_999
    );
}

// ----------------------------------------------------------- B-M1 / B-M2

#[test]
fn vertex_count_below_three_is_rejected() {
    for n in 0..3usize {
        let vertices: Vec<_> = grid_vertices(n);
        assert_eq!(
            err(vertices, vec![vec![0, 1, 2]]),
            NavMeshError::VertexCount { count: n },
            "n = {n}"
        );
    }
}

#[test]
fn three_vertices_is_the_lower_vertex_limit() {
    ok(tri_vertices(), vec![vec![0, 1, 2]]);
}

#[test]
fn vertex_count_one_past_the_maximum_is_rejected() {
    let vertices = grid_vertices(MAX_NAV_VERTICES + 1);
    assert_eq!(
        err(vertices, vec![vec![0, 1, 257, 256]]),
        NavMeshError::VertexCount { count: 65_537 }
    );
}

#[test]
fn vertex_count_exactly_at_the_maximum_is_accepted() {
    let mesh = ok(grid_vertices(MAX_NAV_VERTICES), vec![vec![0, 1, 257, 256]]);
    assert_eq!(mesh.vertices().len(), 65_536);
    assert_eq!(mesh.polygon_count(), 1);
    assert_eq!(mesh.polygon(PolyId(0)), Some(&[0u32, 1, 257, 256][..]));
    // The highest vertex index is usable.
    ok(
        grid_vertices(MAX_NAV_VERTICES),
        vec![vec![65_535, 65_534, 65_279]],
    )
    .polygon_count();
}

#[test]
fn zero_polygons_is_rejected() {
    assert_eq!(
        err(tri_vertices(), vec![]),
        NavMeshError::PolygonCount { count: 0 }
    );
}

#[test]
fn polygon_count_one_past_the_maximum_is_rejected() {
    let (vertices, polygons) = tri_soup(MAX_NAV_POLYGONS + 1);
    assert_eq!(
        err(vertices, polygons),
        NavMeshError::PolygonCount { count: 16_385 }
    );
}

#[test]
fn polygon_count_exactly_at_the_maximum_is_accepted() {
    let (vertices, polygons) = tri_soup(MAX_NAV_POLYGONS);
    let mesh = ok(vertices, polygons);
    assert_eq!(mesh.polygon_count(), 16_384);
    assert_eq!(mesh.vertices().len(), 49_152);
    assert_eq!(nb(&mesh, 16_383), vec![None, None, None]);
    assert_eq!(
        mesh.polygon(PolyId(16_383)),
        Some(&[49_149u32, 49_150, 49_151][..])
    );
    assert_eq!(mesh.polygon(PolyId(16_384)), None);
}

#[test]
fn polygon_count_is_checked_before_polygon_contents() {
    // 16385 copies of one triangle would be a duplicate edge, but the count
    // comes first.
    let polygons = vec![vec![0, 1, 2]; MAX_NAV_POLYGONS + 1];
    assert_eq!(
        err(tri_vertices(), polygons),
        NavMeshError::PolygonCount { count: 16_385 }
    );
}

// ------------------------------------------------ B-M3: coordinate bounds

#[test]
fn every_component_is_bounded_in_both_directions() {
    for &bad in &[
        LIM + 1,
        -LIM - 1,
        i32::MAX,
        i32::MIN,
        i32::MIN + 1,
        LIM + 65_536,
    ] {
        for axis in 0..3 {
            let mut c = [0, 0, 0];
            c[axis] = bad;
            let vertices = vec![rw(0, 0, 0), rw(1, 0, 0), rw(c[0], c[1], c[2])];
            // Keep vertex 2 distinct from the others even when it is 0.
            assert_eq!(
                err(vertices, vec![vec![0, 1, 2]]),
                NavMeshError::VertexOutOfBounds { vertex: 2 },
                "axis {axis}, value {bad}"
            );
        }
    }
}

#[test]
fn the_exact_limit_is_allowed_on_x_in_both_directions() {
    let mesh = ok(
        vec![rw(-LIM, 0, 0), rw(LIM, 0, 0), rw(0, 0, 1)],
        vec![vec![0, 1, 2]],
    );
    assert_eq!(mesh.polygon_count(), 1);
}

#[test]
fn the_exact_limit_is_allowed_on_z_in_both_directions() {
    let mesh = ok(
        vec![rw(0, 0, -LIM), rw(1, 0, -LIM), rw(0, 0, LIM)],
        vec![vec![0, 1, 2]],
    );
    assert_eq!(mesh.polygon_count(), 1);
}

#[test]
fn the_exact_limit_is_allowed_on_y_in_both_directions() {
    for edge in [LIM, -LIM] {
        let mesh = ok(
            vec![rw(0, edge, 0), rw(1, edge, 0), rw(0, edge, 1)],
            vec![vec![0, 1, 2]],
        );
        assert_eq!(mesh.polygon_y(PolyId(0)), Some(Fx16_16::from_raw(edge)));
    }
}

#[test]
fn the_four_corner_square_at_the_limit_is_valid() {
    let mesh = ok(
        vec![
            rw(-LIM, 0, -LIM),
            rw(LIM, 0, -LIM),
            rw(LIM, 0, LIM),
            rw(-LIM, 0, LIM),
        ],
        vec![vec![0, 1, 2, 3]],
    );
    assert_eq!(mesh.polygon(PolyId(0)), Some(&[0u32, 1, 2, 3][..]));
    round_trips(&mesh);
}

#[test]
fn contract_triangle_at_the_corners_is_valid() {
    // A-2: raw [2^29,0,2^29], [-2^29,0,2^29], [-2^29,0,-2^29], polygon [2,0,1].
    let mesh = ok(
        vec![rw(LIM, 0, LIM), rw(-LIM, 0, LIM), rw(-LIM, 0, -LIM)],
        vec![vec![2, 0, 1]],
    );
    assert_eq!(mesh.polygon(PolyId(0)), Some(&[2u32, 0, 1][..]));
}

#[test]
fn one_past_the_limit_is_rejected_contract_example() {
    assert_eq!(
        err(
            vec![rw(0, 0, 0), rw(1, 0, 0), rw(0, 0, LIM + 1)],
            vec![vec![0, 1, 2]]
        ),
        NavMeshError::VertexOutOfBounds { vertex: 2 }
    );
}

#[test]
fn i32_min_magnitude_is_out_of_bounds_and_never_panics() {
    assert_eq!(
        err(
            vec![rw(i32::MIN, 0, 0), rw(1, 0, 0), rw(0, 0, 1)],
            vec![vec![0, 1, 2]]
        ),
        NavMeshError::VertexOutOfBounds { vertex: 0 }
    );
    assert_eq!(
        err(
            vec![rw(0, 0, 0), rw(1, i32::MIN, 0), rw(0, 0, 1)],
            vec![vec![0, 1, 2]]
        ),
        NavMeshError::VertexOutOfBounds { vertex: 1 }
    );
    assert_eq!(
        err(
            vec![rw(0, 0, 0), rw(1, 0, 0), rw(0, 0, i32::MIN)],
            vec![vec![0, 1, 2]]
        ),
        NavMeshError::VertexOutOfBounds { vertex: 2 }
    );
    assert_eq!(
        err(
            vec![rw(i32::MIN, i32::MIN, i32::MIN); 3],
            vec![vec![0, 1, 2]]
        ),
        NavMeshError::VertexOutOfBounds { vertex: 0 }
    );
}

#[test]
fn first_out_of_bounds_vertex_in_index_order_is_reported() {
    let vertices = vec![
        rw(0, 0, 0),
        rw(1, 0, 0),
        rw(0, 0, 1),
        rw(LIM + 1, 0, 0),
        rw(0, 0, i32::MIN),
    ];
    assert_eq!(
        err(vertices, vec![vec![0, 1, 2]]),
        NavMeshError::VertexOutOfBounds { vertex: 3 }
    );
}

#[test]
fn out_of_bounds_vertices_need_not_be_used_by_a_polygon() {
    assert_eq!(
        err(
            vec![rw(0, 0, 0), rw(1, 0, 0), rw(0, 0, 1), rw(0, LIM + 1, 0)],
            vec![vec![0, 1, 2]]
        ),
        NavMeshError::VertexOutOfBounds { vertex: 3 }
    );
}

// --------------------------------------------------- B-M4: duplicate vertices

#[test]
fn duplicate_vertex_reports_the_later_index() {
    assert_eq!(
        err(pv(&[(0, 0), (1, 0), (0, 1), (0, 0)]), vec![vec![0, 1, 2]]),
        NavMeshError::DuplicateVertex { vertex: 3 }
    );
    assert_eq!(
        err(pv(&[(0, 0), (1, 0), (0, 0)]), vec![vec![0, 1, 2]]),
        NavMeshError::DuplicateVertex { vertex: 2 }
    );
}

#[test]
fn first_duplicate_in_index_order_wins() {
    // Index 2 repeats index 1; index 3 repeats index 0. The earlier-found
    // later index (2) is reported.
    assert_eq!(
        err(pv(&[(0, 0), (1, 0), (1, 0), (0, 0)]), vec![vec![0, 1, 3]]),
        NavMeshError::DuplicateVertex { vertex: 2 }
    );
}

#[test]
fn vertices_differing_only_in_y_are_not_duplicates() {
    let mut vertices = tri_vertices();
    vertices.push(rw(0, 65_536, 0));
    let mesh = ok(vertices, vec![vec![0, 1, 2]]);
    assert_eq!(mesh.vertices().len(), 4);
}

#[test]
fn vertices_differing_in_one_raw_unit_are_not_duplicates() {
    ok(
        vec![rw(0, 0, 0), rw(1, 0, 0), rw(0, 0, 1), rw(1, 0, 1)],
        vec![vec![0, 1, 2]],
    );
    ok(
        vec![rw(0, 0, 0), rw(1, 0, 0), rw(0, 0, 1), rw(0, 1, 0)],
        vec![vec![0, 1, 2]],
    );
}

// ------------------------------------------------------- B-M5: polygon size

#[test]
fn polygon_with_two_vertices_is_a_size_error() {
    assert_eq!(
        err(tri_vertices(), vec![vec![0, 1]]),
        NavMeshError::PolygonSize { poly: 0, count: 2 }
    );
    assert_eq!(
        err(tri_vertices(), vec![vec![0]]),
        NavMeshError::PolygonSize { poly: 0, count: 1 }
    );
    assert_eq!(
        err(tri_vertices(), vec![vec![]]),
        NavMeshError::PolygonSize { poly: 0, count: 0 }
    );
}

#[test]
fn polygon_with_nine_vertices_is_a_size_error_even_if_otherwise_valid() {
    // Octagon of side 2 with a collinear midpoint inserted on the first edge.
    let vertices = pv(&[
        (2, 0),
        (3, 0),
        (4, 0),
        (6, 2),
        (6, 4),
        (4, 6),
        (2, 6),
        (0, 4),
        (0, 2),
    ]);
    assert_eq!(
        err(vertices, vec![vec![0, 1, 2, 3, 4, 5, 6, 7, 8]]),
        NavMeshError::PolygonSize { poly: 0, count: 9 }
    );
}

#[test]
fn polygon_with_eight_vertices_is_accepted() {
    let vertices = pv(&[
        (2, 0),
        (4, 0),
        (6, 2),
        (6, 4),
        (4, 6),
        (2, 6),
        (0, 4),
        (0, 2),
    ]);
    let mesh = ok(vertices, vec![vec![0, 1, 2, 3, 4, 5, 6, 7]]);
    assert_eq!(mesh.polygon(PolyId(0)).unwrap().len(), 8);
    assert_eq!(nb(&mesh, 0), vec![None; 8]);
    round_trips(&mesh);
}

#[test]
fn eight_vertices_with_collinear_points_is_accepted() {
    // A square with a midpoint on each side.
    let vertices = pv(&[
        (0, 0),
        (2, 0),
        (4, 0),
        (4, 2),
        (4, 4),
        (2, 4),
        (0, 4),
        (0, 2),
    ]);
    ok(vertices, vec![vec![0, 1, 2, 3, 4, 5, 6, 7]]);
}

#[test]
fn polygon_size_error_names_the_polygon_id() {
    assert_eq!(
        err(
            pv(&[(0, 0), (1, 0), (0, 1), (1, 1)]),
            vec![vec![0, 1, 2], vec![1, 3, 2], vec![0, 1]]
        ),
        NavMeshError::PolygonSize { poly: 2, count: 2 }
    );
}

// ------------------------------------------------------ B-M5: vertex index

#[test]
fn index_past_the_end_is_a_vertex_index_error() {
    assert_eq!(
        err(tri_vertices(), vec![vec![0, 1, 7]]),
        NavMeshError::VertexIndex { poly: 0, index: 7 }
    );
}

#[test]
fn index_equal_to_the_vertex_count_is_missing_but_the_last_is_fine() {
    assert_eq!(
        err(tri_vertices(), vec![vec![0, 1, 3]]),
        NavMeshError::VertexIndex { poly: 0, index: 3 }
    );
    ok(tri_vertices(), vec![vec![0, 1, 2]]);
}

#[test]
fn u32_max_index_does_not_panic() {
    assert_eq!(
        err(tri_vertices(), vec![vec![0, 1, u32::MAX]]),
        NavMeshError::VertexIndex {
            poly: 0,
            index: u32::MAX
        }
    );
}

#[test]
fn first_missing_index_in_list_order_is_reported() {
    assert_eq!(
        err(tri_vertices(), vec![vec![0, 9, 8]]),
        NavMeshError::VertexIndex { poly: 0, index: 9 }
    );
    assert_eq!(
        err(tri_vertices(), vec![vec![8, 0, 9]]),
        NavMeshError::VertexIndex { poly: 0, index: 8 }
    );
}

#[test]
fn vertex_index_error_names_the_polygon_id() {
    assert_eq!(
        err(tri_vertices(), vec![vec![0, 1, 2], vec![2, 1, 5]]),
        NavMeshError::VertexIndex { poly: 1, index: 5 }
    );
}

// ---------------------------------------------------- B-M5: repeated vertex

#[test]
fn repeated_index_is_a_repeated_vertex_error() {
    assert_eq!(
        err(tri_vertices(), vec![vec![0, 1, 1]]),
        NavMeshError::RepeatedVertex { poly: 0 }
    );
    assert_eq!(
        err(tri_vertices(), vec![vec![0, 1, 2, 0]]),
        NavMeshError::RepeatedVertex { poly: 0 }
    );
}

#[test]
fn distinct_indices_with_equal_x_and_z_are_a_repeated_vertex_error() {
    let mut vertices = tri_vertices();
    vertices.push([m(0), m(1), m(0)]);
    assert_eq!(
        err(vertices, vec![vec![0, 1, 2, 3]]),
        NavMeshError::RepeatedVertex { poly: 0 }
    );
}

#[test]
fn nine_index_polygon_with_repeats_is_still_a_size_error() {
    assert_eq!(
        err(tri_vertices(), vec![vec![0, 0, 0, 0, 0, 0, 0, 0, 0]]),
        NavMeshError::PolygonSize { poly: 0, count: 9 }
    );
}

#[test]
fn repeated_vertex_error_names_the_polygon_id() {
    assert_eq!(
        err(tri_vertices(), vec![vec![0, 1, 2], vec![2, 1, 1]]),
        NavMeshError::RepeatedVertex { poly: 1 }
    );
}

#[test]
fn nearly_equal_positions_are_not_repeats() {
    // x differs by one raw unit.
    ok(
        vec![rw(0, 0, 0), rw(1, 0, 0), rw(0, 0, 1), rw(1, 0, 1)],
        vec![vec![0, 1, 3, 2]],
    );
}

// ------------------------------------------------------- B-M6: horizontal

#[test]
fn polygon_with_differing_y_is_not_horizontal() {
    let vertices = vec![p(0, 0), p(1, 0), [m(0), m(1), m(1)]];
    assert_eq!(
        err(vertices, vec![vec![0, 1, 2]]),
        NavMeshError::NotHorizontal { poly: 0 }
    );
}

#[test]
fn one_raw_unit_of_y_difference_is_not_horizontal() {
    for bad in 0..3 {
        let mut vertices = vec![rw(0, 5, 0), rw(4, 5, 0), rw(0, 5, 4)];
        vertices[bad][1] = Fx16_16::from_raw(6);
        assert_eq!(
            err(vertices, vec![vec![0, 1, 2]]),
            NavMeshError::NotHorizontal { poly: 0 },
            "vertex {bad}"
        );
    }
}

#[test]
fn a_common_nonzero_or_negative_y_is_horizontal() {
    let mesh = mesh_t();
    assert_eq!(mesh.polygon_y(PolyId(0)), Some(Fx16_16::from_raw(131_072)));
    let low = ok(
        vec![rw(0, -77, 0), rw(4, -77, 0), rw(0, -77, 4)],
        vec![vec![0, 1, 2]],
    );
    assert_eq!(low.polygon_y(PolyId(0)), Some(Fx16_16::from_raw(-77)));
}

/// Every vertex of the polygon `points` at height 0 except `raised`, which is
/// one raw unit higher; the polygon lists them in order.
fn raised_at(points: &[(i32, i32)], raised: usize) -> (Vec<[Fx16_16; 3]>, Vec<u32>) {
    let vertices = points
        .iter()
        .enumerate()
        .map(|(i, &(x, z))| rw(x * 65_536, i32::from(i == raised), z * 65_536))
        .collect();
    (vertices, (0..points.len() as u32).collect())
}

#[test]
fn a_quad_with_one_raised_vertex_is_not_horizontal_at_any_index() {
    let quad = [(0, 0), (4, 0), (4, 4), (0, 4)];
    for raised in 0..4 {
        let (vertices, polygon) = raised_at(&quad, raised);
        assert_eq!(
            err(vertices, vec![polygon]),
            NavMeshError::NotHorizontal { poly: 0 },
            "raised vertex {raised}"
        );
    }
}

#[test]
fn an_octagon_with_one_raised_vertex_is_not_horizontal_at_any_index() {
    let octagon = [
        (2, 0),
        (4, 0),
        (6, 2),
        (6, 4),
        (4, 6),
        (2, 6),
        (0, 4),
        (0, 2),
    ];
    for raised in 0..8 {
        let (vertices, polygon) = raised_at(&octagon, raised);
        assert_eq!(
            err(vertices, vec![polygon]),
            NavMeshError::NotHorizontal { poly: 0 },
            "raised vertex {raised}"
        );
    }
}

#[test]
fn a_level_quad_and_octagon_are_horizontal() {
    let (vertices, polygon) = raised_at(&[(0, 0), (4, 0), (4, 4), (0, 4)], 99);
    ok(vertices, vec![polygon]);
    let octagon = [
        (2, 0),
        (4, 0),
        (6, 2),
        (6, 4),
        (4, 6),
        (2, 6),
        (0, 4),
        (0, 2),
    ];
    let (vertices, polygon) = raised_at(&octagon, 99);
    ok(vertices, vec![polygon]);
}

#[test]
fn a_raised_late_vertex_names_its_polygon_id() {
    // Polygon 1 is a quad whose last vertex is raised.
    let mut vertices = vec![p(0, 0), p(1, 0), p(0, 1)];
    vertices.extend([
        p(10, 0),
        p(14, 0),
        p(14, 4),
        [m(10), Fx16_16::from_raw(1), m(4)],
    ]);
    assert_eq!(
        err(vertices, vec![vec![0, 1, 2], vec![3, 4, 5, 6]]),
        NavMeshError::NotHorizontal { poly: 1 }
    );
}

#[test]
fn polygons_may_sit_at_different_heights() {
    let vertices = vec![
        rw(0, 0, 0),
        rw(4, 0, 0),
        rw(0, 0, 4),
        rw(10, 9, 0),
        rw(14, 9, 0),
        rw(10, 9, 4),
    ];
    let mesh = ok(vertices, vec![vec![0, 1, 2], vec![3, 4, 5]]);
    assert_eq!(mesh.polygon_y(PolyId(0)), Some(Fx16_16::from_raw(0)));
    assert_eq!(mesh.polygon_y(PolyId(1)), Some(Fx16_16::from_raw(9)));
}

#[test]
fn not_horizontal_error_names_the_polygon_id() {
    let vertices = vec![
        p(0, 0),
        p(1, 0),
        p(0, 1),
        [m(5), m(1), m(5)],
        p(6, 5),
        p(5, 6),
    ];
    // Polygon 1 is [3, 4, 5] with vertex 3 raised.
    assert_eq!(
        err(vertices, vec![vec![0, 1, 2], vec![3, 4, 5]]),
        NavMeshError::NotHorizontal { poly: 1 }
    );
}

// --------------------------------------------------------- B-M7: convexity

#[test]
fn clockwise_triangle_is_not_convex() {
    assert_eq!(
        err(tri_vertices(), vec![vec![0, 2, 1]]),
        NavMeshError::NotConvex { poly: 0 }
    );
}

#[test]
fn every_rotation_of_a_ccw_polygon_is_valid() {
    ok(tri_vertices(), vec![vec![1, 2, 0]]);
    ok(tri_vertices(), vec![vec![2, 0, 1]]);
    ok(
        pv(&[(0, 0), (2, 0), (2, 2), (0, 2)]),
        vec![vec![2, 3, 0, 1]],
    );
}

#[test]
fn the_dart_is_not_convex() {
    assert_eq!(
        err(
            pv(&[(0, 0), (4, 0), (1, 1), (0, 4)]),
            vec![vec![0, 1, 2, 3]]
        ),
        NavMeshError::NotConvex { poly: 0 }
    );
}

#[test]
fn the_back_track_is_not_convex() {
    assert_eq!(
        err(
            pv(&[(0, 0), (2, 0), (1, 0), (0, 2)]),
            vec![vec![0, 1, 2, 3]]
        ),
        NavMeshError::NotConvex { poly: 0 }
    );
}

#[test]
fn a_spike_that_reverses_along_an_edge_is_not_convex() {
    // (4,4) then (4,2): collinear with the previous edge but reversing.
    assert_eq!(
        err(
            pv(&[(0, 0), (4, 0), (4, 4), (4, 2)]),
            vec![vec![0, 1, 2, 3]]
        ),
        NavMeshError::NotConvex { poly: 0 }
    );
}

#[test]
fn a_flat_polygon_is_not_convex() {
    assert_eq!(
        err(pv(&[(0, 0), (1, 0), (2, 0)]), vec![vec![0, 1, 2]]),
        NavMeshError::NotConvex { poly: 0 }
    );
}

#[test]
fn a_flat_polygon_of_four_collinear_points_is_not_convex() {
    assert_eq!(
        err(
            pv(&[(0, 0), (1, 0), (2, 0), (3, 0)]),
            vec![vec![0, 1, 2, 3]]
        ),
        NavMeshError::NotConvex { poly: 0 }
    );
}

#[test]
fn the_bow_tie_is_not_convex() {
    assert_eq!(
        err(
            pv(&[(0, 0), (2, 2), (2, 0), (0, 2)]),
            vec![vec![0, 1, 2, 3]]
        ),
        NavMeshError::NotConvex { poly: 0 }
    );
}

#[test]
fn a_clockwise_square_is_not_convex() {
    assert_eq!(
        err(
            pv(&[(0, 0), (0, 2), (2, 2), (2, 0)]),
            vec![vec![0, 1, 2, 3]]
        ),
        NavMeshError::NotConvex { poly: 0 }
    );
}

#[test]
fn a_reflex_l_shape_is_not_convex() {
    assert_eq!(
        err(
            pv(&[(0, 0), (4, 0), (4, 2), (2, 2), (2, 4), (0, 4)]),
            vec![vec![0, 1, 2, 3, 4, 5]]
        ),
        NavMeshError::NotConvex { poly: 0 }
    );
}

#[test]
fn the_pentagram_passes_every_consecutive_turn_but_is_not_convex() {
    let star = pv(&[(0, 0), (4, 0), (5, 3), (2, 5), (-1, 3)]);
    assert_eq!(
        err(star.clone(), vec![vec![0, 2, 4, 1, 3]]),
        NavMeshError::NotConvex { poly: 0 }
    );
    // The same five points in boundary order are a convex pentagon.
    ok(star, vec![vec![0, 1, 2, 3, 4]]);
}

#[test]
fn collinear_continuing_vertices_are_allowed() {
    let mesh = ok(
        pv(&[(0, 0), (1, 0), (2, 0), (0, 2)]),
        vec![vec![0, 1, 2, 3]],
    );
    assert_eq!(mesh.polygon(PolyId(0)), Some(&[0u32, 1, 2, 3][..]));
    // Collinear on a diagonal, and collinear before the first-listed vertex.
    ok(
        pv(&[(0, 0), (1, 1), (2, 2), (0, 4)]),
        vec![vec![0, 1, 2, 3]],
    );
    ok(
        pv(&[(0, 0), (1, 0), (2, 0), (0, 2)]),
        vec![vec![2, 3, 0, 1]],
    );
}

#[test]
fn a_square_with_a_midpoint_on_every_side_is_convex() {
    let mesh = ok(
        pv(&[
            (0, 0),
            (2, 0),
            (4, 0),
            (4, 2),
            (4, 4),
            (2, 4),
            (0, 4),
            (0, 2),
        ]),
        vec![vec![0, 1, 2, 3, 4, 5, 6, 7]],
    );
    assert_eq!(mesh.polygon_count(), 1);
}

#[test]
fn convexity_is_exact_at_one_raw_unit() {
    // The middle vertex is one raw unit inside the chord: reflex.
    assert_eq!(
        err(
            vec![rw(0, 0, 0), rw(1000, 0, 0), rw(500, 0, 1), rw(0, 0, 1000)],
            vec![vec![0, 1, 2, 3]]
        ),
        NavMeshError::NotConvex { poly: 0 }
    );
    // One raw unit outside the chord: convex.
    ok(
        vec![rw(0, 0, 0), rw(1000, 0, 0), rw(1001, 0, 1), rw(0, 0, 1000)],
        vec![vec![0, 1, 2, 3]],
    );
}

#[test]
fn convexity_uses_wide_arithmetic_at_the_coordinate_limit() {
    // Convex triangle spanning the full square.
    ok(
        vec![rw(-LIM, 0, -LIM), rw(LIM, 0, -LIM), rw(0, 0, LIM)],
        vec![vec![0, 1, 2]],
    );
    // Same corners clockwise.
    assert_eq!(
        err(
            vec![rw(-LIM, 0, -LIM), rw(0, 0, LIM), rw(LIM, 0, -LIM)],
            vec![vec![0, 1, 2]]
        ),
        NavMeshError::NotConvex { poly: 0 }
    );
    // A near-flat triangle at full extent: one raw unit of area-doubling.
    ok(
        vec![rw(-LIM, 0, -LIM), rw(LIM, 0, -LIM), rw(LIM, 0, -LIM + 1)],
        vec![vec![0, 1, 2]],
    );
    assert_eq!(
        err(
            vec![rw(-LIM, 0, -LIM), rw(LIM, 0, -LIM), rw(0, 0, -LIM)],
            vec![vec![0, 1, 2]]
        ),
        NavMeshError::NotConvex { poly: 0 }
    );
}

#[test]
fn not_convex_ignores_y() {
    // Same x/z as a clockwise triangle but with a constant nonzero y.
    assert_eq!(
        err(
            vec![rw(0, 9, 0), rw(0, 9, 4), rw(4, 9, 0)],
            vec![vec![0, 1, 2]]
        ),
        NavMeshError::NotConvex { poly: 0 }
    );
}

#[test]
fn not_convex_error_names_the_polygon_id() {
    assert_eq!(
        err(
            pv(&[(0, 0), (1, 0), (0, 1), (5, 5), (6, 5), (5, 6)]),
            vec![vec![0, 1, 2], vec![3, 5, 4]]
        ),
        NavMeshError::NotConvex { poly: 1 }
    );
}

// ---------------------------------------------------- B-M8: duplicate edges

#[test]
fn a_repeated_directed_edge_reports_the_later_polygon() {
    assert_eq!(
        err(
            pv(&[(0, 0), (1, 0), (0, 1), (1, 1)]),
            vec![vec![0, 1, 2], vec![0, 1, 3]]
        ),
        NavMeshError::DuplicateEdge { poly: 1 }
    );
}

#[test]
fn the_duplicate_edge_may_be_any_edge_of_the_polygon() {
    // P0 [0,1,2] has edges 0->1, 1->2, 2->0. P1 [2,0,3] repeats 2->0 (its
    // first edge) and is counter-clockwise: (0,4),(0,0),(4,4).
    let vertices = pv(&[(0, 0), (4, 0), (0, 4), (4, 4)]);
    assert_eq!(
        err(vertices, vec![vec![0, 1, 2], vec![2, 0, 3]]),
        NavMeshError::DuplicateEdge { poly: 1 }
    );
}

#[test]
fn the_first_polygon_to_repeat_an_edge_is_reported() {
    // P0 [0,1,2]; P1 [0,1,3] repeats 0->1; P2 [0,1,4] also repeats it.
    let vertices = pv(&[(0, 0), (4, 0), (0, 4), (4, 4), (2, 6)]);
    assert_eq!(
        err(vertices, vec![vec![0, 1, 2], vec![0, 1, 3], vec![0, 1, 4]]),
        NavMeshError::DuplicateEdge { poly: 1 }
    );
}

#[test]
fn the_third_polygon_is_named_when_only_it_repeats_an_edge() {
    // P0 [0,1,2]; P1 [1,3,2] is the reversed neighbour of P0's edge 1->2;
    // P2 repeats P1 exactly.
    let vertices = pv(&[(0, 0), (4, 0), (0, 4), (4, 4)]);
    assert_eq!(
        err(vertices, vec![vec![0, 1, 2], vec![1, 3, 2], vec![1, 3, 2]]),
        NavMeshError::DuplicateEdge { poly: 2 }
    );
}

#[test]
fn a_reversed_edge_is_adjacency_not_a_duplicate() {
    let mesh = ok(
        vec![p(0, 0), p(1, 0), p(0, 1), p(1, -1)],
        vec![vec![0, 1, 2], vec![1, 0, 3]],
    );
    assert_eq!(nb(&mesh, 0), vec![Some(1), None, None]);
    assert_eq!(nb(&mesh, 1), vec![Some(0), None, None]);
}

// ----------------------------------------------------------- validation order

#[test]
fn order_vertex_count_before_polygon_count() {
    assert_eq!(
        err(pv(&[(0, 0), (1, 0)]), vec![]),
        NavMeshError::VertexCount { count: 2 }
    );
}

#[test]
fn order_vertex_count_before_bounds() {
    assert_eq!(
        err(vec![rw(0, 0, 0), rw(LIM + 1, 0, 0)], vec![vec![0, 1]]),
        NavMeshError::VertexCount { count: 2 }
    );
}

#[test]
fn order_vertex_count_before_duplicate_vertex() {
    assert_eq!(
        err(pv(&[(0, 0), (0, 0)]), vec![vec![0, 1]]),
        NavMeshError::VertexCount { count: 2 }
    );
}

#[test]
fn order_polygon_count_before_bounds() {
    assert_eq!(
        err(vec![rw(0, 0, 0), rw(1, 0, 0), rw(i32::MIN, 0, 1)], vec![]),
        NavMeshError::PolygonCount { count: 0 }
    );
}

#[test]
fn order_polygon_count_before_duplicate_vertex() {
    assert_eq!(
        err(pv(&[(0, 0), (1, 0), (0, 0)]), vec![]),
        NavMeshError::PolygonCount { count: 0 }
    );
}

#[test]
fn order_bounds_before_duplicate_vertex_even_when_the_duplicate_comes_first() {
    // Index 1 duplicates index 0; index 2 is out of bounds.
    assert_eq!(
        err(
            vec![rw(0, 0, 0), rw(0, 0, 0), rw(0, 0, LIM + 1)],
            vec![vec![0, 1, 2]]
        ),
        NavMeshError::VertexOutOfBounds { vertex: 2 }
    );
}

#[test]
fn order_bounds_before_polygon_rules() {
    assert_eq!(
        err(
            vec![rw(0, 0, 0), rw(1, 0, 0), rw(0, 0, LIM + 1)],
            vec![vec![0, 1]]
        ),
        NavMeshError::VertexOutOfBounds { vertex: 2 }
    );
}

#[test]
fn order_duplicate_vertex_before_polygon_size() {
    assert_eq!(
        err(pv(&[(0, 0), (1, 0), (0, 1), (0, 0)]), vec![vec![0, 1]]),
        NavMeshError::DuplicateVertex { vertex: 3 }
    );
}

#[test]
fn order_duplicate_vertex_before_vertex_index() {
    assert_eq!(
        err(pv(&[(0, 0), (1, 0), (0, 1), (0, 0)]), vec![vec![0, 1, 99]]),
        NavMeshError::DuplicateVertex { vertex: 3 }
    );
}

#[test]
fn order_polygon_size_before_vertex_index() {
    assert_eq!(
        err(tri_vertices(), vec![vec![0, 77]]),
        NavMeshError::PolygonSize { poly: 0, count: 2 }
    );
    assert_eq!(
        err(tri_vertices(), vec![vec![0, 1, 2, 0, 1, 2, 0, 1, 99]]),
        NavMeshError::PolygonSize { poly: 0, count: 9 }
    );
}

#[test]
fn order_vertex_index_before_repeated_vertex() {
    assert_eq!(
        err(tri_vertices(), vec![vec![0, 0, 7]]),
        NavMeshError::VertexIndex { poly: 0, index: 7 }
    );
    assert_eq!(
        err(tri_vertices(), vec![vec![7, 0, 0]]),
        NavMeshError::VertexIndex { poly: 0, index: 7 }
    );
    assert_eq!(
        err(tri_vertices(), vec![vec![0, 1, 1, 7]]),
        NavMeshError::VertexIndex { poly: 0, index: 7 }
    );
}

#[test]
fn order_repeated_vertex_before_not_horizontal() {
    // Index repeated, and the y of the repeated vertex differs from the first.
    let vertices = vec![p(0, 0), [m(1), m(2), m(0)], p(0, 1)];
    assert_eq!(
        err(vertices, vec![vec![0, 1, 1]]),
        NavMeshError::RepeatedVertex { poly: 0 }
    );
    // Equal positions at different heights (the contract's own case).
    let vertices = vec![p(0, 0), p(1, 0), p(0, 1), [m(0), m(1), m(0)]];
    assert_eq!(
        err(vertices, vec![vec![0, 1, 2, 3]]),
        NavMeshError::RepeatedVertex { poly: 0 }
    );
}

#[test]
fn order_not_horizontal_before_not_convex() {
    // Clockwise and not level: the level check wins.
    let vertices = vec![p(0, 0), p(1, 0), [m(0), m(1), m(1)]];
    assert_eq!(
        err(vertices, vec![vec![0, 2, 1]]),
        NavMeshError::NotHorizontal { poly: 0 }
    );
}

#[test]
fn order_not_convex_before_duplicate_edge() {
    // P0 [0,1,2]; P1 [0,1,3] repeats an edge; P2 is clockwise.
    assert_eq!(
        err(
            pv(&[(0, 0), (1, 0), (0, 1), (1, 1)]),
            vec![vec![0, 1, 2], vec![0, 1, 3], vec![0, 2, 1]]
        ),
        NavMeshError::NotConvex { poly: 2 }
    );
}

#[test]
fn order_not_horizontal_before_duplicate_edge() {
    let vertices = vec![p(0, 0), p(1, 0), p(0, 1), [m(1), m(1), m(1)]];
    assert_eq!(
        err(vertices, vec![vec![0, 1, 2], vec![0, 1, 3]]),
        NavMeshError::NotHorizontal { poly: 1 }
    );
}

#[test]
fn order_polygon_rules_before_duplicate_edge_for_a_later_size_error() {
    assert_eq!(
        err(
            pv(&[(0, 0), (1, 0), (0, 1), (1, 1)]),
            vec![vec![0, 1, 2], vec![0, 1, 3], vec![0, 1]]
        ),
        NavMeshError::PolygonSize { poly: 2, count: 2 }
    );
}

// ------------------------------------------- per-polygon interleaving B-M5..7

#[test]
fn interleave_earlier_not_convex_beats_later_size_error() {
    assert_eq!(
        err(tri_vertices(), vec![vec![0, 2, 1], vec![0, 1]]),
        NavMeshError::NotConvex { poly: 0 }
    );
}

#[test]
fn interleave_earlier_not_horizontal_beats_later_size_error() {
    let vertices = vec![p(0, 0), p(1, 0), [m(0), m(1), m(1)]];
    assert_eq!(
        err(vertices, vec![vec![0, 1, 2], vec![0, 1]]),
        NavMeshError::NotHorizontal { poly: 0 }
    );
}

#[test]
fn interleave_earlier_not_horizontal_beats_later_vertex_index() {
    let vertices = vec![p(0, 0), p(1, 0), [m(0), m(1), m(1)]];
    assert_eq!(
        err(vertices, vec![vec![0, 1, 2], vec![0, 1, 9]]),
        NavMeshError::NotHorizontal { poly: 0 }
    );
}

#[test]
fn interleave_earlier_repeated_beats_later_vertex_index() {
    assert_eq!(
        err(tri_vertices(), vec![vec![0, 1, 1], vec![0, 1, 9]]),
        NavMeshError::RepeatedVertex { poly: 0 }
    );
}

#[test]
fn interleave_earlier_not_convex_beats_later_vertex_index_and_repeat() {
    assert_eq!(
        err(tri_vertices(), vec![vec![0, 2, 1], vec![0, 1, 9]]),
        NavMeshError::NotConvex { poly: 0 }
    );
    assert_eq!(
        err(tri_vertices(), vec![vec![0, 2, 1], vec![1, 1, 2]]),
        NavMeshError::NotConvex { poly: 0 }
    );
}

#[test]
fn interleave_earlier_size_beats_later_not_convex() {
    assert_eq!(
        err(tri_vertices(), vec![vec![0, 1], vec![0, 2, 1]]),
        NavMeshError::PolygonSize { poly: 0, count: 2 }
    );
}

#[test]
fn interleave_earlier_vertex_index_beats_later_size() {
    assert_eq!(
        err(tri_vertices(), vec![vec![0, 1, 9], vec![0, 1]]),
        NavMeshError::VertexIndex { poly: 0, index: 9 }
    );
}

#[test]
fn interleave_a_valid_first_polygon_then_the_second_fails() {
    assert_eq!(
        err(
            pv(&[(0, 0), (1, 0), (0, 1), (1, 1)]),
            vec![vec![0, 1, 2], vec![1, 2, 3], vec![0, 1]]
        ),
        NavMeshError::NotConvex { poly: 1 }
    );
}

#[test]
fn interleave_each_polygon_is_finished_before_the_next_starts() {
    // Polygon 1 is not horizontal; polygon 2 has a missing index. The
    // not-horizontal error of the earlier polygon is returned.
    let vertices = vec![
        p(0, 0),
        p(1, 0),
        p(0, 1),
        [m(3), m(1), m(3)],
        p(4, 3),
        p(3, 4),
    ];
    assert_eq!(
        err(vertices, vec![vec![0, 1, 2], vec![3, 4, 5], vec![0, 1, 99]]),
        NavMeshError::NotHorizontal { poly: 1 }
    );
}

#[test]
fn interleave_later_polygon_errors_carry_their_own_id() {
    assert_eq!(
        err(
            tri_vertices(),
            vec![vec![0, 1, 2], vec![1, 2, 0], vec![0, 1, 1]]
        ),
        NavMeshError::RepeatedVertex { poly: 2 }
    );
}

// --------------------------------------------------------- B-M9: adjacency

#[test]
fn mesh_n_neighbours_match_the_contract() {
    let mesh = mesh_n();
    assert_eq!(nb(&mesh, 0), vec![None, Some(1), None, None]);
    assert_eq!(nb(&mesh, 1), vec![None, Some(2), None, Some(0)]);
    assert_eq!(nb(&mesh, 2), vec![Some(1), None, None]);
    assert_eq!(mesh.polygon_count(), 3);
}

#[test]
fn mesh_w_neighbours_match_the_contract() {
    let mesh = mesh_w();
    assert_eq!(nb(&mesh, 0), vec![None, None, Some(1), None]);
    assert_eq!(nb(&mesh, 1), vec![Some(0), Some(2), None, None]);
    assert_eq!(nb(&mesh, 2), vec![None, Some(4), None, Some(1)]);
    assert_eq!(nb(&mesh, 3), vec![None, None, Some(4), None]);
    assert_eq!(nb(&mesh, 4), vec![Some(3), None, None, Some(2)]);
}

#[test]
fn mesh_r_neighbours_are_derived_edge_by_edge() {
    let mesh = mesh_r();
    assert_eq!(mesh.polygon_count(), 8);
    assert_eq!(mesh.vertices().len(), 16);
    assert_eq!(nb(&mesh, 0), vec![None, Some(3), Some(1), None]);
    assert_eq!(nb(&mesh, 1), vec![Some(0), None, Some(2), None]);
    assert_eq!(nb(&mesh, 2), vec![Some(1), Some(4), None, None]);
    assert_eq!(nb(&mesh, 3), vec![None, Some(5), None, Some(0)]);
    assert_eq!(nb(&mesh, 4), vec![None, Some(7), None, Some(2)]);
    assert_eq!(nb(&mesh, 5), vec![None, None, Some(6), Some(3)]);
    assert_eq!(nb(&mesh, 6), vec![Some(5), None, Some(7), None]);
    assert_eq!(nb(&mesh, 7), vec![Some(6), None, None, Some(4)]);
}

#[test]
fn adjacency_is_symmetric_on_every_named_mesh() {
    for mesh in [mesh_n(), mesh_w(), mesh_r()] {
        for a in 0..mesh.polygon_count() as u32 {
            for (edge, other) in nb(&mesh, a).into_iter().enumerate() {
                if let Some(b) = other {
                    let poly_a = mesh.polygon(PolyId(a)).unwrap();
                    let poly_b = mesh.polygon(PolyId(b)).unwrap();
                    let n = poly_a.len();
                    let from = poly_a[edge];
                    let to = poly_a[(edge + 1) % n];
                    // Polygon b holds the reversed pair somewhere, at an edge
                    // whose neighbour is a.
                    let back = (0..poly_b.len())
                        .find(|&j| poly_b[j] == to && poly_b[(j + 1) % poly_b.len()] == from)
                        .expect("reversed edge present");
                    assert_eq!(nb(&mesh, b)[back], Some(a));
                }
            }
        }
    }
}

#[test]
fn neighbour_lists_have_one_entry_per_edge() {
    let mesh = mesh_r();
    for id in 0..8u32 {
        assert_eq!(
            mesh.neighbours(PolyId(id)).unwrap().len(),
            mesh.polygon(PolyId(id)).unwrap().len()
        );
    }
}

#[test]
fn a_vertex_only_touch_is_not_adjacency() {
    let mesh = ok(
        pv(&[(0, 0), (1, 0), (0, 1), (2, 1), (1, 2)]),
        vec![vec![0, 1, 2], vec![1, 3, 4]],
    );
    assert_eq!(nb(&mesh, 0), vec![None, None, None]);
    assert_eq!(nb(&mesh, 1), vec![None, None, None]);
}

#[test]
fn a_vertex_lying_on_another_polygons_edge_is_not_adjacency() {
    // Big square 0..2; two stacked unit squares on its right whose shared
    // corner (2,1) lies on the big square's edge but has its own index.
    let vertices = pv(&[(0, 0), (2, 0), (2, 2), (0, 2), (3, 0), (3, 1), (2, 1)]);
    let mesh = ok(vertices, vec![vec![0, 1, 2, 3], vec![1, 4, 5, 6]]);
    assert_eq!(nb(&mesh, 0), vec![None, None, None, None]);
    assert_eq!(nb(&mesh, 1), vec![None, None, None, None]);
}

#[test]
fn adjacency_requires_the_same_two_vertex_indices() {
    // Two coincident-position edges built from distinct (y-offset) vertices
    // are not adjacent: indices differ.
    let vertices = vec![
        p(0, 0),
        p(1, 0),
        p(0, 1),
        rw(65_536, 1, 0),
        rw(0, 1, 0),
        rw(65_536, 1, -65_536),
    ];
    // Polygon 1 is at height 1 raw: (1,0)->(0,0)->(1,-1) at the same x/z as
    // polygon 0's edge but with different vertex indices.
    let mesh = ok(vertices, vec![vec![0, 1, 2], vec![3, 4, 5]]);
    assert_eq!(nb(&mesh, 0), vec![None, None, None]);
    assert_eq!(nb(&mesh, 1), vec![None, None, None]);
}

#[test]
fn a_fan_of_three_polygons_around_a_vertex() {
    // Triangles around the centre vertex 0.
    let vertices = pv(&[(0, 0), (2, 0), (0, 2), (-2, -2)]);
    let mesh = ok(vertices, vec![vec![0, 1, 2], vec![0, 2, 3], vec![0, 3, 1]]);
    // P0 edges 0->1, 1->2, 2->0; P1 edges 0->2, 2->3, 3->0; P2 edges 0->3, 3->1, 1->0.
    assert_eq!(nb(&mesh, 0), vec![Some(2), None, Some(1)]);
    assert_eq!(nb(&mesh, 1), vec![Some(0), None, Some(2)]);
    assert_eq!(nb(&mesh, 2), vec![Some(1), None, Some(0)]);
}

// ------------------------------------------------- B-M10: accessors, no panic

#[test]
fn accessors_return_what_was_built() {
    let mesh = mesh_n();
    assert_eq!(mesh.polygon_count(), 3);
    assert_eq!(mesh.vertices().len(), 7);
    assert_eq!(mesh.vertices()[4], p(8, 4));
    assert_eq!(mesh.polygon(PolyId(0)), Some(&[0u32, 1, 2, 3][..]));
    assert_eq!(mesh.polygon(PolyId(1)), Some(&[1u32, 4, 5, 2][..]));
    assert_eq!(mesh.polygon(PolyId(2)), Some(&[5u32, 4, 6][..]));
    for id in 0..3 {
        assert_eq!(mesh.polygon_y(PolyId(id)), Some(m(0)));
    }
}

#[test]
fn the_polygon_list_order_and_rotation_are_kept() {
    let mesh = ok(tri_vertices(), vec![vec![2, 0, 1]]);
    assert_eq!(mesh.polygon(PolyId(0)), Some(&[2u32, 0, 1][..]));
}

#[test]
fn ids_past_the_end_return_none() {
    let mesh = mesh_n();
    for id in [3u32, 4, 1000, u32::MAX] {
        assert_eq!(mesh.polygon(PolyId(id)), None, "polygon {id}");
        assert_eq!(mesh.polygon_y(PolyId(id)), None, "polygon_y {id}");
        assert_eq!(mesh.neighbours(PolyId(id)), None, "neighbours {id}");
    }
    assert!(mesh.polygon(PolyId(2)).is_some());
    assert!(mesh.polygon_y(PolyId(2)).is_some());
    assert!(mesh.neighbours(PolyId(2)).is_some());
}

#[test]
fn contract_n_polygon_two_has_height_zero() {
    assert_eq!(mesh_n().polygon_y(PolyId(2)), Some(m(0)));
}

#[test]
fn poly_id_orders_and_compares_by_index() {
    assert!(PolyId(1) < PolyId(2));
    assert_eq!(PolyId(5), PolyId(5));
    let PolyId(raw_id) = PolyId(5);
    assert_eq!(raw_id, 5);
}

#[test]
fn meshes_with_equal_inputs_are_equal_and_cloneable() {
    assert_eq!(mesh_n(), mesh_n());
    assert_ne!(mesh_n(), mesh_w());
    let a = mesh_w();
    assert_eq!(a.clone(), a);
}

#[test]
fn extreme_inputs_never_panic() {
    let extremes = [
        i32::MIN,
        i32::MIN + 1,
        -LIM - 1,
        -LIM,
        -1,
        0,
        1,
        LIM,
        LIM + 1,
        i32::MAX,
    ];
    for &a in &extremes {
        for &b in &extremes {
            let vertices = vec![rw(a, b, a), rw(b, a, b), rw(a, a, b), rw(b, b, a)];
            for polys in [
                vec![vec![0, 1, 2]],
                vec![vec![0, 1, 2, 3]],
                vec![vec![3, 2, 1, 0]],
            ] {
                let _ = build(vertices.clone(), polys);
            }
        }
    }
}

#[test]
fn degenerate_polygon_lists_never_panic() {
    let v = tri_vertices();
    for polys in [
        vec![vec![]],
        vec![vec![u32::MAX; 8]],
        vec![vec![u32::MAX; 9]],
        vec![vec![0; 3]],
        vec![vec![0, 1, 2]; 3],
        vec![vec![], vec![], vec![]],
    ] {
        let _ = build(v.clone(), polys);
    }
    let _ = build(vec![], vec![]);
    let _ = build(vec![], vec![vec![0, 1, 2]]);
}

// ------------------------------------------------------------------ B-C1

#[test]
fn mesh_t_bytes_are_exactly_the_contract_bytes() {
    let bytes = mesh_t().to_canonical_bytes();
    assert_eq!(bytes.len(), 62);
    assert_eq!(bytes, t_bytes());
}

#[test]
fn bytes_start_with_magic_and_version() {
    let bytes = mesh_n().to_canonical_bytes();
    assert_eq!(&bytes[0..4], &NAV_MAGIC);
    assert_eq!(bytes[4], NAV_FORMAT_VERSION);
    assert_eq!(&bytes[5..9], &7u32.to_le_bytes());
    assert_eq!(&bytes[9..13], &3u32.to_le_bytes());
}

#[test]
fn mesh_n_bytes_match_the_layout_model_and_length() {
    let bytes = mesh_n().to_canonical_bytes();
    // 13 + 7 * 12 + (1 + 16) + (1 + 16) + (1 + 12) = 144.
    assert_eq!(bytes.len(), 144);
    let raw = |x: i32, z: i32| [x * 65_536, 0, z * 65_536];
    let expected = enc(
        &[
            raw(0, 0),
            raw(4, 0),
            raw(4, 2),
            raw(0, 2),
            raw(8, 4),
            raw(6, 6),
            raw(9, 8),
        ],
        &[vec![0, 1, 2, 3], vec![1, 4, 5, 2], vec![5, 4, 6]],
    );
    assert_eq!(bytes, expected);
}

#[test]
fn negative_and_extreme_coordinates_are_little_endian_twos_complement() {
    let mesh = ok(
        vec![rw(LIM, 0, LIM), rw(-LIM, 0, LIM), rw(-LIM, 0, -LIM)],
        vec![vec![2, 0, 1]],
    );
    let bytes = mesh.to_canonical_bytes();
    assert_eq!(bytes.len(), 62);
    assert_eq!(&bytes[0..13], &header(3, 1)[..]);
    // Vertex 0: 2^29, 0, 2^29.
    assert_eq!(&bytes[13..25], &[0, 0, 0, 32, 0, 0, 0, 0, 0, 0, 0, 32]);
    // Vertex 1: -2^29, 0, 2^29.
    assert_eq!(&bytes[25..37], &[0, 0, 0, 224, 0, 0, 0, 0, 0, 0, 0, 32]);
    // Vertex 2: -2^29, 0, -2^29.
    assert_eq!(&bytes[37..49], &[0, 0, 0, 224, 0, 0, 0, 0, 0, 0, 0, 224]);
    // Polygon: count 3, indices 2, 0, 1.
    assert_eq!(&bytes[49..62], &[3, 2, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0]);
}

#[test]
fn small_negative_values_are_all_ones_bytes() {
    let mesh = ok(
        vec![rw(-1, 0, -1), rw(0, 0, -1), rw(0, 0, 0)],
        vec![vec![0, 1, 2]],
    );
    let bytes = mesh.to_canonical_bytes();
    assert_eq!(
        &bytes[13..25],
        &[255, 255, 255, 255, 0, 0, 0, 0, 255, 255, 255, 255]
    );
}

#[test]
fn polygon_vertex_count_is_one_byte_and_indices_are_four_bytes() {
    let vertices = pv(&[
        (2, 0),
        (4, 0),
        (6, 2),
        (6, 4),
        (4, 6),
        (2, 6),
        (0, 4),
        (0, 2),
    ]);
    let bytes = ok(vertices, vec![vec![0, 1, 2, 3, 4, 5, 6, 7]]).to_canonical_bytes();
    assert_eq!(bytes.len(), 13 + 8 * 12 + 1 + 8 * 4);
    assert_eq!(bytes[13 + 8 * 12], 8);
}

#[test]
fn indices_above_255_use_all_four_bytes() {
    let mesh = ok(grid_vertices(MAX_NAV_VERTICES), vec![vec![0, 1, 257, 256]]);
    let bytes = mesh.to_canonical_bytes();
    assert_eq!(bytes.len(), 786_462);
    let tail = &bytes[bytes.len() - 17..];
    assert_eq!(tail, &[4, 0, 0, 0, 0, 1, 0, 0, 0, 1, 1, 0, 0, 0, 1, 0, 0]);
    assert_eq!(&bytes[5..9], &65_536u32.to_le_bytes());
}

#[test]
fn neighbours_are_not_stored_in_the_bytes() {
    // Two polygons that are adjacent and two that are not, over the same
    // data layout size: the length is a function of counts only.
    let adjacent = ok(
        vec![p(0, 0), p(1, 0), p(0, 1), p(1, -1)],
        vec![vec![0, 1, 2], vec![1, 0, 3]],
    );
    assert_eq!(adjacent.to_canonical_bytes().len(), 13 + 4 * 12 + 2 * 13);
    let wall = mesh_w();
    assert_eq!(wall.to_canonical_bytes().len(), 13 + 12 * 12 + 5 * 17);
    assert_eq!(wall.to_canonical_bytes().len(), 242);
}

#[test]
fn the_bytes_are_deterministic() {
    assert_eq!(mesh_r().to_canonical_bytes(), mesh_r().to_canonical_bytes());
}

#[test]
fn unused_vertices_are_written() {
    let mut vertices = tri_vertices();
    vertices.push(p(9, 9));
    let bytes = ok(vertices, vec![vec![0, 1, 2]]).to_canonical_bytes();
    assert_eq!(&bytes[5..9], &4u32.to_le_bytes());
    assert_eq!(bytes.len(), 13 + 4 * 12 + 13);
}

// ----------------------------------------------------------- round trips

#[test]
fn named_meshes_round_trip() {
    round_trips(&mesh_t());
    round_trips(&mesh_n());
    round_trips(&mesh_w());
    round_trips(&mesh_r());
}

#[test]
fn round_trip_keeps_neighbours_and_heights() {
    let original = mesh_w();
    let back = decode(&original.to_canonical_bytes()).unwrap();
    for id in 0..5u32 {
        assert_eq!(nb(&back, id), nb(&original, id));
        assert_eq!(back.polygon_y(PolyId(id)), original.polygon_y(PolyId(id)));
    }
    assert_eq!(back.vertices(), original.vertices());
    assert_eq!(decode(&t_bytes()), Ok(mesh_t()));
}

#[test]
fn limit_sized_meshes_round_trip() {
    round_trips(&ok(
        grid_vertices(MAX_NAV_VERTICES),
        vec![vec![0, 1, 257, 256]],
    ));
    let (vertices, polygons) = tri_soup(MAX_NAV_POLYGONS);
    let mesh = ok(vertices, polygons);
    assert_eq!(mesh.to_canonical_bytes().len(), 802_829);
    round_trips(&mesh);
}

#[test]
fn extreme_coordinates_and_unused_vertices_round_trip() {
    let mut vertices = vec![rw(-LIM, -LIM, -LIM), rw(LIM, -LIM, -LIM), rw(0, -LIM, LIM)];
    vertices.push(rw(LIM, LIM, LIM));
    round_trips(&ok(vertices, vec![vec![0, 1, 2]]));
}

// ------------------------------------------------------------------ B-C2

#[test]
fn fewer_than_four_bytes_is_truncated() {
    assert_eq!(decode(&[]), Err(NavDecodeError::Truncated));
    assert_eq!(decode(b"C"), Err(NavDecodeError::Truncated));
    assert_eq!(decode(b"CN"), Err(NavDecodeError::Truncated));
    assert_eq!(decode(b"CNA"), Err(NavDecodeError::Truncated));
    // Even a wrong prefix is Truncated when under four bytes.
    assert_eq!(decode(b"XNA"), Err(NavDecodeError::Truncated));
    assert_eq!(decode(b"X"), Err(NavDecodeError::Truncated));
}

#[test]
fn wrong_magic_is_bad_magic() {
    assert_eq!(decode(b"XNAV"), Err(NavDecodeError::BadMagic));
    assert_eq!(decode(b"cnav"), Err(NavDecodeError::BadMagic));
    assert_eq!(decode(b"CNAX"), Err(NavDecodeError::BadMagic));
    let mut bytes = t_bytes();
    bytes[0] = b'X';
    assert_eq!(decode(&bytes), Err(NavDecodeError::BadMagic));
}

#[test]
fn bad_magic_is_checked_before_the_version() {
    assert_eq!(decode(b"XNAV\x02"), Err(NavDecodeError::BadMagic));
    assert_eq!(decode(b"XNAV\x01"), Err(NavDecodeError::BadMagic));
}

#[test]
fn magic_alone_is_truncated() {
    assert_eq!(decode(b"CNAV"), Err(NavDecodeError::Truncated));
}

#[test]
fn unsupported_version_is_reported_with_its_value() {
    assert_eq!(
        decode(b"CNAV\x02"),
        Err(NavDecodeError::UnsupportedVersion { version: 2 })
    );
    assert_eq!(
        decode(b"CNAV\x00"),
        Err(NavDecodeError::UnsupportedVersion { version: 0 })
    );
    assert_eq!(
        decode(b"CNAV\xff"),
        Err(NavDecodeError::UnsupportedVersion { version: 255 })
    );
    let mut bytes = t_bytes();
    bytes[4] = 9;
    assert_eq!(
        decode(&bytes),
        Err(NavDecodeError::UnsupportedVersion { version: 9 })
    );
}

#[test]
fn version_is_checked_before_the_header_length() {
    // Short, with a bad version: the version error wins over Truncated.
    assert_eq!(
        decode(b"CNAV\x03\x00\x00"),
        Err(NavDecodeError::UnsupportedVersion { version: 3 })
    );
}

#[test]
fn a_header_shorter_than_thirteen_bytes_is_truncated() {
    let full = t_bytes();
    for n in 5..13 {
        assert_eq!(
            decode(&full[..n]),
            Err(NavDecodeError::Truncated),
            "{n} bytes"
        );
    }
}

#[test]
fn the_first_twelve_bytes_of_t_are_truncated() {
    assert_eq!(decode(&t_bytes()[..12]), Err(NavDecodeError::Truncated));
}

#[test]
fn every_proper_prefix_of_t_past_the_header_is_truncated() {
    let full = t_bytes();
    for n in 0..full.len() {
        let result = decode(&full[..n]);
        assert_eq!(result, Err(NavDecodeError::Truncated), "{n} bytes");
    }
    assert_eq!(decode(&full), Ok(mesh_t()));
}

#[test]
fn t_minus_the_last_byte_is_truncated() {
    let mut bytes = t_bytes();
    bytes.pop();
    assert_eq!(decode(&bytes), Err(NavDecodeError::Truncated));
}

#[test]
fn t_plus_one_byte_is_trailing_bytes() {
    let mut bytes = t_bytes();
    bytes.push(0);
    assert_eq!(decode(&bytes), Err(NavDecodeError::TrailingBytes));
    bytes.extend_from_slice(&[1, 2, 3, 4, 5]);
    assert_eq!(decode(&bytes), Err(NavDecodeError::TrailingBytes));
}

#[test]
fn trailing_bytes_win_over_a_mesh_error() {
    let mut bytes = t_bytes();
    let last = bytes.len() - 4;
    bytes[last] = 9;
    bytes.push(0);
    assert_eq!(decode(&bytes), Err(NavDecodeError::TrailingBytes));
}

#[test]
fn truncation_wins_over_a_mesh_error() {
    // Header says three vertices but only two follow; a mesh would be invalid.
    let mut bytes = header(3, 1);
    bytes.extend_from_slice(&t_bytes()[13..13 + 24]);
    assert_eq!(decode(&bytes), Err(NavDecodeError::Truncated));
}

#[test]
fn vertex_count_over_the_limit_is_invalid_before_reading() {
    let bytes = header(65_537, 1);
    assert_eq!(bytes.len(), 13);
    assert_eq!(
        decode(&bytes),
        Err(NavDecodeError::Invalid(NavMeshError::VertexCount {
            count: 65_537
        }))
    );
    assert_eq!(
        decode(&header(u32::MAX, 1)),
        Err(NavDecodeError::Invalid(NavMeshError::VertexCount {
            count: u32::MAX as usize
        }))
    );
}

#[test]
fn polygon_count_over_the_limit_is_invalid_before_reading() {
    assert_eq!(
        decode(&header(3, 16_385)),
        Err(NavDecodeError::Invalid(NavMeshError::PolygonCount {
            count: 16_385
        }))
    );
    assert_eq!(
        decode(&header(3, u32::MAX)),
        Err(NavDecodeError::Invalid(NavMeshError::PolygonCount {
            count: u32::MAX as usize
        }))
    );
}

#[test]
fn vertex_count_limit_is_checked_before_the_polygon_count_limit() {
    assert_eq!(
        decode(&header(65_537, 16_385)),
        Err(NavDecodeError::Invalid(NavMeshError::VertexCount {
            count: 65_537
        }))
    );
}

#[test]
fn counts_exactly_at_the_limits_proceed_to_reading() {
    assert_eq!(decode(&header(65_536, 1)), Err(NavDecodeError::Truncated));
    assert_eq!(decode(&header(3, 16_384)), Err(NavDecodeError::Truncated));
    assert_eq!(
        decode(&header(65_536, 16_384)),
        Err(NavDecodeError::Truncated)
    );
}

#[test]
fn zero_counts_read_nothing_then_fail_validation() {
    assert_eq!(
        decode(&header(0, 0)),
        Err(NavDecodeError::Invalid(NavMeshError::VertexCount {
            count: 0
        }))
    );
    // Three vertices, zero polygons.
    let mut bytes = header(3, 0);
    bytes.extend_from_slice(&t_bytes()[13..49]);
    assert_eq!(
        decode(&bytes),
        Err(NavDecodeError::Invalid(NavMeshError::PolygonCount {
            count: 0
        }))
    );
}

#[test]
fn a_missing_polygon_section_is_truncated() {
    let mut bytes = header(3, 1);
    bytes.extend_from_slice(&t_bytes()[13..49]);
    assert_eq!(decode(&bytes), Err(NavDecodeError::Truncated));
}

#[test]
fn a_polygon_count_byte_without_all_its_indices_is_truncated() {
    let mut bytes = t_bytes();
    bytes.truncate(49 + 1 + 8);
    assert_eq!(decode(&bytes), Err(NavDecodeError::Truncated));
}

#[test]
fn the_polygon_count_byte_accepts_any_u8_before_validation() {
    // 200 indices: read in full, then rejected by the mesh rules.
    let mut bytes = header(3, 1);
    bytes.extend_from_slice(&t_bytes()[13..49]);
    bytes.push(200);
    for _ in 0..200 {
        bytes.extend_from_slice(&0u32.to_le_bytes());
    }
    assert_eq!(
        decode(&bytes),
        Err(NavDecodeError::Invalid(NavMeshError::PolygonSize {
            poly: 0,
            count: 200
        }))
    );
    // 255 declared, but only a few follow: running out of bytes.
    let mut short = header(3, 1);
    short.extend_from_slice(&t_bytes()[13..49]);
    short.push(255);
    short.extend_from_slice(&[0; 12]);
    assert_eq!(decode(&short), Err(NavDecodeError::Truncated));
}

#[test]
fn a_zero_vertex_polygon_decodes_to_a_size_error() {
    let mut bytes = header(3, 1);
    bytes.extend_from_slice(&t_bytes()[13..49]);
    bytes.push(0);
    assert_eq!(
        decode(&bytes),
        Err(NavDecodeError::Invalid(NavMeshError::PolygonSize {
            poly: 0,
            count: 0
        }))
    );
}

#[test]
fn a_bad_index_in_the_bytes_is_an_invalid_vertex_index() {
    let mut bytes = t_bytes();
    let last = bytes.len() - 4;
    assert_eq!(bytes[last], 2);
    bytes[last] = 9;
    assert_eq!(
        decode(&bytes),
        Err(NavDecodeError::Invalid(NavMeshError::VertexIndex {
            poly: 0,
            index: 9
        }))
    );
    bytes[last..].copy_from_slice(&u32::MAX.to_le_bytes());
    assert_eq!(
        decode(&bytes),
        Err(NavDecodeError::Invalid(NavMeshError::VertexIndex {
            poly: 0,
            index: u32::MAX
        }))
    );
}

#[test]
fn mesh_errors_pass_through_invalid() {
    // Clockwise.
    let mut bytes = t_bytes();
    let n = bytes.len();
    bytes[n - 8] = 2;
    bytes[n - 4] = 1;
    assert_eq!(
        decode(&bytes),
        Err(NavDecodeError::Invalid(NavMeshError::NotConvex { poly: 0 }))
    );
    // Repeated vertex.
    let mut bytes = t_bytes();
    bytes[n - 4] = 1;
    assert_eq!(
        decode(&bytes),
        Err(NavDecodeError::Invalid(NavMeshError::RepeatedVertex {
            poly: 0
        }))
    );
    // Not horizontal: raise vertex 2's y.
    let mut bytes = t_bytes();
    bytes[13 + 24 + 6] = 3;
    assert_eq!(
        decode(&bytes),
        Err(NavDecodeError::Invalid(NavMeshError::NotHorizontal {
            poly: 0
        }))
    );
}

#[test]
fn out_of_range_coordinates_in_the_bytes_are_invalid_not_a_panic() {
    for &bad in &[i32::MIN, LIM + 1, -LIM - 1, i32::MAX] {
        let mut bytes = t_bytes();
        bytes[13 + 24..13 + 28].copy_from_slice(&bad.to_le_bytes());
        assert_eq!(
            decode(&bytes),
            Err(NavDecodeError::Invalid(NavMeshError::VertexOutOfBounds {
                vertex: 2
            })),
            "value {bad}"
        );
    }
}

#[test]
fn duplicate_vertices_and_edges_in_the_bytes_are_invalid() {
    let dup_vertex = enc(
        &[[0, 0, 0], [1, 0, 0], [0, 0, 1], [0, 0, 0]],
        &[vec![0, 1, 2]],
    );
    assert_eq!(
        decode(&dup_vertex),
        Err(NavDecodeError::Invalid(NavMeshError::DuplicateVertex {
            vertex: 3
        }))
    );
    let dup_edge = enc(
        &[[0, 0, 0], [1, 0, 0], [0, 0, 1], [1, 0, 1]],
        &[vec![0, 1, 2], vec![0, 1, 3]],
    );
    assert_eq!(
        decode(&dup_edge),
        Err(NavDecodeError::Invalid(NavMeshError::DuplicateEdge {
            poly: 1
        }))
    );
}

#[test]
fn decode_never_panics_on_prefixes_and_single_byte_changes() {
    let base = mesh_n().to_canonical_bytes();
    for n in 0..=base.len() {
        let _ = decode(&base[..n]);
    }
    for i in 0..base.len() {
        let mut changed = base.clone();
        changed[i] = changed[i].wrapping_add(0x55);
        let _ = decode(&changed);
        changed[i] = 0xff;
        let _ = decode(&changed);
        changed[i] = 0;
        let _ = decode(&changed);
    }
}

// ---------------------------------------------------------------- Display

#[test]
fn mesh_error_display_texts_are_exact() {
    let cases: Vec<(NavMeshError, &str)> = vec![
        (
            NavMeshError::VertexCount { count: 2 },
            "navmesh vertex count 2 is outside 3..=65536",
        ),
        (
            NavMeshError::PolygonCount { count: 0 },
            "navmesh polygon count 0 is outside 1..=16384",
        ),
        (
            NavMeshError::VertexOutOfBounds { vertex: 5 },
            "navmesh vertex 5 is outside the coordinate limit",
        ),
        (
            NavMeshError::DuplicateVertex { vertex: 3 },
            "navmesh vertex 3 repeats an earlier vertex",
        ),
        (
            NavMeshError::PolygonSize { poly: 1, count: 9 },
            "navmesh polygon 1 has 9 vertices, outside 3..=8",
        ),
        (
            NavMeshError::VertexIndex { poly: 2, index: 77 },
            "navmesh polygon 2 names missing vertex 77",
        ),
        (
            NavMeshError::RepeatedVertex { poly: 4 },
            "navmesh polygon 4 repeats a vertex position",
        ),
        (
            NavMeshError::NotHorizontal { poly: 6 },
            "navmesh polygon 6 is not horizontal",
        ),
        (
            NavMeshError::NotConvex { poly: 7 },
            "navmesh polygon 7 is not convex and counter-clockwise",
        ),
        (
            NavMeshError::DuplicateEdge { poly: 8 },
            "navmesh polygon 8 repeats a directed edge",
        ),
    ];
    for (error, text) in cases {
        assert_eq!(error.to_string(), text);
    }
}

#[test]
fn mesh_error_display_formats_large_numbers() {
    assert_eq!(
        NavMeshError::VertexIndex {
            poly: u32::MAX,
            index: u32::MAX
        }
        .to_string(),
        "navmesh polygon 4294967295 names missing vertex 4294967295"
    );
    assert_eq!(
        NavMeshError::VertexCount { count: 65_537 }.to_string(),
        "navmesh vertex count 65537 is outside 3..=65536"
    );
}

#[test]
fn decode_error_display_texts_are_exact() {
    assert_eq!(
        NavDecodeError::Truncated.to_string(),
        "navmesh bytes end early"
    );
    assert_eq!(
        NavDecodeError::BadMagic.to_string(),
        "navmesh bytes do not start with CNAV"
    );
    assert_eq!(
        NavDecodeError::UnsupportedVersion { version: 2 }.to_string(),
        "navmesh format version 2 is not supported"
    );
    assert_eq!(
        NavDecodeError::TrailingBytes.to_string(),
        "navmesh bytes continue after the last polygon"
    );
}

#[test]
fn invalid_displays_the_inner_error_text() {
    for inner in [
        NavMeshError::VertexCount { count: 65_537 },
        NavMeshError::PolygonCount { count: 16_385 },
        NavMeshError::VertexIndex { poly: 0, index: 9 },
        NavMeshError::NotConvex { poly: 3 },
    ] {
        assert_eq!(
            NavDecodeError::Invalid(inner.clone()).to_string(),
            inner.to_string()
        );
    }
    assert_eq!(
        NavDecodeError::Invalid(NavMeshError::VertexCount { count: 65_537 }).to_string(),
        "navmesh vertex count 65537 is outside 3..=65536"
    );
}

#[test]
fn decode_errors_from_real_inputs_display_the_table_text() {
    assert_eq!(
        decode(&[]).unwrap_err().to_string(),
        "navmesh bytes end early"
    );
    assert_eq!(
        decode(b"XNAV").unwrap_err().to_string(),
        "navmesh bytes do not start with CNAV"
    );
    assert_eq!(
        decode(b"CNAV\x02").unwrap_err().to_string(),
        "navmesh format version 2 is not supported"
    );
    let mut bytes = t_bytes();
    bytes.push(0);
    assert_eq!(
        decode(&bytes).unwrap_err().to_string(),
        "navmesh bytes continue after the last polygon"
    );
    assert_eq!(
        decode(&header(65_537, 1)).unwrap_err().to_string(),
        "navmesh vertex count 65537 is outside 3..=65536"
    );
    let mut bad = t_bytes();
    let last = bad.len() - 4;
    bad[last] = 9;
    assert_eq!(
        decode(&bad).unwrap_err().to_string(),
        "navmesh polygon 0 names missing vertex 9"
    );
}

#[test]
fn both_error_types_implement_std_error() {
    fn takes(_: &dyn std::error::Error) {}
    takes(&NavMeshError::NotConvex { poly: 0 });
    takes(&NavDecodeError::Truncated);
    let boxed: Box<dyn std::error::Error> = Box::new(NavDecodeError::BadMagic);
    assert_eq!(boxed.to_string(), "navmesh bytes do not start with CNAV");
}

#[test]
fn error_values_compare_and_clone() {
    let a = NavMeshError::PolygonSize { poly: 1, count: 2 };
    assert_eq!(a.clone(), a);
    assert_ne!(a, NavMeshError::PolygonSize { poly: 1, count: 3 });
    assert_ne!(a, NavMeshError::PolygonSize { poly: 2, count: 2 });
    let d = NavDecodeError::Invalid(a.clone());
    assert_eq!(d.clone(), d);
    assert_ne!(d, NavDecodeError::Truncated);
}

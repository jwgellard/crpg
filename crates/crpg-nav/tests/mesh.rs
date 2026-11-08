//! T035a implementer suite: lengths, mesh validation, adjacency, canonical bytes.

use crpg_nav::{
    ceil_sqrt, seg_len, Fx16_16, NavDecodeError, NavMesh, NavMeshError, PolyId, COORD_LIMIT_RAW,
    MAX_NAV_POLYGONS, NAV_FORMAT_VERSION, NAV_MAGIC,
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

fn mesh_err(vertices: Vec<[Fx16_16; 3]>, polygons: Vec<Vec<u32>>) -> NavMeshError {
    NavMesh::new(vertices, polygons).expect_err("mesh should be invalid")
}

/// Mesh N: three non-axis polygons.
fn mesh_n() -> NavMesh {
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
    .expect("N builds")
}

/// Mesh T: one raw-coordinate triangle.
fn mesh_t() -> NavMesh {
    NavMesh::new(
        vec![
            raw(0, 131072, 0),
            raw(65536, 131072, 0),
            raw(0, 131072, 65536),
        ],
        vec![vec![0, 1, 2]],
    )
    .expect("T builds")
}

/// Mesh W (the wall), written out as the literal the bake must later produce.
fn mesh_w() -> NavMesh {
    let xz: [(i32, i32); 12] = [
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
    NavMesh::new(
        xz.iter().map(|&(x, z)| p(x, z)).collect(),
        vec![
            vec![0, 1, 2, 3],
            vec![3, 2, 4, 5],
            vec![2, 6, 7, 4],
            vec![8, 9, 10, 6],
            vec![6, 10, 11, 7],
        ],
    )
    .expect("W builds")
}

fn ids(list: &[Option<u32>]) -> Vec<Option<PolyId>> {
    list.iter().map(|o| o.map(PolyId)).collect()
}

#[test]
fn a1_lengths() {
    assert_eq!(ceil_sqrt(0), 0);
    assert_eq!(ceil_sqrt(1), 1);
    assert_eq!(ceil_sqrt(2), 2);
    assert_eq!(ceil_sqrt(4), 2);
    assert_eq!(ceil_sqrt(5), 3);
    assert_eq!(ceil_sqrt(1 << 60), 1 << 30);
    assert_eq!(ceil_sqrt(1 << 61), 1_518_500_250);
    assert_eq!(ceil_sqrt(u128::MAX), 1 << 64);
    assert_eq!(seg_len(p(0, 0), p(3, 4)), 327_680);
    assert_eq!(seg_len(raw(0, 0, 0), raw(1, 0, 1)), 2);
    assert_eq!(seg_len(raw(5, 0, 5), raw(5, 99, 5)), 0);
    // Symmetric in its arguments.
    assert_eq!(seg_len(p(3, 4), p(0, 0)), 327_680);
    let lo = COORD_LIMIT_RAW;
    assert_eq!(seg_len(raw(-lo, 0, -lo), raw(lo, 0, lo)), 1_518_500_250);
    // Outside U3 it still cannot overflow.
    assert_eq!(
        seg_len(raw(i32::MIN, 0, i32::MIN), raw(i32::MAX, 0, i32::MAX)),
        6_074_000_999
    );
}

#[test]
fn a2_counts_and_bounds() {
    assert_eq!(
        mesh_err(vec![p(0, 0), p(1, 0)], vec![vec![0, 1, 0]]),
        NavMeshError::VertexCount { count: 2 }
    );
    assert_eq!(
        mesh_err(vec![p(0, 0), p(1, 0), p(0, 1)], vec![]),
        NavMeshError::PolygonCount { count: 0 }
    );
    let too_many: Vec<Vec<u32>> = vec![vec![0, 1, 2]; MAX_NAV_POLYGONS + 1];
    assert_eq!(
        mesh_err(vec![p(0, 0), p(1, 0), p(0, 1)], too_many),
        NavMeshError::PolygonCount {
            count: MAX_NAV_POLYGONS + 1
        }
    );
    let lo = COORD_LIMIT_RAW;
    assert_eq!(
        mesh_err(
            vec![raw(0, 0, 0), raw(1, 0, 0), raw(0, 0, lo + 1)],
            vec![vec![0, 1, 2]]
        ),
        NavMeshError::VertexOutOfBounds { vertex: 2 }
    );
    // The magnitude test uses unsigned_abs, so i32::MIN is out and does not panic.
    assert_eq!(
        mesh_err(
            vec![raw(0, 0, 0), raw(i32::MIN, 0, 0), raw(0, 0, 1)],
            vec![vec![0, 1, 2]]
        ),
        NavMeshError::VertexOutOfBounds { vertex: 1 }
    );
    assert_eq!(
        mesh_err(
            vec![raw(0, -lo - 1, 0), raw(1, 0, 0), raw(0, 0, 1)],
            vec![vec![0, 1, 2]]
        ),
        NavMeshError::VertexOutOfBounds { vertex: 0 }
    );
    // The limit itself is allowed, on either sign.
    assert!(NavMesh::new(
        vec![raw(lo, 0, lo), raw(-lo, 0, lo), raw(-lo, 0, -lo)],
        vec![vec![2, 0, 1]]
    )
    .is_ok());
    assert_eq!(
        mesh_err(
            vec![p(0, 0), p(1, 0), p(0, 1), p(0, 0)],
            vec![vec![0, 1, 2]]
        ),
        NavMeshError::DuplicateVertex { vertex: 3 }
    );
    // Bounds are checked before duplicates.
    assert_eq!(
        mesh_err(
            vec![raw(lo + 1, 0, 0), raw(lo + 1, 0, 0), raw(0, 0, 1)],
            vec![vec![0, 1, 2]]
        ),
        NavMeshError::VertexOutOfBounds { vertex: 0 }
    );
    // Equal x and z with a different y is not a duplicate vertex.
    assert!(NavMesh::new(
        vec![p(0, 0), p(1, 0), p(0, 1), [m(0), m(1), m(0)]],
        vec![vec![0, 1, 2]]
    )
    .is_ok());
}

#[test]
fn a3_polygon_rules() {
    let tri = || vec![p(0, 0), p(1, 0), p(0, 1)];
    assert_eq!(
        mesh_err(tri(), vec![vec![0, 1]]),
        NavMeshError::PolygonSize { poly: 0, count: 2 }
    );
    assert_eq!(
        mesh_err(tri(), vec![vec![0, 1, 2, 0, 1, 2, 0, 1, 2]]),
        NavMeshError::PolygonSize { poly: 0, count: 9 }
    );
    assert_eq!(
        mesh_err(tri(), vec![vec![0, 1, 7]]),
        NavMeshError::VertexIndex { poly: 0, index: 7 }
    );
    // The first missing index in list order, and before repeats.
    assert_eq!(
        mesh_err(tri(), vec![vec![9, 7, 7]]),
        NavMeshError::VertexIndex { poly: 0, index: 9 }
    );
    assert_eq!(
        mesh_err(tri(), vec![vec![0, 1, 1]]),
        NavMeshError::RepeatedVertex { poly: 0 }
    );
    let mut four = tri();
    four.push([m(0), m(1), m(0)]);
    assert_eq!(
        mesh_err(four, vec![vec![0, 1, 2, 3]]),
        NavMeshError::RepeatedVertex { poly: 0 }
    );
    assert_eq!(
        mesh_err(
            vec![p(0, 0), p(1, 0), [m(0), m(1), m(1)]],
            vec![vec![0, 1, 2]]
        ),
        NavMeshError::NotHorizontal { poly: 0 }
    );
    assert_eq!(
        mesh_err(tri(), vec![vec![0, 2, 1]]),
        NavMeshError::NotConvex { poly: 0 }
    );
    // The dart (reflex vertex).
    assert_eq!(
        mesh_err(
            vec![p(0, 0), p(4, 0), p(1, 1), p(0, 4)],
            vec![vec![0, 1, 2, 3]]
        ),
        NavMeshError::NotConvex { poly: 0 }
    );
    // The back-track.
    assert_eq!(
        mesh_err(
            vec![p(0, 0), p(2, 0), p(1, 0), p(0, 2)],
            vec![vec![0, 1, 2, 3]]
        ),
        NavMeshError::NotConvex { poly: 0 }
    );
    // The flat triangle.
    assert_eq!(
        mesh_err(vec![p(0, 0), p(1, 0), p(2, 0)], vec![vec![0, 1, 2]]),
        NavMeshError::NotConvex { poly: 0 }
    );
    // The pentagram has only strict left turns between neighbours but is not convex.
    let pent = || vec![p(0, 0), p(4, 0), p(5, 3), p(2, 5), p(-1, 3)];
    assert_eq!(
        mesh_err(pent(), vec![vec![0, 2, 4, 1, 3]]),
        NavMeshError::NotConvex { poly: 0 }
    );
    assert!(NavMesh::new(pent(), vec![vec![0, 1, 2, 3, 4]]).is_ok());
    // A collinear vertex that continues in the same direction is allowed.
    assert!(NavMesh::new(
        vec![p(0, 0), p(1, 0), p(2, 0), p(0, 2)],
        vec![vec![0, 1, 2, 3]]
    )
    .is_ok());
    // Polygon order: all of polygon 0 before polygon 1, and within a polygon
    // size, then index, then repeat, then height, then shape.
    assert_eq!(
        mesh_err(tri(), vec![vec![0, 2, 1], vec![0, 1]]),
        NavMeshError::NotConvex { poly: 0 }
    );
    assert_eq!(
        mesh_err(tri(), vec![vec![0, 1, 2], vec![0, 1]]),
        NavMeshError::PolygonSize { poly: 1, count: 2 }
    );
}

#[test]
fn a4_edges_and_adjacency() {
    assert_eq!(
        mesh_err(
            vec![p(0, 0), p(1, 0), p(0, 1), p(1, 1)],
            vec![vec![0, 1, 2], vec![0, 1, 3]]
        ),
        NavMeshError::DuplicateEdge { poly: 1 }
    );
    // Shape errors in a later polygon win over an earlier duplicate edge.
    assert_eq!(
        mesh_err(
            vec![p(0, 0), p(1, 0), p(0, 1), p(1, 1)],
            vec![vec![0, 1, 2], vec![0, 1, 3], vec![0, 1]]
        ),
        NavMeshError::PolygonSize { poly: 2, count: 2 }
    );
    let n = mesh_n();
    assert_eq!(n.polygon_count(), 3);
    assert_eq!(
        n.neighbours(PolyId(0)).expect("P0"),
        ids(&[None, Some(1), None, None])
    );
    assert_eq!(
        n.neighbours(PolyId(1)).expect("P1"),
        ids(&[None, Some(2), None, Some(0)])
    );
    assert_eq!(
        n.neighbours(PolyId(2)).expect("P2"),
        ids(&[Some(1), None, None])
    );
    assert_eq!(n.polygon(PolyId(3)), None);
    assert_eq!(n.polygon_y(PolyId(3)), None);
    assert_eq!(n.neighbours(PolyId(3)), None);
    assert_eq!(n.polygon(PolyId(u32::MAX)), None);
    assert_eq!(n.polygon_y(PolyId(2)), Some(m(0)));
    assert_eq!(n.polygon(PolyId(2)), Some(&[5u32, 4, 6][..]));
    assert_eq!(n.vertices().len(), 7);
    assert_eq!(n.vertices()[6], p(9, 8));

    // Vertex-only touch is not adjacency.
    let touch = NavMesh::new(
        vec![p(0, 0), p(1, 0), p(0, 1), p(2, 1), p(1, 2)],
        vec![vec![0, 1, 2], vec![1, 3, 4]],
    )
    .expect("touching polygons build");
    assert_eq!(touch.neighbours(PolyId(0)).expect("P0"), ids(&[None; 3]));
    assert_eq!(touch.neighbours(PolyId(1)).expect("P1"), ids(&[None; 3]));

    // Wall mesh: the full literal neighbour table, both directions.
    let w = mesh_w();
    let table: [&[Option<u32>]; 5] = [
        &[None, None, Some(1), None],
        &[Some(0), Some(2), None, None],
        &[None, Some(4), None, Some(1)],
        &[None, None, Some(4), None],
        &[Some(3), None, None, Some(2)],
    ];
    for (i, row) in table.iter().enumerate() {
        assert_eq!(w.neighbours(PolyId(i as u32)).expect("row"), ids(row));
    }
    // A polygon heights from its vertices.
    let high = NavMesh::new(
        vec![[m(0), m(5), m(0)], [m(1), m(5), m(0)], [m(0), m(5), m(1)]],
        vec![vec![0, 1, 2]],
    )
    .expect("raised triangle");
    assert_eq!(high.polygon_y(PolyId(0)), Some(m(5)));
}

#[test]
fn a5_canonical_bytes() {
    let expected: Vec<u8> = [
        "434e4156", "01", "03000000", "01000000", "00000000", "00000200", "00000000", "00000100",
        "00000200", "00000000", "00000000", "00000200", "00000100", "03", "00000000", "01000000",
        "02000000",
    ]
    .iter()
    .flat_map(|chunk| {
        (0..chunk.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&chunk[i..i + 2], 16).expect("hex"))
            .collect::<Vec<u8>>()
    })
    .collect();
    assert_eq!(expected.len(), 62);
    let t = mesh_t();
    assert_eq!(t.to_canonical_bytes(), expected);
    assert_eq!(&expected[..4], &NAV_MAGIC);
    assert_eq!(expected[4], NAV_FORMAT_VERSION);
    assert_eq!(NavMesh::from_canonical_bytes(&expected), Ok(t));

    let n = mesh_n();
    assert_eq!(
        NavMesh::from_canonical_bytes(&n.to_canonical_bytes()),
        Ok(n)
    );
    let w = mesh_w();
    // 4 + 1 + 4 + 4 header bytes, 12 vertices of 12 bytes, 5 quads of 17 bytes.
    assert_eq!(w.to_canonical_bytes().len(), 242);
    assert_eq!(
        NavMesh::from_canonical_bytes(&w.to_canonical_bytes()),
        Ok(w)
    );
    // Negative coordinates survive as little-endian two's complement.
    let neg = NavMesh::new(
        vec![raw(-1, -2, -3), raw(0, -2, -3), raw(-1, -2, 5)],
        vec![vec![0, 1, 2]],
    )
    .expect("negative coordinates");
    let bytes = neg.to_canonical_bytes();
    assert_eq!(&bytes[13..17], &[0xff, 0xff, 0xff, 0xff]);
    assert_eq!(NavMesh::from_canonical_bytes(&bytes), Ok(neg));
}

#[test]
fn a6_decode_errors() {
    use NavDecodeError as E;
    let t = mesh_t().to_canonical_bytes();
    assert_eq!(NavMesh::from_canonical_bytes(&[]), Err(E::Truncated));
    assert_eq!(NavMesh::from_canonical_bytes(b"CNA"), Err(E::Truncated));
    assert_eq!(
        NavMesh::from_canonical_bytes(b"XNAV\x01\x03\x00\x00\x00\x01\x00\x00\x00"),
        Err(E::BadMagic)
    );
    assert_eq!(NavMesh::from_canonical_bytes(b"CNAV"), Err(E::Truncated));
    assert_eq!(
        NavMesh::from_canonical_bytes(b"CNAV\x02"),
        Err(E::UnsupportedVersion { version: 2 })
    );
    assert_eq!(NavMesh::from_canonical_bytes(&t[..12]), Err(E::Truncated));
    assert_eq!(NavMesh::from_canonical_bytes(&t[..13]), Err(E::Truncated));
    assert_eq!(
        NavMesh::from_canonical_bytes(&t[..t.len() - 1]),
        Err(E::Truncated)
    );
    let mut plus = t.clone();
    plus.push(0);
    assert_eq!(NavMesh::from_canonical_bytes(&plus), Err(E::TrailingBytes));

    let mut header = b"CNAV\x01".to_vec();
    header.extend_from_slice(&65_537u32.to_le_bytes());
    header.extend_from_slice(&1u32.to_le_bytes());
    assert_eq!(
        NavMesh::from_canonical_bytes(&header),
        Err(E::Invalid(NavMeshError::VertexCount { count: 65_537 }))
    );
    let mut header = b"CNAV\x01".to_vec();
    header.extend_from_slice(&3u32.to_le_bytes());
    header.extend_from_slice(&16_385u32.to_le_bytes());
    assert_eq!(
        NavMesh::from_canonical_bytes(&header),
        Err(E::Invalid(NavMeshError::PolygonCount { count: 16_385 }))
    );

    let mut bad = t.clone();
    let last = bad.len() - 4;
    bad[last..].copy_from_slice(&9u32.to_le_bytes());
    assert_eq!(
        NavMesh::from_canonical_bytes(&bad),
        Err(E::Invalid(NavMeshError::VertexIndex { poly: 0, index: 9 }))
    );
    // Too few vertices decode far enough to reach the mesh validation.
    let mut tiny = b"CNAV\x01".to_vec();
    tiny.extend_from_slice(&0u32.to_le_bytes());
    tiny.extend_from_slice(&0u32.to_le_bytes());
    assert_eq!(
        NavMesh::from_canonical_bytes(&tiny),
        Err(E::Invalid(NavMeshError::VertexCount { count: 0 }))
    );

    assert_eq!(E::Truncated.to_string(), "navmesh bytes end early");
    assert_eq!(
        E::BadMagic.to_string(),
        "navmesh bytes do not start with CNAV"
    );
    assert_eq!(
        E::UnsupportedVersion { version: 2 }.to_string(),
        "navmesh format version 2 is not supported"
    );
    assert_eq!(
        E::TrailingBytes.to_string(),
        "navmesh bytes continue after the last polygon"
    );
    assert_eq!(
        E::Invalid(NavMeshError::VertexIndex { poly: 0, index: 9 }).to_string(),
        "navmesh polygon 0 names missing vertex 9"
    );
    let _boxed: Box<dyn std::error::Error> = Box::new(E::Truncated);
}

#[test]
fn a7_display_texts() {
    use NavMeshError as E;
    let table: [(E, &str); 10] = [
        (
            E::VertexCount { count: 2 },
            "navmesh vertex count 2 is outside 3..=65536",
        ),
        (
            E::PolygonCount { count: 0 },
            "navmesh polygon count 0 is outside 1..=16384",
        ),
        (
            E::VertexOutOfBounds { vertex: 4 },
            "navmesh vertex 4 is outside the coordinate limit",
        ),
        (
            E::DuplicateVertex { vertex: 3 },
            "navmesh vertex 3 repeats an earlier vertex",
        ),
        (
            E::PolygonSize { poly: 1, count: 9 },
            "navmesh polygon 1 has 9 vertices, outside 3..=8",
        ),
        (
            E::VertexIndex { poly: 2, index: 7 },
            "navmesh polygon 2 names missing vertex 7",
        ),
        (
            E::RepeatedVertex { poly: 5 },
            "navmesh polygon 5 repeats a vertex position",
        ),
        (
            E::NotHorizontal { poly: 6 },
            "navmesh polygon 6 is not horizontal",
        ),
        (
            E::NotConvex { poly: 7 },
            "navmesh polygon 7 is not convex and counter-clockwise",
        ),
        (
            E::DuplicateEdge { poly: 8 },
            "navmesh polygon 8 repeats a directed edge",
        ),
    ];
    for (value, text) in table {
        assert_eq!(value.to_string(), text);
    }
    let _boxed: Box<dyn std::error::Error> = Box::new(E::NotConvex { poly: 0 });
}

#[test]
fn b_m10_no_panic_on_hostile_input() {
    // Extreme and degenerate shapes return errors rather than panicking.
    let extreme = vec![
        raw(i32::MAX, i32::MAX, i32::MAX),
        raw(i32::MIN, 0, 0),
        raw(0, 0, 0),
    ];
    assert!(NavMesh::new(extreme, vec![vec![0, 1, 2]]).is_err());
    let at_limit = COORD_LIMIT_RAW;
    // Full-range cross products fit in i128 and the triangle is valid.
    let big = NavMesh::new(
        vec![
            raw(-at_limit, 0, -at_limit),
            raw(at_limit, 0, -at_limit),
            raw(0, 0, at_limit),
        ],
        vec![vec![0, 1, 2]],
    );
    assert!(big.is_ok());
    // Arbitrary short byte strings error cleanly.
    for len in 0..40usize {
        let bytes: Vec<u8> = (0..len).map(|i| (i * 37 % 251) as u8).collect();
        assert!(NavMesh::from_canonical_bytes(&bytes).is_err());
    }
}

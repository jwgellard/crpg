#![forbid(unsafe_code)]
use crpg_data::*;
use proptest::prelude::*;

fn requirement(package: &str, version: &str) -> PackageRequirement {
    PackageRequirement {
        kind: PackageKind::Module,
        package: package.parse().unwrap(),
        version: version.parse().unwrap(),
    }
}
fn candidate(package: &str, version: &str) -> PackageCandidate {
    PackageCandidate {
        kind: PackageKind::Module,
        package: package.parse().unwrap(),
        version: version.parse().unwrap(),
        checksum: Digest::from_bytes([7; 32]),
    }
}

#[test]
fn intersects_ranges_and_uses_semver_prerelease_rules() {
    let catalog = [
        candidate("a", "1.2.0"),
        candidate("a", "1.3.0"),
        candidate("a", "1.4.0"),
        candidate("a", "2.0.0"),
        candidate("a", "1.3.1-rc.1"),
    ];
    for (ranges, expected) in [
        (vec!["^1.2", "<1.4"], "1.3.0"),
        (vec!["~1.2"], "1.2.0"),
        (vec!["=1.3.0"], "1.3.0"),
        (vec![">=1.2, <=1.4"], "1.4.0"),
        (vec![">=1.3.1-rc.1, <1.4"], "1.3.1-rc.1"),
    ] {
        let reqs: Vec<_> = ranges.into_iter().map(|r| requirement("a", r)).collect();
        let result = resolve_packages(&reqs, &catalog).unwrap();
        assert_eq!(result[0].version.to_string(), expected);
        assert_eq!(result[0].checksum, Digest::from_bytes([7; 32]));
    }
    assert!(matches!(
        resolve_packages(&[requirement("a", "*")], &[candidate("a", "1.0.0-rc.1")]),
        Err(DataError::UnresolvedPackage { .. })
    ));
    assert!(matches!(
        resolve_packages(
            &[requirement("a", ">=1.0.0-rc.1")],
            &[candidate("a", "1.1.0-rc.1")]
        ),
        Err(DataError::UnresolvedPackage { .. })
    ));
}

#[test]
fn build_metadata_uses_lexical_complete_version_tie() {
    let mut catalog = vec![
        candidate("a", "1.0.0+9"),
        candidate("a", "1.0.0+10"),
        candidate("a", "1.0.0"),
    ];
    catalog[0].checksum = Digest::from_bytes([9; 32]);
    catalog[1].checksum = Digest::from_bytes([10; 32]);
    catalog[2].checksum = Digest::from_bytes([11; 32]);
    catalog.push(catalog[0].clone());
    let selected = &resolve_packages(&[requirement("a", "*")], &catalog).unwrap()[0];
    assert_eq!(selected.version.to_string(), "1.0.0+9");
    assert_eq!(selected.checksum, Digest::from_bytes([9; 32]));
    catalog.push(candidate("a", "1.0.1+0"));
    assert_eq!(
        resolve_packages(&[requirement("a", "*")], &catalog).unwrap()[0]
            .version
            .to_string(),
        "1.0.1+0"
    );
}

#[test]
fn conflict_precedence_and_lexical_reporting() {
    let mut a = candidate("unused", "1.0.0");
    a.checksum = Digest::from_bytes([8; 32]);
    let mut catalog = vec![candidate("unused", "1.0.0"), a];
    assert!(
        matches!(resolve_packages(&[], &catalog), Err(DataError::CandidateConflict { package, version }) if package.as_str() == "unused" && version == "1.0.0")
    );
    let mut conflicting = requirement("a", "*");
    conflicting.kind = PackageKind::Ruleset;
    assert!(
        matches!(resolve_packages(&[requirement("a", "*"), conflicting], &catalog), Err(DataError::PackageKindConflict { package }) if package.as_str() == "a")
    );
    let mut by_kind = candidate("a", "1.10.0");
    by_kind.kind = PackageKind::Ruleset;
    catalog.extend([candidate("a", "1.10.0"), by_kind]);
    let mut by_checksum = candidate("a", "1.2.0");
    by_checksum.checksum = Digest::from_bytes([9; 32]);
    catalog.extend([candidate("a", "1.2.0"), by_checksum]);
    for _ in 0..catalog.len() {
        assert!(
            matches!(resolve_packages(&[requirement("missing", "*")], &catalog), Err(DataError::CandidateConflict { package, version }) if package.as_str() == "a" && version == "1.10.0")
        );
        catalog.rotate_left(1);
    }
    let mut a = candidate("a", "1.0.0");
    a.checksum = Digest::from_bytes([1; 32]);
    let mut b = candidate("b", "1.0.0");
    b.checksum = Digest::from_bytes([1; 32]);
    let mut conflicts = vec![candidate("b", "1.0.0"), b, candidate("a", "1.0.0"), a];
    for _ in 0..conflicts.len() {
        assert!(
            matches!(resolve_packages(&[], &conflicts), Err(DataError::CandidateConflict { package, version }) if package.as_str() == "a" && version == "1.0.0")
        );
        conflicts.rotate_left(1);
    }
}

#[test]
fn unresolved_fields_empty_and_unused_catalog() {
    assert!(resolve_packages(&[], &[]).unwrap().is_empty());
    assert!(resolve_packages(&[], &[candidate("unused", "1.0.0")])
        .unwrap()
        .is_empty());
    let reqs = [
        requirement("z", "*"),
        requirement("a", "^2"),
        requirement("a", "<1"),
        requirement("a", "^2"),
    ];
    match resolve_packages(&reqs, &[candidate("a", "1.0.0")]).unwrap_err() {
        DataError::UnresolvedPackage {
            package,
            requirements,
        } => {
            assert_eq!(package.as_str(), "a");
            assert_eq!(requirements, ["<1", "^2"]);
        }
        error => panic!("{error}"),
    }
    assert!(
        matches!(resolve_packages(&[requirement("missing", "*")], &[]), Err(DataError::UnresolvedPackage { package, requirements }) if package.as_str() == "missing" && requirements == ["*"])
    );
    let mut wrong = candidate("a", "1.0.0");
    wrong.kind = PackageKind::Campaign;
    assert!(matches!(
        resolve_packages(&[requirement("a", "*")], &[wrong]),
        Err(DataError::UnresolvedPackage { .. })
    ));
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]
    #[test]
    fn permutations_preserve_resolution_and_errors(keys in prop::collection::vec(any::<u64>(), 12), conflict in any::<bool>(), kind_conflict in any::<bool>(), unresolved in any::<bool>()) {
        let mut reqs = vec![requirement("b", "^1"), requirement("a", "*"), requirement("a", "<2"), requirement("b", ">=1.0.0")];
        if kind_conflict { let mut r = requirement("a", "*"); r.kind = PackageKind::Ruleset; reqs.push(r); }
        let mut catalog = vec![candidate("b", "1.0.0"), candidate("a", "1.0.0+9"), candidate("a", "1.0.0+10"), candidate("a", "1.0.0+9"), candidate("unused", "2.0.0")];
        if unresolved { catalog.retain(|candidate| candidate.package.as_str() != "b"); }
        if conflict { let mut c = candidate("unused", "2.0.0"); c.checksum = Digest::from_bytes([0;32]); catalog.push(c); }
        let before = resolve_packages(&reqs, &catalog);
        let mut reqs: Vec<_> = reqs.into_iter().enumerate().collect();
        reqs.sort_by_key(|(i,_)| keys[*i]);
        let mut catalog: Vec<_> = catalog.into_iter().enumerate().collect();
        catalog.sort_by_key(|(i,_)| keys[*i + 6]);
        let after = resolve_packages(&reqs.into_iter().map(|(_,r)|r).collect::<Vec<_>>(), &catalog.into_iter().map(|(_,c)|c).collect::<Vec<_>>());
        match (before, after) {
            (Ok(a), Ok(b)) => prop_assert_eq!(a,b),
            (Err(DataError::PackageKindConflict { package:a }), Err(DataError::PackageKindConflict { package:b })) => prop_assert_eq!(a,b),
            (Err(DataError::CandidateConflict { package:a, version:av }), Err(DataError::CandidateConflict { package:b, version:bv })) => prop_assert_eq!((a,av),(b,bv)),
            (Err(DataError::UnresolvedPackage { package:a, requirements:ar }), Err(DataError::UnresolvedPackage { package:b, requirements:br })) => prop_assert_eq!((a,ar),(b,br)),
            (a,b) => prop_assert!(false,"{a:?} != {b:?}"),
        }
    }
}

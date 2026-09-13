#![forbid(unsafe_code)]
mod support;
use crpg_data::*;
use std::collections::BTreeMap;

fn empty_assets() -> AssetsLock {
    AssetsLock {
        assets: BTreeMap::new(),
        note: None,
    }
}
fn record() -> AssetRecord {
    AssetRecord {
        hash: Digest::from_bytes([1; 32]),
        import: BTreeMap::new(),
    }
}
fn package(id: &str) -> ResolvedPackage {
    ResolvedPackage {
        kind: PackageKind::Module,
        package: id.parse().unwrap(),
        version: "1.0.0".parse().unwrap(),
        checksum: Digest::from_bytes([2; 32]),
    }
}

#[test]
fn fixture_lock_bytes_and_known_digest_vector() {
    let files = support::fixture_files();
    let assets_bytes = &files[&"assets/assets.lock".parse().unwrap()];
    assert_eq!(
        assets_bytes,
        b"{\n  \"assets\": {},\n  \"schema\": \"crpg.assets-lock/1\"\n}\n"
    );
    let assets = read_assets_lock(assets_bytes).unwrap();
    let expected: Digest = "957bc137f1abb3cde6cee277d10c099b1dcd2f8814a5fd0e29b1df3506ee44fb"
        .parse()
        .unwrap();
    assert_eq!(assets_lock_digest(&assets).unwrap(), expected);
    assert_eq!(*blake3::hash(assets_bytes).as_bytes(), *expected.as_bytes());
    assert_eq!(write_assets_lock(&assets).unwrap(), *assets_bytes);
    let lock_bytes = &files[&"campaign.lock".parse().unwrap()];
    let lock = read_campaign_lock(lock_bytes).unwrap();
    assert_eq!(lock.assets_lock, expected);
    assert_eq!(write_campaign_lock(&lock).unwrap(), *lock_bytes);
    assert_eq!(make_campaign_lock(&[], &[], &assets).unwrap(), lock);
}

#[test]
fn digest_and_identity_newtypes_validate_exact_text() {
    let digest: Digest = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
        .parse()
        .unwrap();
    assert_eq!(
        digest.as_bytes()[0..8],
        [1, 35, 69, 103, 137, 171, 205, 239]
    );
    assert_eq!(Digest::from_bytes(*digest.as_bytes()), digest);
    for text in [
        "",
        "00",
        "0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF",
    ] {
        assert!(
            matches!(text.parse::<Digest>(), Err(DataError::InvalidDigest { value }) if value == text)
        );
    }
    for text in ["pkg", "a0.b-2"] {
        assert_eq!(text.parse::<PackageId>().unwrap().as_str(), text);
    }
    for text in ["", "A", "-a", "a..b", "a_b", "a.", "a/1", "é"] {
        assert!(matches!(
            text.parse::<PackageId>(),
            Err(DataError::InvalidPackageId { .. })
        ));
    }
    for text in ["a", "A_0/a-b.c", ".hidden", ".../x"] {
        assert_eq!(text.parse::<SourcePath>().unwrap().as_str(), text);
    }
    for text in [
        "", "/a", "a/", "a//b", ".", "..", "a/../b", "a/./b", "C:/a", "a\\b", "é",
    ] {
        assert!(matches!(
            text.parse::<SourcePath>(),
            Err(DataError::InvalidPath { .. })
        ));
    }
    assert!(matches!(
        read_assets_lock(b"{\"schema\":\"crpg.assets-lock/1\",\"assets\":{\"../x\":{}}}"),
        Err(DataError::Malformed { .. })
    ));
}

#[test]
fn lock_arrays_reject_unsorted_and_duplicate_values_on_both_paths() {
    for packages in [
        vec![package("b"), package("a")],
        vec![package("a"), package("a")],
    ] {
        let lock = CampaignLock {
            packages,
            assets_lock: Digest::from_bytes([0; 32]),
            note: None,
        };
        assert!(matches!(
            write_campaign_lock(&lock),
            Err(DataError::InvalidLock { .. })
        ));
        assert!(matches!(
            read_campaign_lock(&canonical_json(&Document::CampaignLock(lock)).unwrap()),
            Err(DataError::InvalidLock { .. })
        ));
    }
    let lock = CampaignLock {
        packages: vec![package("a"), package("b")],
        assets_lock: Digest::from_bytes([0; 32]),
        note: Some("note".into()),
    };
    assert_eq!(
        read_campaign_lock(&write_campaign_lock(&lock).unwrap()).unwrap(),
        lock
    );
}

#[test]
fn asset_roots_case_collisions_and_make_precedence() {
    for paths in [
        vec!["source.bin"],
        vec!["assets/assets.lock"],
        vec!["assets/ASSETS.LOCK"],
        vec!["assets/A", "assets/a"],
        vec!["Assets/x"],
    ] {
        let lock = AssetsLock {
            assets: paths
                .into_iter()
                .map(|p| (p.parse().unwrap(), record()))
                .collect(),
            note: None,
        };
        assert!(matches!(
            write_assets_lock(&lock),
            Err(DataError::InvalidLock { .. })
        ));
        assert!(matches!(
            read_assets_lock(&canonical_json(&Document::AssetsLock(lock.clone())).unwrap()),
            Err(DataError::InvalidLock { .. })
        ));
        let requirements = [PackageRequirement {
            kind: PackageKind::Module,
            package: "missing".parse().unwrap(),
            version: "*".parse().unwrap(),
        }];
        assert!(matches!(
            make_campaign_lock(&requirements, &[], &lock),
            Err(DataError::InvalidLock { .. })
        ));
    }
    let compound = AssetsLock {
        assets: BTreeMap::from([
            ("assets/ASSETS.LOCK".parse().unwrap(), record()),
            ("source.bin".parse().unwrap(), record()),
        ]),
        note: None,
    };
    assert!(
        matches!(write_assets_lock(&compound), Err(DataError::InvalidLock { message }) if message.contains("assets/ASSETS.LOCK"))
    );
    assert!(matches!(
        read_campaign_lock(&write_assets_lock(&empty_assets()).unwrap()),
        Err(DataError::Layout { path: None, .. })
    ));
    let campaign = make_campaign_lock(&[], &[], &empty_assets()).unwrap();
    assert!(matches!(
        read_assets_lock(&write_campaign_lock(&campaign).unwrap()),
        Err(DataError::Layout { path: None, .. })
    ));
}

#[test]
fn every_asset_authority_field_changes_digest() {
    let base = AssetsLock {
        assets: BTreeMap::from([("assets/x".parse().unwrap(), record())]),
        note: None,
    };
    let original = assets_lock_digest(&base).unwrap();
    let mut hash = base.clone();
    hash.assets.values_mut().next().unwrap().hash = Digest::from_bytes([2; 32]);
    let mut import = base.clone();
    import
        .assets
        .values_mut()
        .next()
        .unwrap()
        .import
        .insert("setting".into(), DataValue::Bool(true));
    let mut note = base.clone();
    note.note = Some("annotation\n雪".into());
    for changed in [hash, import, note] {
        assert_ne!(assets_lock_digest(&changed).unwrap(), original);
    }
    let campaign = make_campaign_lock(&[], &[], &base).unwrap();
    let wire: serde_json::Value =
        serde_json::from_slice(&write_campaign_lock(&campaign).unwrap()).unwrap();
    assert!(wire.get("assets").is_none());
    assert!(wire.get("hash").is_none());
}

#[test]
fn pathful_and_pathless_errors_render_logical_locations() {
    assert_eq!(
        DataError::Malformed {
            path: None,
            message: "bad value".into(),
        }
        .to_string(),
        "malformed document: bad value"
    );
    assert_eq!(
        DataError::UnsupportedSchema {
            path: Some("worlds/main.json".parse().unwrap()),
            found: "future/1".into(),
        }
        .to_string(),
        "unsupported schema \"future/1\" at worlds/main.json"
    );
    assert_eq!(
        DataError::Layout {
            path: None,
            message: "missing campaign".into(),
        }
        .to_string(),
        "invalid layout: missing campaign"
    );
}

#![forbid(unsafe_code)]
//! T058a: RFC 6901 pointer edits of one typed document at its current tag.

mod support;
use crpg_core::{Fx16_16, Ulid};
use crpg_data::*;
use proptest::prelude::*;
use serde_json::Value;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

fn engine() -> semver::Version {
    "0.1.0".parse().unwrap()
}
fn id(n: u128) -> Ulid {
    Ulid::from_u128(n)
}
fn path(s: &str) -> SourcePath {
    s.parse().unwrap()
}
fn set(pointer: &str, value: &str) -> PointerEdit {
    PointerEdit::Set {
        pointer: pointer.into(),
        value: value.as_bytes().to_vec(),
    }
}
fn insert(pointer: &str, value: &str) -> PointerEdit {
    PointerEdit::Insert {
        pointer: pointer.into(),
        value: value.as_bytes().to_vec(),
    }
}
fn remove(pointer: &str) -> PointerEdit {
    PointerEdit::Remove {
        pointer: pointer.into(),
    }
}
fn set_bytes(pointer: &str, value: &[u8]) -> PointerEdit {
    PointerEdit::Set {
        pointer: pointer.into(),
        value: value.to_vec(),
    }
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}
fn read_tree(root: &Path) -> BTreeMap<SourcePath, Vec<u8>> {
    fn walk(dir: &Path, root: &Path, out: &mut BTreeMap<SourcePath, Vec<u8>>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            let kind = entry.file_type().unwrap();
            if kind.is_dir() {
                walk(&path, root, out);
            } else if kind.is_file() {
                let name = path.file_name().unwrap().to_string_lossy().into_owned();
                if name == ".gitattributes" {
                    continue;
                }
                let relative = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                let key: SourcePath = relative.parse().unwrap();
                out.insert(key, std::fs::read(&path).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}
/// Every campaign root of `campaign_index`'s fixture set.
fn fixture_roots() -> Vec<PathBuf> {
    let shared = repo_root().join("campaigns/fixtures");
    let mut roots: Vec<PathBuf> = std::fs::read_dir(&shared)
        .unwrap()
        .map(|entry| entry.unwrap())
        .filter(|entry| entry.file_type().unwrap().is_dir())
        .map(|entry| entry.path())
        .collect();
    roots.sort();
    for required in ["combat_basic", "combat_srd"] {
        assert!(roots.contains(&shared.join(required)), "{required}");
    }
    let local = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    for name in [
        "one_area_one_creature",
        "broken_references",
        "migration_v1/campaign",
    ] {
        roots.push(local.join(name));
    }
    roots
}

fn fixture(name: &str) -> Document {
    read_document(&support::fixture_files()[&path(name)]).unwrap()
}
fn creature() -> Document {
    fixture("creatures/creature.json")
}
fn locale() -> Document {
    fixture("locale/en.json")
}
fn campaign() -> Document {
    fixture("campaign.json")
}
fn placements() -> Document {
    fixture("areas/start/placements.json")
}
fn strike() -> Document {
    let bytes =
        std::fs::read(repo_root().join("campaigns/fixtures/combat_basic/abilities/strike.json"))
            .unwrap();
    read_document(&bytes).unwrap()
}
fn migrated_item() -> Document {
    let bytes = std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/migration_v1/campaign/items/item.json"),
    )
    .unwrap();
    read_document(&bytes).unwrap()
}

fn as_creature(document: &mut Document) -> &mut Creature {
    let Document::Creature(c) = document else {
        panic!("creature")
    };
    c
}
fn as_locale(document: &mut Document) -> &mut LocaleDocument {
    let Document::Locale(l) = document else {
        panic!("locale")
    };
    l
}
fn as_placements(document: &mut Document) -> &mut PlacementsDocument {
    let Document::Placements(p) = document else {
        panic!("placements")
    };
    p
}

/// §3.3: `r` is the input's variant, writable, and survives a strict re-read.
fn assert_canonical(input: &Document, r: &Document) {
    assert_eq!(std::mem::discriminant(input), std::mem::discriminant(r));
    let bytes = write_document(r).unwrap();
    assert_eq!(&read_document(&bytes).unwrap(), r);
}

/// Applies an edit that must succeed; the input is unchanged.
fn ok(document: &Document, edit: &PointerEdit) -> Document {
    let before = document.clone();
    let result = edit_document(document, edit)
        .unwrap_or_else(|e| panic!("{edit:?} should succeed, got {e}"));
    assert_eq!(document, &before);
    assert_canonical(document, &result);
    result
}

/// Applies an edit that must fail; the input and its bytes are unchanged.
fn err(document: &Document, edit: &PointerEdit) -> PointerEditError {
    let before = document.clone();
    let bytes = write_document(document).unwrap();
    let error = edit_document(document, edit).expect_err("edit should fail");
    assert_eq!(document, &before);
    assert_eq!(write_document(document).unwrap(), bytes);
    error
}

fn is_value(error: &PointerEditError) -> bool {
    matches!(error, PointerEditError::Value { .. })
}
fn is_document(error: &PointerEditError) -> bool {
    matches!(error, PointerEditError::Document { .. })
}

fn second_placement() -> Placement {
    Placement {
        id: id(9),
        slug: "second".into(),
        name: "fixture.spawn".into(),
        note: None,
        prefab: id(4),
        transform: Transform {
            position: [Fx16_16::ONE; 3],
            rotation: [Fx16_16::ZERO; 3],
            scale: [Fx16_16::ONE; 3],
        },
        overrides: BTreeMap::new(),
    }
}
fn text(value: &impl serde::Serialize) -> String {
    String::from_utf8(canonical_json(value).unwrap()).unwrap()
}

#[test]
fn set_replaces_existing_member() {
    let input = creature();
    let mut expected = input.clone();
    as_creature(&mut expected).slug = "goblin".into();
    assert_eq!(ok(&input, &set("/slug", "\"goblin\"")), expected);
}

#[test]
fn set_adds_absent_object_member() {
    let input = locale();
    let mut expected = input.clone();
    as_locale(&mut expected)
        .strings
        .insert("mvp.npc".into(), "Mayor".into());
    assert_eq!(ok(&input, &set("/strings/mvp.npc", "\"Mayor\"")), expected);
}

#[test]
fn set_replaces_array_element() {
    let input = creature();
    let mut expected = input.clone();
    as_creature(&mut expected).tags = vec!["boss".into()];
    assert_eq!(ok(&input, &set("/tags/0", "\"boss\"")), expected);
}

#[test]
fn set_identical_value_returns_equal_document() {
    let input = creature();
    assert_eq!(ok(&input, &set("/slug", "\"creature\"")), input);
}

#[test]
fn insert_appends_and_inserts_at_index() {
    let input = creature();
    let mut appended = input.clone();
    as_creature(&mut appended).tags = vec!["fixture".into(), "new".into()];
    let mut prepended = input.clone();
    as_creature(&mut prepended).tags = vec!["new".into(), "fixture".into()];
    assert_eq!(ok(&input, &insert("/tags/-", "\"new\"")), appended);
    assert_eq!(ok(&input, &insert("/tags/0", "\"new\"")), prepended);
    assert_eq!(ok(&input, &insert("/tags/1", "\"new\"")), appended);

    let input = placements();
    let mut expected = input.clone();
    as_placements(&mut expected)
        .placements
        .push(second_placement());
    assert_eq!(
        ok(&input, &insert("/placements/-", &text(&second_placement()))),
        expected
    );
}

#[test]
fn remove_member_and_array_element() {
    let input = campaign();
    let mut expected = input.clone();
    let Document::Campaign(manifest) = &mut expected else {
        panic!()
    };
    assert!(manifest.note.is_some());
    manifest.note = None;
    assert_eq!(ok(&input, &remove("/_note")), expected);

    let input = creature();
    let mut expected = input.clone();
    as_creature(&mut expected).tags.clear();
    assert_eq!(ok(&input, &remove("/tags/0")), expected);

    let mut input = placements();
    as_placements(&mut input)
        .placements
        .push(second_placement());
    let mut expected = input.clone();
    as_placements(&mut expected).placements.remove(0);
    let result = ok(&input, &remove("/placements/0"));
    assert_eq!(result, expected);
    let mut result = result;
    assert_eq!(as_placements(&mut result).placements[0].id, id(9));
}

#[test]
fn escapes_decode_per_rfc6901() {
    assert_eq!(
        pointer_tokens("/a~1b/c~0d/~01/").unwrap(),
        ["a/b", "c~d", "~1", ""]
    );
    let input = locale();
    for (pointer, key) in [
        ("/strings/a~1b", "a/b"),
        ("/strings/a~0b", "a~b"),
        ("/strings/~01", "~1"),
    ] {
        let mut expected = input.clone();
        as_locale(&mut expected)
            .strings
            .insert(key.into(), "v".into());
        assert_eq!(ok(&input, &set(pointer, "\"v\"")), expected, "{pointer}");
    }

    let both = ok(
        &ok(&input, &set("/strings/a~1b", "\"v\"")),
        &set("/strings/a~0b", "\"v\""),
    );
    let mut expected = input.clone();
    as_locale(&mut expected)
        .strings
        .insert("a~b".into(), "v".into());
    assert_eq!(ok(&both, &remove("/strings/a~1b")), expected);
}

#[test]
fn empty_tokens_name_empty_members() {
    assert_eq!(pointer_tokens("/").unwrap(), [""]);
    assert_eq!(pointer_tokens("//").unwrap(), ["", ""]);
    assert_eq!(pointer_tokens("/a/").unwrap(), ["a", ""]);
    let input = locale();
    let mut expected = input.clone();
    as_locale(&mut expected)
        .strings
        .insert(String::new(), "e".into());
    assert_eq!(ok(&input, &set("/strings/", "\"e\"")), expected);
}

#[test]
fn syntax_errors_are_invalid_pointer() {
    let input = locale();
    for pointer in ["slug", "#/slug", "/a~", "/a~2", "/~", "/strings/~x"] {
        assert_eq!(
            pointer_tokens(pointer),
            Err(PointerEditError::InvalidPointer),
            "{pointer}"
        );
        assert_eq!(
            err(&input, &set(pointer, "\"x\"")),
            PointerEditError::InvalidPointer,
            "{pointer}"
        );
    }
    // Positive control: the same target spelled correctly.
    ok(&input, &set("/strings/~0x", "\"x\""));
}

#[test]
fn array_index_tokens_are_strict() {
    let input = creature();
    for token in ["01", "00", "+0", "-1", " 0", "0 ", "1e0", "0x0", ""] {
        let pointer = format!("/tags/{token}");
        assert_eq!(
            err(&input, &set(&pointer, "\"x\"")),
            PointerEditError::InvalidPointer,
            "{pointer:?}"
        );
        assert_eq!(
            err(&input, &remove(&pointer)),
            PointerEditError::InvalidPointer,
            "{pointer:?}"
        );
        assert_eq!(
            err(&input, &insert(&pointer, "\"x\"")),
            PointerEditError::InvalidPointer,
            "{pointer:?}"
        );
    }
    for edit in [
        set("/tags/18446744073709551616", "\"x\""),
        remove("/tags/18446744073709551616"),
        insert("/tags/18446744073709551616", "\"x\""),
        set("/tags/-", "\"x\""),
        remove("/tags/-"),
    ] {
        assert_eq!(err(&input, &edit), PointerEditError::NotFound, "{edit:?}");
    }
    let placements = placements();
    assert_eq!(
        err(&placements, &set("/placements/-/slug", "\"x\"")),
        PointerEditError::NotFound
    );
    assert_eq!(
        err(&placements, &set("/placements/01/slug", "\"x\"")),
        PointerEditError::InvalidPointer
    );
    // Positive controls: a well-formed index, and the same spelling as an
    // object member name.
    ok(&placements, &set("/placements/0/slug", "\"x\""));
    ok(&input, &set("/tags/0", "\"x\""));
    let mut expected = input.clone();
    as_creature(&mut expected)
        .stats
        .insert("01".into(), Fx16_16::ONE);
    assert_eq!(ok(&input, &set("/stats/01", "65536")), expected);
}

#[test]
fn missing_targets_are_not_found() {
    let input = creature();
    for edit in [
        set("/nosuch/x", "1"),
        set("/slug/x", "1"),
        set("/tags/9", "\"x\""),
        remove("/nosuch"),
        remove("/tags/1"),
    ] {
        assert_eq!(err(&input, &edit), PointerEditError::NotFound, "{edit:?}");
    }
    ok(&input, &remove("/tags/0"));
}

#[test]
fn insert_requires_array_parent_and_index_in_range() {
    let input = creature();
    for edit in [
        insert("/slug/0", "\"x\""),
        insert("/stats/x", "65536"),
        insert("/tags/2", "\"x\""),
    ] {
        assert_eq!(err(&input, &edit), PointerEditError::NotFound, "{edit:?}");
    }
    let mut expected = input.clone();
    as_creature(&mut expected).tags.push("x".into());
    assert_eq!(ok(&input, &insert("/tags/1", "\"x\"")), expected);
}

#[test]
fn root_pointer_is_refused() {
    assert_eq!(pointer_tokens("").unwrap(), Vec::<String>::new());
    let input = creature();
    let whole = String::from_utf8(write_document(&input).unwrap()).unwrap();
    for edit in [set("", &whole), insert("", &whole), remove("")] {
        assert_eq!(
            err(&input, &edit),
            PointerEditError::RootPointer,
            "{edit:?}"
        );
    }
    ok(&input, &set("/slug", "\"x\""));
}

#[test]
fn schema_tag_is_refused() {
    let input = creature();
    for edit in [
        set("/schema", "\"crpg.item/2\""),
        insert("/schema", "\"crpg.item/2\""),
        remove("/schema"),
        set("/schema/x", "1"),
    ] {
        assert_eq!(err(&input, &edit), PointerEditError::SchemaTag, "{edit:?}");
    }
    let input = locale();
    let result = ok(&input, &set("/strings/schema", "\"crpg.item/1\""));
    let written: Value = serde_json::from_slice(&write_document(&result).unwrap()).unwrap();
    assert_eq!(written["schema"], "crpg.locale/1");
    assert_eq!(written["strings"]["schema"], "crpg.item/1");
}

#[test]
fn pointer_length_bound() {
    let input = creature();
    let at_bound = format!("/{}", "a".repeat(8_191));
    assert_eq!(at_bound.len(), MAX_EDIT_POINTER_BYTES);
    assert_eq!(pointer_tokens(&at_bound).unwrap().len(), 1);
    assert_eq!(err(&input, &remove(&at_bound)), PointerEditError::NotFound);
    let over = format!("/{}", "a".repeat(8_192));
    assert_eq!(pointer_tokens(&over), Err(PointerEditError::PointerTooLong));
    assert_eq!(
        err(&input, &remove(&over)),
        PointerEditError::PointerTooLong
    );
    let wide = format!("/{}", "\u{20ac}".repeat(2_731));
    assert_eq!(wide.len(), 8_194);
    assert_eq!(wide.chars().count(), 2_732);
    assert_eq!(pointer_tokens(&wide), Err(PointerEditError::PointerTooLong));
    assert_eq!(
        err(&input, &remove(&wide)),
        PointerEditError::PointerTooLong
    );
}

#[test]
fn malformed_values_are_refused() {
    let input = creature();
    let values: [&[u8]; 8] = [
        b"1.5",
        b"1e3",
        b"-0.0",
        br#"{"a":1,"a":2}"#,
        br#""unterminated"#,
        b"",
        &[0xff],
        b"1 2",
    ];
    for value in values {
        let error = err(&input, &set_bytes("/slug", value));
        let PointerEditError::Value { message } = &error else {
            panic!("{value:?}: {error:?}")
        };
        assert!(
            message.starts_with("malformed document: "),
            "{value:?}: {message}"
        );
    }
    ok(&input, &set_bytes("/slug", br#""ok""#));
}

#[test]
fn typed_decode_failures_are_document_errors() {
    let input = creature();
    for edit in [
        set("/stats/health", "\"x\""),
        set("/bogus", "1"),
        remove("/slug"),
    ] {
        let error = err(&input, &edit);
        assert!(is_document(&error), "{edit:?}: {error:?}");
    }
    let ability = strike();
    let error = err(&ability, &set("/cost", "0"));
    let PointerEditError::Document { message } = &error else {
        panic!("{error:?}")
    };
    assert!(
        message.contains("ability must spend something"),
        "{message}"
    );
    // Positive controls.
    ok(&input, &set("/stats/health", "65536"));
    ok(&ability, &set("/cost", "1"));
}

/// Nested `DataValue::List` text: each level adds an object and an array.
fn nested_list(levels: usize) -> Vec<u8> {
    let mut value = DataValue::Bool(true);
    for _ in 0..levels {
        value = DataValue::List(vec![value]);
    }
    serde_json::to_vec(&value).unwrap()
}

#[test]
fn writer_depth_limit_is_a_document_error() {
    let input = placements();
    let pointer = "/placements/0/overrides/deep";
    let mut first_failing = None;
    for levels in 1..=200 {
        let value = nested_list(levels);
        match edit_document(&input, &set_bytes(pointer, &value)) {
            Ok(result) => assert_canonical(&input, &result),
            Err(error) => {
                assert!(is_document(&error), "{levels}: {error:?}");
                first_failing = Some(levels);
                break;
            }
        }
    }
    let levels = first_failing.expect("some depth exceeds the writer limit");
    // The failing value's own JSON nesting (an object and an array per level,
    // plus the innermost object) is within the strict parser's limit.
    let value_depth = 2 * levels + 1;
    assert!(value_depth <= 128, "{value_depth}");
    assert!(is_document(&err(
        &input,
        &set_bytes(pointer, &nested_list(levels))
    )));
    let mut shallower = ok(&input, &set_bytes(pointer, &nested_list(levels - 1)));
    assert!(as_placements(&mut shallower).placements[0]
        .overrides
        .contains_key("deep"));
}

#[test]
fn precedence_is_first_failure_wins() {
    let input = creature();
    let bad = "1.5";
    let typed_invalid = "1";
    let long_bad_syntax = "x".repeat(MAX_EDIT_POINTER_BYTES + 1);
    let cases = [
        (set(&long_bad_syntax, bad), PointerEditError::PointerTooLong),
        (set("slug", bad), PointerEditError::InvalidPointer),
        (set("", bad), PointerEditError::RootPointer),
        (set("/schema", bad), PointerEditError::SchemaTag),
        (set("/nosuch/x", typed_invalid), PointerEditError::NotFound),
        (
            set("/tags/01", typed_invalid),
            PointerEditError::InvalidPointer,
        ),
    ];
    for (edit, expected) in cases {
        assert_eq!(err(&input, &edit), expected, "{edit:?}");
    }
    for pointer in ["/nosuch/x", "/tags/01"] {
        let error = err(&input, &set(pointer, bad));
        assert!(is_value(&error), "{pointer}: {error:?}");
    }
    // The typed-invalid value alone, at a valid target, is a Document error.
    assert!(is_document(&err(&input, &set("/tags/0", typed_invalid))));
}

#[test]
fn edits_never_migrate_or_retag() {
    let current = |family: &str| {
        let version = schema_versions()
            .iter()
            .find(|v| v.schema_type == family)
            .unwrap();
        format!("{}/{}", version.schema_type, version.current)
    };
    for (input, family) in [(migrated_item(), "crpg.item"), (strike(), "crpg.ability")] {
        let result = ok(&input, &set("/slug", "\"renamed\""));
        let written: Value = serde_json::from_slice(&write_document(&result).unwrap()).unwrap();
        assert_eq!(written["schema"], current(family).as_str());
        assert_eq!(written["slug"], "renamed");
    }
    assert_eq!(current("crpg.item"), "crpg.item/2");
    assert_eq!(current("crpg.ability"), "crpg.ability/2");
}

fn escape(token: &str) -> String {
    token.replace('~', "~0").replace('/', "~1")
}

#[test]
fn identity_set_is_lossless_for_every_fixture_document() {
    let mut families = std::collections::BTreeSet::new();
    let mut edits = 0;
    for root in fixture_roots() {
        let loaded = load_campaign(&read_tree(&root), &engine())
            .unwrap_or_else(|e| panic!("{}: {e}", root.display()));
        for (source, document) in &loaded.documents {
            let value = serde_json::to_value(document).unwrap();
            let Value::Object(members) = &value else {
                panic!("{source}")
            };
            families.insert(members["schema"].as_str().unwrap().to_owned());
            for (key, member) in members {
                if key == "schema" {
                    continue;
                }
                let edit = set_bytes(
                    &format!("/{}", escape(key)),
                    &canonical_json(member).unwrap(),
                );
                assert_eq!(&ok(document, &edit), document, "{source} /{key}");
                edits += 1;
            }
        }
    }
    for family in ["crpg.campaign-lock/1", "crpg.assets-lock/1"] {
        assert!(families.contains(family), "{family}");
    }
    assert!(edits > 100, "{edits}");
}

#[test]
fn edit_does_not_police_identity() {
    let input = creature();
    let mut expected = input.clone();
    as_creature(&mut expected).id = id(99);
    assert_eq!(ok(&input, &set("/id", &text(&id(99)))), expected);
}

#[test]
fn errors_leave_input_unchanged() {
    let input = creature();
    let cases = [
        remove(&format!("/{}", "a".repeat(MAX_EDIT_POINTER_BYTES))),
        remove("slug"),
        remove(""),
        remove("/schema"),
        remove("/nosuch"),
        set("/slug", "1.5"),
        remove("/slug"),
    ];
    let variants: Vec<&str> = cases
        .iter()
        // `err` asserts the document and its canonical bytes are unchanged.
        .map(|edit| match err(&input, edit) {
            PointerEditError::PointerTooLong => "PointerTooLong",
            PointerEditError::InvalidPointer => "InvalidPointer",
            PointerEditError::RootPointer => "RootPointer",
            PointerEditError::SchemaTag => "SchemaTag",
            PointerEditError::NotFound => "NotFound",
            PointerEditError::Value { .. } => "Value",
            PointerEditError::Document { .. } => "Document",
        })
        .collect();
    assert_eq!(
        variants,
        [
            "PointerTooLong",
            "InvalidPointer",
            "RootPointer",
            "SchemaTag",
            "NotFound",
            "Value",
            "Document"
        ]
    );
}

#[test]
fn error_display_strings_are_pinned() {
    let cases = [
        (
            PointerEditError::PointerTooLong,
            "json pointer exceeds 8192 bytes",
        ),
        (PointerEditError::InvalidPointer, "invalid json pointer"),
        (
            PointerEditError::RootPointer,
            "json pointer must not be empty",
        ),
        (
            PointerEditError::SchemaTag,
            "the schema tag cannot be edited",
        ),
        (PointerEditError::NotFound, "json pointer target not found"),
        (
            PointerEditError::Value {
                message: "m1".into(),
            },
            "invalid value: m1",
        ),
        (
            PointerEditError::Document {
                message: "m2".into(),
            },
            "edited document is invalid: m2",
        ),
    ];
    for (error, text) in cases {
        assert_eq!(error.to_string(), text);
    }
    let error = err(&creature(), &set("/slug", ""));
    assert!(error
        .to_string()
        .starts_with("invalid value: malformed document: "));
    let error = err(&creature(), &remove("/slug"));
    assert!(error
        .to_string()
        .starts_with("edited document is invalid: malformed document: "));
}

const TOKENS: [&str; 12] = [
    "tags", "stats", "strings", "slug", "0", "1", "-", "01", "~0", "~1", "schema", "",
];
const VALUES: [&str; 12] = [
    "\"x\"",
    "1",
    "65536",
    "[]",
    "{}",
    "null",
    "[\"a\"]",
    "true",
    "1.5",
    "",
    "{\"a\":1,\"a\":2}",
    "[1,",
];

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn random_edits_are_pure_and_canonical(
        tokens in prop::collection::vec(prop::sample::select(TOKENS.to_vec()), 0..=4),
        value in prop::sample::select(VALUES.to_vec()),
        kind in 0usize..3,
        use_locale in any::<bool>(),
    ) {
        let pointer: String = tokens.iter().map(|t| format!("/{t}")).collect();
        let edit = match kind {
            0 => set(&pointer, value),
            1 => insert(&pointer, value),
            _ => remove(&pointer),
        };
        let input = if use_locale { locale() } else { creature() };
        let before = input.clone();
        let first = edit_document(&input, &edit);
        prop_assert_eq!(&input, &before);
        if let Ok(result) = &first {
            assert_canonical(&input, result);
        }
        let second = edit_document(&input, &edit);
        prop_assert_eq!(&input, &before);
        prop_assert_eq!(first, second);
    }
}

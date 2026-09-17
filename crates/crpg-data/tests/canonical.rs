#![forbid(unsafe_code)]
use crpg_data::*;
use proptest::prelude::*;
use serde::ser::{Serialize, SerializeMap, Serializer};
use serde_json::json;
use std::collections::BTreeMap;

struct Ordered<'a>(&'a [(&'a str, serde_json::Value)]);
impl Serialize for Ordered<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut map = s.serialize_map(Some(self.0.len()))?;
        for (key, value) in self.0 {
            map.serialize_entry(key, value)?;
        }
        map.end()
    }
}

#[derive(Clone, Copy)]
struct ReverseMap;

impl Serialize for ReverseMap {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut map = s.serialize_map(Some(2))?;
        map.serialize_entry("z", &2)?;
        map.serialize_entry("a", &1)?;
        map.end()
    }
}

struct NestedReverseMap;

impl Serialize for NestedReverseMap {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut map = s.serialize_map(Some(1))?;
        map.serialize_entry("outer", &[ReverseMap])?;
        map.end()
    }
}

#[test]
fn literal_byte_oracle() {
    let value = Ordered(&[
        ("z", json!([{"z": 2, "a": 1}, {"b": [], "a": {}}])),
        ("text", json!("雪\n\t\u{0001}\"\\")),
        ("null", json!(null)),
        ("min", json!(i64::MIN)),
        ("max", json!(u64::MAX)),
    ]);
    let expected = concat!(
        "{\n",
        "  \"max\": 18446744073709551615,\n",
        "  \"min\": -9223372036854775808,\n",
        "  \"null\": null,\n",
        "  \"text\": \"雪\\n\\t\\u0001\\\"\\\\\",\n",
        "  \"z\": [\n",
        "    {\n",
        "      \"a\": 1,\n",
        "      \"z\": 2\n",
        "    },\n",
        "    {\n",
        "      \"a\": {},\n",
        "      \"b\": []\n",
        "    }\n",
        "  ]\n",
        "}\n"
    );
    let bytes = canonical_json(&value).unwrap();
    assert_eq!(bytes, expected.as_bytes());
    let parsed: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(canonical_json(&parsed).unwrap(), bytes);
    assert!(!bytes.contains(&b'\r'));
    assert!(!bytes.starts_with(&[239, 187, 191]));
}

#[test]
fn recursively_sorts_custom_serializer_maps() {
    assert_eq!(
        canonical_json(&NestedReverseMap).unwrap(),
        b"{\n  \"outer\": [\n    {\n      \"a\": 1,\n      \"z\": 2\n    }\n  ]\n}\n"
    );
}

#[test]
fn rejects_illegal_json_before_schema_selection() {
    let inputs: &[&[u8]] = &[
        b"{\"schema\":\"future/1\",\"x\":1.0}",
        b"{\"schema\":\"future/1\",\"x\":1e0}",
        b"{\"schema\":\"future/1\",\"x\":-0}",
        b"{\"schema\":\"future/1\",\"x\":18446744073709551616}",
        b"{\"schema\":\"future/1\",\"x\":-9223372036854775809}",
        b"{\"schema\":\"future/1\",\"schema\":\"future/1\"}",
        b"{\"schema\":\"future/1\",\"x\":[{\"a\":1,\"a\":2}]}",
        b"{\"schema\":\"future/1\",",
        b"{\"schema\":\"crpg.item/1\",\"x\":1.0}",
        b"{\"schema\":\"crpg.item/1\",\"x\":-0}",
        b"\xff",
        b"\xef\xbb\xbf{}",
        b"{} {}",
    ];
    for bytes in inputs {
        assert!(
            matches!(
                read_document(bytes),
                Err(DataError::Malformed { path: None, .. })
            ),
            "{bytes:?}"
        );
    }
    for bytes in [b"null".as_slice(), b"[]", b"{}", b"{\"schema\":1}"] {
        assert!(matches!(
            read_document(bytes),
            Err(DataError::Malformed { .. })
        ));
    }
    let duplicate = Ordered(&[("same", json!(1)), ("same", json!(2))]);
    assert!(matches!(
        canonical_json(&duplicate),
        Err(DataError::Malformed { .. })
    ));
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]
    #[test]
    fn insertion_order_is_irrelevant(values in prop::collection::btree_map("[a-z]{1,8}", any::<i64>(), 0..12)) {
        let forward: Vec<_> = values.iter().map(|(k,v)| (k.as_str(), json!({"z":v,"a":[v,0]}))).collect();
        let reverse: Vec<_> = forward.iter().rev().cloned().collect();
        prop_assert_eq!(canonical_json(&Ordered(&forward)).unwrap(), canonical_json(&Ordered(&reverse)).unwrap());
        let ordinary: BTreeMap<_,_> = forward.into_iter().collect();
        prop_assert_eq!(canonical_json(&ordinary).unwrap(), canonical_json(&Ordered(&reverse)).unwrap());
    }
}

//! Integer-only canonical JSON with duplicate-key rejection.

use crate::{error::malformed, DataError};
use serde::{
    de::{self, MapAccess, SeqAccess, Visitor},
    Deserialize, Deserializer, Serialize,
};
use serde_json::Value;
use std::{collections::BTreeMap, fmt};

struct StrictValue(Value);

impl<'de> Deserialize<'de> for StrictValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct IntegerJson;
        impl<'de> Visitor<'de> for IntegerJson {
            type Value = StrictValue;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("JSON containing only integer numbers and unique object keys")
            }
            fn visit_bool<E: de::Error>(self, v: bool) -> Result<Self::Value, E> {
                Ok(StrictValue(v.into()))
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
                Ok(StrictValue(v.into()))
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
                Ok(StrictValue(v.into()))
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                Ok(StrictValue(v.into()))
            }
            fn visit_string<E: de::Error>(self, v: String) -> Result<Self::Value, E> {
                Ok(StrictValue(v.into()))
            }
            fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(StrictValue(Value::Null))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                let mut values = Vec::new();
                while let Some(StrictValue(value)) = seq.next_element()? {
                    values.push(value);
                }
                Ok(StrictValue(Value::Array(values)))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut values = BTreeMap::new();
                while let Some(key) = map.next_key::<String>()? {
                    if values.contains_key(&key) {
                        return Err(de::Error::custom(format!("duplicate key {key:?}")));
                    }
                    let StrictValue(value) = map.next_value()?;
                    values.insert(key, value);
                }
                // Insertion is lexical even if preserve_order is feature-unified.
                Ok(StrictValue(Value::Object(values.into_iter().collect())))
            }
        }
        deserializer.deserialize_any(IntegerJson)
    }
}

pub(crate) fn parse(bytes: &[u8]) -> Result<Value, DataError> {
    let text = std::str::from_utf8(bytes).map_err(malformed)?;
    let mut deserializer = serde_json::Deserializer::from_str(text);
    let StrictValue(value) = StrictValue::deserialize(&mut deserializer).map_err(malformed)?;
    deserializer.end().map_err(malformed)?;
    Ok(value)
}

/// Encodes integer JSON with recursive lexical keys, ordered arrays and one LF.
///
/// This encoding utility accepts any JSON root; document validation is separate.
pub fn canonical_json<T: Serialize + ?Sized>(value: &T) -> Result<Vec<u8>, DataError> {
    let raw = serde_json::to_vec(value).map_err(malformed)?;
    let ordered = parse(&raw)?;
    let mut bytes = serde_json::to_vec_pretty(&ordered).map_err(malformed)?;
    bytes.push(b'\n');
    Ok(bytes)
}

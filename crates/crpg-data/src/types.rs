//! Shared authored values and distinct identity domains.

use crate::DataError;
use crpg_core::{Fx16_16, Ulid};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fmt, str::FromStr};

/// Immutable publication coordinate, distinct from an object ULID or slug.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, JsonSchema)]
#[schemars(with = "crate::schema::PackageText")]
pub struct PackageId(String);

impl PackageId {
    /// Borrows the validated coordinate.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for PackageId {
    type Err = DataError;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.split(['.', '-']).all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        }) {
            Ok(Self(value.into()))
        } else {
            Err(DataError::InvalidPackageId {
                value: value.into(),
            })
        }
    }
}

/// Portable campaign-relative logical path, without filesystem normalization.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, JsonSchema)]
#[schemars(with = "crate::schema::PathText")]
pub struct SourcePath(String);

impl SourcePath {
    /// Borrows the validated logical path.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for SourcePath {
    type Err = DataError;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.split('/').all(|part| {
            !part.is_empty()
                && part != "."
                && part != ".."
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        }) {
            Ok(Self(value.into()))
        } else {
            Err(DataError::InvalidPath {
                value: value.into(),
            })
        }
    }
}

/// A BLAKE3 content digest encoded as exactly 64 lowercase hex characters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, JsonSchema)]
#[schemars(with = "crate::schema::DigestText")]
pub struct Digest([u8; 32]);

impl Digest {
    /// Constructs a digest from all 32 supplied bytes.
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
    /// Borrows the complete digest bytes.
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl FromStr for Digest {
    type Err = DataError;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.len() != 64
            || !value
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(DataError::InvalidDigest {
                value: value.into(),
            });
        }
        let mut bytes = [0; 32];
        for (out, pair) in bytes.iter_mut().zip(value.as_bytes().as_chunks::<2>().0) {
            let digit = |b: u8| {
                if b.is_ascii_digit() {
                    b - b'0'
                } else {
                    b - b'a' + 10
                }
            };
            *out = digit(pair[0]) * 16 + digit(pair[1]);
        }
        Ok(Self(bytes))
    }
}

impl fmt::Display for PackageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl fmt::Display for SourcePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl fmt::Display for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

macro_rules! text_serde {
    ($($ty:ty),+ $(,)?) => { $(
        impl Serialize for $ty {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.collect_str(self)
            }
        }
        impl<'de> Deserialize<'de> for $ty {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                String::deserialize(deserializer)?.parse().map_err(serde::de::Error::custom)
            }
        }
    )+ };
}
text_serde!(PackageId, SourcePath, Digest);

pub(crate) fn required_nullable<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> Result<Option<T>, D::Error> {
    Option::<T>::deserialize(d)
}

/// Explicitly tagged values; signed and unsigned integers remain distinct.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "type",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum DataValue {
    /// Boolean value.
    Bool(bool),
    /// Signed integer.
    Integer(i64),
    /// Unsigned integer.
    Unsigned(u64),
    /// Raw fixed-point integer on the wire.
    Fixed(#[schemars(with = "i32")] Fx16_16),
    /// Literal text, including Unicode.
    Text(String),
    /// Authored-object reference.
    ObjectRef(#[schemars(with = "crate::schema::UlidText")] Ulid),
    /// Ordered heterogeneous values.
    List(Vec<DataValue>),
    /// Lexically keyed heterogeneous values.
    Map(BTreeMap<String, DataValue>),
}

/// Declarative value category; containers hold tagged heterogeneous values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ValueType {
    /// Boolean.
    Bool,
    /// Signed integer.
    Integer,
    /// Unsigned integer.
    Unsigned,
    /// Fixed-point value.
    Fixed,
    /// Text.
    Text,
    /// Authored-object reference.
    ObjectRef,
    /// Ordered tagged values.
    List,
    /// Named tagged values.
    Map,
}

/// Ownership scope of a declared variable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum VariableScope {
    /// Campaign state.
    Campaign,
    /// World state.
    World,
    /// Area state.
    Area,
    /// Graph-local state.
    Graph,
}

/// Authored variable declaration; default-type consistency is a T011 check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VarDecl {
    /// Symbolic variable name.
    pub name: String,
    /// Declared category.
    pub value_type: ValueType,
    /// Initial tagged value.
    pub default: DataValue,
    /// Owning scope.
    pub scope: VariableScope,
}

/// Authored fixed-point transform, independent of runtime spatial storage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Transform {
    /// XYZ position.
    #[schemars(with = "[i32; 3]")]
    pub position: [Fx16_16; 3],
    /// Euler XYZ rotation in degrees.
    #[schemars(with = "[i32; 3]")]
    pub rotation: [Fx16_16; 3],
    /// Dimensionless XYZ scale.
    #[schemars(with = "[i32; 3]")]
    pub scale: [Fx16_16; 3],
}

/// Authored fixed-point bounding coordinates, without geometry validation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Bounds {
    /// Minimum XYZ coordinates.
    #[schemars(with = "[i32; 3]")]
    pub min: [Fx16_16; 3],
    /// Maximum XYZ coordinates.
    #[schemars(with = "[i32; 3]")]
    pub max: [Fx16_16; 3],
}

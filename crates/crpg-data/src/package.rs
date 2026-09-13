//! Pure flat semver resolution and separate source/package lock authorities.

use crate::{error::invalid_lock, DataError, DataValue, Digest, Document, PackageId, SourcePath};
use schemars::JsonSchema;
use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Publication kind, in the specified catalog normalization order.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum PackageKind {
    /// Campaign package.
    Campaign,
    /// Reusable content module.
    Module,
    /// Ruleset package.
    Ruleset,
}

/// One authored flat dependency constraint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PackageRequirement {
    /// Required package category.
    pub kind: PackageKind,
    /// Immutable coordinate.
    pub package: PackageId,
    /// Accepted semantic version range.
    #[schemars(with = "crate::schema::RequirementText")]
    pub version: VersionReq,
}

/// Caller-supplied catalog entry; no registry is consulted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PackageCandidate {
    /// Package category.
    pub kind: PackageKind,
    /// Immutable coordinate.
    pub package: PackageId,
    /// Exact semantic version including build metadata.
    #[schemars(with = "crate::schema::VersionText")]
    pub version: Version,
    /// Supplied package content digest.
    pub checksum: Digest,
}

/// Selected exact package version and supplied checksum.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResolvedPackage {
    /// Package category.
    pub kind: PackageKind,
    /// Immutable coordinate.
    pub package: PackageId,
    /// Exact locked version.
    #[schemars(with = "crate::schema::VersionText")]
    pub version: Version,
    /// Supplied package content digest.
    pub checksum: Digest,
}

/// Authority over package resolution and the canonical assets-lock digest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CampaignLock {
    /// Unique packages, already strictly ascending by package id.
    pub packages: Vec<ResolvedPackage>,
    /// BLAKE3 of complete canonical assets.lock document bytes.
    pub assets_lock: Digest,
    /// Preserved annotation.
    #[serde(rename = "_note", default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Source-asset digest and tagged import settings supplied by the caller.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AssetRecord {
    /// Source content digest; the library does not read the source asset.
    pub hash: Digest,
    /// Named tagged import settings.
    pub import: BTreeMap<String, DataValue>,
}

/// Source-asset authority, separate from package and final packaged-byte locks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AssetsLock {
    /// Campaign-relative source paths and their records.
    pub assets: BTreeMap<SourcePath, AssetRecord>,
    /// Preserved annotation included in the canonical digest.
    #[serde(rename = "_note", default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

fn grouped_requirements(
    requirements: &[PackageRequirement],
) -> BTreeMap<&PackageId, Vec<&PackageRequirement>> {
    let mut grouped: BTreeMap<_, Vec<_>> = BTreeMap::new();
    for requirement in requirements {
        grouped
            .entry(&requirement.package)
            .or_default()
            .push(requirement);
    }
    grouped
}

/// Resolves a flat supplied catalog deterministically, intersecting repeated ranges.
///
/// Kind conflicts precede all candidate conflicts, which precede no-match errors.
/// Build-metadata precedence ties select the lexically greatest full version text.
pub fn resolve_packages(
    requirements: &[PackageRequirement],
    candidates: &[PackageCandidate],
) -> Result<Vec<ResolvedPackage>, DataError> {
    let grouped = grouped_requirements(requirements);
    for (package, requirements) in &grouped {
        if requirements.iter().any(|r| r.kind != requirements[0].kind) {
            return Err(DataError::PackageKindConflict {
                package: (*package).clone(),
            });
        }
    }
    // Group by the reporting key so the first conflict is independent of kind
    // and input order; kind/checksum agreement collapses identical candidates.
    let mut catalog: BTreeMap<(&PackageId, String), &PackageCandidate> = BTreeMap::new();
    let mut conflicts = BTreeSet::new();
    for candidate in candidates {
        let key = (&candidate.package, candidate.version.to_string());
        if let Some(previous) = catalog.get(&key) {
            if previous.kind != candidate.kind || previous.checksum != candidate.checksum {
                conflicts.insert(key);
            }
        } else {
            catalog.insert(key, candidate);
        }
    }
    if let Some((package, version)) = conflicts.into_iter().next() {
        return Err(DataError::CandidateConflict {
            package: package.clone(),
            version,
        });
    }
    let mut resolved = Vec::new();
    for (package, requirements) in grouped {
        let selected = catalog
            .values()
            .copied()
            .filter(|candidate| {
                &candidate.package == package
                    && candidate.kind == requirements[0].kind
                    && requirements
                        .iter()
                        .all(|r| r.version.matches(&candidate.version))
            })
            .max_by(|a, b| {
                a.version
                    .cmp_precedence(&b.version)
                    .then_with(|| a.version.to_string().cmp(&b.version.to_string()))
            });
        let Some(candidate) = selected else {
            return Err(DataError::UnresolvedPackage {
                package: package.clone(),
                requirements: requirements
                    .iter()
                    .map(|r| r.version.to_string())
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect(),
            });
        };
        resolved.push(ResolvedPackage {
            kind: candidate.kind,
            package: package.clone(),
            version: candidate.version.clone(),
            checksum: candidate.checksum,
        });
    }
    Ok(resolved)
}

pub(crate) fn validate_campaign_lock(lock: &CampaignLock) -> Result<(), DataError> {
    if lock
        .packages
        .windows(2)
        .any(|pair| pair[0].package >= pair[1].package)
    {
        return Err(invalid_lock(
            "packages must be unique and strictly ascending by package id",
        ));
    }
    Ok(())
}

pub(crate) fn validate_assets_lock(lock: &AssetsLock) -> Result<(), DataError> {
    let mut seen = BTreeSet::new();
    for path in lock.assets.keys() {
        let folded = path.as_str().to_ascii_lowercase();
        if !path.as_str().starts_with("assets/") || folded == "assets/assets.lock" {
            return Err(invalid_lock(format!("invalid asset root: {path}")));
        }
        if !seen.insert(folded) {
            return Err(invalid_lock(format!("asset path case collision: {path}")));
        }
    }
    Ok(())
}

pub(crate) fn validate_coverage(
    requirements: &[PackageRequirement],
    lock: &CampaignLock,
) -> Result<(), DataError> {
    let grouped = grouped_requirements(requirements);
    let locked: BTreeMap<_, _> = lock.packages.iter().map(|p| (&p.package, p)).collect();
    let ids: BTreeSet<_> = grouped.keys().chain(locked.keys()).copied().collect();
    for id in ids {
        let Some(requirements) = grouped.get(id) else {
            return Err(invalid_lock(format!("unexpected locked package: {id}")));
        };
        let Some(package) = locked.get(id) else {
            return Err(invalid_lock(format!("missing locked package: {id}")));
        };
        if requirements
            .iter()
            .any(|r| r.kind != package.kind || !r.version.matches(&package.version))
        {
            return Err(invalid_lock(format!(
                "locked kind/version does not satisfy requirements: {id}"
            )));
        }
    }
    Ok(())
}

/// Hashes the complete canonical assets-lock envelope, including its final LF.
pub fn assets_lock_digest(lock: &AssetsLock) -> Result<Digest, DataError> {
    Ok(Digest::from_bytes(
        *blake3::hash(&write_assets_lock(lock)?).as_bytes(),
    ))
}

/// Validates assets first, resolves packages, and computes the assets-lock digest.
pub fn make_campaign_lock(
    requirements: &[PackageRequirement],
    candidates: &[PackageCandidate],
    assets: &AssetsLock,
) -> Result<CampaignLock, DataError> {
    validate_assets_lock(assets)?;
    let packages = resolve_packages(requirements, candidates)?;
    Ok(CampaignLock {
        packages,
        assets_lock: assets_lock_digest(assets)?,
        note: None,
    })
}

/// Reads a complete campaign-lock envelope, requiring the correct document kind.
pub fn read_campaign_lock(bytes: &[u8]) -> Result<CampaignLock, DataError> {
    match crate::read_document(bytes)? {
        Document::CampaignLock(lock) => Ok(lock),
        _ => Err(DataError::Layout {
            path: None,
            message: "expected campaign lock".into(),
        }),
    }
}

/// Writes a locally valid campaign lock without silently sorting package arrays.
pub fn write_campaign_lock(lock: &CampaignLock) -> Result<Vec<u8>, DataError> {
    crate::write_document(&Document::CampaignLock(lock.clone()))
}

/// Reads a complete assets-lock envelope, requiring the correct document kind.
pub fn read_assets_lock(bytes: &[u8]) -> Result<AssetsLock, DataError> {
    match crate::read_document(bytes)? {
        Document::AssetsLock(lock) => Ok(lock),
        _ => Err(DataError::Layout {
            path: None,
            message: "expected assets lock".into(),
        }),
    }
}

/// Writes an assets lock after validating source roots and ASCII-case uniqueness.
pub fn write_assets_lock(lock: &AssetsLock) -> Result<Vec<u8>, DataError> {
    crate::write_document(&Document::AssetsLock(lock.clone()))
}

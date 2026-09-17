//! Read-only gate shared by integration tests and private registry tests.
use super::gate_api as data;
use data::{SchemaVersion, SourcePath};
use serde::Deserialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

pub(crate) type Snapshot = BTreeMap<String, Vec<u8>>;
pub(crate) type EdgeKey = (String, u32, u32);
type Check<T> = Result<T, String>;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Entry {
    pub schema_type: String,
    pub from: u32,
    pub to: u32,
    pub root: String,
    pub document: String,
    pub golden: String,
}

impl Entry {
    pub fn key(&self) -> EdgeKey {
        (self.schema_type.clone(), self.from, self.to)
    }
}

pub(crate) struct Oracle {
    pub edge: EdgeKey,
    pub before: Value,
    pub after: Value,
}

pub(crate) fn snapshot() -> Check<Snapshot> {
    fn walk(dir: &Path, prefix: &str, files: &mut Snapshot) -> Check<()> {
        for entry in fs::read_dir(dir).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| "non-Unicode fixture")?;
            let logical = format!("{prefix}{name}");
            let kind = entry.file_type().map_err(|e| e.to_string())?;
            if kind.is_dir() {
                walk(&entry.path(), &format!("{logical}/"), files)?;
            } else if kind.is_file() {
                files.insert(logical, fs::read(entry.path()).map_err(|e| e.to_string())?);
            } else {
                return Err(format!("nonregular fixture: {logical}"));
            }
        }
        Ok(())
    }
    let mut files = Snapshot::new();
    walk(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures"),
        "",
        &mut files,
    )?;
    Ok(files)
}

fn bytes<'a>(files: &'a Snapshot, name: &str) -> Check<&'a [u8]> {
    files
        .get(name)
        .map(Vec::as_slice)
        .ok_or_else(|| format!("missing fixture: {name}"))
}

fn canonical(bytes: &[u8]) -> Check<Value> {
    let value: Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    // Comparing the original bytes also rejects duplicate keys lost by Value.
    if data::canonical_json(&value).map_err(|e| e.to_string())? != bytes {
        return Err("noncanonical fixture JSON".into());
    }
    Ok(value)
}

fn tag(value: &Value) -> Check<(String, u32)> {
    let tag = value.as_str().ok_or("missing schema tag")?;
    let (name, text) = tag.split_once('/').ok_or("schema tag without version")?;
    let version: u32 = text.parse().map_err(|_| "invalid schema version")?;
    if name.is_empty()
        || name.chars().any(char::is_whitespace)
        || version == 0
        || text != version.to_string()
    {
        return Err(format!("noncanonical schema tag: {tag}"));
    }
    Ok((name.into(), version))
}

fn version_map(versions: &[SchemaVersion]) -> Check<BTreeMap<String, u32>> {
    let mut out = BTreeMap::new();
    for version in versions {
        let (name, current) = tag(&Value::String(format!(
            "{}/{}",
            version.schema_type, version.current
        )))?;
        if out.insert(name, current).is_some() {
            return Err("duplicate registry family".into());
        }
    }
    if versions.is_empty()
        || !versions
            .windows(2)
            .all(|pair| pair[0].schema_type < pair[1].schema_type)
    {
        return Err("registry must be nonempty and sorted".into());
    }
    Ok(out)
}

pub(crate) fn check_tags(
    versions: &[SchemaVersion],
    schemas: &Snapshot,
    serialized: &[Vec<u8>],
) -> Check<()> {
    let expected = version_map(versions)?;
    let mut schema_tags = BTreeMap::new();
    for bytes in schemas.values() {
        let root = canonical(bytes)?;
        let shape = if let Some(variants) = root.get("oneOf") {
            let variants = variants.as_array().ok_or("invalid schema union")?;
            if variants.len() != 1 {
                return Err("document schema must have one variant".into());
            }
            &variants[0]
        } else {
            let properties = root["properties"]
                .as_object()
                .ok_or("missing schema properties")?;
            if properties.contains_key("schema") {
                return Err("unrecognized document envelope".into());
            }
            continue;
        };
        let (name, version) = tag(&shape["properties"]["schema"]["const"])?;
        if schema_tags.insert(name, version).is_some() {
            return Err("duplicate schema family".into());
        }
    }
    let mut serde_tags = BTreeMap::new();
    for bytes in serialized {
        let (name, version) = tag(&canonical(bytes)?["schema"])?;
        if serde_tags.insert(name, version).is_some() {
            return Err("duplicate representative family".into());
        }
    }
    if schema_tags != expected || serde_tags != expected {
        return Err("registry/schema/serde versions disagree".into());
    }
    Ok(())
}

pub(crate) fn check_edges(versions: &[SchemaVersion], edges: &[EdgeKey]) -> Check<()> {
    let versions = version_map(versions)?;
    let mut seen = BTreeSet::new();
    for (name, from, to) in edges {
        let current = versions.get(name).ok_or("edge for unknown family")?;
        if *from == 0
            || from.checked_add(1) != Some(*to)
            || to > current
            || !seen.insert((name.clone(), *from))
        {
            return Err("duplicate, jump, backward, zero, overflowing or extra edge".into());
        }
    }
    for (name, current) in versions {
        // Count valid unique adjacent edges instead of allocating 1..current.
        let count = seen.iter().filter(|(family, _)| *family == name).count();
        if u32::try_from(count).map_err(|_| "too many edges")? != current - 1 {
            return Err(format!("incomplete chain: {name}"));
        }
    }
    if !edges.iter().any(|edge| edge == &("crpg.item".into(), 1, 2)) {
        return Err("gate must contain the real Item edge".into());
    }
    Ok(())
}

pub(crate) fn check_manifest(versions: &[SchemaVersion], files: &Snapshot) -> Check<Vec<Entry>> {
    let entries: Vec<Entry> = serde_json::from_value(canonical(bytes(files, "migrations.json")?)?)
        .map_err(|e| e.to_string())?;
    if !entries
        .windows(2)
        .all(|pair| (&pair[0].schema_type, pair[0].from) < (&pair[1].schema_type, pair[1].from))
    {
        return Err("manifest rows must be strictly sorted".into());
    }
    for entry in &entries {
        for name in [&entry.root, &entry.document, &entry.golden] {
            name.parse::<SourcePath>().map_err(|e| e.to_string())?;
        }
    }
    check_edges(
        versions,
        &entries.iter().map(Entry::key).collect::<Vec<_>>(),
    )?;
    Ok(entries)
}

pub(crate) fn check_fixtures(versions: &[SchemaVersion], files: &Snapshot) -> Check<Vec<Oracle>> {
    let entries = check_manifest(versions, files)?;
    let oracles = check_oracle_inventory(versions, files)?;
    let gate = canonical(bytes(files, "expected.json")?)?;
    let gate = gate.as_array().ok_or("gate-8 manifest must be an array")?;
    for entry in entries {
        let roots: Vec<_> = gate
            .iter()
            .filter(|row| row["root"] == entry.root)
            .collect();
        if roots.len() != 1
            || roots[0]["expect"] != "clean"
            || roots[0].get("snapshot") != Some(&Value::Null)
        {
            return Err(format!(
                "fixture is not a unique clean gate-8 root: {}",
                entry.root
            ));
        }
        let prefix = format!("{}/", entry.root);
        let mut campaign = BTreeMap::new();
        for (name, bytes) in files {
            if let Some(relative) = name.strip_prefix(&prefix) {
                let path: SourcePath = relative
                    .parse()
                    .map_err(|e: data::DataError| e.to_string())?;
                canonical(bytes)?;
                campaign.insert(path, bytes.clone());
            }
        }
        if campaign.is_empty() {
            return Err("empty or missing campaign fixture".into());
        }
        let golden: BTreeMap<String, String> =
            serde_json::from_value(canonical(bytes(files, &entry.golden)?)?)
                .map_err(|e| e.to_string())?;
        let mut expected = BTreeMap::new();
        for (name, text) in &golden {
            canonical(text.as_bytes())?;
            expected.insert(
                name.parse::<SourcePath>().map_err(|e| e.to_string())?,
                text.as_bytes().to_vec(),
            );
        }
        let loaded = data::load_campaign(
            &campaign,
            &"0.1.0".parse().map_err(|_| "invalid test engine")?,
        )
        .map_err(|e| e.to_string())?;
        let actual = data::serialize_campaign(&loaded).map_err(|e| e.to_string())?;
        if actual != expected {
            return Err("complete campaign output differs from golden".into());
        }
    }
    Ok(oracles)
}

pub(crate) fn check_oracle_inventory(
    versions: &[SchemaVersion],
    files: &Snapshot,
) -> Check<Vec<Oracle>> {
    let entries = check_manifest(versions, files)?;
    let versions = version_map(versions)?;
    let mut expected_steps = BTreeSet::new();
    let mut oracles = Vec::new();
    for entry in entries {
        let before = canonical(bytes(files, &format!("{}/{}", entry.root, entry.document))?)?;
        if tag(&before["schema"])? != (entry.schema_type.clone(), entry.from) {
            return Err("source tag disagrees with manifest".into());
        }
        let after = if entry.to < versions[&entry.schema_type] {
            let name = format!(
                "migration_steps/{}/{}-to-{}.json",
                entry.schema_type, entry.from, entry.to
            );
            expected_steps.insert(name.clone());
            canonical(bytes(files, &name)?)?
        } else {
            let golden: BTreeMap<String, String> =
                serde_json::from_value(canonical(bytes(files, &entry.golden)?)?)
                    .map_err(|e| e.to_string())?;
            canonical(
                golden
                    .get(&entry.document)
                    .ok_or("golden missing named document")?
                    .as_bytes(),
            )?
        };
        if tag(&after["schema"])? != (entry.schema_type.clone(), entry.to) {
            return Err("immediate oracle tag disagrees with edge".into());
        }
        oracles.push(Oracle {
            edge: entry.key(),
            before,
            after,
        });
    }
    let actual_steps: BTreeSet<_> = files
        .keys()
        .filter(|name| name.starts_with("migration_steps/"))
        .cloned()
        .collect();
    if expected_steps != actual_steps {
        return Err("immediate-edge oracle inventory differs".into());
    }
    Ok(oracles)
}

pub(crate) fn check_steps(
    oracles: &[Oracle],
    mut step: impl FnMut(&EdgeKey, &mut Value) -> Check<()>,
) -> Check<()> {
    for oracle in oracles {
        let mut actual = oracle.before.clone();
        step(&oracle.edge, &mut actual)?;
        if actual != oracle.after {
            return Err(format!("single-edge output differs: {:?}", oracle.edge));
        }
    }
    Ok(())
}

pub(crate) fn representatives(files: &Snapshot) -> Check<Vec<Vec<u8>>> {
    use data::*;
    let mut docs: Vec<Document> = files
        .iter()
        .filter(|(name, _)| name.starts_with("one_area_one_creature/"))
        .map(|(_, bytes)| read_document(bytes).map_err(|e| e.to_string()))
        .collect::<Check<_>>()?;
    let id = crpg_core::Ulid::from_u128;
    docs.extend([
        Document::Item(Item {
            id: id(10),
            slug: "i".into(),
            name: "i".into(),
            note: None,
            stats: BTreeMap::new(),
            tags: Vec::new(),
        }),
        Document::Faction(Faction {
            id: id(11),
            slug: "f".into(),
            name: "f".into(),
            note: None,
            relations: Vec::new(),
        }),
        Document::Dialogue(Dialogue {
            id: id(12),
            slug: "d".into(),
            name: "d".into(),
            note: None,
            entry: id(13),
            nodes: vec![DialogueNode {
                id: id(13),
                body: DialogueBody::End,
            }],
        }),
        Document::Quest(Quest {
            id: id(14),
            slug: "q".into(),
            name: "q".into(),
            note: None,
            entry: id(15),
            states: vec![QuestState {
                id: id(15),
                name: "s".into(),
                terminal: true,
                on_enter: Vec::new(),
                transitions: Vec::new(),
            }],
        }),
        Document::Graph(EventGraph {
            id: id(16),
            slug: "g".into(),
            name: "g".into(),
            note: None,
            entry: Trigger::Timer { ticks: 0 },
            start: id(17),
            nodes: vec![Node {
                id: id(17),
                body: NodeBody::Wait { ticks: 0 },
            }],
            edges: Vec::new(),
            locals: Vec::new(),
        }),
    ]);
    docs.iter()
        .map(|doc| {
            let bytes = write_document(doc).map_err(|e| e.to_string())?;
            if read_document(&bytes).map_err(|e| e.to_string())? != *doc {
                return Err("representative round trip differs".into());
            }
            Ok(bytes)
        })
        .collect()
}

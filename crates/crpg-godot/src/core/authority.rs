//! The authority half: one embedded `Host`, the campaign walk, the NPC driver
//! and the delivery hand-off (ADR-0033 §5-§8, §19).
//!
//! Nothing here is visible to GDScript. The authority owns the only mutable
//! game state, receives player intents through the link, drives the NPC by the
//! same host API a player would use, and acknowledges delivery only for the
//! frames the link accepted.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::Path;

use crpg_core::{EntityId, Ulid};
use crpg_data::{Document, LoadedCampaign, SourcePath};
use crpg_net::codec_v2::encode_intent;
use crpg_net::protocol::{IntentBody, IntentFrame, NetId, LANE_COMBAT, SEQ_FIRST};
use crpg_net::transport::Transport;
use crpg_server::host::{
    AdmissionResult, DeliveryEntry, DeliveryKind, DisclosureGrants, EntityDisclosure, Host,
    IngestDisposition, LegalIntent, PeerHandle,
};

use super::client::{Offered, PresentationRow, PresentationTable, StepReport};
use super::errors::{host_fault, BridgeCode, Fault};
use super::link::LinkEnd;

/// How long the NPC waits after an option appears before acting.
pub(crate) const NPC_ACTION_DELAY_MS: u64 = 600;

/// The NPC's choice: the first attack on a different side, else the first
/// option, else nothing (B10).
pub(crate) fn choose_npc(
    options: &[LegalIntent],
    sides: &BTreeMap<NetId, String>,
) -> Option<usize> {
    for (index, option) in options.iter().enumerate() {
        if let IntentBody::DeclareAction { target, .. } = option.body {
            match (sides.get(&option.actor), sides.get(&target)) {
                (Some(actor), Some(target)) if actor != target => return Some(index),
                _ => {}
            }
        }
    }
    if options.is_empty() {
        None
    } else {
        Some(0)
    }
}

/// Sends the player's delivery entries in order, stopping at the first full
/// link, and returns the last accepted ordinal (B4).
pub(crate) fn hand_off(
    entries: &[DeliveryEntry],
    link: &mut impl Transport,
) -> Result<Option<u64>, Fault> {
    let mut accepted = None;
    for entry in entries {
        match entry.kind {
            DeliveryKind::Transfer => {
                return Err(Fault {
                    code: BridgeCode::UnexpectedTransfer,
                    detail: format!("transfer entry {}", entry.ordinal),
                })
            }
            DeliveryKind::Delta => {
                if link.send(&entry.bytes).is_err() {
                    break;
                }
                accepted = Some(entry.ordinal);
            }
        }
    }
    Ok(accepted)
}

/// Folds the pump's admission results into the NPC's in-flight sequence (B9).
///
/// A retained head-of-line result is a hold, not a rejection. Any other NPC
/// rejection is fatal; an applied NPC intent that was in flight clears it.
pub(crate) fn npc_results(
    results: &[AdmissionResult],
    npc: PeerHandle,
    inflight: Option<u64>,
) -> Result<Option<u64>, Fault> {
    let mut current = inflight;
    for result in results {
        if result.peer != npc || result.retained_in_ingress {
            continue;
        }
        match result.status {
            crpg_net::protocol::ReceiptStatus::Applied => {
                if Some(result.seq.unwrap_or_default()) == current {
                    current = None;
                }
            }
            crpg_net::protocol::ReceiptStatus::Rejected(_) => {
                return Err(Fault {
                    code: BridgeCode::HostFailed,
                    detail: "NPC intent rejected".to_string(),
                });
            }
        }
    }
    Ok(current)
}

/// The NPC's arming and firing state (B8).
#[derive(Default)]
pub(crate) struct NpcPacer {
    due: Option<u64>,
}

impl NpcPacer {
    /// Advances the pacer; `true` means fire exactly one intent now.
    pub(crate) fn step(&mut self, now_ms: u64, has_options: bool, inflight: bool) -> bool {
        if inflight || !has_options {
            self.due = None;
            false
        } else if self.due.is_none() {
            self.due = Some(now_ms.saturating_add(NPC_ACTION_DELAY_MS));
            false
        } else if Some(now_ms) >= self.due {
            self.due = None;
            true
        } else {
            false
        }
    }
}

/// Checks the start order's `(EntityId, placement)` sequence against the
/// encounter's authored participants; mismatch is `START_FAILED` (B1(8)).
pub(crate) fn check_binding(
    bound: &[(EntityId, Ulid)],
    participants: &[Ulid],
) -> Result<(), Fault> {
    if bound.len() != participants.len() {
        return Err(Fault {
            code: BridgeCode::StartFailed,
            detail: format!(
                "placement binding mismatch ({} combatants, {} participants)",
                bound.len(),
                participants.len()
            ),
        });
    }
    for (index, (_, placement)) in bound.iter().enumerate() {
        if *placement != participants[index] {
            return Err(Fault {
                code: BridgeCode::StartFailed,
                detail: format!(
                    "placement binding mismatch at index {index} (expected {}, found {placement})",
                    participants[index]
                ),
            });
        }
    }
    Ok(())
}

/// Observed entry shape. The walker never follows links.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EntryKind {
    /// Ordinary directory.
    Directory,
    /// Ordinary file.
    File,
    /// Symbolic link, never followed.
    Symlink,
    /// Anything else.
    Other,
}

/// Converts link metadata without following it.
fn entry_kind(file_type: &fs::FileType) -> EntryKind {
    if file_type.is_symlink() {
        EntryKind::Symlink
    } else if file_type.is_dir() {
        EntryKind::Directory
    } else if file_type.is_file() {
        EntryKind::File
    } else {
        EntryKind::Other
    }
}

/// Filesystem operations the collector needs; tests inject fakes.
pub(crate) trait WalkFs {
    /// The entry kind at `path`, without following links.
    fn metadata(&self, path: &Path) -> io::Result<EntryKind>;
    /// The entry names of a directory, in any order.
    fn read_dir_names(&self, path: &Path) -> io::Result<Vec<OsString>>;
    /// The bytes of a regular file.
    fn read_file(&self, path: &Path) -> io::Result<Vec<u8>>;
}

/// Production filesystem access.
pub(crate) struct RealFs;

impl WalkFs for RealFs {
    fn metadata(&self, path: &Path) -> io::Result<EntryKind> {
        path.symlink_metadata()
            .map(|meta| entry_kind(&meta.file_type()))
    }

    fn read_dir_names(&self, path: &Path) -> io::Result<Vec<OsString>> {
        let mut names = Vec::new();
        for entry in fs::read_dir(path)? {
            names.push(entry?.file_name());
        }
        Ok(names)
    }

    fn read_file(&self, path: &Path) -> io::Result<Vec<u8>> {
        fs::read(path)
    }
}

/// Builds a filesystem fault; the detail is `"<op> <logical>"`.
fn io_fault(code: BridgeCode, op: &str, logical: &str) -> Fault {
    Fault {
        code,
        detail: format!("{op} {logical}"),
    }
}

/// Rejects a supplied campaign root whose lexical ancestors traverse a link or
/// a non-directory.
fn check_walk_root_ancestors(fs: &impl WalkFs, root: &Path) -> Result<(), Fault> {
    let mut ancestors: Vec<&Path> = root.ancestors().skip(1).collect();
    ancestors.reverse();
    for ancestor in ancestors {
        if ancestor.as_os_str().is_empty() {
            continue;
        }
        match fs.metadata(ancestor) {
            Err(_) => {
                return Err(io_fault(
                    BridgeCode::CampaignIo,
                    "open root",
                    "<campaign-root>",
                ))
            }
            Ok(EntryKind::Directory) => {}
            Ok(EntryKind::Symlink) => {
                return Err(io_fault(
                    BridgeCode::CampaignIo,
                    "open root",
                    "<campaign-root>",
                ))
            }
            Ok(EntryKind::File) | Ok(EntryKind::Other) => {
                return Err(io_fault(
                    BridgeCode::CampaignIo,
                    "open root",
                    "<campaign-root>",
                ))
            }
        }
    }
    Ok(())
}

/// Walks `root` and collects every classified campaign document.
///
/// Filesystem failures are `CAMPAIGN_IO` (20); classifier rejections are
/// `CAMPAIGN_INVALID` (21). The walk is a sorted depth-first pre-order, and
/// the first failure in that order wins.
pub(crate) fn collect_campaign(
    fs: &impl WalkFs,
    root: &Path,
) -> Result<BTreeMap<SourcePath, Vec<u8>>, Fault> {
    if root.to_str().is_none() {
        return Err(io_fault(
            BridgeCode::CampaignIo,
            "open root",
            "<campaign-root>",
        ));
    }
    check_walk_root_ancestors(fs, root)?;
    match fs.metadata(root) {
        Err(_) => {
            return Err(io_fault(
                BridgeCode::CampaignIo,
                "open root",
                "<campaign-root>",
            ))
        }
        Ok(EntryKind::Directory) => {}
        Ok(EntryKind::Symlink) | Ok(EntryKind::File) | Ok(EntryKind::Other) => {
            return Err(io_fault(
                BridgeCode::CampaignIo,
                "open root",
                "<campaign-root>",
            ))
        }
    }
    let mut files = BTreeMap::new();
    collect_into(fs, root, "", &mut files)?;
    Ok(files)
}

/// Lists one directory, sorts names by encoded bytes, and recurses.
fn collect_into(
    fs: &impl WalkFs,
    disk_dir: &Path,
    logical_dir: &str,
    files: &mut BTreeMap<SourcePath, Vec<u8>>,
) -> Result<(), Fault> {
    let mut names = fs.read_dir_names(disk_dir).map_err(|_| {
        let logical = if logical_dir.is_empty() {
            "<campaign-root>"
        } else {
            logical_dir
        };
        io_fault(BridgeCode::CampaignIo, "list", logical)
    })?;
    names.sort_by(|a, b| a.as_encoded_bytes().cmp(b.as_encoded_bytes()));
    for name in &names {
        let Some(name_text) = name.to_str() else {
            return Err(io_fault(BridgeCode::CampaignIo, "list", "<campaign-root>"));
        };
        let logical = if logical_dir.is_empty() {
            name_text.to_owned()
        } else {
            format!("{logical_dir}/{name_text}")
        };
        let disk_child = disk_dir.join(name);
        let kind = fs
            .metadata(&disk_child)
            .map_err(|_| io_fault(BridgeCode::CampaignIo, "classify", &logical))?;
        if kind == EntryKind::Directory {
            collect_into(fs, &disk_child, &logical, files)?;
        } else {
            collect_file_entry(fs, &disk_child, &logical, kind, files)?;
        }
    }
    Ok(())
}

/// Classifies one non-directory entry and reads it when it is a document.
fn collect_file_entry(
    fs: &impl WalkFs,
    disk_path: &Path,
    logical: &str,
    kind: EntryKind,
    files: &mut BTreeMap<SourcePath, Vec<u8>>,
) -> Result<(), Fault> {
    let path = match crpg_data::campaign_document_path(logical) {
        Err(_) => return Err(io_fault(BridgeCode::CampaignInvalid, "classify", logical)),
        Ok(None) => return Ok(()),
        Ok(Some(path)) => path,
    };
    match kind {
        EntryKind::File => {
            let bytes = fs
                .read_file(disk_path)
                .map_err(|_| io_fault(BridgeCode::CampaignIo, "read", logical))?;
            files.insert(path, bytes);
            Ok(())
        }
        EntryKind::Symlink => Err(io_fault(BridgeCode::CampaignIo, "classify", logical)),
        EntryKind::Directory | EntryKind::Other => {
            Err(io_fault(BridgeCode::CampaignIo, "classify", logical))
        }
    }
}

/// Campaign documents resolved for presentation: names, sides and abilities
/// (D8, a private copy of the CLI's `Names`).
pub(crate) struct Presentation {
    strings: BTreeMap<String, String>,
    placements: BTreeMap<Ulid, (String, Ulid)>,
    creatures: BTreeMap<Ulid, (String, Option<Ulid>)>,
    factions: BTreeMap<Ulid, String>,
    abilities: BTreeMap<Ulid, String>,
}

impl Presentation {
    /// Collects naming documents in deterministic source-path order.
    pub(crate) fn from_loaded(loaded: &LoadedCampaign) -> Self {
        let mut strings = BTreeMap::new();
        let mut placements = BTreeMap::new();
        let mut creatures = BTreeMap::new();
        let mut factions = BTreeMap::new();
        let mut abilities = BTreeMap::new();
        for document in loaded.documents.values() {
            match document {
                Document::Locale(locale) => {
                    for (key, text) in &locale.strings {
                        strings.insert(key.clone(), text.clone());
                    }
                }
                Document::Ability(ability) => {
                    abilities.insert(ability.id, ability.name.clone());
                }
                Document::Placements(list) => {
                    for placement in &list.placements {
                        placements.insert(placement.id, (placement.name.clone(), placement.prefab));
                    }
                }
                Document::Creature(creature) => {
                    creatures.insert(creature.id, (creature.name.clone(), creature.faction));
                }
                Document::Faction(faction) => {
                    factions.insert(faction.id, faction.name.clone());
                }
                _ => {}
            }
        }
        Self {
            strings,
            placements,
            creatures,
            factions,
            abilities,
        }
    }

    /// Resolves one locale key, else prints the key itself.
    pub(crate) fn text(&self, key: &str) -> String {
        self.strings
            .get(key)
            .cloned()
            .unwrap_or_else(|| key.to_owned())
    }

    /// One combatant's display name and side.
    pub(crate) fn combatant(&self, placement: Ulid) -> (String, String) {
        let placed = self.placements.get(&placement);
        let creature = placed.and_then(|(_, prefab)| self.creatures.get(prefab));
        let name = match creature {
            Some((key, _)) => self.text(key),
            None => match placed {
                Some((key, _)) => self.text(key),
                None => placement.to_string(),
            },
        };
        let side = match creature {
            Some((key, faction)) => match faction.and_then(|id| self.factions.get(&id)) {
                Some(faction_key) => self.text(faction_key),
                None => self.text(key),
            },
            None => name.clone(),
        };
        (name, side)
    }

    /// The player's ability display table.
    pub(crate) fn ability_table(&self) -> BTreeMap<Ulid, String> {
        self.abilities
            .iter()
            .map(|(id, key)| (*id, self.text(key)))
            .collect()
    }
}

/// The authority's live, trusted state and its per-poll step.
pub(crate) struct Authority {
    host: Host,
    player: PeerHandle,
    npc: PeerHandle,
    end: LinkEnd,
    npc_sides: BTreeMap<NetId, String>,
    next_seq: u64,
    inflight: Option<u64>,
    npc_submitted: u64,
    #[cfg_attr(not(test), allow(dead_code))]
    bound: Vec<(EntityId, Ulid)>,
    pacer: NpcPacer,
}

impl Authority {
    /// Builds the authority half from a started history world and both peer
    /// bindings (B1(10)-(12)).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        host: Host,
        player: PeerHandle,
        npc: PeerHandle,
        end: LinkEnd,
        npc_sides: BTreeMap<NetId, String>,
        bound: Vec<(EntityId, Ulid)>,
    ) -> Self {
        Self {
            host,
            player,
            npc,
            end,
            npc_sides,
            next_seq: SEQ_FIRST,
            inflight: None,
            npc_submitted: 0,
            bound,
            pacer: NpcPacer::default(),
        }
    }

    /// Shuts the host down; used only by the bridge's fence and close paths.
    pub(crate) fn shutdown(&mut self) {
        self.host.shutdown();
    }

    /// One authority step in the contract's exact order (B3).
    pub(crate) fn step(&mut self, now_ms: u64) -> Result<StepReport, Fault> {
        let mut refused = None;

        // 1. Drain the player's upstream queue.
        while let Some(bytes) = self.end.recv() {
            match self.host.ingest(self.player, &bytes, now_ms) {
                Ok(IngestDisposition::Staged) => {}
                Ok(IngestDisposition::Refused { status }) => {
                    refused = Some(format!("player intent refused: {status:?}"));
                }
                Err(error) => return Err(host_fault(&error)),
            }
        }

        // 2. Drive the NPC; at most one ingest per poll.
        let options = self
            .host
            .legal_intents(self.npc)
            .map_err(|error| host_fault(&error))?;
        let fire = self
            .pacer
            .step(now_ms, !options.is_empty(), self.inflight.is_some());
        if fire {
            if let Some(index) = choose_npc(&options, &self.npc_sides) {
                let chosen = options[index];
                let frame = IntentFrame {
                    epoch: self
                        .host
                        .epoch(self.npc)
                        .map_err(|error| host_fault(&error))?,
                    lane: LANE_COMBAT,
                    seq: self.next_seq,
                    observed_tick: self.host.server_tick(),
                    actor: chosen.actor,
                    body: chosen.body,
                };
                let bytes = encode_intent(&frame).map_err(|error| Fault {
                    code: BridgeCode::HostFailed,
                    detail: error.to_string(),
                })?;
                match self.host.ingest(self.npc, &bytes, now_ms) {
                    Ok(IngestDisposition::Staged) => {
                        self.inflight = Some(self.next_seq);
                        self.next_seq = self.next_seq.saturating_add(1);
                        self.npc_submitted = self.npc_submitted.saturating_add(1);
                    }
                    Ok(IngestDisposition::Refused { .. }) => {
                        return Err(Fault {
                            code: BridgeCode::HostFailed,
                            detail: "NPC intent refused".to_string(),
                        })
                    }
                    Err(error) => return Err(host_fault(&error)),
                }
            }
        }

        // 3. Pump; fold the NPC's result and attribute a dropped reply to the
        //    player (B9 has already fenced any NPC rejection).
        let summary = self.host.pump(now_ms).map_err(|error| host_fault(&error))?;
        self.inflight = npc_results(&summary.results, self.npc, self.inflight)?;
        if summary.dropped_replies > 0 {
            refused.get_or_insert_with(|| "dropped reply".to_string());
        }

        // 4. Hand off the player's delivery, then acknowledge the NPC's
        //    delivery unread.
        let player_entries = self
            .host
            .take_delivery_entries(self.player)
            .map_err(|error| host_fault(&error))?;
        if let Some(ordinal) = hand_off(&player_entries, &mut self.end)? {
            self.host
                .acknowledge_delivery(self.player, ordinal)
                .map_err(|error| host_fault(&error))?;
        }
        let npc_entries = self
            .host
            .take_delivery_entries(self.npc)
            .map_err(|error| host_fault(&error))?;
        if let Some(last) = npc_entries.last() {
            self.host
                .acknowledge_delivery(self.npc, last.ordinal)
                .map_err(|error| host_fault(&error))?;
        }

        // 5. Watermark captures every poll.
        let capture = self.host.last_capture();
        self.host
            .acknowledge_captures(capture)
            .map_err(|error| host_fault(&error))?;

        // 6. The next player options, in host order.
        let options = self
            .host
            .legal_intents(self.player)
            .map_err(|error| host_fault(&error))?;
        Ok(StepReport {
            options: options
                .into_iter()
                .map(|option| Offered {
                    actor: option.actor,
                    body: option.body,
                })
                .collect(),
            refused,
        })
    }
}

#[cfg(test)]
impl Authority {
    /// The owned host, for K15/K20/K21's trusted assertions.
    pub(crate) fn host(&self) -> &Host {
        &self.host
    }

    /// The player's peer handle.
    pub(crate) fn player(&self) -> PeerHandle {
        self.player
    }

    /// The NPC's peer handle.
    pub(crate) fn npc(&self) -> PeerHandle {
        self.npc
    }

    /// The recorded `(EntityId, placement)` binding order.
    pub(crate) fn bound(&self) -> &[(EntityId, Ulid)] {
        &self.bound
    }

    /// NPC intents ingested so far.
    pub(crate) fn npc_submitted(&self) -> u64 {
        self.npc_submitted
    }
}

/// Builds the shared disclosure grants (D7).
pub(crate) fn grants(
    bound: &[(EntityId, Ulid)],
    abilities: &BTreeSet<Ulid>,
    encounter: Ulid,
) -> DisclosureGrants {
    DisclosureGrants {
        entities: bound
            .iter()
            .map(|(entity, _)| EntityDisclosure {
                entity: *entity,
                present: true,
                health: true,
                spawn: true,
                despawn: true,
                died: true,
                action_actor: true,
                action_target: true,
                action_outcome: true,
                action_damage: true,
                turn_start: true,
            })
            .collect(),
        abilities: abilities.iter().copied().collect(),
        encounters: vec![encounter],
        turn_round: true,
        encounter_end: true,
        turn_state: true,
    }
}

/// Looks up one ruleset ability set from a loaded campaign.
pub(crate) fn ruleset_abilities(loaded: &LoadedCampaign, ruleset: Ulid) -> BTreeSet<Ulid> {
    let mut set = BTreeSet::new();
    for document in loaded.documents.values() {
        if let Document::Ruleset(value) = document {
            if value.id == ruleset {
                set.extend(value.abilities.iter().copied());
            }
        }
    }
    set
}

/// The first encounter in document order (B1(5)).
pub(crate) fn first_encounter(loaded: &LoadedCampaign) -> Option<Ulid> {
    loaded
        .documents
        .values()
        .find_map(|document| match document {
            Document::Encounter(encounter) => Some(encounter.id),
            _ => None,
        })
}

/// Builds the presentation tables for both peers (B1(12)).
pub(crate) fn presentation_tables(
    host: &Host,
    player: PeerHandle,
    npc: PeerHandle,
    bound: &[(EntityId, Ulid)],
    presentation: &Presentation,
) -> Result<(PresentationTable, BTreeMap<NetId, String>), Fault> {
    let mut rows = BTreeMap::new();
    let mut npc_sides = BTreeMap::new();
    for (entity, placement) in bound {
        let player_id = host
            .replica_id(player, *entity)
            .map_err(|error| host_fault(&error))?
            .ok_or_else(|| Fault {
                code: BridgeCode::StartFailed,
                detail: format!("no player replica id for {placement}"),
            })?;
        let npc_id = host
            .replica_id(npc, *entity)
            .map_err(|error| host_fault(&error))?
            .ok_or_else(|| Fault {
                code: BridgeCode::StartFailed,
                detail: format!("no NPC replica id for {placement}"),
            })?;
        let (name, side) = presentation.combatant(*placement);
        rows.insert(
            player_id,
            PresentationRow {
                placement: *placement,
                name,
                side: side.clone(),
            },
        );
        npc_sides.insert(npc_id, side);
    }
    let table = PresentationTable {
        rows,
        abilities: presentation.ability_table(),
    };
    Ok((table, npc_sides))
}

/// Builds the start spec's document lookups from the loaded campaign (B1(6)).
pub(crate) struct SpecParts<'a> {
    /// The matching ruleset document.
    pub(crate) ruleset: &'a crpg_data::Ruleset,
    /// Ability documents by identity.
    pub(crate) abilities: BTreeMap<Ulid, &'a crpg_data::Ability>,
    /// Outcome-table documents by identity.
    pub(crate) outcome_tables: BTreeMap<Ulid, &'a crpg_data::OutcomeTable>,
    /// Placement documents with their owning areas.
    pub(crate) placements: BTreeMap<Ulid, crpg_sim::PlacementAndArea<'a>>,
    /// Creature prefabs by identity.
    pub(crate) creatures: BTreeMap<Ulid, &'a crpg_data::Creature>,
    /// Effect documents by identity.
    pub(crate) effects: BTreeMap<Ulid, &'a crpg_data::Effect>,
}

impl<'a> SpecParts<'a> {
    /// Collects every reference the encounter's ruleset needs.
    pub(crate) fn load(
        loaded: &'a LoadedCampaign,
        encounter: &crpg_data::Encounter,
    ) -> Result<Self, Fault> {
        let ruleset = loaded
            .documents
            .values()
            .find_map(|document| match document {
                Document::Ruleset(value) if value.id == encounter.ruleset => Some(value),
                _ => None,
            })
            .ok_or_else(|| Fault {
                code: BridgeCode::StartFailed,
                detail: format!("missing ruleset {}", encounter.ruleset),
            })?;
        let mut abilities = BTreeMap::new();
        for document in loaded.documents.values() {
            if let Document::Ability(value) = document {
                abilities.insert(value.id, value);
            }
        }
        for listed in &ruleset.abilities {
            if !abilities.contains_key(listed) {
                return Err(Fault {
                    code: BridgeCode::StartFailed,
                    detail: format!("missing ability {listed}"),
                });
            }
        }
        let mut outcome_tables = BTreeMap::new();
        for document in loaded.documents.values() {
            if let Document::OutcomeTable(value) = document {
                outcome_tables.insert(value.id, value);
            }
        }
        let mut placements = BTreeMap::new();
        for document in loaded.documents.values() {
            if let Document::Placements(value) = document {
                for placement in &value.placements {
                    placements.insert(
                        placement.id,
                        crpg_sim::PlacementAndArea {
                            placement,
                            area: value.area,
                        },
                    );
                }
            }
        }
        let mut creatures = BTreeMap::new();
        for document in loaded.documents.values() {
            if let Document::Creature(value) = document {
                creatures.insert(value.id, value);
            }
        }
        let mut effects = BTreeMap::new();
        for document in loaded.documents.values() {
            if let Document::Effect(value) = document {
                effects.insert(value.id, value);
            }
        }
        Ok(Self {
            ruleset,
            abilities,
            outcome_tables,
            placements,
            creatures,
            effects,
        })
    }

    /// Borrows these parts as the sim's start spec.
    pub(crate) fn spec(
        &'a self,
        encounter: &'a crpg_data::Encounter,
    ) -> crpg_sim::EncounterSpec<'a> {
        crpg_sim::EncounterSpec {
            encounter,
            ruleset: self.ruleset,
            abilities: &self.abilities,
            outcome_tables: &self.outcome_tables,
            placements: &self.placements,
            creatures: &self.creatures,
            effects: &self.effects,
        }
    }
}

#[cfg(test)]
mod tests;

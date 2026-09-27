//! Private combat intents for `crpgc replay --campaign` (T016d, T017f).
//!
//! The caller-owns-payload rule means this crate decides what a combat
//! payload means. This module decodes the version-1 grammar
//! (`{"combat": 1, "op": "init" | "attack" | "end"}`) onto exactly the two
//! public sim operations (`start_encounter`, `perform_action`) through a
//! transient borrowed spec assembled from the loaded campaign. It owns
//! decode and bind glue only; combat semantics stay in `crpg-sim`, replay
//! semantics stay in `crpg-testkit`, and validation semantics stay in
//! `crpg-data`. Nothing outside this crate may import it.

use std::collections::BTreeMap;
use std::str::FromStr;

use crpg_core::{EntityId, Ulid};
use crpg_data::{Document, LoadedCampaign};
use crpg_sim::{EncounterSpec, PlacementAndArea, World};

/// Builds the caller-owned combat apply closure over owned campaign content.
///
/// The returned closure owns the loaded documents, the placement-to-runtime
/// map, and the initialized flag, satisfying the existing `ApplyInput`
/// lifetime. Only `start_encounter` and `perform_action` mutate `World`;
/// the action result is discarded without naming further types.
pub(crate) fn combat_intents(loaded: LoadedCampaign) -> crpg_testkit::ApplyInput {
    let mut adapter = CombatAdapter::new(loaded);
    Box::new(move |world: &mut World, payload: &serde_json::Value| adapter.apply(world, payload))
}

/// Owned combat decode/bind state for one playback.
struct CombatAdapter {
    /// Owned production documents.
    loaded: LoadedCampaign,
    /// Authored placement identity to runtime entity, bound at init.
    entities: BTreeMap<Ulid, EntityId>,
    /// Whether `init` has been accepted.
    initialized: bool,
}

impl CombatAdapter {
    /// Wraps loaded production content in an uninitialized adapter.
    fn new(loaded: LoadedCampaign) -> Self {
        Self {
            loaded,
            entities: BTreeMap::new(),
            initialized: false,
        }
    }

    /// Decodes one payload and executes only the public sim operation.
    fn apply(&mut self, world: &mut World, payload: &serde_json::Value) -> Result<(), String> {
        let object = payload
            .as_object()
            .ok_or_else(|| "malformed combat payload: expected object".to_string())?;
        if object.get("combat").and_then(|value| value.as_u64()) != Some(1) {
            return Err("unsupported combat payload version".to_string());
        }
        let op = object
            .get("op")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        if op != "init" && op != "attack" && op != "end" {
            return Err("unknown combat op".to_string());
        }
        let allowed: &[&str] = if op == "init" {
            &["combat", "op", "encounter"]
        } else if op == "attack" {
            &["combat", "op", "actor", "ability", "target"]
        } else {
            &["combat", "op", "actor"]
        };
        let mut extra: Vec<&String> = object
            .keys()
            .filter(|key| !allowed.contains(&key.as_str()))
            .collect();
        extra.sort();
        if let Some(field) = extra.first() {
            return Err(format!("malformed combat {op}: unknown field {field}"));
        }
        if op == "init" {
            let encounter = decode_ulid(object, op, "encounter")?;
            return self.apply_init(world, encounter);
        }
        let actor = decode_ulid(object, op, "actor")?;
        if op == "end" {
            return self.apply_end(world, actor);
        }
        let ability = decode_ulid(object, op, "ability")?;
        let target = decode_ulid(object, op, "target")?;
        self.apply_attack(world, actor, ability, target)
    }

    /// Binds the named encounter and publishes it through `start_encounter`.
    fn apply_init(&mut self, world: &mut World, encounter_id: Ulid) -> Result<(), String> {
        if self.initialized {
            return Err("duplicate combat init".to_string());
        }
        let encounter = self
            .loaded
            .documents
            .values()
            .find_map(|doc| match doc {
                Document::Encounter(value) if value.id == encounter_id => Some(value),
                _ => None,
            })
            .ok_or_else(|| format!("unknown encounter {encounter_id}"))?;
        let ruleset = self
            .loaded
            .documents
            .values()
            .find_map(|doc| match doc {
                Document::Ruleset(value) if value.id == encounter.ruleset => Some(value),
                _ => None,
            })
            .ok_or_else(|| format!("init failed: missing ruleset {}", encounter.ruleset))?;
        let mut abilities = BTreeMap::new();
        for doc in self.loaded.documents.values() {
            if let Document::Ability(value) = doc {
                abilities.insert(value.id, value);
            }
        }
        for listed in &ruleset.abilities {
            if !abilities.contains_key(listed) {
                return Err(format!("init failed: missing ability {listed}"));
            }
        }
        let mut outcome_tables = BTreeMap::new();
        for doc in self.loaded.documents.values() {
            if let Document::OutcomeTable(value) = doc {
                outcome_tables.insert(value.id, value);
            }
        }
        let mut placements: BTreeMap<Ulid, PlacementAndArea<'_>> = BTreeMap::new();
        for doc in self.loaded.documents.values() {
            if let Document::Placements(value) = doc {
                for placement in &value.placements {
                    placements.insert(
                        placement.id,
                        PlacementAndArea {
                            placement,
                            area: value.area,
                        },
                    );
                }
            }
        }
        let mut creatures = BTreeMap::new();
        for doc in self.loaded.documents.values() {
            if let Document::Creature(value) = doc {
                creatures.insert(value.id, value);
            }
        }
        let mut effects = BTreeMap::new();
        for doc in self.loaded.documents.values() {
            if let Document::Effect(value) = doc {
                effects.insert(value.id, value);
            }
        }
        let spec = EncounterSpec {
            encounter,
            ruleset,
            abilities: &abilities,
            outcome_tables: &outcome_tables,
            placements: &placements,
            creatures: &creatures,
            effects: &effects,
        };
        crpg_sim::start_encounter(world, &spec).map_err(|error| format!("init failed: {error}"))?;
        let bound: Vec<(EntityId, Ulid)> = world
            .combatants()
            .iter()
            .map(|(entity, combatant)| (entity, combatant.placement()))
            .collect();
        if bound.len() != encounter.participants.len() {
            return Err(format!(
                "init failed: placement binding mismatch ({} combatants, {} participants)",
                bound.len(),
                encounter.participants.len()
            ));
        }
        for (index, (entity, placement)) in bound.iter().enumerate() {
            let expected = encounter.participants[index].placement;
            if *placement != expected {
                return Err(format!(
                    "init failed: placement binding mismatch at index {index} (expected {expected}, found {placement})"
                ));
            }
            self.entities.insert(expected, *entity);
        }
        let definition_ability = world
            .combat()
            .ok_or_else(|| "init failed: missing encounter state".to_string())?
            .definition
            .ability;
        let expected_ability = ruleset
            .abilities
            .first()
            .ok_or_else(|| "init failed: ruleset lists no ability".to_string())?;
        if definition_ability != *expected_ability {
            return Err(format!(
                "init failed: ability binding mismatch (expected {expected_ability}, found {definition_ability})"
            ));
        }
        self.initialized = true;
        Ok(())
    }

    /// Resolves bound placements and applies one authoritative action.
    fn apply_attack(
        &mut self,
        world: &mut World,
        actor_placement: Ulid,
        ability: Ulid,
        target_placement: Ulid,
    ) -> Result<(), String> {
        if !self.initialized {
            return Err("combat not initialized".to_string());
        }
        let actor = self
            .entities
            .get(&actor_placement)
            .copied()
            .ok_or_else(|| format!("unknown combat identity actor: {actor_placement}"))?;
        let target = self
            .entities
            .get(&target_placement)
            .copied()
            .ok_or_else(|| format!("unknown combat identity target: {target_placement}"))?;
        crpg_sim::perform_action(
            world,
            &crpg_sim::CombatAction::UseAbility {
                actor,
                ability,
                target,
            },
        )
        .map(|_| ())
        .map_err(|error| format!("attack failed: {error}"))?;
        Ok(())
    }

    /// Completes the bound actor's turn without spending, drawing, damage,
    /// or effect. The initialized guard runs before identity resolution,
    /// exactly like `apply_attack`; sim turn-holding checks run last.
    fn apply_end(&mut self, world: &mut World, actor_placement: Ulid) -> Result<(), String> {
        if !self.initialized {
            return Err("combat not initialized".to_string());
        }
        let actor = self
            .entities
            .get(&actor_placement)
            .copied()
            .ok_or_else(|| format!("unknown combat identity actor: {actor_placement}"))?;
        crpg_sim::perform_action(world, &crpg_sim::CombatAction::EndTurn { actor })
            .map_err(|error| format!("end failed: {error}"))
            .and_then(require_end_outcome)
    }
}

/// Guards the defensive successful-result shape of an accepted `end`.
///
/// Pure: no World access, no sim backend, no public API. `None` (the only
/// result the controller returns for `EndTurn`) maps to `Ok(())`; an
/// impossible `Some(_)` maps to the exact internal-violation report. The
/// caller reports it and aborts playback; the already-successful call is
/// not claimed to have rolled back, so no cloning machinery exists for
/// this branch.
fn require_end_outcome<T>(outcome: Option<T>) -> Result<(), String> {
    match outcome {
        None => Ok(()),
        Some(_) => Err("end failed: unexpected outcome".to_string()),
    }
}

/// Decodes one required ULID string field in payload order.
fn decode_ulid(
    object: &serde_json::Map<String, serde_json::Value>,
    op: &str,
    field: &str,
) -> Result<Ulid, String> {
    let text = object
        .get(field)
        .and_then(|value| value.as_str())
        .ok_or_else(|| format!("malformed combat {op}: {field} must be a ULID string"))?;
    Ulid::from_str(text).map_err(|_| format!("malformed combat {op}: invalid {field}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Loads the checked-in combat campaign through the production collector
    /// and loader (read-only; no test-only tree reader is duplicated here).
    fn load_combat_basic() -> LoadedCampaign {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("campaigns")
            .join("fixtures")
            .join("combat_basic");
        let files = crate::collect_campaign_files(&root).expect("combat_basic collects");
        assert_eq!(files.len(), 14);
        let engine = "0.1.0".parse().expect("engine parses");
        let loaded = crpg_data::load_campaign(&files, &engine).expect("combat_basic loads");
        assert!(crpg_data::validate(&loaded).is_empty());
        loaded
    }

    /// Loads the checked-in srd campaign through the production collector
    /// and loader (read-only; no test-only tree reader is duplicated here).
    fn load_combat_srd() -> LoadedCampaign {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("campaigns")
            .join("fixtures")
            .join("combat_srd");
        let files = crate::collect_campaign_files(&root).expect("combat_srd collects");
        assert_eq!(files.len(), 17);
        let engine = "0.1.0".parse().expect("engine parses");
        let loaded = crpg_data::load_campaign(&files, &engine).expect("combat_srd loads");
        assert!(crpg_data::validate(&loaded).is_empty());
        loaded
    }

    fn hero() -> Ulid {
        Ulid::from_u128(14)
    }

    fn goblin() -> Ulid {
        Ulid::from_u128(15)
    }

    fn srd_hero() -> Ulid {
        Ulid::from_u128(34)
    }

    fn srd_goblin() -> Ulid {
        Ulid::from_u128(35)
    }

    fn srd_encounter() -> Ulid {
        Ulid::from_u128(44)
    }

    fn end_payload(actor: Ulid) -> serde_json::Value {
        json!({
            "combat": 1,
            "op": "end",
            "actor": actor.to_string(),
        })
    }

    fn ability() -> Ulid {
        Ulid::from_u128(19)
    }

    fn encounter() -> Ulid {
        Ulid::from_u128(21)
    }

    fn init_payload() -> serde_json::Value {
        json!({
            "combat": 1,
            "op": "init",
            "encounter": encounter().to_string(),
        })
    }

    fn attack_payload(actor: Ulid, target: Ulid) -> serde_json::Value {
        json!({
            "combat": 1,
            "op": "attack",
            "actor": actor.to_string(),
            "ability": ability().to_string(),
            "target": target.to_string(),
        })
    }

    /// Serialized full-world bytes for rollback comparison.
    fn world_bytes(world: &World) -> Vec<u8> {
        serde_json::to_vec(world).expect("world serializes")
    }

    #[test]
    fn init_publishes_binding_and_first_attack_succeeds() {
        let mut adapter = CombatAdapter::new(load_combat_basic());
        let mut world = World::new(0);
        adapter
            .apply(&mut world, &init_payload())
            .expect("init applies");
        assert!(adapter.initialized);
        assert_eq!(adapter.entities.len(), 2);
        let before = world_bytes(&world);
        // The goblin (initiative 9) holds the opening turn.
        adapter
            .apply(&mut world, &attack_payload(goblin(), hero()))
            .expect("opening attack applies");
        assert_ne!(before, world_bytes(&world));
        assert!(adapter.initialized);
        assert_eq!(adapter.entities.len(), 2);
    }

    #[test]
    fn decode_precedence_names_first_failure() {
        let mut adapter = CombatAdapter::new(load_combat_basic());
        let mut world = World::new(0);
        // Non-object beats everything.
        assert_eq!(
            adapter
                .apply(&mut world, &json!([1, 2]))
                .expect_err("array"),
            "malformed combat payload: expected object"
        );
        // Version beats op.
        assert_eq!(
            adapter
                .apply(&mut world, &json!({"op": "init"}))
                .expect_err("missing version"),
            "unsupported combat payload version"
        );
        assert_eq!(
            adapter
                .apply(&mut world, &json!({"combat": 2, "op": "init"}))
                .expect_err("wrong version"),
            "unsupported combat payload version"
        );
        assert_eq!(
            adapter
                .apply(&mut world, &json!({"combat": "1", "op": "init"}))
                .expect_err("string version"),
            "unsupported combat payload version"
        );
        // Unknown op beats unknown fields and missing ULIDs.
        assert_eq!(
            adapter
                .apply(&mut world, &json!({"combat": 1, "op": "retreat"}))
                .expect_err("unknown op"),
            "unknown combat op"
        );
        assert_eq!(
            adapter
                .apply(&mut world, &json!({"combat": 1}))
                .expect_err("missing op"),
            "unknown combat op"
        );
        // Unknown fields beat missing required fields; lexically smallest wins.
        assert_eq!(
            adapter
                .apply(
                    &mut world,
                    &json!({"combat": 1, "op": "init", "zz": 0, "aa": 0})
                )
                .expect_err("extra fields"),
            "malformed combat init: unknown field aa"
        );
        // Required ULIDs in order: encounter for init; actor, ability, target.
        assert_eq!(
            adapter
                .apply(&mut world, &json!({"combat": 1, "op": "init"}))
                .expect_err("missing encounter"),
            "malformed combat init: encounter must be a ULID string"
        );
        assert_eq!(
            adapter
                .apply(
                    &mut world,
                    &json!({"combat": 1, "op": "attack", "ability": ability().to_string(), "target": hero().to_string()}),
                )
                .expect_err("missing actor"),
            "malformed combat attack: actor must be a ULID string"
        );
        // Non-string and unparseable ULIDs keep their split reasons.
        assert_eq!(
            adapter
                .apply(
                    &mut world,
                    &json!({"combat": 1, "op": "init", "encounter": 12})
                )
                .expect_err("numeric encounter"),
            "malformed combat init: encounter must be a ULID string"
        );
        assert_eq!(
            adapter
                .apply(
                    &mut world,
                    &json!({"combat": 1, "op": "init", "encounter": "not-a-ulid!!!!!!!!!!!!!!!!"}),
                )
                .expect_err("bad encounter"),
            "malformed combat init: invalid encounter"
        );
        assert_eq!(
            adapter
                .apply(
                    &mut world,
                    &json!({"combat": 1, "op": "attack", "actor": "bad", "ability": ability().to_string(), "target": hero().to_string()}),
                )
                .expect_err("bad actor"),
            "malformed combat attack: invalid actor"
        );
    }

    #[test]
    fn state_and_identity_precedence_with_rollback() {
        let mut adapter = CombatAdapter::new(load_combat_basic());
        let mut world = World::new(0);
        let pristine = world_bytes(&world);
        // Attack before init beats unknown identities.
        let unknown = Ulid::from_u128(999).to_string();
        assert_eq!(
            adapter
                .apply(
                    &mut world,
                    &json!({"combat": 1, "op": "attack", "actor": unknown, "ability": ability().to_string(), "target": hero().to_string()}),
                )
                .expect_err("pre-init"),
            "combat not initialized"
        );
        assert_eq!(pristine, world_bytes(&world));
        assert!(!adapter.initialized);
        assert!(adapter.entities.is_empty());
        // Unknown encounter identity after successful decode.
        assert_eq!(
            adapter
                .apply(
                    &mut world,
                    &json!({"combat": 1, "op": "init", "encounter": unknown})
                )
                .expect_err("unknown encounter"),
            format!("unknown encounter {unknown}")
        );
        assert_eq!(pristine, world_bytes(&world));
        assert!(!adapter.initialized);
        // Successful init publishes the binding.
        adapter
            .apply(&mut world, &init_payload())
            .expect("init applies");
        assert!(adapter.initialized);
        let bound = world_bytes(&world);
        // Duplicate init beats a second unknown-encounter lookup.
        assert_eq!(
            adapter
                .apply(
                    &mut world,
                    &json!({"combat": 1, "op": "init", "encounter": unknown})
                )
                .expect_err("duplicate"),
            "duplicate combat init"
        );
        assert_eq!(bound, world_bytes(&world));
        assert_eq!(adapter.entities.len(), 2);
        // Actor resolves before target.
        assert_eq!(
            adapter
                .apply(
                    &mut world,
                    &json!({"combat": 1, "op": "attack", "actor": unknown, "ability": ability().to_string(), "target": hero().to_string()}),
                )
                .expect_err("unknown actor"),
            format!("unknown combat identity actor: {unknown}")
        );
        assert_eq!(
            adapter
                .apply(
                    &mut world,
                    &json!({"combat": 1, "op": "attack", "actor": goblin().to_string(), "ability": ability().to_string(), "target": unknown}),
                )
                .expect_err("unknown target"),
            format!("unknown combat identity target: {unknown}")
        );
        assert_eq!(bound, world_bytes(&world));
        assert_eq!(adapter.entities.len(), 2);
    }

    #[test]
    fn sim_rejections_preserve_full_state() {
        let mut adapter = CombatAdapter::new(load_combat_basic());
        let mut world = World::new(0);
        adapter
            .apply(&mut world, &init_payload())
            .expect("init applies");
        let bound = world_bytes(&world);
        // A well-formed but unknown ability reaches sim validation verbatim.
        let strange = Ulid::from_u128(999).to_string();
        assert_eq!(
            adapter
                .apply(
                    &mut world,
                    &json!({"combat": 1, "op": "attack", "actor": goblin().to_string(), "ability": strange, "target": hero().to_string()}),
                )
                .expect_err("unknown ability"),
            "attack failed: UnknownAbility at combat/ability"
        );
        assert_eq!(bound, world_bytes(&world));
        assert_eq!(adapter.entities.len(), 2);
        // The hero acts out of turn on the opening tick.
        assert_eq!(
            adapter
                .apply(&mut world, &attack_payload(hero(), goblin()))
                .expect_err("out of turn"),
            "attack failed: OutOfTurn at combat/active"
        );
        assert_eq!(bound, world_bytes(&world));
        assert!(adapter.initialized);
    }

    #[test]
    fn multifault_rows_pin_decode_bind_sim_order() {
        let mut adapter = CombatAdapter::new(load_combat_basic());
        let mut world = World::new(0);
        // Unknown field beats a missing required ULID.
        assert_eq!(
            adapter
                .apply(
                    &mut world,
                    &json!({"combat": 1, "op": "attack", "actor": "x", "extra": 0}),
                )
                .expect_err("field first"),
            "malformed combat attack: unknown field extra"
        );
        // Missing actor beats a missing ability/target.
        assert_eq!(
            adapter
                .apply(&mut world, &json!({"combat": 1, "op": "attack"}))
                .expect_err("actor first"),
            "malformed combat attack: actor must be a ULID string"
        );
        // Duplicate init beats the unknown-encounter identity on a second init.
        adapter
            .apply(&mut world, &init_payload())
            .expect("init applies");
        let unknown = Ulid::from_u128(999).to_string();
        assert_eq!(
            adapter
                .apply(
                    &mut world,
                    &json!({"combat": 1, "op": "init", "encounter": unknown})
                )
                .expect_err("duplicate first"),
            "duplicate combat init"
        );
        // Unknown actor beats sim validation for a well-formed ability.
        assert_eq!(
            adapter
                .apply(
                    &mut world,
                    &json!({"combat": 1, "op": "attack", "actor": unknown, "ability": ability().to_string(), "target": hero().to_string()}),
                )
                .expect_err("identity first"),
            format!("unknown combat identity actor: {unknown}")
        );
    }

    #[test]
    fn require_end_outcome_maps_shapes_exactly() {
        // Synthetic only: the controller never returns Some for EndTurn, so
        // no World corruption is arranged to force the unreachable arm.
        assert_eq!(require_end_outcome(None::<()>), Ok(()));
        assert_eq!(
            require_end_outcome(Some(())),
            Err("end failed: unexpected outcome".to_string())
        );
    }

    #[test]
    fn end_decode_precedence_names_first_failure() {
        let mut adapter = CombatAdapter::new(load_combat_basic());
        let mut world = World::new(0);
        // Non-object beats everything, even for end.
        assert_eq!(
            adapter
                .apply(&mut world, &json!([1, 2]))
                .expect_err("array"),
            "malformed combat payload: expected object"
        );
        // Version beats op.
        assert_eq!(
            adapter
                .apply(&mut world, &json!({"combat": 2, "op": "end", "actor": "x"}))
                .expect_err("wrong version"),
            "unsupported combat payload version"
        );
        // Op membership beats unknown fields and missing ULIDs.
        assert_eq!(
            adapter
                .apply(
                    &mut world,
                    &json!({"combat": 1, "op": "rest", "actor": "x"})
                )
                .expect_err("unknown op"),
            "unknown combat op"
        );
        // Unknown fields beat a missing actor; lexically smallest wins.
        assert_eq!(
            adapter
                .apply(
                    &mut world,
                    &json!({"combat": 1, "op": "end", "zz": 0, "aa": 0})
                )
                .expect_err("extra fields"),
            "malformed combat end: unknown field aa"
        );
        // Attack-only fields are unknown on end.
        assert_eq!(
            adapter
                .apply(
                    &mut world,
                    &json!({"combat": 1, "op": "end", "actor": goblin().to_string(), "ability": "x"}),
                )
                .expect_err("ability is extra on end"),
            "malformed combat end: unknown field ability"
        );
        // Missing and non-string actors keep the must-be-string reason.
        assert_eq!(
            adapter
                .apply(&mut world, &json!({"combat": 1, "op": "end"}))
                .expect_err("missing actor"),
            "malformed combat end: actor must be a ULID string"
        );
        assert_eq!(
            adapter
                .apply(&mut world, &json!({"combat": 1, "op": "end", "actor": 12}))
                .expect_err("numeric actor"),
            "malformed combat end: actor must be a ULID string"
        );
        // Unparseable actor beats the initialized guard: decoding precedes it.
        assert_eq!(
            adapter
                .apply(
                    &mut world,
                    &json!({"combat": 1, "op": "end", "actor": "bad"})
                )
                .expect_err("bad actor"),
            "malformed combat end: invalid actor"
        );
        assert!(!adapter.initialized);
    }

    #[test]
    fn end_guards_identity_and_sim_rejections_preserve_full_state() {
        let mut adapter = CombatAdapter::new(load_combat_basic());
        let mut world = World::new(0);
        let pristine = world_bytes(&world);
        // A valid actor before init fails the guard before binding lookup.
        assert_eq!(
            adapter
                .apply(&mut world, &end_payload(goblin()))
                .expect_err("pre-init"),
            "combat not initialized"
        );
        assert_eq!(pristine, world_bytes(&world));
        assert!(adapter.entities.is_empty());
        adapter
            .apply(&mut world, &init_payload())
            .expect("init applies");
        let bound = world_bytes(&world);
        // Syntactically valid but unbound identity.
        let unknown = Ulid::from_u128(999).to_string();
        assert_eq!(
            adapter
                .apply(
                    &mut world,
                    &json!({"combat": 1, "op": "end", "actor": unknown})
                )
                .expect_err("unknown actor"),
            format!("unknown combat identity actor: {unknown}")
        );
        assert_eq!(bound, world_bytes(&world));
        assert_eq!(adapter.entities.len(), 2);
        // The hero ends out of turn on the opening tick (goblin holds it).
        assert_eq!(
            adapter
                .apply(&mut world, &end_payload(hero()))
                .expect_err("out of turn"),
            "end failed: OutOfTurn at combat/active"
        );
        assert_eq!(bound, world_bytes(&world));
        assert_eq!(adapter.entities.len(), 2);
        assert!(adapter.initialized);
    }

    #[test]
    fn accepted_end_advances_without_rng_draw() {
        let mut adapter = CombatAdapter::new(load_combat_basic());
        let mut world = World::new(0);
        adapter
            .apply(&mut world, &init_payload())
            .expect("init applies");
        let rng_before = serde_json::to_value(&world).expect("world serializes")["rng"].clone();
        // The goblin holds the opening turn; ending it is accepted.
        adapter
            .apply(&mut world, &end_payload(goblin()))
            .expect("accepted end applies");
        let after = serde_json::to_value(&world).expect("world serializes");
        assert_eq!(after["rng"], rng_before, "an accepted end draws nothing");
        assert_eq!(adapter.entities.len(), 2);
        assert!(adapter.initialized);
        // Core ULID aliases normalize through decode: the lowercase hero
        // string names the same identity and ends the turn it now holds.
        let lower = hero().to_string().to_lowercase();
        assert_ne!(lower, hero().to_string());
        adapter
            .apply(
                &mut world,
                &json!({"combat": 1, "op": "end", "actor": lower}),
            )
            .expect("alias end applies");
    }

    #[test]
    fn srd_end_decodes_against_srd_content() {
        let mut adapter = CombatAdapter::new(load_combat_srd());
        let mut world = World::new(285);
        adapter
            .apply(
                &mut world,
                &json!({"combat": 1, "op": "init", "encounter": srd_encounter().to_string()}),
            )
            .expect("srd init applies");
        let bound = world_bytes(&world);
        assert_eq!(adapter.entities.len(), 2);
        // The hero ends out of turn while the goblin holds the srd opening.
        assert_eq!(
            adapter
                .apply(&mut world, &end_payload(srd_hero()))
                .expect_err("out of turn"),
            "end failed: OutOfTurn at combat/active"
        );
        assert_eq!(bound, world_bytes(&world));
        // The goblin's own end dispatches through the real sim.
        adapter
            .apply(&mut world, &end_payload(srd_goblin()))
            .expect("accepted srd end applies");
        assert_eq!(adapter.entities.len(), 2);
        assert!(adapter.initialized);
    }
}

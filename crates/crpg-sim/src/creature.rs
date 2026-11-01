//! Persistent creature state outside encounters (T139 D1).
//!
//! A [`CreatureState`] (a *record*) is the only copy of a creature while it is
//! outside every encounter; while it fights, the `Combatant` is the only
//! copy, and an entity never holds both. [`register_creature`] instantiates
//! one authored placement as a record without starting an encounter. Nothing
//! here draws RNG, interns a string, schedules a turn or reads the tick:
//! outside an encounter nothing happens to a creature (T139 D9).

use serde::{Deserialize, Serialize};

use crpg_core::{EntityId, Fx16_16, Ulid};
use crpg_data::{
    Creature as AuthoredCreature, Placement as AuthoredPlacement, Ruleset as AuthoredRuleset,
};
use crpg_rules::{ResourcePool, ResourcePoolId, MAX_TAGS};

use crate::combat::{
    convert_refresh, read_creature_stats, CombatError, CreatureStats, MAX_COMBAT_STATS,
};
use crate::world::{EntityMeta, World};

/// One suspended effect attachment: the rounds it had left when its holder
/// left an encounter (T139 D9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SuspendedEffect {
    /// The effect identity.
    pub effect: Ulid,
    /// Rounds left; at least one.
    pub remaining_rounds: u32,
}

/// One creature's persistent state while it is outside every encounter
/// (T139 D1). Keyed by entity in [`World::creatures`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreatureState {
    pub(crate) placement: Ulid,
    pub(crate) ruleset: Ulid,
    pub(crate) attributes: Vec<(String, i32)>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) fixed: Vec<(String, Fx16_16)>,
    pub(crate) health: u32,
    pub(crate) max_health: u32,
    pub(crate) pools: Vec<ResourcePool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) suspended: Vec<SuspendedEffect>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) tags: Vec<String>,
}

impl CreatureState {
    /// The placement this creature was instantiated from.
    pub fn placement(&self) -> Ulid {
        self.placement
    }

    /// The ruleset of its last registration or exit.
    pub fn ruleset(&self) -> Ulid {
        self.ruleset
    }

    /// Base non-health, non-derived `Int` stats in authored stat order.
    pub fn attributes(&self) -> &[(String, i32)] {
        &self.attributes
    }

    /// Base non-derived `Fixed` stats in authored stat order (D6).
    pub fn fixed(&self) -> &[(String, Fx16_16)] {
        &self.fixed
    }

    /// Current health; zero means dead.
    pub fn health(&self) -> u32 {
        self.health
    }

    /// Health maximum, at least one.
    pub fn max_health(&self) -> u32 {
        self.max_health
    }

    /// `health == 0`: the dead stay dead (D2).
    pub fn dead(&self) -> bool {
        self.health == 0
    }

    /// Pool balances in ruleset template order, primary first.
    pub fn pools(&self) -> &[ResourcePool] {
        &self.pools
    }

    /// Suspended attachments, ascending effect identity.
    pub fn suspended(&self) -> &[SuspendedEffect] {
        &self.suspended
    }

    /// The creature's own tags, ascending and distinct; never a grant (D5).
    pub fn tags(&self) -> &[String] {
        &self.tags
    }
}

/// Resolved references one registration consumes (T139).
#[derive(Debug)]
pub struct CreatureSpec<'a> {
    /// The ruleset the creature is instantiated under.
    pub ruleset: &'a AuthoredRuleset,
    /// The placed-creature document.
    pub placement: &'a AuthoredPlacement,
    /// The area owning the placement.
    pub area: Ulid,
    /// The placement's prefab.
    pub creature: &'a AuthoredCreature,
}

/// Instantiates one placement outside any encounter: spawns a fresh entity
/// holding a full-health record with pools at maximum and the prefab's
/// tags (B3, B4).
///
/// Checks in order, mutating nothing on failure: `TooManyStats` (R1),
/// `AreaMismatch` (R2), `MissingCreature` (R3), the prefab's stat errors in
/// start's order (R4), `PlacementInUse` (R5) and `TooManyTags` (R6). It draws
/// no RNG, interns nothing, adds no transform and no timeline entry, and is
/// allowed while an encounter is active. On success it enqueues one
/// `Spawned`.
///
/// # Panics
///
/// With start's text if `ruleset.pools` is empty, which data read-time
/// checks rule out.
pub fn register_creature(
    world: &mut World,
    spec: &CreatureSpec<'_>,
) -> Result<EntityId, CombatError> {
    // R1.
    if spec.ruleset.stats.len() > MAX_COMBAT_STATS {
        return Err(CombatError::TooManyStats);
    }
    // R2.
    if let Some(expected) = world.area() {
        if expected != spec.area {
            return Err(CombatError::AreaMismatch {
                expected,
                found: spec.area,
            });
        }
    }
    // R3.
    if spec.creature.id != spec.placement.prefab {
        return Err(CombatError::MissingCreature {
            prefab: spec.placement.prefab,
        });
    }
    // R4.
    let CreatureStats {
        attributes,
        health,
        fixed,
        tags,
    } = read_creature_stats(spec.ruleset, spec.creature)?;
    // R5.
    let placement = spec.placement.id;
    let held = world
        .creatures()
        .iter()
        .any(|(_, record)| record.placement == placement)
        || world
            .combatants()
            .iter()
            .any(|(_, combatant)| combatant.placement() == placement);
    if held {
        return Err(CombatError::PlacementInUse { placement });
    }
    // R6: no grant applies outside an encounter (D5).
    if tags.len() > MAX_TAGS {
        return Err(CombatError::TooManyTags);
    }
    if spec.ruleset.pools.is_empty() {
        panic!("ruleset pools are validated nonempty by data read-time checks");
    }
    let pools: Vec<ResourcePool> = spec
        .ruleset
        .pools
        .iter()
        .map(|template| {
            ResourcePool::new(
                ResourcePoolId(template.id),
                template.max,
                template.max,
                convert_refresh(template.refresh),
            )
            .expect("pool templates carry validated data values")
        })
        .collect();
    let entity = world.spawn(EntityMeta {});
    world.creatures_mut().insert(
        entity,
        CreatureState {
            placement,
            ruleset: spec.ruleset.id,
            attributes,
            fixed,
            health,
            max_health: health,
            pools,
            suspended: Vec::new(),
            tags,
        },
    );
    Ok(entity)
}

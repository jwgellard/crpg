//! The simulation skeleton: [`World`] and [`EntityMeta`].
//!
//! `World` owns one loaded area's worth of state: the entity arena (identity,
//! inherited from `crpg-core` with all five arena invariants), one component
//! store per engine component type, the timeline container, the live event
//! queue, the deterministic RNG, and the tick counter. Systems are ordinary
//! functions over `&mut World` called in a fixed hand-written order — but
//! that order, and the `tick` function that runs it, belong to T008, not
//! here. This module builds the data structure and its spawn/despawn/query
//! surface; it advances nothing.
//!
//! Serde covers the skeleton plus the T016b combat layer (E014):
//! entities, stores, timeline, event queue, RNG, tick, combatants,
//! encounter state, and the issuing interner. Stat references in combat
//! state persist as symbolic strings through the explicit conversion the
//! adapter performs — never a derived `Serialize` on a map with `StatId`
//! keys. If a struct in this module ever holds an interned handle, that
//! struct does not derive serde; it converts.
//!
//! Loading is validated: component and timeline ids must address live
//! entities, duplicate timeline entities are rejected alongside the
//! [`Timeline`](crate::Timeline) guard, and encounter state must agree
//! with its combatant store (no dangling combatants, no encounter without
//! combatants or vice versa, no dangling active turn). Combat saves carry
//! the full coherence check (T016e): the interned roll tag, definition
//! attribute/cost declarations, per-combatant health/dead/maximum agreement
//! with pool/attribute/placement identity, and active/timeline scheduling
//! against live non-dead combatants.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crpg_core::{DeterministicRng, EntityId, EventQueue, GenerationalArena, Interners, Tick, Ulid};

use crate::combat::{CombatState, Combatant, TransitionFacts, COMBAT_ROLL_TAG};
use crate::event::SimEvent;
use crate::store::ComponentStore;
use crate::timeline::Timeline;
use crate::transform::Transform;

/// Per-entity metadata: reserved for now.
///
/// The arena needs a value type per slot, and the spec sketch names this one,
/// but no consumer exists yet — no areas, no persistence, no authority model
/// — so any fields would be fiction a later task has to contradict. It is
/// passed explicitly at [`spawn`](World::spawn) so the call signature is
/// already stable for the day areas give it fields.
///
/// Deliberately a *braced* empty struct rather than a unit struct: the arena
/// stores slots as `Option<T>`, and in JSON a unit struct serializes as
/// `null` — indistinguishable from `None`. Every occupied slot would load as
/// vacant and the arena guard would reject its own save as corrupt (which is
/// exactly how this was found). `{}` and `null` stay distinct.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntityMeta {}

/// One loaded area's simulation state.
///
/// Areas simulate independently (spec §2.4): cross-area effects travel
/// through a message queue on the `Campaign` object, which does not exist
/// yet. Likewise single-player runs this same struct in-process (spec §2.1);
/// there is no second world type for it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct World {
    entities: GenerationalArena<EntityMeta>,
    transforms: ComponentStore<Transform>,
    timeline: Timeline,
    events: EventQueue<SimEvent>,
    rng: DeterministicRng,
    tick: Tick,
    /// Authoritative per-combatant encounter state (T016b, ADR-0013).
    ///
    /// Absent (empty) in non-combat worlds, so those serialize
    /// byte-identically to the T007/T008a skeleton. This is a
    /// compatibility shape, not a hash exclusion: when combat is active
    /// every field hashes.
    #[serde(default, skip_serializing_if = "ComponentStore::is_empty")]
    combatants: ComponentStore<Combatant>,
    /// Encounter-level state; `None` when no encounter is active.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    combat: Option<CombatState>,
    /// The issuing interner (T016b, ADR-0013).
    ///
    /// Persists as its ordered string list, so no interned handle is ever
    /// serialized (E014). Empty in worlds that never started an encounter.
    #[serde(default, skip_serializing_if = "interners_empty")]
    interners: Interners,
    /// The authored area this world simulates (T027a, ADR-0020).
    ///
    /// Immutable after construction; `None` for legacy unbound worlds, which
    /// omit the field and serialize byte-identically. An absence
    /// compatibility shape, not a hash exclusion: a present area hashes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    area: Option<Ulid>,
}

/// Reports whether an interner holds no strings in either namespace.
fn interners_empty(interners: &Interners) -> bool {
    interners.stats().is_empty() && interners.tags().is_empty()
}

/// Serialized shape of [`World`], in field order. Deserialization goes
/// through this shape and then validates cross-field invariants before
/// constructing a `World`, so persisted state cannot introduce dangling
/// component or timeline references.
#[derive(Deserialize)]
struct WorldRepr {
    entities: GenerationalArena<EntityMeta>,
    transforms: ComponentStore<Transform>,
    timeline: Timeline,
    events: EventQueue<SimEvent>,
    rng: DeterministicRng,
    tick: Tick,
    #[serde(default = "ComponentStore::new")]
    combatants: ComponentStore<Combatant>,
    #[serde(default)]
    combat: Option<CombatState>,
    #[serde(default)]
    interners: Interners,
    #[serde(default)]
    area: Option<Ulid>,
}

impl<'de> Deserialize<'de> for World {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let repr = WorldRepr::deserialize(deserializer)?;
        for (id, _) in repr.transforms.iter() {
            if !repr.entities.contains(id) {
                return Err(serde::de::Error::custom(format!(
                    "dangling transform for non-live {id:?}"
                )));
            }
        }
        for (id, _) in repr.combatants.iter() {
            if !repr.entities.contains(id) {
                return Err(serde::de::Error::custom(format!(
                    "dangling combatant for non-live {id:?}"
                )));
            }
        }
        for (_, id) in repr.timeline.iter() {
            if !repr.entities.contains(id) {
                return Err(serde::de::Error::custom(format!(
                    "dangling timeline entry for non-live {id:?}"
                )));
            }
        }
        match &repr.combat {
            Some(state) => {
                if repr.combatants.is_empty() {
                    return Err(serde::de::Error::custom(
                        "encounter state with an empty combatant store",
                    ));
                }
                if let Some(active) = state.active {
                    if !repr.entities.contains(active) || !repr.combatants.contains(active) {
                        return Err(serde::de::Error::custom(format!(
                            "dangling active turn entity {active:?}"
                        )));
                    }
                }
                // Combat coherence (T016e): the controller expects the
                // interned roll tag, the declared checked attribute, fitting
                // costs, and scheduled live combatants, so a save breaking
                // any of them fails here instead of publishing a World that
                // panics or deadlocks on its next action.
                if repr.interners.tag(COMBAT_ROLL_TAG).is_none() {
                    return Err(serde::de::Error::custom(
                        "combat encounter without its interned roll tag",
                    ));
                }
                let definition = &state.definition;
                if definition.attribute_names.is_empty() {
                    return Err(serde::de::Error::custom(
                        "combat definition declares no attributes",
                    ));
                }
                {
                    let mut seen = BTreeSet::new();
                    for name in &definition.attribute_names {
                        if !seen.insert(name) {
                            return Err(serde::de::Error::custom(
                                "combat definition declares a duplicate attribute",
                            ));
                        }
                    }
                }
                if !definition.attribute_names.contains(&definition.attribute) {
                    return Err(serde::de::Error::custom(
                        "combat definition names an undeclared checked attribute",
                    ));
                }
                if definition.health_stat.is_empty()
                    || definition.attribute_names.contains(&definition.health_stat)
                {
                    return Err(serde::de::Error::custom(
                        "combat definition misdeclares its health stat",
                    ));
                }
                if definition.pools.is_empty() {
                    return Err(serde::de::Error::custom(
                        "combat definition declares no pools",
                    ));
                }
                if definition.abilities.is_empty() {
                    return Err(serde::de::Error::custom(
                        "combat definition declares no abilities",
                    ));
                }
                {
                    let mut seen = BTreeSet::new();
                    for template in &definition.pools {
                        if !seen.insert(template.id) {
                            return Err(serde::de::Error::custom(
                                "combat definition declares a duplicate pool",
                            ));
                        }
                    }
                }
                for pair in definition.abilities.windows(2) {
                    if pair[0].ability >= pair[1].ability {
                        return Err(serde::de::Error::custom(
                            "combat definition abilities break ascending order",
                        ));
                    }
                }
                for pair in definition.effects.windows(2) {
                    if pair[0].effect >= pair[1].effect {
                        return Err(serde::de::Error::custom(
                            "combat definition effects break ascending order",
                        ));
                    }
                }
                if definition.pool_id != definition.pools[0].id
                    || definition.pool_max != definition.pools[0].max
                    || definition.pool_refresh != definition.pools[0].refresh
                {
                    return Err(serde::de::Error::custom(
                        "combat definition primary pool disagrees with its templates",
                    ));
                }
                if definition.cost > definition.pool_max {
                    return Err(serde::de::Error::custom(
                        "combat definition cost exceeds its pool maximum",
                    ));
                }
                {
                    let Some(first) = definition
                        .abilities
                        .iter()
                        .find(|entry| entry.ability == definition.ability)
                    else {
                        return Err(serde::de::Error::custom(
                            "combat definition legacy ability outside its abilities",
                        ));
                    };
                    if first.dice != definition.dice
                        || first.attribute != definition.attribute
                        || first.damage != definition.damage
                        || first.requires_target != definition.requires_target
                        || first.allow_self_target != definition.allow_self_target
                        || first.outcome_table != definition.outcome_table
                    {
                        return Err(serde::de::Error::custom(
                            "combat definition legacy ability disagrees with its abilities",
                        ));
                    }
                    let primary_cost = first
                        .costs
                        .iter()
                        .find(|(pool, _)| pool.0 == definition.pool_id)
                        .map(|(_, amount)| *amount)
                        .unwrap_or(0);
                    if primary_cost != definition.cost {
                        return Err(serde::de::Error::custom(
                            "combat definition legacy cost disagrees with its abilities",
                        ));
                    }
                }
                for ability in &definition.abilities {
                    if !definition.attribute_names.contains(&ability.attribute) {
                        return Err(serde::de::Error::custom(
                            "combat ability names an undeclared checked attribute",
                        ));
                    }
                    if ability.costs.is_empty() {
                        return Err(serde::de::Error::custom("combat ability declares no costs"));
                    }
                    for (pool, amount) in &ability.costs {
                        let Some(template) =
                            definition.pools.iter().find(|entry| entry.id == pool.0)
                        else {
                            return Err(serde::de::Error::custom(
                                "combat ability costs draw outside its pools",
                            ));
                        };
                        if *amount == 0 || *amount > template.max {
                            return Err(serde::de::Error::custom(
                                "combat ability cost exceeds its pool maximum",
                            ));
                        }
                    }
                    if let crate::combat::AbilityDefense::TargetStat { stat } = &ability.defense {
                        if *stat == definition.health_stat
                            || !definition.attribute_names.contains(stat)
                        {
                            return Err(serde::de::Error::custom(
                                "combat ability names an undeclared defense stat",
                            ));
                        }
                    }
                    if let Some(effect) = ability.effect {
                        if !definition
                            .effects
                            .iter()
                            .any(|entry| entry.effect == effect)
                        {
                            return Err(serde::de::Error::custom(
                                "combat ability effect resolves outside its effects",
                            ));
                        }
                    }
                    if let Some(index) = ability.natural_die {
                        if index >= ability.dice.count() {
                            return Err(serde::de::Error::custom(
                                "combat ability natural selector names no drawn die",
                            ));
                        }
                    }
                }
                for effect in &definition.effects {
                    if effect.duration_rounds == 0
                        || effect.modifiers.is_empty()
                        || effect.mod_type.is_empty()
                    {
                        return Err(serde::de::Error::custom(
                            "combat effect carries an empty shape",
                        ));
                    }
                    for modifier in &effect.modifiers {
                        let nameless = modifier.name.is_none();
                        let is_set = matches!(modifier.op, crate::combat::EffectOp::Set);
                        if is_set
                            && matches!(
                                effect.policy,
                                crpg_rules::StackingPolicy::HighestBonusWorstPenalty
                            )
                        {
                            return Err(serde::de::Error::custom(
                                "combat effect carries an operation its policy rejects",
                            ));
                        }
                        if nameless
                            && matches!(
                                effect.policy,
                                crpg_rules::StackingPolicy::HighestPriorityPerName
                            )
                        {
                            return Err(serde::de::Error::custom(
                                "combat effect carries an operation its policy rejects",
                            ));
                        }
                    }
                }
                {
                    let mut ordered: Vec<crpg_core::Ulid> = Vec::new();
                    for effect in &definition.effects {
                        for modifier in &effect.modifiers {
                            ordered.push(modifier.id);
                        }
                    }
                    ordered.sort();
                    for pair in ordered.windows(2) {
                        if pair[0] == pair[1] {
                            return Err(serde::de::Error::custom(
                                "combat effects share a modifier identity",
                            ));
                        }
                    }
                }
                {
                    let mut by_type: BTreeMap<&str, crpg_rules::StackingPolicy> = BTreeMap::new();
                    for effect in &definition.effects {
                        match by_type.get(effect.mod_type.as_str()) {
                            None => {
                                by_type.insert(effect.mod_type.as_str(), effect.policy);
                            }
                            Some(kept) if *kept != effect.policy => {
                                return Err(serde::de::Error::custom(
                                    "combat effects disagree on one modifier type",
                                ));
                            }
                            Some(_) => {}
                        }
                    }
                }
                {
                    let mut placements = BTreeSet::new();
                    for (_, combatant) in repr.combatants.iter() {
                        if combatant.max_health() == 0 {
                            return Err(serde::de::Error::custom(
                                "combatant with a nonpositive maximum",
                            ));
                        }
                        if combatant.health() > combatant.max_health() {
                            return Err(serde::de::Error::custom(
                                "combatant health exceeds its maximum",
                            ));
                        }
                        if combatant.dead() != (combatant.health() == 0) {
                            return Err(serde::de::Error::custom(
                                "combatant health disagrees with its dead flag",
                            ));
                        }
                        if combatant.action_pool().id().0 != definition.pool_id {
                            return Err(serde::de::Error::custom(
                                "combatant pool identity disagrees with its definition",
                            ));
                        }
                        if combatant.action_pool().max() != definition.pool_max {
                            return Err(serde::de::Error::custom(
                                "combatant pool maximum disagrees with its definition",
                            ));
                        }
                        if combatant.action_pool().refresh_trigger() != definition.pool_refresh {
                            return Err(serde::de::Error::custom(
                                "combatant pool refresh disagrees with its definition",
                            ));
                        }
                        let names: Vec<&str> = combatant
                            .attributes()
                            .iter()
                            .map(|(name, _)| name.as_str())
                            .collect();
                        let declared: Vec<&str> = definition
                            .attribute_names
                            .iter()
                            .map(String::as_str)
                            .collect();
                        if names != declared {
                            return Err(serde::de::Error::custom(
                                "combatant attributes disagree with their definition",
                            ));
                        }
                        if combatant.extra_pools().len() + 1 != definition.pools.len() {
                            return Err(serde::de::Error::custom(
                                "combatant pool count disagrees with its definition",
                            ));
                        }
                        for (index, pool) in combatant.extra_pools().iter().enumerate() {
                            let template = &definition.pools[index + 1];
                            if pool.id().0 != template.id
                                || pool.max() != template.max
                                || pool.refresh_trigger() != template.refresh
                            {
                                return Err(serde::de::Error::custom(
                                    "combatant pool disagrees with its definition",
                                ));
                            }
                        }
                        for pair in combatant.attached().windows(2) {
                            if pair[0].effect >= pair[1].effect {
                                return Err(serde::de::Error::custom(
                                    "combatant attachments break ascending order",
                                ));
                            }
                        }
                        {
                            let mut seen = BTreeSet::new();
                            for attached in combatant.attached() {
                                if !seen.insert(attached.effect) {
                                    return Err(serde::de::Error::custom(
                                        "combatant attaches one effect twice",
                                    ));
                                }
                                if !definition
                                    .effects
                                    .iter()
                                    .any(|entry| entry.effect == attached.effect)
                                {
                                    return Err(serde::de::Error::custom(
                                        "combatant attachment resolves outside its effects",
                                    ));
                                }
                                if attached.expires_round <= state.round {
                                    return Err(serde::de::Error::custom(
                                        "combatant attachment carries no future expiry",
                                    ));
                                }
                            }
                        }
                        if !placements.insert(combatant.placement()) {
                            return Err(serde::de::Error::custom(
                                "combatants share a placement identity",
                            ));
                        }
                    }
                }
                for (_, id) in repr.timeline.iter() {
                    match repr.combatants.get(id) {
                        Some(combatant) if !combatant.dead() => {}
                        _ => {
                            return Err(serde::de::Error::custom(
                                "combat timeline schedules a non-combatant",
                            ));
                        }
                    }
                }
                if let Some(active) = state.active {
                    let head = repr.timeline.iter().next().map(|(_, id)| id);
                    let scheduled = matches!(
                        repr.combatants.get(active),
                        Some(combatant) if !combatant.dead()
                    );
                    if !scheduled || head != Some(active) {
                        return Err(serde::de::Error::custom(
                            "combat active turn is not the scheduled head",
                        ));
                    }
                }
            }
            None => {
                if !repr.combatants.is_empty() {
                    return Err(serde::de::Error::custom(
                        "combatant store without encounter state",
                    ));
                }
            }
        }
        Ok(Self {
            entities: repr.entities,
            transforms: repr.transforms,
            timeline: repr.timeline,
            events: repr.events,
            rng: repr.rng,
            tick: repr.tick,
            combatants: repr.combatants,
            combat: repr.combat,
            interners: repr.interners,
            area: repr.area,
        })
    }
}

impl World {
    /// A new, empty world: tick zero, RNG seeded from `seed`.
    ///
    /// Same seed in, same world out — the seed is the first input the
    /// determinism story depends on.
    pub fn new(seed: u64) -> Self {
        Self {
            entities: GenerationalArena::new(),
            transforms: ComponentStore::new(),
            timeline: Timeline::new(),
            events: EventQueue::new(),
            rng: DeterministicRng::from_seed(seed),
            tick: Tick::ZERO,
            combatants: ComponentStore::new(),
            combat: None,
            interners: Interners::new(),
            area: None,
        }
    }

    /// A new, empty world bound to the authored `area` (T027a, ADR-0020).
    ///
    /// Otherwise identical to [`new`](Self::new). The area is immutable: there
    /// is no setter and no late binding of an unbound world. Sim cannot check
    /// that `area` names an authored area; the host that loads content does.
    pub fn new_in_area(seed: u64, area: Ulid) -> Self {
        Self {
            area: Some(area),
            ..Self::new(seed)
        }
    }

    /// The authored area this world simulates, or `None` when unbound.
    pub fn area(&self) -> Option<Ulid> {
        self.area
    }

    /// The area `entity` is present in: this world's area iff the full,
    /// generation-bearing id is live here, otherwise `None`.
    ///
    /// Death is not despawn, so a dead, retained combatant is still present.
    /// Across worlds, identity is `(area, EntityId)`: two worlds can mint
    /// equal ids.
    pub fn area_of(&self, entity: EntityId) -> Option<Ulid> {
        if self.contains(entity) {
            self.area
        } else {
            None
        }
    }

    /// The metadata of one live entity. Crate-visible only: the area transfer
    /// copies it to the destination.
    pub(crate) fn entity_meta(&self, entity: EntityId) -> Option<EntityMeta> {
        self.entities.get(entity).copied()
    }

    /// Mints an entity holding `meta` and enqueues `Spawned` at the current
    /// tick.
    ///
    /// Identity comes from the arena untouched: generations from 1, the
    /// `u32::MAX` tombstone never issued, ascending-index iteration. See the
    /// `crpg-core` contract, which this method inherits rather than restates.
    pub fn spawn(&mut self, meta: EntityMeta) -> EntityId {
        let id = self.entities.insert(meta);
        self.events
            .push(self.tick, SimEvent::Spawned { entity: id });
        id
    }

    /// Removes `id`, strips its component and timeline entries, and enqueues
    /// `Despawned`.
    ///
    /// Returns `false` — and enqueues nothing — for a dead id. After a
    /// successful despawn no store key and no timeline entry addresses `id`;
    /// maintaining that is this method's whole job, because the stores
    /// themselves do no liveness checking. Combat cleanup (T016b, T016f)
    /// rides along: the combatant component is stripped with the rest;
    /// removing the active combatant advances (new head, rollover, or
    /// terminal) through the shared combat helper, removing any other
    /// leaves the turn, and removing the last releases the encounter
    /// (`combat` back to `None`). Despawning never emits `Died`.
    /// Interners, events, RNG, and tick are retained either way.
    pub fn despawn(&mut self, id: EntityId) -> bool {
        self.despawn_tracked(id).0
    }

    /// Removes `id` like [`despawn`](Self::despawn), reporting the turn or
    /// terminal transition the removal actually caused.
    ///
    /// Crate-visible only: [`despawn`](Self::despawn) delegates here and
    /// discards the facts, so legacy emissions, bytes and signatures are
    /// unchanged; the history wrapper consumes the facts to journal its
    /// trailing `TurnStarted` or `EncounterEnded` (T020, ADR-0017). The
    /// encounter and round behind an implicit last-participant release are
    /// captured before the removal; releasing an already terminal encounter
    /// reports no second end.
    pub(crate) fn despawn_tracked(&mut self, id: EntityId) -> (bool, TransitionFacts) {
        let prior = self
            .combat
            .as_ref()
            .map(|state| (state.definition.encounter, state.round, state.active));
        if self.entities.remove(id).is_none() {
            return (false, TransitionFacts::default());
        }
        self.transforms.remove(id);
        self.combatants.remove(id);
        self.timeline.remove(id);
        let mut facts = TransitionFacts::default();
        if self.combatants.is_empty() {
            self.combat = None;
            if let Some((encounter, round, active)) = prior {
                if active.is_some() {
                    facts.encounter_ended = Some((encounter, round));
                }
            }
        } else if self.combat.is_some()
            && self
                .combat
                .as_ref()
                .expect("combat presence is checked above")
                .active
                == Some(id)
        {
            facts = crate::combat::advance_tracked(&mut *self);
        }
        self.events
            .push(self.tick, SimEvent::Despawned { entity: id });
        (true, facts)
    }

    /// Whether `id` names a live entity.
    pub fn contains(&self, id: EntityId) -> bool {
        self.entities.contains(id)
    }

    /// The number of live entities.
    pub fn len(&self) -> usize {
        self.entities.len()
    }

    /// Whether no entities are live.
    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }

    /// Every live id, in ascending index order (inherited arena invariant).
    pub fn ids(&self) -> impl Iterator<Item = EntityId> + '_ {
        self.entities.ids()
    }

    /// The world's current tick. Getter only: advancing time — the tick
    /// loop, systems, event-graph runs — is T008.
    pub fn tick(&self) -> Tick {
        self.tick
    }

    /// Advances the tick counter by one, saturating at `u64::MAX`.
    ///
    /// Crate-visible only: the [`tick`](crate::tick) loop calls it; no outer
    /// code sets time, so there is deliberately no public setter (see the
    /// crate contract). Saturation is unreachable-by-construction — 2^64
    /// ticks is not a workload — exactly as the event `seq` counter in
    /// `crpg-core`, and for the same reason it saturates rather than wraps:
    /// wrapping would silently reorder time.
    pub(crate) fn advance_tick(&mut self) {
        self.tick = self.tick.saturating_add(1);
    }

    /// The transform store. No liveness check: check
    /// [`contains`](Self::contains) first, or keep the id you spawned.
    pub fn transforms(&self) -> &ComponentStore<Transform> {
        &self.transforms
    }

    /// Mutably, the transform store (same caveat as
    /// [`transforms`](Self::transforms)).
    pub fn transforms_mut(&mut self) -> &mut ComponentStore<Transform> {
        &mut self.transforms
    }

    /// The timeline container. Advance policy is T008's, not this type's.
    pub fn timeline(&self) -> &Timeline {
        &self.timeline
    }

    /// Mutably, the timeline container.
    pub fn timeline_mut(&mut self) -> &mut Timeline {
        &mut self.timeline
    }

    /// The live event queue. Draining it is T008's tick loop's job; reading
    /// it is anyone's.
    pub fn events(&self) -> &EventQueue<SimEvent> {
        &self.events
    }

    /// Mutably, the live event queue.
    pub fn events_mut(&mut self) -> &mut EventQueue<SimEvent> {
        &mut self.events
    }

    /// The world's deterministic RNG. Callers name their stream explicitly;
    /// streams they do not name they do not disturb.
    pub fn rng_mut(&mut self) -> &mut DeterministicRng {
        &mut self.rng
    }

    /// The authoritative combatant store (T016b, ADR-0013).
    ///
    /// Insertion order is authored participant order, giving the
    /// deterministic placement-to-entity mapping.
    pub fn combatants(&self) -> &ComponentStore<Combatant> {
        &self.combatants
    }

    /// Mutably, the combatant store. Crate-visible only: the combat
    /// controller owns combat mutation; outer code reads through
    /// [`combatants`](Self::combatants).
    pub(crate) fn combatants_mut(&mut self) -> &mut ComponentStore<Combatant> {
        &mut self.combatants
    }

    /// Encounter-level state, or `None` when no encounter is active.
    pub fn combat(&self) -> Option<&CombatState> {
        self.combat.as_ref()
    }

    /// Mutably, the encounter state. Crate-visible only, same owner as
    /// [`combatants_mut`](Self::combatants_mut).
    pub(crate) fn combat_mut(&mut self) -> &mut Option<CombatState> {
        &mut self.combat
    }

    /// The issuing interner (T016b, ADR-0013).
    ///
    /// The world interned the resolution roll tag here at encounter start;
    /// combat state resolves it by name on every use, so differently
    /// ordered interners preserve symbol meaning (E014).
    pub fn interners(&self) -> &Interners {
        &self.interners
    }

    /// Mutably, the issuing interner. Crate-visible only, same owner as
    /// [`combatants_mut`](Self::combatants_mut).
    pub(crate) fn interners_mut(&mut self) -> &mut Interners {
        &mut self.interners
    }
}

impl Default for World {
    /// `new(0)`: for tests and tooling that need a world but own no seed.
    fn default() -> Self {
        Self::new(0)
    }
}

//! The MVP decision: attack, walk toward, or end the turn (M-1..M-9).

use crpg_core::{EntityId, Fx16_16, Ulid};
use crpg_sim::{
    legal_actions, seg_len, snap, validate_action, validate_move, validate_route, within_range,
    AbilityDefinition, CombatAction, CombatError, Combatant, EncounterMode, LegalActionsError,
    World,
};

use crate::route::{host_route, measure_of, point_along, STANDOFF_MARGIN_RAW};
use crate::sides::Sides;

/// One next step for one actor (Readings 3, 10, 11).
///
/// The driver's mapping: `Act` goes to the action path; `Move` to `host_move`
/// (turn-based) or `move_to` (real time); `Halt` to `halt`; `Hold` to nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Perform this action; always an element of `legal_actions(world, actor)`.
    Act(CombatAction),
    /// Walk to `destination`, toward the hostile `target`.
    Move {
        /// The hostile the walk approaches.
        target: EntityId,
        /// The destination the host computes the route to.
        destination: [Fx16_16; 3],
    },
    /// Clear the actor's route (real time only).
    Halt,
    /// Do nothing now (real time only).
    Hold,
}

/// The MVP decision (M-1..M-9). Read-only and deterministic.
///
/// Legality is [`legal_actions`]' alone: [`LegalActionsError`] is returned
/// unchanged, and every returned [`Decision::Act`] is one of its options.
/// Movement and navigation failures are ordinary decisions, never errors
/// (Reading 12).
pub fn decide(
    world: &World,
    actor: EntityId,
    sides: &Sides,
    nav: Option<&crpg_nav::NavMesh>,
) -> Result<Decision, LegalActionsError> {
    // M-1: legality first; an error is returned unchanged.
    let listed = legal_actions(world, actor)?;
    let combat = world
        .combat()
        .expect("legal_actions validated the encounter");
    let real_time = combat.mode == EncounterMode::RealTime;
    let standing = world.combatants().get(actor).and_then(Combatant::order);
    let route = world.movers().get(actor).and_then(|mover| mover.route());
    let fallback = if real_time {
        if route.is_some() {
            Decision::Halt
        } else {
            Decision::Hold
        }
    } else {
        Decision::Act(CombatAction::EndTurn { actor })
    };
    let no_intent = if real_time {
        Decision::Hold
    } else {
        Decision::Act(CombatAction::EndTurn { actor })
    };

    // M-2: every live, hostile, non-self combatant, ascending `EntityId`.
    let mut hostiles: Vec<EntityId> = world
        .combatants()
        .iter()
        .filter(|(entity, state)| {
            *entity != actor && !state.dead() && sides.hostile(actor, *entity)
        })
        .map(|(entity, _)| entity)
        .collect();
    hostiles.sort_unstable();

    // M-3: no hostile.
    if hostiles.is_empty() {
        return Ok(if !real_time {
            Decision::Act(CombatAction::EndTurn { actor })
        } else if standing.is_some() {
            Decision::Act(CombatAction::Stop { actor })
        } else if route.is_some() {
            Decision::Halt
        } else {
            Decision::Hold
        });
    }

    // M-4: the least available attack on a hostile, by (distance, target, ability).
    let mut attack: Option<((u64, EntityId, Ulid), CombatAction)> = None;
    for option in &listed {
        let CombatAction::UseAbility {
            ability, target, ..
        } = *option
        else {
            continue;
        };
        if !hostiles.contains(&target) {
            continue;
        }
        let key = (distance(world, actor, target), target, ability);
        let better = match &attack {
            Some((best, _)) => key < *best,
            None => true,
        };
        if better {
            attack = Some((key, *option));
        }
    }
    if let Some((_, option)) = attack {
        if !real_time {
            return Ok(Decision::Act(option));
        }
        if route.is_some() {
            return Ok(Decision::Halt);
        }
        if let CombatAction::UseAbility {
            ability, target, ..
        } = option
        {
            if standing.is_some_and(|order| order.ability == ability && order.target == target) {
                return Ok(Decision::Hold);
            }
        }
        return Ok(Decision::Act(option));
    }

    // M-5: the approach target and the intended ability.
    let approach = hostiles
        .iter()
        .copied()
        .min_by_key(|entity| (distance(world, actor, *entity), *entity))
        .expect("hostiles is not empty");
    let mut abilities: Vec<&AbilityDefinition> = combat.definition.abilities.iter().collect();
    abilities.sort_by_key(|entry| entry.ability);
    let mut intended: Option<&AbilityDefinition> = None;
    for entry in abilities {
        let probe = CombatAction::UseAbility {
            actor,
            ability: entry.ability,
            target: approach,
        };
        if validate_action(world, &probe) == Err(CombatError::OutOfRange) {
            intended = Some(entry);
            break;
        }
    }
    let Some(entry) = intended else {
        return Ok(no_intent);
    };
    let Some(range) = entry.range else {
        return Ok(no_intent);
    };
    let range_raw = range.to_raw();
    if range_raw < 0 {
        return Ok(no_intent);
    }
    let Some(actor_mover) = world.movers().get(actor) else {
        return Ok(fallback);
    };
    let Some(target_mover) = world.movers().get(approach) else {
        return Ok(fallback);
    };
    let actor_pos = actor_mover.position();
    let target_pos = target_mover.position();
    let r = u64::try_from(range_raw).expect("a nonnegative range fits in u64");

    // M-6: real time, a route already ends in range, so hold before the mesh.
    if real_time {
        if let Some(active) = route {
            if let Some(last) = active.route().points.last() {
                if within_range(measure_of(world), range, *last, target_pos) {
                    return Ok(Decision::Hold);
                }
            }
        }
    }

    // M-7: the standoff destination, capped by what a turn can afford.
    let Some(nav) = nav else {
        return Ok(fallback);
    };
    let goal = nav.nearest_on_mesh(snap(measure_of(world), target_pos));
    let Ok(path) = nav.find_path(actor_pos, goal) else {
        return Ok(fallback);
    };
    let remaining = r.saturating_sub(STANDOFF_MARGIN_RAW);
    let mut d = path.length().saturating_sub(remaining);
    if !real_time {
        let Some(movement) = combat.definition.movement else {
            return Ok(fallback);
        };
        let Some(combatant) = world.combatants().get(actor) else {
            return Ok(fallback);
        };
        let balance = pool_balance(combatant, movement.pool);
        let reserve = reserved_cost(entry, movement.pool);
        let units = balance.saturating_sub(reserve) / movement.step_cost;
        let step_raw = u64::try_from(actor_mover.step().to_raw()).map_or(1, |raw| raw.max(1));
        d = d.min(u64::from(units) * step_raw);
    }
    if d == 0 {
        return Ok(fallback);
    }
    let Some(destination) = point_along(path.points(), d) else {
        return Ok(fallback);
    };

    // M-8: verify with the host's own route and the movement validation.
    let Ok(walk) = host_route(nav, world, actor, destination) else {
        return Ok(fallback);
    };
    if real_time {
        if validate_route(&walk, world.nav(), actor_pos).is_err() {
            return Ok(fallback);
        }
    } else if validate_move(world, actor, &walk).is_err() {
        return Ok(fallback);
    }
    Ok(Decision::Move {
        target: approach,
        destination,
    })
}

/// The U4 horizontal distance between two actors, `u64::MAX` without a mover
/// on both sides (Reading 5).
fn distance(world: &World, a: EntityId, b: EntityId) -> u64 {
    match (world.movers().get(a), world.movers().get(b)) {
        (Some(from), Some(to)) => seg_len(from.position(), to.position()),
        _ => u64::MAX,
    }
}

/// The balance of one template pool on one combatant, zero when it holds none.
///
/// The primary pool when its identity matches, else the matching extra pool,
/// exactly sim's crate-private reader.
fn pool_balance(combatant: &Combatant, pool: Ulid) -> u32 {
    let primary = combatant.action_pool();
    if primary.id().0 == pool {
        return primary.current();
    }
    combatant
        .extra_pools()
        .iter()
        .find(|entry| entry.id().0 == pool)
        .map_or(0, |entry| entry.current())
}

/// The ability's total cost in `pool`, saturating (Reading 8's reserve).
fn reserved_cost(entry: &AbilityDefinition, pool: Ulid) -> u32 {
    entry.costs.iter().fold(0_u32, |sum, &(id, amount)| {
        if id.0 == pool {
            sum.saturating_add(amount)
        } else {
            sum
        }
    })
}

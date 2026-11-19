//! The MVP AI driver (T153): `Host::drive_ai`.
//!
//! One trusted host operation the embedding calls between ticks. It decides
//! for the caller's AI-controlled combatants and turns each [`crpg_ai::Decision`]
//! into the host's own step — move by the host's route, attack, end the turn —
//! within an [`AiBudget`], reporting what it did in an [`AiReport`]. Refusals
//! are report steps; only a shut-down host is an `Err`.

use crpg_core::EntityId;
use crpg_sim::{CombatAction, EncounterMode};

use super::{Host, HostError, HOST_ROUTE_TAG};

// Reading 12, at compile time.
const _: () = assert!(crpg_ai::AI_ROUTE_TAG.0 == HOST_ROUTE_TAG.0);

/// How much one `drive_ai` call may do (Readings 5, 7, 8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AiBudget {
    /// Turn-based: `decide` calls allowed in one turn, counted across calls;
    /// at the count the driver ends the head's turn instead of deciding.
    pub decisions_per_turn: u32,
    /// Real time: the most controlled combatants one call decides for.
    pub actors_per_drive: u32,
}

/// The MVP budget: 16 decisions per turn, 8 creatures per call.
pub const MVP_AI_BUDGET: AiBudget = AiBudget {
    decisions_per_turn: 16,
    actors_per_drive: 8,
};

/// One thing a `drive_ai` call did for one actor (Readings 9, 10).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AiStep {
    /// The decision was applied by its host operation (`Hold`: by nothing).
    Applied {
        /// The actor that decided.
        actor: EntityId,
        /// The applied decision.
        decision: crpg_ai::Decision,
    },
    /// The host operation refused the decision; it changed nothing.
    Refused {
        /// The actor that decided.
        actor: EntityId,
        /// The refused decision.
        decision: crpg_ai::Decision,
        /// The operation's unchanged refusal.
        error: HostError,
    },
    /// `decide` returned an error, unchanged; nothing was applied.
    Undecided {
        /// The actor that could not be decided.
        actor: EntityId,
        /// The unchanged `decide` error.
        error: crpg_sim::LegalActionsError,
    },
}

/// What one `drive_ai` call did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiReport {
    /// In the order they happened.
    pub steps: Vec<AiStep>,
    /// A budget limited the call: turn-based, the turn cap was reached, so the
    /// call's one step is the forced `EndTurn`, applied or refused (D-5(1));
    /// real time, some candidates were left for a later call.
    pub capped: bool,
}

/// The current AI turn and its decision count (D-5). Crate-private.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AiTurn {
    /// The encounter round this count belongs to.
    pub(crate) round: u32,
    /// The head whose turn is being driven.
    pub(crate) head: EntityId,
    /// The `decide` calls made for that turn so far.
    pub(crate) decisions: u32,
}

impl Host {
    /// One AI drive (D-1..D-12): `Closed` first; otherwise always `Ok`.
    pub fn drive_ai(
        &mut self,
        sides: &crpg_ai::Sides,
        actors: &[EntityId],
        budget: AiBudget,
    ) -> Result<AiReport, HostError> {
        // D-1: a shut-down host refuses first.
        if self.closed {
            return Err(HostError::Closed);
        }

        // D-3: the controlled set, ascending with duplicates removed.
        let mut ctl: Vec<EntityId> = actors.to_vec();
        ctl.sort_unstable();
        ctl.dedup();

        // Read the encounter's facts up front so the world borrow ends before
        // the mutating steps.
        enum Plan {
            TurnBased { head: EntityId, round: u32 },
            RealTime { candidates: Vec<EntityId> },
        }
        let plan = {
            let world = self.history.world();
            let Some(combat) = world.combat() else {
                // D-2: no fight.
                return Ok(AiReport {
                    steps: Vec::new(),
                    capped: false,
                });
            };
            match combat.mode {
                EncounterMode::TurnBased => {
                    // D-4: no head, or an uncontrolled head, does nothing.
                    let Some(head) = combat.active else {
                        return Ok(AiReport {
                            steps: Vec::new(),
                            capped: false,
                        });
                    };
                    if !ctl.contains(&head) {
                        return Ok(AiReport {
                            steps: Vec::new(),
                            capped: false,
                        });
                    }
                    Plan::TurnBased {
                        head,
                        round: combat.round,
                    }
                }
                EncounterMode::RealTime => {
                    // D-6: the `awaiting_orders` fight gate.
                    let Some(clock) = combat.clock else {
                        return Ok(AiReport {
                            steps: Vec::new(),
                            capped: false,
                        });
                    };
                    let alive = world
                        .combatants()
                        .iter()
                        .filter(|(_, state)| !state.dead())
                        .count();
                    if alive < 2 || clock.elapsed >= crpg_sim::RT_CLOCK_MAX {
                        return Ok(AiReport {
                            steps: Vec::new(),
                            capped: false,
                        });
                    }
                    let candidates = ctl
                        .into_iter()
                        .filter(|entity| {
                            world
                                .combatants()
                                .get(*entity)
                                .is_some_and(|state| !state.dead())
                        })
                        .collect();
                    Plan::RealTime { candidates }
                }
            }
        };

        match plan {
            Plan::TurnBased { head, round } => self.drive_turn_based(head, round, sides, budget),
            Plan::RealTime { candidates } => self.drive_real_time(candidates, sides, budget),
        }
    }

    /// D-5: one turn-based step for the head, capped by the turn budget.
    fn drive_turn_based(
        &mut self,
        head: EntityId,
        round: u32,
        sides: &crpg_ai::Sides,
        budget: AiBudget,
    ) -> Result<AiReport, HostError> {
        // (Re)start the count when the turn changed.
        let current = match self.ai_turn {
            Some(turn) if turn.round == round && turn.head == head => turn,
            _ => AiTurn {
                round,
                head,
                decisions: 0,
            },
        };

        // D-5(1): at the cap, end the head's turn instead of deciding; the
        // counter is not incremented.
        if current.decisions >= budget.decisions_per_turn {
            self.ai_turn = Some(current);
            let decision = crpg_ai::Decision::Act(CombatAction::EndTurn { actor: head });
            let step = self.apply(head, decision, false);
            return Ok(AiReport {
                steps: vec![step],
                capped: true,
            });
        }

        // D-5(2): count the decision (saturating), then decide.
        self.ai_turn = Some(AiTurn {
            round,
            head,
            decisions: current.decisions.saturating_add(1),
        });
        let decided = {
            let world = self.history.world();
            let mesh = self.nav.as_ref().map(|nav| &nav.mesh);
            crpg_ai::decide(world, head, sides, mesh)
        };
        let step = match decided {
            Err(error) => AiStep::Undecided { actor: head, error },
            Ok(decision) => self.apply(head, decision, false),
        };
        Ok(AiReport {
            steps: vec![step],
            capped: false,
        })
    }

    /// D-6: real-time decisions for the live controlled candidates, rotated by
    /// the world tick when they exceed the per-call budget.
    fn drive_real_time(
        &mut self,
        candidates: Vec<EntityId>,
        sides: &crpg_ai::Sides,
        budget: AiBudget,
    ) -> Result<AiReport, HostError> {
        let n = candidates.len();
        let k = budget.actors_per_drive as usize;
        let mut capped = false;
        let order: Vec<EntityId> = if n <= k {
            candidates
        } else {
            capped = true;
            let start = (self.server_tick() as usize) % n;
            (0..k).map(|i| candidates[(start + i) % n]).collect()
        };

        let mut steps = Vec::with_capacity(order.len());
        for actor in order {
            let decided = {
                let world = self.history.world();
                let mesh = self.nav.as_ref().map(|nav| &nav.mesh);
                crpg_ai::decide(world, actor, sides, mesh)
            };
            let step = match decided {
                Err(error) => AiStep::Undecided { actor, error },
                Ok(decision) => self.apply(actor, decision, true),
            };
            steps.push(step);
        }
        Ok(AiReport { steps, capped })
    }

    /// D-7: maps one decision to its host operation, applied or refused.
    fn apply(&mut self, actor: EntityId, decision: crpg_ai::Decision, real_time: bool) -> AiStep {
        use crpg_ai::Decision;
        let result = match decision {
            Decision::Act(action) => self.host_action(&action).map(|_| ()),
            Decision::Move { destination, .. } => {
                if real_time {
                    self.move_to(actor, destination).map(|_| ())
                } else {
                    self.host_move(actor, destination).map(|_| ())
                }
            }
            Decision::Halt => self.halt(actor).map(|_| ()),
            Decision::Hold => return AiStep::Applied { actor, decision },
        };
        match result {
            Ok(()) => AiStep::Applied { actor, decision },
            Err(error) => AiStep::Refused {
                actor,
                decision,
                error,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use crpg_core::{EntityId, Fx16_16};
    use crpg_nav::{bake_box, NavRect};
    use crpg_sim::{HistoryWorld, MoveError, MoveRoute, NavRevision, RouteTag};
    use serde_json::{json, Value};

    use crate::host::HOST_ROUTE_TAG;
    use crate::nav::{plan_route, HostNav, RouteRefusal};

    use super::{AiBudget, MVP_AI_BUDGET};

    const FIXTURE: &str = include_str!("../../tests/fixtures/three_combatants.json");

    fn eid(index: u32) -> EntityId {
        serde_json::from_value(json!({"index": index, "generation": 1})).expect("valid entity id")
    }
    fn a() -> EntityId {
        eid(0)
    }
    fn c() -> EntityId {
        eid(2)
    }
    fn e7() -> EntityId {
        eid(7)
    }

    /// A whole-metre floor point `[x, 0, z]`.
    fn m(x: i32, z: i32) -> [Fx16_16; 3] {
        [Fx16_16::from_int(x), Fx16_16::ZERO, Fx16_16::from_int(z)]
    }

    fn rect(x0: i32, z0: i32, x1: i32, z1: i32) -> NavRect {
        NavRect {
            min_x: Fx16_16::from_int(x0),
            min_z: Fx16_16::from_int(z0),
            max_x: Fx16_16::from_int(x1),
            max_z: Fx16_16::from_int(z1),
        }
    }

    fn m10() -> crpg_nav::NavMesh {
        bake_box(Fx16_16::ZERO, rect(0, 0, 10, 10), &[]).expect("bakes")
    }
    fn mh() -> crpg_nav::NavMesh {
        bake_box(Fx16_16::ZERO, rect(0, 0, 10, 10), &[rect(4, 0, 6, 8)]).expect("bakes")
    }

    fn r(mesh: &crpg_nav::NavMesh) -> NavRevision {
        NavRevision::of_bytes(&mesh.to_canonical_bytes())
    }

    fn tb3_value() -> Value {
        serde_json::from_str(FIXTURE).expect("fixture parses")
    }

    /// The `movement_edits` of `TB3` (T037a): three movers at 1 m/tick with a
    /// 10 m step, their mirrors, and the definition's one-`JY`-per-10 m rules.
    fn movement_edits(mut value: Value) -> Value {
        value["world"]["movers"] = json!([
            [{"index": 0, "generation": 1},
             {"position": [65536, 0, 65536], "speed": 65536, "step": 655360}],
            [{"index": 1, "generation": 1},
             {"position": [196608, 0, 65536], "speed": 65536, "step": 655360}],
            [{"index": 2, "generation": 1},
             {"position": [65536, 0, 327680], "speed": 65536, "step": 655360}]
        ]);
        value["world"]["transforms"] = json!([
            [{"index": 0, "generation": 1},
             {"position": [1.0, 0.0, 1.0], "velocity": [0.0, 0.0, 0.0]}],
            [{"index": 1, "generation": 1},
             {"position": [3.0, 0.0, 1.0], "velocity": [0.0, 0.0, 0.0]}],
            [{"index": 2, "generation": 1},
             {"position": [1.0, 0.0, 5.0], "velocity": [0.0, 0.0, 0.0]}]
        ]);
        value["world"]["combat"]["definition"]["movement"] = json!({
            "measure": "euclidean",
            "pool": "000000000000000000000000JY",
            "step_cost": 1
        });
        value
    }

    /// `movement_edits(TB3)` without C's mover and transform.
    fn movement_edits_no_c() -> Value {
        let mut value = movement_edits(tb3_value());
        for key in ["movers", "transforms"] {
            let pairs = value["world"][key].as_array_mut().expect("pair list");
            pairs.retain(|pair| pair[0]["index"].as_u64() != Some(2));
        }
        value
    }

    fn world_of(value: &Value) -> HistoryWorld {
        let mut history: HistoryWorld =
            serde_json::from_value(value.clone()).expect("fixture loads");
        history
            .acknowledge(history.last_sequence())
            .expect("acknowledges its own range");
        history
    }

    /// D-12: `crpg_ai::host_route`'s refusal mapped to `plan_route`'s, field
    /// for field; `NavUnbound` cannot occur on a bound world.
    fn as_refusal(
        result: Result<MoveRoute, crpg_ai::RouteError>,
    ) -> Result<MoveRoute, RouteRefusal> {
        result.map_err(|error| match error {
            crpg_ai::RouteError::NavUnbound => unreachable!("the test binds the mesh"),
            crpg_ai::RouteError::AbsentEntity { entity } => {
                RouteRefusal::Move(MoveError::AbsentEntity { entity })
            }
            crpg_ai::RouteError::NotMover { entity } => {
                RouteRefusal::Move(MoveError::NotMover { entity })
            }
            crpg_ai::RouteError::Nav(error) => RouteRefusal::Nav(error),
        })
    }

    /// U1: every AI route equals the host's, revision, tag, points and
    /// refusals included.
    #[test]
    fn host_route_equals_plan_route() {
        let mut history = world_of(&movement_edits(tb3_value()));
        history.bind_nav(r(&m10())).expect("binds");
        let world = history.world();
        let nav = HostNav {
            mesh: m10(),
            revision: r(&m10()),
        };

        let parity = |actor: EntityId, destination: [Fx16_16; 3]| {
            assert_eq!(
                as_refusal(crpg_ai::host_route(&nav.mesh, world, actor, destination)),
                plan_route(Some(&nav), world, actor, destination, HOST_ROUTE_TAG)
            );
        };

        // (A, m(4, 1)): a straight two-point route.
        parity(a(), m(4, 1));
        assert_eq!(
            plan_route(Some(&nav), world, a(), m(4, 1), HOST_ROUTE_TAG),
            Ok(MoveRoute {
                revision: r(&m10()),
                tag: RouteTag(0),
                points: vec![m(1, 1), m(4, 1)],
            })
        );

        // (A, m(12, 1)): the goal snaps to the nearest mesh point.
        parity(a(), m(12, 1));
        assert_eq!(
            plan_route(Some(&nav), world, a(), m(12, 1), HOST_ROUTE_TAG),
            Ok(MoveRoute {
                revision: r(&m10()),
                tag: RouteTag(0),
                points: vec![m(1, 1), m(10, 1)],
            })
        );

        // (A, m(1, 1)): already there.
        parity(a(), m(1, 1));

        // (e7, m(4, 1)): the absent entity refusal.
        parity(e7(), m(4, 1));

        // (C, m(4, 1)) on a world without C's mover: the not-mover refusal.
        let mut no_c = world_of(&movement_edits_no_c());
        no_c.bind_nav(r(&m10())).expect("binds");
        let no_c_world = no_c.world();
        assert_eq!(
            as_refusal(crpg_ai::host_route(&nav.mesh, no_c_world, c(), m(4, 1))),
            plan_route(Some(&nav), no_c_world, c(), m(4, 1), HOST_ROUTE_TAG)
        );

        // Rebound to MH: the wall bends the route into at least three points.
        history.bind_nav(r(&mh())).expect("rebinds");
        let wall_nav = HostNav {
            mesh: mh(),
            revision: r(&mh()),
        };
        let world = history.world();
        assert_eq!(
            as_refusal(crpg_ai::host_route(&wall_nav.mesh, world, a(), m(9, 1))),
            plan_route(Some(&wall_nav), world, a(), m(9, 1), HOST_ROUTE_TAG)
        );
        let bent = plan_route(Some(&wall_nav), world, a(), m(9, 1), HOST_ROUTE_TAG)
            .expect("a path around the wall");
        assert!(bent.points.len() >= 3);
    }

    /// U2: the route tags agree at `RouteTag(0)` and the MVP budget is 16/8.
    #[test]
    fn route_tags_agree() {
        assert_eq!(crpg_ai::AI_ROUTE_TAG, HOST_ROUTE_TAG);
        assert_eq!(HOST_ROUTE_TAG, RouteTag(0));
        assert_eq!(
            MVP_AI_BUDGET,
            AiBudget {
                decisions_per_turn: 16,
                actors_per_drive: 8,
            }
        );
    }
}

//! The client half: apply delivered deltas, build the immutable view, submit
//! one intent (ADR-0033 §6, §8, §9 and §11).
//!
//! This module decides no rule. It reads frame bytes, asks the T046 replica to
//! apply them, and formats the resulting state for GDScript. Options arrive
//! from the authority's `legal_intents`; nothing here invents one.

use std::collections::BTreeMap;
use std::rc::Rc;

use crpg_core::Ulid;
use crpg_net::codec_v2::{decode_delta, encode_intent};
use crpg_net::protocol::{
    DeltaOp as LegacyOp, IntentBody, IntentFrame, NetId, ReceiptStatus, SessionEpoch, LANE_COMBAT,
    SEQ_FIRST,
};
use crpg_net::protocol_v2::DeltaOp;
use crpg_net::replica::ReplicaWorld;
use crpg_net::transport::Transport;

use super::errors::{BridgeCode, Fault};

/// One immutable snapshot of the session, safe to hand to GDScript.
///
/// Every slice has `net_ids().len()` entries and is in ascending `NetId`
/// order. The default value is the empty revision-0 view.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SessionView {
    revision: i64,
    net_ids: Vec<i64>,
    names: Vec<String>,
    sides: Vec<String>,
    placements: Vec<String>,
    health: Vec<i64>,
    max_health: Vec<i64>,
    health_known: Vec<u8>,
    dead: Vec<u8>,
    active: i64,
    round: i64,
    ended: bool,
    option_labels: Vec<String>,
    option_targets: Vec<i64>,
    events: Vec<ViewEvent>,
    receipts: Vec<ViewReceipt>,
}

impl SessionView {
    /// The revision this snapshot belongs to; starts at 0.
    pub fn revision(&self) -> i64 {
        self.revision
    }

    /// The replica ids of the live rows, ascending.
    pub fn net_ids(&self) -> &[i64] {
        &self.net_ids
    }

    /// The presentation name of every row (empty when unknown).
    pub fn names(&self) -> &[String] {
        &self.names
    }

    /// The presentation side of every row (empty when unknown).
    pub fn sides(&self) -> &[String] {
        &self.sides
    }

    /// The placement ULID text of every row (empty when unknown).
    pub fn placements(&self) -> &[String] {
        &self.placements
    }

    /// Current health of every row, or 0 while unknown.
    pub fn health(&self) -> &[i64] {
        &self.health
    }

    /// Maximum health of every row, or 0 while unknown.
    pub fn max_health(&self) -> &[i64] {
        &self.max_health
    }

    /// `1` when the row's health is known, else `0`.
    pub fn health_known(&self) -> &[u8] {
        &self.health_known
    }

    /// `1` when the last health fact said the row is dead, else `0`.
    pub fn dead(&self) -> &[u8] {
        &self.dead
    }

    /// The live active actor id, or 0 when there is none.
    pub fn active(&self) -> i64 {
        self.active
    }

    /// The current round, or 0 before the first turn fact.
    pub fn round(&self) -> i64 {
        self.round
    }

    /// Whether an `EncounterEnded` op has been seen in this session.
    pub fn ended(&self) -> bool {
        self.ended
    }

    /// One label per offered option, in host order.
    pub fn option_labels(&self) -> &[String] {
        &self.option_labels
    }

    /// One target id per offered option: the target, or 0 for `End turn`.
    pub fn option_targets(&self) -> &[i64] {
        &self.option_targets
    }

    /// The events collected by the step that produced this revision.
    pub fn events(&self) -> &[ViewEvent] {
        &self.events
    }

    /// The receipts collected by the step that produced this revision.
    pub fn receipts(&self) -> &[ViewReceipt] {
        &self.receipts
    }
}

/// One player-visible event, oldest first inside a view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViewEvent {
    /// One disclosed ability resolution.
    ActionResolved {
        /// Acting replica id.
        actor: i64,
        /// Targeted replica id.
        target: i64,
        /// Displayed ability name (the table entry, else the ULID text).
        ability: String,
        /// The authored ability ULID text.
        ability_id: String,
        /// The symbolic outcome.
        outcome: String,
        /// The reported damage.
        damage: i64,
    },
    /// One logical turn start.
    TurnStarted {
        /// Incoming active replica id.
        actor: i64,
        /// Combat round.
        round: i64,
    },
    /// One disclosed death notice.
    Died {
        /// The replica id that died.
        entity: i64,
    },
    /// The encounter reached its terminal state.
    EncounterEnded {
        /// The encounter ULID text.
        encounter: String,
        /// Closing combat round.
        round: i64,
    },
}

impl ViewEvent {
    /// The entry keys GDScript reads, `"kind"` first.
    pub fn entries(&self) -> Vec<(&'static str, ViewValue<'_>)> {
        match self {
            Self::ActionResolved {
                actor,
                target,
                ability,
                ability_id,
                outcome,
                damage,
            } => vec![
                ("kind", ViewValue::Text("action_resolved")),
                ("actor", ViewValue::Int(*actor)),
                ("target", ViewValue::Int(*target)),
                ("ability", ViewValue::Text(ability.as_str())),
                ("ability_id", ViewValue::Text(ability_id.as_str())),
                ("outcome", ViewValue::Text(outcome.as_str())),
                ("damage", ViewValue::Int(*damage)),
            ],
            Self::TurnStarted { actor, round } => vec![
                ("kind", ViewValue::Text("turn_started")),
                ("actor", ViewValue::Int(*actor)),
                ("round", ViewValue::Int(*round)),
            ],
            Self::Died { entity } => vec![
                ("kind", ViewValue::Text("died")),
                ("entity", ViewValue::Int(*entity)),
            ],
            Self::EncounterEnded { encounter, round } => vec![
                ("kind", ViewValue::Text("encounter_ended")),
                ("encounter", ViewValue::Text(encounter.as_str())),
                ("round", ViewValue::Int(*round)),
            ],
        }
    }
}

/// One receipt collected by the step that produced a revision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewReceipt {
    /// The command sequence the receipt finalizes.
    pub seq: i64,
    /// Whether the command applied.
    pub applied: bool,
    /// `0` for applied, else the frozen rejection code.
    pub code: i64,
}

impl ViewReceipt {
    /// The entry keys GDScript reads, in order.
    pub fn entries(&self) -> Vec<(&'static str, ViewValue<'_>)> {
        vec![
            ("seq", ViewValue::Int(self.seq)),
            ("applied", ViewValue::Bool(self.applied)),
            ("code", ViewValue::Int(self.code)),
        ]
    }
}

/// One typed value in an [`entries`](ViewEvent::entries) list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewValue<'a> {
    /// An integer.
    Int(i64),
    /// A borrowed string.
    Text(&'a str),
    /// A boolean.
    Bool(bool),
}

/// One option the authority offered the player.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Offered {
    /// The acting replica entity.
    pub(crate) actor: NetId,
    /// The typed body to encode on choice.
    pub(crate) body: IntentBody,
}

/// The authority's per-poll report: options plus an optional refusal text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StepReport {
    /// The legal options, in host order.
    pub(crate) options: Vec<Offered>,
    /// `Some(text)` when the host refused or dropped a player frame.
    pub(crate) refused: Option<String>,
}

/// Presentation data for one replica row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PresentationRow {
    /// The row's placement ULID.
    pub(crate) placement: Ulid,
    /// The display name.
    pub(crate) name: String,
    /// The display side.
    pub(crate) side: String,
}

/// Presentation tables keyed by the player's replica ids.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct PresentationTable {
    /// One row per replica id.
    pub(crate) rows: BTreeMap<NetId, PresentationRow>,
    /// Ability display names by authored ULID.
    pub(crate) abilities: BTreeMap<Ulid, String>,
}

/// The engine-neutral client half of one session.
pub(crate) struct Client {
    replica: ReplicaWorld,
    table: PresentationTable,
    view: Rc<SessionView>,
    options: Vec<Offered>,
    pending: Option<u64>,
    next_seq: u64,
    ended: bool,
}

impl Client {
    /// Starts an empty client half for `epoch` with the player's tables.
    pub(crate) fn new(epoch: SessionEpoch, table: PresentationTable) -> Self {
        Self {
            replica: ReplicaWorld::new(epoch),
            table,
            view: Rc::new(SessionView::default()),
            options: Vec::new(),
            pending: None,
            next_seq: SEQ_FIRST,
            ended: false,
        }
    }

    /// The current immutable snapshot.
    pub(crate) fn view(&self) -> Rc<SessionView> {
        Rc::clone(&self.view)
    }

    /// Drains delivered frames, then rebuilds the view at most once.
    ///
    /// `options` is the authority's host-ordered offer list. `refused` is true
    /// when this poll reported a dropped or rejected player frame, which
    /// clears the outstanding intent. A decode or replica refusal is fatal
    /// (`52` / `53`) and leaves the view at its previous revision.
    pub(crate) fn step(
        &mut self,
        link: &mut impl Transport,
        options: &[Offered],
        refused: bool,
    ) -> Result<(), Fault> {
        let mut events = Vec::new();
        let mut receipts = Vec::new();
        while let Some(bytes) = link.recv() {
            let frame = decode_delta(&bytes).map_err(|error| Fault {
                code: BridgeCode::FrameUndecodable,
                detail: error.to_string(),
            })?;
            self.replica.apply_delta(&frame).map_err(|error| Fault {
                code: BridgeCode::ReplicaRejected,
                detail: error.to_string(),
            })?;
            for op in &frame.ops {
                match op {
                    DeltaOp::ActionResolved {
                        actor,
                        target,
                        ability,
                        outcome,
                        damage,
                    } => {
                        let ability_id = ability.to_string();
                        let ability = self
                            .table
                            .abilities
                            .get(ability)
                            .cloned()
                            .unwrap_or_else(|| ability_id.clone());
                        events.push(ViewEvent::ActionResolved {
                            actor: wide(actor.get()),
                            target: wide(target.get()),
                            ability,
                            ability_id,
                            outcome: outcome.clone(),
                            damage: i64::from(*damage),
                        });
                    }
                    DeltaOp::TurnStarted { actor, round } => {
                        events.push(ViewEvent::TurnStarted {
                            actor: wide(actor.get()),
                            round: wide(*round),
                        });
                    }
                    DeltaOp::EncounterEnded { encounter, round } => {
                        self.ended = true;
                        events.push(ViewEvent::EncounterEnded {
                            encounter: encounter.to_string(),
                            round: wide(*round),
                        });
                    }
                    DeltaOp::Legacy(LegacyOp::Died { entity }) => {
                        events.push(ViewEvent::Died {
                            entity: wide(entity.get()),
                        });
                    }
                    DeltaOp::Legacy(LegacyOp::Receipt { seq, status, .. }) => {
                        let applied = matches!(status, ReceiptStatus::Applied);
                        let code = match status {
                            ReceiptStatus::Applied => 0,
                            ReceiptStatus::Rejected(code) => i64::from(code.as_u8()),
                        };
                        receipts.push(ViewReceipt {
                            seq: wide(*seq),
                            applied,
                            code,
                        });
                        if self.pending == Some(*seq) {
                            self.pending = None;
                        }
                    }
                    _ => {}
                }
            }
        }
        if refused {
            self.pending = None;
        }

        let net_ids: Vec<i64> = self
            .replica
            .entities()
            .iter()
            .map(|entity| wide(entity.net_id.get()))
            .collect();
        let mut names = Vec::with_capacity(net_ids.len());
        let mut sides = Vec::with_capacity(net_ids.len());
        let mut placements = Vec::with_capacity(net_ids.len());
        let mut health = Vec::with_capacity(net_ids.len());
        let mut max_health = Vec::with_capacity(net_ids.len());
        let mut health_known = Vec::with_capacity(net_ids.len());
        let mut dead = Vec::with_capacity(net_ids.len());
        for entity in self.replica.entities() {
            let row = self.table.rows.get(&entity.net_id);
            names.push(row.map(|row| row.name.clone()).unwrap_or_default());
            sides.push(row.map(|row| row.side.clone()).unwrap_or_default());
            placements.push(row.map(|row| row.placement.to_string()).unwrap_or_default());
            match &entity.health {
                Some(fact) => {
                    health.push(i64::from(fact.health));
                    max_health.push(i64::from(fact.max_health));
                    health_known.push(1);
                    dead.push(u8::from(fact.dead));
                }
                None => {
                    health.push(0);
                    max_health.push(0);
                    health_known.push(0);
                    dead.push(0);
                }
            }
        }
        let active = self
            .replica
            .turn()
            .and_then(|turn| turn.active)
            .filter(|active| self.replica.entity(*active).is_some())
            .map(|active| wide(active.get()))
            .unwrap_or(0);
        let round = self
            .replica
            .turn()
            .map(|turn| wide(turn.round))
            .unwrap_or(0);
        let (option_labels, option_targets) = self.labels(options);

        let unchanged = net_ids == self.view.net_ids
            && names == self.view.names
            && sides == self.view.sides
            && placements == self.view.placements
            && health == self.view.health
            && max_health == self.view.max_health
            && health_known == self.view.health_known
            && dead == self.view.dead
            && active == self.view.active
            && round == self.view.round
            && self.ended == self.view.ended
            && option_labels == self.view.option_labels
            && option_targets == self.view.option_targets;
        if events.is_empty() && receipts.is_empty() && unchanged {
            self.options = options.to_vec();
            return Ok(());
        }

        self.view = Rc::new(SessionView {
            revision: self.view.revision.saturating_add(1),
            net_ids,
            names,
            sides,
            placements,
            health,
            max_health,
            health_known,
            dead,
            active,
            round,
            ended: self.ended,
            option_labels,
            option_targets,
            events,
            receipts,
        });
        self.options = options.to_vec();
        Ok(())
    }

    /// Validates and encodes one chosen option, then sends it upstream.
    ///
    /// The checks are the client-owned prefix of B14: negative arguments
    /// (`3`), stale revision (`4`), out-of-range index (`6`), outstanding
    /// intent (`5`). `BridgeCore` checks `Fenced`/`Closed` and the negative
    /// arguments before this call so the contract's full order holds. A full
    /// link is `30`; only the successful branch advances the sequence and
    /// marks the intent pending.
    pub(crate) fn submit(
        &mut self,
        revision: i64,
        index: i64,
        link: &mut impl Transport,
    ) -> Result<(), Fault> {
        if revision < 0 || index < 0 {
            return Err(fault(BridgeCode::BadArgument, "negative argument"));
        }
        if revision != self.view.revision {
            return Err(fault(
                BridgeCode::StaleOptions,
                "revision is not the current view",
            ));
        }
        let Ok(index) = usize::try_from(index) else {
            return Err(fault(BridgeCode::NoSuchOption, "index is out of range"));
        };
        let Some(offered) = self.options.get(index) else {
            return Err(fault(BridgeCode::NoSuchOption, "index is out of range"));
        };
        if self.pending.is_some() {
            return Err(fault(BridgeCode::IntentPending, "an intent is outstanding"));
        }
        let frame = IntentFrame {
            epoch: self.replica.epoch().0,
            lane: LANE_COMBAT,
            seq: self.next_seq,
            observed_tick: self.replica.server_tick(),
            actor: offered.actor,
            body: offered.body,
        };
        let bytes = encode_intent(&frame).map_err(|error| Fault {
            code: BridgeCode::BadArgument,
            detail: error.to_string(),
        })?;
        link.send(&bytes)
            .map_err(|_| fault(BridgeCode::IntentRefused, "link full"))?;
        self.pending = Some(self.next_seq);
        self.next_seq = self.next_seq.saturating_add(1);
        Ok(())
    }

    /// Builds the host-ordered labels and targets for `options`.
    fn labels(&self, options: &[Offered]) -> (Vec<String>, Vec<i64>) {
        let mut labels = Vec::with_capacity(options.len());
        let mut targets = Vec::with_capacity(options.len());
        for offered in options {
            match offered.body {
                IntentBody::EndTurn => {
                    labels.push("End turn".to_string());
                    targets.push(0);
                }
                IntentBody::DeclareAction { ability, target } => {
                    let name = self
                        .table
                        .abilities
                        .get(&ability)
                        .cloned()
                        .unwrap_or_else(|| ability.to_string());
                    let label = match self.table.rows.get(&target) {
                        Some(row) => format!("{name} -> {}", row.name),
                        None => name,
                    };
                    labels.push(label);
                    targets.push(wide(target.get()));
                }
            }
        }
        (labels, targets)
    }
}

/// Widens an unsigned wire value without panicking.
fn wide(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

/// Builds a fault with a static summary.
fn fault(code: BridgeCode, detail: &str) -> Fault {
    Fault {
        code,
        detail: detail.to_string(),
    }
}

#[cfg(test)]
mod tests;

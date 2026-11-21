//! The protocol v4 client half: pool, replica, lane-0 intent state and the
//! lane-1 outbox (T151a).
//!
//! This module decides no rule. It routes delivered frames by their first
//! byte (`0x82` transfer, `0xA1` lane 1, anything else a v4 delta), answers
//! each `Published` transfer, and formats the resulting state for GDScript.
//! Positions, `Moved` events and move receipts extend the v2 view columns;
//! every other column is built exactly as the v2 client builds it.
//!
//! T151b wires this half into `BridgeCore`.
use std::rc::Rc;

use crpg_net::codec_v4::encode_intent as encode_lane0;
use crpg_net::movement_v4::{
    decode_down, encode_intent as encode_move, MoveBody, MoveDown, MoveOutcome, MoveReceipt,
    MOVEMENT_VERSION,
};
use crpg_net::outbox_v4::MoveOutbox;
use crpg_net::pool_v4::{
    GateMode, ReassemblyPoolV4, ReceiverIdV4, ReplicaUpdateV4, TransferError, TransferLimitsV3,
    TransferOutcome,
};
use crpg_net::protocol::{IntentBody, NetId, SessionEpoch, LANE_COMBAT, SEQ_FIRST};
use crpg_net::protocol_v3::{DeltaOp as DeltaOpV3, ReceiptStatus};
use crpg_net::protocol_v4::{DeltaOp, IntentBody as IntentBodyV4, IntentFrame, Point};
use crpg_net::replica_v4::ReplicaWorldV4;
use crpg_net::transfer_v3::{encode_up, TransferUp, TRANSFER_VERSION_V3};
use crpg_net::transport::Transport;

use super::client::{
    fault, option_labels, wide, Offered, PresentationTable, SessionView, ViewEvent,
    ViewMoveReceipt, ViewReceipt,
};
use super::errors::{BridgeCode, Fault};

/// The v4 client half: pool, replica, lane-0 intent state and lane-1 outbox.
pub(crate) struct ClientV4 {
    pool: ReassemblyPoolV4,
    receiver: ReceiverIdV4,
    replica: ReplicaWorldV4,
    outbox: MoveOutbox,
    table: PresentationTable,
    view: Rc<SessionView>,
    options: Vec<Offered>,
    pending: Option<u64>,
    next_seq: u64,
    ended: bool,
}

impl ClientV4 {
    /// Starts an empty v4 client half for `epoch` with the player's tables.
    pub(crate) fn new(epoch: SessionEpoch, table: PresentationTable) -> Result<Self, Fault> {
        let mut pool = ReassemblyPoolV4::new(TransferLimitsV3::v3()).map_err(|error| Fault {
            code: BridgeCode::StartFailed,
            detail: error.to_string(),
        })?;
        let receiver = pool
            .add_receiver(epoch.0, GateMode::AwaitSnapshot)
            .map_err(|error| Fault {
                code: BridgeCode::StartFailed,
                detail: error.to_string(),
            })?;
        Ok(Self {
            pool,
            receiver,
            replica: ReplicaWorldV4::new(epoch),
            outbox: MoveOutbox::new(epoch.0),
            table,
            view: Rc::new(SessionView::default()),
            options: Vec::new(),
            pending: None,
            next_seq: SEQ_FIRST,
            ended: false,
        })
    }

    /// The current immutable snapshot.
    pub(crate) fn view(&self) -> Rc<SessionView> {
        Rc::clone(&self.view)
    }

    /// Drains delivered frames, then rebuilds the view at most once.
    ///
    /// `options` is the host-ordered offer list. `refused` is true
    /// when the poll reported a dropped player frame, which clears the
    /// outstanding lane-0 intent. `now_ms` drives the pool's timers. A decode,
    /// pool, replica or outbox refusal is fatal (`52` / `53`) and leaves the
    /// view at its previous revision.
    pub(crate) fn step(
        &mut self,
        link: &mut impl Transport,
        options: &[Offered],
        refused: bool,
        now_ms: u64,
    ) -> Result<(), Fault> {
        let mut events = Vec::new();
        let mut receipts = Vec::new();
        let mut move_receipts = Vec::new();
        while let Some(bytes) = link.recv() {
            if bytes.first() == Some(&TRANSFER_VERSION_V3) {
                let outcome = self
                    .pool
                    .offer_transfer(self.receiver, &bytes, now_ms)
                    .map_err(transfer_fault)?;
                if let TransferOutcome::Published { transfer, .. } = outcome {
                    let reply = encode_up(&TransferUp::Published {
                        epoch: self.replica.v3().epoch().0,
                        transfer,
                    })
                    .map_err(|_| {
                        fault(
                            BridgeCode::ReplicaRejected,
                            "transfer reply did not fit the link",
                        )
                    })?;
                    link.send(&reply).map_err(|_| {
                        fault(
                            BridgeCode::ReplicaRejected,
                            "transfer reply did not fit the link",
                        )
                    })?;
                }
            } else if bytes.first() == Some(&MOVEMENT_VERSION) {
                let down = decode_down(&bytes).map_err(|error| Fault {
                    code: BridgeCode::FrameUndecodable,
                    detail: error.to_string(),
                })?;
                match down {
                    MoveDown::Receipt(receipt) => {
                        self.outbox.receive(&receipt).map_err(|error| Fault {
                            code: BridgeCode::ReplicaRejected,
                            detail: error.to_string(),
                        })?;
                        move_receipts.push(view_move_receipt(&receipt));
                    }
                    MoveDown::Sample(sample) => {
                        self.replica.apply_sample(&sample);
                    }
                }
            } else {
                self.pool
                    .offer_delta(self.receiver, &bytes, now_ms)
                    .map_err(transfer_fault)?;
            }
            while let Some(update) = self.pool.pop_update(self.receiver) {
                self.apply_update(update, &mut events, &mut receipts)?;
            }
        }
        if refused {
            self.pending = None;
        }
        self.rebuild(options, events, receipts, move_receipts);
        Ok(())
    }

    /// Validates and encodes one chosen option, then sends it upstream.
    ///
    /// The checks are the client-owned prefix of B14: negative arguments
    /// (`3`), stale revision (`4`), out-of-range index (`6`), outstanding
    /// intent (`5`). A full link is `30`; only the successful branch advances
    /// the sequence and marks the intent pending.
    pub(crate) fn submit(
        &mut self,
        revision: i64,
        index: i64,
        link: &mut impl Transport,
    ) -> Result<(), Fault> {
        if revision < 0 || index < 0 {
            return Err(fault(BridgeCode::BadArgument, "negative argument"));
        }
        if revision != self.view.revision() {
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
        let body = match offered.body {
            IntentBody::DeclareAction { ability, target } => {
                IntentBodyV4::DeclareAction { ability, target }
            }
            IntentBody::EndTurn => IntentBodyV4::EndTurn,
        };
        self.send_intent(offered.actor, body, link)
    }

    /// A turn-based move of the current options' actor.
    ///
    /// The checks are, in order: negative revision (`3`), stale revision
    /// (`4`), no offered option so no actor to move (`6`), outstanding
    /// intent (`5`). The sent frame is B-A7's with the first option's actor
    /// and a `DeclareMove` body.
    pub(crate) fn declare_move(
        &mut self,
        revision: i64,
        destination: Point,
        link: &mut impl Transport,
    ) -> Result<(), Fault> {
        if revision < 0 {
            return Err(fault(BridgeCode::BadArgument, "negative argument"));
        }
        if revision != self.view.revision() {
            return Err(fault(
                BridgeCode::StaleOptions,
                "revision is not the current view",
            ));
        }
        let Some(first) = self.options.first() else {
            return Err(fault(BridgeCode::NoSuchOption, "no actor to move"));
        };
        if self.pending.is_some() {
            return Err(fault(BridgeCode::IntentPending, "an intent is outstanding"));
        }
        let actor = first.actor;
        self.send_intent(actor, IntentBodyV4::DeclareMove { destination }, link)
    }

    /// A lane-1 `MoveTo` for `actor`.
    ///
    /// An outbox refusal is `30`; a full link restores the outbox and is
    /// `30` `"link full"`. The view is untouched.
    pub(crate) fn move_to(
        &mut self,
        actor: NetId,
        destination: Point,
        link: &mut impl Transport,
    ) -> Result<(), Fault> {
        self.send_move(actor, MoveBody::MoveTo { destination }, link)
    }

    /// A lane-1 `Halt` for `actor`.
    ///
    /// An outbox refusal is `30`; a full link restores the outbox and is
    /// `30` `"link full"`. The view is untouched.
    pub(crate) fn halt(&mut self, actor: NetId, link: &mut impl Transport) -> Result<(), Fault> {
        self.send_move(actor, MoveBody::Halt, link)
    }

    /// Encodes one lane-0 intent and sends it upstream.
    fn send_intent(
        &mut self,
        actor: NetId,
        body: IntentBodyV4,
        link: &mut impl Transport,
    ) -> Result<(), Fault> {
        let frame = IntentFrame {
            epoch: self.replica.v3().epoch().0,
            lane: LANE_COMBAT,
            seq: self.next_seq,
            observed_tick: self.replica.v3().server_tick(),
            actor,
            body,
        };
        let bytes = encode_lane0(&frame).map_err(|error| Fault {
            code: BridgeCode::BadArgument,
            detail: error.to_string(),
        })?;
        link.send(&bytes)
            .map_err(|_| fault(BridgeCode::IntentRefused, "link full"))?;
        self.pending = Some(self.next_seq);
        self.next_seq = self.next_seq.saturating_add(1);
        Ok(())
    }

    /// Mints one lane-1 intent and sends it upstream, restoring the outbox
    /// when the link is full.
    fn send_move(
        &mut self,
        actor: NetId,
        body: MoveBody,
        link: &mut impl Transport,
    ) -> Result<(), Fault> {
        let saved = self.outbox.clone();
        let intent = self.outbox.send(actor, body).map_err(|error| Fault {
            code: BridgeCode::IntentRefused,
            detail: error.to_string(),
        })?;
        let bytes = encode_move(&intent).map_err(|error| Fault {
            code: BridgeCode::IntentRefused,
            detail: error.to_string(),
        })?;
        if link.send(&bytes).is_err() {
            self.outbox = saved;
            return Err(fault(BridgeCode::IntentRefused, "link full"));
        }
        Ok(())
    }

    /// Applies one pooled update, collecting its events and receipts.
    fn apply_update(
        &mut self,
        update: ReplicaUpdateV4,
        events: &mut Vec<ViewEvent>,
        receipts: &mut Vec<ViewReceipt>,
    ) -> Result<(), Fault> {
        match update {
            ReplicaUpdateV4::Snapshot(publication) => {
                self.replica
                    .apply_snapshot(&publication)
                    .map_err(|error| Fault {
                        code: BridgeCode::ReplicaRejected,
                        detail: error.to_string(),
                    })?;
            }
            ReplicaUpdateV4::Delta(frame) => {
                self.replica.apply_delta(&frame).map_err(|error| Fault {
                    code: BridgeCode::ReplicaRejected,
                    detail: error.to_string(),
                })?;
                for op in &frame.ops {
                    self.apply_op(op, events, receipts);
                }
            }
        }
        Ok(())
    }

    /// Maps one applied delta op onto the view's events and receipts.
    fn apply_op(
        &mut self,
        op: &DeltaOp,
        events: &mut Vec<ViewEvent>,
        receipts: &mut Vec<ViewReceipt>,
    ) {
        match op {
            DeltaOp::V3(DeltaOpV3::ActionResolved {
                actor,
                target,
                ability,
                outcome,
                damage,
            }) => {
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
            DeltaOp::V3(DeltaOpV3::TurnStarted { actor, round }) => {
                events.push(ViewEvent::TurnStarted {
                    actor: wide(actor.get()),
                    round: wide(*round),
                });
            }
            DeltaOp::V3(DeltaOpV3::EncounterEnded { encounter, round }) => {
                self.ended = true;
                events.push(ViewEvent::EncounterEnded {
                    encounter: encounter.to_string(),
                    round: wide(*round),
                });
            }
            DeltaOp::V3(DeltaOpV3::Died { entity }) => {
                events.push(ViewEvent::Died {
                    entity: wide(entity.get()),
                });
            }
            DeltaOp::V3(DeltaOpV3::Receipt { seq, status, .. }) => {
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
            DeltaOp::Moved { entity, points } if self.replica.v3().entity(*entity).is_some() => {
                events.push(ViewEvent::Moved {
                    entity: wide(entity.get()),
                    points: points
                        .iter()
                        .flat_map(|point| point.raw())
                        .map(i64::from)
                        .collect(),
                });
            }
            _ => {}
        }
    }

    /// Rebuilds the view from the replica, at most one revision per call.
    fn rebuild(
        &mut self,
        options: &[Offered],
        events: Vec<ViewEvent>,
        receipts: Vec<ViewReceipt>,
        move_receipts: Vec<ViewMoveReceipt>,
    ) {
        let inner = self.replica.v3();
        let net_ids: Vec<i64> = inner
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
        for entity in inner.entities() {
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
        let active = inner
            .turn()
            .and_then(|turn| turn.active)
            .filter(|active| inner.entity(*active).is_some())
            .map(|active| wide(active.get()))
            .unwrap_or(0);
        let round = inner.turn().map(|turn| wide(turn.round)).unwrap_or(0);
        let mut positions = Vec::with_capacity(3 * net_ids.len());
        let mut position_known = Vec::with_capacity(net_ids.len());
        let mut moving = Vec::with_capacity(net_ids.len());
        for entity in inner.entities() {
            match self.replica.position(entity.net_id) {
                Some(state) => {
                    positions.extend(state.point.raw().iter().map(|raw| i64::from(*raw)));
                    position_known.push(1);
                    moving.push(u8::from(state.moving));
                }
                None => {
                    positions.extend([0, 0, 0]);
                    position_known.push(0);
                    moving.push(0);
                }
            }
        }
        let (option_labels, option_targets) = option_labels(&self.table, options);

        let current = &self.view;
        let unchanged = net_ids == current.net_ids()
            && names == current.names()
            && sides == current.sides()
            && placements == current.placements()
            && health == current.health()
            && max_health == current.max_health()
            && health_known == current.health_known()
            && dead == current.dead()
            && active == current.active()
            && round == current.round()
            && self.ended == current.ended()
            && option_labels == current.option_labels()
            && option_targets == current.option_targets()
            && positions == current.positions()
            && position_known == current.position_known()
            && moving == current.moving();
        if events.is_empty() && receipts.is_empty() && move_receipts.is_empty() && unchanged {
            self.options = options.to_vec();
            return;
        }

        self.view = Rc::new(SessionView::assemble(
            self.view.revision().saturating_add(1),
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
            self.ended,
            option_labels,
            option_targets,
            events,
            receipts,
            positions,
            position_known,
            moving,
            move_receipts,
        ));
        self.options = options.to_vec();
    }

    /// Test seam: the receiver starts `GateMode::Live`, so tests feed deltas
    /// from `event_seq` 1.
    #[cfg(test)]
    pub(crate) fn live_for_test(epoch: SessionEpoch, table: PresentationTable) -> Self {
        let mut pool = ReassemblyPoolV4::new(TransferLimitsV3::v3()).expect("v3 limits validate");
        let receiver = pool
            .add_receiver(epoch.0, GateMode::Live)
            .expect("a fresh pool admits a receiver");
        Self {
            pool,
            receiver,
            replica: ReplicaWorldV4::new(epoch),
            outbox: MoveOutbox::new(epoch.0),
            table,
            view: Rc::new(SessionView::default()),
            options: Vec::new(),
            pending: None,
            next_seq: SEQ_FIRST,
            ended: false,
        }
    }

    /// Test seam: the lane-1 sender state.
    #[cfg(test)]
    pub(crate) fn outbox(&self) -> &MoveOutbox {
        &self.outbox
    }
}

/// Maps a pool refusal onto the bridge table: a bare codec failure is
/// undecodable, anything else is a replica refusal.
fn transfer_fault(error: TransferError) -> Fault {
    match error {
        TransferError::Codec(codec) => Fault {
            code: BridgeCode::FrameUndecodable,
            detail: codec.to_string(),
        },
        other => Fault {
            code: BridgeCode::ReplicaRejected,
            detail: other.to_string(),
        },
    }
}

/// Maps one admitted lane-1 receipt onto the view's receipt row.
fn view_move_receipt(receipt: &MoveReceipt) -> ViewMoveReceipt {
    let (outcome, code, start_tick) = match receipt.outcome {
        MoveOutcome::Accepted { start_tick } => ("accepted", 0, wide(start_tick)),
        MoveOutcome::Superseded => ("superseded", 0, 0),
        MoveOutcome::Rejected(code) => ("rejected", i64::from(code.as_u8()), 0),
    };
    ViewMoveReceipt {
        seq: wide(receipt.seq),
        actor: wide(receipt.actor.get()),
        outcome,
        code,
        processed_tick: wide(receipt.processed_tick),
        start_tick,
    }
}

#[cfg(test)]
mod tests;

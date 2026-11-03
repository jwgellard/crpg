//! K8-K14: the client half's rows, events, revisions, receipts, labels and
//! frame refusals, plus the client-owned part of `submit_option`.

use std::rc::Rc;

use crpg_core::Ulid;
use crpg_net::codec_v2::{decode_intent, encode_delta};
use crpg_net::protocol::{
    DeltaOp as LegacyOp, IntentBody, NetId, ReceiptStatus, RejectionCode, SessionEpoch,
};
use crpg_net::protocol_v2::{DeltaFrame, DeltaOp};
use crpg_net::transport::Transport;

use super::{
    wide, Client, Offered, PresentationRow, PresentationTable, SessionView, ViewEvent, ViewReceipt,
    ViewValue,
};
use crate::core::errors::BridgeCode;
use crate::core::link::LocalLink;

fn epoch() -> SessionEpoch {
    SessionEpoch([1; 16])
}

fn id(raw: u64) -> NetId {
    NetId::new(raw).expect("nonzero id")
}

fn strike() -> Ulid {
    Ulid::from_u128(10)
}

fn other_ability() -> Ulid {
    Ulid::from_u128(11)
}

fn table() -> PresentationTable {
    let mut rows = std::collections::BTreeMap::new();
    rows.insert(
        id(1),
        PresentationRow {
            placement: Ulid::from_u128(14),
            name: "Hero".to_string(),
            side: "Player".to_string(),
        },
    );
    rows.insert(
        id(2),
        PresentationRow {
            placement: Ulid::from_u128(15),
            name: "Goblin".to_string(),
            side: "Monster".to_string(),
        },
    );
    let mut abilities = std::collections::BTreeMap::new();
    abilities.insert(strike(), "Strike".to_string());
    PresentationTable { rows, abilities }
}

fn client() -> Client {
    Client::new(epoch(), table())
}

fn enter(entity: u64) -> DeltaOp {
    DeltaOp::Legacy(LegacyOp::EntityEnter { entity: id(entity) })
}

fn leave(entity: u64) -> DeltaOp {
    DeltaOp::Legacy(LegacyOp::EntityLeave { entity: id(entity) })
}

fn health(entity: u64, health: u32, max_health: u32, dead: bool) -> DeltaOp {
    DeltaOp::Legacy(LegacyOp::Health {
        entity: id(entity),
        health,
        max_health,
        dead,
    })
}

fn turn(active: Option<u64>, round: u64) -> DeltaOp {
    DeltaOp::Legacy(LegacyOp::Turn {
        active: active.map(id),
        round,
    })
}

fn receipt(seq: u64, status: ReceiptStatus) -> DeltaOp {
    DeltaOp::Legacy(LegacyOp::Receipt {
        epoch: [1; 16],
        lane: 0,
        seq,
        processed_tick: 0,
        status,
    })
}

fn encoded(seq: u64, ops: Vec<DeltaOp>) -> Vec<u8> {
    encode_delta(&DeltaFrame {
        lane: 0,
        server_tick: 0,
        event_seq: seq,
        ops,
    })
    .expect("frame encodes")
}

fn push(link: &LocalLink, bytes: Vec<u8>) {
    link.authority_end().send(&bytes).expect("link has room");
}

fn offered(actor: u64, body: IntentBody) -> Offered {
    Offered {
        actor: id(actor),
        body,
    }
}

/// The default view is the contract's empty revision 0.
#[test]
fn the_default_view_is_empty_revision_zero() {
    let view = SessionView::default();
    assert_eq!(view, SessionView::default());
    assert_eq!(view.revision(), 0);
    assert!(view.net_ids().is_empty());
    assert_eq!(view.active(), 0);
    assert_eq!(view.round(), 0);
    assert!(!view.ended());
    assert_eq!(view.events(), &[]);
    assert_eq!(view.receipts(), &[]);
}

/// K8 / B11: rows come from the replica; the table columns are exact and a
/// missing row yields empty strings; `health_known` is not `health > 0`.
#[test]
fn rows_follow_the_replica_and_known_health_is_independent_of_zero() {
    let link = LocalLink::new(128);
    let mut end = link.client_end();
    let mut client = client();

    // Entity 3 has no table row; it must not panic and must read as "".
    push(&link, encoded(1, vec![enter(1), enter(2), enter(3)]));
    client.step(&mut end, &[], false).unwrap();
    let view = client.view();
    assert_eq!(view.net_ids(), &[1, 2, 3]);
    assert_eq!(
        view.names(),
        &["Hero".to_string(), "Goblin".to_string(), String::new()]
    );
    assert_eq!(
        view.sides(),
        &["Player".to_string(), "Monster".to_string(), String::new()]
    );
    assert_eq!(
        view.placements(),
        &[
            Ulid::from_u128(14).to_string(),
            Ulid::from_u128(15).to_string(),
            String::new(),
        ]
    );
    assert_eq!(view.health_known(), &[0, 0, 0]);
    assert_eq!(view.health(), &[0, 0, 0]);
    for column in [
        view.names().len(),
        view.sides().len(),
        view.placements().len(),
        view.health().len(),
        view.max_health().len(),
        view.health_known().len(),
        view.dead().len(),
    ] {
        assert_eq!(column, 3);
    }
    drop(view);

    push(&link, encoded(2, vec![health(1, 7, 9, false)]));
    client.step(&mut end, &[], false).unwrap();
    let view = client.view();
    assert_eq!(view.health_known(), &[1, 0, 0]);
    assert_eq!(view.health(), &[7, 0, 0]);
    assert_eq!(view.max_health(), &[9, 0, 0]);
    assert_eq!(view.dead(), &[0, 0, 0]);
    drop(view);

    // Control: a known health of 0 is still known.
    push(&link, encoded(3, vec![health(1, 0, 9, true)]));
    client.step(&mut end, &[], false).unwrap();
    let view = client.view();
    assert_eq!(view.health_known(), &[1, 0, 0]);
    assert_eq!(view.health(), &[0, 0, 0]);
    assert_eq!(view.dead(), &[1, 0, 0]);
    drop(view);

    // u32::MAX widens without truncation.
    push(
        &link,
        encoded(4, vec![health(2, u32::MAX, u32::MAX, false)]),
    );
    client.step(&mut end, &[], false).unwrap();
    let view = client.view();
    assert_eq!(view.health(), &[0, 4_294_967_295, 0]);
    assert_eq!(view.max_health(), &[9, 4_294_967_295, 0]);
    assert_eq!(view.health_known(), &[1, 1, 0]);
}

/// K9 / B6: one frame's events and receipts, in op order, with exact keys;
/// the next health-only step carries no old events and keeps `ended`.
#[test]
fn events_and_receipts_are_exact_and_not_repeated() {
    let link = LocalLink::new(128);
    let mut end = link.client_end();
    let mut client = client();
    let encounter = Ulid::from_u128(20);
    let strike_id = strike().to_string();
    let encounter_id = encounter.to_string();

    push(
        &link,
        encoded(
            1,
            vec![
                enter(1),
                enter(2),
                DeltaOp::ActionResolved {
                    actor: id(1),
                    target: id(2),
                    ability: strike(),
                    outcome: "success".to_string(),
                    damage: 5,
                },
                DeltaOp::Legacy(LegacyOp::Died { entity: id(2) }),
                DeltaOp::TurnStarted {
                    actor: id(1),
                    round: 2,
                },
                DeltaOp::EncounterEnded {
                    encounter,
                    round: 2,
                },
                receipt(1, ReceiptStatus::Applied),
                receipt(2, ReceiptStatus::Rejected(RejectionCode::NotAuthorized)),
            ],
        ),
    );
    client.step(&mut end, &[], false).unwrap();
    let view = client.view();

    let expected = vec![
        ViewEvent::ActionResolved {
            actor: 1,
            target: 2,
            ability: "Strike".to_string(),
            ability_id: strike().to_string(),
            outcome: "success".to_string(),
            damage: 5,
        },
        ViewEvent::Died { entity: 2 },
        ViewEvent::TurnStarted { actor: 1, round: 2 },
        ViewEvent::EncounterEnded {
            encounter: encounter.to_string(),
            round: 2,
        },
    ];
    assert_eq!(view.events(), expected.as_slice());
    assert_eq!(
        view.events()[0].entries(),
        vec![
            ("kind", ViewValue::Text("action_resolved")),
            ("actor", ViewValue::Int(1)),
            ("target", ViewValue::Int(2)),
            ("ability", ViewValue::Text("Strike")),
            ("ability_id", ViewValue::Text(&strike_id)),
            ("outcome", ViewValue::Text("success")),
            ("damage", ViewValue::Int(5)),
        ]
    );
    assert_eq!(
        view.events()[1].entries(),
        vec![
            ("kind", ViewValue::Text("died")),
            ("entity", ViewValue::Int(2)),
        ]
    );
    assert_eq!(
        view.events()[2].entries(),
        vec![
            ("kind", ViewValue::Text("turn_started")),
            ("actor", ViewValue::Int(1)),
            ("round", ViewValue::Int(2)),
        ]
    );
    assert_eq!(
        view.events()[3].entries(),
        vec![
            ("kind", ViewValue::Text("encounter_ended")),
            ("encounter", ViewValue::Text(&encounter_id)),
            ("round", ViewValue::Int(2)),
        ]
    );
    assert_eq!(
        view.receipts(),
        &[
            ViewReceipt {
                seq: 1,
                applied: true,
                code: 0
            },
            ViewReceipt {
                seq: 2,
                applied: false,
                code: 17
            },
        ]
    );
    assert_eq!(
        view.receipts()[0].entries(),
        vec![
            ("seq", ViewValue::Int(1)),
            ("applied", ViewValue::Bool(true)),
            ("code", ViewValue::Int(0)),
        ]
    );
    assert!(view.ended());
    let revision = view.revision();
    drop(view);

    // Control: a health-only frame bumps once, carries no events, keeps ended.
    push(&link, encoded(2, vec![health(1, 4, 10, false)]));
    client.step(&mut end, &[], false).unwrap();
    let view = client.view();
    assert_eq!(view.revision(), revision + 1);
    assert_eq!(view.events(), &[]);
    assert_eq!(view.receipts(), &[]);
    assert!(view.ended());
}

/// K10 / B7: the same `Rc` is kept when nothing changed; one bump per step.
#[test]
fn revision_bumps_at_most_once_and_keeps_the_rc_when_idle() {
    let link = LocalLink::new(128);
    let mut end = link.client_end();
    let mut client = client();
    let options = vec![offered(1, IntentBody::EndTurn)];

    client.step(&mut end, &options, false).unwrap();
    let first = client.view();
    assert_eq!(first.revision(), 1);

    client.step(&mut end, &options, false).unwrap();
    assert!(Rc::ptr_eq(&first, &client.view()));

    let before = first.revision();
    push(&link, encoded(1, vec![enter(1)]));
    push(&link, encoded(2, vec![health(1, 4, 10, false)]));
    client.step(&mut end, &options, false).unwrap();
    assert_eq!(client.view().revision(), before + 1, "two frames, one bump");
    let second = client.view();
    drop(first);
    drop(second);

    client.step(&mut end, &[], false).unwrap();
    assert_eq!(client.view().revision(), before + 2);
    assert_eq!(client.view().events(), &[]);
    assert_eq!(client.view().option_labels(), &[] as &[String]);

    let third = client.view();
    client.step(&mut end, &[], false).unwrap();
    assert!(Rc::ptr_eq(&third, &client.view()));
    let revision = third.revision();
    drop(third);

    // A frame that changes only the round still bumps by exactly one.
    push(&link, encoded(3, vec![turn(None, 1)]));
    client.step(&mut end, &[], false).unwrap();
    assert_eq!(client.view().revision(), revision + 1);
    assert_eq!(client.view().round(), 1);
    assert_eq!(client.view().events(), &[]);
    push(&link, encoded(4, vec![turn(None, 2)]));
    client.step(&mut end, &[], false).unwrap();
    assert_eq!(client.view().revision(), revision + 2);
    assert_eq!(client.view().round(), 2);
    assert_eq!(client.view().events(), &[]);

    // A frame that changes only `dead` still bumps by exactly one.
    push(&link, encoded(5, vec![health(1, 4, 10, true)]));
    client.step(&mut end, &[], false).unwrap();
    assert_eq!(client.view().revision(), revision + 3);
    assert_eq!(client.view().dead(), &[1]);
    assert_eq!(client.view().events(), &[]);
}

/// K11 / B12: an active id that left the rows becomes 0; round is kept.
#[test]
fn active_must_be_a_live_row_and_round_is_unadjusted() {
    let link = LocalLink::new(128);
    let mut end = link.client_end();
    let mut client = client();

    push(
        &link,
        encoded(1, vec![enter(1), enter(2), turn(Some(2), 5)]),
    );
    client.step(&mut end, &[], false).unwrap();
    assert_eq!(client.view().active(), 2);
    assert_eq!(client.view().round(), 5);

    push(&link, encoded(2, vec![leave(2)]));
    client.step(&mut end, &[], false).unwrap();
    assert_eq!(client.view().active(), 0);
    assert_eq!(client.view().round(), 5);
}

/// K12 / B6, B14: only the matching receipt or a refusal clears pending.
#[test]
fn pending_is_cleared_only_by_its_receipt_or_a_refusal() {
    let link = LocalLink::new(128);
    let mut end = link.client_end();
    let mut client = client();
    client.pending = Some(4);

    push(&link, encoded(1, vec![receipt(3, ReceiptStatus::Applied)]));
    client.step(&mut end, &[], false).unwrap();
    assert_eq!(client.pending, Some(4));

    push(
        &link,
        encoded(
            2,
            vec![receipt(
                4,
                ReceiptStatus::Rejected(RejectionCode::IllegalAction),
            )],
        ),
    );
    client.step(&mut end, &[], false).unwrap();
    assert_eq!(client.pending, None);

    client.pending = Some(4);
    client.step(&mut end, &[], true).unwrap();
    assert_eq!(client.pending, None);
}

/// K13 / B13: labels and targets in host order with both fallbacks.
#[test]
fn option_labels_use_the_full_table_then_their_fallbacks() {
    let link = LocalLink::new(128);
    let mut end = link.client_end();
    let mut client = client();
    let options = vec![
        offered(
            1,
            IntentBody::DeclareAction {
                ability: strike(),
                target: id(2),
            },
        ),
        offered(1, IntentBody::EndTurn),
        offered(
            1,
            IntentBody::DeclareAction {
                ability: strike(),
                target: id(99),
            },
        ),
        offered(
            1,
            IntentBody::DeclareAction {
                ability: other_ability(),
                target: id(2),
            },
        ),
    ];

    client.step(&mut end, &options, false).unwrap();
    let view = client.view();
    assert_eq!(
        view.option_labels(),
        &[
            "Strike -> Goblin".to_string(),
            "End turn".to_string(),
            "Strike".to_string(),
            format!("{} -> Goblin", other_ability()),
        ]
    );
    assert_eq!(view.option_targets(), &[2, 0, 99, 2]);
    drop(view);

    client.step(&mut end, &[], false).unwrap();
    assert_eq!(client.view().option_labels(), &[] as &[String]);
    assert_eq!(client.view().option_targets(), &[] as &[i64]);
}

/// K14 / B6: a bad frame is fatal, never skipped; a gap is `53`.
#[test]
fn bad_frames_are_fatal_and_never_skipped() {
    let link = LocalLink::new(128);
    let mut bad = client();
    let mut end = link.client_end();

    push(&link, vec![0xff]);
    let error = bad.step(&mut end, &[], false).unwrap_err();
    assert_eq!(error.code, BridgeCode::FrameUndecodable);

    let mut gap = self::client();
    let before = gap.view();
    push(&link, encoded(2, vec![]));
    let error = gap.step(&mut end, &[], false).unwrap_err();
    assert_eq!(error.code, BridgeCode::ReplicaRejected);
    assert!(Rc::ptr_eq(&before, &gap.view()));
    assert!(gap.view().net_ids().is_empty());

    // Control: event_seq 1 is accepted.
    let mut control = self::client();
    push(&link, encoded(1, vec![]));
    control.step(&mut end, &[], false).unwrap();
}

/// B14: the client-owned precedence, and only a successful submit sends.
#[test]
fn submit_checks_in_order_and_only_the_success_sends() {
    let link = LocalLink::new(2);
    let mut end = link.client_end();
    let mut client = client();
    let options = vec![offered(1, IntentBody::EndTurn)];
    client.step(&mut end, &options, false).unwrap();
    let revision = client.view().revision();

    for (revision, index, code) in [
        (-1, 0, BridgeCode::BadArgument),
        (revision, -1, BridgeCode::BadArgument),
        (revision + 1, 0, BridgeCode::StaleOptions),
        (revision, 99, BridgeCode::NoSuchOption),
    ] {
        assert_eq!(
            client.submit(revision, index, &mut end).unwrap_err().code,
            code
        );
        assert_eq!(link.queued().0, 0, "a refused submit sends nothing");
    }

    client.submit(revision, 0, &mut end).unwrap();
    assert_eq!(link.queued().0, 1);
    assert_eq!(client.pending, Some(1));

    // 6 (index) beats 5 (pending), and 5 still sends nothing.
    assert_eq!(
        client.submit(revision, 99, &mut end).unwrap_err().code,
        BridgeCode::NoSuchOption
    );
    assert_eq!(
        client.submit(revision, 0, &mut end).unwrap_err().code,
        BridgeCode::IntentPending
    );
    assert_eq!(link.queued().0, 1);

    let bytes = link.authority_end().recv().expect("one frame");
    let frame = decode_intent(&bytes).expect("intent decodes");
    assert_eq!(frame.epoch, [1; 16]);
    assert_eq!(frame.lane, 0);
    assert_eq!(frame.seq, 1);
    assert_eq!(frame.observed_tick, 0);
    assert_eq!(frame.body, IntentBody::EndTurn);
    assert_eq!(frame.actor, id(1));

    // A full link is 30 and leaves pending untouched.
    let full = LocalLink::new(1);
    let mut full_end = full.client_end();
    let mut client = self::client();
    client.step(&mut full_end, &options, false).unwrap();
    let revision = client.view().revision();
    client.submit(revision, 0, &mut full_end).unwrap();
    client.pending = None;
    assert_eq!(
        client.submit(revision, 0, &mut full_end).unwrap_err().code,
        BridgeCode::IntentRefused
    );
    assert_eq!(client.pending, None);
    assert_eq!(full.queued().0, 1);
}

/// B6 / D8: an ability missing from the table displays its ULID text, and
/// `wide` saturates above `i64::MAX` instead of wrapping.
#[test]
fn missing_ability_uses_ulid_text_and_wide_saturates() {
    let link = LocalLink::new(128);
    let mut end = link.client_end();
    let mut client = client();
    let ability_id = other_ability().to_string();

    push(
        &link,
        encoded(
            1,
            vec![
                enter(1),
                enter(2),
                DeltaOp::ActionResolved {
                    actor: id(1),
                    target: id(2),
                    ability: other_ability(),
                    outcome: "success".to_string(),
                    damage: 3,
                },
            ],
        ),
    );
    client.step(&mut end, &[], false).unwrap();
    let view = client.view();
    assert_eq!(
        view.events(),
        &[ViewEvent::ActionResolved {
            actor: 1,
            target: 2,
            ability: ability_id.clone(),
            ability_id: ability_id.clone(),
            outcome: "success".to_string(),
            damage: 3,
        }]
    );
    assert_eq!(
        view.events()[0].entries(),
        vec![
            ("kind", ViewValue::Text("action_resolved")),
            ("actor", ViewValue::Int(1)),
            ("target", ViewValue::Int(2)),
            ("ability", ViewValue::Text(&ability_id)),
            ("ability_id", ViewValue::Text(&ability_id)),
            ("outcome", ViewValue::Text("success")),
            ("damage", ViewValue::Int(3)),
        ]
    );
    drop(view);

    assert_eq!(wide(0), 0);
    assert_eq!(wide(i64::MAX as u64), i64::MAX);
    assert_eq!(wide((i64::MAX as u64) + 1), i64::MAX);
    assert_eq!(wide(u64::MAX), i64::MAX);
}

/// B6: `Died` is a notice, not a health fact; `encounter_ended` latches.
#[test]
fn died_does_not_set_the_dead_column() {
    let link = LocalLink::new(128);
    let mut end = link.client_end();
    let mut client = client();

    push(&link, encoded(1, vec![enter(1), enter(2)]));
    client.step(&mut end, &[], false).unwrap();
    push(
        &link,
        encoded(2, vec![DeltaOp::Legacy(LegacyOp::Died { entity: id(2) })]),
    );
    client.step(&mut end, &[], false).unwrap();
    assert_eq!(client.view().dead(), &[0, 0]);
}

/// R1 / B12: a leave removes exactly its row from every column on one bump, a
/// dead row stays, and a frame re-entering a retired id is refused with 53
/// without changing the view `Rc`; a fresh id in the same frame is accepted.
#[test]
fn a_leave_shrinks_every_column_and_a_retired_id_cannot_return() {
    let link = LocalLink::new(128);
    let mut end = link.client_end();
    let mut client = client();

    push(&link, encoded(1, vec![enter(1), enter(2)]));
    client.step(&mut end, &[], false).unwrap();
    let view = client.view();
    assert_eq!(view.revision(), 1);
    assert_eq!(view.net_ids(), &[1, 2]);
    assert_eq!(view.names(), &["Hero".to_string(), "Goblin".to_string()]);
    assert_eq!(view.sides(), &["Player".to_string(), "Monster".to_string()]);
    assert_eq!(view.dead(), &[0, 0]);
    drop(view);

    // A health fact with dead = true keeps the row and bumps by one.
    push(&link, encoded(2, vec![health(2, 0, 7, true)]));
    client.step(&mut end, &[], false).unwrap();
    let view = client.view();
    assert_eq!(view.revision(), 2);
    assert_eq!(view.net_ids(), &[1, 2]);
    assert_eq!(view.dead(), &[0, 1]);
    drop(view);

    // Leaving id 1 proves the survivor's columns realign.
    push(&link, encoded(3, vec![leave(1)]));
    client.step(&mut end, &[], false).unwrap();
    let view = client.view();
    assert_eq!(view.revision(), 3);
    assert_eq!(view.net_ids(), &[2]);
    assert_eq!(view.names(), &["Goblin".to_string()]);
    assert_eq!(view.sides(), &["Monster".to_string()]);
    assert_eq!(view.placements(), &[Ulid::from_u128(15).to_string()]);
    assert_eq!(view.health(), &[0]);
    assert_eq!(view.max_health(), &[7]);
    assert_eq!(view.health_known(), &[1]);
    assert_eq!(view.dead(), &[1]);
    drop(view);

    push(&link, encoded(4, vec![leave(2)]));
    client.step(&mut end, &[], false).unwrap();
    let view = client.view();
    assert_eq!(view.revision(), 4);
    assert!(view.net_ids().is_empty());
    assert!(view.names().is_empty());
    assert!(view.sides().is_empty());
    assert!(view.placements().is_empty());
    assert!(view.health().is_empty());
    assert!(view.max_health().is_empty());
    assert!(view.health_known().is_empty());
    assert!(view.dead().is_empty());
    drop(view);

    let before = client.view();
    push(&link, encoded(5, vec![enter(1)]));
    let error = client.step(&mut end, &[], false).unwrap_err();
    assert_eq!(error.code, BridgeCode::ReplicaRejected);
    assert!(Rc::ptr_eq(&before, &client.view()));
    assert_eq!(client.view().revision(), 4);
    drop(before);

    // Control: the same frames accept a fresh id, so the refusal is the id.
    let control_link = LocalLink::new(128);
    let mut control_end = control_link.client_end();
    let mut control = self::client();
    push(&control_link, encoded(1, vec![enter(1), enter(2)]));
    control.step(&mut control_end, &[], false).unwrap();
    push(&control_link, encoded(2, vec![health(2, 0, 7, true)]));
    control.step(&mut control_end, &[], false).unwrap();
    push(&control_link, encoded(3, vec![leave(1)]));
    control.step(&mut control_end, &[], false).unwrap();
    push(&control_link, encoded(4, vec![leave(2)]));
    control.step(&mut control_end, &[], false).unwrap();
    push(&control_link, encoded(5, vec![enter(3)]));
    control.step(&mut control_end, &[], false).unwrap();
    let view = control.view();
    assert_eq!(view.revision(), 5);
    assert_eq!(view.net_ids(), &[3]);
    assert_eq!(view.names(), &[String::new()]);
    drop(view);
}

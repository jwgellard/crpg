//! T151a: the v4 client half's positions, `Moved`, move receipts, views,
//! lane-0 submits and frame refusals, plus the client-owned part of
//! `declare_move`.

use std::rc::Rc;

use crpg_core::Ulid;
use crpg_net::codec_v2;
use crpg_net::codec_v4::{decode_intent, encode_delta};
use crpg_net::movement_v4::{
    decode_intent as decode_move, encode_down, encode_intent as encode_move, MoveBody, MoveDown,
    MoveIntent, MoveOutcome, MoveReceipt, MoveSample, SampleEntry,
};
use crpg_net::protocol::{DeltaOp as LegacyOp, IntentBody, NetId, SessionEpoch, SEQ_FIRST};
use crpg_net::protocol_v2::{DeltaFrame as DeltaFrameV2, DeltaOp as DeltaOpV2};
use crpg_net::protocol_v3::{DeltaOp as DeltaOpV3, ReceiptStatus, RejectionCode};
use crpg_net::protocol_v4::{DeltaFrame, DeltaOp, IntentBody as IntentBodyV4, Point, PositionFact};
use crpg_net::transport::Transport;

use super::ClientV4;
use crate::core::client::{
    Client, Offered, PresentationRow, PresentationTable, SessionView, ViewEvent, ViewMoveReceipt,
    ViewReceipt, ViewValue,
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

fn live() -> ClientV4 {
    ClientV4::live_for_test(epoch(), table())
}

fn p(x: i32, y: i32, z: i32) -> Point {
    Point::new([x, y, z]).expect("in range")
}

fn v4(seq: u64, tick: u64, ops: Vec<DeltaOp>) -> Vec<u8> {
    encode_delta(&DeltaFrame {
        lane: 0,
        server_tick: tick,
        event_seq: seq,
        ops,
    })
    .expect("frame encodes")
}

fn enter(entity: u64) -> DeltaOp {
    DeltaOp::V3(DeltaOpV3::EntityEnter { entity: id(entity) })
}

fn pos(entity: u64, point: Point, moving: bool) -> DeltaOp {
    DeltaOp::Position {
        entity: id(entity),
        position: Some(PositionFact { point, moving }),
    }
}

fn receipt(seq: u64, status: ReceiptStatus) -> DeltaOp {
    DeltaOp::V3(DeltaOpV3::Receipt {
        epoch: [1; 16],
        lane: 0,
        seq,
        processed_tick: 0,
        status,
    })
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

fn move_down(seq: u64, actor: u64, tick: u64, outcome: MoveOutcome) -> Vec<u8> {
    encode_down(&MoveDown::Receipt(MoveReceipt {
        epoch: [1; 16],
        seq,
        actor: id(actor),
        processed_tick: tick,
        outcome,
    }))
    .expect("receipt encodes")
}

/// B-A2: `new` starts empty at the first sequence with the default view.
#[test]
fn new_builds_an_empty_client_half_at_the_first_sequence() {
    let client = ClientV4::new(epoch(), table()).expect("fresh resources admit a receiver");
    assert_eq!(client.view(), Rc::new(SessionView::default()));
    assert_eq!(client.next_seq, SEQ_FIRST);
    assert_eq!(client.pending, None);
    assert!(client.options.is_empty());
    assert!(!client.ended);
    assert!(client.outbox().pending().is_empty());
    assert_eq!(client.outbox().next_seq(), SEQ_FIRST);
}

/// B-A1, B-A3, B-A5, B-A6: anchors fill the position columns, samples move
/// them only when newer, and an idle step keeps the `Rc`.
#[test]
fn positions_follow_anchors_and_samples() {
    let link = LocalLink::new(128);
    let mut end = link.client_end();
    let mut client = live();

    push(
        &link,
        v4(
            1,
            5,
            vec![enter(1), enter(2), pos(1, p(65536, 0, 0), false)],
        ),
    );
    client.step(&mut end, &[], false, 0).unwrap();
    let view = client.view();
    assert_eq!(view.revision(), 1);
    assert_eq!(view.positions(), &[65536, 0, 0, 0, 0, 0]);
    assert_eq!(view.position_known(), &[1, 0]);
    assert_eq!(view.moving(), &[0, 0]);
    assert!(view.events().is_empty());
    assert!(view.move_receipts().is_empty());
    drop(view);

    let sample = MoveSample {
        server_tick: 6,
        processed_seq: 0,
        entries: vec![
            SampleEntry {
                entity: id(1),
                point: p(131072, 0, 0),
                moving: true,
            },
            SampleEntry {
                entity: id(2),
                point: p(1, 1, 1),
                moving: false,
            },
        ],
    };
    let bytes = encode_down(&MoveDown::Sample(sample)).expect("sample encodes");
    push(&link, bytes.clone());
    client.step(&mut end, &[], false, 0).unwrap();
    let view = client.view();
    assert_eq!(view.revision(), 2);
    assert_eq!(view.positions(), &[131072, 0, 0, 0, 0, 0]);
    assert_eq!(view.moving(), &[1, 0]);
    assert_eq!(view.position_known(), &[1, 0]);
    drop(view);

    // The same sample again, then one at tick 4: neither is newer, so the
    // revision stays and the `Rc` is kept.
    let second = client.view();
    push(&link, bytes.clone());
    client.step(&mut end, &[], false, 0).unwrap();
    assert!(Rc::ptr_eq(&second, &client.view()));
    push(
        &link,
        encode_down(&MoveDown::Sample(MoveSample {
            server_tick: 4,
            processed_seq: 0,
            entries: vec![
                SampleEntry {
                    entity: id(1),
                    point: p(131072, 0, 0),
                    moving: true,
                },
                SampleEntry {
                    entity: id(2),
                    point: p(1, 1, 1),
                    moving: false,
                },
            ],
        }))
        .expect("sample encodes"),
    );
    client.step(&mut end, &[], false, 0).unwrap();
    assert_eq!(client.view().revision(), 2);
    assert!(Rc::ptr_eq(&second, &client.view()));
    drop(second);

    push(
        &link,
        v4(
            2,
            7,
            vec![DeltaOp::Position {
                entity: id(1),
                position: None,
            }],
        ),
    );
    client.step(&mut end, &[], false, 0).unwrap();
    let view = client.view();
    assert_eq!(view.revision(), 3);
    assert_eq!(view.position_known(), &[0, 0]);
    assert_eq!(view.positions(), &[0, 0, 0, 0, 0, 0]);
}

/// B-A4: `Moved` is an event only for a live row, with exact keys.
#[test]
fn moved_is_an_event_only_for_live_rows() {
    let link = LocalLink::new(128);
    let mut end = link.client_end();
    let mut client = live();

    push(
        &link,
        v4(
            1,
            0,
            vec![
                enter(1),
                DeltaOp::Moved {
                    entity: id(1),
                    points: vec![p(0, 0, 0), p(32768, 0, 32768)],
                },
                DeltaOp::Moved {
                    entity: id(9),
                    points: vec![p(0, 0, 0), p(65536, 0, 0)],
                },
            ],
        ),
    );
    client.step(&mut end, &[], false, 0).unwrap();
    let view = client.view();
    assert_eq!(
        view.events(),
        &[ViewEvent::Moved {
            entity: 1,
            points: vec![0, 0, 0, 32768, 0, 32768],
        }]
    );
    assert_eq!(
        view.events()[0].entries(),
        vec![
            ("kind", ViewValue::Text("moved")),
            ("entity", ViewValue::Int(1)),
            ("points", ViewValue::Ints(&[0, 0, 0, 32768, 0, 32768])),
        ]
    );
    assert_eq!(view.position_known(), &[0]);
}

/// B-A4: every mapped v3 op reads exactly as the v2 client builds it.
#[test]
fn v3_ops_map_as_v2() {
    let link = LocalLink::new(128);
    let mut end = link.client_end();
    let mut client = live();
    let encounter = Ulid::from_u128(20);
    let strike_id = strike().to_string();
    let encounter_id = encounter.to_string();

    push(
        &link,
        v4(
            1,
            0,
            vec![
                enter(1),
                enter(2),
                DeltaOp::V3(DeltaOpV3::ActionResolved {
                    actor: id(1),
                    target: id(2),
                    ability: strike(),
                    outcome: "success".to_string(),
                    damage: 5,
                }),
                DeltaOp::V3(DeltaOpV3::Died { entity: id(2) }),
                DeltaOp::V3(DeltaOpV3::TurnStarted {
                    actor: id(1),
                    round: 2,
                }),
                DeltaOp::V3(DeltaOpV3::EncounterEnded {
                    encounter,
                    round: 2,
                }),
                receipt(1, ReceiptStatus::Applied),
                receipt(2, ReceiptStatus::Rejected(RejectionCode::WrongMode)),
            ],
        ),
    );
    client.step(&mut end, &[], false, 0).unwrap();
    let view = client.view();

    assert_eq!(
        view.events(),
        &[
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
        ]
    );
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
                code: 22
            },
        ]
    );
    assert!(view.ended());
}

/// B-A5, B-A9: lane-1 receipts complete the outbox in order with exact rows.
#[test]
fn lane1_receipts_complete_the_outbox_in_order() {
    let link = LocalLink::new(128);
    let mut end = link.client_end();
    let mut client = live();

    client.move_to(id(1), p(65536, 0, 0), &mut end).unwrap();
    assert_eq!(link.queued().0, 1);
    let bytes = link.authority_end().recv().expect("one frame");
    assert_eq!(
        bytes,
        encode_move(&MoveIntent {
            epoch: [1; 16],
            seq: 1,
            actor: id(1),
            body: MoveBody::MoveTo {
                destination: p(65536, 0, 0),
            },
        })
        .expect("intent encodes")
    );
    client.halt(id(1), &mut end).unwrap();
    client.move_to(id(1), p(65536, 0, 0), &mut end).unwrap();
    assert_eq!(link.queued().0, 2);

    push(
        &link,
        move_down(1, 1, 7, MoveOutcome::Accepted { start_tick: 8 }),
    );
    push(
        &link,
        move_down(2, 1, 7, MoveOutcome::Rejected(RejectionCode::WrongMode)),
    );
    push(&link, move_down(3, 1, 9, MoveOutcome::Superseded));
    client.step(&mut end, &[], false, 0).unwrap();
    let view = client.view();
    assert_eq!(view.revision(), 1);
    assert_eq!(
        view.move_receipts(),
        &[
            ViewMoveReceipt {
                seq: 1,
                actor: 1,
                outcome: "accepted",
                code: 0,
                processed_tick: 7,
                start_tick: 8,
            },
            ViewMoveReceipt {
                seq: 2,
                actor: 1,
                outcome: "rejected",
                code: 22,
                processed_tick: 7,
                start_tick: 0,
            },
            ViewMoveReceipt {
                seq: 3,
                actor: 1,
                outcome: "superseded",
                code: 0,
                processed_tick: 9,
                start_tick: 0,
            },
        ]
    );
    drop(view);
    assert!(client.outbox().pending().is_empty());
    assert_eq!(client.outbox().processed_seq(), 3);

    // Control: a receipt for seq 2 before seq 1 is a 53 that changes nothing.
    let link = LocalLink::new(128);
    let mut end = link.client_end();
    let mut client = live();
    client.move_to(id(1), p(0, 0, 0), &mut end).unwrap();
    client.halt(id(1), &mut end).unwrap();
    push(&link, move_down(2, 1, 7, MoveOutcome::Superseded));
    let before = client.view();
    let error = client.step(&mut end, &[], false, 0).unwrap_err();
    assert_eq!(error.code, BridgeCode::ReplicaRejected);
    assert_eq!(error.detail, "Unexpected at movement/outbox");
    assert!(Rc::ptr_eq(&before, &client.view()));
    assert_eq!(client.view().revision(), 0);
}

/// B-A9: a refused send consumes no `seq` and leaves no pending intent.
#[test]
fn move_window_and_full_link_change_nothing() {
    let link = LocalLink::new(128);
    let mut end = link.client_end();
    let mut client = live();

    for _ in 0..8 {
        client.move_to(id(1), p(0, 0, 0), &mut end).unwrap();
    }
    let error = client.move_to(id(1), p(0, 0, 0), &mut end).unwrap_err();
    assert_eq!(error.code, BridgeCode::IntentRefused);
    assert_eq!(error.detail, "Full at movement/outbox");
    assert_eq!(link.queued().0, 8);

    // Control: after the seq-1 receipt is stepped, the ninth is `Ok` with
    // seq 9.
    push(&link, move_down(1, 1, 0, MoveOutcome::Superseded));
    client.step(&mut end, &[], false, 0).unwrap();
    client.move_to(id(1), p(0, 0, 0), &mut end).unwrap();
    let mut last = Vec::new();
    let mut up = link.authority_end();
    while let Some(bytes) = up.recv() {
        last = bytes;
    }
    assert_eq!(decode_move(&last).expect("intent decodes").seq, 9);

    // A full link restores the outbox: the next `seq` and the pending count
    // are unchanged, and draining lets the same `seq` send.
    let small = LocalLink::new(1);
    let mut end = small.client_end();
    let mut client = live();
    client.move_to(id(1), p(0, 0, 0), &mut end).unwrap();
    let error = client.move_to(id(1), p(1, 1, 1), &mut end).unwrap_err();
    assert_eq!(error.code, BridgeCode::IntentRefused);
    assert_eq!(error.detail, "link full");
    assert_eq!(client.outbox().next_seq(), 2);
    assert_eq!(client.outbox().pending().len(), 1);
    small.authority_end().recv().expect("one frame");
    client.move_to(id(1), p(1, 1, 1), &mut end).unwrap();
    let bytes = small.authority_end().recv().expect("one frame");
    assert_eq!(decode_move(&bytes).expect("intent decodes").seq, 2);
}

/// B-A7, B-A8: lane-0 intents encode as v4 with the observed tick and the
/// options' actor, and `declare_move` follows its precedence.
#[test]
fn lane0_submits_v4_and_declare_move() {
    let link = LocalLink::new(128);
    let mut end = link.client_end();
    let mut client = live();
    let options = vec![
        offered(
            2,
            IntentBody::DeclareAction {
                ability: strike(),
                target: id(1),
            },
        ),
        offered(2, IntentBody::EndTurn),
    ];

    push(&link, v4(1, 9, vec![enter(1), enter(2)]));
    client.step(&mut end, &options, false, 0).unwrap();
    assert_eq!(client.view().revision(), 1);

    client.submit(1, 1, &mut end).unwrap();
    let bytes = link.authority_end().recv().expect("one frame");
    assert_eq!(bytes.first(), Some(&4));
    let frame = decode_intent(&bytes).expect("intent decodes");
    assert_eq!(frame.epoch, [1; 16]);
    assert_eq!(frame.lane, 0);
    assert_eq!(frame.seq, 1);
    assert_eq!(frame.observed_tick, 9);
    assert_eq!(frame.actor, id(2));
    assert_eq!(frame.body, IntentBodyV4::EndTurn);

    let error = client
        .declare_move(1, p(32768, 0, 32768), &mut end)
        .unwrap_err();
    assert_eq!(error.code, BridgeCode::IntentPending);

    push(&link, v4(2, 9, vec![receipt(1, ReceiptStatus::Applied)]));
    client.step(&mut end, &options, false, 0).unwrap();
    assert_eq!(client.view().revision(), 2);

    let error = client
        .declare_move(1, p(32768, 0, 32768), &mut end)
        .unwrap_err();
    assert_eq!(error.code, BridgeCode::StaleOptions);
    let error = client
        .declare_move(-1, p(32768, 0, 32768), &mut end)
        .unwrap_err();
    assert_eq!(error.code, BridgeCode::BadArgument);

    client
        .declare_move(2, p(32768, 0, 32768), &mut end)
        .unwrap();
    let bytes = link.authority_end().recv().expect("one frame");
    let frame = decode_intent(&bytes).expect("intent decodes");
    assert_eq!(frame.seq, 2);
    assert_eq!(frame.observed_tick, 9);
    assert_eq!(frame.actor, id(2));
    assert_eq!(
        frame.body,
        IntentBodyV4::DeclareMove {
            destination: p(32768, 0, 32768),
        }
    );

    // A step with no options and no pending leaves `declare_move` with no
    // actor to move.
    push(&link, v4(3, 9, vec![receipt(2, ReceiptStatus::Applied)]));
    client.step(&mut end, &[], false, 0).unwrap();
    let revision = client.view().revision();
    let error = client
        .declare_move(revision, p(0, 0, 0), &mut end)
        .unwrap_err();
    assert_eq!(error.code, BridgeCode::NoSuchOption);
    assert_eq!(error.detail, "no actor to move");
}

/// B-A3, B-A5: a bad frame is fatal, never skipped.
#[test]
fn bad_frames_are_fatal() {
    let link = LocalLink::new(128);
    let mut end = link.client_end();
    let mut bad = live();

    push(&link, vec![0xA1, 0xFF]);
    let error = bad.step(&mut end, &[], false, 0).unwrap_err();
    assert_eq!(error.code, BridgeCode::FrameUndecodable);
    assert_eq!(bad.view().revision(), 0);

    let link = LocalLink::new(128);
    let mut end = link.client_end();
    let mut gap = live();
    let before = gap.view();
    push(
        &link,
        codec_v2::encode_delta(&DeltaFrameV2 {
            lane: 0,
            server_tick: 0,
            event_seq: 1,
            ops: vec![],
        })
        .expect("v2 frame encodes"),
    );
    let error = gap.step(&mut end, &[], false, 0).unwrap_err();
    assert_eq!(error.code, BridgeCode::FrameUndecodable);
    assert!(Rc::ptr_eq(&before, &gap.view()));
    assert_eq!(gap.view().revision(), 0);
    drop(before);

    let link = LocalLink::new(128);
    let mut end = link.client_end();
    let mut unknown = live();
    push(&link, v4(1, 0, vec![pos(1, p(0, 0, 0), false)]));
    let error = unknown.step(&mut end, &[], false, 0).unwrap_err();
    assert_eq!(error.code, BridgeCode::ReplicaRejected);
    assert_eq!(unknown.view().revision(), 0);

    // Control: the same anchor with its row enters cleanly.
    let link = LocalLink::new(128);
    let mut end = link.client_end();
    let mut control = live();
    push(&link, v4(1, 0, vec![enter(1), pos(1, p(0, 0, 0), false)]));
    control.step(&mut end, &[], false, 0).unwrap();
}

/// B-A1: the v2 client fills the new columns with zeros.
#[test]
fn v2_views_have_zeroed_position_columns() {
    let link = LocalLink::new(128);
    let mut end = link.client_end();
    let mut client = Client::new(epoch(), table());

    push(
        &link,
        codec_v2::encode_delta(&DeltaFrameV2 {
            lane: 0,
            server_tick: 0,
            event_seq: 1,
            ops: vec![
                DeltaOpV2::Legacy(LegacyOp::EntityEnter { entity: id(1) }),
                DeltaOpV2::Legacy(LegacyOp::EntityEnter { entity: id(2) }),
            ],
        })
        .expect("v2 frame encodes"),
    );
    client.step(&mut end, &[], false).unwrap();
    let view = client.view();
    assert_eq!(view.positions(), &[0; 6]);
    assert_eq!(view.position_known(), &[0, 0]);
    assert_eq!(view.moving(), &[0, 0]);
    assert!(view.move_receipts().is_empty());
    drop(view);

    // Control: the default view has all four empty.
    let view = SessionView::default();
    assert!(view.positions().is_empty());
    assert!(view.position_known().is_empty());
    assert!(view.moving().is_empty());
    assert!(view.move_receipts().is_empty());
}

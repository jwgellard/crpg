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
    PresentationTable {
        rows,
        abilities,
        movement_pool: None,
    }
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

/// B-A6: `step` with `refused = true` clears lane-0 pending; `false` keeps it.
#[test]
fn step_refused_clears_lane0_pending() {
    let options = vec![
        offered(2, IntentBody::EndTurn),
        offered(2, IntentBody::EndTurn),
    ];
    let link = LocalLink::new(128);
    let mut end = link.client_end();
    let mut client = live();
    push(&link, v4(1, 9, vec![enter(1), enter(2)]));
    client.step(&mut end, &options, false, 0).unwrap();
    client.submit(1, 0, &mut end).unwrap();
    assert_eq!(client.pending, Some(1));
    assert_eq!(client.next_seq, 2);
    client.step(&mut end, &options, true, 0).unwrap();
    assert_eq!(client.pending, None);

    // Control: the same with `refused = false` keeps the intent pending.
    let link = LocalLink::new(128);
    let mut end = link.client_end();
    let mut control = live();
    push(&link, v4(1, 9, vec![enter(1), enter(2)]));
    control.step(&mut end, &options, false, 0).unwrap();
    control.submit(1, 0, &mut end).unwrap();
    assert_eq!(control.pending, Some(1));
    control.step(&mut end, &options, false, 0).unwrap();
    assert_eq!(control.pending, Some(1));
}

/// B-A7 check 3: `submit` refuses either argument negative with code 3.
#[test]
fn submit_negative_either_argument_is_bad_argument() {
    let options = vec![
        offered(2, IntentBody::EndTurn),
        offered(2, IntentBody::EndTurn),
    ];
    let link = LocalLink::new(128);
    let mut end = link.client_end();
    let mut client = live();
    push(&link, v4(1, 9, vec![enter(1), enter(2)]));
    client.step(&mut end, &options, false, 0).unwrap();

    // Only the revision is negative.
    let error = client.submit(-1, 0, &mut end).unwrap_err();
    assert_eq!(error.code, BridgeCode::BadArgument);
    assert_eq!(client.pending, None);
    assert_eq!(client.next_seq, SEQ_FIRST);

    // Only the index is negative.
    let error = client.submit(1, -1, &mut end).unwrap_err();
    assert_eq!(error.code, BridgeCode::BadArgument);
    assert_eq!(client.pending, None);
    assert_eq!(client.next_seq, SEQ_FIRST);

    // Control: both non-negative are accepted.
    client.submit(1, 0, &mut end).unwrap();
    assert_eq!(client.pending, Some(1));
    assert_eq!(client.next_seq, 2);
}

/// B-A7 check 4: `submit` refuses a stale revision with code 4.
#[test]
fn submit_stale_revision_is_stale_options() {
    let options = vec![
        offered(2, IntentBody::EndTurn),
        offered(2, IntentBody::EndTurn),
    ];
    let link = LocalLink::new(128);
    let mut end = link.client_end();
    let mut client = live();
    push(&link, v4(1, 9, vec![enter(1), enter(2)]));
    client.step(&mut end, &options, false, 0).unwrap();
    assert_eq!(client.view().revision(), 1);

    let error = client.submit(0, 0, &mut end).unwrap_err();
    assert_eq!(error.code, BridgeCode::StaleOptions);
    assert_eq!(client.pending, None);
    assert_eq!(client.next_seq, SEQ_FIRST);

    // Control: the current revision is accepted.
    client.submit(1, 0, &mut end).unwrap();
    assert_eq!(client.pending, Some(1));
    assert_eq!(client.next_seq, 2);
}

/// B-A7 check 5: `submit` refuses while an intent is pending with code 5.
#[test]
fn submit_while_pending_is_intent_pending() {
    let options = vec![
        offered(2, IntentBody::EndTurn),
        offered(2, IntentBody::EndTurn),
    ];
    let link = LocalLink::new(128);
    let mut end = link.client_end();
    let mut client = live();
    push(&link, v4(1, 9, vec![enter(1), enter(2)]));
    client.step(&mut end, &options, false, 0).unwrap();
    client.submit(1, 0, &mut end).unwrap();
    assert_eq!(client.pending, Some(1));
    assert_eq!(client.next_seq, 2);

    // A second submit without a receipt is pending.
    let error = client.submit(1, 1, &mut end).unwrap_err();
    assert_eq!(error.code, BridgeCode::IntentPending);
    assert_eq!(client.pending, Some(1));
    assert_eq!(client.next_seq, 2);

    // Control: after the receipt is stepped, a submit is accepted.
    push(&link, v4(2, 9, vec![receipt(1, ReceiptStatus::Applied)]));
    client.step(&mut end, &options, false, 0).unwrap();
    assert_eq!(client.pending, None);
    client.submit(2, 0, &mut end).unwrap();
    assert_eq!(client.pending, Some(2));
    assert_eq!(client.next_seq, 3);
}

/// B-A7 order 3, 4, 6, 5: two failing checks return the earlier code.
#[test]
fn submit_check_order_is_negative_stale_range_pending() {
    let options = vec![
        offered(2, IntentBody::EndTurn),
        offered(2, IntentBody::EndTurn),
    ];
    let link = LocalLink::new(128);
    let mut end = link.client_end();
    let mut client = live();
    push(&link, v4(1, 9, vec![enter(1), enter(2)]));
    client.step(&mut end, &options, false, 0).unwrap();

    // Negative and stale at once: 3 wins over 4.
    let error = client.submit(-5, 0, &mut end).unwrap_err();
    assert_eq!(error.code, BridgeCode::BadArgument);
    assert_eq!(client.pending, None);
    assert_eq!(client.next_seq, SEQ_FIRST);

    // Stale and out of range at once: 4 wins over 6.
    let error = client.submit(99, 5, &mut end).unwrap_err();
    assert_eq!(error.code, BridgeCode::StaleOptions);
    assert_eq!(client.pending, None);
    assert_eq!(client.next_seq, SEQ_FIRST);

    // Out of range and pending at once: 6 wins over 5.
    client.submit(1, 0, &mut end).unwrap();
    assert_eq!(client.pending, Some(1));
    let error = client.submit(1, 5, &mut end).unwrap_err();
    assert_eq!(error.code, BridgeCode::NoSuchOption);
    assert_eq!(client.pending, Some(1));
    assert_eq!(client.next_seq, 2);
}

/// B-A7 Errors: a lane-0 send onto a full link is 30 `link full`.
#[test]
fn lane0_send_onto_full_link_is_link_full() {
    let options = vec![
        offered(2, IntentBody::EndTurn),
        offered(2, IntentBody::EndTurn),
    ];
    // `submit` onto a full up link.
    let link = LocalLink::new(1);
    let mut end = link.client_end();
    let mut client = live();
    push(&link, v4(1, 9, vec![enter(1), enter(2)]));
    client.step(&mut end, &options, false, 0).unwrap();
    client.move_to(id(1), p(0, 0, 0), &mut end).unwrap();
    assert_eq!(link.queued().0, 1);
    let error = client.submit(1, 0, &mut end).unwrap_err();
    assert_eq!(error.code, BridgeCode::IntentRefused);
    assert_eq!(error.detail, "link full");
    assert_eq!(client.pending, None);
    assert_eq!(client.next_seq, SEQ_FIRST);

    // Control: draining the up queue lets the same `submit` through.
    link.authority_end().recv().expect("one frame");
    client.submit(1, 0, &mut end).unwrap();
    assert_eq!(client.pending, Some(1));
    assert_eq!(client.next_seq, 2);

    // `declare_move` onto a full up link, separately.
    let link = LocalLink::new(1);
    let mut end = link.client_end();
    let mut client = live();
    push(&link, v4(1, 9, vec![enter(1), enter(2)]));
    client.step(&mut end, &options, false, 0).unwrap();
    client.move_to(id(1), p(0, 0, 0), &mut end).unwrap();
    assert_eq!(link.queued().0, 1);
    let error = client
        .declare_move(1, p(32768, 0, 32768), &mut end)
        .unwrap_err();
    assert_eq!(error.code, BridgeCode::IntentRefused);
    assert_eq!(error.detail, "link full");
    assert_eq!(client.pending, None);
    assert_eq!(client.next_seq, SEQ_FIRST);

    // Control: draining lets the same `declare_move` through.
    link.authority_end().recv().expect("one frame");
    client
        .declare_move(1, p(32768, 0, 32768), &mut end)
        .unwrap();
    assert_eq!(client.pending, Some(1));
    assert_eq!(client.next_seq, 2);
}

/// B-A8 (1): `declare_move` accepts revision 0 at revision 0; -1 is 3.
#[test]
fn declare_move_accepts_revision_zero() {
    let link = LocalLink::new(128);
    let mut end = link.client_end();
    let mut client = live();
    assert_eq!(client.view().revision(), 0);
    client.options = vec![offered(2, IntentBody::EndTurn)];
    client
        .declare_move(0, p(32768, 0, 32768), &mut end)
        .unwrap();
    assert_eq!(client.pending, Some(1));
    assert_eq!(client.next_seq, 2);

    // Control: -1 is a negative argument with no other change.
    let link = LocalLink::new(128);
    let mut end = link.client_end();
    let mut control = live();
    control.options = vec![offered(2, IntentBody::EndTurn)];
    let error = control
        .declare_move(-1, p(32768, 0, 32768), &mut end)
        .unwrap_err();
    assert_eq!(error.code, BridgeCode::BadArgument);
    assert_eq!(control.pending, None);
    assert_eq!(control.next_seq, SEQ_FIRST);
}

/// B-A8 (2): `declare_move` with a revision above the view's is 4.
#[test]
fn declare_move_above_the_view_is_stale() {
    let options = vec![
        offered(2, IntentBody::EndTurn),
        offered(2, IntentBody::EndTurn),
    ];
    let link = LocalLink::new(128);
    let mut end = link.client_end();
    let mut client = live();
    push(&link, v4(1, 9, vec![enter(1), enter(2)]));
    client.step(&mut end, &options, false, 0).unwrap();
    assert_eq!(client.view().revision(), 1);

    let error = client
        .declare_move(2, p(32768, 0, 32768), &mut end)
        .unwrap_err();
    assert_eq!(error.code, BridgeCode::StaleOptions);
    assert_eq!(client.pending, None);
    assert_eq!(client.next_seq, SEQ_FIRST);

    // Control: the view's own revision is accepted.
    client
        .declare_move(1, p(32768, 0, 32768), &mut end)
        .unwrap();
    assert_eq!(client.pending, Some(1));
    assert_eq!(client.next_seq, 2);
}

/// B-A8: the declared move carries `options[0]`'s actor.
#[test]
fn declare_move_uses_the_first_option_actor() {
    let options = vec![
        offered(2, IntentBody::EndTurn),
        offered(1, IntentBody::EndTurn),
    ];
    let link = LocalLink::new(128);
    let mut end = link.client_end();
    let mut client = live();
    push(&link, v4(1, 9, vec![enter(1), enter(2)]));
    client.step(&mut end, &options, false, 0).unwrap();
    client
        .declare_move(1, p(32768, 0, 32768), &mut end)
        .unwrap();
    let bytes = link.authority_end().recv().expect("one frame");
    let frame = decode_intent(&bytes).expect("intent decodes");
    assert_eq!(frame.actor, id(2));

    // Control: swapping the options swaps the actor.
    let swapped = vec![
        offered(1, IntentBody::EndTurn),
        offered(2, IntentBody::EndTurn),
    ];
    let link = LocalLink::new(128);
    let mut end = link.client_end();
    let mut control = live();
    push(&link, v4(1, 9, vec![enter(1), enter(2)]));
    control.step(&mut end, &swapped, false, 0).unwrap();
    control
        .declare_move(1, p(32768, 0, 32768), &mut end)
        .unwrap();
    let bytes = link.authority_end().recv().expect("one frame");
    let frame = decode_intent(&bytes).expect("intent decodes");
    assert_eq!(frame.actor, id(1));
}

/// M32 (line 389): a receipt clears `pending` only when its seq equals the
/// pending seq (B-A4). A receipt for n+1 (then n-1) leaves it; the receipt
/// for n clears it.
#[test]
fn receipt_clears_pending_only_on_matching_seq() {
    let options = vec![
        offered(2, IntentBody::EndTurn),
        offered(2, IntentBody::EndTurn),
    ];
    let link = LocalLink::new(128);
    let mut end = link.client_end();
    let mut client = live();
    push(&link, v4(1, 9, vec![enter(1), enter(2)]));
    client.step(&mut end, &options, false, 0).unwrap();
    client.submit(1, 0, &mut end).unwrap();
    assert_eq!(client.pending, Some(1));

    // A receipt for seq 2 (n+1) is applied but leaves seq 1 pending.
    push(&link, v4(2, 9, vec![receipt(2, ReceiptStatus::Applied)]));
    client.step(&mut end, &options, false, 0).unwrap();
    assert_eq!(client.pending, Some(1));

    // Control: the receipt for seq 1 clears it.
    push(&link, v4(3, 9, vec![receipt(1, ReceiptStatus::Applied)]));
    client.step(&mut end, &options, false, 0).unwrap();
    assert_eq!(client.pending, None);

    // The next submit takes seq 2; a receipt for seq 1 (n-1) leaves it.
    client.submit(3, 0, &mut end).unwrap();
    assert_eq!(client.pending, Some(2));
    push(&link, v4(4, 9, vec![receipt(1, ReceiptStatus::Applied)]));
    client.step(&mut end, &options, false, 0).unwrap();
    assert_eq!(client.pending, Some(2));

    // Control: the receipt for seq 2 clears it.
    push(&link, v4(5, 9, vec![receipt(2, ReceiptStatus::Applied)]));
    client.step(&mut end, &options, false, 0).unwrap();
    assert_eq!(client.pending, None);
}

/// M36 (line 398): `Moved` points are each point's raw x, y, z in order
/// (B-A4). Three distinct components per point catch a swap or reversal.
#[test]
fn moved_points_are_each_raw_xyz_in_order() {
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
                    points: vec![p(1, 2, 3), p(4, 5, 6)],
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
            points: vec![1, 2, 3, 4, 5, 6],
        }]
    );
    assert_eq!(
        view.events()[0].entries(),
        vec![
            ("kind", ViewValue::Text("moved")),
            ("entity", ViewValue::Int(1)),
            ("points", ViewValue::Ints(&[1, 2, 3, 4, 5, 6])),
        ]
    );
    drop(view);

    // Control: different components give the different flat list, same order.
    let link = LocalLink::new(128);
    let mut end = link.client_end();
    let mut control = live();
    push(
        &link,
        v4(
            1,
            0,
            vec![
                enter(1),
                DeltaOp::Moved {
                    entity: id(1),
                    points: vec![p(9, 8, 7), p(6, 5, 4)],
                },
            ],
        ),
    );
    control.step(&mut end, &[], false, 0).unwrap();
    assert_eq!(
        control.view().events(),
        &[ViewEvent::Moved {
            entity: 1,
            points: vec![9, 8, 7, 6, 5, 4],
        }]
    );
}

/// M49, M50, M51, M52, M47, M48 (lines 430-486): the v2-built columns in the
/// v4 view (B-A6). Two creatures on different sides, one dead: every column
/// reads exactly as v2's view gives it for the same state.
#[test]
fn v4_view_v2_columns_match_v2_exactly() {
    use crpg_net::codec_v2::encode_delta as encode_v2;
    use crpg_net::protocol_v2::DeltaFrame as DeltaFrameV2;
    use crpg_net::protocol_v2::DeltaOp as DeltaOpV2;

    let options = vec![
        offered(2, IntentBody::EndTurn),
        offered(
            1,
            IntentBody::DeclareAction {
                ability: strike(),
                target: id(2),
            },
        ),
    ];
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
                enter(2),
                DeltaOp::V3(DeltaOpV3::Health {
                    entity: id(1),
                    health: 7,
                    max_health: 9,
                    dead: false,
                }),
                DeltaOp::V3(DeltaOpV3::Health {
                    entity: id(2),
                    health: 0,
                    max_health: 7,
                    dead: true,
                }),
                DeltaOp::V3(DeltaOpV3::Turn {
                    active: Some(id(2)),
                    round: 5,
                }),
            ],
        ),
    );
    client.step(&mut end, &options, false, 0).unwrap();
    let view = client.view();
    assert_eq!(view.net_ids(), &[1, 2]);
    assert_eq!(view.names(), &["Hero".to_string(), "Goblin".to_string()]);
    assert_eq!(view.sides(), &["Player".to_string(), "Monster".to_string()]);
    assert_eq!(
        view.placements(),
        &[
            Ulid::from_u128(14).to_string(),
            Ulid::from_u128(15).to_string(),
        ]
    );
    assert_eq!(view.health(), &[7, 0]);
    assert_eq!(view.max_health(), &[9, 7]);
    assert_eq!(view.health_known(), &[1, 1]);
    assert_eq!(view.dead(), &[0, 1]);
    assert_eq!(view.active(), 2);
    assert_eq!(view.round(), 5);
    assert_eq!(
        view.option_labels(),
        &["End turn".to_string(), "Strike -> Goblin".to_string()]
    );
    assert_eq!(view.option_targets(), &[0, 2]);
    drop(view);

    // The v2 twin of the same state reads every column the same way.
    let v2link = LocalLink::new(128);
    let mut v2end = v2link.client_end();
    let mut v2 = Client::new(epoch(), table());
    push(
        &v2link,
        encode_v2(&DeltaFrameV2 {
            lane: 0,
            server_tick: 0,
            event_seq: 1,
            ops: vec![
                DeltaOpV2::Legacy(LegacyOp::EntityEnter { entity: id(1) }),
                DeltaOpV2::Legacy(LegacyOp::EntityEnter { entity: id(2) }),
                DeltaOpV2::Legacy(LegacyOp::Health {
                    entity: id(1),
                    health: 7,
                    max_health: 9,
                    dead: false,
                }),
                DeltaOpV2::Legacy(LegacyOp::Health {
                    entity: id(2),
                    health: 0,
                    max_health: 7,
                    dead: true,
                }),
                DeltaOpV2::Legacy(LegacyOp::Turn {
                    active: Some(id(2)),
                    round: 5,
                }),
            ],
        })
        .expect("v2 frame encodes"),
    );
    v2.step(&mut v2end, &options, false).unwrap();
    let v2view = v2.view();
    let view = client.view();
    assert_eq!(view.net_ids(), v2view.net_ids());
    assert_eq!(view.names(), v2view.names());
    assert_eq!(view.sides(), v2view.sides());
    assert_eq!(view.placements(), v2view.placements());
    assert_eq!(view.health(), v2view.health());
    assert_eq!(view.max_health(), v2view.max_health());
    assert_eq!(view.health_known(), v2view.health_known());
    assert_eq!(view.dead(), v2view.dead());
    assert_eq!(view.active(), v2view.active());
    assert_eq!(view.round(), v2view.round());
    assert_eq!(view.option_labels(), v2view.option_labels());
    assert_eq!(view.option_targets(), v2view.option_targets());
}

/// M40, M41, M42 (lines 488-490): the revision rises when exactly one of
/// positions, position_known, moving differs (B-A6). Each case changes only
/// that one column; the control changes none and keeps the `Rc`.
#[test]
fn revision_bumps_when_exactly_one_position_column_differs() {
    // Only positions differs: same known and moving, new point.
    let link = LocalLink::new(128);
    let mut end = link.client_end();
    let mut client = live();
    push(
        &link,
        v4(1, 5, vec![enter(1), pos(1, p(65536, 0, 0), false)]),
    );
    client.step(&mut end, &[], false, 0).unwrap();
    let revision = client.view().revision();
    assert_eq!(client.view().positions(), &[65536, 0, 0]);
    push(
        &link,
        v4(
            2,
            6,
            vec![DeltaOp::Position {
                entity: id(1),
                position: Some(PositionFact {
                    point: p(131072, 0, 0),
                    moving: false,
                }),
            }],
        ),
    );
    client.step(&mut end, &[], false, 0).unwrap();
    assert_eq!(client.view().revision(), revision + 1);
    assert_eq!(client.view().positions(), &[131072, 0, 0]);
    assert_eq!(client.view().position_known(), &[1]);
    assert_eq!(client.view().moving(), &[0]);

    // Only position_known differs: the point is already all zeros, so the
    // withdrawal keeps positions and moving.
    let link = LocalLink::new(128);
    let mut end = link.client_end();
    let mut client = live();
    push(&link, v4(1, 5, vec![enter(1), pos(1, p(0, 0, 0), false)]));
    client.step(&mut end, &[], false, 0).unwrap();
    let revision = client.view().revision();
    assert_eq!(client.view().positions(), &[0, 0, 0]);
    assert_eq!(client.view().position_known(), &[1]);
    push(
        &link,
        v4(
            2,
            6,
            vec![DeltaOp::Position {
                entity: id(1),
                position: None,
            }],
        ),
    );
    client.step(&mut end, &[], false, 0).unwrap();
    assert_eq!(client.view().revision(), revision + 1);
    assert_eq!(client.view().positions(), &[0, 0, 0]);
    assert_eq!(client.view().position_known(), &[0]);
    assert_eq!(client.view().moving(), &[0]);

    // Only moving differs: the same point, newly with a route.
    let link = LocalLink::new(128);
    let mut end = link.client_end();
    let mut client = live();
    push(
        &link,
        v4(1, 5, vec![enter(1), pos(1, p(65536, 0, 0), false)]),
    );
    client.step(&mut end, &[], false, 0).unwrap();
    let revision = client.view().revision();
    push(&link, v4(2, 6, vec![pos(1, p(65536, 0, 0), true)]));
    client.step(&mut end, &[], false, 0).unwrap();
    assert_eq!(client.view().revision(), revision + 1);
    assert_eq!(client.view().positions(), &[65536, 0, 0]);
    assert_eq!(client.view().position_known(), &[1]);
    assert_eq!(client.view().moving(), &[1]);

    // Control: the same anchor again changes no column, so no bump and the
    // `Rc` is kept even at a newer tick.
    let link = LocalLink::new(128);
    let mut end = link.client_end();
    let mut client = live();
    push(
        &link,
        v4(1, 5, vec![enter(1), pos(1, p(65536, 0, 0), false)]),
    );
    client.step(&mut end, &[], false, 0).unwrap();
    let held = client.view();
    let revision = held.revision();
    push(&link, v4(2, 6, vec![pos(1, p(65536, 0, 0), false)]));
    client.step(&mut end, &[], false, 0).unwrap();
    assert_eq!(client.view().revision(), revision);
    assert!(Rc::ptr_eq(&held, &client.view()));
}

/// M43 (line 491): an event alone bumps the revision (B-A6). `Died` is a
/// notice, so it changes no column; the control with no event keeps the `Rc`.
#[test]
fn event_alone_bumps_revision() {
    let link = LocalLink::new(128);
    let mut end = link.client_end();
    let mut client = live();
    push(&link, v4(1, 0, vec![enter(1), enter(2)]));
    client.step(&mut end, &[], false, 0).unwrap();
    let revision = client.view().revision();
    assert!(client.view().events().is_empty());

    push(
        &link,
        v4(2, 0, vec![DeltaOp::V3(DeltaOpV3::Died { entity: id(1) })]),
    );
    client.step(&mut end, &[], false, 0).unwrap();
    assert_eq!(client.view().revision(), revision + 1);
    assert_eq!(client.view().events(), &[ViewEvent::Died { entity: 1 }]);
    assert_eq!(client.view().net_ids(), &[1, 2]);
    assert_eq!(client.view().dead(), &[0, 0]);

    // Control: no event and no column change keeps the `Rc`.
    let held = client.view();
    let revision = held.revision();
    client.step(&mut end, &[], false, 0).unwrap();
    assert_eq!(client.view().revision(), revision);
    assert!(Rc::ptr_eq(&held, &client.view()));
}

/// M10 (line 559): any non-codec `TransferError` is `ReplicaRejected` (53)
/// (B-A3). Two buffered deltas with the same seq but different bytes
/// conflict in the pool; the same bytes twice are a harmless duplicate.
#[test]
fn non_codec_transfer_error_is_replica_rejected() {
    let first = v4(1, 5, vec![enter(1)]);
    let second = v4(1, 6, vec![enter(1)]);
    assert_ne!(first, second, "same seq with different bytes must differ");

    let link = LocalLink::new(128);
    let mut end = link.client_end();
    let mut client = ClientV4::new(epoch(), table()).expect("fresh pool admits");
    push(&link, first.clone());
    push(&link, second.clone());
    let before = client.view();
    let error = client.step(&mut end, &[], false, 0).unwrap_err();
    assert_eq!(error.code, BridgeCode::ReplicaRejected);
    assert_eq!(error.code.as_i64(), 53);
    assert_eq!(client.view().revision(), 0);
    assert!(Rc::ptr_eq(&before, &client.view()));
    drop(before);

    // Control: the same bytes twice duplicate without fault and stay at 0.
    let link = LocalLink::new(128);
    let mut end = link.client_end();
    let mut control = ClientV4::new(epoch(), table()).expect("fresh pool admits");
    push(&link, first.clone());
    push(&link, first.clone());
    control.step(&mut end, &[], false, 0).unwrap();
    assert_eq!(control.view().revision(), 0);
}

/// M47 (`client_v4.rs` line 486): the revision rises when only the offered
/// options' labels differ (B-A6). Targets, rows, health, turn and round stay
/// equal; the control with the identical options keeps the `Rc`.
#[test]
fn revision_bumps_when_only_option_labels_differ() {
    let other = Ulid::from_u128(11);
    let other_id = other.to_string();
    let first = vec![
        offered(2, IntentBody::EndTurn),
        offered(
            1,
            IntentBody::DeclareAction {
                ability: strike(),
                target: id(2),
            },
        ),
    ];
    // Same length and the same targets `[0, 2]`; the second label changes
    // from the mapped "Strike -> Goblin" to the unmapped ability's id plus
    // the same target row name.
    let second_label = format!("{other_id} -> Goblin");
    let second = vec![
        offered(2, IntentBody::EndTurn),
        offered(
            1,
            IntentBody::DeclareAction {
                ability: other,
                target: id(2),
            },
        ),
    ];

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
                enter(2),
                DeltaOp::V3(DeltaOpV3::Health {
                    entity: id(1),
                    health: 7,
                    max_health: 9,
                    dead: false,
                }),
                DeltaOp::V3(DeltaOpV3::Health {
                    entity: id(2),
                    health: 0,
                    max_health: 7,
                    dead: true,
                }),
                DeltaOp::V3(DeltaOpV3::Turn {
                    active: Some(id(2)),
                    round: 5,
                }),
            ],
        ),
    );
    client.step(&mut end, &first, false, 0).unwrap();
    assert_eq!(client.view().revision(), 1);
    assert_eq!(
        client.view().option_labels(),
        &["End turn".to_string(), "Strike -> Goblin".to_string()]
    );
    assert_eq!(client.view().option_targets(), &[0, 2]);
    let held = client.view();

    // Only the labels differ: the revision rises by exactly 1.
    client.step(&mut end, &second, false, 0).unwrap();
    assert_eq!(client.view().revision(), 2);
    assert!(!Rc::ptr_eq(&held, &client.view()));
    assert_eq!(
        client.view().option_labels(),
        &["End turn".to_string(), second_label]
    );
    assert_eq!(client.view().option_targets(), &[0, 2]);
    assert_eq!(client.view().net_ids(), &[1, 2]);
    assert_eq!(client.view().health(), &[7, 0]);
    assert_eq!(client.view().max_health(), &[9, 7]);
    assert_eq!(client.view().health_known(), &[1, 1]);
    assert_eq!(client.view().dead(), &[0, 1]);
    assert_eq!(client.view().active(), 2);
    assert_eq!(client.view().round(), 5);
    drop(held);

    // Control: the identical options change no column, so no bump.
    let held = client.view();
    client.step(&mut end, &second, false, 0).unwrap();
    assert_eq!(client.view().revision(), 2);
    assert!(Rc::ptr_eq(&held, &client.view()));
}

/// M48 (`client_v4.rs` line 483): the revision rises when only the active
/// actor differs (B-A6). Round, rows, health and options stay equal; the
/// control with the same active actor keeps the `Rc`.
#[test]
fn revision_bumps_when_only_active_differs() {
    let options = vec![
        offered(2, IntentBody::EndTurn),
        offered(
            1,
            IntentBody::DeclareAction {
                ability: strike(),
                target: id(2),
            },
        ),
    ];

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
                enter(2),
                DeltaOp::V3(DeltaOpV3::Health {
                    entity: id(1),
                    health: 7,
                    max_health: 9,
                    dead: false,
                }),
                DeltaOp::V3(DeltaOpV3::Health {
                    entity: id(2),
                    health: 0,
                    max_health: 7,
                    dead: true,
                }),
                DeltaOp::V3(DeltaOpV3::Turn {
                    active: Some(id(2)),
                    round: 5,
                }),
            ],
        ),
    );
    client.step(&mut end, &options, false, 0).unwrap();
    assert_eq!(client.view().revision(), 1);
    assert_eq!(client.view().active(), 2);
    assert_eq!(client.view().round(), 5);
    let held = client.view();

    // Only the active actor differs: the revision rises by exactly 1.
    push(
        &link,
        v4(
            2,
            1,
            vec![DeltaOp::V3(DeltaOpV3::Turn {
                active: Some(id(1)),
                round: 5,
            })],
        ),
    );
    client.step(&mut end, &options, false, 0).unwrap();
    assert_eq!(client.view().revision(), 2);
    assert!(!Rc::ptr_eq(&held, &client.view()));
    assert_eq!(client.view().active(), 1);
    assert_eq!(client.view().round(), 5);
    assert_eq!(
        client.view().option_labels(),
        &["End turn".to_string(), "Strike -> Goblin".to_string()]
    );
    assert_eq!(client.view().option_targets(), &[0, 2]);
    assert_eq!(client.view().net_ids(), &[1, 2]);
    assert_eq!(client.view().health(), &[7, 0]);
    assert_eq!(client.view().dead(), &[0, 1]);
    drop(held);

    // Control: the same active actor changes no column, so no bump.
    push(
        &link,
        v4(
            3,
            2,
            vec![DeltaOp::V3(DeltaOpV3::Turn {
                active: Some(id(1)),
                round: 5,
            })],
        ),
    );
    let held = client.view();
    client.step(&mut end, &options, false, 0).unwrap();
    assert_eq!(client.view().revision(), 2);
    assert!(Rc::ptr_eq(&held, &client.view()));
}

/// M63b, M64 (`client.rs` lines 337-341): `ViewMoveReceipt::entries()` gives
/// seq, actor, outcome, code, processed_tick, start_tick in that order
/// (Interface). Six distinct values catch any swap; the control differs in
/// every value with the same key order.
#[test]
fn move_receipt_entries_are_in_order() {
    let receipt = ViewMoveReceipt {
        seq: 11,
        actor: 2,
        outcome: "rejected",
        code: 22,
        processed_tick: 7,
        start_tick: 0,
    };
    assert_eq!(
        receipt.entries(),
        vec![
            ("seq", ViewValue::Int(11)),
            ("actor", ViewValue::Int(2)),
            ("outcome", ViewValue::Text("rejected")),
            ("code", ViewValue::Int(22)),
            ("processed_tick", ViewValue::Int(7)),
            ("start_tick", ViewValue::Int(0)),
        ]
    );

    // Control: every value differs, the keys and their order do not.
    let control = ViewMoveReceipt {
        seq: 3,
        actor: 1,
        outcome: "accepted",
        code: 0,
        processed_tick: 9,
        start_tick: 8,
    };
    assert_eq!(
        control.entries(),
        vec![
            ("seq", ViewValue::Int(3)),
            ("actor", ViewValue::Int(1)),
            ("outcome", ViewValue::Text("accepted")),
            ("code", ViewValue::Int(0)),
            ("processed_tick", ViewValue::Int(9)),
            ("start_tick", ViewValue::Int(8)),
        ]
    );
}

// ---------------------------------------------------------------------------
// T157e fix A: the walk open's fact grants (K-1).
// ---------------------------------------------------------------------------

/// `<CARGO_MANIFEST_DIR>/../../campaigns/demo/adventure`.
fn adventure_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("campaigns")
        .join("demo")
        .join("adventure")
}

/// K-1: the walk open grants each player actor its pools and nothing else
/// (no effects, stats or tags); the goblin's row is withheld entirely.
#[test]
fn walk_open_grants_pools_and_nothing_else() {
    use crate::core::authority::v4::rt_tests::{settle, ManualClock};
    use crate::core::{BridgeCore, State};

    let mut core = BridgeCore::new();
    let clock = ManualClock::new();
    let root = adventure_root();
    assert_eq!(
        core.open_v4_walk_with(
            root.to_str().expect("adventure path"),
            "",
            "",
            1,
            clock.clone()
        ),
        BridgeCode::Ok
    );
    let (code, _) = settle(&mut core, &clock, 0);
    assert_eq!(code, BridgeCode::Ok);

    let State::OpenV4(live) = &core.state else {
        panic!("a live v4 session");
    };
    let replica = live.client.replica.v3();
    for raw in [1u64, 2] {
        let row = replica.entity(id(raw)).expect("a hero row");
        assert!(row.facts.pools.is_some(), "hero {raw} pools are granted");
        assert_eq!(row.facts.effects, None, "hero {raw} effects are withheld");
        assert_eq!(row.facts.stats, None, "hero {raw} stats are withheld");
        assert_eq!(row.facts.tags, None, "hero {raw} tags are withheld");
    }
    let goblin = replica.entity(id(3)).expect("the goblin row");
    assert_eq!(goblin.facts.pools, None);
    assert_eq!(goblin.facts.effects, None);
    assert_eq!(goblin.facts.stats, None);
    assert_eq!(goblin.facts.tags, None);
    core.close();
}

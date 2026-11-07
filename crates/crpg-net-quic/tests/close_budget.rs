//! T098g: every `Flush` close stops flushing at `B − R`, then sends the
//! CONNECTION_CLOSE, so the goodbye still fits inside the close budget
//! (D43a). Tests only.

#[path = "support/quic_rig.rs"]
mod quic_rig;

use std::time::{Duration, Instant};

use crpg_net_quic::{
    ClientLimits, CloseCode, CloseMode, CloseReason, SendError, ServerError, ServerLimits,
};
use quic_rig::*;

const INTENT_CAP: usize = 4_096;
const DELTA_CAP: usize = 65_536;

/// Test 3: the D43 regression. A client `Flush` close whose flush outlives
/// its budget still reaches the peer, which sees the peer prefix and then
/// `Peer { code: 0 }` rather than the 60 s idle timeout.
#[test]
fn client_flush_close_over_budget_reaches_peer() {
    let mut server = bind_server(ServerLimits::v1());
    for (stream, budget) in [(1u8, 2_000u32), (2, 100)] {
        let mut limits = ClientLimits::v1();
        limits.close_flush_timeout_ms = budget;
        let (mut client, conn) = connect_accepted(&mut server, limits, 1, epoch(stream));

        for n in 1..=10u32 {
            client
                .try_send(&pattern(stream, n, INTENT_CAP))
                .expect("send");
        }
        for n in 1..=10u32 {
            assert_eq!(
                server_recv(&mut server, conn),
                Ok(pattern(stream, n, INTENT_CAP))
            );
        }
        for n in 11..=128u32 {
            let frame = pattern(stream, n, INTENT_CAP);
            until("queue", || match client.try_send(&frame) {
                Ok(()) => true,
                Err(SendError::QueueFull) => false,
                Err(error) => panic!("send {n}: {error:?}"),
            });
        }

        let report = client.close(CloseCode::Normal, CloseMode::Flush);
        assert_eq!(report.inbound_frames_discarded, 0);
        assert!(
            report.outbound_frames_discarded >= 1,
            "discarded={}",
            report.outbound_frames_discarded
        );

        let mut k = 11usize;
        loop {
            match server_recv(&mut server, conn) {
                Ok(frame) => {
                    assert_eq!(frame, pattern(stream, k as u32, INTENT_CAP));
                    k += 1;
                }
                Err(error) => {
                    assert_eq!(error, ServerError::Closed(CloseReason::Peer { code: 0 }));
                    break;
                }
            }
        }
        assert!(k < 128, "k={k}");
        assert!(
            k + report.outbound_frames_discarded <= 128,
            "k={k} discarded={}",
            report.outbound_frames_discarded
        );
        assert_eq!(
            next_closed(&mut server),
            (conn, CloseReason::Peer { code: 0 })
        );
    }
}

/// A client that absorbs one delta frame and no more, so a server backlog
/// stalls behind QUIC's flow credit (the `quic_gaps` #38 shape).
fn stalled_client() -> ClientLimits {
    let mut limits = ClientLimits::v1();
    limits.inbound_frames = 1;
    limits.inbound_bytes = DELTA_CAP;
    limits.stream_window_bytes = 65_540;
    limits.connection_window_bytes = 65_540;
    limits
}

/// Test 4: `shutdown` also stops flushing at `B − R`, and every client still
/// gets its in-order prefix followed by the `Shutdown` close.
#[test]
fn shutdown_over_budget_reaches_every_client() {
    let mut server = bind_server(ServerLimits::v1());
    let (mut stalled, stalled_conn) = connect_accepted(&mut server, stalled_client(), 1, epoch(1));
    let (mut steady, steady_conn) = connect_accepted(&mut server, ClientLimits::v1(), 1, epoch(2));

    for n in 1..=8u32 {
        server
            .try_send(stalled_conn, &pattern(1, n, DELTA_CAP))
            .expect("send stalled");
    }
    for n in 1..=3u32 {
        server
            .try_send(steady_conn, &pattern(2, n, 1_000))
            .expect("send steady");
    }

    let report = server.shutdown();
    assert_eq!(report.connections_closed, 2);
    assert_eq!(report.inbound_frames_discarded, 0);
    assert!(
        report.outbound_frames_discarded >= 1,
        "discarded={}",
        report.outbound_frames_discarded
    );

    for n in 1..=3u32 {
        assert_eq!(client_recv(&mut steady), Ok(pattern(2, n, 1_000)));
    }
    assert_eq!(client_recv(&mut steady), Err(CloseReason::Peer { code: 1 }));

    let mut k = 0usize;
    loop {
        match client_recv(&mut stalled) {
            Ok(frame) => {
                k += 1;
                assert_eq!(frame, pattern(1, k as u32, DELTA_CAP));
            }
            Err(reason) => {
                assert_eq!(reason, CloseReason::Peer { code: 1 });
                break;
            }
        }
    }
    assert!(k < 8, "k={k}");
    assert!(
        k + report.outbound_frames_discarded <= 8,
        "k={k} discarded={}",
        report.outbound_frames_discarded
    );
}

/// Test 5: a single-connection `QuicServer::close(Flush)` keeps the reserve
/// too (D4, user 2026-11-07), and the peer still sees the close.
#[test]
fn single_close_flush_keeps_the_reserve() {
    let mut server = bind_server(ServerLimits::v1());
    let (mut client, conn) = connect_accepted(&mut server, stalled_client(), 1, epoch(3));

    for n in 1..=8u32 {
        server
            .try_send(conn, &pattern(3, n, DELTA_CAP))
            .expect("send");
    }

    let t0 = Instant::now();
    let report = server
        .close(conn, CloseCode::Normal, CloseMode::Flush)
        .expect("close");
    let took = t0.elapsed();
    assert!(took >= Duration::from_millis(1_800), "took {took:?}");
    assert!(
        report.outbound_frames_discarded >= 1,
        "discarded={}",
        report.outbound_frames_discarded
    );
    assert_eq!(report.inbound_frames_discarded, 0);

    let mut k = 0usize;
    loop {
        match client_recv(&mut client) {
            Ok(frame) => {
                k += 1;
                assert_eq!(frame, pattern(3, k as u32, DELTA_CAP));
            }
            Err(reason) => {
                assert_eq!(reason, CloseReason::Peer { code: 0 });
                break;
            }
        }
    }
    assert!(k < 8, "k={k}");
    assert!(
        k + report.outbound_frames_discarded <= 8,
        "k={k} discarded={}",
        report.outbound_frames_discarded
    );
    assert_eq!(
        next_closed(&mut server),
        (conn, CloseReason::Local(CloseCode::Normal))
    );
}

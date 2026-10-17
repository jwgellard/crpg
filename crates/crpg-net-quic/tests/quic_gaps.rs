//! T120 audit test gaps for crpg-net-quic. Tests only.

#[path = "support/quic_rig.rs"]
mod quic_rig;

use std::time::Duration;

use crpg_net_quic::{
    ClientLimits, CloseCode, CloseMode, ConnectError, QuicClient, SendError, ServerEvent,
    ServerLimits,
};
use quic_rig::*;

const INTENT_CAP: usize = 4_096;
const DELTA_CAP: usize = 65_536;

// #5: a parked inbound reader keeps its frame across a peer close.
#[test]
fn inbound_parked_frame_delivered_before_peer_close() {
    let mut limits = ServerLimits::v1();
    limits.inbound.per_peer_frames = 4;
    let mut server = bind_server(limits);
    let (mut client, conn) = connect_accepted(&mut server, ClientLimits::v1(), 1, epoch(1));

    // Fill to 4 queued + 1 parked.
    for n in 1..=5u32 {
        client.try_send(&pattern(1, n, INTENT_CAP)).expect("send");
    }
    until("4 queued + parked", || {
        let stats = server.stats(conn).expect("stats");
        stats.inbound_frames == 4 && stats.reader_parked
    });

    // Peer closes (Discard) while the frame is parked.
    let _ = client.close(CloseCode::Normal, CloseMode::Discard);

    // Closed must not be published before the parked frame is taken.
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_millis(100) {
        assert!(
            !matches!(server.poll_event(), Some(ServerEvent::Closed { .. })),
            "Closed published before parked frame taken"
        );
        server.wait(Duration::from_millis(10));
    }

    // Queued frames, then the parked 5th, are delivered before Closed.
    for n in 1..=5u32 {
        assert_eq!(
            server_recv(&mut server, conn),
            Ok(pattern(1, n, INTENT_CAP))
        );
    }
    until("closed event", || {
        matches!(server.poll_event(), Some(ServerEvent::Closed { .. }))
    });
}

// #34: host-wide outbound cap binds across connections.
#[test]
fn outbound_host_wide_cap_filled_across_connections() {
    // Frame-count host cap.
    {
        let mut limits = ServerLimits::v1();
        limits.outbound.per_peer_frames = 2;
        limits.outbound.per_peer_bytes = 2 * DELTA_CAP;
        limits.outbound.host_frames = 2;
        limits.outbound.host_bytes = 2 * DELTA_CAP;
        let mut server = bind_server(limits);

        let mut slow = ClientLimits::v1();
        slow.inbound_frames = 1;
        slow.inbound_bytes = DELTA_CAP;
        slow.stream_window_bytes = 65_540;
        slow.connection_window_bytes = 65_540;
        let (mut client_a, conn_a) = connect_accepted(&mut server, slow, 1, epoch(1));
        let (mut client_b, conn_b) = connect_accepted(&mut server, slow, 1, epoch(2));

        // Fill host cap using A only.
        let mut accepted = 0u32;
        loop {
            match server.try_send(conn_a, &pattern(9, accepted + 1, DELTA_CAP)) {
                Ok(()) => accepted += 1,
                Err(SendError::QueueFull) => break,
                Err(e) => panic!("{:?}", e),
            }
            if accepted > 100 {
                panic!("did not hit host cap");
            }
        }
        assert!(accepted >= 1);

        // B is refused on the host cap while its own per-connection queue is empty.
        assert_eq!(server.stats(conn_b).expect("stats").outbound_frames, 0);
        assert_eq!(
            server.try_send(conn_b, &pattern(9, 99, DELTA_CAP)),
            Err(SendError::QueueFull)
        );
        assert_eq!(server.stats(conn_b).expect("stats").outbound_frames, 0);

        // Drain A, then B is admitted.
        for n in 1..=accepted {
            assert_eq!(client_recv(&mut client_a), Ok(pattern(9, n, DELTA_CAP)));
        }
        until("a drained", || {
            server.stats(conn_a).expect("stats").outbound_frames == 0
        });
        server
            .try_send(conn_b, &pattern(9, 99, DELTA_CAP))
            .expect("b admitted");
        assert_eq!(client_recv(&mut client_b), Ok(pattern(9, 99, DELTA_CAP)));
    }

    // Byte-count host cap (frames cap loose).
    {
        let mut limits = ServerLimits::v1();
        limits.outbound.per_peer_frames = 10;
        limits.outbound.per_peer_bytes = 2 * DELTA_CAP;
        limits.outbound.host_frames = 10;
        limits.outbound.host_bytes = 2 * DELTA_CAP;
        let mut server = bind_server(limits);

        let mut slow = ClientLimits::v1();
        slow.inbound_frames = 1;
        slow.inbound_bytes = DELTA_CAP;
        slow.stream_window_bytes = 65_540;
        slow.connection_window_bytes = 65_540;
        let (mut client_a, conn_a) = connect_accepted(&mut server, slow, 1, epoch(3));
        let (mut client_b, conn_b) = connect_accepted(&mut server, slow, 1, epoch(4));

        let mut accepted = 0u32;
        loop {
            match server.try_send(conn_a, &pattern(9, accepted + 1, DELTA_CAP)) {
                Ok(()) => accepted += 1,
                Err(SendError::QueueFull) => break,
                Err(e) => panic!("{:?}", e),
            }
            if accepted > 100 {
                panic!("did not hit host cap");
            }
        }
        assert!(accepted >= 1);

        assert_eq!(server.stats(conn_b).expect("stats").outbound_frames, 0);
        assert_eq!(
            server.try_send(conn_b, &pattern(9, 99, DELTA_CAP)),
            Err(SendError::QueueFull)
        );
        assert_eq!(server.stats(conn_b).expect("stats").outbound_frames, 0);

        for n in 1..=accepted {
            assert_eq!(client_recv(&mut client_a), Ok(pattern(9, n, DELTA_CAP)));
        }
        until("a drained", || {
            server.stats(conn_a).expect("stats").outbound_frames == 0
        });
        server
            .try_send(conn_b, &pattern(9, 99, DELTA_CAP))
            .expect("b admitted");
        assert_eq!(client_recv(&mut client_b), Ok(pattern(9, 99, DELTA_CAP)));
    }
}

// #35: a pending connection holds a connection-limit slot.
#[test]
fn pending_connection_holds_connection_limit_slot() {
    let mut limits = ServerLimits::v1();
    limits.max_connections = 1;
    let mut server = bind_server(limits);

    // First client reaches HelloReceived but is not accepted.
    let pending = connect_async(
        client_config(ClientLimits::v1()),
        server.local_addr(),
        hello(1),
    );
    let (conn, _, _) = next_hello(&mut server);
    assert_eq!(server.totals().pending, 1);
    assert_eq!(server.totals().connections, 1);

    // A further client is refused at the limit.
    let third = QuicClient::connect(
        client_config(ClientLimits::v1()),
        server.local_addr(),
        &hello(1),
    );
    assert_eq!(third.err(), Some(ConnectError::EndpointFull));
    until("refused at limit", || server.totals().refused_at_limit == 1);

    // Cleanup.
    server.refuse(conn, CloseCode::AuthRefused).expect("refuse");
    let _ = pending.join().expect("thread");
}

// #36: a connection failing before Hello releases its slot at once.
#[test]
fn connection_failing_before_hello_releases_slot() {
    let mut limits = ServerLimits::v1();
    limits.max_connections = 1;
    let mut server = bind_server(limits);

    // A foreign-ALPN connection fails before HelloReceived.
    let raw = raw_connect(server.local_addr(), b"crpg-lane0/2");
    assert!(raw.is_err(), "foreign ALPN fails");
    until("refused handshake counted", || {
        server.totals().refused_handshakes == 1
    });
    assert_eq!(server.totals().connections, 0);

    // A normal client can now connect.
    let (_client, conn) = connect_accepted(&mut server, ClientLimits::v1(), 1, epoch(6));
    assert_eq!(conn.get(), 1);
}

// #37: client connect timeout.
#[test]
fn client_connect_timeout_returns_timed_out() {
    let socket = std::net::UdpSocket::bind("127.0.0.1:0").expect("bind");
    let addr = socket.local_addr().expect("addr");
    // Keep socket alive but never answer.
    let mut limits = ClientLimits::v1();
    limits.connect_timeout_ms = 100;
    let result = QuicClient::connect(client_config(limits), addr, &hello(1));
    assert_eq!(result.err(), Some(ConnectError::TimedOut));
    drop(socket);
}

// #38: graceful close exceeding flush timeout reports discard counts.
#[test]
fn graceful_close_exceeding_flush_timeout_reports_discard() {
    let mut limits = ServerLimits::v1();
    limits.close_flush_timeout_ms = 100;
    let mut server = bind_server(limits);

    let mut client_limits = ClientLimits::v1();
    client_limits.inbound_frames = 1;
    client_limits.inbound_bytes = DELTA_CAP;
    client_limits.stream_window_bytes = 65_540;
    client_limits.connection_window_bytes = 65_540;
    let (_client, conn) = connect_accepted(&mut server, client_limits, 1, epoch(8));

    // Queue many frames; the client stops reading after the first.
    for n in 1..=8u32 {
        let _ = server.try_send(conn, &pattern(5, n, DELTA_CAP));
    }

    let report = server
        .close(conn, CloseCode::Normal, CloseMode::Flush)
        .expect("close");
    assert!(
        report.outbound_frames_discarded > 0,
        "discarded={}",
        report.outbound_frames_discarded
    );
    assert_eq!(report.inbound_frames_discarded, 0);
}

// #39: shutdown with parked inbound frames reports discarded count.
#[test]
fn shutdown_with_parked_inbound_frames_reports_discarded() {
    let mut limits = ServerLimits::v1();
    limits.inbound.per_peer_frames = 2;
    let mut server = bind_server(limits);
    let (mut client, conn) = connect_accepted(&mut server, ClientLimits::v1(), 1, epoch(9));

    for n in 1..=3u32 {
        client.try_send(&pattern(1, n, INTENT_CAP)).expect("send");
    }
    until("2 queued + parked", || {
        let stats = server.stats(conn).expect("stats");
        stats.inbound_frames == 2 && stats.reader_parked
    });

    let report = server.shutdown();
    assert_eq!(report.connections_closed, 1);
    // Queued (2) + parked (1) = 3.
    assert_eq!(
        report.inbound_frames_discarded, 3,
        "discarded={}",
        report.inbound_frames_discarded
    );
}

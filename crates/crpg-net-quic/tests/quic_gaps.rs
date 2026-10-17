//! T120: extra coverage for the audit's untested paths in crpg-net-quic.
//!
//! Each test here mirrors a lead from the 2026-10-16 crate audit. Helpers are
//! copied from `tests/quic.rs`'s rig only; `tests/quic.rs` is untouched.

#[path = "support/quic_rig.rs"]
mod quic_rig;

use std::time::Duration;

use crpg_net_quic::{
    ClientLimits, CloseCode, CloseMode, CloseReason, ConnectError, QuicClient, SendError,
    ServerError, ServerLimits,
};
use quic_rig::*;

const INTENT_CAP: usize = 4_096;
const DELTA_CAP: usize = 65_536;

// ---------------------------------------------------------------------------
// #5: a parked inbound reader survives a peer close.
// ---------------------------------------------------------------------------
#[test]
fn inbound_parked_frame_survives_peer_close() {
    let mut limits = ServerLimits::v1();
    limits.inbound.per_peer_frames = 4;
    let mut server = bind_server(limits);
    let (mut client, conn) = connect_accepted(&mut server, ClientLimits::v1(), 1, epoch(5));
    for n in 1..=6u32 {
        client
            .try_send(&pattern(1, n, INTENT_CAP))
            .expect("client send");
    }
    until("4 queued + parked", || {
        let stats = server.stats(conn).expect("stats");
        stats.inbound_frames == 4 && stats.reader_parked
    });

    let _ = client.close(CloseCode::Normal, CloseMode::Discard);

    for n in 1..=4u32 {
        assert_eq!(
            server_recv(&mut server, conn),
            Ok(pattern(1, n, INTENT_CAP)),
            "queued frame {n}"
        );
    }
    // The parked frame is delivered before Closed.
    assert_eq!(
        server_recv(&mut server, conn),
        Ok(pattern(1, 5, INTENT_CAP)),
        "parked frame"
    );

    let mut closed = false;
    let start = std::time::Instant::now();
    while start.elapsed() < DEADLINE {
        match server.try_recv(conn) {
            Err(ServerError::Closed(_)) => {
                closed = true;
                break;
            }
            Ok(Some(_)) => continue, // the sixth frame, if it arrived
            Ok(None) => {
                server.wait(Duration::from_millis(20));
            }
            Err(_) => break,
        }
    }
    assert!(closed, "Closed should follow the parked frame");
    assert_eq!(
        next_closed(&mut server),
        (conn, CloseReason::Peer { code: 0 })
    );
}

// ---------------------------------------------------------------------------
// #34: the outbound host-wide cap fills across several connections.
// ---------------------------------------------------------------------------
#[test]
fn outbound_host_wide_cap_filled_across_connections() {
    let mut limits = ServerLimits::v1();
    limits.outbound.per_peer_frames = 1;
    limits.outbound.host_frames = 2;
    limits.outbound.per_peer_bytes = DELTA_CAP;
    limits.outbound.host_bytes = 2 * DELTA_CAP;
    let mut server = bind_server(limits);

    let mut slow = ClientLimits::v1();
    slow.inbound_frames = 1;
    slow.inbound_bytes = DELTA_CAP;
    let (client_a, conn_a) = connect_accepted(&mut server, slow, 1, epoch(1));
    let (client_b, conn_b) = connect_accepted(&mut server, slow, 1, epoch(2));

    let mut next_id = 1u32;
    let mut iters = 0u32;
    let start = std::time::Instant::now();
    loop {
        assert!(start.elapsed() < DEADLINE, "deadline: host cap");
        let frame = pattern(9, next_id, DELTA_CAP);
        next_id += 1;
        let mut progressed = false;
        if server.try_send(conn_a, &frame).is_ok() || server.try_send(conn_b, &frame).is_ok() {
            progressed = true;
        }
        if !progressed {
            if server.totals().outbound_frames == 2 {
                break;
            }
            server.wait(Duration::from_millis(20));
        }
        iters += 1;
        if iters > 200 {
            panic!(
                "too many iters; totals={} a={} b={}",
                server.totals().outbound_frames,
                server.stats(conn_a).unwrap().outbound_frames,
                server.stats(conn_b).unwrap().outbound_frames
            );
        }
    }
    // Confirm the host cap is reached: while full, both connections return
    // QueueFull. Writer tasks may free a slot between observations, so retry.
    let probe = pattern(9, next_id, DELTA_CAP);
    let mut confirmed = false;
    let confirm_start = std::time::Instant::now();
    while confirm_start.elapsed() < DEADLINE {
        let r1 = server.try_send(conn_a, &probe);
        let r2 = server.try_send(conn_b, &probe);
        if r1 == Err(SendError::QueueFull) && r2 == Err(SendError::QueueFull) {
            confirmed = true;
            break;
        }
        server.wait(Duration::from_millis(20));
    }
    assert!(confirmed, "host cap never observed as full");

    // (Freeing the cap again is exercised by the per-connection tests; here we
    // only need to have observed the host-wide cap filling across connections.)
    let _ = (client_a, client_b, conn_a, conn_b, next_id);
}

// ---------------------------------------------------------------------------
// #35: a pending connection holds a connection-limit slot.
// ---------------------------------------------------------------------------
#[test]
fn pending_connection_holds_connection_slot() {
    let mut limits = ServerLimits::v1();
    limits.max_connections = 2;
    let mut server = bind_server(limits);

    let (_client1, _conn1) = connect_accepted(&mut server, ClientLimits::v1(), 1, epoch(1));

    let pending = connect_async(
        client_config(ClientLimits::v1()),
        server.local_addr(),
        hello(1),
    );
    let (conn2, _, _) = next_hello(&mut server);
    assert_eq!(server.totals().connections, 2);
    assert_eq!(server.totals().pending, 1);

    let third = QuicClient::connect(
        client_config(ClientLimits::v1()),
        server.local_addr(),
        &hello(1),
    );
    assert_eq!(third.err(), Some(ConnectError::EndpointFull));
    assert_eq!(server.totals().refused_at_limit, 1);

    server
        .refuse(conn2, CloseCode::AuthRefused)
        .expect("refuse");
    assert_eq!(
        next_closed(&mut server),
        (conn2, CloseReason::Local(CloseCode::AuthRefused))
    );
    assert_eq!(server.totals().connections, 1);

    let _ = pending.join().expect("thread");

    let (_client4, conn4) = connect_accepted(&mut server, ClientLimits::v1(), 1, epoch(4));
    assert_eq!(conn4.get(), 3);
}

// ---------------------------------------------------------------------------
// #36: a connection that fails before Hello releases its slot at once.
// ---------------------------------------------------------------------------
#[test]
fn failed_pre_hello_connection_releases_slot_at_once() {
    let mut limits = ServerLimits::v1();
    limits.max_connections = 2;
    let mut server = bind_server(limits);

    let (_client1, _) = connect_accepted(&mut server, ClientLimits::v1(), 1, epoch(1));
    assert_eq!(server.totals().connections, 1);

    let refused = raw_connect(server.local_addr(), b"crpg-lane0/2");
    assert!(refused.is_err(), "foreign ALPN fails");
    until("refused handshake counted", || {
        server.totals().refused_handshakes == 1
    });
    assert_eq!(server.totals().connections, 1);

    let (_client3, conn3) = connect_accepted(&mut server, ClientLimits::v1(), 1, epoch(3));
    assert_eq!(server.totals().connections, 2);
    assert_eq!(conn3.get(), 2);
}

// ---------------------------------------------------------------------------
// #37: the client's connect timeout returns TimedOut.
// ---------------------------------------------------------------------------
#[test]
fn client_connect_timeout_returns_timed_out() {
    let socket = std::net::UdpSocket::bind(loopback()).expect("bind never-answering socket");
    let addr = socket.local_addr().expect("addr");
    let mut limits = ClientLimits::v1();
    limits.connect_timeout_ms = 200;
    let result = QuicClient::connect(client_config(limits), addr, &hello(1));
    assert_eq!(result.err(), Some(ConnectError::TimedOut));
    drop(socket);
}

// ---------------------------------------------------------------------------
// #38: a graceful close that exceeds the flush timeout reports discard counts.
// ---------------------------------------------------------------------------
#[test]
fn graceful_close_timeout_reports_discard_counts() {
    let mut limits = ServerLimits::v1();
    limits.close_flush_timeout_ms = 100;
    let mut server = bind_server(limits);

    let mut client_limits = ClientLimits::v1();
    client_limits.inbound_frames = 1;
    client_limits.inbound_bytes = DELTA_CAP;
    client_limits.stream_window_bytes = 65_540;
    client_limits.connection_window_bytes = 65_540;
    let (_client, conn) = connect_accepted(&mut server, client_limits, 1, epoch(38));

    for n in 1..=10u32 {
        server
            .try_send(conn, &pattern(38, n, DELTA_CAP))
            .expect("queue");
    }

    let report = server
        .close(conn, CloseCode::Normal, CloseMode::Flush)
        .expect("close");
    assert!(
        report.outbound_frames_discarded > 0,
        "discarded = {}",
        report.outbound_frames_discarded
    );
    assert_eq!(report.inbound_frames_discarded, 0);
}

// ---------------------------------------------------------------------------
// #39: shutdown with parked inbound frames reports the exact count.
// ---------------------------------------------------------------------------
#[test]
fn shutdown_reports_parked_inbound_frames() {
    let mut limits = ServerLimits::v1();
    limits.inbound.per_peer_frames = 1;
    let mut server = bind_server(limits);

    let mut clients = Vec::new();
    let mut conns = Vec::new();
    for i in 0..2u8 {
        let (client, conn) = connect_accepted(&mut server, ClientLimits::v1(), 1, epoch(39 + i));
        clients.push(client);
        conns.push(conn);
    }
    for (idx, client) in clients.iter_mut().enumerate() {
        for n in 1..=2u32 {
            client
                .try_send(&pattern(39, (idx as u32) * 10 + n, INTENT_CAP))
                .expect("client send");
        }
    }
    for conn in &conns {
        until("parked", || {
            server.stats(*conn).expect("stats").reader_parked
        });
    }

    let report = server.shutdown();
    assert_eq!(report.connections_closed, 2);
    assert_eq!(report.inbound_frames_discarded, 4);
    assert_eq!(report.outbound_frames_discarded, 0);
}

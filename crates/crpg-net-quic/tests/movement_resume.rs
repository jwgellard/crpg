//! T155b suite: the resumable movement endpoint (`bind_movement_resumable`,
//! `connect_movement_resume`, and the lane-1 claim in `accept_resume`), cases
//! R1–R4 of `tasks/T155.md`, through the public `crpg_net_quic` API over real
//! QUIC on IPv4 loopback.
//!
//! Waits are the rig's 30 s failure guards, never oracles. Every refusal has
//! a positive control on the same endpoint kind.

#[path = "support/movement_rig.rs"]
mod movement_rig;
#[path = "support/quic_rig.rs"]
mod quic_rig;

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use crpg_net_quic::{
    Channel, ClientLimits, CloseCode, CloseMode, CloseReason, ConfigError, ConnectError,
    ConnectionId, Framing, QuicClient, QuicServer, ServerConfig, ServerError, ServerLimits,
    ServerNotice, Welcome,
};
use movement_rig::*;
use quic_rig::*;

/// The session a resumed movement connection carries (wire version 4).
const SESSION4: Welcome = Welcome {
    wire_version: 4,
    epoch: [7; 16],
};

/// A resume request body: the transport reads neither header nor payload.
const REQ: [u8; 7] = [0x91, 0x03, 0x40, 1, 2, 3, 4];

/// A resume answer body: the transport does not decode it either.
const ANS: [u8; 4] = [0x91, 0x03, 0x31, 9];

type ResumeJoin = std::thread::JoinHandle<Result<(QuicClient, Vec<u8>), ConnectError>>;

fn bind_movement_resumable_server(limits: ServerLimits) -> QuicServer {
    QuicServer::bind_movement_resumable(ServerConfig {
        bind: loopback(),
        identity: identity_a(),
        limits,
    })
    .expect("bind movement resumable server")
}

fn bind_resumable_server(limits: ServerLimits) -> QuicServer {
    QuicServer::bind_resumable(ServerConfig {
        bind: loopback(),
        identity: identity_a(),
        limits,
    })
    .expect("bind resumable server")
}

/// `connect_movement_resume` on its own thread.
fn connect_movement_resume_async(
    limits: ClientLimits,
    server: SocketAddr,
    session: Welcome,
    request: Vec<u8>,
) -> ResumeJoin {
    std::thread::spawn(move || {
        QuicClient::connect_movement_resume(client_config(limits), server, session, &request)
    })
}

/// `connect_resume` (`/2`) on its own thread.
fn connect_resume_async(
    limits: ClientLimits,
    server: SocketAddr,
    session: Welcome,
    request: Vec<u8>,
) -> ResumeJoin {
    std::thread::spawn(move || {
        QuicClient::connect_resume(client_config(limits), server, session, &request)
    })
}

/// Guard: the server's next notice.
fn next_notice(server: &mut QuicServer) -> ServerNotice {
    let start = Instant::now();
    loop {
        if let Some(notice) = server.poll_notice() {
            return notice;
        }
        assert!(start.elapsed() < DEADLINE, "deadline: notice");
        server.wait(Duration::from_millis(20));
    }
}

/// Guard: the next notice, which must be a `Resume`; returns its connection
/// and request bytes.
fn next_resume(server: &mut QuicServer) -> (ConnectionId, Vec<u8>) {
    match next_notice(server) {
        ServerNotice::Resume(received) => (received.conn, received.request),
        other => panic!("expected Resume, got {other:?}"),
    }
}

/// Guard: the client's next lane-0 frame with its channel.
fn client_channel_next(client: &mut QuicClient) -> (Channel, Vec<u8>) {
    let start = Instant::now();
    loop {
        match client.try_recv_channel() {
            Ok(Some(next)) => return next,
            Ok(None) => {}
            Err(reason) => panic!("client closed: {reason:?}"),
        }
        assert!(start.elapsed() < DEADLINE, "deadline: client channel frame");
        client.wait(Duration::from_millis(20));
    }
}

/// Guard: the server's next lane-1 frame, or its error.
fn server_lane1_next(server: &mut QuicServer, conn: ConnectionId) -> Result<Vec<u8>, ServerError> {
    let start = Instant::now();
    loop {
        match server.try_recv_lane1(conn) {
            Ok(Some(frame)) => return Ok(frame),
            Ok(None) => {}
            Err(error) => return Err(error),
        }
        assert!(start.elapsed() < DEADLINE, "deadline: server lane-1 frame");
        server.wait(Duration::from_millis(20));
    }
}

/// The R1 opening: a movement resume client against `server`, the server side
/// checked and accepted with `ANS`. Returns the client and its connection.
fn resumed_movement(server: &mut QuicServer) -> (QuicClient, ConnectionId) {
    let pending = connect_movement_resume_async(
        ClientLimits::v1(),
        server.local_addr(),
        SESSION4,
        REQ.to_vec(),
    );
    let (conn, request) = next_resume(server);
    assert_eq!(request, REQ.to_vec());
    assert_eq!(server.framing(conn), Ok(Framing::V2));
    assert_eq!(server.movement(conn), Ok(true));
    assert_eq!(server.accept_resume(conn, &ANS), Ok(()));
    let (client, answer) = pending
        .join()
        .expect("client thread")
        .expect("connect_movement_resume");
    assert_eq!(answer, ANS.to_vec());
    (client, conn)
}

/// A hello client on the same endpoint (R1's control): `hello4(fill)` is
/// received and accepted with version 4.
fn hello_movement(server: &mut QuicServer, fill: u8) -> (QuicClient, ConnectionId) {
    let pending = connect_movement_async(ClientLimits::v1(), server.local_addr(), hello4(fill));
    let (conn, _, hello) = next_hello(server);
    assert_eq!(hello.wire_version, 4);
    assert_eq!(server.movement(conn), Ok(true));
    assert_eq!(
        server.accept(
            conn,
            Welcome {
                wire_version: 4,
                epoch: epoch(3),
            }
        ),
        Ok(())
    );
    (pending.join().expect("thread").expect("connect"), conn)
}

/// Every lane works on `client` / `conn`: lane 1 both ways, a datagram and
/// lane 0 downstream.
fn assert_all_lanes_work(server: &mut QuicServer, conn: ConnectionId, client: &mut QuicClient) {
    client.try_send_lane1(&h(1)).expect("lane-1 up");
    assert_eq!(server_recv_lane1(server, conn), h(1));
    server.try_send_lane1(conn, &r1()).expect("lane-1 down");
    assert_eq!(client_recv_lane1(client), r1());
    assert_eq!(
        datagram_until(server, conn, client, &[1, 2, 3]),
        vec![1, 2, 3]
    );
    server
        .try_send_channel(conn, Channel::Lane0, &[5])
        .expect("lane-0 down");
    assert_eq!(client_channel_next(client), (Channel::Lane0, vec![5]));
}

// ---------------------------------------------------------------------------
// R1
// ---------------------------------------------------------------------------

#[test]
fn r1_resume_opening_on_the_resumable_movement_endpoint() {
    let mut server = bind_movement_resumable_server(ServerLimits::v1());
    let (mut client, conn) = resumed_movement(&mut server);
    assert!(client.movement());
    assert_eq!(client.framing(), Framing::V2);
    assert_eq!(client.welcome(), SESSION4);
    assert_all_lanes_work(&mut server, conn, &mut client);

    // Control: the same endpoint takes a `hello4` and that client's lanes
    // work the same way.
    let (mut hello_client, hello_conn) = hello_movement(&mut server, 1);
    assert!(hello_client.movement());
    assert_eq!(hello_client.framing(), Framing::V2);
    assert_all_lanes_work(&mut server, hello_conn, &mut hello_client);
}

// ---------------------------------------------------------------------------
// R2
// ---------------------------------------------------------------------------

#[test]
fn r2_refusals_close_with_their_code() {
    let mut server = bind_movement_resumable_server(ServerLimits::v1());
    for (close, code) in [(CloseCode::AuthRefused, 5u64), (CloseCode::ServerBusy, 7)] {
        let pending = connect_movement_resume_async(
            ClientLimits::v1(),
            server.local_addr(),
            SESSION4,
            REQ.to_vec(),
        );
        let (conn, request) = next_resume(&mut server);
        assert_eq!(request, REQ.to_vec());
        assert_eq!(server.refuse(conn, close), Ok(()));
        let result = pending.join().expect("client thread");
        assert_eq!(result.err(), Some(ConnectError::Refused { code }));
        assert_eq!(next_closed(&mut server), (conn, CloseReason::Local(close)));
    }
}

// ---------------------------------------------------------------------------
// R3
// ---------------------------------------------------------------------------

/// A client of the wrong kind against `server` fails TLS: `Handshake`, one
/// refused handshake counted, and no notice.
fn assert_kind_refused(
    server: &mut QuicServer,
    result: Result<(QuicClient, Vec<u8>), ConnectError>,
) {
    assert_eq!(result.err(), Some(ConnectError::Handshake));
    until("the TLS failure is counted", || {
        server.totals().refused_handshakes == 1
    });
    assert_eq!(server.totals().refused_handshakes, 1);
    assert!(
        server.poll_notice().is_none(),
        "no notice for a TLS failure"
    );
}

#[test]
fn r3_endpoint_kinds_and_early_refusals() {
    // (a) the movement resume client against a `/2` resumable endpoint.
    let mut combat = bind_resumable_server(ServerLimits::v1());
    let result = QuicClient::connect_movement_resume(
        client_config(ClientLimits::v1()),
        combat.local_addr(),
        SESSION4,
        &REQ,
    );
    assert_kind_refused(&mut combat, result);
    // Control: that server takes its own client kind.
    let session2 = Welcome {
        wire_version: 2,
        epoch: [7; 16],
    };
    let pending = connect_resume_async(
        ClientLimits::v1(),
        combat.local_addr(),
        session2,
        REQ.to_vec(),
    );
    let (conn, request) = next_resume(&mut combat);
    assert_eq!(request, REQ.to_vec());
    assert_eq!(combat.movement(conn), Ok(false));
    assert_eq!(combat.accept_resume(conn, &ANS), Ok(()));
    let (client, answer) = pending.join().expect("thread").expect("connect_resume");
    assert_eq!(answer, ANS.to_vec());
    assert!(!client.movement());

    // (b) the `/2` resume client against the resumable movement endpoint.
    let mut movement = bind_movement_resumable_server(ServerLimits::v1());
    let result = QuicClient::connect_resume(
        client_config(ClientLimits::v1()),
        movement.local_addr(),
        session2,
        &REQ,
    );
    assert_kind_refused(&mut movement, result);
    // Control: that server takes its own client kind.
    let (client, _conn) = resumed_movement(&mut movement);
    assert!(client.movement());

    // (c) the early refusals never reach the network.
    let before = movement.totals();
    let config = client_config(ClientLimits::v1());
    let addr = movement.local_addr();
    let bad_session = Welcome {
        wire_version: 2,
        epoch: [7; 16],
    };
    assert_eq!(
        QuicClient::connect_movement_resume(config.clone(), addr, bad_session, &REQ).err(),
        Some(ConnectError::Handshake)
    );
    assert_eq!(
        QuicClient::connect_movement_resume(config.clone(), addr, SESSION4, &[]).err(),
        Some(ConnectError::Handshake)
    );
    assert_eq!(
        QuicClient::connect_movement_resume(config.clone(), addr, SESSION4, &vec![0x5A; 4_097])
            .err(),
        Some(ConnectError::Handshake)
    );
    let mut bad_limits = ClientLimits::v1();
    bad_limits.inbound_frames = 0;
    let bad_config = crpg_net_quic::ClientConfig {
        limits: bad_limits,
        ..config
    };
    assert_eq!(
        QuicClient::connect_movement_resume(bad_config, addr, SESSION4, &REQ).err(),
        Some(ConnectError::Config(ConfigError::InvalidLimits {
            field: "inbound_frames"
        }))
    );
    let after = movement.totals();
    assert_eq!(after.refused_handshakes, before.refused_handshakes);
    assert_eq!(after.connections, before.connections);
    assert!(movement.poll_notice().is_none(), "no notice before any I/O");
}

// ---------------------------------------------------------------------------
// R4
// ---------------------------------------------------------------------------

#[test]
fn r4_a_resumed_movement_connection_keeps_its_lane1_frames() {
    let mut server = bind_movement_resumable_server(ServerLimits::v1());
    let (mut client, conn) = resumed_movement(&mut server);
    // No priming and no waiting: the frames and the close leave back to back,
    // so a close published before the lane-1 reader queued them loses frames.
    client.try_send_lane1(&h(1)).expect("h1");
    client.try_send_lane1(&h(2)).expect("h2");
    client.try_send_lane1(&h(3)).expect("h3");
    client.close(CloseCode::Normal, CloseMode::Flush);
    assert_eq!(
        next_closed(&mut server),
        (conn, CloseReason::Peer { code: 0 })
    );
    assert_eq!(server_lane1_next(&mut server, conn), Ok(h(1)));
    assert_eq!(server_lane1_next(&mut server, conn), Ok(h(2)));
    assert_eq!(server_lane1_next(&mut server, conn), Ok(h(3)));
    assert_eq!(
        server_lane1_next(&mut server, conn),
        Err(ServerError::Closed(CloseReason::Peer { code: 0 }))
    );
}

// ---------------------------------------------------------------------------
// T155b fix round A: the N-4 (3) boundary and the resumed framing
// ---------------------------------------------------------------------------

/// Guard: the server's next lane-0 frame with its channel, or its error.
fn server_channel_next(server: &mut QuicServer, conn: ConnectionId) -> (Channel, Vec<u8>) {
    let start = Instant::now();
    loop {
        match server.try_recv_channel(conn) {
            Ok(Some(next)) => return next,
            Ok(None) => {}
            Err(error) => panic!("server closed: {error:?}"),
        }
        assert!(start.elapsed() < DEADLINE, "deadline: server channel frame");
        server.wait(Duration::from_millis(20));
    }
}

#[test]
fn r5_the_request_cap_is_inclusive() {
    // N-4 (3): a request is refused when *longer than* 4,096 bytes; exactly
    // 4,096 bytes is legal and reaches the server whole.
    let mut server = bind_movement_resumable_server(ServerLimits::v1());
    let at_cap = vec![0x5A; 4_096];
    let pending = connect_movement_resume_async(
        ClientLimits::v1(),
        server.local_addr(),
        SESSION4,
        at_cap.clone(),
    );
    let (conn, request) = next_resume(&mut server);
    assert_eq!(request.len(), 4_096);
    assert_eq!(request, at_cap);
    assert_eq!(server.accept_resume(conn, &ANS), Ok(()));
    let (client, answer) = pending
        .join()
        .expect("client thread")
        .expect("a 4,096-byte request is accepted");
    assert_eq!(answer, ANS.to_vec());
    assert!(client.movement());

    // Control: one byte more is refused before any I/O.
    let before = server.totals();
    let over = vec![0x5A; 4_097];
    assert_eq!(
        QuicClient::connect_movement_resume(
            client_config(ClientLimits::v1()),
            server.local_addr(),
            SESSION4,
            &over,
        )
        .err(),
        Some(ConnectError::Handshake)
    );
    assert_eq!(server.totals().connections, before.connections);
    assert!(server.poll_notice().is_none(), "no notice for 4,097 bytes");
}

#[test]
fn r6_a_resumed_movement_client_keeps_v2_channels() {
    let mut server = bind_movement_resumable_server(ServerLimits::v1());
    let (mut client, conn) = resumed_movement(&mut server);
    assert_eq!(client.framing(), Framing::V2);
    // A transfer frame is lane-2 framing: accepted on `V2`, `ChannelUnavailable`
    // on `V1`.
    assert_eq!(client.try_send_channel(Channel::Transfer, b"xyz"), Ok(()));
    assert_eq!(
        server_channel_next(&mut server, conn),
        (Channel::Transfer, b"xyz".to_vec())
    );

    // Control: the hello client on the same endpoint behaves the same way.
    let (mut hello_client, hello_conn) = hello_movement(&mut server, 1);
    assert_eq!(
        hello_client.try_send_channel(Channel::Transfer, b"xyz"),
        Ok(())
    );
    assert_eq!(
        server_channel_next(&mut server, hello_conn),
        (Channel::Transfer, b"xyz".to_vec())
    );
}

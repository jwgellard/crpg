//! T150b movement-endpoint suite: the 13 cases M1–M13 of `tasks/T150.md`,
//! all through the public `crpg_net_quic` API over real QUIC on IPv4
//! loopback, plus raw quinn clients for the bytes `QuicClient` never writes.
//!
//! Waits are the rig's 30 s failure guards, never oracles. Every refusal has
//! a positive control on the same endpoint. The client-side and report-count
//! cases that need a raw server are T150b2's.

#[path = "support/movement_rig.rs"]
mod movement_rig;
#[path = "support/quic_rig.rs"]
mod quic_rig;

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use crpg_net::movement_v4::{
    decode_down, decode_intent, sample_datagrams, MoveDown, MoveSample, SampleEntry,
};
use crpg_net::protocol::NetId;
use crpg_net::protocol_v4::Point;
use crpg_net_quic::{
    Channel, ChannelSendError, ClientLimits, CloseCode, CloseMode, CloseReason, ConfigError,
    ConnectError, ConnectionId, Framing, Hello, Lane1Stats, QuicClient, QuicServer, SendError,
    ServerConfig, ServerError, ServerEvent, ServerLimits, ServerNotice, Welcome, ALPN_LANE0_V1,
    ALPN_LANE0_V2, ALPN_LANE0_V3, LANE1_OUTBOUND_FRAMES, MAX_DATAGRAM_BYTES, MOVEMENT_WIRE_VERSION,
};
use movement_rig::*;
use quic_rig::*;

const ZERO_STATS: Lane1Stats = Lane1Stats {
    inbound_frames: 0,
    inbound_bytes: 0,
    outbound_frames: 0,
    outbound_bytes: 0,
    reader_parked: false,
    datagrams_queued: 0,
    datagrams_dropped: 0,
    max_datagram_bytes: None,
};

/// The queue counts of a snapshot. `max_datagram_bytes` is left out: path
/// MTU discovery may raise it between two snapshots.
fn queue_fields(stats: Lane1Stats) -> (usize, usize, usize, usize, bool, usize, u64) {
    (
        stats.inbound_frames,
        stats.inbound_bytes,
        stats.outbound_frames,
        stats.outbound_bytes,
        stats.reader_parked,
        stats.datagrams_queued,
        stats.datagrams_dropped,
    )
}

fn sent(error: SendError) -> ChannelSendError {
    ChannelSendError::Send(error)
}

fn bind_dual_server(limits: ServerLimits) -> QuicServer {
    QuicServer::bind_dual(ServerConfig {
        bind: loopback(),
        identity: identity_a(),
        limits,
    })
    .expect("bind dual server")
}

fn bind_v1_server(limits: ServerLimits) -> QuicServer {
    bind_server(limits)
}

/// A `crpg-lane0/2` client on its own thread.
fn connect_v2_async(
    limits: ClientLimits,
    server: std::net::SocketAddr,
    hello: Hello,
) -> std::thread::JoinHandle<Result<QuicClient, ConnectError>> {
    std::thread::spawn(move || {
        QuicClient::connect_framed(client_config(limits), server, &hello, Framing::V2)
    })
}

/// An accepted `/2` connection on a `bind_dual` server.
fn accept_v2(server: &mut QuicServer) -> (QuicClient, ConnectionId) {
    let pending = connect_v2_async(ClientLimits::v1(), server.local_addr(), hello(2));
    let (conn, _, _) = next_hello(server);
    server
        .accept(
            conn,
            Welcome {
                wire_version: 2,
                epoch: epoch(4),
            },
        )
        .expect("accept /2");
    (pending.join().expect("thread").expect("connect /2"), conn)
}

/// Guard: the client's next lane-1 frame, or its close reason.
fn client_lane1_next(client: &mut QuicClient) -> Result<Vec<u8>, CloseReason> {
    let start = Instant::now();
    loop {
        match client.try_recv_lane1() {
            Ok(Some(frame)) => return Ok(frame),
            Ok(None) => {}
            Err(reason) => return Err(reason),
        }
        assert!(start.elapsed() < DEADLINE, "deadline: client lane-1 frame");
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

/// Guard: the server's next lane-0 frame (with channel), or its error.
fn server_channel_next(
    server: &mut QuicServer,
    conn: ConnectionId,
) -> Result<(Channel, Vec<u8>), ServerError> {
    let start = Instant::now();
    loop {
        match server.try_recv_channel(conn) {
            Ok(Some(next)) => return Ok(next),
            Ok(None) => {}
            Err(error) => return Err(error),
        }
        assert!(start.elapsed() < DEADLINE, "deadline: server lane-0 frame");
        server.wait(Duration::from_millis(20));
    }
}

/// Guard: waits until the client records a close, then returns it.
fn client_closed(client: &QuicClient) -> CloseReason {
    let mut reason = None;
    until("client close", || {
        reason = client.close_reason();
        reason.is_some()
    });
    reason.expect("close reason")
}

/// Guard: the server's next notice (the `poll_notice` path).
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

/// The `Flush` cutoff with `ClientLimits::v1()`: 2,000 ms less its 200 ms
/// reserve (`close_budget.rs:166`). A close whose writers finish returns well
/// before it.
const FLUSH_END: Duration = Duration::from_millis(1_800);

/// Client limits with the smallest lane window (65,540 bytes), so a stalled
/// lane-1 writer is reached after a few hundred small frames. Nothing under
/// test changes: the lane-1 queues are fixed per-connection constants.
fn tight_window_client() -> ClientLimits {
    let mut limits = ClientLimits::v1();
    limits.stream_window_bytes = 65_540;
    limits.connection_window_bytes = 65_540;
    limits
}

/// Fills the server's lane-1 outbound queue with `size`-byte frames until the
/// writer is stalled and the queue sits exactly at its frame cap; returns the
/// frames accepted, in order. The client must not read lane 1.
fn fill_server_lane1(server: &mut QuicServer, conn: ConnectionId, size: usize) -> Vec<Vec<u8>> {
    let mut accepted: Vec<Vec<u8>> = Vec::new();
    let start = Instant::now();
    loop {
        loop {
            let frame = pat(accepted.len(), size);
            match server.try_send_lane1(conn, &frame) {
                Ok(()) => accepted.push(frame),
                Err(ChannelSendError::Send(SendError::QueueFull)) => break,
                Err(other) => panic!("unexpected {other:?}"),
            }
        }
        let before = queue_fields(server.lane1_stats(conn).expect("lane 1 open"));
        std::thread::park_timeout(Duration::from_millis(20));
        let after = queue_fields(server.lane1_stats(conn).expect("lane 1 open"));
        assert!(
            before.2 <= LANE1_OUTBOUND_FRAMES,
            "lane-1 outbound cap above {}: {before:?}",
            LANE1_OUTBOUND_FRAMES
        );
        if before == after && before.2 == LANE1_OUTBOUND_FRAMES {
            return accepted;
        }
        assert!(
            !(before == after && before.2 < LANE1_OUTBOUND_FRAMES),
            "lane-1 outbound cap below {}: {before:?}",
            LANE1_OUTBOUND_FRAMES
        );
        assert!(
            start.elapsed() < DEADLINE,
            "deadline: stalled lane-1 writer"
        );
    }
}

/// Fills the client's lane-1 outbound queue with `size`-byte frames until its
/// writer is stalled and the queue sits exactly at its 64-frame cap; returns
/// the frames accepted, in order. The server must not read lane 1.
fn fill_client_lane1(client: &mut QuicClient, size: usize) -> Vec<Vec<u8>> {
    let mut accepted: Vec<Vec<u8>> = Vec::new();
    let start = Instant::now();
    loop {
        loop {
            let frame = pat(accepted.len(), size);
            match client.try_send_lane1(&frame) {
                Ok(()) => accepted.push(frame),
                Err(ChannelSendError::Send(SendError::QueueFull)) => break,
                Err(other) => panic!("unexpected {other:?}"),
            }
        }
        let before = queue_fields(client.lane1_stats());
        std::thread::park_timeout(Duration::from_millis(20));
        let after = queue_fields(client.lane1_stats());
        assert!(
            before.2 <= 64,
            "client lane-1 outbound cap above 64: {before:?}"
        );
        if before == after && before.2 == 64 {
            return accepted;
        }
        assert!(
            !(before == after && before.2 < 64),
            "client lane-1 outbound cap below 64: {before:?}"
        );
        assert!(
            start.elapsed() < DEADLINE,
            "deadline: stalled client writer"
        );
    }
}

// ---------------------------------------------------------------------------
// M1
// ---------------------------------------------------------------------------

#[test]
fn m1_movement_constants() {
    assert_eq!(ALPN_LANE0_V3, b"crpg-lane0/3");
    assert_eq!(MOVEMENT_WIRE_VERSION, 4);
    assert_eq!(MAX_DATAGRAM_BYTES, 1_024);
    assert_eq!(LANE1_OUTBOUND_FRAMES, 128);
    assert_eq!(Framing::from_alpn(ALPN_LANE0_V3), None);
    assert_eq!(Framing::from_alpn(ALPN_LANE0_V1), Some(Framing::V1));
    assert_eq!(Framing::from_alpn(ALPN_LANE0_V2), Some(Framing::V2));
    assert_eq!(Framing::V1.alpn(), ALPN_LANE0_V1);
    assert_eq!(Framing::V2.alpn(), ALPN_LANE0_V2);
    assert_eq!(h(1).len(), 21);
    assert_eq!(h(127).len(), 21);
    assert_eq!(r1().len(), 24);
}

// ---------------------------------------------------------------------------
// M2
// ---------------------------------------------------------------------------

#[test]
fn m2_endpoint_kinds_and_parameters() {
    // A movement server offers `/3` alone.
    let mut server = bind_movement_server(ServerLimits::v1());
    assert!(raw_connect(server.local_addr(), ALPN_LANE0_V1).is_err());
    assert!(raw_connect(server.local_addr(), ALPN_LANE0_V2).is_err());
    until("two refused handshakes", || {
        server.totals().refused_handshakes == 2
    });
    assert!(server.poll_event().is_none(), "no event for a TLS failure");
    // Control: `/3` connects and advertises no datagram receive capacity.
    let raw = raw_movement(server.local_addr(), Some(65_536)).expect("raw /3");
    assert_eq!(raw.connection.max_datagram_size(), Some(0));
    let _streams = raw.open_and_write(&raw_hello4(7));
    let (conn, _, hello) = next_hello(&mut server);
    assert_eq!(hello.wire_version, 4);
    assert_eq!(server.framing(conn), Ok(Framing::V2));
    assert_eq!(server.movement(conn), Ok(true));
    assert_eq!(server.totals().refused_handshakes, 2);

    // `bind_dual` keeps its ALPN list and its datagram-free parameters.
    let mut dual = bind_dual_server(ServerLimits::v1());
    assert!(raw_movement(dual.local_addr(), Some(65_536)).is_err());
    until("dual refuses /3", || dual.totals().refused_handshakes == 1);
    assert!(dual.poll_event().is_none());
    let raw2 = raw_connect(dual.local_addr(), ALPN_LANE0_V2).expect("raw /2");
    assert_eq!(raw2.connection.max_datagram_size(), None);
    let (client, conn2) = accept_v2(&mut dual);
    assert_eq!(dual.movement(conn2), Ok(false));
    assert_eq!(dual.lane1_stats(conn2), Ok(ZERO_STATS));
    assert!(!client.movement());
    assert_eq!(client.lane1_stats(), ZERO_STATS);
    assert_eq!(client.framing(), Framing::V2);

    // `bind` keeps `/1` alone: a raw `/3` client fails the same way, and a
    // raw `/1` client sees no datagram capacity.
    let v1 = bind_v1_server(ServerLimits::v1());
    assert!(raw_movement(v1.local_addr(), Some(65_536)).is_err());
    until("v1 refuses /3", || v1.totals().refused_handshakes == 1);
    let raw1 = raw_connect(v1.local_addr(), ALPN_LANE0_V1).expect("raw /1");
    assert_eq!(raw1.connection.max_datagram_size(), None);
    // A `/1` connection is not a movement connection.
    let (client1, conn1) = {
        let mut v1 = v1;
        let (client1, conn1) = connect_accepted(&mut v1, ClientLimits::v1(), 1, epoch(3));
        assert_eq!(v1.movement(conn1), Ok(false));
        assert_eq!(v1.lane1_stats(conn1), Ok(ZERO_STATS));
        (client1, conn1)
    };
    assert!(!client1.movement());
    let _ = conn1;
}

/// Reading 1 / N-2 / N-3: a movement endpoint never takes a resume opening
/// and grants exactly two bidirectional streams.
#[test]
fn m2_third_stream_and_resume_opening_refused() {
    let mut server = bind_movement_server(ServerLimits::v1());
    // A `/3` opening in the resume layout is parsed as a hello: its 4-byte
    // header names a length far above the 130-byte hello cap, so it closes 3
    // with no notice.
    let raw = raw_movement(server.local_addr(), Some(65_536)).expect("raw /3");
    let _streams = raw.open_and_write(&[0x91, 0, 0, 4, 1, 2, 3, 4]);
    assert_eq!(raw.closed_code(), Some(3));
    until("resume opening refused", || {
        server.totals().refused_handshakes == 1
    });
    assert!(server.poll_notice().is_none(), "no resume notice on /3");
    assert!(server.poll_event().is_none());

    // Control: the same endpoint takes a hello4 and its lane-1 stream.
    let mut rm = raw_accepted(&mut server, 65_536, epoch(1));
    let (mut send2, _recv2) = rm.open_lane1(&[0, 0, 0, 1, 7]);
    assert_eq!(server_lane1_next(&mut server, rm.conn), Ok(vec![7]));
    // Two bidi streams are open (the hello stream and lane 1); the endpoint
    // grants no third, so `open_bi` never completes.
    let connection = rm.raw.connection.clone();
    let third = rm.raw.rt.run(async move {
        tokio::time::timeout(Duration::from_secs(1), connection.open_bi()).await
    });
    assert!(
        third.is_err(),
        "the movement endpoint granted a third bidirectional stream"
    );
    // Control: both existing streams still work.
    rm.raw.write(&mut send2, &[0, 0, 0, 1, 9]);
    assert_eq!(server_lane1_next(&mut server, rm.conn), Ok(vec![9]));
    server.try_send(rm.conn, &[5]).expect("lane 0 send");
    let mut frame = [0u8; 5];
    rm.raw
        .rt
        .run(async { rm.recv.read_exact(&mut frame).await })
        .expect("lane-0 frame");
    assert_eq!(frame, [0, 0, 0, 1, 5]);
}

// ---------------------------------------------------------------------------
// M3
// ---------------------------------------------------------------------------

#[test]
fn m3_movement_hello_and_welcome() {
    let mut server = bind_movement_server(ServerLimits::v1());
    // A hello that cannot be a `/3` hello fails before anything is sent.
    for version in [1u8, 2] {
        let bad = Hello {
            wire_version: version,
            credential: credential(32, 1),
        };
        let result = QuicClient::connect_movement(
            client_config(ClientLimits::v1()),
            server.local_addr(),
            &bad,
        );
        assert!(matches!(result, Err(ConnectError::Handshake)), "{version}");
    }
    let totals = server.totals();
    assert_eq!(totals.connections, 0);
    assert_eq!(totals.refused_handshakes, 0);

    // A raw `/3` hello naming 2, then 3, then 5: close 6, no event.
    let mut refused = 0u64;
    for version in [2u8, 3, 5] {
        let raw = raw_movement(server.local_addr(), Some(65_536)).expect("raw /3");
        let mut bytes = raw_hello4(7);
        bytes[4] = version;
        let _streams = raw.open_and_write(&bytes);
        assert_eq!(raw.closed_code(), Some(6), "version {version}");
        refused += 1;
        until("refused handshake counted", || {
            server.totals().refused_handshakes == refused
        });
        assert!(server.poll_event().is_none());
    }

    // Pair: the version is judged before the credential length.
    {
        let raw = raw_movement(server.local_addr(), Some(65_536)).expect("raw /3");
        let mut bytes = vec![0, 0, 0, 17, 2, 15];
        bytes.extend_from_slice(&[7u8; 15]);
        let _streams = raw.open_and_write(&bytes);
        assert_eq!(raw.closed_code(), Some(6));
        refused += 1;
        until("third refusal", || {
            server.totals().refused_handshakes == refused
        });
        // Control: version 4 with the same bad length is the credential's
        // fault, close 2.
        let raw = raw_movement(server.local_addr(), Some(65_536)).expect("raw /3");
        let mut bytes = vec![0, 0, 0, 17, 4, 15];
        bytes.extend_from_slice(&[7u8; 15]);
        let _streams = raw.open_and_write(&bytes);
        assert_eq!(raw.closed_code(), Some(2));
        refused += 1;
        until("fourth refusal", || {
            server.totals().refused_handshakes == refused
        });
        assert!(server.poll_event().is_none());
    }

    // Version 4: a hello event; a welcome of another version is refused and
    // sends nothing; the version-4 welcome is the first thing read.
    let raw = raw_movement(server.local_addr(), Some(65_536)).expect("raw /3");
    let (_send, mut recv) = raw.open_and_write(&raw_hello4(7));
    let (conn, _, hello) = next_hello(&mut server);
    assert_eq!(hello.wire_version, 4);
    for version in [1u8, 2, 3] {
        assert_eq!(
            server.accept(
                conn,
                Welcome {
                    wire_version: version,
                    epoch: epoch(3),
                }
            ),
            Err(ServerError::InvalidWelcome),
            "welcome {version}"
        );
    }
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
    let welcome = read_raw_welcome4(&raw, &mut recv);
    let mut expected = vec![0, 0, 0, 17, 4];
    expected.extend_from_slice(&epoch(3));
    assert_eq!(welcome.to_vec(), expected);

    // The crate's client: the welcome, the framing and `movement()`.
    let (client, _) = accept_movement(&mut server, ClientLimits::v1(), 3, epoch(9));
    assert_eq!(
        client.welcome(),
        Welcome {
            wire_version: 4,
            epoch: epoch(9),
        }
    );
    assert_eq!(client.framing(), Framing::V2);
    assert!(client.movement());

    // Reading 10 / N-14: `/1` and `/2` keep the 1-or-2 rule, so a version-4
    // hello or welcome closes or is refused there.
    let mut v1 = bind_v1_server(ServerLimits::v1());
    {
        let raw = raw_connect(v1.local_addr(), ALPN_LANE0_V1).expect("raw /1");
        let _streams = raw.open_and_write(&raw_hello4(7));
        assert_eq!(raw.closed_code(), Some(6), "a /1 hello naming 4");
        until("v1 refuses version 4", || {
            v1.totals().refused_handshakes == 1
        });
        assert!(v1.poll_event().is_none());
        // Control: the same raw layout naming 1 is a hello.
        let raw = raw_connect(v1.local_addr(), ALPN_LANE0_V1).expect("raw /1");
        let mut bytes = raw_hello4(7);
        bytes[4] = 1;
        let (_send, mut recv) = raw.open_and_write(&bytes);
        let (conn1, _, hello) = next_hello(&mut v1);
        assert_eq!(hello.wire_version, 1);
        assert_eq!(
            v1.accept(
                conn1,
                Welcome {
                    wire_version: 4,
                    epoch: epoch(3),
                }
            ),
            Err(ServerError::InvalidWelcome),
            "a /1 welcome naming 4"
        );
        assert_eq!(
            v1.accept(
                conn1,
                Welcome {
                    wire_version: 1,
                    epoch: epoch(3),
                }
            ),
            Ok(())
        );
        assert_eq!(raw.read_welcome(&mut recv).wire_version, 1);
    }
    let mut dual = bind_dual_server(ServerLimits::v1());
    {
        let raw = raw_connect(dual.local_addr(), ALPN_LANE0_V2).expect("raw /2");
        let _streams = raw.open_and_write(&raw_hello4(7));
        assert_eq!(raw.closed_code(), Some(6), "a /2 hello naming 4");
        until("dual refuses version 4", || {
            dual.totals().refused_handshakes == 1
        });
        assert!(dual.poll_event().is_none());
        // Control: a version-2 hello is accepted; its welcome may not name 4.
        let raw = raw_connect(dual.local_addr(), ALPN_LANE0_V2).expect("raw /2");
        let mut bytes = raw_hello4(7);
        bytes[4] = 2;
        let (_send, mut recv) = raw.open_and_write(&bytes);
        let (conn2, _, hello) = next_hello(&mut dual);
        assert_eq!(hello.wire_version, 2);
        assert_eq!(
            dual.accept(
                conn2,
                Welcome {
                    wire_version: 4,
                    epoch: epoch(3),
                }
            ),
            Err(ServerError::InvalidWelcome),
            "a /2 welcome naming 4"
        );
        assert_eq!(
            dual.accept(
                conn2,
                Welcome {
                    wire_version: 2,
                    epoch: epoch(3),
                }
            ),
            Ok(())
        );
        assert_eq!(raw.read_welcome(&mut recv).wire_version, 2);
    }

    // N-4 order: the limits are validated before the hello is encoded, so a
    // version-2 hello with invalid limits is `Config`, not `Handshake`.
    let mut bad = ClientLimits::v1();
    bad.inbound_frames = 0;
    assert_eq!(
        QuicClient::connect_movement(
            client_config(bad),
            server.local_addr(),
            &Hello {
                wire_version: 2,
                credential: credential(32, 1),
            }
        )
        .unwrap_err(),
        ConnectError::Config(ConfigError::InvalidLimits {
            field: "inbound_frames"
        })
    );
    // Control: valid limits with the same hello are `Handshake` (above), and
    // invalid limits with a version-4 hello are the same `Config` with no
    // connection made.
    let before = server.totals();
    assert!(matches!(
        QuicClient::connect_movement(
            client_config(bad),
            server.local_addr(),
            &Hello {
                wire_version: 4,
                credential: credential(32, 1),
            }
        ),
        Err(ConnectError::Config(_))
    ));
    let after = server.totals();
    assert_eq!(after.connections, before.connections);
    assert_eq!(after.refused_handshakes, before.refused_handshakes);

    // Control: a `/2` client with a version-2 hello still connects to
    // `bind_dual`.
    let (client2, conn2) = accept_v2(&mut dual);
    assert_eq!(client2.welcome().wire_version, 2);
    assert_eq!(dual.framing(conn2), Ok(Framing::V2));
}

// ---------------------------------------------------------------------------
// M4
// ---------------------------------------------------------------------------

#[test]
fn m4_datagram_floor_at_connect() {
    let mut server = bind_movement_server(ServerLimits::v1());
    // No datagram support at all.
    let raw = raw_movement(server.local_addr(), None).expect("tls completes");
    assert_eq!(raw.closed_code(), Some(2));
    until("first refusal", || server.totals().refused_handshakes == 1);
    assert!(server.poll_event().is_none());
    // Advertised 1,032: sendable 1,023.
    let raw = raw_movement(server.local_addr(), Some(1_032)).expect("tls completes");
    assert_eq!(raw.closed_code(), Some(2));
    until("second refusal", || server.totals().refused_handshakes == 2);
    assert!(server.poll_event().is_none());
    // Advertised 1,033: sendable exactly 1,024. The hello gets through.
    let raw = raw_movement(server.local_addr(), Some(1_033)).expect("tls completes");
    let _streams = raw.open_and_write(&raw_hello4(7));
    let (conn, _, hello) = next_hello(&mut server);
    assert_eq!(hello.wire_version, 4);
    assert_eq!(
        server.lane1_stats(conn).unwrap().max_datagram_bytes,
        Some(1_024)
    );
    assert_eq!(server.totals().refused_handshakes, 2);

    // The crate's client: the server may send at least 1,024; the client
    // sends none.
    let (client, conn2) = accept_movement(&mut server, ClientLimits::v1(), 3, epoch(1));
    let size = server.lane1_stats(conn2).unwrap().max_datagram_bytes;
    assert!(
        matches!(size, Some(n) if (1_024..=65_526).contains(&n)),
        "server datagram size {size:?}"
    );
    assert_eq!(client.lane1_stats().max_datagram_bytes, Some(0));
}

// ---------------------------------------------------------------------------
// M5
// ---------------------------------------------------------------------------

#[test]
fn m5_lane1_round_trip() {
    let mut server = bind_movement_server(ServerLimits::v1());
    let (mut client, conn) = accept_movement(&mut server, ClientLimits::v1(), 1, epoch(2));
    let (f1, f2, f3, f4) = (h(1), m(2), h(3), pat(1, 4_096));
    let (l0a, l0b) = (pat(9, 10), pat(10, 11));
    // Interleaved: lane 1, lane 0, lane 1, lane 0, lane 1, lane 1.
    client.try_send_lane1(&f1).unwrap();
    client.try_send(&l0a).unwrap();
    client.try_send_lane1(&f2).unwrap();
    client.try_send(&l0b).unwrap();
    client.try_send_lane1(&f3).unwrap();
    client.try_send_lane1(&f4).unwrap();
    assert!(server.wait(DEADLINE));
    let mut lane1: Vec<Vec<u8>> = Vec::new();
    let mut lane0: Vec<Vec<u8>> = Vec::new();
    until("four lane-1 and two lane-0 frames", || {
        while let Some(frame) = server.try_recv_lane1(conn).expect("lane 1 open") {
            lane1.push(frame);
        }
        while let Some((channel, frame)) = server.try_recv_channel(conn).expect("lane 0 open") {
            assert_eq!(channel, Channel::Lane0);
            lane0.push(frame);
        }
        lane1.len() >= 4 && lane0.len() >= 2
    });
    assert_eq!(lane1, vec![f1.clone(), f2.clone(), f3.clone(), f4.clone()]);
    assert_eq!(lane0, vec![l0a.clone(), l0b.clone()]);
    assert_eq!(decode_intent(&lane1[0]), Ok(h_intent(1)));
    assert_eq!(decode_intent(&lane1[1]), Ok(m_intent(2)));
    assert_eq!(decode_intent(&lane1[2]), Ok(h_intent(3)));
    assert_eq!(server.try_recv_lane1(conn), Ok(None));
    assert_eq!(server.try_recv_channel(conn), Ok(None));
    assert_eq!(server.try_recv(conn), Ok(None));

    // Downstream: lane 1 and lane 0 stay separate.
    server.try_send_lane1(conn, &r1()).unwrap();
    server.try_send_lane1(conn, &pat(2, 1_024)).unwrap();
    server.try_send(conn, &pat(3, 50)).unwrap();
    assert_eq!(client_recv_lane1(&mut client), r1());
    assert_eq!(client_recv_lane1(&mut client), pat(2, 1_024));
    assert_eq!(client_recv(&mut client), Ok(pat(3, 50)));
    assert_eq!(client.try_recv_lane1(), Ok(None));
    assert_eq!(client.try_recv(), Ok(None));

    // Quiescent now: both outbound queues are empty and both inbound drained.
    assert_eq!(
        queue_fields(server.lane1_stats(conn).unwrap()),
        queue_fields(ZERO_STATS)
    );
    assert_eq!(queue_fields(client.lane1_stats()), queue_fields(ZERO_STATS));
    // Boundaries, each refused with nothing changed; the cap exactly was the
    // control above (4,096 up, 1,024 down).
    let client_before = queue_fields(client.lane1_stats());
    assert_eq!(
        client.try_send_lane1(&pat(1, 4_097)),
        Err(sent(SendError::FrameTooLarge))
    );
    assert_eq!(queue_fields(client.lane1_stats()), client_before);
    assert_eq!(client.try_send_lane1(&[]), Err(sent(SendError::EmptyFrame)));
    assert_eq!(queue_fields(client.lane1_stats()), client_before);
    let server_before = queue_fields(server.lane1_stats(conn).unwrap());
    assert_eq!(
        server.try_send_lane1(conn, &pat(1, 1_025)),
        Err(sent(SendError::FrameTooLarge))
    );
    assert_eq!(
        queue_fields(server.lane1_stats(conn).unwrap()),
        server_before
    );
    assert_eq!(
        server.try_send_lane1(conn, &[]),
        Err(sent(SendError::EmptyFrame))
    );
    assert_eq!(
        queue_fields(server.lane1_stats(conn).unwrap()),
        server_before
    );
    // One byte under each cap still goes through.
    // Both queues are empty, so a lane-1 frame alone must wake `wait` on each
    // side.
    client.try_send_lane1(&pat(4, 4_095)).unwrap();
    assert!(server.wait(DEADLINE), "a lane-1 frame wakes the server");
    assert_eq!(server_lane1_next(&mut server, conn), Ok(pat(4, 4_095)));
    server.try_send_lane1(conn, &pat(5, 1_023)).unwrap();
    assert!(client.wait(DEADLINE), "a lane-1 frame wakes the client");
    assert_eq!(client_recv_lane1(&mut client), pat(5, 1_023));
    // N-12: once lane 1 is drained, `wait` times out on both sides (lane 0
    // and the datagram queue are empty too).
    assert_eq!(server.try_recv_lane1(conn), Ok(None));
    assert_eq!(client.try_recv_lane1(), Ok(None));
    assert!(
        !server.wait(Duration::from_millis(100)),
        "a drained lane 1 must not wake the server"
    );
    assert!(
        !client.wait(Duration::from_millis(100)),
        "a drained lane 1 must not wake the client"
    );
}

// ---------------------------------------------------------------------------
// M6
// ---------------------------------------------------------------------------

#[test]
fn m6_lane1_and_datagram_refusals() {
    // A pending `/3` connection: hello received, not accepted.
    let mut server = bind_movement_server(ServerLimits::v1());
    let raw = raw_movement(server.local_addr(), Some(65_536)).expect("raw /3");
    let _streams = raw.open_and_write(&raw_hello4(7));
    let (pending, _, _) = next_hello(&mut server);
    assert_eq!(
        server.try_send_lane1(pending, &[]),
        Err(sent(SendError::NotAccepted)),
        "NotAccepted before EmptyFrame"
    );
    assert_eq!(
        server.try_send_lane1(pending, &[1]),
        Err(sent(SendError::NotAccepted))
    );
    assert_eq!(
        server.try_send_datagram(pending, &[1]),
        Err(sent(SendError::NotAccepted))
    );
    assert_eq!(
        server.try_send_datagram(pending, &[]),
        Err(sent(SendError::NotAccepted))
    );
    assert_eq!(
        server.try_recv_lane1(pending),
        Err(ServerError::NotAccepted)
    );
    // Pair: NotAccepted before Closed, on a pending connection that was
    // refused.
    server.refuse(pending, CloseCode::ServerBusy).unwrap();
    assert_eq!(
        server.try_send_lane1(pending, &[1]),
        Err(sent(SendError::NotAccepted))
    );
    assert_eq!(
        server.try_send_datagram(pending, &[1]),
        Err(sent(SendError::NotAccepted))
    );
    assert_eq!(
        server.try_recv_lane1(pending),
        Err(ServerError::NotAccepted)
    );
    assert_eq!(
        next_closed(&mut server),
        (pending, CloseReason::Local(CloseCode::ServerBusy))
    );

    // An accepted `/2` connection on `bind_dual`.
    let mut dual = bind_dual_server(ServerLimits::v1());
    let (mut client2, conn2) = accept_v2(&mut dual);
    assert_eq!(
        dual.try_send_lane1(conn2, &[]),
        Err(ChannelSendError::ChannelUnavailable),
        "ChannelUnavailable before EmptyFrame"
    );
    assert_eq!(
        dual.try_send_lane1(conn2, &[1]),
        Err(ChannelSendError::ChannelUnavailable)
    );
    assert_eq!(
        dual.try_send_datagram(conn2, &[1]),
        Err(ChannelSendError::ChannelUnavailable)
    );
    assert_eq!(
        dual.try_send_datagram(conn2, &[]),
        Err(ChannelSendError::ChannelUnavailable)
    );
    assert_eq!(dual.try_recv_lane1(conn2), Ok(None));
    assert_eq!(
        client2.try_send_lane1(&[1]),
        Err(ChannelSendError::ChannelUnavailable)
    );
    assert_eq!(
        client2.try_send_lane1(&[]),
        Err(ChannelSendError::ChannelUnavailable)
    );
    assert_eq!(client2.try_recv_lane1(), Ok(None));
    assert_eq!(client2.try_recv_datagram(), Ok(None));
    dual.close(conn2, CloseCode::Normal, CloseMode::Discard)
        .unwrap();
    let closed = sent(SendError::Closed(CloseReason::Local(CloseCode::Normal)));
    assert_eq!(dual.try_send_lane1(conn2, &[1]), Err(closed));
    assert_eq!(dual.try_send_lane1(conn2, &[]), Err(closed));
    assert_eq!(dual.try_send_datagram(conn2, &[1]), Err(closed));
    // The same pair seen from the `/2` client once the server's close
    // arrives: Closed before ChannelUnavailable.
    assert_eq!(client_closed(&client2), CloseReason::Peer { code: 0 });
    let peer_closed = sent(SendError::Closed(CloseReason::Peer { code: 0 }));
    assert_eq!(client2.try_send_lane1(&[1]), Err(peer_closed));
    assert_eq!(client2.try_send_lane1(&[]), Err(peer_closed));

    // An accepted `/3` connection.
    let (mut client, conn) = accept_movement(&mut server, ClientLimits::v1(), 3, epoch(5));
    assert_eq!(
        server.try_send_datagram(conn, &[]),
        Err(sent(SendError::EmptyFrame))
    );
    assert_eq!(
        server.try_send_datagram(conn, &pat(1, 1_025)),
        Err(sent(SendError::FrameTooLarge))
    );
    assert_eq!(server.try_send_datagram(conn, &pat(1, 1_024)), Ok(()));
    assert_eq!(server.try_send_datagram(conn, &[1]), Ok(()));
    // Not buildable: EmptyFrame with FrameTooLarge (no frame is both).
    server
        .close(conn, CloseCode::Normal, CloseMode::Discard)
        .unwrap();
    // Closed before EmptyFrame and before FrameTooLarge.
    assert_eq!(server.try_send_datagram(conn, &[1]), Err(closed));
    assert_eq!(server.try_send_datagram(conn, &[]), Err(closed));
    assert_eq!(server.try_send_datagram(conn, &pat(1, 1_025)), Err(closed));
    assert_eq!(server.try_send_lane1(conn, &[]), Err(closed));
    assert_eq!(server.try_send_lane1(conn, &pat(1, 1_025)), Err(closed));
    // The `/3` client sees the same order once the close arrives.
    assert_eq!(client_closed(&client), CloseReason::Peer { code: 0 });
    assert_eq!(client.try_send_lane1(&[]), Err(peer_closed));
    assert_eq!(client.try_send_lane1(&pat(1, 4_097)), Err(peer_closed));
    // Once its `Closed` is polled and forgotten, every call is unknown.
    let (gone, reason) = next_closed(&mut server);
    assert_eq!(gone, conn);
    assert_eq!(reason, CloseReason::Local(CloseCode::Normal));
    let unknown = sent(SendError::UnknownConnection);
    assert_eq!(server.try_send_lane1(conn, &[]), Err(unknown));
    assert_eq!(server.try_send_lane1(conn, &[1]), Err(unknown));
    assert_eq!(server.try_send_datagram(conn, &[]), Err(unknown));
    assert_eq!(server.try_send_datagram(conn, &[1]), Err(unknown));
    assert_eq!(
        server.try_recv_lane1(conn),
        Err(ServerError::UnknownConnection)
    );
    assert_eq!(
        server.lane1_stats(conn),
        Err(ServerError::UnknownConnection)
    );
    assert_eq!(server.movement(conn), Err(ServerError::UnknownConnection));
}

// ---------------------------------------------------------------------------
// M7
// ---------------------------------------------------------------------------

/// Runs one violation case: `bytes` on the raw lane-1 stream, then expects
/// `code` from the server and a matching local close event.
fn violation_case(
    server: &mut QuicServer,
    bytes: &[u8],
    finish: bool,
    expected: u64,
    expected_reason: CloseCode,
) {
    let mut rm = raw_accepted(server, 65_536, epoch(1));
    let (mut send2, _recv2) = rm.open_lane1(bytes);
    if finish {
        rm.raw.finish(&mut send2);
    }
    assert_eq!(rm.raw.closed_code(), Some(expected));
    let (conn, reason) = next_closed(server);
    assert_eq!(conn, rm.conn);
    assert_eq!(reason, CloseReason::Local(expected_reason));
    let _ = &mut rm;
}

#[test]
fn m7_raw_lane1_violations() {
    let mut server = bind_movement_server(ServerLimits::v1());
    // (a) a zero length.
    violation_case(
        &mut server,
        &[0, 0, 0, 0],
        false,
        2,
        CloseCode::ProtocolViolation,
    );
    // (b) 4,097 over the 4,096 cap, no body written.
    violation_case(
        &mut server,
        &[0, 0, 16, 1],
        false,
        3,
        CloseCode::FrameTooLarge,
    );
    // (c) a FIN mid-frame.
    violation_case(
        &mut server,
        &[0, 0, 0, 5, 1, 2],
        true,
        2,
        CloseCode::ProtocolViolation,
    );
    // (d) a RESET_STREAM at a frame boundary, after the server read the frame.
    {
        let mut rm = raw_accepted(&mut server, 65_536, epoch(1));
        let (mut send2, _recv2) = rm.open_lane1(&[0, 0, 0, 1, 7]);
        assert_eq!(server_lane1_next(&mut server, rm.conn), Ok(vec![7]));
        send2.reset(quinn::VarInt::from_u32(0)).expect("reset");
        assert_eq!(rm.raw.closed_code(), Some(2));
        let (conn, reason) = next_closed(&mut server);
        assert_eq!(conn, rm.conn);
        assert_eq!(reason, CloseReason::Local(CloseCode::ProtocolViolation));
        let _ = &mut rm;
    }
    // (e) control: the 4,096 cap exactly is no violation (M5), a good frame is
    // delivered, and a FIN at the boundary closes nothing.
    {
        let mut rm = raw_accepted(&mut server, 65_536, epoch(1));
        let (mut send2, _recv2) = rm.open_lane1(&[0, 0, 0, 4, 1, 2, 3, 4]);
        assert_eq!(
            server_lane1_next(&mut server, rm.conn),
            Ok(vec![1, 2, 3, 4])
        );
        rm.raw.finish(&mut send2);
        // The connection stays open: lane 0 still carries a frame down.
        server.try_send(rm.conn, &[5]).expect("lane 0 send");
        let mut frame = [0u8; 5];
        rm.raw
            .rt
            .run(async { rm.recv.read_exact(&mut frame).await })
            .expect("lane-0 frame");
        assert_eq!(frame, [0, 0, 0, 1, 5]);
        assert!(rm.raw.connection.close_reason().is_none());
        assert_eq!(server.try_recv_lane1(rm.conn), Ok(None));
        // Datagrams: the client may send none; an empty one is a transport
        // violation at the server's quinn.
        assert_eq!(
            rm.raw.connection.send_datagram(vec![1, 2, 3].into()),
            Err(quinn::SendDatagramError::TooLarge)
        );
        assert!(rm.raw.connection.close_reason().is_none());
        rm.raw
            .connection
            .send_datagram(Vec::<u8>::new().into())
            .expect("empty datagram is accepted locally");
        let (conn, reason) = next_closed(&mut server);
        assert_eq!(conn, rm.conn);
        assert_eq!(reason, CloseReason::Transport);
    }
    // (f) STOP_SENDING on the lane-1 send half closes at the next lane-1
    // write.
    {
        let mut rm = raw_accepted(&mut server, 65_536, epoch(1));
        let (_send2, mut recv2) = rm.open_lane1(&[0, 0, 0, 1, 7]);
        assert_eq!(server_lane1_next(&mut server, rm.conn), Ok(vec![7]));
        recv2.stop(quinn::VarInt::from_u32(0)).expect("stop");
        // The server only sees the stop at a lane-1 write; the first write
        // may beat the frame carrying it, so write until the close lands.
        let start = Instant::now();
        loop {
            let _ = server.try_send_lane1(rm.conn, &pat(1, 8));
            if let Some(event) = server.poll_event() {
                match event {
                    crpg_net_quic::ServerEvent::Closed { conn, reason } => {
                        assert_eq!(conn, rm.conn);
                        assert_eq!(reason, CloseReason::Local(CloseCode::ProtocolViolation));
                        break;
                    }
                    other => panic!("unexpected {other:?}"),
                }
            }
            assert!(start.elapsed() < DEADLINE, "deadline: stop_sending close");
            server.wait(Duration::from_millis(10));
        }
        assert_eq!(rm.raw.closed_code(), Some(2));
        let _ = &mut rm;
    }
}

// ---------------------------------------------------------------------------
// M8
// ---------------------------------------------------------------------------

#[test]
fn m8_lane1_backpressure_inbound() {
    let mut server = bind_movement_server(ServerLimits::v1());
    let (mut client, conn) = accept_movement(&mut server, ClientLimits::v1(), 1, epoch(2));
    // The server reads no lane 1. The client sends until its own queue is
    // full; the sender keeps going while the server's queue and parked slot
    // fill behind it.
    let mut accepted: Vec<Vec<u8>> = Vec::new();
    let start = Instant::now();
    let send_one = |client: &mut QuicClient, accepted: &mut Vec<Vec<u8>>| {
        let frame = pat(accepted.len(), 1_024);
        match client.try_send_lane1(&frame) {
            Ok(()) => {
                accepted.push(frame);
                true
            }
            Err(ChannelSendError::Send(SendError::QueueFull)) => false,
            Err(other) => panic!("unexpected {other:?}"),
        }
    };
    while send_one(&mut client, &mut accepted) {
        assert!(start.elapsed() < DEADLINE, "deadline: no QueueFull");
    }
    until("server queue full and parked", || {
        send_one(&mut client, &mut accepted);
        let stats = server.lane1_stats(conn).unwrap();
        stats.inbound_frames == 64 && stats.inbound_bytes == 65_536 && stats.reader_parked
    });
    // A refused send leaves the client's queue counts unchanged; the writer
    // is blocked by flow control, so the counts settle.
    let mut settled = false;
    for _ in 0..1_000 {
        while send_one(&mut client, &mut accepted) {}
        let before = queue_fields(client.lane1_stats());
        let refused = client.try_send_lane1(&pat(accepted.len(), 1_024));
        let after = queue_fields(client.lane1_stats());
        match refused {
            Err(ChannelSendError::Send(SendError::QueueFull)) => {
                if before == after {
                    settled = true;
                    break;
                }
            }
            Ok(()) => {
                let frame = pat(accepted.len(), 1_024);
                accepted.push(frame);
            }
            Err(other) => panic!("unexpected {other:?}"),
        }
        if settled {
            break;
        }
        client.wait(Duration::from_millis(5));
    }
    assert!(settled, "a refused lane-1 send changed the client's queue");
    // Pairs: the size checks come before QueueFull on a full queue.
    assert_eq!(
        client.try_send_lane1(&pat(1, 4_097)),
        Err(sent(SendError::FrameTooLarge))
    );
    assert_eq!(client.try_send_lane1(&[]), Err(sent(SendError::EmptyFrame)));
    // The server's queue stays at its caps and lane 0 still flows.
    let stats = server.lane1_stats(conn).unwrap();
    assert_eq!((stats.inbound_frames, stats.inbound_bytes), (64, 65_536));
    assert!(stats.reader_parked);
    client.try_send(&[1]).unwrap();
    assert_eq!(server_recv(&mut server, conn), Ok(vec![1]));
    // Drain lane 1 while the client keeps sending: every accepted frame
    // arrives exactly once, in order.
    let mut got: Vec<Vec<u8>> = Vec::new();
    let mut extra = 20;
    let start = Instant::now();
    while extra > 0 || got.len() < accepted.len() {
        if extra > 0 && send_one(&mut client, &mut accepted) {
            extra -= 1;
        }
        while let Some(frame) = server.try_recv_lane1(conn).expect("lane 1 open") {
            got.push(frame);
        }
        assert!(start.elapsed() < DEADLINE, "deadline: drain");
        if got.len() < accepted.len() {
            server.wait(Duration::from_millis(2));
        }
    }
    assert_eq!(got, accepted);
    assert_eq!(server.try_recv_lane1(conn), Ok(None));
}

#[test]
fn m8_lane1_backpressure_outbound() {
    let mut server = bind_movement_server(ServerLimits::v1());
    let (mut client, conn) = accept_movement(&mut server, ClientLimits::v1(), 1, epoch(2));
    prime(&mut client, &mut server, conn);
    // The client reads no lane 1; the server sends until its queue is full
    // and its writer is stalled: the 128-frame cap equals 131,072 bytes at
    // 1,024-byte frames, so the stalled state pins both caps exactly.
    let accepted = fill_server_lane1(&mut server, conn, 1_024);
    let stats = server.lane1_stats(conn).unwrap();
    assert_eq!(stats.outbound_frames, LANE1_OUTBOUND_FRAMES, "{stats:?}");
    assert_eq!(stats.outbound_bytes, 131_072, "{stats:?}");
    assert!(stats.outbound_frames > 0, "N-13: non-zero outbound count");
    // Pairs on a full queue: the size checks come first.
    assert_eq!(
        server.try_send_lane1(conn, &pat(1, 1_025)),
        Err(sent(SendError::FrameTooLarge))
    );
    assert_eq!(
        server.try_send_lane1(conn, &[]),
        Err(sent(SendError::EmptyFrame))
    );
    // Lane 0 still flows.
    server.try_send(conn, &[2]).unwrap();
    assert_eq!(client_recv(&mut client), Ok(vec![2]));
    // The client drains lane 1: every accepted frame once, in order, with its
    // inbound queue never above 128.
    let mut got: Vec<Vec<u8>> = Vec::new();
    let start = Instant::now();
    while got.len() < accepted.len() {
        let inbound = client.lane1_stats().inbound_frames;
        assert!(inbound <= 128, "client inbound {inbound}");
        match client.try_recv_lane1().expect("lane 1 open") {
            Some(frame) => got.push(frame),
            None => {
                assert!(start.elapsed() < DEADLINE, "deadline: drain");
                client.wait(Duration::from_millis(2));
            }
        }
    }
    assert_eq!(got, accepted);
    assert_eq!(client.try_recv_lane1(), Ok(None));
}

/// Reading 8 / N-6: 1-byte frames fill the server's *frame* cap (64) alone,
/// 64 bytes below the 65,536-byte cap, and the 65th is parked.
#[test]
fn m8_lane1_inbound_frame_cap_binds_alone() {
    let mut server = bind_movement_server(ServerLimits::v1());
    let (mut client, conn) = accept_movement(&mut server, ClientLimits::v1(), 1, epoch(2));
    let mut sent = 0usize;
    let mut parked_offered = false;
    until("server lane-1 frame cap and parked slot", || {
        while sent < 64 {
            let frame = pat(sent, 1);
            match client.try_send_lane1(&frame) {
                Ok(()) => sent += 1,
                Err(ChannelSendError::Send(SendError::QueueFull)) => break,
                Err(other) => panic!("unexpected {other:?}"),
            }
        }
        if sent < 64 {
            return false;
        }
        if !parked_offered {
            match client.try_send_lane1(&pat(200, 1)) {
                Ok(()) => parked_offered = true,
                Err(ChannelSendError::Send(SendError::QueueFull)) => {}
                Err(other) => panic!("unexpected {other:?}"),
            }
        }
        let stats = server.lane1_stats(conn).expect("lane 1 open");
        assert!(
            stats.inbound_frames <= 64,
            "server lane-1 inbound cap above 64: {stats:?}"
        );
        assert!(
            !(stats.reader_parked && stats.inbound_frames < 64),
            "server lane-1 inbound cap below 64: {stats:?}"
        );
        stats.inbound_frames == 64 && stats.inbound_bytes == 64 && stats.reader_parked
    });
    // The cap is exact: 64 frames of 1 byte, and one held behind them.
    assert_eq!(server.try_recv_lane1(conn), Ok(Some(vec![0])));
    let stats = server.lane1_stats(conn).unwrap();
    assert_eq!(stats.inbound_frames, 64, "the parked frame enters at once");
    assert_eq!(stats.inbound_bytes, 64);
    assert!(!stats.reader_parked);
    assert_eq!(server.try_recv_lane1(conn), Ok(Some(vec![1])));
}

/// Reading 8 / N-6: 100-byte frames fill the server's *frame* cap (128)
/// alone: 12,800 bytes, far below 131,072, with the writer stalled.
#[test]
fn m8_lane1_outbound_frame_cap_binds_alone() {
    let mut server = bind_movement_server(ServerLimits::v1());
    let (mut client, conn) = accept_movement(&mut server, tight_window_client(), 1, epoch(2));
    prime(&mut client, &mut server, conn);
    let accepted = fill_server_lane1(&mut server, conn, 100);
    let stats = server.lane1_stats(conn).unwrap();
    assert_eq!(stats.outbound_frames, LANE1_OUTBOUND_FRAMES, "{stats:?}");
    assert_eq!(
        stats.outbound_bytes,
        LANE1_OUTBOUND_FRAMES * 100,
        "{stats:?}"
    );
    // Pairs on a full queue, with the byte cap far from binding.
    let before = queue_fields(stats);
    assert_eq!(
        server.try_send_lane1(conn, &pat(1, 1_025)),
        Err(sent(SendError::FrameTooLarge))
    );
    assert_eq!(
        server.try_send_lane1(conn, &[]),
        Err(sent(SendError::EmptyFrame))
    );
    assert_eq!(queue_fields(server.lane1_stats(conn).unwrap()), before);
    // Control: every accepted frame is still delivered once, in order.
    let mut got: Vec<Vec<u8>> = Vec::new();
    let start = Instant::now();
    while got.len() < accepted.len() {
        match client.try_recv_lane1().expect("lane 1 open") {
            Some(frame) => got.push(frame),
            None => {
                assert!(start.elapsed() < DEADLINE, "deadline: drain");
                client.wait(Duration::from_millis(2));
            }
        }
    }
    assert_eq!(got, accepted);
}

/// Reading 8 client caps: lane-1 outbound is 64 frames / 65,536 bytes and
/// inbound 128 / 131,072 (not swapped and not the server's).
#[test]
fn m8_client_lane1_caps_are_not_swapped() {
    // Outbound: 100-byte frames bind the 64-frame cap alone (6,400 bytes).
    let mut server = bind_movement_server(ServerLimits::v1());
    let (mut client, _conn) = accept_movement(&mut server, ClientLimits::v1(), 1, epoch(2));
    let accepted = fill_client_lane1(&mut client, 100);
    let stats = client.lane1_stats();
    assert_eq!(stats.outbound_frames, 64, "{stats:?}");
    assert_eq!(stats.outbound_bytes, 6_400, "{stats:?}");
    assert!(accepted.len() >= 64, "{}", accepted.len());
    drop(client);

    // Inbound: 100-byte frames fill 128 frames (12,800 bytes) and park the
    // 129th; the client reads no lane 1.
    let mut server = bind_movement_server(ServerLimits::v1());
    let (mut client, conn) = accept_movement(&mut server, tight_window_client(), 1, epoch(3));
    prime(&mut client, &mut server, conn);
    let mut sent = 0usize;
    let mut parked_offered = false;
    until("client lane-1 inbound cap and parked slot", || {
        while sent < 128 {
            let frame = pat(sent, 100);
            match server.try_send_lane1(conn, &frame) {
                Ok(()) => sent += 1,
                Err(ChannelSendError::Send(SendError::QueueFull)) => break,
                Err(other) => panic!("unexpected {other:?}"),
            }
        }
        if sent < 128 {
            return false;
        }
        if !parked_offered {
            match server.try_send_lane1(conn, &pat(200, 100)) {
                Ok(()) => parked_offered = true,
                Err(ChannelSendError::Send(SendError::QueueFull)) => {}
                Err(other) => panic!("unexpected {other:?}"),
            }
        }
        let stats = client.lane1_stats();
        assert!(
            stats.inbound_frames <= 128,
            "client lane-1 inbound cap above 128: {stats:?}"
        );
        assert!(
            !(stats.reader_parked && stats.inbound_frames < 128),
            "client lane-1 inbound cap below 128: {stats:?}"
        );
        stats.inbound_frames == 128 && stats.inbound_bytes == 12_800 && stats.reader_parked
    });
    assert_eq!(client.try_recv_lane1(), Ok(Some(pat(0, 100))));
    let stats = client.lane1_stats();
    assert_eq!(stats.inbound_frames, 128, "the parked frame enters at once");
    assert_eq!(stats.inbound_bytes, 12_800);
    assert!(!stats.reader_parked);
    assert_eq!(client.try_recv_lane1(), Ok(Some(pat(1, 100))));
}

// ---------------------------------------------------------------------------
// M9
// ---------------------------------------------------------------------------

#[test]
fn m9_datagram_round_trip() {
    let mut server = bind_movement_server(ServerLimits::v1());
    let (mut client, conn) = accept_movement(&mut server, ClientLimits::v1(), 1, epoch(2));
    for bytes in [pat(1, 1), pat(2, 1_012), pat(3, 1_024)] {
        let got = datagram_until(&mut server, conn, &mut client, &bytes);
        assert_eq!(got, bytes);
    }
    // `wait` returns while only a datagram is queued.
    let marker = pat(4, 20);
    let start = Instant::now();
    loop {
        server.try_send_datagram(conn, &marker).unwrap();
        if client.wait(Duration::from_millis(100)) {
            break;
        }
        assert!(start.elapsed() < DEADLINE, "deadline: wait on datagram");
    }
    let stats = client.lane1_stats();
    assert!(stats.datagrams_queued >= 1, "{stats:?}");
    assert_eq!(client.try_recv_lane1(), Ok(None));
    assert_eq!(client.try_recv(), Ok(None));
    assert_eq!(client.try_recv_datagram(), Ok(Some(marker)));
    // The server never queues datagrams of its own.
    let server_stats = server.lane1_stats(conn).unwrap();
    assert_eq!(server_stats.datagrams_queued, 0);
    assert_eq!(server_stats.datagrams_dropped, 0);

    // A real sample: 38 entries fill one datagram of at most 1,012 bytes.
    let entries: Vec<SampleEntry> = (1..=38u64)
        .map(|i| SampleEntry {
            entity: NetId::new(i).unwrap(),
            point: Point::new([65_536 * i32::try_from(i).unwrap(), 0, 0]).unwrap(),
            moving: true,
        })
        .collect();
    let datagrams = sample_datagrams(5, 0, &entries).unwrap();
    assert_eq!(datagrams.len(), 1);
    assert!(datagrams[0].len() <= 1_012, "{}", datagrams[0].len());
    let got = datagram_until(&mut server, conn, &mut client, &datagrams[0]);
    assert_eq!(
        decode_down(&got).unwrap(),
        MoveDown::Sample(MoveSample {
            server_tick: 5,
            processed_seq: 0,
            entries,
        })
    );
}

// ---------------------------------------------------------------------------
// M10
// ---------------------------------------------------------------------------

fn id_of(datagram: &[u8]) -> u64 {
    u64::from_be_bytes(datagram.try_into().expect("8-byte datagram"))
}

#[test]
fn m10_client_keeps_newest_datagrams() {
    let mut server = bind_movement_server(ServerLimits::v1());
    let (mut client, conn) = accept_movement(&mut server, ClientLimits::v1(), 1, epoch(2));
    // The client reads no datagram. Cap: batches of 100 until its queue holds
    // 64 and has dropped at least one.
    let mut next_id = 0u64;
    let start = Instant::now();
    loop {
        for _ in 0..100 {
            server
                .try_send_datagram(conn, &next_id.to_be_bytes())
                .unwrap();
            next_id += 1;
        }
        client.wait(Duration::from_millis(100));
        let stats = client.lane1_stats();
        assert!(stats.datagrams_queued <= 64, "{stats:?}");
        if stats.datagrams_queued == 64 && stats.datagrams_dropped >= 1 {
            break;
        }
        assert!(start.elapsed() < DEADLINE, "deadline: cap");
    }
    // Newest kept: rounds of 100 ids and a marker; the first round that
    // arrives whole and ends in the marker asserts.
    let marker = u64::MAX.to_be_bytes();
    let mut rounds = 0;
    loop {
        rounds += 1;
        assert!(rounds < 200, "no whole round in 200 tries");
        assert!(start.elapsed() < DEADLINE, "deadline: newest kept");
        let before = client.lane1_stats();
        let base = next_id;
        for _ in 0..100 {
            server
                .try_send_datagram(conn, &next_id.to_be_bytes())
                .unwrap();
            next_id += 1;
        }
        server.try_send_datagram(conn, &marker).unwrap();
        // Every datagram of the round counts once, as queued or as a drop of
        // the oldest; poll until all 101 are in (a lost one never will be, and
        // that round is void).
        let round_start = Instant::now();
        let all_in = loop {
            let stats = client.lane1_stats();
            assert!(stats.datagrams_queued <= 64, "{stats:?}");
            let arrived = (stats.datagrams_dropped - before.datagrams_dropped) as i64
                + stats.datagrams_queued as i64
                - before.datagrams_queued as i64;
            if arrived == 101 {
                break true;
            }
            if round_start.elapsed() > Duration::from_secs(2) {
                break false;
            }
            std::thread::park_timeout(Duration::from_millis(1));
        };
        if !all_in {
            continue;
        }
        let mut drained: Vec<Vec<u8>> = Vec::new();
        while let Some(datagram) = client.try_recv_datagram().expect("open") {
            drained.push(datagram);
        }
        if drained.last().map(Vec::as_slice) != Some(marker.as_slice()) {
            continue;
        }
        let ids: Vec<u64> = drained.iter().map(|d| id_of(d)).collect();
        if ids[..ids.len() - 1].contains(&u64::MAX) {
            continue;
        }
        assert_eq!(drained.len(), 64);
        assert!(
            ids.windows(2).all(|pair| pair[0] < pair[1]),
            "ids strictly increasing: {ids:?}"
        );
        assert!(
            ids[..63].iter().all(|id| *id >= base + 37),
            "kept the newest 63: {ids:?}"
        );
        let expected: Vec<u64> = (base + 37..base + 100).collect();
        assert_eq!(&ids[..63], expected.as_slice());
        break;
    }
}

// ---------------------------------------------------------------------------
// M11
// ---------------------------------------------------------------------------

/// Reads lane 1 to its end: the frames, then the close reason.
fn client_lane1_all(client: &mut QuicClient) -> (Vec<Vec<u8>>, CloseReason) {
    let mut frames = Vec::new();
    loop {
        match client_lane1_next(client) {
            Ok(frame) => frames.push(frame),
            Err(reason) => return (frames, reason),
        }
    }
}

/// Reads lane 0 to its end: the frames, then the close reason.
fn client_lane0_all(client: &mut QuicClient) -> (Vec<Vec<u8>>, CloseReason) {
    let mut frames = Vec::new();
    loop {
        match client_recv(client) {
            Ok(frame) => frames.push(frame),
            Err(reason) => return (frames, reason),
        }
    }
}

#[test]
fn m11a_server_flush_drains_both_lanes() {
    let mut server = bind_movement_server(ServerLimits::v1());
    let (mut client, conn) = accept_movement(&mut server, ClientLimits::v1(), 1, epoch(2));
    prime(&mut client, &mut server, conn);
    for k in 1..=3 {
        server.try_send_lane1(conn, &pat(k, 60)).unwrap();
    }
    server.try_send(conn, &pat(4, 10)).unwrap();
    server.try_send(conn, &pat(5, 10)).unwrap();
    let t0 = Instant::now();
    let report = server
        .close(conn, CloseCode::Normal, CloseMode::Flush)
        .unwrap();
    let took = t0.elapsed();
    assert_eq!(report.outbound_frames_discarded, 0);
    // Prompt: the flush ended when both writers finished, not at `flush_end`.
    assert!(took < FLUSH_END, "flush took {took:?}");
    let (lane1, reason1) = client_lane1_all(&mut client);
    assert_eq!(lane1, vec![pat(1, 60), pat(2, 60), pat(3, 60)]);
    assert_eq!(reason1, CloseReason::Peer { code: 0 });
    let (lane0, reason0) = client_lane0_all(&mut client);
    assert_eq!(lane0, vec![pat(4, 10), pat(5, 10)]);
    assert_eq!(reason0, CloseReason::Peer { code: 0 });
}

/// A `Flush` waits for a lane-1 writer that still has a full queue to send:
/// 128 maximal frames cannot leave before the lane-0 writer is done.
#[test]
fn m11a_server_flush_waits_for_a_full_lane1_queue() {
    let mut server = bind_movement_server(ServerLimits::v1());
    let (mut client, conn) = accept_movement(&mut server, ClientLimits::v1(), 1, epoch(2));
    prime(&mut client, &mut server, conn);
    let frames: Vec<Vec<u8>> = (0..LANE1_OUTBOUND_FRAMES)
        .map(|k| pat(k, MAX_DATAGRAM_BYTES))
        .collect();
    for frame in &frames {
        server.try_send_lane1(conn, frame).unwrap();
    }
    server.try_send(conn, &pat(200, 10)).unwrap();
    let t0 = Instant::now();
    let report = server
        .close(conn, CloseCode::Normal, CloseMode::Flush)
        .unwrap();
    let took = t0.elapsed();
    assert_eq!(report.outbound_frames_discarded, 0);
    // The flush waited for the lane-1 writer (FIN) and returned promptly.
    assert!(took < FLUSH_END, "flush took {took:?}");
    let (lane1, reason1) = client_lane1_all(&mut client);
    assert_eq!(lane1, frames);
    assert_eq!(reason1, CloseReason::Peer { code: 0 });
    let (lane0, reason0) = client_lane0_all(&mut client);
    assert_eq!(lane0, vec![pat(200, 10)]);
    assert_eq!(reason0, CloseReason::Peer { code: 0 });
}

#[test]
fn m11b_client_flush_and_record_lifetime() {
    // Lane 0 reported first, lane 1 second.
    let mut server = bind_movement_server(ServerLimits::v1());
    let (mut client, conn) = accept_movement(&mut server, ClientLimits::v1(), 1, epoch(2));
    client.try_send_lane1(&h(1)).unwrap();
    client.try_send_lane1(&h(2)).unwrap();
    client.try_send(&pat(6, 10)).unwrap();
    client.close(CloseCode::Normal, CloseMode::Flush);
    let (closed, reason) = next_closed(&mut server);
    assert_eq!((closed, reason), (conn, CloseReason::Peer { code: 0 }));
    assert_eq!(
        server_channel_next(&mut server, conn),
        Ok((Channel::Lane0, pat(6, 10)))
    );
    assert_eq!(
        server_channel_next(&mut server, conn),
        Err(ServerError::Closed(CloseReason::Peer { code: 0 }))
    );
    // Lane 1 still holds frames: the record stays.
    assert!(server.stats(conn).is_ok());
    assert_eq!(server_lane1_next(&mut server, conn), Ok(h(1)));
    assert_eq!(server_lane1_next(&mut server, conn), Ok(h(2)));
    assert_eq!(
        server_lane1_next(&mut server, conn),
        Err(ServerError::Closed(CloseReason::Peer { code: 0 }))
    );
    assert_eq!(server.stats(conn), Err(ServerError::UnknownConnection));

    // Mirror order: lane 1 drained first.
    let (mut client, conn) = accept_movement(&mut server, ClientLimits::v1(), 2, epoch(2));
    client.try_send_lane1(&h(1)).unwrap();
    client.try_send_lane1(&h(2)).unwrap();
    client.try_send(&pat(6, 10)).unwrap();
    client.close(CloseCode::Normal, CloseMode::Flush);
    let (closed, reason) = next_closed(&mut server);
    assert_eq!((closed, reason), (conn, CloseReason::Peer { code: 0 }));
    assert_eq!(server_lane1_next(&mut server, conn), Ok(h(1)));
    assert_eq!(server_lane1_next(&mut server, conn), Ok(h(2)));
    assert_eq!(
        server_lane1_next(&mut server, conn),
        Err(ServerError::Closed(CloseReason::Peer { code: 0 }))
    );
    // Lane 0 still holds a frame: the record stays.
    assert!(server.stats(conn).is_ok());
    assert_eq!(
        server_channel_next(&mut server, conn),
        Ok((Channel::Lane0, pat(6, 10)))
    );
    assert_eq!(
        server_channel_next(&mut server, conn),
        Err(ServerError::Closed(CloseReason::Peer { code: 0 }))
    );
    assert_eq!(server.stats(conn), Err(ServerError::UnknownConnection));
}

/// N-11 (6) on the `poll_event` path: a record whose lane 0 held nothing and
/// whose lane 1 held frames survives the `Closed` event and is forgotten only
/// after lane 1 reports `Closed`.
#[test]
fn m11b_lane1_only_record_survives_the_event_poll() {
    let mut server = bind_movement_server(ServerLimits::v1());
    let (mut client, conn) = accept_movement(&mut server, ClientLimits::v1(), 1, epoch(2));
    // Lane 0 carries nothing at all; only lane 1 holds frames.
    client.try_send_lane1(&h(1)).unwrap();
    client.try_send_lane1(&h(2)).unwrap();
    client.close(CloseCode::Normal, CloseMode::Flush);
    let (closed, reason) = next_closed(&mut server);
    assert_eq!((closed, reason), (conn, CloseReason::Peer { code: 0 }));
    // The event poll must not forget a record lane 1 still owes frames from.
    assert!(
        server.stats(conn).is_ok(),
        "a lane-1-only record stays under its Closed event"
    );
    assert_eq!(server_lane1_next(&mut server, conn), Ok(h(1)));
    assert_eq!(server_lane1_next(&mut server, conn), Ok(h(2)));
    assert_eq!(
        server_lane1_next(&mut server, conn),
        Err(ServerError::Closed(CloseReason::Peer { code: 0 }))
    );
    assert_eq!(server.stats(conn), Err(ServerError::UnknownConnection));
}

/// N-11 (6) on the `poll_notice` path: the same lane-1-only record.
#[test]
fn m11b_lane1_only_record_survives_the_notice_poll() {
    let mut server = bind_movement_server(ServerLimits::v1());
    let pending = connect_movement_async(ClientLimits::v1(), server.local_addr(), hello4(1));
    let conn = match next_notice(&mut server) {
        ServerNotice::Event(ServerEvent::HelloReceived { conn, .. }) => conn,
        other => panic!("expected HelloReceived, got {other:?}"),
    };
    server
        .accept(
            conn,
            Welcome {
                wire_version: 4,
                epoch: epoch(2),
            },
        )
        .expect("accept");
    let mut client = pending.join().expect("thread").expect("connect");
    client.try_send_lane1(&h(1)).unwrap();
    client.close(CloseCode::Normal, CloseMode::Flush);
    match next_notice(&mut server) {
        ServerNotice::Event(ServerEvent::Closed {
            conn: closed,
            reason,
        }) => {
            assert_eq!(closed, conn);
            assert_eq!(reason, CloseReason::Peer { code: 0 });
        }
        other => panic!("expected Closed, got {other:?}"),
    }
    assert!(
        server.stats(conn).is_ok(),
        "a lane-1-only record stays under its Closed notice"
    );
    assert_eq!(server_lane1_next(&mut server, conn), Ok(h(1)));
    assert_eq!(
        server_lane1_next(&mut server, conn),
        Err(ServerError::Closed(CloseReason::Peer { code: 0 }))
    );
    assert_eq!(server.stats(conn), Err(ServerError::UnknownConnection));
}

/// N-11 (6): both lanes drain to `Closed` before the `Closed` event is polled;
/// the record survives until its event is polled, then is forgotten.
#[test]
fn m11b_record_survives_until_its_event_is_polled() {
    let mut server = bind_movement_server(ServerLimits::v1());
    let (mut client, conn) = accept_movement(&mut server, ClientLimits::v1(), 1, epoch(2));
    client.try_send_lane1(&h(1)).unwrap();
    client.try_send(&pat(6, 10)).unwrap();
    client.close(CloseCode::Normal, CloseMode::Flush);
    // Lane 0 and lane 1 both report Closed first.
    assert_eq!(
        server_channel_next(&mut server, conn),
        Ok((Channel::Lane0, pat(6, 10)))
    );
    assert_eq!(
        server_channel_next(&mut server, conn),
        Err(ServerError::Closed(CloseReason::Peer { code: 0 }))
    );
    assert_eq!(server_lane1_next(&mut server, conn), Ok(h(1)));
    assert_eq!(
        server_lane1_next(&mut server, conn),
        Err(ServerError::Closed(CloseReason::Peer { code: 0 }))
    );
    // Both claims reported, event still queued: the record stays.
    assert!(
        server.stats(conn).is_ok(),
        "a record waits for its Closed event"
    );
    assert_eq!(
        next_closed(&mut server),
        (conn, CloseReason::Peer { code: 0 })
    );
    assert_eq!(server.stats(conn), Err(ServerError::UnknownConnection));
}

/// N-11 (1) client side: a `Flush` with a full lane-1 queue waits for the
/// lane-1 writer, so every queued frame reaches the peer before the close.
#[test]
fn m11b_client_flush_waits_for_a_full_lane1_queue() {
    let mut server = bind_movement_server(ServerLimits::v1());
    let (mut client, conn) = accept_movement(&mut server, ClientLimits::v1(), 1, epoch(2));
    // The server reads no lane 1 while the client fills its 64-frame queue.
    let accepted = fill_client_lane1(&mut client, 100);
    assert_eq!(
        client.lane1_stats().outbound_frames,
        64,
        "the queue under the flush"
    );
    let closer = std::thread::spawn(move || client.close(CloseCode::Normal, CloseMode::Flush));
    let mut got: Vec<Vec<u8>> = Vec::new();
    let start = Instant::now();
    loop {
        match server.try_recv_lane1(conn) {
            Ok(Some(frame)) => got.push(frame),
            Ok(None) => {}
            Err(ServerError::Closed(reason)) => {
                assert_eq!(reason, CloseReason::Peer { code: 0 });
                break;
            }
            Err(other) => panic!("unexpected {other:?}"),
        }
        assert!(start.elapsed() < DEADLINE, "deadline: client flush drain");
        server.wait(Duration::from_millis(2));
    }
    let report = closer.join().expect("closer thread");
    assert_eq!(
        report.outbound_frames_discarded, 0,
        "a lane-1 queue with a writer is flushed, not discarded"
    );
    assert_eq!(got, accepted, "every queued lane-1 frame arrived, in order");
}

#[test]
fn m11c_datagrams_do_not_survive_a_close() {
    let mut server = bind_movement_server(ServerLimits::v1());
    let (mut client, conn) = accept_movement(&mut server, ClientLimits::v1(), 1, epoch(2));
    let bytes = pat(7, 20);
    let start = Instant::now();
    while client.lane1_stats().datagrams_queued < 1 {
        server.try_send_datagram(conn, &bytes).unwrap();
        client.wait(Duration::from_millis(100));
        assert!(start.elapsed() < DEADLINE, "deadline: datagram");
    }
    server
        .close(conn, CloseCode::Normal, CloseMode::Discard)
        .unwrap();
    assert_eq!(client_closed(&client), CloseReason::Peer { code: 0 });
    let mut seen = 0;
    let start = Instant::now();
    loop {
        match client.try_recv_datagram() {
            Ok(Some(datagram)) => {
                assert_eq!(datagram, bytes);
                seen += 1;
            }
            Ok(None) => {
                assert!(start.elapsed() < DEADLINE, "deadline: datagram close");
                client.wait(Duration::from_millis(20));
            }
            Err(reason) => {
                assert_eq!(reason, CloseReason::Peer { code: 0 });
                break;
            }
        }
    }
    assert!(seen >= 1, "the queued datagram stays drainable");
}

#[test]
fn m11d_flush_while_the_lane1_accept_is_pending() {
    let mut server = bind_movement_server(ServerLimits::v1());
    let (mut client, conn) = accept_movement(&mut server, ClientLimits::v1(), 1, epoch(2));
    // The client never writes on lane 1: the server has no lane-1 stream.
    for k in 1..=3 {
        server.try_send_lane1(conn, &pat(k, 60)).unwrap();
    }
    server.try_send(conn, &pat(4, 10)).unwrap();
    let t0 = Instant::now();
    let report = server
        .close(conn, CloseCode::Normal, CloseMode::Flush)
        .unwrap();
    let took = t0.elapsed();
    assert_eq!(report.outbound_frames_discarded, 3);
    // No lane-1 writer exists, so the flush waits for lane 0 alone and returns
    // well under `flush_end` (a mutant that waits for a stream that is never
    // made visible takes the whole 1,800 ms).
    assert!(
        took < FLUSH_END / 2,
        "a pending lane-1 accept must not delay the flush: {took:?}"
    );
    // The recorded close discarded the lane-1 queue (N-11 (2)).
    let stats = server.lane1_stats(conn).unwrap();
    assert_eq!((stats.outbound_frames, stats.outbound_bytes), (0, 0));
    let (lane0, reason0) = client_lane0_all(&mut client);
    assert_eq!(lane0, vec![pat(4, 10)]);
    assert_eq!(reason0, CloseReason::Peer { code: 0 });
    assert_eq!(
        client_lane1_next(&mut client),
        Err(CloseReason::Peer { code: 0 })
    );
}

#[test]
fn m11e_discard_counts_the_lane1_share() {
    let mut server = bind_movement_server(ServerLimits::v1());
    let (client, conn) = accept_movement(&mut server, ClientLimits::v1(), 1, epoch(2));
    for k in 1..=3 {
        server.try_send_lane1(conn, &pat(k, 60)).unwrap();
    }
    assert_eq!(server.lane1_stats(conn).unwrap().outbound_frames, 3);
    let report = server
        .close(conn, CloseCode::Normal, CloseMode::Discard)
        .unwrap();
    assert_eq!(report.outbound_frames_discarded, 3);
    assert_eq!(report.inbound_frames_discarded, 0);
    // N-11 (2): Discard empties the lane-1 outbound queue's accounting.
    let stats = server.lane1_stats(conn).unwrap();
    assert_eq!((stats.outbound_frames, stats.outbound_bytes), (0, 0));
    drop(client);
}

// ---------------------------------------------------------------------------
// M12
// ---------------------------------------------------------------------------

#[test]
fn m12_shutdown_flushes_lane1() {
    let mut server = bind_movement_server(ServerLimits::v1());
    let (mut client, conn) = accept_movement(&mut server, ClientLimits::v1(), 1, epoch(2));
    prime(&mut client, &mut server, conn);
    for k in 1..=3 {
        server.try_send_lane1(conn, &pat(k, 60)).unwrap();
    }
    let t0 = Instant::now();
    let report = server.shutdown();
    let took = t0.elapsed();
    assert_eq!(report.connections_closed, 1);
    assert_eq!(report.outbound_frames_discarded, 0);
    // Prompt: the flush ended on the lane-1 writer's FIN, not at `flush_end`.
    assert!(took < FLUSH_END, "shutdown took {took:?}");
    let (lane1, reason) = client_lane1_all(&mut client);
    assert_eq!(lane1, vec![pat(1, 60), pat(2, 60), pat(3, 60)]);
    assert_eq!(reason, CloseReason::Peer { code: 1 });
}

// ---------------------------------------------------------------------------
// M13
// ---------------------------------------------------------------------------

fn impaired() -> RelayConfig {
    RelayConfig {
        drop_every: 13,
        reorder_depth: 4,
        seed: 23,
    }
}

/// 300 lane-1 and 300 lane-0 frames from the client, alternating; the server
/// must receive each lane complete, in order, exactly once.
fn relay_exchange(server: &mut QuicServer, conn: ConnectionId, client: &mut QuicClient) {
    const COUNT: usize = 300;
    let (mut sent1, mut sent0, mut got1, mut got0) = (0usize, 0usize, 0usize, 0usize);
    let start = Instant::now();
    while got1 < COUNT || got0 < COUNT {
        let mut progress = false;
        if sent1 < COUNT {
            let frame = pat(sent1, cycling_size(u32::try_from(sent1).unwrap(), 4_096));
            match client.try_send_lane1(&frame) {
                Ok(()) => {
                    sent1 += 1;
                    progress = true;
                }
                Err(ChannelSendError::Send(SendError::QueueFull)) => {}
                Err(other) => panic!("lane 1 send {other:?}"),
            }
        }
        if sent0 < COUNT {
            let frame = pat(sent0, cycling_size(u32::try_from(sent0).unwrap(), 4_096));
            match client.try_send(&frame) {
                Ok(()) => {
                    sent0 += 1;
                    progress = true;
                }
                Err(SendError::QueueFull) => {}
                Err(other) => panic!("lane 0 send {other:?}"),
            }
        }
        while let Some(frame) = server.try_recv_lane1(conn).expect("lane 1 open") {
            assert!(got1 < COUNT, "extra lane-1 frame");
            assert_eq!(
                frame,
                pat(got1, cycling_size(u32::try_from(got1).unwrap(), 4_096))
            );
            got1 += 1;
            progress = true;
        }
        while let Some(frame) = server.try_recv(conn).expect("lane 0 open") {
            assert!(got0 < COUNT, "extra lane-0 frame");
            assert_eq!(
                frame,
                pat(got0, cycling_size(u32::try_from(got0).unwrap(), 4_096))
            );
            got0 += 1;
            progress = true;
        }
        assert!(start.elapsed() < DEADLINE, "deadline: relay exchange");
        if !progress {
            server.wait(Duration::from_millis(1));
        }
    }
    assert_eq!(server.try_recv_lane1(conn), Ok(None));
    assert_eq!(server.try_recv(conn), Ok(None));
}

#[test]
fn m13_lanes_through_impaired_relay() {
    {
        let mut server = bind_movement_server(ServerLimits::v1());
        let relay = UdpRelay::start(server.local_addr(), impaired());
        let pending = connect_movement_async(ClientLimits::v1(), relay.addr(), hello4(1));
        let (conn, _, _) = next_hello(&mut server);
        server
            .accept(
                conn,
                Welcome {
                    wire_version: 4,
                    epoch: epoch(14),
                },
            )
            .expect("accept");
        let mut client = pending.join().expect("thread").expect("connect via relay");
        // The handshake ran clean; impair the exchange, which is what this
        // case measures.
        relay.set_impaired(true);
        relay_exchange(&mut server, conn, &mut client);
        assert!(relay.dropped() > 0, "impairment control: dropped");
        assert!(relay.reordered() > 0, "impairment control: reordered");
        // 40 datagrams, once each; then a lane-1 frame proves the stream
        // path is still ordered behind them.
        for i in 0..40usize {
            server.try_send_datagram(conn, &pat(i, 100)).unwrap();
        }
        server.try_send_lane1(conn, &pat(0, 1)).unwrap();
        assert_eq!(client_recv_lane1(&mut client), pat(0, 1));
        let mut seen = BTreeSet::new();
        while let Some(datagram) = client.try_recv_datagram().expect("open") {
            let index = (0..40usize)
                .find(|i| datagram == pat(*i, 100))
                .unwrap_or_else(|| panic!("a datagram that was never sent: {datagram:?}"));
            assert!(seen.insert(index), "datagram {index} repeated");
        }
    }
    // Control: the same 300 + 300 through a clean relay.
    let mut server = bind_movement_server(ServerLimits::v1());
    let relay = UdpRelay::start(server.local_addr(), RelayConfig::clean());
    let pending = connect_movement_async(ClientLimits::v1(), relay.addr(), hello4(1));
    let (conn, _, _) = next_hello(&mut server);
    server
        .accept(
            conn,
            Welcome {
                wire_version: 4,
                epoch: epoch(14),
            },
        )
        .expect("accept");
    let mut client = pending.join().expect("thread").expect("connect via relay");
    relay_exchange(&mut server, conn, &mut client);
    assert_eq!(relay.dropped(), 0);
}

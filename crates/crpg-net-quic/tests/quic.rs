//! T023 endpoint suite (R8.2): E§13.2 cases 1–22 plus case 23, all through
//! the public `crpg_net_quic` API over real QUIC on IPv4 loopback.
//!
//! T023c (C§8) adds cases 24–27 for the flow-control window limits: their
//! validation (24), the exact stall bound a stalled client's windows put on
//! the server's queue (25) and a stalled server's windows on the client's
//! (26), and maximal frames at the floor windows (27).
//!
//! T023v adds case 28: a `Discard` close reaches the peer as its close code
//! while the server is congestion blocked (vendored quinn-proto fix,
//! `third_party/quinn-proto/VENDOR.md`).
//!
//! T023d adds cases 29–30: a refusal (29) or a pending `Discard` close (30)
//! still reaches the client as its code when the host polls `Closed` at once.
//!
//! Every rejection has a positive control on the same server. Waits are
//! 30 s failure guards from `support/quic_rig.rs`, never oracles.

#[path = "support/quic_rig.rs"]
mod quic_rig;

use std::collections::BTreeMap;
use std::time::Duration;

use crpg_net::protocol::{
    DeltaFrame, DeltaOp, IntentBody, IntentFrame, NetId, ReceiptStatus, LANE_COMBAT,
};
use crpg_net::transport::Transport;
use crpg_net::{codec, codec_v2, protocol_v2};
use crpg_net_quic::{
    frame_header, parse_frame_header, BindError, CertificatePin, ClientLimits, CloseCode,
    CloseMode, CloseReason, ConfigError, ConnectError, ConnectionId, Credential, CredentialError,
    FrameError, HandshakeError, Hello, QuicClient, QuicServer, SendError, ServerConfig,
    ServerError, ServerEvent, ServerIdentity, ServerLimits, Welcome, ALPN_LANE0_V1,
    FRAME_HEADER_BYTES, MAX_CREDENTIAL_BYTES, MAX_HELLO_BYTES, MIN_CREDENTIAL_BYTES,
    TLS_SERVER_NAME, WELCOME_BYTES,
};
use quic_rig::*;

/// SHA-256 of `tests/fixtures/server_a.cert.der`, computed independently
/// with `sha256sum` when the fixture was generated.
const PIN_A_HEX: &str = "9da1730bd38407644e8677d66f182438b9eec028f4d2b538e14d352a26d7696b";
/// SHA-256 of `tests/fixtures/server_b.cert.der` (`sha256sum`).
const PIN_B_HEX: &str = "c97480c61dc5eff8f9f4fe11a5378f4e1d434e5b5fec2f2b70c88f9bb4e46f27";

const INTENT_CAP: usize = 4_096;
const DELTA_CAP: usize = 65_536;

fn hello_frame(wire_version: u8, cred_len: usize) -> Vec<u8> {
    let hello = Hello {
        wire_version,
        credential: credential(cred_len, 0x42),
    };
    frame_bytes(&hello.encode().expect("hello"))
}

fn queue_fields(stats: crpg_net_quic::ConnectionStats) -> (usize, usize, usize, usize, bool) {
    (
        stats.inbound_frames,
        stats.inbound_bytes,
        stats.outbound_frames,
        stats.outbound_bytes,
        stats.reader_parked,
    )
}

// ---------------------------------------------------------------------------
// 1–4: bytes, credentials, pins, limits.
// ---------------------------------------------------------------------------

#[test]
fn frame_and_handshake_bytes_are_exact() {
    // §4 literal vectors.
    let hello = Hello {
        wire_version: 1,
        credential: credential(16, 0x11),
    };
    let payload = hello.encode().expect("encode");
    let mut expected = vec![0x01, 0x10];
    expected.extend_from_slice(&[0x11; 16]);
    assert_eq!(payload, expected);
    assert_eq!(
        frame_header(payload.len(), MAX_HELLO_BYTES),
        Ok([0x00, 0x00, 0x00, 0x12])
    );
    assert_eq!(frame_header(3, 4096), Ok([0x00, 0x00, 0x00, 0x03]));
    assert_eq!(MAX_HELLO_BYTES, 130);
    assert_eq!(WELCOME_BYTES, 17);
    assert_eq!(FRAME_HEADER_BYTES, 4);
    assert_eq!(ALPN_LANE0_V1, b"crpg-lane0/1");
    assert_eq!(TLS_SERVER_NAME, "crpg.invalid");
    let decoded = Hello::decode(&payload).expect("decode");
    assert_eq!(decoded.wire_version, 1);
    assert!(decoded.credential.ct_eq(&[0x11; 16]));

    // Hello::decode precedence.
    let cred16 = [0x22u8; 16];
    let with = |version: u8, len_byte: u8, body: &[u8]| {
        let mut out = vec![version, len_byte];
        out.extend_from_slice(body);
        out
    };
    assert_eq!(Hello::decode(&[]).unwrap_err(), HandshakeError::Malformed);
    assert_eq!(Hello::decode(&[1]).unwrap_err(), HandshakeError::Malformed);
    assert_eq!(Hello::decode(&[3]).unwrap_err(), HandshakeError::Malformed);
    assert_eq!(
        Hello::decode(&with(3, 16, &cred16)).unwrap_err(),
        HandshakeError::UnsupportedWireVersion
    );
    assert_eq!(
        Hello::decode(&with(0, 15, &[0; 15])).unwrap_err(),
        HandshakeError::UnsupportedWireVersion
    );
    assert_eq!(
        Hello::decode(&with(1, 15, &[0; 15])).unwrap_err(),
        HandshakeError::CredentialLength
    );
    assert_eq!(
        Hello::decode(&with(1, 129, &[0; 129])).unwrap_err(),
        HandshakeError::CredentialLength
    );
    assert_eq!(
        Hello::decode(&with(1, 16, &[0; 15])).unwrap_err(),
        HandshakeError::Malformed
    );
    assert_eq!(
        Hello::decode(&with(1, 16, &[0; 17])).unwrap_err(),
        HandshakeError::Malformed
    );
    let v2 = Hello::decode(&with(2, 128, &[7; 128])).expect("wire 2, 128-byte credential");
    assert_eq!(v2.wire_version, 2);
    assert_eq!(v2.credential.as_bytes(), &[7u8; 128][..]);
    assert_eq!(
        Hello {
            wire_version: 3,
            credential: credential(16, 0)
        }
        .encode()
        .unwrap_err(),
        HandshakeError::UnsupportedWireVersion
    );

    // Welcome bytes and decode precedence.
    let welcome = Welcome {
        wire_version: 2,
        epoch: [0xAB; 16],
    };
    let bytes = welcome.encode().expect("welcome");
    let mut expected = [0xAB; 17];
    expected[0] = 2;
    assert_eq!(bytes, expected);
    assert_eq!(Welcome::decode(&bytes), Ok(welcome));
    assert_eq!(
        Welcome::decode(&bytes[..16]),
        Err(HandshakeError::Malformed)
    );
    assert_eq!(Welcome::decode(&[3; 16]), Err(HandshakeError::Malformed));
    let mut long = bytes.to_vec();
    long.push(0);
    assert_eq!(Welcome::decode(&long), Err(HandshakeError::Malformed));
    assert_eq!(
        Welcome::decode(&[3; 17]),
        Err(HandshakeError::UnsupportedWireVersion)
    );
    assert_eq!(
        Welcome {
            wire_version: 0,
            epoch: [0; 16]
        }
        .encode(),
        Err(HandshakeError::UnsupportedWireVersion)
    );

    // Frame header bounds.
    assert_eq!(frame_header(0, 4096), Err(FrameError::Empty));
    assert_eq!(frame_header(4097, 4096), Err(FrameError::TooLarge));
    assert_eq!(frame_header(4096, 4096), Ok([0, 0, 0x10, 0]));
    assert_eq!(
        parse_frame_header([0xFF; 4], 4096),
        Err(FrameError::TooLarge)
    );
    assert_eq!(parse_frame_header([0; 4], 4096), Err(FrameError::Empty));
    assert_eq!(parse_frame_header([0, 0, 0x10, 0], 4096), Ok(4096));
    assert_eq!(
        parse_frame_header([0, 0, 0x10, 1], 4096),
        Err(FrameError::TooLarge)
    );

    // Close codes.
    let names = [
        "Normal",
        "Shutdown",
        "ProtocolViolation",
        "FrameTooLarge",
        "HelloTimeout",
        "AuthRefused",
        "VersionRefused",
        "ServerBusy",
        "AuthTimeout",
        "SessionFenced",
    ];
    for (raw, name) in names.iter().enumerate() {
        let code = CloseCode::from_u64(raw as u64).expect("code 0..=9");
        assert_eq!(code.as_u32() as usize, raw);
        assert_eq!(code.name(), *name);
        assert!(code.name().len() <= 17 && code.name().is_ascii());
    }
    assert_eq!(CloseCode::from_u64(10), None);
    assert_eq!(CloseCode::from_u64(u64::MAX), None);

    // Display: `<VariantName> at quic/<area>`.
    assert_eq!(FrameError::Empty.to_string(), "Empty at quic/frame");
    assert_eq!(
        CredentialError::TooLong.to_string(),
        "TooLong at quic/credential"
    );
    assert_eq!(
        HandshakeError::CredentialLength.to_string(),
        "CredentialLength at quic/handshake"
    );
    assert_eq!(
        ConfigError::InvalidLimits { field: "x" }.to_string(),
        "InvalidLimits at quic/config"
    );
    assert_eq!(
        SendError::Closed(CloseReason::Transport).to_string(),
        "Closed at quic/send"
    );
    assert_eq!(
        ServerError::Closed(CloseReason::Reset).to_string(),
        "Closed at quic/server"
    );
    assert_eq!(
        BindError::Io(std::io::ErrorKind::AddrInUse).to_string(),
        "Io at quic/bind"
    );
    assert_eq!(
        ConnectError::Refused { code: 5 }.to_string(),
        "Refused at quic/connect"
    );
}

#[test]
fn credential_bounds_and_redaction() {
    assert_eq!(MIN_CREDENTIAL_BYTES, 16);
    assert_eq!(MAX_CREDENTIAL_BYTES, 128);
    assert_eq!(
        Credential::new(vec![1; 15]).err(),
        Some(CredentialError::TooShort)
    );
    assert!(Credential::new(vec![1; 16]).is_ok());
    assert!(Credential::new(vec![1; 128]).is_ok());
    assert_eq!(
        Credential::new(vec![1; 129]).err(),
        Some(CredentialError::TooLong)
    );

    let secret = vec![0x5A; 20];
    let credential = Credential::new(secret.clone()).expect("credential");
    assert_eq!(format!("{credential:?}"), "Credential(<redacted>)");
    let hello = Hello {
        wire_version: 1,
        credential: credential.clone(),
    };
    let hello_debug = format!("{hello:?}");
    assert!(
        hello_debug.contains("Credential(<redacted>)"),
        "{hello_debug}"
    );
    assert!(!hello_debug.contains("90"), "{hello_debug}"); // 0x5A = 90
    let event = ServerEvent::HelloReceived {
        conn: connection_id_of(1),
        remote: loopback(),
        hello,
    };
    let event_debug = format!("{event:?}");
    assert!(
        event_debug.contains("Credential(<redacted>)"),
        "{event_debug}"
    );
    assert!(!event_debug.contains("90,"), "{event_debug}");

    assert!(credential.ct_eq(&secret));
    let mut other = secret.clone();
    other[19] ^= 1;
    assert!(!credential.ct_eq(&other), "content differs");
    assert!(!credential.ct_eq(&secret[..19]), "length differs");
}

/// A `ConnectionId` with the given raw value, minted by a real server: ids
/// start at 1 and are never reused.
fn connection_id_of(raw: u64) -> ConnectionId {
    let mut server = bind_server(ServerLimits::v1());
    let mut last = None;
    for _ in 0..raw {
        let raw_client = raw_lane(server.local_addr());
        raw_client.open_and_write(&hello_frame(1, 16));
        let (conn, _, _) = next_hello(&mut server);
        last = Some(conn);
    }
    let conn = last.expect("at least one connection");
    assert_eq!(conn.get(), raw);
    conn
}

#[test]
fn certificate_pin_hex_and_fixture_values() {
    let pin_a = CertificatePin::of_certificate(SERVER_A_CERT);
    let pin_b = CertificatePin::of_certificate(SERVER_B_CERT);
    assert_eq!(pin_a.to_hex(), PIN_A_HEX);
    assert_eq!(pin_b.to_hex(), PIN_B_HEX);
    assert_ne!(pin_a, pin_b);
    assert_eq!(CertificatePin::from_hex(PIN_A_HEX), Ok(pin_a));
    assert_eq!(
        CertificatePin::from_sha256(*pin_a.as_bytes()),
        pin_a,
        "round trip through the digest bytes"
    );
    assert_eq!(format!("{pin_a:?}"), format!("CertificatePin({PIN_A_HEX})"));
    assert_eq!(identity_a().pin(), pin_a);

    for bad in [
        &PIN_A_HEX[..63],
        &format!("{PIN_A_HEX}0"),
        &PIN_A_HEX.to_uppercase(),
        &format!("{}g", &PIN_A_HEX[..63]),
        "",
    ] {
        assert_eq!(
            CertificatePin::from_hex(bad),
            Err(ConfigError::InvalidPin),
            "{bad}"
        );
    }

    let identity_debug = format!("{:?}", identity_a());
    assert!(identity_debug.contains(PIN_A_HEX), "{identity_debug}");
    assert!(
        identity_debug.contains("key: <redacted>"),
        "{identity_debug}"
    );
}

/// A named limits mutation.
type Case<L> = (&'static str, Box<dyn Fn(&mut L)>);

#[test]
fn limits_validate_and_refuse_loosening() {
    let server_v1 = ServerLimits::v1();
    let client_v1 = ClientLimits::v1();
    assert_eq!(server_v1.validate(), Ok(()));
    assert_eq!(client_v1.validate(), Ok(()));
    assert_eq!(server_v1.max_connections, 64);
    assert_eq!(server_v1.idle_timeout_ms, 60_000);
    assert_eq!(server_v1.keep_alive_ms, 15_000);
    assert_eq!(server_v1.hello_timeout_ms, 5_000);
    assert_eq!(server_v1.decision_timeout_ms, 5_000);
    assert_eq!(server_v1.close_flush_timeout_ms, 2_000);
    assert_eq!(server_v1.inbound, crpg_net::sim::QueueCaps::v1());
    assert_eq!(server_v1.outbound, crpg_net::sim::QueueCaps::v1_egress());
    assert_eq!(client_v1.idle_timeout_ms, 60_000);
    assert_eq!(client_v1.keep_alive_ms, 15_000);
    assert_eq!(client_v1.connect_timeout_ms, 10_000);
    assert_eq!(
        (
            client_v1.inbound_frames,
            client_v1.inbound_bytes,
            client_v1.outbound_frames,
            client_v1.outbound_bytes
        ),
        (128, 2_097_152, 128, 262_144)
    );

    let loosens = |field| Err(ConfigError::LoosensPolicy { field });
    let invalid = |field| Err(ConfigError::InvalidLimits { field });

    // One step past v1 on every policy field. (`keep_alive_ms` is not a
    // policy cap; `outbound.host_frames` is already `usize::MAX`.)
    let server_cases: Vec<Case<ServerLimits>> = vec![
        ("idle_timeout_ms", Box::new(|l| l.idle_timeout_ms += 1)),
        ("hello_timeout_ms", Box::new(|l| l.hello_timeout_ms += 1)),
        (
            "decision_timeout_ms",
            Box::new(|l| l.decision_timeout_ms += 1),
        ),
        (
            "close_flush_timeout_ms",
            Box::new(|l| l.close_flush_timeout_ms += 1),
        ),
        ("max_connections", Box::new(|l| l.max_connections += 1)),
        (
            "inbound.per_peer_frames",
            Box::new(|l| l.inbound.per_peer_frames += 1),
        ),
        (
            "inbound.per_peer_bytes",
            Box::new(|l| l.inbound.per_peer_bytes += 1),
        ),
        (
            "inbound.host_frames",
            Box::new(|l| l.inbound.host_frames += 1),
        ),
        (
            "inbound.host_bytes",
            Box::new(|l| l.inbound.host_bytes += 1),
        ),
        (
            "outbound.per_peer_frames",
            Box::new(|l| l.outbound.per_peer_frames += 1),
        ),
        (
            "outbound.per_peer_bytes",
            Box::new(|l| l.outbound.per_peer_bytes += 1),
        ),
        (
            "outbound.host_bytes",
            Box::new(|l| l.outbound.host_bytes += 1),
        ),
    ];
    for (field, change) in &server_cases {
        let mut limits = ServerLimits::v1();
        change(&mut limits);
        assert_eq!(limits.validate(), loosens(*field), "{field}");
    }
    let mut keep_alive = ServerLimits::v1();
    keep_alive.keep_alive_ms += 1;
    assert_eq!(
        keep_alive.validate(),
        Ok(()),
        "keep-alive is range-checked only"
    );

    let client_cases: Vec<Case<ClientLimits>> = vec![
        ("idle_timeout_ms", Box::new(|l| l.idle_timeout_ms += 1)),
        (
            "connect_timeout_ms",
            Box::new(|l| l.connect_timeout_ms += 1),
        ),
        (
            "close_flush_timeout_ms",
            Box::new(|l| l.close_flush_timeout_ms += 1),
        ),
        ("inbound_frames", Box::new(|l| l.inbound_frames += 1)),
        ("inbound_bytes", Box::new(|l| l.inbound_bytes += 1)),
        ("outbound_frames", Box::new(|l| l.outbound_frames += 1)),
        ("outbound_bytes", Box::new(|l| l.outbound_bytes += 1)),
    ];
    for (field, change) in &client_cases {
        let mut limits = ClientLimits::v1();
        change(&mut limits);
        assert_eq!(limits.validate(), loosens(*field), "{field}");
    }

    // Zero, below floors, below one maximal frame, inconsistent.
    let server_invalid: Vec<Case<ServerLimits>> = vec![
        ("idle_timeout_ms", Box::new(|l| l.idle_timeout_ms = 999)),
        ("keep_alive_ms", Box::new(|l| l.keep_alive_ms = 99)),
        (
            "keep_alive_ms",
            Box::new(|l| l.keep_alive_ms = l.idle_timeout_ms),
        ),
        ("hello_timeout_ms", Box::new(|l| l.hello_timeout_ms = 99)),
        (
            "decision_timeout_ms",
            Box::new(|l| l.decision_timeout_ms = 0),
        ),
        ("max_connections", Box::new(|l| l.max_connections = 0)),
        (
            "inbound.per_peer_frames",
            Box::new(|l| l.inbound.per_peer_frames = 0),
        ),
        (
            "inbound.per_peer_bytes",
            Box::new(|l| l.inbound.per_peer_bytes = INTENT_CAP - 1),
        ),
        (
            "inbound.host_frames",
            Box::new(|l| l.inbound.host_frames = l.inbound.per_peer_frames - 1),
        ),
        (
            "inbound.host_bytes",
            Box::new(|l| l.inbound.host_bytes = l.inbound.per_peer_bytes - 1),
        ),
        (
            "outbound.per_peer_bytes",
            Box::new(|l| l.outbound.per_peer_bytes = DELTA_CAP - 1),
        ),
        (
            "outbound.host_frames",
            Box::new(|l| l.outbound.host_frames = l.outbound.per_peer_frames - 1),
        ),
    ];
    for (field, change) in &server_invalid {
        let mut limits = ServerLimits::v1();
        change(&mut limits);
        assert_eq!(limits.validate(), invalid(*field), "{field}");
    }
    let mut off = ServerLimits::v1();
    off.keep_alive_ms = 0;
    off.close_flush_timeout_ms = 0;
    assert_eq!(
        off.validate(),
        Ok(()),
        "keep-alive off and zero flush are valid"
    );
    // Declaration order: the first failing field wins, InvalidLimits first.
    let mut two = ServerLimits::v1();
    two.hello_timeout_ms = 6_000;
    two.idle_timeout_ms = 0;
    assert_eq!(two.validate(), invalid("idle_timeout_ms"));

    let client_invalid: Vec<Case<ClientLimits>> = vec![
        ("idle_timeout_ms", Box::new(|l| l.idle_timeout_ms = 0)),
        ("keep_alive_ms", Box::new(|l| l.keep_alive_ms = 60_000)),
        (
            "connect_timeout_ms",
            Box::new(|l| l.connect_timeout_ms = 99),
        ),
        ("inbound_frames", Box::new(|l| l.inbound_frames = 0)),
        (
            "inbound_bytes",
            Box::new(|l| l.inbound_bytes = DELTA_CAP - 1),
        ),
        ("outbound_frames", Box::new(|l| l.outbound_frames = 0)),
        (
            "outbound_bytes",
            Box::new(|l| l.outbound_bytes = INTENT_CAP - 1),
        ),
    ];
    for (field, change) in &client_invalid {
        let mut limits = ClientLimits::v1();
        change(&mut limits);
        assert_eq!(limits.validate(), invalid(*field), "{field}");
    }

    // bind/connect surface the config error before any I/O.
    let mut bad_server = ServerLimits::v1();
    bad_server.max_connections = 65;
    let bound = QuicServer::bind(ServerConfig {
        bind: loopback(),
        identity: identity_a(),
        limits: bad_server,
    });
    assert_eq!(
        bound.err(),
        Some(BindError::Config(ConfigError::LoosensPolicy {
            field: "max_connections"
        }))
    );
    let mut bad_client = ClientLimits::v1();
    bad_client.outbound_frames = 0;
    let connected = QuicClient::connect(client_config(bad_client), loopback(), &hello(1));
    assert_eq!(
        connected.err(),
        Some(ConnectError::Config(ConfigError::InvalidLimits {
            field: "outbound_frames"
        }))
    );
    // A key that does not match the certificate is refused at bind.
    let mismatched = QuicServer::bind(ServerConfig {
        bind: loopback(),
        identity: ServerIdentity::new(SERVER_A_CERT.to_vec(), SERVER_B_KEY.to_vec()),
        limits: ServerLimits::v1(),
    });
    assert_eq!(mismatched.err(), Some(BindError::Identity));
    // Positive control: the matching pair binds.
    let server_b = QuicServer::bind(ServerConfig {
        bind: loopback(),
        identity: ServerIdentity::new(SERVER_B_CERT.to_vec(), SERVER_B_KEY.to_vec()),
        limits: ServerLimits::v1(),
    });
    assert!(server_b.is_ok());
}

// ---------------------------------------------------------------------------
// 5–11: handshake.
// ---------------------------------------------------------------------------

fn end_turn(epoch: [u8; 16], seq: u64) -> IntentFrame {
    IntentFrame {
        epoch,
        lane: LANE_COMBAT,
        seq,
        observed_tick: 0,
        actor: NetId::new(1).expect("nonzero"),
        body: IntentBody::EndTurn,
    }
}

fn receipt(epoch: [u8; 16], seq: u64) -> DeltaOp {
    DeltaOp::Receipt {
        epoch,
        lane: LANE_COMBAT,
        seq,
        processed_tick: seq,
        status: ReceiptStatus::Applied,
    }
}

fn receipt_delta(epoch: [u8; 16], seq: u64) -> DeltaFrame {
    DeltaFrame {
        lane: LANE_COMBAT,
        server_tick: seq,
        event_seq: seq,
        ops: vec![receipt(epoch, seq)],
    }
}

#[test]
fn loopback_handshake_and_round_trip() {
    for wire_version in [1u8, 2] {
        let mut server = bind_server(ServerLimits::v1());
        let secret = vec![0xC3; 40];
        let pending = connect_async(
            client_config(ClientLimits::v1()),
            server.local_addr(),
            Hello {
                wire_version,
                credential: Credential::new(secret.clone()).expect("credential"),
            },
        );
        let (conn, remote, hello) = next_hello(&mut server);
        assert_eq!(hello.wire_version, wire_version);
        assert!(hello.credential.ct_eq(&secret));
        let welcome = Welcome {
            wire_version,
            epoch: epoch(0xE0 + wire_version),
        };
        server.accept(conn, welcome).expect("accept");
        let mut client = pending.join().expect("thread").expect("connect");
        assert_eq!(remote, client.local_addr());
        assert_eq!(client.welcome(), welcome);

        let intent = end_turn(client.welcome().epoch, 1);
        let intent_bytes = if wire_version == 1 {
            codec::encode_intent(&intent).expect("v1 intent")
        } else {
            codec_v2::encode_intent(&intent).expect("v2 intent")
        };
        client.try_send(&intent_bytes).expect("send intent");
        let got = server_recv(&mut server, conn).expect("intent");
        assert_eq!(got, intent_bytes);
        let decoded = if wire_version == 1 {
            codec::decode_intent(&got).expect("decode v1")
        } else {
            codec_v2::decode_intent(&got).expect("decode v2")
        };
        assert_eq!(decoded, intent);

        if wire_version == 1 {
            let delta = receipt_delta(welcome.epoch, 1);
            let bytes = codec::encode_delta(&delta).expect("v1 delta");
            server.try_send(conn, &bytes).expect("send delta");
            let got = client_recv(&mut client).expect("delta");
            assert_eq!(got, bytes);
            assert_eq!(codec::decode_delta(&got).expect("decode"), delta);
        } else {
            let delta = protocol_v2::DeltaFrame {
                lane: LANE_COMBAT,
                server_tick: 1,
                event_seq: 1,
                ops: vec![protocol_v2::DeltaOp::Legacy(receipt(welcome.epoch, 1))],
            };
            let bytes = codec_v2::encode_delta(&delta).expect("v2 delta");
            server.try_send(conn, &bytes).expect("send delta");
            let got = client_recv(&mut client).expect("delta");
            assert_eq!(got, bytes);
            assert_eq!(codec_v2::decode_delta(&got).expect("decode"), delta);
        }
    }
}

#[test]
fn wrong_certificate_pin_refused() {
    let mut server = bind_server(ServerLimits::v1());
    let mut config = client_config(ClientLimits::v1());
    config.pin = pin_b();
    let before = server.totals().refused_handshakes;
    let refused = QuicClient::connect(config, server.local_addr(), &hello(1));
    assert_eq!(refused.err(), Some(ConnectError::PinMismatch));
    until("refused handshake counted", || {
        server.totals().refused_handshakes == before + 1
    });
    assert!(server.poll_event().is_none(), "no HelloReceived");

    // Control: the correct pin on the same server.
    let (_client, conn) = connect_accepted(&mut server, ClientLimits::v1(), 1, epoch(1));
    assert_eq!(conn.get(), 1);
    assert_eq!(server.totals().refused_handshakes, before + 1);
}

#[test]
fn foreign_alpn_refused() {
    let mut server = bind_server(ServerLimits::v1());
    let foreign = raw_connect(server.local_addr(), b"crpg-lane0/2");
    assert!(foreign.is_err(), "foreign ALPN fails the handshake");
    until("refused handshake counted", || {
        server.totals().refused_handshakes == 1
    });
    assert!(server.poll_event().is_none());

    // Control: the right ALPN reaches HelloReceived.
    let raw = raw_lane(server.local_addr());
    raw.open_and_write(&hello_frame(1, 16));
    let (_conn, _, hello) = next_hello(&mut server);
    assert_eq!(hello.wire_version, 1);
    assert_eq!(server.totals().refused_handshakes, 1);
}

#[test]
fn malformed_hello_refused_with_exact_codes() {
    let mut server = bind_server(ServerLimits::v1());
    let mut cases: Vec<(&str, Vec<u8>, bool, u64)> = Vec::new();
    cases.push(("header 131", vec![0, 0, 0, 131], false, 3));
    cases.push(("zero length", vec![0, 0, 0, 0], false, 2));
    let mut short = vec![0, 0, 0, 17, 1, 15];
    short.extend_from_slice(&[9; 15]);
    cases.push(("cred_len 15", short, false, 2));
    let mut disagree = vec![0, 0, 0, 22, 1, 16];
    disagree.extend_from_slice(&[9; 20]);
    cases.push(("cred_len disagrees", disagree, false, 2));
    let mut version3 = vec![0, 0, 0, 18, 3, 16];
    version3.extend_from_slice(&[9; 16]);
    cases.push(("wire version 3", version3, false, 6));
    let mut partial = vec![0, 0, 0, 18, 1, 16];
    partial.extend_from_slice(&[9; 5]);
    cases.push(("FIN before complete hello", partial, true, 2));

    for (expected_refusals, (name, bytes, fin, code)) in (1u64..).zip(cases) {
        let raw = raw_lane(server.local_addr());
        let (mut send, _recv) = raw.open_and_write(&bytes);
        if fin {
            raw.finish(&mut send);
        }
        assert_eq!(raw.closed_code(), Some(code), "{name}");
        until(name, || {
            server.totals().refused_handshakes == expected_refusals
        });
        assert!(server.poll_event().is_none(), "{name}: no event");
    }

    // Control: 16- and 128-byte credentials both reach HelloReceived.
    for len in [16, 128] {
        let raw = raw_lane(server.local_addr());
        raw.open_and_write(&hello_frame(2, len));
        let (_, _, hello) = next_hello(&mut server);
        assert_eq!(hello.credential.as_bytes().len(), len);
        assert_eq!(hello.wire_version, 2);
    }
}

#[test]
fn hello_timeout_closes_silent_client() {
    let mut limits = ServerLimits::v1();
    limits.hello_timeout_ms = 200;
    let mut server = bind_server(limits);
    let raw = raw_lane(server.local_addr());
    let (_send, _recv) = raw.open_and_write(&[]);
    assert_eq!(raw.closed_code(), Some(4));
    until("refused handshake counted", || {
        server.totals().refused_handshakes == 1
    });
    assert!(server.poll_event().is_none());

    // Control: a prompt hello on the same server is not timed out.
    let raw = raw_lane(server.local_addr());
    raw.open_and_write(&hello_frame(1, 16));
    let (conn, _, _) = next_hello(&mut server);
    assert_eq!(conn.get(), 2);
}

#[test]
fn host_refusals_reach_client_exactly() {
    let mut server = bind_server(ServerLimits::v1());
    for code in [
        CloseCode::AuthRefused,
        CloseCode::VersionRefused,
        CloseCode::ServerBusy,
    ] {
        let pending = connect_async(
            client_config(ClientLimits::v1()),
            server.local_addr(),
            hello(1),
        );
        let (conn, _, _) = next_hello(&mut server);
        server.refuse(conn, code).expect("refuse");
        let refused = pending.join().expect("thread");
        assert_eq!(
            refused.err(),
            Some(ConnectError::Refused {
                code: u64::from(code.as_u32())
            })
        );
        assert_eq!(next_closed(&mut server), (conn, CloseReason::Local(code)));
        assert_eq!(server.try_recv(conn), Err(ServerError::UnknownConnection));
    }

    // Wrong welcome version: refused locally, nothing is sent.
    let pending = connect_async(
        client_config(ClientLimits::v1()),
        server.local_addr(),
        hello(1),
    );
    let (conn, _, _) = next_hello(&mut server);
    let wrong = Welcome {
        wire_version: 2,
        epoch: epoch(2),
    };
    assert_eq!(server.accept(conn, wrong), Err(ServerError::InvalidWelcome));
    let unknown = Welcome {
        wire_version: 3,
        epoch: epoch(3),
    };
    assert_eq!(
        server.accept(conn, unknown),
        Err(ServerError::InvalidWelcome)
    );
    let right = Welcome {
        wire_version: 1,
        epoch: epoch(1),
    };
    server.accept(conn, right).expect("accept");
    let client = pending.join().expect("thread").expect("connect");
    assert_eq!(client.welcome(), right, "the first welcome on the wire");
    assert_eq!(server.accept(conn, right), Err(ServerError::NotPending));
    assert_eq!(
        server.refuse(conn, CloseCode::AuthRefused),
        Err(ServerError::NotPending)
    );

    // Decision timeout.
    let mut limits = ServerLimits::v1();
    limits.decision_timeout_ms = 200;
    let mut slow = bind_server(limits);
    let pending = connect_async(
        client_config(ClientLimits::v1()),
        slow.local_addr(),
        hello(1),
    );
    let (conn, _, _) = next_hello(&mut slow);
    let timed_out = pending.join().expect("thread");
    assert_eq!(timed_out.err(), Some(ConnectError::Refused { code: 8 }));
    let late = CloseReason::Local(CloseCode::AuthTimeout);
    assert_eq!(slow.accept(conn, right), Err(ServerError::Closed(late)));
    assert_eq!(
        slow.refuse(conn, CloseCode::AuthRefused),
        Err(ServerError::Closed(late))
    );
    assert_eq!(next_closed(&mut slow), (conn, late));
}

#[test]
fn pending_connection_never_surfaces_lane_frames() {
    let mut server = bind_server(ServerLimits::v1());
    let early = pattern(7, 1, 300);

    // Refused: the early frame never surfaces.
    let raw = raw_lane(server.local_addr());
    let mut bytes = hello_frame(1, 16);
    bytes.extend_from_slice(&frame_bytes(&early));
    let (_send, _recv) = raw.open_and_write(&bytes);
    let (conn, _, _) = next_hello(&mut server);
    assert_eq!(server.try_recv(conn), Err(ServerError::NotAccepted));
    assert_eq!(server.try_send(conn, b"x"), Err(SendError::NotAccepted));
    assert_eq!(server.totals().pending, 1);
    server.refuse(conn, CloseCode::AuthRefused).expect("refuse");
    assert_eq!(raw.closed_code(), Some(5));
    assert_eq!(server.try_recv(conn), Err(ServerError::NotAccepted));
    assert_eq!(server.totals().inbound_frames, 0);
    assert_eq!(
        next_closed(&mut server),
        (conn, CloseReason::Local(CloseCode::AuthRefused))
    );

    // Control: the same choreography with accept delivers it after the welcome.
    let raw = raw_lane(server.local_addr());
    let (_send, mut recv) = raw.open_and_write(&bytes);
    let (conn, _, _) = next_hello(&mut server);
    assert_eq!(server.try_recv(conn), Err(ServerError::NotAccepted));
    let welcome = Welcome {
        wire_version: 1,
        epoch: epoch(9),
    };
    server.accept(conn, welcome).expect("accept");
    assert_eq!(raw.read_welcome(&mut recv), welcome);
    assert_eq!(server_recv(&mut server, conn), Ok(early));
}

// ---------------------------------------------------------------------------
// 12–16: caps, ordering, impairment, backpressure.
// ---------------------------------------------------------------------------

#[test]
fn lane_frame_caps_enforced_both_directions() {
    let mut server = bind_server(ServerLimits::v1());
    let (mut client, conn) = connect_accepted(&mut server, ClientLimits::v1(), 1, epoch(1));

    let before = queue_fields(client.stats());
    assert_eq!(
        client.try_send(&vec![1; INTENT_CAP + 1]),
        Err(SendError::FrameTooLarge)
    );
    assert_eq!(queue_fields(client.stats()), before);
    assert_eq!(client.try_send(&[]), Err(SendError::EmptyFrame));
    assert_eq!(queue_fields(client.stats()), before);
    let max_intent = pattern(1, 1, INTENT_CAP);
    client.try_send(&max_intent).expect("4,096 bytes");
    assert_eq!(server_recv(&mut server, conn), Ok(max_intent));

    let before = queue_fields(server.stats(conn).expect("stats"));
    assert_eq!(
        server.try_send(conn, &vec![2; DELTA_CAP + 1]),
        Err(SendError::FrameTooLarge)
    );
    assert_eq!(server.try_send(conn, &[]), Err(SendError::EmptyFrame));
    assert_eq!(queue_fields(server.stats(conn).expect("stats")), before);
    let max_delta = pattern(2, 1, DELTA_CAP);
    server.try_send(conn, &max_delta).expect("65,536 bytes");
    assert_eq!(client_recv(&mut client), Ok(max_delta));

    // A raw client: two valid frames, then a header over the cap.
    let raw = raw_lane(server.local_addr());
    let (mut send, mut recv) = raw.open_and_write(&hello_frame(1, 16));
    let (raw_conn, _, _) = next_hello(&mut server);
    let welcome = Welcome {
        wire_version: 1,
        epoch: epoch(4),
    };
    server.accept(raw_conn, welcome).expect("accept");
    raw.read_welcome(&mut recv);
    let first = pattern(3, 1, 10);
    let second = pattern(3, 2, 20);
    let mut bytes = frame_bytes(&first);
    bytes.extend_from_slice(&frame_bytes(&second));
    bytes.extend_from_slice(&[0, 0, 0x10, 0x01]);
    raw.write(&mut send, &bytes);
    assert_eq!(server_recv(&mut server, raw_conn), Ok(first));
    assert_eq!(server_recv(&mut server, raw_conn), Ok(second));
    assert_eq!(
        server_recv(&mut server, raw_conn),
        Err(ServerError::Closed(CloseReason::Local(
            CloseCode::FrameTooLarge
        )))
    );
    assert_eq!(raw.closed_code(), Some(3));
    assert_eq!(
        next_closed(&mut server),
        (raw_conn, CloseReason::Local(CloseCode::FrameTooLarge))
    );

    // A raw zero-length header after accept.
    let raw = raw_lane(server.local_addr());
    let (mut send, mut recv) = raw.open_and_write(&hello_frame(1, 16));
    let (zero_conn, _, _) = next_hello(&mut server);
    server.accept(zero_conn, welcome).expect("accept");
    raw.read_welcome(&mut recv);
    raw.write(&mut send, &[0, 0, 0, 0]);
    assert_eq!(raw.closed_code(), Some(2));
    assert_eq!(
        server_recv(&mut server, zero_conn),
        Err(ServerError::Closed(CloseReason::Local(
            CloseCode::ProtocolViolation
        )))
    );

    // The well-behaved client is unaffected.
    client.try_send(b"still open").expect("send");
    assert_eq!(server_recv(&mut server, conn), Ok(b"still open".to_vec()));
}

const ORDER_FRAMES: u32 = 1_000;

#[test]
fn ordering_preserved_under_concurrent_traffic() {
    let mut server = bind_server(ServerLimits::v1());
    let addr = server.local_addr();
    let mut threads = Vec::new();
    for client_id in 0..4u8 {
        let pending = connect_async(
            client_config(ClientLimits::v1()),
            addr,
            Hello {
                wire_version: 1,
                credential: credential(16, client_id),
            },
        );
        let (conn, _, hello) = next_hello(&mut server);
        assert!(hello.credential.ct_eq(&[client_id; 16]));
        server
            .accept(
                conn,
                Welcome {
                    wire_version: 1,
                    epoch: epoch(client_id),
                },
            )
            .expect("accept");
        let client = pending.join().expect("thread").expect("connect");
        threads.push((
            conn,
            client_id,
            std::thread::spawn(move || client_traffic(client, client_id)),
        ));
    }

    // The server sends 1..=1,000 to each client and checks what it receives.
    let mut next_send: BTreeMap<ConnectionId, u32> = BTreeMap::new();
    let mut next_recv: BTreeMap<ConnectionId, u32> = BTreeMap::new();
    let ids: BTreeMap<ConnectionId, u8> =
        threads.iter().map(|(conn, id, _)| (*conn, *id)).collect();
    for conn in ids.keys() {
        next_send.insert(*conn, 1);
        next_recv.insert(*conn, 1);
    }
    let start = std::time::Instant::now();
    loop {
        let mut progress = false;
        for (&conn, &client_id) in &ids {
            let counter = next_send.get_mut(&conn).expect("conn");
            while *counter <= ORDER_FRAMES {
                let frame = pattern(100 + client_id, *counter, cycling_size(*counter, DELTA_CAP));
                match server.try_send(conn, &frame) {
                    Ok(()) => {
                        *counter += 1;
                        progress = true;
                    }
                    Err(SendError::QueueFull) => break,
                    Err(error) => panic!("server send: {error:?}"),
                }
            }
            let expected = next_recv.get_mut(&conn).expect("conn");
            if *expected > ORDER_FRAMES {
                // All 1,000 arrived; the client may already have closed.
                continue;
            }
            while *expected <= ORDER_FRAMES {
                let Some(frame) = server.try_recv(conn).expect("server recv") else {
                    break;
                };
                assert_eq!(
                    frame,
                    pattern(client_id, *expected, cycling_size(*expected, INTENT_CAP)),
                    "client {client_id} frame {expected}"
                );
                *expected += 1;
                progress = true;
            }
        }
        let done = next_send.values().all(|n| *n > ORDER_FRAMES)
            && next_recv.values().all(|n| *n > ORDER_FRAMES);
        if done {
            break;
        }
        assert!(start.elapsed() < DEADLINE, "deadline: server traffic");
        if !progress {
            server.wait(Duration::from_millis(1));
        }
    }
    for (_, _, thread) in threads {
        thread.join().expect("client traffic");
    }
    // Each client closed (Normal, Flush) after its 1,000: closed and drained
    // with nothing extra queued.
    for conn in ids.keys() {
        assert_eq!(
            server_recv(&mut server, *conn),
            Err(ServerError::Closed(CloseReason::Peer { code: 0 })),
            "no extras"
        );
    }
}

/// One client's half of case 13: sends 1..=1,000 and checks 1..=1,000 back.
fn client_traffic(mut client: QuicClient, client_id: u8) {
    let mut next_send = 1u32;
    let mut next_recv = 1u32;
    let start = std::time::Instant::now();
    while next_send <= ORDER_FRAMES || next_recv <= ORDER_FRAMES {
        let mut progress = false;
        while next_send <= ORDER_FRAMES {
            let frame = pattern(client_id, next_send, cycling_size(next_send, INTENT_CAP));
            match client.try_send(&frame) {
                Ok(()) => {
                    next_send += 1;
                    progress = true;
                }
                Err(SendError::QueueFull) => break,
                Err(error) => panic!("client send: {error:?}"),
            }
        }
        while let Some(frame) = client.try_recv().expect("client recv") {
            assert!(next_recv <= ORDER_FRAMES, "extra frame to {client_id}");
            assert_eq!(
                frame,
                pattern(
                    100 + client_id,
                    next_recv,
                    cycling_size(next_recv, DELTA_CAP)
                ),
                "client {client_id} receives frame {next_recv}"
            );
            next_recv += 1;
            progress = true;
        }
        assert!(start.elapsed() < DEADLINE, "deadline: client traffic");
        if !progress {
            client.wait(Duration::from_millis(1));
        }
    }
    // Wait for the server to drain our last frames before closing.
    let report = client.close(CloseCode::Normal, CloseMode::Flush);
    assert_eq!(report.outbound_frames_discarded, 0);
}

/// Pumps `count` frames each way between one client and the server,
/// asserting exactly-once in-order delivery both directions.
fn exchange_exactly_once(
    server: &mut QuicServer,
    conn: ConnectionId,
    client: &mut QuicClient,
    count: u32,
    client_cap: usize,
    server_cap: usize,
) {
    let (mut sent_c, mut sent_s, mut got_c, mut got_s) = (1u32, 1u32, 1u32, 1u32);
    let start = std::time::Instant::now();
    while got_c <= count || got_s <= count {
        let mut progress = false;
        while sent_c <= count {
            match client.try_send(&pattern(1, sent_c, cycling_size(sent_c, client_cap))) {
                Ok(()) => {
                    sent_c += 1;
                    progress = true;
                }
                Err(SendError::QueueFull) => break,
                Err(error) => panic!("client send: {error:?}"),
            }
        }
        while sent_s <= count {
            match server.try_send(conn, &pattern(2, sent_s, cycling_size(sent_s, server_cap))) {
                Ok(()) => {
                    sent_s += 1;
                    progress = true;
                }
                Err(SendError::QueueFull) => break,
                Err(error) => panic!("server send: {error:?}"),
            }
        }
        while let Some(frame) = server.try_recv(conn).expect("server recv") {
            assert!(got_s <= count, "extra frame at the server");
            assert_eq!(frame, pattern(1, got_s, cycling_size(got_s, client_cap)));
            got_s += 1;
            progress = true;
        }
        while let Some(frame) = client.try_recv().expect("client recv") {
            assert!(got_c <= count, "extra frame at the client");
            assert_eq!(frame, pattern(2, got_c, cycling_size(got_c, server_cap)));
            got_c += 1;
            progress = true;
        }
        assert!(start.elapsed() < DEADLINE, "deadline: exchange");
        if !progress {
            client.wait(Duration::from_millis(1));
        }
    }
    assert_eq!(server.try_recv(conn), Ok(None));
    assert_eq!(client.try_recv(), Ok(None));
}

fn impaired() -> RelayConfig {
    RelayConfig {
        drop_every: 13,
        reorder_depth: 4,
        seed: 23,
    }
}

fn connect_via_relay(
    server: &mut QuicServer,
    relay: &UdpRelay,
    limits: ClientLimits,
    welcome: Welcome,
) -> (QuicClient, ConnectionId) {
    let pending = connect_async(
        client_config(limits),
        relay.addr(),
        hello(welcome.wire_version),
    );
    let (conn, _, _) = next_hello(server);
    server.accept(conn, welcome).expect("accept");
    let client = pending.join().expect("thread").expect("connect via relay");
    (client, conn)
}

#[test]
fn ordered_exactly_once_through_impaired_relay() {
    let welcome = Welcome {
        wire_version: 1,
        epoch: epoch(14),
    };
    {
        let mut server = bind_server(ServerLimits::v1());
        let relay = UdpRelay::start(server.local_addr(), impaired());
        let (mut client, conn) =
            connect_via_relay(&mut server, &relay, ClientLimits::v1(), welcome);
        exchange_exactly_once(
            &mut server,
            conn,
            &mut client,
            2_000,
            INTENT_CAP,
            INTENT_CAP,
        );
        assert!(relay.dropped() >= 1, "impairment control: dropped");
        assert!(relay.reordered() >= 1, "impairment control: reordered");
    }
    // Positive control: the same transfer through a clean relay.
    let mut server = bind_server(ServerLimits::v1());
    let relay = UdpRelay::start(server.local_addr(), RelayConfig::clean());
    let (mut client, conn) = connect_via_relay(&mut server, &relay, ClientLimits::v1(), welcome);
    exchange_exactly_once(
        &mut server,
        conn,
        &mut client,
        2_000,
        INTENT_CAP,
        INTENT_CAP,
    );
    assert_eq!(relay.dropped(), 0);
    assert!(relay.forwarded() > 0);
}

#[test]
fn inbound_backpressure_at_exact_bounds() {
    let sent = |stream: u8, n: u32| pattern(stream, n, INTENT_CAP);

    // Frames: 4 per connection, 6 sent.
    let mut limits = ServerLimits::v1();
    limits.inbound.per_peer_frames = 4;
    let mut server = bind_server(limits);
    let (mut client, conn) = connect_accepted(&mut server, ClientLimits::v1(), 1, epoch(1));
    for n in 1..=6 {
        client.try_send(&sent(1, n)).expect("send");
    }
    until("4 queued + parked", || {
        let stats = server.stats(conn).expect("stats");
        stats.inbound_frames == 4 && stats.reader_parked
    });
    assert_eq!(
        server.stats(conn).expect("stats").inbound_bytes,
        4 * INTENT_CAP
    );
    assert_eq!(server.try_recv(conn), Ok(Some(sent(1, 1))));
    until("4 queued + parked again", || {
        let stats = server.stats(conn).expect("stats");
        stats.inbound_frames == 4 && stats.reader_parked
    });
    for n in 2..=6 {
        assert_eq!(server_recv(&mut server, conn), Ok(sent(1, n)));
    }
    assert_eq!(server.try_recv(conn), Ok(None));
    let stats = server.stats(conn).expect("stats");
    assert_eq!((stats.inbound_frames, stats.reader_parked), (0, false));

    // Bytes: 10,000 per connection holds exactly two 4,096-byte frames.
    let mut limits = ServerLimits::v1();
    limits.inbound.per_peer_bytes = 10_000;
    let mut server = bind_server(limits);
    let (mut client, conn) = connect_accepted(&mut server, ClientLimits::v1(), 1, epoch(1));
    for n in 1..=4 {
        client.try_send(&sent(2, n)).expect("send");
    }
    until("2 queued + parked", || {
        let stats = server.stats(conn).expect("stats");
        stats.inbound_frames == 2 && stats.reader_parked
    });
    assert_eq!(
        server.stats(conn).expect("stats").inbound_bytes,
        2 * INTENT_CAP
    );
    for n in 1..=4 {
        assert_eq!(server_recv(&mut server, conn), Ok(sent(2, n)));
    }
    assert_eq!(server.try_recv(conn), Ok(None));

    // Host-wide frames = 3: A holds 2, B holds 1 and parks.
    let mut limits = ServerLimits::v1();
    limits.inbound.per_peer_frames = 2;
    limits.inbound.host_frames = 3;
    let mut server = bind_server(limits);
    let (mut client_a, conn_a) = connect_accepted(&mut server, ClientLimits::v1(), 1, epoch(1));
    let (mut client_b, conn_b) = connect_accepted(&mut server, ClientLimits::v1(), 1, epoch(2));
    for n in 1..=3 {
        client_a.try_send(&sent(3, n)).expect("send a");
    }
    until("A holds 2 + parked", || {
        let stats = server.stats(conn_a).expect("stats");
        stats.inbound_frames == 2 && stats.reader_parked
    });
    for n in 1..=2 {
        client_b.try_send(&sent(4, n)).expect("send b");
    }
    until("B holds 1 + parked", || {
        let stats = server.stats(conn_b).expect("stats");
        stats.inbound_frames == 1 && stats.reader_parked
    });
    assert_eq!(server.totals().inbound_frames, 3);
    for n in 1..=3 {
        assert_eq!(server_recv(&mut server, conn_a), Ok(sent(3, n)));
    }
    for n in 1..=2 {
        assert_eq!(server_recv(&mut server, conn_b), Ok(sent(4, n)));
    }
    assert_eq!(server.try_recv(conn_a), Ok(None));
    assert_eq!(server.try_recv(conn_b), Ok(None));
    assert_eq!(server.totals().inbound_frames, 0);
}

/// Sends unique frames until two consecutive `QueueFull`s leave the stats
/// unchanged at the cap; returns the ids of the frames that were accepted.
fn fill_until_stable_queue_full(
    mut send: impl FnMut(&[u8]) -> Result<(), SendError>,
    mut stats: impl FnMut() -> (usize, usize),
    cap: (usize, usize),
    size: usize,
    next_id: &mut u32,
) -> Vec<u32> {
    let mut accepted = Vec::new();
    let start = std::time::Instant::now();
    loop {
        assert!(start.elapsed() < DEADLINE, "deadline: QueueFull");
        let id = *next_id;
        *next_id += 1;
        match send(&pattern(9, id, size)) {
            Ok(()) => {
                accepted.push(id);
                continue;
            }
            Err(SendError::QueueFull) => {}
            Err(error) => panic!("send: {error:?}"),
        }
        let before = stats();
        let id = *next_id;
        *next_id += 1;
        match send(&pattern(9, id, size)) {
            Ok(()) => {
                accepted.push(id);
                continue;
            }
            Err(SendError::QueueFull) => {}
            Err(error) => panic!("send: {error:?}"),
        }
        let after = stats();
        // Two consecutive QueueFulls at exactly the cap, and the second
        // one changed nothing.
        if before == after && after == cap {
            return accepted;
        }
    }
}

#[test]
fn outbound_queue_full_is_retryable_and_lossless() {
    // A non-reading client: a one-frame inbound queue stops its reader, so
    // QUIC flow control eventually blocks the server's writer.
    let mut slow_reader = ClientLimits::v1();
    slow_reader.inbound_frames = 1;
    slow_reader.inbound_bytes = DELTA_CAP;

    for (frames_cap, bytes_cap) in [(4usize, 2_097_152usize), (128, 3 * DELTA_CAP)] {
        let mut limits = ServerLimits::v1();
        limits.outbound.per_peer_frames = frames_cap;
        limits.outbound.per_peer_bytes = bytes_cap;
        let mut server = bind_server(limits);
        let (mut client, conn) = connect_accepted(&mut server, slow_reader, 1, epoch(1));
        let mut next_id = 1u32;
        let cap = (frames_cap.min(bytes_cap / DELTA_CAP), bytes_cap);
        let server_cell = std::cell::RefCell::new(&mut server);
        let accepted = fill_until_stable_queue_full(
            |frame| server_cell.borrow_mut().try_send(conn, frame),
            || {
                let stats = server_cell.borrow().stats(conn).expect("stats");
                (stats.outbound_frames, stats.outbound_bytes)
            },
            (cap.0, cap.0 * DELTA_CAP),
            DELTA_CAP,
            &mut next_id,
        );
        // Drain everything; then the next send succeeds.
        let mut received = Vec::new();
        for _ in 0..accepted.len() {
            received.push(client_recv(&mut client).expect("frame"));
        }
        let last = next_id;
        server
            .try_send(conn, &pattern(9, last, DELTA_CAP))
            .expect("room after drain");
        received.push(client_recv(&mut client).expect("frame"));
        let expected: Vec<Vec<u8>> = accepted
            .iter()
            .chain(std::iter::once(&last))
            .map(|id| pattern(9, *id, DELTA_CAP))
            .collect();
        assert_eq!(received, expected, "exactly the Ok sends, in order");
        assert_eq!(client.try_recv(), Ok(None));
    }

    // Client-side QueueFull with the server not draining.
    let mut limits = ServerLimits::v1();
    limits.inbound.per_peer_frames = 1;
    limits.inbound.per_peer_bytes = INTENT_CAP;
    let mut server = bind_server(limits);
    let mut client_limits = ClientLimits::v1();
    client_limits.outbound_frames = 4;
    let (client, conn) = connect_accepted(&mut server, client_limits, 1, epoch(1));
    let client_cell = std::cell::RefCell::new(client);
    let mut next_id = 1u32;
    let accepted = fill_until_stable_queue_full(
        |frame| client_cell.borrow_mut().try_send(frame),
        || {
            let stats = client_cell.borrow().stats();
            (stats.outbound_frames, stats.outbound_bytes)
        },
        (4, 4 * INTENT_CAP),
        INTENT_CAP,
        &mut next_id,
    );
    let mut client = client_cell.into_inner();
    let mut received = Vec::new();
    for _ in 0..accepted.len() {
        received.push(server_recv(&mut server, conn).expect("frame"));
    }
    let last = next_id;
    client
        .try_send(&pattern(9, last, INTENT_CAP))
        .expect("room after drain");
    received.push(server_recv(&mut server, conn).expect("frame"));
    let expected: Vec<Vec<u8>> = accepted
        .iter()
        .chain(std::iter::once(&last))
        .map(|id| pattern(9, *id, INTENT_CAP))
        .collect();
    assert_eq!(received, expected);
    assert_eq!(server.try_recv(conn), Ok(None));
}

// ---------------------------------------------------------------------------
// 17–21: close, shutdown, abrupt peers, idle, fail-closed.
// ---------------------------------------------------------------------------

#[test]
fn graceful_close_flushes_and_reports_codes() {
    let mut server = bind_server(ServerLimits::v1());

    // Server Flush with 3 queued frames.
    let (mut client, conn) = connect_accepted(&mut server, ClientLimits::v1(), 1, epoch(1));
    for n in 1..=3 {
        server.try_send(conn, &pattern(5, n, 1_000)).expect("send");
    }
    let report = server
        .close(conn, CloseCode::Normal, CloseMode::Flush)
        .expect("close");
    assert_eq!(report.outbound_frames_discarded, 0);
    assert_eq!(report.inbound_frames_discarded, 0);
    for n in 1..=3 {
        assert_eq!(client_recv(&mut client), Ok(pattern(5, n, 1_000)));
    }
    assert_eq!(client_recv(&mut client), Err(CloseReason::Peer { code: 0 }));
    assert_eq!(
        next_closed(&mut server),
        (conn, CloseReason::Local(CloseCode::Normal))
    );
    assert_eq!(
        server.close(conn, CloseCode::Normal, CloseMode::Flush),
        Err(ServerError::UnknownConnection),
        "forgotten after Closed was polled with no frames held"
    );

    // Server Discard with N queued: an in-order prefix, then code 9.
    let (mut client, conn) = connect_accepted(&mut server, ClientLimits::v1(), 1, epoch(2));
    let n_frames = 64u32;
    for n in 1..=n_frames {
        server
            .try_send(conn, &pattern(6, n, DELTA_CAP))
            .expect("send");
    }
    let report = server
        .close(conn, CloseCode::SessionFenced, CloseMode::Discard)
        .expect("close");
    assert_eq!(
        server.try_send(conn, b"late"),
        Err(SendError::Closed(CloseReason::Local(
            CloseCode::SessionFenced
        )))
    );
    let mut k = 0u32;
    loop {
        match client_recv(&mut client) {
            Ok(frame) => {
                k += 1;
                assert_eq!(frame, pattern(6, k, DELTA_CAP), "in-order prefix");
            }
            Err(reason) => {
                assert_eq!(reason, CloseReason::Peer { code: 9 });
                break;
            }
        }
    }
    assert!(
        k as usize + report.outbound_frames_discarded <= n_frames as usize,
        "k = {k}, discarded = {}",
        report.outbound_frames_discarded
    );
    assert_eq!(
        next_closed(&mut server),
        (conn, CloseReason::Local(CloseCode::SessionFenced))
    );

    // Client Flush after 3 sends.
    let (mut client, conn) = connect_accepted(&mut server, ClientLimits::v1(), 1, epoch(3));
    for n in 1..=3 {
        client.try_send(&pattern(7, n, 500)).expect("send");
    }
    let report = client.close(CloseCode::Normal, CloseMode::Flush);
    assert_eq!(report.outbound_frames_discarded, 0);
    assert_eq!(report.inbound_frames_discarded, 0);
    for n in 1..=3 {
        assert_eq!(server_recv(&mut server, conn), Ok(pattern(7, n, 500)));
    }
    assert_eq!(
        server_recv(&mut server, conn),
        Err(ServerError::Closed(CloseReason::Peer { code: 0 }))
    );
    assert_eq!(
        next_closed(&mut server),
        (conn, CloseReason::Peer { code: 0 })
    );
    assert_eq!(server.try_recv(conn), Err(ServerError::UnknownConnection));
}

#[test]
fn server_shutdown_and_drop_close_every_connection() {
    let mut server = bind_server(ServerLimits::v1());
    let mut clients = Vec::new();
    for i in 0..3u8 {
        let (client, conn) = connect_accepted(&mut server, ClientLimits::v1(), 1, epoch(i));
        for n in 1..=5 {
            server.try_send(conn, &pattern(i, n, 2_000)).expect("queue");
        }
        clients.push((i, client));
    }
    let report = server.shutdown();
    assert_eq!(report.connections_closed, 3);
    assert_eq!(report.outbound_frames_discarded, 0);
    assert_eq!(report.inbound_frames_discarded, 0);
    for (i, mut client) in clients {
        for n in 1..=5 {
            assert_eq!(client_recv(&mut client), Ok(pattern(i, n, 2_000)));
        }
        assert_eq!(client_recv(&mut client), Err(CloseReason::Peer { code: 1 }));
    }

    // Dropped without shutdown: the client sees Shutdown, not an idle timeout.
    let mut server = bind_server(ServerLimits::v1());
    let (mut client, _conn) = connect_accepted(&mut server, ClientLimits::v1(), 1, epoch(9));
    drop(server);
    assert_eq!(client_recv(&mut client), Err(CloseReason::Peer { code: 1 }));
}

#[test]
fn peer_abrupt_close_keeps_complete_frames_only() {
    let mut server = bind_server(ServerLimits::v1());
    let raw = raw_lane(server.local_addr());
    let (mut send, mut recv) = raw.open_and_write(&hello_frame(1, 16));
    let (conn, _, _) = next_hello(&mut server);
    server
        .accept(
            conn,
            Welcome {
                wire_version: 1,
                epoch: epoch(1),
            },
        )
        .expect("accept");
    raw.read_welcome(&mut recv);
    let one = pattern(8, 1, 64);
    let two = pattern(8, 2, 64);
    let mut bytes = frame_bytes(&one);
    bytes.extend_from_slice(&frame_bytes(&two));
    let third = frame_bytes(&pattern(8, 3, 100));
    bytes.extend_from_slice(&third[..54]);
    raw.write(&mut send, &bytes);
    until("two complete frames queued", || {
        server.stats(conn).expect("stats").inbound_frames == 2
    });
    raw.close(0x99);
    assert_eq!(
        next_closed(&mut server),
        (conn, CloseReason::Peer { code: 0x99 })
    );
    assert_eq!(server.try_recv(conn), Ok(Some(one)));
    assert_eq!(server.try_recv(conn), Ok(Some(two)));
    assert_eq!(
        server.try_recv(conn),
        Err(ServerError::Closed(CloseReason::Peer { code: 0x99 }))
    );
    assert_eq!(server.try_recv(conn), Err(ServerError::UnknownConnection));
}

#[test]
fn idle_timeout_closes_silent_peer() {
    let mut server_limits = ServerLimits::v1();
    server_limits.idle_timeout_ms = 1_000;
    server_limits.keep_alive_ms = 300;
    let mut client_limits = ClientLimits::v1();
    client_limits.idle_timeout_ms = 1_000;
    client_limits.keep_alive_ms = 300;
    let mut server = bind_server(server_limits);
    let relay = UdpRelay::start(server.local_addr(), RelayConfig::clean());
    let welcome = Welcome {
        wire_version: 1,
        epoch: epoch(20),
    };
    let (mut client, conn) = connect_via_relay(&mut server, &relay, client_limits, welcome);

    // Positive control: keep-alives carry the connection past the idle period.
    until("five keep-alive pings", || {
        client.stats().keepalive_pings_sent >= 5
    });
    assert_eq!(client.close_reason(), None);
    assert_eq!(client.try_recv(), Ok(None));
    assert!(
        server.poll_event().is_none(),
        "no Closed while keep-alives flow"
    );
    assert_eq!(server.try_recv(conn), Ok(None));

    // Blackhole the path: both sides time out.
    relay.set_blackhole(true);
    assert_eq!(next_closed(&mut server), (conn, CloseReason::IdleTimeout));
    assert_eq!(client_recv(&mut client), Err(CloseReason::IdleTimeout));
}

#[test]
fn datagrams_and_connection_limit_fail_closed() {
    let server = bind_server(ServerLimits::v1());
    let raw = raw_lane(server.local_addr());
    assert_eq!(raw.connection.max_datagram_size(), None);
    assert!(matches!(
        raw.connection.send_datagram(vec![1u8, 2, 3].into()),
        Err(quinn::SendDatagramError::UnsupportedByPeer)
    ));
    raw.close(0);
    drop(server);

    let mut limits = ServerLimits::v1();
    limits.max_connections = 2;
    let mut server = bind_server(limits);
    let (first, first_conn) = connect_accepted(&mut server, ClientLimits::v1(), 1, epoch(1));
    let (_second, _) = connect_accepted(&mut server, ClientLimits::v1(), 1, epoch(2));
    assert_eq!(server.totals().connections, 2);
    let third = QuicClient::connect(
        client_config(ClientLimits::v1()),
        server.local_addr(),
        &hello(1),
    );
    assert_eq!(third.err(), Some(ConnectError::EndpointFull));
    assert_eq!(server.totals().refused_at_limit, 1);

    // After one Closed is polled, a new client connects.
    first.close(CloseCode::Normal, CloseMode::Discard);
    assert_eq!(
        next_closed(&mut server),
        (first_conn, CloseReason::Peer { code: 0 })
    );
    let (_fourth, conn) = connect_accepted(&mut server, ClientLimits::v1(), 1, epoch(4));
    assert_eq!(conn.get(), 3);
    assert_eq!(server.totals().refused_at_limit, 1);
}

// ---------------------------------------------------------------------------
// 22–23: Transport and real codec frames.
// ---------------------------------------------------------------------------

fn send_via<T: Transport<Error = SendError>>(endpoint: &mut T, bytes: &[u8]) {
    endpoint.send(bytes).expect("transport send");
}

fn recv_via<T: Transport>(endpoint: &mut T) -> Vec<u8> {
    let start = std::time::Instant::now();
    loop {
        if let Some(frame) = endpoint.recv() {
            return frame;
        }
        assert!(start.elapsed() < DEADLINE, "deadline: transport recv");
        std::thread::park_timeout(Duration::from_millis(1));
    }
}

#[test]
fn transport_trait_round_trip() {
    let mut server = bind_server(ServerLimits::v1());
    let (mut client, conn) = connect_accepted(&mut server, ClientLimits::v1(), 1, epoch(22));
    for n in 1..=3u32 {
        let up = pattern(1, n, 100 * n as usize);
        send_via(&mut client, &up);
        let mut lane = server.lane(conn).expect("lane");
        assert_eq!(recv_via(&mut lane), up);
        assert_eq!(lane.recv(), None, "None when empty");
        let down = pattern(2, n, 1_000 * n as usize);
        send_via(&mut lane, &down);
        assert_eq!(recv_via(&mut client), down);
        assert_eq!(Transport::recv(&mut client), None, "None when empty");
    }
    let mut lane = server.lane(conn).expect("lane");
    assert_eq!(lane.send(&[]), Err(SendError::EmptyFrame));
    assert_eq!(
        Transport::send(&mut client, &vec![0; INTENT_CAP + 1]),
        Err(SendError::FrameTooLarge)
    );

    // A pending connection has no lane.
    let raw = raw_lane(server.local_addr());
    raw.open_and_write(&hello_frame(1, 16));
    let (pending, _, _) = next_hello(&mut server);
    assert_eq!(server.lane(pending).err(), Some(ServerError::NotAccepted));
}

/// Case 23: 1,000 v1 `EndTurn` intents up, 1,000 receipt deltas down, all
/// decoded after delivery. Returns the relay for its counters.
fn codec_frames_through(relay_config: RelayConfig) -> UdpRelay {
    let session_epoch = epoch(0x23);
    let welcome = Welcome {
        wire_version: 1,
        epoch: session_epoch,
    };
    let mut server = bind_server(ServerLimits::v1());
    let relay = UdpRelay::start(server.local_addr(), relay_config);
    let (mut client, conn) = connect_via_relay(&mut server, &relay, ClientLimits::v1(), welcome);
    let client_epoch = client.welcome().epoch;
    assert_eq!(client_epoch, session_epoch, "epoch handoff");

    let count = 1_000u64;
    let (mut next_intent, mut next_seen, mut next_receipt) = (1u64, 1u64, 1u64);
    let mut pending_replies: std::collections::VecDeque<Vec<u8>> = Default::default();
    let mut tampered_once = false;
    let start = std::time::Instant::now();
    while next_receipt <= count {
        let mut progress = false;
        while next_intent <= count {
            let bytes = codec::encode_intent(&end_turn(client_epoch, next_intent)).expect("intent");
            match client.try_send(&bytes) {
                Ok(()) => {
                    next_intent += 1;
                    progress = true;
                }
                Err(SendError::QueueFull) => break,
                Err(error) => panic!("client send: {error:?}"),
            }
        }
        while let Some(frame) = server.try_recv(conn).expect("server recv") {
            let intent = codec::decode_intent(&frame).expect("decode intent");
            assert_eq!(intent.seq, next_seen, "intents in order");
            assert_eq!(intent.epoch, session_epoch);
            assert_eq!(intent.body, IntentBody::EndTurn);
            if !tampered_once {
                // Negative control: a test-side copy with one byte flipped
                // fails the same decode assertion.
                let mut copy = frame.clone();
                copy[0] ^= 0xFF;
                assert!(codec::decode_intent(&copy).is_err());
                tampered_once = true;
            }
            pending_replies.push_back(
                codec::encode_delta(&receipt_delta(session_epoch, intent.seq)).expect("delta"),
            );
            next_seen += 1;
            progress = true;
        }
        while let Some(reply) = pending_replies.front() {
            match server.try_send(conn, reply) {
                Ok(()) => {
                    pending_replies.pop_front();
                    progress = true;
                }
                Err(SendError::QueueFull) => break,
                Err(error) => panic!("server send: {error:?}"),
            }
        }
        while let Some(frame) = client.try_recv().expect("client recv") {
            let delta = codec::decode_delta(&frame).expect("decode delta");
            assert_eq!(delta, receipt_delta(session_epoch, next_receipt));
            match &delta.ops[..] {
                [DeltaOp::Receipt { seq, status, .. }] => {
                    assert_eq!(*seq, next_receipt, "receipt echoes its seq");
                    assert_eq!(*status, ReceiptStatus::Applied);
                }
                other => panic!("unexpected ops {other:?}"),
            }
            next_receipt += 1;
            progress = true;
        }
        assert!(start.elapsed() < DEADLINE, "deadline: codec exchange");
        if !progress {
            client.wait(Duration::from_millis(1));
        }
    }
    assert!(tampered_once);
    assert_eq!(next_seen, count + 1);
    assert_eq!(server.try_recv(conn), Ok(None));
    assert_eq!(client.try_recv(), Ok(None));
    relay
}

#[test]
fn lane0_codec_frames_decode_after_impaired_delivery() {
    let impaired_relay = codec_frames_through(impaired());
    assert!(impaired_relay.dropped() >= 1, "impairment control: dropped");
    assert!(
        impaired_relay.reordered() >= 1,
        "impairment control: reordered"
    );
    let clean_relay = codec_frames_through(RelayConfig::clean());
    assert_eq!(clean_relay.dropped(), 0, "positive control");
}

// ---------------------------------------------------------------------------
// 24–27: flow-control window limits (T023c).
// ---------------------------------------------------------------------------

/// Sends `pattern(10, 1..=n, size)`, retrying `QueueFull` after a 1 ms park,
/// until exactly `n` are accepted (30 s guard); then 50 more attempts, each
/// after a 2 ms park, must all be `QueueFull`. Parks are pacing; the counts
/// are the oracle.
fn fill_to_window_bound(mut send: impl FnMut(&[u8]) -> Result<(), SendError>, n: u32, size: usize) {
    let start = std::time::Instant::now();
    let mut accepted = 0u32;
    while accepted < n {
        assert!(
            start.elapsed() < DEADLINE,
            "deadline: {accepted} of {n} accepted"
        );
        match send(&pattern(10, accepted + 1, size)) {
            Ok(()) => accepted += 1,
            Err(SendError::QueueFull) => std::thread::park_timeout(Duration::from_millis(1)),
            Err(error) => panic!("send {}: {error:?}", accepted + 1),
        }
    }
    // Negative control: with no reads there is no window update, so a
    // correct endpoint can never accept one of these.
    for attempt in 1..=50 {
        std::thread::park_timeout(Duration::from_millis(2));
        assert_eq!(
            send(&pattern(10, n + 1, size)),
            Err(SendError::QueueFull),
            "trailing attempt {attempt} past the bound {n}"
        );
    }
}

#[test]
fn window_limits_validate_and_refuse_loosening() {
    let server_v1 = ServerLimits::v1();
    let client_v1 = ClientLimits::v1();
    assert_eq!(
        (
            server_v1.stream_window_bytes,
            server_v1.connection_window_bytes,
            server_v1.send_window_bytes
        ),
        (65_536, 131_072, 1_048_576)
    );
    assert_eq!(
        (
            client_v1.stream_window_bytes,
            client_v1.connection_window_bytes,
            client_v1.send_window_bytes
        ),
        (1_048_576, 2_097_152, 262_144)
    );
    assert_eq!(server_v1.validate(), Ok(()));
    assert_eq!(client_v1.validate(), Ok(()));

    let loosens = |field| Err(ConfigError::LoosensPolicy { field });
    let invalid = |field| Err(ConfigError::InvalidLimits { field });

    // Loosening: each window one byte past v1, the rest v1.
    let server_loosen: Vec<Case<ServerLimits>> = vec![
        (
            "stream_window_bytes",
            Box::new(|l| l.stream_window_bytes += 1),
        ),
        (
            "connection_window_bytes",
            Box::new(|l| l.connection_window_bytes += 1),
        ),
        ("send_window_bytes", Box::new(|l| l.send_window_bytes += 1)),
    ];
    for (field, change) in &server_loosen {
        let mut limits = ServerLimits::v1();
        change(&mut limits);
        assert_eq!(limits.validate(), loosens(*field), "server {field}");
    }
    let client_loosen: Vec<Case<ClientLimits>> = vec![
        (
            "stream_window_bytes",
            Box::new(|l| l.stream_window_bytes += 1),
        ),
        (
            "connection_window_bytes",
            Box::new(|l| l.connection_window_bytes += 1),
        ),
        ("send_window_bytes", Box::new(|l| l.send_window_bytes += 1)),
    ];
    for (field, change) in &client_loosen {
        let mut limits = ClientLimits::v1();
        change(&mut limits);
        assert_eq!(limits.validate(), loosens(*field), "client {field}");
    }

    // Below the floors, and a connection window below its stream window.
    let server_invalid: Vec<Case<ServerLimits>> = vec![
        (
            "stream_window_bytes",
            Box::new(|l| l.stream_window_bytes = 4_099),
        ),
        (
            "stream_window_bytes",
            Box::new(|l| l.stream_window_bytes = 0),
        ),
        (
            "send_window_bytes",
            Box::new(|l| l.send_window_bytes = 65_539),
        ),
        (
            "connection_window_bytes",
            Box::new(|l| {
                l.stream_window_bytes = 65_536;
                l.connection_window_bytes = 65_535;
            }),
        ),
    ];
    for (field, change) in &server_invalid {
        let mut limits = ServerLimits::v1();
        change(&mut limits);
        assert_eq!(limits.validate(), invalid(*field), "server {field}");
    }
    let client_invalid: Vec<Case<ClientLimits>> = vec![
        (
            "stream_window_bytes",
            Box::new(|l| l.stream_window_bytes = 65_539),
        ),
        (
            "stream_window_bytes",
            Box::new(|l| l.stream_window_bytes = 0),
        ),
        (
            "send_window_bytes",
            Box::new(|l| l.send_window_bytes = 4_099),
        ),
        (
            "connection_window_bytes",
            Box::new(|l| {
                l.stream_window_bytes = 1_048_576;
                l.connection_window_bytes = 1_048_575;
            }),
        ),
        (
            "connection_window_bytes",
            Box::new(|l| {
                l.stream_window_bytes = 100_000;
                l.connection_window_bytes = 99_999;
            }),
        ),
    ];
    for (field, change) in &client_invalid {
        let mut limits = ClientLimits::v1();
        change(&mut limits);
        assert_eq!(limits.validate(), invalid(*field), "client {field}");
    }

    // Exactly at the floors: positive controls.
    let mut server_floor = ServerLimits::v1();
    server_floor.stream_window_bytes = 4_100;
    server_floor.connection_window_bytes = 4_100;
    server_floor.send_window_bytes = 65_540;
    assert_eq!(server_floor.validate(), Ok(()));
    let mut client_floor = ClientLimits::v1();
    client_floor.stream_window_bytes = 65_540;
    client_floor.connection_window_bytes = 65_540;
    client_floor.send_window_bytes = 4_100;
    assert_eq!(client_floor.validate(), Ok(()));
    let mut client_stream_only = ClientLimits::v1();
    client_stream_only.stream_window_bytes = 65_540;
    assert_eq!(client_stream_only.validate(), Ok(()));
    let mut server_equal = ServerLimits::v1();
    server_equal.stream_window_bytes = 65_536;
    server_equal.connection_window_bytes = 65_536;
    assert_eq!(server_equal.validate(), Ok(()));

    // Declaration order: queue fields come first; the first failing field
    // wins.
    let mut server_order = ServerLimits::v1();
    server_order.outbound.per_peer_bytes = DELTA_CAP - 1;
    server_order.stream_window_bytes = 65_537;
    assert_eq!(server_order.validate(), invalid("outbound.per_peer_bytes"));
    let mut client_order = ClientLimits::v1();
    client_order.stream_window_bytes = 1_048_577;
    client_order.connection_window_bytes = 1;
    assert_eq!(client_order.validate(), loosens("stream_window_bytes"));

    // bind/connect surface the config error before any I/O.
    let mut bad_server = ServerLimits::v1();
    bad_server.send_window_bytes = 1_048_577;
    let bound = QuicServer::bind(ServerConfig {
        bind: loopback(),
        identity: identity_a(),
        limits: bad_server,
    });
    assert_eq!(
        bound.err(),
        Some(BindError::Config(ConfigError::LoosensPolicy {
            field: "send_window_bytes"
        }))
    );
    let mut bad_client = ClientLimits::v1();
    bad_client.stream_window_bytes = 65_539;
    let connected = QuicClient::connect(client_config(bad_client), loopback(), &hello(1));
    assert_eq!(
        connected.err(),
        Some(ConnectError::Config(ConfigError::InvalidLimits {
            field: "stream_window_bytes"
        }))
    );

    assert_eq!(
        ConfigError::InvalidLimits {
            field: "stream_window_bytes"
        }
        .to_string(),
        "InvalidLimits at quic/config"
    );
}

#[test]
fn stalled_client_window_bounds_server_queue() {
    let welcome = FRAME_HEADER_BYTES + WELCOME_BYTES;
    // (client stream window W, connection window C, payload p, N)
    let rows: [(u32, u32, usize, u32); 3] = [
        (65_540, 65_540, 1_000, 67),
        (262_144, 262_144, 1_000, 263),
        (1_048_576, 2_097_152, 4_000, 263),
    ];
    for (w, c, p, n) in rows {
        let framed = p + FRAME_HEADER_BYTES;
        let window = w as usize;
        assert!(welcome + 2 * framed < window / 8, "no window update");
        assert_eq!((window - welcome) / framed + 2, n as usize, "formula");

        let mut limits = ServerLimits::v1();
        limits.outbound.per_peer_frames = 1;
        limits.outbound.per_peer_bytes = DELTA_CAP;
        let mut server = bind_server(limits);
        let mut client_limits = ClientLimits::v1();
        client_limits.inbound_frames = 1;
        client_limits.inbound_bytes = DELTA_CAP;
        client_limits.stream_window_bytes = w;
        client_limits.connection_window_bytes = c;
        let (mut client, conn) = connect_accepted(&mut server, client_limits, 1, epoch(25));

        fill_to_window_bound(|frame| server.try_send(conn, frame), n, p);
        let stats = server.stats(conn).expect("stats");
        assert_eq!(
            (stats.outbound_frames, stats.outbound_bytes),
            (1, p),
            "W {w}"
        );
        // Guard only: the client's reader parks asynchronously.
        until("client parked", || client.stats().reader_parked);
        let stats = client.stats();
        assert_eq!(
            (
                stats.inbound_frames,
                stats.inbound_bytes,
                stats.reader_parked
            ),
            (1, p, true),
            "W {w}"
        );

        for k in 1..=n {
            assert_eq!(client_recv(&mut client), Ok(pattern(10, k, p)), "W {w}");
        }
        // Positive control: room again once the client has read.
        server
            .try_send(conn, &pattern(10, n + 1, p))
            .expect("send after reads");
        assert_eq!(client_recv(&mut client), Ok(pattern(10, n + 1, p)));
        assert_eq!(client.try_recv(), Ok(None));
    }
}

#[test]
fn stalled_server_window_bounds_client_queue() {
    // The rig's hello carries a 16-byte credential.
    let hello_framed = FRAME_HEADER_BYTES + 2 + 16;
    assert_eq!(hello_framed, 22);
    // (server stream window W, connection window C, payload p, N)
    let rows: [(u32, u32, usize, u32); 2] = [(4_100, 4_100, 100, 41), (65_536, 131_072, 1_000, 67)];
    for (w, c, p, n) in rows {
        let framed = p + FRAME_HEADER_BYTES;
        let window = w as usize;
        assert!(hello_framed + 2 * framed < window / 8, "no window update");
        assert_eq!((window - hello_framed) / framed + 2, n as usize, "formula");

        let mut limits = ServerLimits::v1();
        limits.inbound.per_peer_frames = 1;
        limits.inbound.per_peer_bytes = INTENT_CAP;
        limits.stream_window_bytes = w;
        limits.connection_window_bytes = c;
        let mut server = bind_server(limits);
        let mut client_limits = ClientLimits::v1();
        client_limits.outbound_frames = 1;
        client_limits.outbound_bytes = INTENT_CAP;
        let (mut client, conn) = connect_accepted(&mut server, client_limits, 1, epoch(26));

        fill_to_window_bound(|frame| client.try_send(frame), n, p);
        let stats = client.stats();
        assert_eq!(
            (stats.outbound_frames, stats.outbound_bytes),
            (1, p),
            "W {w}"
        );
        // Guard only: the server's reader parks asynchronously.
        until("server parked", || {
            server.stats(conn).expect("stats").reader_parked
        });
        let stats = server.stats(conn).expect("stats");
        assert_eq!(
            (
                stats.inbound_frames,
                stats.inbound_bytes,
                stats.reader_parked
            ),
            (1, p, true),
            "W {w}"
        );

        for k in 1..=n {
            assert_eq!(
                server_recv(&mut server, conn),
                Ok(pattern(10, k, p)),
                "W {w}"
            );
        }
        // Positive control: room again once the server has read.
        client
            .try_send(&pattern(10, n + 1, p))
            .expect("send after reads");
        assert_eq!(server_recv(&mut server, conn), Ok(pattern(10, n + 1, p)));
        assert_eq!(server.try_recv(conn), Ok(None));
    }
}

#[test]
fn floor_windows_carry_maximal_frames_in_order() {
    let mut limits = ServerLimits::v1();
    limits.stream_window_bytes = 4_100;
    limits.connection_window_bytes = 4_100;
    limits.send_window_bytes = 65_540;
    let mut server = bind_server(limits);
    let mut client_limits = ClientLimits::v1();
    client_limits.stream_window_bytes = 65_540;
    client_limits.connection_window_bytes = 65_540;
    client_limits.send_window_bytes = 4_100;
    let (mut client, conn) = connect_accepted(&mut server, client_limits, 1, epoch(27));

    // One maximal frame each way at the smallest allowed windows.
    server
        .try_send(conn, &pattern(11, 1, DELTA_CAP))
        .expect("server send");
    client
        .try_send(&pattern(12, 1, INTENT_CAP))
        .expect("client send");
    assert_eq!(client_recv(&mut client), Ok(pattern(11, 1, DELTA_CAP)));
    assert_eq!(
        server_recv(&mut server, conn),
        Ok(pattern(12, 1, INTENT_CAP))
    );

    exchange_exactly_once(&mut server, conn, &mut client, 200, INTENT_CAP, DELTA_CAP);

    let report = client.close(CloseCode::Normal, CloseMode::Flush);
    assert_eq!(report.outbound_frames_discarded, 0);
    assert_eq!(report.inbound_frames_discarded, 0);
    assert_eq!(
        server_recv(&mut server, conn),
        Err(ServerError::Closed(CloseReason::Peer { code: 0 }))
    );
    assert_eq!(
        next_closed(&mut server),
        (conn, CloseReason::Peer { code: 0 })
    );
}

// ---------------------------------------------------------------------------
// 28: a Discard close reaches the peer while congestion blocked (T023v).
// ---------------------------------------------------------------------------

/// Server → client datagrams that must have left the server after the
/// client → server path is cut, before the close is issued. With no ACK
/// arriving, bytes in flight never fall, so the server sends one burst up
/// to its congestion window (7–9 datagrams measured) and then only PTO
/// probes, two per probe timeout. 14 therefore means the window was full
/// for at least one probe timeout before the close.
const CONGESTION_BLOCKED_DATAGRAMS: u64 = 14;

#[test]
fn discard_close_reaches_peer_when_congestion_blocked() {
    let mut server = bind_server(ServerLimits::v1());
    let relay = UdpRelay::start(server.local_addr(), RelayConfig::clean());
    let welcome = Welcome {
        wire_version: 1,
        epoch: epoch(28),
    };
    let (mut client, conn) = connect_via_relay(&mut server, &relay, ClientLimits::v1(), welcome);

    // Positive control: the relay forwards a frame.
    server.try_send(conn, &pattern(28, 0, 1_000)).expect("send");
    assert_eq!(client_recv(&mut client), Ok(pattern(28, 0, 1_000)));

    // Cut client → server: from here on no ACK reaches the server.
    relay.set_blackhole_to_server(true);
    let before = relay.forwarded_to_client();
    let n_frames = 16u32;
    for n in 1..=n_frames {
        server
            .try_send(conn, &pattern(28, n, DELTA_CAP))
            .expect("send");
    }
    until("server congestion blocked", || {
        relay.forwarded_to_client() >= before + CONGESTION_BLOCKED_DATAGRAMS
    });
    let report = server
        .close(conn, CloseCode::SessionFenced, CloseMode::Discard)
        .expect("close");
    relay.set_blackhole_to_server(false);
    assert!(relay.dropped() >= 1, "the client's ACKs were cut");

    let mut k = 0u32;
    loop {
        match client_recv(&mut client) {
            Ok(frame) => {
                k += 1;
                assert_eq!(frame, pattern(28, k, DELTA_CAP), "in-order prefix");
            }
            Err(reason) => {
                // quinn-proto 0.11.19 unpatched: `Reset` (T023c C§1.4).
                assert_eq!(reason, CloseReason::Peer { code: 9 });
                break;
            }
        }
    }
    assert!(
        k as usize + report.outbound_frames_discarded <= n_frames as usize,
        "k = {k}, discarded = {}",
        report.outbound_frames_discarded
    );
    assert_eq!(
        next_closed(&mut server),
        (conn, CloseReason::Local(CloseCode::SessionFenced))
    );
}

// ---------------------------------------------------------------------------
// 29–30: a pending close is delivered even when `Closed` is polled at once
// (T023d).
// ---------------------------------------------------------------------------

/// The three host decision codes a pending connection is closed with.
const PENDING_CLOSE_CODES: [CloseCode; 3] = [
    CloseCode::AuthRefused,
    CloseCode::VersionRefused,
    CloseCode::ServerBusy,
];

/// After `Closed` is polled the connection is unknown to the API at once,
/// and the client still receives `code`.
fn assert_forgotten_then_refused(
    server: &mut QuicServer,
    conn: ConnectionId,
    code: CloseCode,
    pending: std::thread::JoinHandle<Result<QuicClient, ConnectError>>,
) {
    assert_eq!(next_closed(server), (conn, CloseReason::Local(code)));
    assert_eq!(server.try_recv(conn), Err(ServerError::UnknownConnection));
    assert_eq!(
        server.stats(conn).err(),
        Some(ServerError::UnknownConnection)
    );
    let refused = pending.join().expect("thread");
    assert_eq!(
        refused.err(),
        Some(ConnectError::Refused {
            code: u64::from(code.as_u32())
        })
    );
}

#[test]
fn refusal_reaches_client_when_closed_is_polled_at_once() {
    let mut server = bind_server(ServerLimits::v1());
    for code in PENDING_CLOSE_CODES {
        let pending = connect_async(
            client_config(ClientLimits::v1()),
            server.local_addr(),
            hello(1),
        );
        let (conn, _, _) = next_hello(&mut server);
        server.refuse(conn, code).expect("refuse");
        assert_forgotten_then_refused(&mut server, conn, code, pending);
    }

    // Positive control: the same server accepts a fourth client, and one
    // frame crosses each way.
    let (mut client, conn) = connect_accepted(&mut server, ClientLimits::v1(), 1, epoch(29));
    client.try_send(&pattern(29, 1, 64)).expect("client send");
    assert_eq!(server_recv(&mut server, conn), Ok(pattern(29, 1, 64)));
    server
        .try_send(conn, &pattern(29, 2, 64))
        .expect("server send");
    assert_eq!(client_recv(&mut client), Ok(pattern(29, 2, 64)));
}

#[test]
fn pending_discard_close_reaches_client_when_polled_at_once() {
    let mut server = bind_server(ServerLimits::v1());
    for code in PENDING_CLOSE_CODES {
        let pending = connect_async(
            client_config(ClientLimits::v1()),
            server.local_addr(),
            hello(1),
        );
        let (conn, _, _) = next_hello(&mut server);
        let report = server.close(conn, code, CloseMode::Discard).expect("close");
        assert_eq!(report.outbound_frames_discarded, 0);
        assert_eq!(report.inbound_frames_discarded, 0);
        assert_forgotten_then_refused(&mut server, conn, code, pending);
    }
}

//! T025b-q acceptance suite (`tasks/T025b.md` N§1–N§9): the resumable QUIC
//! adapter, public API only, over real `crpg-net-quic` loopback.
//!
//! The `/2` client stand-in reads every received frame by its channel with
//! `try_recv_channel`, recording `(Channel, Vec<u8>)` in arrival order; the
//! resume ticket, lane-0 receipts and transfer frames are all read from that
//! raw log. Oracles are hand-written expected bytes and the host's own state,
//! independent of the adapter's projection.
//!
//! Every wait is a guard that panics `"deadline"` after 30 s of real time; no
//! case sleeps as an oracle. Injected time comes only from the rig's
//! `TestClock`.

#[path = "support/quic_host_rig.rs"]
mod rig;

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::time::Duration;

use crpg_net::codec_resume;
use crpg_net::protocol::{self, ReceiptStatus};
use crpg_net::protocol_resume::{
    ConnectionFence, LaneClaim, ResumeProof, ResumeRejection, ResumeRequest,
};
use crpg_net::protocol_transfer::TransferDown;
use crpg_net::protocol_v2::DeltaOp;
use crpg_net_quic::{
    Channel, ClientLimits, CloseReason, ConnectError, ConnectionId, Framing, Hello, QuicClient,
    QuicServer, ServerConfig, ServerLimits, Welcome,
};
use crpg_server::host::{PeerHandle, ResumeAnswer};
use crpg_server::quic::{
    Fence, FenceCause, GrantChange, HelloOutcome, HostCall, InvitationId, QuicHost, QuicHostError,
    ResumeOutcome, ResumeReport,
};
use rig::*;

// ---------------------------------------------------------------------------
// Identities, proofs and the request codec.
// ---------------------------------------------------------------------------

/// The one invitation every case uses: `cred_bytes(0)` = `[0xA1; 32]`,
/// control over A/B/C, full grants. Its first binding is handle 1.
fn aid_bytes() -> Vec<u8> {
    cred_bytes(0)
}

/// The session epoch of the first binding: `epoch_of(7, 1)`.
fn epoch() -> [u8; 16] {
    epoch_of(7, 1)
}

/// N-10: the keyed BLAKE3 proof for `epoch` under credential `[0xA1; 32]`.
fn proof(epoch: [u8; 16]) -> Vec<u8> {
    let mut message = b"crpg-resume-proof/1".to_vec();
    message.extend_from_slice(&epoch);
    blake3::keyed_hash(&[0xA1; 32], &message)
        .as_bytes()
        .to_vec()
}

/// The exact ticket bytes: `[0x91, 0x03, 0x30] ++ epoch ++ [0x20] ++ proof`.
fn ticket_bytes(epoch: [u8; 16]) -> Vec<u8> {
    let mut out = vec![0x91, 0x03, 0x30];
    out.extend_from_slice(&epoch);
    out.push(0x20);
    out.extend_from_slice(&proof(epoch));
    out
}

/// A resume request's canonical bytes.
fn encode_request(
    epoch: [u8; 16],
    fence: u64,
    proof_bytes: &[u8],
    claims: &[(u8, u64)],
) -> Vec<u8> {
    codec_resume::encode_request(&ResumeRequest {
        epoch,
        fence: ConnectionFence::new(fence).expect("nonzero fence"),
        proof: ResumeProof::new(proof_bytes.to_vec()).expect("valid proof"),
        claims: claims
            .iter()
            .map(|(lane, through)| LaneClaim {
                lane: *lane,
                receipted_through: *through,
            })
            .collect(),
    })
    .expect("encodes a resume request")
}

fn grant2() -> ConnectionFence {
    ConnectionFence::new(2).expect("grant 2")
}

/// The accepted answer bytes for `epoch` at fence 2 with lane-0 progress 1.
fn accepted_answer(epoch: [u8; 16]) -> Vec<u8> {
    let mut a = vec![0x91, 0x03, 0x31];
    a.extend_from_slice(&epoch);
    a.extend_from_slice(&[0x02, 0x01, 0x00, 0x01]);
    a
}

// ---------------------------------------------------------------------------
// Loopback builders and the recording client.
// ---------------------------------------------------------------------------

fn start_resumable_v2(limits: ServerLimits) -> (QuicHost, TestClock) {
    let clock = TestClock::new();
    let qh = QuicHost::start_resumable(
        new_host(V2),
        QuicServer::bind_resumable(ServerConfig {
            bind: loopback(),
            identity: identity(),
            limits,
        })
        .expect("bind resumable server"),
        clock.now(),
    )
    .expect("starts over a fresh host");
    (qh, clock)
}

fn start_dual_v2(limits: ServerLimits) -> (QuicHost, TestClock) {
    let clock = TestClock::new();
    let qh = QuicHost::start(
        new_host(V2),
        QuicServer::bind_dual(ServerConfig {
            bind: loopback(),
            identity: identity(),
            limits,
        })
        .expect("bind dual server"),
        clock.now(),
    )
    .expect("starts over a fresh host");
    (qh, clock)
}

fn register_a(qh: &mut QuicHost) -> InvitationId {
    qh.register_invitation(invitation(&aid_bytes(), all_actors(), full_grants()))
        .expect("registers A")
}

/// The live binding's handle for an invitation.
fn bound_peer(qh: &QuicHost, aid: InvitationId) -> PeerHandle {
    qh.invitation_binding(aid)
        .expect("known invitation")
        .expect("bound")
        .1
}

/// A real `QuicClient` plus what it sent and received, read by channel.
struct RClient {
    quic: QuicClient,
    conn: ConnectionId,
    sent: Vec<Vec<u8>>,
    recv: Vec<(Channel, Vec<u8>)>,
    closed: Option<CloseReason>,
}

impl RClient {
    fn new(quic: QuicClient, conn: ConnectionId) -> Self {
        Self {
            quic,
            conn,
            sent: Vec::new(),
            recv: Vec::new(),
            closed: None,
        }
    }

    fn send(&mut self, bytes: Vec<u8>) {
        self.quic.try_send(&bytes).expect("client lane-0 send");
        self.sent.push(bytes);
    }

    fn read_available(&mut self) -> usize {
        let mut count = 0;
        if self.closed.is_some() {
            return 0;
        }
        loop {
            match self.quic.try_recv_channel() {
                Ok(Some((channel, bytes))) => {
                    self.recv.push((channel, bytes));
                    count += 1;
                }
                Ok(None) => break,
                Err(reason) => {
                    self.closed = Some(reason);
                    break;
                }
            }
        }
        count
    }

    fn read_until(&mut self, what: &str, mut cond: impl FnMut(&RClient) -> bool) {
        let start = std::time::Instant::now();
        loop {
            self.read_available();
            if cond(self) {
                return;
            }
            assert!(start.elapsed() < DEADLINE, "deadline: {what}");
            self.quic.wait(Duration::from_millis(5));
        }
    }

    fn read_to_close(&mut self) -> CloseReason {
        self.read_until("client close", |c| c.closed.is_some());
        self.closed.expect("closed")
    }
}

fn connect_framed_async(
    limits: ClientLimits,
    server: SocketAddr,
    version: u8,
    credential_bytes: &[u8],
    framing: Framing,
) -> std::thread::JoinHandle<Result<QuicClient, ConnectError>> {
    let config = client_config(limits);
    let hello = Hello {
        wire_version: version,
        credential: credential(credential_bytes),
    };
    std::thread::spawn(move || QuicClient::connect_framed(config, server, &hello, framing))
}

/// Connects a real `/2` client and returns the recording client plus the
/// pumps that decided its hello.
fn connect_hello(
    qh: &mut QuicHost,
    clock: &mut TestClock,
    server: SocketAddr,
    framing: Framing,
) -> (RClient, Vec<Step>) {
    let pending = connect_framed_async(
        ClientLimits::v1(),
        server,
        version_byte(V2),
        &aid_bytes(),
        framing,
    );
    let (decision, steps) = decide_hello(qh, clock, 100);
    let HelloOutcome::Accepted { .. } = decision.outcome else {
        panic!("expected Accepted, got {decision:?}");
    };
    let quic = pending.join().expect("client thread").expect("connect");
    let client = RClient::new(quic, decision.conn);
    (client, steps)
}

/// Connects to the adapter's own loopback address.
fn connect_hello_direct(
    qh: &mut QuicHost,
    clock: &mut TestClock,
    framing: Framing,
) -> (RClient, Vec<Step>) {
    let addr = qh.local_addr();
    connect_hello(qh, clock, addr, framing)
}

fn connect_resume_async(
    limits: ClientLimits,
    server: SocketAddr,
    session: Welcome,
    request: &[u8],
) -> std::thread::JoinHandle<Result<(QuicClient, Vec<u8>), ConnectError>> {
    let config = client_config(limits);
    let request = request.to_vec();
    std::thread::spawn(move || QuicClient::connect_resume(config, server, session, &request))
}

/// Opens a resume connection and pumps until its notice is decided.
fn connect_resume(
    qh: &mut QuicHost,
    clock: &mut TestClock,
    session: Welcome,
    request: &[u8],
) -> (Result<(QuicClient, Vec<u8>), ConnectError>, Vec<Step>) {
    let pending = connect_resume_async(ClientLimits::v1(), qh.local_addr(), session, request);
    let steps = pump_until(qh, clock, 0, "resume decided", |_, report| {
        !report.resumes.is_empty()
    });
    let outcome = pending.join().expect("client thread");
    (outcome, steps)
}

/// Every receipt (seq, status, processed_tick) in the raw receive log.
fn lane0_receipts(recv: &[(Channel, Vec<u8>)]) -> Vec<(u64, ReceiptStatus, u64)> {
    let mut out = Vec::new();
    for (channel, bytes) in recv {
        if *channel != Channel::Lane0 {
            continue;
        }
        let frame = decode_frame(V2, bytes);
        for op in &frame.ops {
            if let DeltaOp::Legacy(protocol::DeltaOp::Receipt {
                seq,
                processed_tick,
                status,
                ..
            }) = op
            {
                out.push((*seq, *status, *processed_tick));
            }
        }
    }
    out
}

fn decode_transfer(bytes: &[u8]) -> TransferDown {
    crpg_net::codec_transfer::decode_down(bytes).expect("transfer frame decodes")
}

fn declare1() -> Vec<u8> {
    declare(2, epoch(), 1, 0, 1, strike(), 2)
}

fn grace_limits() -> ServerLimits {
    ServerLimits {
        idle_timeout_ms: 1_000,
        keep_alive_ms: 300,
        ..ServerLimits::v1()
    }
}

// ---------------------------------------------------------------------------
// The in-memory replay oracle (N2).
// ---------------------------------------------------------------------------

/// Replays every traced host call against a fresh in-memory host. Ingest and
/// resume-request bytes are recovered from the clients' send logs by
/// connection and ordinal; tickets are rebuilt with `proof(epoch)`.
fn replay(
    steps: &[Step],
    sent: &BTreeMap<ConnectionId, Vec<Vec<u8>>>,
    resume_requests: &BTreeMap<ConnectionId, Vec<u8>>,
    epoch: [u8; 16],
) -> Result<(), String> {
    let mut host = new_host(V2);
    for (index, (now, report)) in steps.iter().enumerate() {
        for call in &report.calls {
            match call {
                HostCall::BindPeer { result, .. } => {
                    let got = host.bind_peer(all_actors(), full_grants(), *now);
                    if got != *result {
                        return Err(format!("step {index}: bind {got:?} != {result:?}"));
                    }
                }
                HostCall::BindSnapshotPeer { result, .. } => {
                    let got = host.bind_snapshot_peer(all_actors(), full_grants(), *now);
                    if got != *result {
                        return Err(format!("step {index}: snapshot bind {got:?} != {result:?}"));
                    }
                }
                HostCall::UnbindPeer { peer } => host.unbind_peer(*peer),
                HostCall::Ingest {
                    conn,
                    peer,
                    frame,
                    len,
                    result,
                } => {
                    let bytes = sent
                        .get(conn)
                        .and_then(|log| log.get(*frame as usize - 1))
                        .ok_or(format!("step {index}: no frame {frame} on {conn:?}"))?;
                    if bytes.len() != *len {
                        return Err(format!("step {index}: length differs"));
                    }
                    let got = host.ingest(*peer, bytes, *now);
                    if got != Ok(result.clone()) {
                        return Err(format!("step {index}: ingest {got:?} != {result:?}"));
                    }
                }
                HostCall::IngestTransfer {
                    conn,
                    peer,
                    frame,
                    len,
                    result,
                } => {
                    let bytes = sent
                        .get(conn)
                        .and_then(|log| log.get(*frame as usize - 1))
                        .ok_or(format!("step {index}: no frame {frame} on {conn:?}"))?;
                    if bytes.len() != *len {
                        return Err(format!("step {index}: length differs"));
                    }
                    let got = host.ingest_transfer(*peer, bytes, *now);
                    if got != Ok(result.clone()) {
                        return Err(format!("step {index}: transfer {got:?} != {result:?}"));
                    }
                }
                HostCall::Pump { summary } => {
                    let got = host.pump(*now);
                    if got.as_ref() != Ok(summary) {
                        return Err(format!("step {index}: pump {got:?} != {summary:?}"));
                    }
                }
                HostCall::TakeDelivery { peer, result } => {
                    let got = host.take_delivery_entries(*peer);
                    let count = got.as_ref().map(Vec::len).map_err(Clone::clone);
                    if count != *result {
                        return Err(format!("step {index}: take {count:?} != {result:?}"));
                    }
                }
                HostCall::AcknowledgeDelivery { peer, through } => {
                    host.acknowledge_delivery(*peer, *through)
                        .map_err(|e| format!("step {index}: ack {e:?}"))?;
                }
                HostCall::IssueResumeTicket { peer, result } => {
                    let proof = ResumeProof::new(proof(epoch)).expect("32-byte proof");
                    let got = host.issue_resume_ticket(*peer, proof);
                    if got.as_ref().map(|_| ()).map_err(Clone::clone) != *result {
                        return Err(format!("step {index}: ticket {got:?} != {result:?}"));
                    }
                }
                HostCall::CheckFrame {
                    peer,
                    fence,
                    result,
                    ..
                } => {
                    let got = host.check_frame(*peer, *fence);
                    if got != *result {
                        return Err(format!("step {index}: check {got:?} != {result:?}"));
                    }
                }
                HostCall::BeginGrace { peer, result } => {
                    let got = host.begin_grace(*peer, *now);
                    if got != *result {
                        return Err(format!("step {index}: grace {got:?} != {result:?}"));
                    }
                }
                HostCall::ResumePeer { conn, result } => {
                    let bytes = resume_requests
                        .get(conn)
                        .ok_or(format!("step {index}: no resume request on {conn:?}"))?;
                    let request = codec_resume::decode_request(bytes)
                        .map_err(|e| format!("step {index}: resume decode {e:?}"))?;
                    let got = host.resume_peer(&request, *now);
                    if got != *result {
                        return Err(format!("step {index}: resume {got:?} != {result:?}"));
                    }
                }
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// N1 `ticket_follows_the_welcome` (N-3, N-9, N-10).
// ---------------------------------------------------------------------------

#[test]
fn ticket_follows_the_welcome() {
    let (mut qh, mut clock) = start_resumable_v2(ServerLimits::v1());
    let aid = register_a(&mut qh);
    let (mut client, steps) = connect_hello_direct(&mut qh, &mut clock, Framing::V2);
    client.read_until("ticket then initial", |c| c.recv.len() >= 2);

    // The first frame after the welcome is the resume ticket.
    let (channel, bytes) = &client.recv[0];
    assert_eq!(*channel, Channel::Resume);
    assert_eq!(bytes, &ticket_bytes(epoch()));
    // Then a lane-0 frame at event_seq 1.
    let (channel, bytes) = &client.recv[1];
    assert_eq!(*channel, Channel::Lane0);
    assert_eq!(decode_frame(V2, bytes).event_seq, 1);

    // Calls: the accepted hello's bind is immediately followed by the
    // ticket issue (earlier pumps were empty and logged only `Pump`).
    let callset = calls(&steps);
    let h1 = bound_peer(&qh, aid);
    assert_eq!(h1.get(), 1);
    let issue_pos = callset
        .iter()
        .position(|call| matches!(call, HostCall::IssueResumeTicket { .. }))
        .expect("issue ticket");
    assert!(issue_pos > 0, "the ticket issue follows the bind");
    assert!(
        matches!(
            &callset[issue_pos - 1],
            HostCall::BindSnapshotPeer {
                invitation,
                result: Ok(peer),
                ..
            } if *invitation == aid && *peer == h1
        ),
        "the bind precedes the ticket issue: {callset:?}"
    );
    assert!(
        matches!(
            &callset[issue_pos],
            HostCall::IssueResumeTicket { peer, result: Ok(()) } if *peer == h1
        ),
        "the ticket issue succeeds: {callset:?}"
    );

    // N-9: no proof or credential bytes leave the adapter.
    assert!(!format!("{callset:?}").contains(&format!("{:?}", proof(epoch()))));
    assert!(!format!("{qh:?}").contains(&format!("{:?}", aid_bytes())));

    // A `/1` client on the same adapter gets no ticket and no issue call.
    let (mut v1client, v1steps) = connect_hello_direct(&mut qh, &mut clock, Framing::V1);
    v1client.read_until("v1 initial", |c| c.recv.len() >= 1);
    assert!(
        v1client
            .recv
            .iter()
            .all(|(channel, _)| *channel != Channel::Resume),
        "a /1 client gets no resume frame"
    );
    assert!(
        calls(&v1steps)
            .iter()
            .all(|call| !matches!(call, HostCall::IssueResumeTicket { .. })),
        "a /1 binding gets no issue ticket"
    );

    // A `start` adapter gives a `/2` client none.
    let (mut qh2, mut clock2) = start_dual_v2(ServerLimits::v1());
    let _aid2 = register_a(&mut qh2);
    let (mut plain, plain_steps) = connect_hello_direct(&mut qh2, &mut clock2, Framing::V2);
    plain.read_until("plain initial", |c| c.recv.len() >= 1);
    assert!(
        plain
            .recv
            .iter()
            .all(|(channel, _)| *channel != Channel::Resume),
        "a start adapter sends no ticket"
    );
    assert!(
        calls(&plain_steps)
            .iter()
            .all(|call| !matches!(call, HostCall::IssueResumeTicket { .. })),
        "a start adapter makes no issue ticket"
    );
}

// ---------------------------------------------------------------------------
// N2 `silent_loss_then_resume` (N-4, N-5, N-7).
// ---------------------------------------------------------------------------

#[test]
fn silent_loss_then_resume() {
    let (mut qh, mut clock) = start_resumable_v2(grace_limits());
    let aid = register_a(&mut qh);
    let relay = UdpRelay::start(qh.local_addr(), RelayConfig::clean());

    let mut sent: BTreeMap<ConnectionId, Vec<Vec<u8>>> = BTreeMap::new();
    let mut resume_requests: BTreeMap<ConnectionId, Vec<u8>> = BTreeMap::new();
    let mut all_steps: Vec<Step> = Vec::new();

    // Connect through the relay, read the ticket and initial state.
    let (mut c1, steps) = connect_hello(&mut qh, &mut clock, relay.addr(), Framing::V2);
    all_steps.extend(steps);
    let h1 = bound_peer(&qh, aid);
    c1.read_until("ticket then initial", |c| c.recv.len() >= 2);
    // A strikes B (declare seq 1), then read the receipt.
    let d1 = declare1();
    c1.send(d1.clone());
    sent.insert(c1.conn, vec![d1.clone()]);
    all_steps.extend(pump_until(
        &mut qh,
        &mut clock,
        100,
        "declare applied",
        |qh, _| qh.host().executions() == 1,
    ));
    c1.read_until("receipt", |c| !lane0_receipts(&c.recv).is_empty());
    let receipts = lane0_receipts(&c1.recv);
    assert_eq!(receipts.len(), 1);
    assert_eq!(receipts[0].0, 1);
    assert_eq!(receipts[0].1, ReceiptStatus::Applied);
    let original_tick = receipts[0].2;

    // Graced via the relay: blackhole and wait for the grace.
    relay.set_blackhole(true);
    all_steps.extend(pump_until(
        &mut qh,
        &mut clock,
        100,
        "begin grace",
        |_, report| {
            report
                .calls
                .iter()
                .any(|call| matches!(call, HostCall::BeginGrace { result: Ok(_), .. }))
        },
    ));
    let callset = calls(&all_steps);
    let deadline = callset
        .iter()
        .find_map(|call| match call {
            HostCall::BeginGrace { result: Ok(d), .. } => Some(*d),
            _ => None,
        })
        .expect("begin grace deadline");
    assert!(
        callset
            .iter()
            .all(|call| !matches!(call, HostCall::UnbindPeer { peer } if *peer == h1)),
        "a silent loss never unbinds"
    );
    let t = deadline - 30_000;

    // Resume at T + 29_999, just inside the grace.
    clock.set(t + 29_999);
    let request = encode_request(epoch(), 1, &proof(epoch()), &[(0, 1)]);
    let (resume, resume_steps) = connect_resume(
        &mut qh,
        &mut clock,
        Welcome {
            wire_version: 2,
            epoch: epoch(),
        },
        &request,
    );
    all_steps.extend(resume_steps);
    let (quic, answer) = resume.expect("resume accepted");
    assert_eq!(answer, accepted_answer(epoch()));

    // The resume report.
    let resumes: Vec<ResumeReport> = all_steps
        .iter()
        .flat_map(|(_, report)| report.resumes.clone())
        .collect();
    assert_eq!(resumes.len(), 1);
    let conn2 = resumes[0].conn;
    assert_eq!(
        resumes[0].outcome,
        ResumeOutcome::Accepted {
            invitation: aid,
            peer: h1,
            granted: grant2(),
        }
    );
    resume_requests.insert(conn2, request);

    // The client's next frames are the resume snapshot.
    let mut c2 = RClient::new(quic, conn2);
    c2.read_until("snapshot", |c| {
        c.recv
            .iter()
            .filter(|(ch, _)| *ch == Channel::Transfer)
            .count()
            >= 2
    });
    let transfers: Vec<&Vec<u8>> = c2
        .recv
        .iter()
        .filter(|(ch, _)| *ch == Channel::Transfer)
        .map(|(_, bytes)| bytes)
        .collect();
    let begin = match decode_transfer(transfers[0]) {
        TransferDown::Begin(begin) => begin,
        other => panic!("expected Begin, got {other:?}"),
    };
    assert_eq!(begin.transfer.get(), 1);
    assert_eq!(begin.cursor, 2);
    assert!(matches!(
        decode_transfer(transfers[1]),
        TransferDown::Chunk(_)
    ));

    // Resend the stored seq-1 bytes: cached receipt, no re-execution.
    c2.send(d1.clone());
    sent.insert(conn2, vec![d1.clone()]);
    all_steps.extend(pump_until(
        &mut qh,
        &mut clock,
        100,
        "cached receipt",
        |_, report| {
            report
                .calls
                .iter()
                .any(|call| matches!(call, HostCall::Ingest { conn, .. } if *conn == conn2))
        },
    ));
    c2.read_until("cached receipt", |c| !lane0_receipts(&c.recv).is_empty());
    let cached = lane0_receipts(&c2.recv);
    assert_eq!(cached.len(), 1);
    assert_eq!(cached[0].0, 1);
    assert_eq!(cached[0].1, ReceiptStatus::Applied);
    assert_eq!(cached[0].2, original_tick);
    assert_eq!(qh.host().executions(), 1);

    // N-4: every Ingest follows a CheckFrame Ok for the same connection,
    // fence 1 before the loss, 2 after.
    let all_calls = calls(&all_steps);
    let mut fences = Vec::new();
    for (idx, call) in all_calls.iter().enumerate() {
        if let HostCall::Ingest { conn, .. } = call {
            let prev = if idx > 0 {
                &all_calls[idx - 1]
            } else {
                panic!("ingest with no preceding call");
            };
            let HostCall::CheckFrame {
                conn: check_conn,
                fence,
                result: Ok(()),
                ..
            } = prev
            else {
                panic!("ingest without a preceding CheckFrame at {idx}");
            };
            assert_eq!(*check_conn, *conn, "CheckFrame must be for the same conn");
            fences.push(fence.get());
        }
    }
    assert_eq!(fences, vec![1, 2], "fence 1 before the loss, 2 after");

    // N2's in-memory replay oracle: every result reproduces.
    replay(&all_steps, &sent, &resume_requests, epoch()).expect("the resumable trace replays");
}

// ---------------------------------------------------------------------------
// N3 `live_resume_fences_the_old_connection` (N-5).
// ---------------------------------------------------------------------------

#[test]
fn live_resume_fences_the_old_connection() {
    let (mut qh, mut clock) = start_resumable_v2(ServerLimits::v1());
    let aid = register_a(&mut qh);
    let (mut c1, _steps) = connect_hello_direct(&mut qh, &mut clock, Framing::V2);
    c1.read_until("ticket then initial", |c| c.recv.len() >= 2);
    let h1 = bound_peer(&qh, aid);
    assert_eq!(qh.host().epoch(h1), Ok(epoch()));

    let request = encode_request(epoch(), 1, &proof(epoch()), &[(0, 0)]);
    let (resume, steps) = connect_resume(
        &mut qh,
        &mut clock,
        Welcome {
            wire_version: 2,
            epoch: epoch(),
        },
        &request,
    );
    let (quic, _answer) = resume.expect("resume accepted");

    let resumes: Vec<ResumeReport> = steps
        .iter()
        .flat_map(|(_, report)| report.resumes.clone())
        .collect();
    assert_eq!(resumes.len(), 1);
    let conn2 = resumes[0].conn;
    assert_eq!(
        resumes[0].outcome,
        ResumeOutcome::Accepted {
            invitation: aid,
            peer: h1,
            granted: grant2(),
        }
    );
    // The epoch is unchanged throughout; the old connection is fenced with
    // `Resumed`, never unbound.
    assert_eq!(qh.host().epoch(h1), Ok(epoch()));
    let fences: Vec<Fence> = steps.iter().flat_map(|(_, r)| r.fences.clone()).collect();
    assert_eq!(
        fences,
        vec![Fence {
            conn: c1.conn,
            peer: h1,
            cause: FenceCause::Resumed,
        }]
    );
    assert!(
        calls(&steps)
            .iter()
            .all(|call| !matches!(call, HostCall::UnbindPeer { .. })),
        "a live resume never unbinds"
    );

    // The old connection reads a `SessionFenced` close (code 9).
    let reason = c1.read_to_close();
    assert_eq!(reason, CloseReason::Peer { code: 9 });

    // The new connection reads the snapshot.
    let mut c2 = RClient::new(quic, conn2);
    c2.read_until("snapshot", |c| {
        c.recv.iter().any(|(ch, _)| *ch == Channel::Transfer)
    });
    let begin = c2
        .recv
        .iter()
        .find_map(|(ch, bytes)| (*ch == Channel::Transfer).then(|| decode_transfer(bytes)))
        .expect("a transfer frame");
    assert!(matches!(begin, TransferDown::Begin(_)));

    // Control: a fresh hello with A supersedes and unbinds instead.
    let (mut qh2, mut clock2) = start_resumable_v2(ServerLimits::v1());
    let aid2 = register_a(&mut qh2);
    let (mut d1, _) = connect_hello_direct(&mut qh2, &mut clock2, Framing::V2);
    d1.read_until("ticket then initial", |c| c.recv.len() >= 2);
    let dh = bound_peer(&qh2, aid2);
    let (_, dsteps) = connect_hello_direct(&mut qh2, &mut clock2, Framing::V2);
    assert!(
        calls(&dsteps)
            .iter()
            .any(|call| matches!(call, HostCall::UnbindPeer { peer } if *peer == dh)),
        "a superseding hello unbinds"
    );
}

// ---------------------------------------------------------------------------
// N4 `refusals_hang_up_with_a_code` (N-5; Question 2 (b)).
// ---------------------------------------------------------------------------

#[test]
fn refusals_hang_up_with_a_code() {
    let (mut qh, mut clock) = start_resumable_v2(ServerLimits::v1());
    let aid = register_a(&mut qh);
    let (mut c1, _) = connect_hello_direct(&mut qh, &mut clock, Framing::V2);
    c1.read_until("ticket then initial", |c| c.recv.len() >= 2);
    let h1 = bound_peer(&qh, aid);

    // (1) A malformed request: refused 5, no host call.
    let (bad, bad_steps) = connect_resume(
        &mut qh,
        &mut clock,
        Welcome {
            wire_version: 2,
            epoch: epoch(),
        },
        &[0x91, 0x03, 0x40],
    );
    assert!(matches!(bad, Err(ConnectError::Refused { code: 5 })));
    assert!(
        calls(&bad_steps)
            .iter()
            .all(|call| !matches!(call, HostCall::ResumePeer { .. })),
        "a malformed request makes no host call"
    );
    assert_eq!(
        bad_steps
            .iter()
            .flat_map(|(_, r)| r.resumes.clone())
            .count(),
        1
    );

    // (2) A wrong proof: refused 5, traced Rejected(Denied).
    let (wrong, wrong_steps) = connect_resume(
        &mut qh,
        &mut clock,
        Welcome {
            wire_version: 2,
            epoch: epoch(),
        },
        &encode_request(epoch(), 1, &[0xA6; 32], &[(0, 0)]),
    );
    assert!(matches!(wrong, Err(ConnectError::Refused { code: 5 })));
    assert!(
        calls(&wrong_steps).iter().any(|call| matches!(
            call,
            HostCall::ResumePeer {
                result: Ok(ResumeAnswer::Rejected(rejected)),
                ..
            } if rejected.epoch == epoch() && rejected.reason == ResumeRejection::Denied
        )),
        "a wrong proof is a traced Denied"
    );

    // (3) A wrong epoch: refused 5.
    let (wrong_epoch, _) = connect_resume(
        &mut qh,
        &mut clock,
        Welcome {
            wire_version: 2,
            epoch: epoch_of(7, 99),
        },
        &encode_request(epoch_of(7, 99), 1, &proof(epoch_of(7, 99)), &[(0, 0)]),
    );
    assert!(matches!(
        wrong_epoch,
        Err(ConnectError::Refused { code: 5 })
    ));

    // (4) The correct request still resumes (no refusal moved the fence).
    let request = encode_request(epoch(), 1, &proof(epoch()), &[(0, 0)]);
    let (ok, steps) = connect_resume(
        &mut qh,
        &mut clock,
        Welcome {
            wire_version: 2,
            epoch: epoch(),
        },
        &request,
    );
    assert!(ok.is_ok());
    let resumes: Vec<ResumeReport> = steps
        .iter()
        .flat_map(|(_, report)| report.resumes.clone())
        .collect();
    assert_eq!(
        resumes[0].outcome,
        ResumeOutcome::Accepted {
            invitation: aid,
            peer: h1,
            granted: grant2(),
        }
    );

    // (5) A graced session resumed at the deadline is denied (expired).
    let (mut qh3, mut clock3) = start_resumable_v2(grace_limits());
    let aid3 = register_a(&mut qh3);
    let relay = UdpRelay::start(qh3.local_addr(), RelayConfig::clean());
    let (mut c3, _) = connect_hello(&mut qh3, &mut clock3, relay.addr(), Framing::V2);
    c3.read_until("ticket then initial", |c| c.recv.len() >= 2);
    let h3 = bound_peer(&qh3, aid3);
    relay.set_blackhole(true);
    let grace_steps = pump_until(&mut qh3, &mut clock3, 100, "begin grace", |_, report| {
        report
            .calls
            .iter()
            .any(|call| matches!(call, HostCall::BeginGrace { result: Ok(_), .. }))
    });
    let deadline = calls(&grace_steps)
        .iter()
        .find_map(|call| match call {
            HostCall::BeginGrace { result: Ok(d), .. } => Some(*d),
            _ => None,
        })
        .expect("deadline");
    clock3.set(deadline);
    let (expired, _) = connect_resume(
        &mut qh3,
        &mut clock3,
        Welcome {
            wire_version: 2,
            epoch: epoch(),
        },
        &encode_request(epoch(), 1, &proof(epoch()), &[(0, 0)]),
    );
    assert!(matches!(expired, Err(ConnectError::Refused { code: 5 })));
    assert_eq!(qh3.host().grace_deadline(h3), None);
}

// ---------------------------------------------------------------------------
// N5 `what_ends_a_grace` (N-6, N-7, N-8).
// ---------------------------------------------------------------------------

/// Connects a `/2` client through a relay, blackholes it and returns the
/// graced handle and deadline once `BeginGrace` is traced.
fn grace_over_relay(
    qh: &mut QuicHost,
    clock: &mut TestClock,
) -> (RClient, UdpRelay, PeerHandle, u64) {
    let relay = UdpRelay::start(qh.local_addr(), RelayConfig::clean());
    let (client, _) = connect_hello(qh, clock, relay.addr(), Framing::V2);
    relay.set_blackhole(true);
    let steps = pump_until(qh, clock, 100, "begin grace", |_, report| {
        report
            .calls
            .iter()
            .any(|call| matches!(call, HostCall::BeginGrace { result: Ok(_), .. }))
    });
    let callset = calls(&steps);
    let peer = callset
        .iter()
        .find_map(|call| match call {
            HostCall::BeginGrace { peer, .. } => Some(*peer),
            _ => None,
        })
        .expect("graced peer");
    let deadline = callset
        .iter()
        .find_map(|call| match call {
            HostCall::BeginGrace { result: Ok(d), .. } => Some(*d),
            _ => None,
        })
        .expect("deadline");
    (client, relay, peer, deadline)
}

#[test]
fn what_ends_a_grace() {
    // (a) A peer's own close (Normal) unbinds, never begins a grace.
    let (mut qh, mut clock) = start_resumable_v2(grace_limits());
    let aid = register_a(&mut qh);
    let (c1, _) = connect_hello_direct(&mut qh, &mut clock, Framing::V2);
    let h1 = bound_peer(&qh, aid);
    drop(c1);
    let steps = pump_until(&mut qh, &mut clock, 100, "normal close", |_, report| {
        report
            .calls
            .iter()
            .any(|call| matches!(call, HostCall::UnbindPeer { peer } if *peer == h1))
    });
    assert!(
        calls(&steps)
            .iter()
            .all(|call| !matches!(call, HostCall::BeginGrace { .. })),
        "a Normal close starts no grace"
    );

    // (b) Revoking the invitation ends the grace: the resume is code 5.
    let (mut qh, mut clock) = start_resumable_v2(grace_limits());
    let aid = register_a(&mut qh);
    let (_c, _relay, _peer, _deadline) = grace_over_relay(&mut qh, &mut clock);
    qh.revoke_invitation(aid).expect("revokes");
    let (resume, _) = connect_resume(
        &mut qh,
        &mut clock,
        Welcome {
            wire_version: 2,
            epoch: epoch(),
        },
        &encode_request(epoch(), 1, &proof(epoch()), &[(0, 0)]),
    );
    assert!(matches!(resume, Err(ConnectError::Refused { code: 5 })));

    // (c) A hello with A ends the grace first: UnbindPeer then a fresh bind.
    let (mut qh, mut clock) = start_resumable_v2(grace_limits());
    let _aid = register_a(&mut qh);
    let (_c, _relay, peer, deadline) = grace_over_relay(&mut qh, &mut clock);
    assert_eq!(qh.host().grace_deadline(peer), Some(deadline));
    let (_c2, steps) = connect_hello_direct(&mut qh, &mut clock, Framing::V2);
    let callset = calls(&steps);
    let unbind_pos = callset
        .iter()
        .position(|call| matches!(call, HostCall::UnbindPeer { peer: p } if *p == peer))
        .expect("graced peer unbound");
    let bind_pos = callset
        .iter()
        .position(|call| matches!(call, HostCall::BindSnapshotPeer { result: Ok(_), .. }))
        .expect("fresh bind");
    assert!(
        unbind_pos < bind_pos,
        "the grace ends before the fresh bind"
    );
    assert_eq!(qh.host().grace_deadline(peer), None);

    // (d) Changing the grants of A ends the grace and returns Stored.
    let (mut qh, mut clock) = start_resumable_v2(grace_limits());
    let aid = register_a(&mut qh);
    let (_c, _relay, peer, deadline) = grace_over_relay(&mut qh, &mut clock);
    assert_eq!(qh.host().grace_deadline(peer), Some(deadline));
    let change = qh.set_invitation_grants(aid, observer_grants());
    assert_eq!(change, Ok(GrantChange::Stored));
    assert_eq!(qh.host().grace_deadline(peer), None);

    // (e) Expiry, then a hello binds with no unbind.
    let (mut qh, mut clock) = start_resumable_v2(grace_limits());
    let _aid = register_a(&mut qh);
    let (_c, _relay, _peer, deadline) = grace_over_relay(&mut qh, &mut clock);
    clock.set(deadline);
    let _ = qh.pump(clock.now()).expect("sweep pump");
    let (_c2, steps) = connect_hello_direct(&mut qh, &mut clock, Framing::V2);
    assert!(
        calls(&steps)
            .iter()
            .all(|call| !matches!(call, HostCall::UnbindPeer { .. })),
        "an expired grace is gone before the hello"
    );
}

// ---------------------------------------------------------------------------
// N6 `start_is_unchanged` (N-1, N-7).
// ---------------------------------------------------------------------------

#[test]
fn start_is_unchanged() {
    // `start` stays non-resumable: no ticket, no grace.
    let (mut qh, mut clock) = start_dual_v2(ServerLimits::v1());
    assert!(!qh.is_resumable());
    let _aid = register_a(&mut qh);
    let (mut c1, _) = connect_hello_direct(&mut qh, &mut clock, Framing::V2);
    c1.read_until("initial", |c| c.recv.len() >= 1);
    assert!(
        c1.recv
            .iter()
            .all(|(channel, _)| *channel != Channel::Resume),
        "a start adapter sends no resume ticket"
    );

    // Silent loss on a `start` adapter: unbind, no grace.
    let (mut qh2, mut clock2) = start_dual_v2(grace_limits());
    let aid2 = register_a(&mut qh2);
    let relay = UdpRelay::start(qh2.local_addr(), RelayConfig::clean());
    let (mut c2, _) = connect_hello(&mut qh2, &mut clock2, relay.addr(), Framing::V2);
    c2.read_until("initial", |c| c.recv.len() >= 1);
    let h2 = bound_peer(&qh2, aid2);
    relay.set_blackhole(true);
    let steps = pump_until(&mut qh2, &mut clock2, 100, "silent loss", |_, report| {
        report
            .calls
            .iter()
            .any(|call| matches!(call, HostCall::UnbindPeer { peer } if *peer == h2))
    });
    assert!(
        calls(&steps)
            .iter()
            .all(|call| !matches!(call, HostCall::BeginGrace { .. })),
        "a start adapter never begins a grace"
    );

    // `start_resumable` refuses a host holding a grace.
    let mut graced_host = new_host(V2);
    let g = graced_host
        .bind_snapshot_peer(all_actors(), full_grants(), CLOCK_START)
        .expect("bind");
    graced_host
        .issue_resume_ticket(g, ResumeProof::new(proof(epoch())).expect("proof"))
        .expect("ticket");
    graced_host.begin_grace(g, CLOCK_START).expect("grace");
    let err = QuicHost::start_resumable(
        graced_host,
        QuicServer::bind_resumable(ServerConfig {
            bind: loopback(),
            identity: identity(),
            limits: ServerLimits::v1(),
        })
        .expect("bind"),
        CLOCK_START,
    )
    .expect_err("a graced host is in use");
    assert_eq!(err.error, QuicHostError::HostInUse);

    // A closed host.
    let mut closed_host = new_host(V2);
    closed_host.shutdown();
    let err = QuicHost::start_resumable(
        closed_host,
        QuicServer::bind_resumable(ServerConfig {
            bind: loopback(),
            identity: identity(),
            limits: ServerLimits::v1(),
        })
        .expect("bind"),
        CLOCK_START,
    )
    .expect_err("a closed host refuses");
    assert_eq!(err.error, QuicHostError::HostClosed);

    // A regressed clock.
    let err = QuicHost::start_resumable(
        new_host(V2),
        QuicServer::bind_resumable(ServerConfig {
            bind: loopback(),
            identity: identity(),
            limits: ServerLimits::v1(),
        })
        .expect("bind"),
        CLOCK_START - 1,
    )
    .expect_err("a regressed clock refuses");
    assert_eq!(err.error, QuicHostError::TimeRegression);
}

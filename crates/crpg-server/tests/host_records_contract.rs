//! T141s two-keys contract suite (`tasks/T141.md`, part T141s): host records
//! and checkpoint version 2, checked from the contract text through the
//! public `crpg-server` API plus checkpoint JSON surgery.
//!
//! Oracles are independent of the host: a test-owned mirror `HistoryWorld`
//! driven with the same sim calls (acknowledged after each), hand-encoded
//! intent bytes, hand-written expected frames, and checkpoint texts that are
//! assembled in the test from the mirror's own serialization. Every rejected
//! checkpoint edit sits beside an accepted control built the same way.
//!
//! Fixtures follow the contract: `TB3` is `fixtures/three_combatants.json`
//! (A, B, C; strike, smite, whiff; active A) acknowledged through its four
//! start envelopes; `TB3c` is `TB3` at history version 2 with an encounter
//! clock; `RT3` is `TB3c` in real-time mode with no active head and every
//! timeline key 0.

use crpg_core::{EntityId, Ulid};
use crpg_net::codec;
use crpg_net::codec_v2;
use crpg_net::protocol::{self, NetId, ReceiptStatus, LANE_COMBAT};
use crpg_net::protocol_v2::DeltaOp;
use crpg_server::capture::{
    CaptureEntry, CapturedRecord, HostOp, HostRecord, MAX_CAPTURE_BYTES, MAX_CAPTURE_PAGE,
    MAX_CAPTURE_RECORDS, MAX_CAPTURE_RECORD_BYTES, MAX_HOST_RECORD_BYTES, MAX_HOST_RECORD_EVENTS,
    MAX_NEW_EVENTS_PER_COMMAND, MAX_NEW_EVENTS_PER_COMMAND_RT,
};
use crpg_server::checkpoint::{
    load_checkpoint, CheckpointError, CHECKPOINT_VERSION, CHECKPOINT_VERSION_HOST_RECORDS,
};
use crpg_server::host::{
    ControlGrant, DisclosureGrants, EntityDisclosure, Host, HostConfig, HostError,
    IngestDisposition, PeerHandle, ProtocolSelection,
};
use crpg_sim::{
    history_hash, CombatAction, EncounterMode, HistoryEnvelope, HistoryEvent, HistoryWorld,
};
use serde_json::{json, Value};

const FIXTURE: &str = include_str!("fixtures/three_combatants.json");

const V1: ProtocolSelection = ProtocolSelection::V1;
const V2: ProtocolSelection = ProtocolSelection::V2;

// ---------------------------------------------------------------------------
// Identities.
// ---------------------------------------------------------------------------

fn eid(index: u32) -> EntityId {
    serde_json::from_value(json!({"index": index, "generation": 1})).expect("valid entity id")
}
fn a() -> EntityId {
    eid(0)
}
fn b() -> EntityId {
    eid(1)
}
fn c() -> EntityId {
    eid(2)
}
fn strike() -> Ulid {
    Ulid::from_u128(603)
}
fn smite() -> Ulid {
    Ulid::from_u128(604)
}
fn whiff() -> Ulid {
    Ulid::from_u128(605)
}
fn encounter() -> Ulid {
    Ulid::from_u128(601)
}
fn n(raw: u64) -> NetId {
    NetId::new(raw).expect("nonzero")
}

fn full(entity: EntityId) -> EntityDisclosure {
    EntityDisclosure {
        entity,
        present: true,
        health: true,
        spawn: true,
        despawn: true,
        died: true,
        action_actor: true,
        action_target: true,
        action_outcome: true,
        action_damage: true,
        turn_start: true,
    }
}

fn full_grants() -> DisclosureGrants {
    DisclosureGrants {
        entities: vec![full(a()), full(b()), full(c())],
        abilities: vec![strike(), smite(), whiff()],
        encounters: vec![encounter()],
        turn_round: true,
        encounter_end: true,
        turn_state: true,
    }
}

fn all_actors() -> ControlGrant {
    ControlGrant {
        actors: vec![a(), b(), c()],
    }
}

// ---------------------------------------------------------------------------
// Hand-encoded intents (independent of the codec).
// ---------------------------------------------------------------------------

fn varint(out: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        out.push((value as u8) | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

fn header(version: u8, epoch: [u8; 16], seq: u64, observed: u64, actor: u64) -> Vec<u8> {
    let mut out = vec![version];
    out.extend_from_slice(&epoch);
    out.push(LANE_COMBAT);
    varint(&mut out, seq);
    varint(&mut out, observed);
    varint(&mut out, actor);
    out
}

fn declare_bytes(
    version: u8,
    epoch: [u8; 16],
    seq: u64,
    observed: u64,
    actor: u64,
    ability: Ulid,
    target: u64,
) -> Vec<u8> {
    let mut out = header(version, epoch, seq, observed, actor);
    out.push(0);
    let text = ability.to_string();
    varint(&mut out, text.len() as u64);
    out.extend_from_slice(text.as_bytes());
    varint(&mut out, target);
    out
}

fn end_turn_bytes(version: u8, epoch: [u8; 16], seq: u64, observed: u64, actor: u64) -> Vec<u8> {
    let mut out = header(version, epoch, seq, observed, actor);
    out.push(1);
    out
}

// ---------------------------------------------------------------------------
// Fixtures.
// ---------------------------------------------------------------------------

fn tb3_json() -> Value {
    serde_json::from_str(FIXTURE).expect("fixture parses")
}

fn tb3c_json() -> Value {
    let mut v = tb3_json();
    v["version"] = json!(2);
    v["world"]["combat"]["clock"] = json!({"round_ticks": 4, "elapsed": 0});
    v
}

fn rt3_json() -> Value {
    let mut v = tb3c_json();
    v["world"]["combat"]["mode"] = json!("real_time");
    v["world"]["combat"]["active"] = Value::Null;
    for entry in v["world"]["timeline"].as_array_mut().expect("timeline") {
        entry[0] = json!(0);
    }
    v
}

fn history_of(value: &Value) -> HistoryWorld {
    let mut history: HistoryWorld =
        serde_json::from_str(&value.to_string()).expect("fixture loads through validated serde");
    history
        .acknowledge(history.last_sequence())
        .expect("acknowledges its own range");
    history
}

// ---------------------------------------------------------------------------
// Rig: one host, one mirror, one fully-privileged v1/v2 peer `V`.
// ---------------------------------------------------------------------------

struct Rig {
    host: Host,
    mirror: HistoryWorld,
    peer: PeerHandle,
    protocol: ProtocolSelection,
    now: u64,
}

fn decode_ops(protocol: ProtocolSelection, bytes: &[u8]) -> (u64, Vec<DeltaOp>) {
    match protocol {
        ProtocolSelection::V1 => {
            let frame = codec::decode_delta(bytes).expect("v1 delivery decodes");
            (
                frame.event_seq,
                frame.ops.into_iter().map(DeltaOp::Legacy).collect(),
            )
        }
        ProtocolSelection::V2 => {
            let frame = codec_v2::decode_delta(bytes).expect("v2 delivery decodes");
            (frame.event_seq, frame.ops)
        }
    }
}

impl Rig {
    /// Builds the host and mirror from `json`; `realtime_first` runs
    /// `request_mode(RealTime)` on both authorities before the wrap.
    fn new(json: &Value, protocol: ProtocolSelection, realtime_first: bool) -> Self {
        let mut host_history = history_of(json);
        let mut mirror = history_of(json);
        if realtime_first {
            for history in [&mut host_history, &mut mirror] {
                history
                    .request_mode(EncounterMode::RealTime)
                    .expect("mode request");
                history
                    .acknowledge(history.last_sequence())
                    .expect("acknowledges");
            }
        }
        let mut host = Host::from_history(
            host_history,
            HostConfig {
                protocol,
                incarnation: 1,
            },
            0,
        )
        .expect("wraps an acknowledged authority");
        let peer = host
            .bind_peer(all_actors(), full_grants(), 0)
            .expect("binds");
        let mut rig = Self {
            host,
            mirror,
            peer,
            protocol,
            now: 0,
        };
        rig.drain();
        rig
    }

    fn rt3() -> Self {
        Self::new(&rt3_json(), V2, false)
    }

    fn tb3() -> Self {
        Self::new(&tb3_json(), V2, false)
    }

    fn version(&self) -> u8 {
        self.protocol.as_u8()
    }

    /// Takes, decodes and acknowledges every pending frame of `V`.
    fn drain(&mut self) -> Vec<Vec<DeltaOp>> {
        let frames = self.host.take_delivery(self.peer).expect("bound");
        let mut out = Vec::new();
        let mut last = None;
        for bytes in &frames {
            let (event_seq, ops) = decode_ops(self.protocol, bytes);
            last = Some(event_seq);
            out.push(ops);
        }
        if let Some(through) = last {
            self.host
                .acknowledge_delivery(self.peer, through)
                .expect("acks");
        }
        out
    }

    /// Ingests and pumps one command; returns its status and `V`'s frames.
    fn send(&mut self, bytes: &[u8]) -> (ReceiptStatus, Vec<Vec<DeltaOp>>) {
        self.now += 100;
        assert_eq!(
            self.host.ingest(self.peer, bytes, self.now),
            Ok(IngestDisposition::Staged)
        );
        let summary = self.host.pump(self.now).expect("pump");
        assert_eq!(summary.results.len(), 1, "{summary:?}");
        let status = summary.results[0].status;
        (status, self.drain())
    }

    fn declare(&mut self, seq: u64, actor: u64, ability: Ulid, target: u64) -> Vec<Vec<DeltaOp>> {
        let bytes = declare_bytes(
            self.version(),
            self.host.epoch(self.peer).expect("bound"),
            seq,
            self.host.server_tick(),
            actor,
            ability,
            target,
        );
        let (status, frames) = self.send(&bytes);
        assert_eq!(status, ReceiptStatus::Applied, "declare seq {seq}");
        frames
    }

    fn end_turn(&mut self, seq: u64, actor: u64) -> Vec<Vec<DeltaOp>> {
        let bytes = end_turn_bytes(
            self.version(),
            self.host.epoch(self.peer).expect("bound"),
            seq,
            self.host.server_tick(),
            actor,
        );
        let (status, frames) = self.send(&bytes);
        assert_eq!(status, ReceiptStatus::Applied, "end turn seq {seq}");
        frames
    }

    /// Mirrors one accepted sim operation and returns its journal page.
    fn mirrored<F>(&mut self, op: F) -> Vec<HistoryEnvelope>
    where
        F: FnOnce(&mut HistoryWorld),
    {
        let before = self.mirror.last_sequence();
        op(&mut self.mirror);
        let page = if self.mirror.last_sequence() > before {
            self.mirror.read_after(before, 256).expect("readable")
        } else {
            Vec::new()
        };
        self.mirror
            .acknowledge(self.mirror.last_sequence())
            .expect("acks");
        page
    }

    fn m_action(&mut self, action: CombatAction) -> Vec<HistoryEnvelope> {
        self.mirrored(|m| {
            m.perform_action(&action).expect("mirror accepts");
        })
    }

    fn m_tick(&mut self) -> Vec<HistoryEnvelope> {
        self.mirrored(|m| m.tick().expect("mirror ticks"))
    }

    fn assert_authority(&self) {
        assert_eq!(self.host.authority_hash(), history_hash(&self.mirror));
    }

    fn entries(&self) -> Vec<CaptureEntry> {
        self.host
            .read_capture_entries(self.host.capture_acknowledged(), MAX_CAPTURE_PAGE)
            .expect("readable")
    }

    fn saved(&self) -> String {
        String::from_utf8(self.host.save_checkpoint().expect("saves")).expect("utf8")
    }
}

fn cmd(entry: &CaptureEntry) -> &CapturedRecord {
    match entry {
        CaptureEntry::Command(record) => record,
        CaptureEntry::Host(_) => panic!("expected a Command entry, got {entry:?}"),
    }
}

fn hostrec(entry: &CaptureEntry) -> &HostRecord {
    match entry {
        CaptureEntry::Host(record) => record,
        CaptureEntry::Command(_) => panic!("expected a Host entry, got {entry:?}"),
    }
}

fn payloads(events: &[HistoryEnvelope]) -> Vec<HistoryEvent> {
    events.iter().map(|e| e.payload.clone()).collect()
}

fn resolved(
    actor: EntityId,
    target: EntityId,
    ability: Ulid,
    outcome: &str,
    damage: u32,
) -> HistoryEvent {
    HistoryEvent::ActionResolved {
        actor,
        target,
        ability,
        outcome: outcome.to_owned(),
        damage,
    }
}

// ---------------------------------------------------------------------------
// Checkpoint surgery.
// ---------------------------------------------------------------------------

fn parse(text: &str) -> Value {
    serde_json::from_str(text).expect("checkpoint text is JSON")
}

fn emit(value: &Value) -> Vec<u8> {
    serde_json::to_vec(value).expect("encodes")
}

/// Loads under incarnation 2; the host on success, the error otherwise.
fn load(bytes: &[u8]) -> Result<Host, CheckpointError> {
    load_checkpoint(bytes, 2, 0)
}

fn load_err(bytes: &[u8]) -> Option<CheckpointError> {
    load(bytes).err()
}

/// `text` edited through `edit`, loaded.
fn edited_err(text: &str, edit: impl FnOnce(&mut Value)) -> Option<CheckpointError> {
    let mut value = parse(text);
    edit(&mut value);
    load_err(&emit(&value))
}

/// Asserts the untouched text loads and the edit alone is `Malformed`.
fn assert_malformed(label: &str, text: &str, edit: impl FnOnce(&mut Value)) {
    assert_eq!(
        load_err(text.as_bytes()),
        None,
        "{label}: the control text loads"
    );
    assert_eq!(
        edited_err(text, edit),
        Some(CheckpointError::Malformed),
        "{label}: the edit alone is Malformed"
    );
}

fn entry_kinds(host: &Host) -> Vec<char> {
    host.read_capture_entries(host.capture_acknowledged(), MAX_CAPTURE_PAGE)
        .expect("readable")
        .iter()
        .map(|entry| match entry {
            CaptureEntry::Command(_) => 'C',
            CaptureEntry::Host(_) => 'H',
        })
        .collect()
}

fn insert_before_ack(text: &str, member: &str) -> String {
    let at = text.find("\"capture_acknowledged\"").expect("watermark");
    format!("{}{}{}", &text[..at], member, &text[at..])
}

// ---------------------------------------------------------------------------
// Constants and wire shapes.
// ---------------------------------------------------------------------------

#[test]
fn constants_and_serde_names() {
    assert_eq!(MAX_NEW_EVENTS_PER_COMMAND, 3);
    assert_eq!(MAX_NEW_EVENTS_PER_COMMAND_RT, 4);
    assert_eq!(MAX_HOST_RECORD_EVENTS, 4096);
    assert_eq!(MAX_HOST_RECORD_BYTES, 1_114_112);
    assert_eq!(MAX_CAPTURE_RECORD_BYTES, 16_384);
    assert_eq!(MAX_CAPTURE_RECORDS, 4096);
    assert_eq!(MAX_CAPTURE_BYTES, 8 * 1024 * 1024);
    assert_eq!(CHECKPOINT_VERSION, 1);
    assert_eq!(CHECKPOINT_VERSION_HOST_RECORDS, 2);

    assert_eq!(serde_json::to_string(&HostOp::Tick).unwrap(), "\"tick\"");
    assert_eq!(
        serde_json::to_string(&HostOp::Action).unwrap(),
        "\"action\""
    );
    assert_eq!(
        serde_json::to_string(&HostOp::TurnTimer).unwrap(),
        "\"turn_timer\""
    );
    for (text, op) in [
        ("\"tick\"", HostOp::Tick),
        ("\"action\"", HostOp::Action),
        ("\"turn_timer\"", HostOp::TurnTimer),
    ] {
        assert_eq!(serde_json::from_str::<HostOp>(text).unwrap(), op);
    }
    for bad in ["\"Tick\"", "\"TICK\"", "\"bogus\"", "\"\"", "0", "null"] {
        assert!(serde_json::from_str::<HostOp>(bad).is_err(), "{bad}");
    }

    // Declared field order is the JSON order, and unknown fields are refused.
    let record = HostRecord {
        capture_seq: 7,
        op: HostOp::Action,
        history_start: 1,
        history_end: 2,
        processed_tick: 3,
        events: Vec::new(),
    };
    assert_eq!(
        serde_json::to_string(&record).unwrap(),
        "{\"capture_seq\":7,\"op\":\"action\",\"history_start\":1,\"history_end\":2,\"processed_tick\":3,\"events\":[]}"
    );
    assert!(serde_json::from_str::<HostRecord>(
        "{\"capture_seq\":7,\"op\":\"action\",\"history_start\":1,\"history_end\":2,\"processed_tick\":3,\"events\":[],\"x\":0}"
    )
    .is_err());

    assert_eq!(
        HostError::CaptureFull.to_string(),
        "CaptureFull at host/admission"
    );
    assert!(std::error::Error::source(&HostError::CaptureFull).is_none());
}

// ---------------------------------------------------------------------------
// S1: a real-time tick is recorded.
// ---------------------------------------------------------------------------

/// S1 steps 1..=3 on `RT3`; returns the rig and the tick's mirror page.
fn s1_run() -> (Rig, Vec<HistoryEnvelope>) {
    let mut rig = Rig::rt3();

    // (1) V seq 1 DeclareAction strike A -> B.
    rig.declare(1, 1, strike(), 2);
    rig.m_action(CombatAction::UseAbility {
        actor: a(),
        ability: strike(),
        target: b(),
    });
    let entries = rig.entries();
    assert_eq!(entries.len(), 1);
    let first = cmd(&entries[0]);
    assert_eq!(first.capture_seq, 1);
    assert_eq!(first.seq, 1);
    assert_eq!(first.lane, LANE_COMBAT);
    assert_eq!(first.epoch, rig.host.epoch(rig.peer).unwrap());
    assert_eq!((first.history_start, first.history_end), (4, 4));
    assert!(first.events.is_empty());

    // (2) tick().
    assert_eq!(rig.host.tick(), Ok(()));
    let page = rig.m_tick();
    (rig, page)
}

#[test]
fn s1_rt_tick_is_recorded() {
    let (mut rig, page) = s1_run();

    // The tick's one envelope is the mirror's envelope 5.
    assert_eq!(page.len(), 1);
    assert_eq!(page[0].seq, 5);
    assert_eq!(page[0].payload, resolved(a(), b(), strike(), "success", 3));

    let entries = rig.entries();
    assert_eq!(entries.len(), 2);
    assert_eq!(
        entries[1],
        CaptureEntry::Host(HostRecord {
            capture_seq: 2,
            op: HostOp::Tick,
            history_start: 4,
            history_end: 5,
            processed_tick: 1,
            events: page.clone(),
        })
    );
    // Each stored field, named.
    let record = hostrec(&entries[1]);
    assert_eq!(record.capture_seq, 2);
    assert_eq!(record.op, HostOp::Tick);
    assert_eq!(record.history_start, 4);
    assert_eq!(record.history_end, 5);
    assert_eq!(record.processed_tick, 1);
    assert_eq!(record.events, page);
    assert_eq!(entries[1].capture_seq(), 2);

    assert_eq!(rig.host.last_capture(), 2);
    assert_eq!(rig.host.executions(), 1, "a tick is not a command");
    assert!(
        rig.drain().is_empty(),
        "a host operation pushes no frame to a v2 session"
    );
    rig.assert_authority();

    // (3) V seq 2 DeclareAction strike B -> C.
    rig.declare(2, 2, strike(), 3);
    rig.m_action(CombatAction::UseAbility {
        actor: b(),
        ability: strike(),
        target: c(),
    });
    let entries = rig.entries();
    assert_eq!(entries.len(), 3);
    let third = cmd(&entries[2]);
    assert_eq!(third.capture_seq, 3);
    assert_eq!(third.seq, 2);
    assert_eq!((third.history_start, third.history_end), (5, 5));
    assert_eq!(third.processed_tick, 1);
    assert_eq!(rig.host.executions(), 2);
    assert_eq!(entry_kinds(&rig.host), vec!['C', 'H', 'C']);
    assert_eq!(rig.host.last_capture(), 3);
    rig.assert_authority();

    // Reads: both kinds by entries, Commands only by read_captures.
    let all = rig.host.read_capture_entries(0, 256).unwrap();
    assert_eq!(all, entries);
    let commands = rig.host.read_captures(0, 256).unwrap();
    assert_eq!(
        commands.iter().map(|r| r.capture_seq).collect::<Vec<_>>(),
        vec![1, 3]
    );
    assert_eq!(&commands[0], cmd(&entries[0]));
    assert_eq!(&commands[1], cmd(&entries[2]));
    // A page of two entries holds one record.
    let two = rig.host.read_captures(0, 2).unwrap();
    assert_eq!(two.len(), 1);
    assert_eq!(two[0].capture_seq, 1);
    // A page that is only a host record holds no command.
    assert_eq!(rig.host.read_captures(1, 1).unwrap(), Vec::new());
    assert_eq!(
        rig.host.read_capture_entries(1, 1).unwrap(),
        vec![entries[1].clone()]
    );
    let after_host = rig.host.read_captures(1, 2).unwrap();
    assert_eq!(after_host.len(), 1);
    assert_eq!(after_host[0].capture_seq, 3);
    assert_eq!(rig.host.read_capture_entries(3, 5).unwrap(), Vec::new());

    // (4) The version-2 checkpoint, byte for byte from independent parts.
    let text = rig.saved();
    assert!(
        text.starts_with("{\"version\":2,\"incarnation\":1,"),
        "{text}"
    );
    assert!(text.contains(
        "\"host_records\":[{\"capture_seq\":2,\"op\":\"tick\",\"history_start\":4,\"history_end\":5,\"processed_tick\":1,\"events\":["
    ));
    let captured: Vec<&CapturedRecord> = vec![cmd(&entries[0]), cmd(&entries[2])];
    let expected = format!(
        "{{\"version\":2,\"incarnation\":1,\"protocol\":2,\"server_tick\":1,\"history\":{},\"captured\":{},\"host_records\":{},\"capture_acknowledged\":0}}",
        serde_json::to_string(&rig.mirror).unwrap(),
        serde_json::to_string(&captured).unwrap(),
        serde_json::to_string(&vec![hostrec(&entries[1])]).unwrap(),
    );
    assert_eq!(text, expected);

    let loaded = load(text.as_bytes()).expect("version 2 loads");
    assert_eq!(loaded.authority_hash(), rig.host.authority_hash());
    assert_eq!(
        loaded.read_capture_entries(0, 256).unwrap(),
        entries,
        "entries survive the round trip"
    );
    assert_eq!(loaded.last_capture(), 3);
    assert_eq!(loaded.capture_acknowledged(), 0);
    assert_eq!(loaded.server_tick(), 1);

    // (5) Retiring the host record's prefix drops the version back to 1.
    rig.host.acknowledge_captures(2).unwrap();
    assert_eq!(rig.host.capture_acknowledged(), 2);
    assert_eq!(entry_kinds(&rig.host), vec!['C']);
    let text = rig.saved();
    assert!(text.starts_with("{\"version\":1,"), "{text}");
    assert!(!text.contains("host_records"));
    let loaded = load(text.as_bytes()).expect("version 1 loads");
    assert_eq!(loaded.authority_hash(), rig.host.authority_hash());
    assert_eq!(loaded.capture_acknowledged(), 2);
    assert_eq!(loaded.last_capture(), 3);
}

#[test]
fn s1_acknowledging_a_prefix_retires_both_kinds() {
    let (mut rig, _) = s1_run();
    rig.declare(2, 2, strike(), 3);
    assert_eq!(rig.host.last_capture(), 3);

    // Acknowledging only the command keeps the host record and version 2.
    rig.host.acknowledge_captures(1).unwrap();
    assert_eq!(entry_kinds(&rig.host), vec!['H', 'C']);
    assert!(rig.saved().starts_with("{\"version\":2,"));
    // Acknowledging through the host record retires it too.
    rig.host.acknowledge_captures(2).unwrap();
    assert_eq!(entry_kinds(&rig.host), vec!['C']);
    // At or below the watermark is a no-op; past the last is Future.
    assert_eq!(rig.host.acknowledge_captures(1), Ok(()));
    assert_eq!(rig.host.capture_acknowledged(), 2);
    assert_eq!(
        rig.host.acknowledge_captures(4),
        Err(HostError::FutureCaptureCursor {
            requested: 4,
            last: 3
        })
    );
    assert_eq!(rig.host.capture_acknowledged(), 2);
    assert_eq!(rig.host.last_capture(), 3);
}

#[test]
fn s2_read_cursor_checks_run_in_the_read_captures_order() {
    let (mut rig, _) = s1_run();
    rig.declare(2, 2, strike(), 3);
    rig.host.acknowledge_captures(1).unwrap();
    // watermark 1, last 3.
    let bad_limits = [0usize, MAX_CAPTURE_PAGE + 1];
    for limit in bad_limits {
        // InvalidPageLimit wins over a stale cursor and over a future one.
        for after in [0u64, 2, 99] {
            assert_eq!(
                rig.host.read_capture_entries(after, limit),
                Err(HostError::InvalidPageLimit { limit })
            );
            assert_eq!(
                rig.host.read_captures(after, limit),
                Err(HostError::InvalidPageLimit { limit })
            );
        }
    }
    assert_eq!(
        rig.host.read_capture_entries(0, 4),
        Err(HostError::StaleCaptureCursor {
            requested: 0,
            acknowledged: 1
        })
    );
    assert_eq!(
        rig.host.read_captures(0, 4),
        Err(HostError::StaleCaptureCursor {
            requested: 0,
            acknowledged: 1
        })
    );
    assert_eq!(
        rig.host.read_capture_entries(4, 4),
        Err(HostError::FutureCaptureCursor {
            requested: 4,
            last: 3
        })
    );
    assert_eq!(
        rig.host.read_captures(4, 4),
        Err(HostError::FutureCaptureCursor {
            requested: 4,
            last: 3
        })
    );
    // Accepted controls: the page limit at its bound, the watermark and the
    // last sequence as cursors.
    assert_eq!(
        rig.host
            .read_capture_entries(1, MAX_CAPTURE_PAGE)
            .unwrap()
            .len(),
        2
    );
    assert_eq!(rig.host.read_capture_entries(3, 1).unwrap(), Vec::new());
    assert_eq!(rig.host.read_captures(1, 1).unwrap(), Vec::new());
}

#[test]
fn s1_a_tick_that_kills_and_ends_records_more_than_four_events() {
    // A host record has no per-command bound: five envelopes in one record.
    let mut rig = Rig::rt3();
    rig.declare(1, 1, smite(), 2);
    rig.declare(2, 3, smite(), 1);
    rig.m_action(CombatAction::UseAbility {
        actor: a(),
        ability: smite(),
        target: b(),
    });
    rig.m_action(CombatAction::UseAbility {
        actor: c(),
        ability: smite(),
        target: a(),
    });
    assert_eq!(rig.host.tick(), Ok(()));
    let page = rig.m_tick();
    assert_eq!(
        payloads(&page),
        vec![
            resolved(a(), b(), smite(), "critical_success", 4_000_000_000),
            HistoryEvent::Died { entity: b() },
            resolved(c(), a(), smite(), "critical_success", 4_000_000_000),
            HistoryEvent::Died { entity: a() },
            HistoryEvent::EncounterEnded {
                encounter: encounter(),
                round: 0
            },
        ]
    );
    let entries = rig.entries();
    let record = hostrec(&entries[2]);
    assert_eq!(record.capture_seq, 3);
    assert_eq!(record.op, HostOp::Tick);
    assert_eq!((record.history_start, record.history_end), (4, 9));
    assert_eq!(record.processed_tick, 1);
    assert_eq!(record.events, page);
    assert_eq!(rig.host.executions(), 2);
    rig.assert_authority();
    assert!(rig.drain().is_empty());

    let text = rig.saved();
    assert!(text.starts_with("{\"version\":2,"));
    let loaded = load(text.as_bytes()).expect("a five-event host record loads");
    assert_eq!(loaded.authority_hash(), rig.host.authority_hash());
    assert_eq!(loaded.read_capture_entries(0, 256).unwrap(), entries);
}

// ---------------------------------------------------------------------------
// S2: turn-based hosts write version 1.
// ---------------------------------------------------------------------------

/// The version-1 text assembled from the mirror and the captured commands.
fn expected_v1(rig: &Rig, server_tick: u64) -> String {
    let commands = rig
        .host
        .read_captures(rig.host.capture_acknowledged(), 256)
        .unwrap();
    format!(
        "{{\"version\":1,\"incarnation\":1,\"protocol\":2,\"server_tick\":{},\"history\":{},\"captured\":{},\"capture_acknowledged\":{}}}",
        server_tick,
        serde_json::to_string(&rig.mirror).unwrap(),
        serde_json::to_string(&commands).unwrap(),
        rig.host.capture_acknowledged()
    )
}

#[test]
fn s2_turn_based_hosts_write_version_one_byte_for_byte() {
    let mut rig = Rig::tb3();
    rig.end_turn(1, 1);
    rig.m_action(CombatAction::EndTurn { actor: a() });
    for _ in 0..3 {
        assert_eq!(rig.host.tick(), Ok(()));
        let page = rig.m_tick();
        assert!(page.is_empty(), "a turn-based tick journals nothing");
    }
    assert_eq!(rig.host.last_capture(), 1);
    assert_eq!(entry_kinds(&rig.host), vec!['C']);
    assert_eq!(rig.host.server_tick(), 3);
    rig.assert_authority();
    let text = rig.saved();
    assert!(text.starts_with("{\"version\":1,"));
    assert!(!text.contains("host_records"));
    assert_eq!(text, expected_v1(&rig, 3));
    assert!(load(text.as_bytes()).is_ok());
}

#[test]
fn s2_turn_based_hosts_with_a_clock_record_no_host_record() {
    // TB3c: real-time capable but still turn-based; a tick journals nothing.
    let mut rig = Rig::new(&tb3c_json(), V2, false);
    for _ in 0..3 {
        assert_eq!(rig.host.tick(), Ok(()));
        assert!(rig.m_tick().is_empty());
    }
    assert_eq!(rig.host.last_capture(), 0);
    assert!(rig.saved().starts_with("{\"version\":1,"));
    rig.assert_authority();

    // A pending switch to real time is still turn-based until the boundary.
    let mut rig = Rig::new(&tb3c_json(), V2, true);
    assert_eq!(rig.host.tick(), Ok(()));
    assert!(rig.m_tick().is_empty());
    assert_eq!(rig.host.last_capture(), 0);
    assert!(rig.saved().starts_with("{\"version\":1,"));
}

#[test]
fn s2_rt_tick_without_orders_makes_no_record() {
    let mut rig = Rig::rt3();
    assert_eq!(rig.host.tick(), Ok(()));
    let page = rig.m_tick();
    assert!(page.is_empty(), "no orders, no events");
    assert_eq!(rig.host.last_capture(), 0);
    assert_eq!(rig.host.server_tick(), 1);
    assert!(rig.entries().is_empty());
    let text = rig.saved();
    assert!(text.starts_with("{\"version\":1,"));
    assert!(!text.contains("host_records"));
    assert_eq!(text, expected_v1(&rig, 1));
    rig.assert_authority();

    // An order command with no events keeps version 1 until a tick records.
    let mut rig = Rig::rt3();
    rig.declare(1, 1, strike(), 2);
    assert!(rig.saved().starts_with("{\"version\":1,"));
    assert_eq!(rig.host.tick(), Ok(()));
    assert!(rig.saved().starts_with("{\"version\":2,"));
}

// ---------------------------------------------------------------------------
// S3: four events per peer command.
// ---------------------------------------------------------------------------

struct S3 {
    rig: Rig,
    text: String,
    seq3_frame: Vec<DeltaOp>,
}

fn s3_run(protocol: ProtocolSelection) -> S3 {
    let mut rig = Rig::new(&tb3c_json(), protocol, true);
    rig.declare(1, 1, strike(), 2);
    rig.m_action(CombatAction::UseAbility {
        actor: a(),
        ability: strike(),
        target: b(),
    });
    rig.declare(2, 2, whiff(), 3);
    rig.m_action(CombatAction::UseAbility {
        actor: b(),
        ability: whiff(),
        target: c(),
    });
    let mut frames = rig.declare(3, 3, smite(), 1);
    rig.m_action(CombatAction::UseAbility {
        actor: c(),
        ability: smite(),
        target: a(),
    });
    assert_eq!(frames.len(), 1, "one frame for the fourth-event command");
    let seq3_frame = frames.remove(0);
    let text = rig.saved();
    S3 {
        rig,
        text,
        seq3_frame,
    }
}

fn receipt_op(epoch: [u8; 16], seq: u64, tick: u64) -> DeltaOp {
    DeltaOp::Legacy(protocol::DeltaOp::Receipt {
        epoch,
        lane: LANE_COMBAT,
        seq,
        processed_tick: tick,
        status: ReceiptStatus::Applied,
    })
}
fn health_op(id: u64, health: u32, max_health: u32, dead: bool) -> DeltaOp {
    DeltaOp::Legacy(protocol::DeltaOp::Health {
        entity: n(id),
        health,
        max_health,
        dead,
    })
}
fn turn_op(active: Option<u64>, round: u64) -> DeltaOp {
    DeltaOp::Legacy(protocol::DeltaOp::Turn {
        active: active.map(n),
        round,
    })
}

#[test]
fn s3_four_event_command_is_journaled_and_delivered() {
    let S3 {
        rig,
        text,
        seq3_frame,
    } = s3_run(V2);
    let entries = rig.entries();
    assert_eq!(entries.len(), 3);
    let lens: Vec<usize> = entries.iter().map(|e| cmd(e).events.len()).collect();
    assert_eq!(lens, vec![2, 2, 4]);
    let ranges: Vec<(u64, u64)> = entries
        .iter()
        .map(|e| (cmd(e).history_start, cmd(e).history_end))
        .collect();
    assert_eq!(ranges, vec![(4, 6), (6, 8), (8, 12)]);

    let third = cmd(&entries[2]);
    assert_eq!(
        payloads(&third.events),
        vec![
            resolved(c(), a(), smite(), "critical_success", 4_000_000_000),
            HistoryEvent::Died { entity: a() },
            HistoryEvent::ModeChanged {
                mode: EncounterMode::RealTime,
                round: 1
            },
            HistoryEvent::RoundStarted { round: 1 },
        ]
    );
    assert!(serde_json::to_vec(third).unwrap().len() <= MAX_CAPTURE_RECORD_BYTES);

    let epoch = rig.host.epoch(rig.peer).unwrap();
    assert_eq!(
        seq3_frame,
        vec![
            receipt_op(epoch, 3, 0),
            DeltaOp::ActionResolved {
                actor: n(3),
                target: n(1),
                ability: smite(),
                outcome: "critical_success".to_owned(),
                damage: 4_000_000_000,
            },
            DeltaOp::Legacy(protocol::DeltaOp::Died { entity: n(1) }),
            health_op(1, 0, 10, true),
            health_op(3, 3, 3, false),
            turn_op(None, 1),
        ]
    );
    rig.assert_authority();

    assert!(
        text.starts_with("{\"version\":1,"),
        "no host record: version 1"
    );
    assert!(
        load(text.as_bytes()).is_ok(),
        "the four-event command loads"
    );
    assert_eq!(text, expected_v1(&rig, 0));
}

#[test]
fn s3_v1_session_sees_state_ops_only_for_the_four_event_command() {
    let S3 {
        rig, seq3_frame, ..
    } = s3_run(V1);
    let epoch = rig.host.epoch(rig.peer).unwrap();
    assert_eq!(
        seq3_frame,
        vec![
            receipt_op(epoch, 3, 0),
            health_op(1, 0, 10, true),
            health_op(3, 3, 3, false),
            turn_op(None, 1),
        ]
    );
    rig.assert_authority();
}

#[test]
fn s3_command_event_bound_is_four() {
    let S3 { rig, text, .. } = s3_run(V2);
    let saved_hash = rig.host.authority_hash();
    assert_eq!(load_err(text.as_bytes()), None, "control: 4 events load");

    // Records 1 and 2 merged (4 events over (4, 8]), record 3 renumbered:
    // the largest legal record still loads and the other records survive.
    let merged_ok = edited_err(&text, |v| {
        let cap = v["captured"].as_array_mut().unwrap();
        let second = cap.remove(1);
        cap[0]["history_end"] = json!(8);
        let more = second["events"].as_array().unwrap().clone();
        cap[0]["events"].as_array_mut().unwrap().extend(more);
        cap[1]["capture_seq"] = json!(2);
    });
    assert_eq!(merged_ok, None, "a command record of 4 events loads");

    // Record 2's last envelope moved to the front of record 3: 5 events.
    let five = edited_err(&text, |v| {
        let cap = v["captured"].as_array_mut().unwrap();
        let moved = cap[1]["events"].as_array_mut().unwrap().pop().unwrap();
        cap[1]["history_end"] = json!(7);
        cap[2]["history_start"] = json!(7);
        cap[2]["events"].as_array_mut().unwrap().insert(0, moved);
    });
    assert_eq!(five, Some(CheckpointError::Malformed), "5 events refused");

    // Records 2 and 3 merged: 6 events.
    let six = edited_err(&text, |v| {
        let cap = v["captured"].as_array_mut().unwrap();
        let third = cap.remove(2);
        cap[1]["history_end"] = json!(12);
        let more = third["events"].as_array().unwrap().clone();
        cap[1]["events"].as_array_mut().unwrap().extend(more);
    });
    assert_eq!(six, Some(CheckpointError::Malformed), "6 events refused");

    // The refusals left the source host and its journal untouched.
    assert_eq!(rig.host.authority_hash(), saved_hash);
    assert_eq!(rig.saved(), text);
}

// ---------------------------------------------------------------------------
// S4: checkpoint version 2 refusals.
// ---------------------------------------------------------------------------

#[test]
fn s4_checkpoint_v2_single_edit_refusals() {
    let (rig, _) = s1_run();
    let mut rig = rig;
    rig.declare(2, 2, strike(), 3);
    let text = rig.saved();
    assert!(text.starts_with("{\"version\":2,"));
    assert_eq!(load_err(text.as_bytes()), None, "control: S1's text loads");

    // The `"host_records":[…],` member removed (version 2 needs the key).
    let start = text.find("\"host_records\":[").unwrap();
    let end = text.find(",\"capture_acknowledged\"").unwrap();
    let removed = format!("{}{}", text[..start].trim_end_matches(','), &text[end..]);
    assert!(!removed.contains("host_records"));
    assert_eq!(
        load_err(removed.as_bytes()),
        Some(CheckpointError::Malformed)
    );

    assert_malformed("empty events with history_end 4", &text, |v| {
        v["host_records"][0]["events"] = json!([]);
        v["host_records"][0]["history_end"] = json!(4);
    });
    assert_malformed("duplicate capture_seq", &text, |v| {
        v["host_records"][0]["capture_seq"] = json!(3);
    });
    assert_malformed("unknown op", &text, |v| {
        v["host_records"][0]["op"] = json!("bogus");
    });
    assert_malformed("processed_tick past server_tick", &text, |v| {
        v["host_records"][0]["processed_tick"] = json!(999);
    });
    assert_malformed("processed_tick one past server_tick", &text, |v| {
        v["host_records"][0]["processed_tick"] = json!(2);
    });
    assert_malformed("extra key in a host record", &text, |v| {
        v["host_records"][0]["x"] = json!(0);
    });
    assert_malformed("overlap with the previous entry", &text, |v| {
        let record = &mut v["host_records"][0];
        record["history_start"] = json!(3);
        let mut copy = record["events"][0].clone();
        copy["seq"] = json!(4);
        record["events"].as_array_mut().unwrap().insert(0, copy);
    });
    assert_malformed("numbering gap by removing the host record", &text, |v| {
        v["host_records"] = json!([]);
    });
    assert_malformed(
        "history gap 4 -> 5 once the host record is gone",
        &text,
        |v| {
            v["host_records"] = json!([]);
            v["captured"][1]["capture_seq"] = json!(2);
        },
    );
    assert_malformed("range wider than its events", &text, |v| {
        v["host_records"][0]["history_end"] = json!(6);
    });
    assert_malformed("range narrower than its events", &text, |v| {
        let record = &mut v["host_records"][0];
        let copy = record["events"][0].clone();
        record["events"].as_array_mut().unwrap().push(copy);
    });
    assert_malformed(
        "envelope sequence not starting after history_start",
        &text,
        |v| {
            v["host_records"][0]["events"][0]["seq"] = json!(6);
        },
    );
    assert_malformed("missing field in a host record", &text, |v| {
        v["host_records"][0]
            .as_object_mut()
            .unwrap()
            .remove("processed_tick");
    });
    assert_malformed("duplicate host record numbering", &text, |v| {
        let copy = v["host_records"][0].clone();
        v["host_records"].as_array_mut().unwrap().push(copy);
    });
    assert_malformed("captured list descending", &text, |v| {
        v["captured"].as_array_mut().unwrap().swap(0, 1);
    });

    // Version 3 (and 0) are the unsupported versions; version 1 and 2 are not.
    for version in [0u32, 3, 4, 99] {
        assert_eq!(
            edited_err(&text, |v| v["version"] = json!(version)),
            Some(CheckpointError::UnsupportedVersion { version }),
            "version {version}"
        );
    }
}

#[test]
fn s4_version_two_when_host_records_present_and_version_one_without() {
    let (rig, _) = s1_run();
    let v2_text = rig.saved();
    assert!(v2_text.starts_with("{\"version\":2,"));

    // Version 1 with a host_records key is Malformed, with or without the
    // record; the same text as version 2 loads.
    assert_eq!(load_err(v2_text.as_bytes()), None);
    assert_eq!(
        edited_err(&v2_text, |v| v["version"] = json!(1)),
        Some(CheckpointError::Malformed)
    );

    let mut rig = Rig::rt3();
    rig.declare(1, 1, strike(), 2);
    let v1_text = rig.saved();
    assert!(v1_text.starts_with("{\"version\":1,"));
    assert_eq!(load_err(v1_text.as_bytes()), None);
    let with_key = insert_before_ack(&v1_text, "\"host_records\":[],");
    assert_eq!(
        load_err(with_key.as_bytes()),
        Some(CheckpointError::Malformed),
        "\"host_records\":[] added to a version-1 save"
    );
    assert_eq!(
        edited_err(&v1_text, |v| v["host_records"] = json!([])),
        Some(CheckpointError::Malformed)
    );
}

#[test]
fn s4_version_is_checked_before_everything_else() {
    let (rig, _) = s1_run();
    let text = rig.saved();
    assert_eq!(load_err(text.as_bytes()), None);

    // Version 3 beats a missing host_records key.
    let mut value = parse(&text);
    value["version"] = json!(3);
    value.as_object_mut().unwrap().remove("host_records");
    assert_eq!(
        load_err(&emit(&value)),
        Some(CheckpointError::UnsupportedVersion { version: 3 })
    );
    // Version 3 beats a broken host record.
    let mut value = parse(&text);
    value["version"] = json!(3);
    value["host_records"][0]["processed_tick"] = json!(999);
    assert_eq!(
        load_err(&emit(&value)),
        Some(CheckpointError::UnsupportedVersion { version: 3 })
    );
    // The same broken record at version 2 is Malformed (control).
    let mut value = parse(&text);
    value["host_records"][0]["processed_tick"] = json!(999);
    assert_eq!(load_err(&emit(&value)), Some(CheckpointError::Malformed));
}

#[test]
fn s4_every_known_op_loads_and_every_bad_op_is_refused() {
    let (rig, _) = s1_run();
    let text = rig.saved();
    for op in ["tick", "action", "turn_timer"] {
        assert_eq!(
            edited_err(&text, |v| v["host_records"][0]["op"] = json!(op)),
            None,
            "op {op} loads with a full valid remainder"
        );
    }
    for bad in [
        json!("Tick"),
        json!("TICK"),
        json!("turn-timer"),
        json!("turnTimer"),
        json!(""),
        json!(0),
        json!(null),
    ] {
        assert_eq!(
            edited_err(&text, |v| v["host_records"][0]["op"] = bad.clone()),
            Some(CheckpointError::Malformed),
            "op {bad}"
        );
    }
}

#[test]
fn s4_host_record_ranges_and_processed_tick_at_their_limits() {
    let (rig, _) = s1_run();
    let text = rig.saved();
    // server_tick is 1: processed_tick 0 and 1 load, 2 does not.
    for tick in [0u64, 1] {
        assert_eq!(
            edited_err(&text, |v| v["host_records"][0]["processed_tick"] =
                json!(tick)),
            None,
            "processed_tick {tick}"
        );
    }
    assert_eq!(
        edited_err(&text, |v| v["host_records"][0]["processed_tick"] = json!(2)),
        Some(CheckpointError::Malformed)
    );
}

#[test]
fn s4_lists_must_ascend_and_numbering_must_run_from_the_watermark() {
    // [Command, Host, Command, Host] on RT3.
    let mut rig = Rig::rt3();
    rig.declare(1, 1, strike(), 2);
    rig.m_action(CombatAction::UseAbility {
        actor: a(),
        ability: strike(),
        target: b(),
    });
    assert_eq!(rig.host.tick(), Ok(()));
    rig.m_tick();
    rig.declare(2, 2, strike(), 3);
    rig.m_action(CombatAction::UseAbility {
        actor: b(),
        ability: strike(),
        target: c(),
    });
    assert_eq!(rig.host.tick(), Ok(()));
    rig.m_tick();
    assert_eq!(entry_kinds(&rig.host), vec!['C', 'H', 'C', 'H']);
    rig.assert_authority();
    let text = rig.saved();
    assert!(text.starts_with("{\"version\":2,"));
    let loaded = load(text.as_bytes()).expect("two host records load");
    assert_eq!(entry_kinds(&loaded), vec!['C', 'H', 'C', 'H']);
    assert_eq!(loaded.authority_hash(), rig.host.authority_hash());

    // (i) The two host records swapped: Malformed, though the merged
    // numbering is intact.
    assert_malformed("host list swapped", &text, |v| {
        v["host_records"].as_array_mut().unwrap().swap(0, 1);
    });
    assert_malformed("captured list swapped", &text, |v| {
        v["captured"].as_array_mut().unwrap().swap(0, 1);
    });
    // Numbering: duplicate and gap across the lists.
    assert_malformed("duplicate across lists", &text, |v| {
        v["host_records"][1]["capture_seq"] = json!(3);
    });
    assert_malformed("gap across lists", &text, |v| {
        v["host_records"][1]["capture_seq"] = json!(5);
    });

    // A command after a host record starting one past the host's end (gap),
    // beside the host after a command starting one early (overlap).
    assert_malformed("command after host leaves a history gap", &text, |v| {
        // Drop the second host record and push the last command's range one
        // up, keeping the authority's last sequence equal to its end.
        v["host_records"].as_array_mut().unwrap().pop();
        v["captured"][1]["history_start"] = json!(6);
        v["captured"][1]["history_end"] = json!(6);
        v["history"]["next_seq"] = json!(7);
        v["history"]["acknowledged"] = json!(6);
    });
    assert_eq!(
        edited_err(&text, |v| {
            // Control: the same shape with the command in place.
            v["host_records"].as_array_mut().unwrap().pop();
            v["history"]["next_seq"] = json!(6);
            v["history"]["acknowledged"] = json!(5);
        }),
        None,
        "dropping the trailing host record and trimming the history loads"
    );
    assert_malformed("host after command overlaps it", &text, |v| {
        let record = &mut v["host_records"][1];
        let start = record["history_start"].as_u64().unwrap();
        record["history_start"] = json!(start - 1);
        let mut copy = record["events"][0].clone();
        copy["seq"] = json!(start);
        record["events"].as_array_mut().unwrap().insert(0, copy);
    });

    // The watermark: numbering must start at capture_acknowledged + 1.
    rig.host.acknowledge_captures(1).unwrap();
    let text = rig.saved();
    assert!(text.starts_with("{\"version\":2,"));
    assert_eq!(
        load_err(text.as_bytes()),
        None,
        "control: watermark 1 loads"
    );
    assert_malformed("watermark below the first entry", &text, |v| {
        v["capture_acknowledged"] = json!(0);
    });
    assert_malformed("watermark above the first entry", &text, |v| {
        v["capture_acknowledged"] = json!(2);
    });
}

#[test]
fn s4_an_empty_version_two_list_loads_and_saves_as_version_one() {
    let mut rig = Rig::tb3();
    rig.end_turn(1, 1);
    rig.m_action(CombatAction::EndTurn { actor: a() });
    for _ in 0..3 {
        assert_eq!(rig.host.tick(), Ok(()));
    }
    let v1 = rig.saved();
    assert!(v1.starts_with("{\"version\":1,"));
    let v2 = insert_before_ack(
        &v1.replacen("{\"version\":1,", "{\"version\":2,", 1),
        "\"host_records\":[],",
    );
    assert!(v2.starts_with("{\"version\":2,"));
    let loaded = load(v2.as_bytes()).expect("an empty version-2 list loads");
    assert_eq!(loaded.authority_hash(), rig.host.authority_hash());
    assert_eq!(loaded.last_capture(), 1);
    let resaved = String::from_utf8(loaded.save_checkpoint().unwrap()).unwrap();
    assert!(resaved.starts_with("{\"version\":1,"), "{resaved}");
    assert!(!resaved.contains("host_records"));
    assert_eq!(
        load_err(resaved.as_bytes()),
        Some(CheckpointError::EpochReused)
    );
    assert!(load_checkpoint(resaved.as_bytes(), 3, 0).is_ok());

    // Version-1 bytes are unchanged but for the incarnation.
    let expected = v1.replacen("\"incarnation\":1,", "\"incarnation\":2,", 1);
    assert_eq!(resaved, expected);
}

#[test]
fn s4_version_one_with_an_absent_key_and_version_two_control() {
    // Both directions of the key rule on one state, with full remainders.
    let (rig, _) = s1_run();
    let v2 = rig.saved();
    let mut rig = rig;
    rig.host.acknowledge_captures(2).unwrap();
    let v1 = rig.saved();
    assert_eq!(load_err(v2.as_bytes()), None);
    assert_eq!(load_err(v1.as_bytes()), None);
    // Version 2 without the key.
    assert_eq!(
        edited_err(&v2, |v| {
            v.as_object_mut().unwrap().remove("host_records");
        }),
        Some(CheckpointError::Malformed)
    );
    // Version 1 with the key.
    assert_eq!(
        edited_err(&v1, |v| v["host_records"] = json!([])),
        Some(CheckpointError::Malformed)
    );
}

// ---------------------------------------------------------------------------
// S5: history ranges stay contiguous.
// ---------------------------------------------------------------------------

#[test]
fn s5_history_ranges_contiguous() {
    let mut rig = Rig::tb3();
    rig.end_turn(1, 1);
    rig.end_turn(2, 2);
    let entries = rig.entries();
    let first = cmd(&entries[0]);
    let second = cmd(&entries[1]);
    assert_eq!((first.history_start, first.history_end), (4, 5));
    assert_eq!(
        payloads(&first.events),
        vec![HistoryEvent::TurnStarted {
            actor: b(),
            round: 0
        }]
    );
    assert_eq!((second.history_start, second.history_end), (5, 6));
    assert_eq!(
        payloads(&second.events),
        vec![HistoryEvent::TurnStarted {
            actor: c(),
            round: 0
        }]
    );
    let text = rig.saved();
    assert!(text.starts_with("{\"version\":1,"));
    assert_malformed("overlap of command records", &text, |v| {
        let first_event = v["captured"][0]["events"][0].clone();
        let second_event = v["captured"][1]["events"][0].clone();
        v["captured"][1]["history_start"] = json!(4);
        v["captured"][1]["events"] = json!([first_event, second_event]);
    });
    assert_malformed("gap between command records", &text, |v| {
        v["captured"][1]["history_start"] = json!(6);
        v["captured"][1]["events"] = json!([]);
    });
    // A gap before the first retained entry is not a fault: the evidence
    // rule starts at the first retained entry.
    let mut acked = Rig::tb3();
    acked.end_turn(1, 1);
    acked.end_turn(2, 2);
    acked.host.acknowledge_captures(1).unwrap();
    assert_eq!(load_err(acked.saved().as_bytes()), None);
}

// ---------------------------------------------------------------------------
// S6: CaptureFull.
// ---------------------------------------------------------------------------

fn fill_rt(rig: &mut Rig, count: u64) {
    for seq in 1..=count {
        rig.declare(seq, 1, strike(), 2);
        rig.m_action(CombatAction::UseAbility {
            actor: a(),
            ability: strike(),
            target: b(),
        });
    }
}

#[test]
fn s6_capture_full_refuses_an_rt_tick_without_room() {
    let mut rig = Rig::rt3();
    fill_rt(&mut rig, MAX_CAPTURE_RECORDS as u64);
    assert_eq!(rig.host.last_capture(), 4096);
    assert_eq!(rig.host.executions(), 4096);
    assert_eq!(rig.host.server_tick(), 0);
    let hash = rig.host.authority_hash();
    rig.assert_authority();

    // tick() is refused and changes nothing.
    assert_eq!(rig.host.tick(), Err(HostError::CaptureFull));
    assert_eq!(rig.host.server_tick(), 0, "the tick did not run");
    assert_eq!(rig.host.executions(), 4096);
    assert_eq!(rig.host.last_capture(), 4096);
    assert_eq!(rig.host.capture_acknowledged(), 0);
    assert_eq!(rig.host.authority_hash(), hash, "no order executed");
    rig.assert_authority();
    assert!(rig.drain().is_empty());
    // Refused again: nothing consumed by the first refusal.
    assert_eq!(rig.host.tick(), Err(HostError::CaptureFull));

    // Acknowledging the first entry frees the room the tick needs.
    rig.host.acknowledge_captures(1).unwrap();
    assert_eq!(rig.host.tick(), Ok(()));
    let page = rig.m_tick();
    assert_eq!(page.len(), 1);
    assert_eq!(rig.host.last_capture(), 4097);
    let mut entries = Vec::new();
    let mut after = rig.host.capture_acknowledged();
    loop {
        let page = rig
            .host
            .read_capture_entries(after, MAX_CAPTURE_PAGE)
            .unwrap();
        let Some(last) = page.last() else { break };
        after = last.capture_seq();
        entries.extend(page);
    }
    assert_eq!(entries.len(), 4096);
    assert_eq!(entries[0].capture_seq(), 2);
    let tail = hostrec(entries.last().unwrap());
    assert_eq!(tail.capture_seq, 4097);
    assert_eq!(tail.op, HostOp::Tick);
    assert_eq!((tail.history_start, tail.history_end), (4, 5));
    assert_eq!(tail.processed_tick, 1);
    assert_eq!(tail.events, page);
    rig.assert_authority();

    // The same journal, saved and loaded: 4,096 entries (4,095 commands and
    // the host record) is the entry bound.
    let text = rig.saved();
    assert!(text.starts_with("{\"version\":2,"));
    let loaded = load(text.as_bytes()).expect("4,096 entries load");
    assert_eq!(loaded.last_capture(), 4097);
    assert_eq!(loaded.capture_acknowledged(), 1);
    assert_eq!(loaded.authority_hash(), rig.host.authority_hash());

    // One more entry (a copy of the last command under capture_seq 4,098,
    // an empty range at the host record's end) is the only fault.
    let mut value = parse(&text);
    let end = value["host_records"][0]["history_end"].clone();
    let mut copy = value["captured"]
        .as_array()
        .unwrap()
        .last()
        .unwrap()
        .clone();
    copy["capture_seq"] = json!(4098);
    copy["history_start"] = end.clone();
    copy["history_end"] = end;
    copy["events"] = json!([]);
    value["captured"].as_array_mut().unwrap().push(copy);
    assert_eq!(
        load_err(&emit(&value)),
        Some(CheckpointError::Malformed),
        "4,097 entries"
    );

    // Closed wins over a full journal.
    let mut full = Rig::rt3();
    fill_rt(&mut full, MAX_CAPTURE_RECORDS as u64);
    full.host.shutdown();
    assert_eq!(full.host.tick(), Err(HostError::Closed));
}

#[test]
fn s6_a_turn_based_tick_needs_no_room() {
    let mut rig = Rig::tb3();
    for i in 0..MAX_CAPTURE_RECORDS as u64 {
        let actor = 1 + (i % 3);
        rig.end_turn(i + 1, actor);
        rig.m_action(CombatAction::EndTurn {
            actor: [a(), b(), c()][(i % 3) as usize],
        });
    }
    assert_eq!(rig.host.last_capture(), 4096);
    let before = rig.host.last_capture();
    assert_eq!(rig.host.tick(), Ok(()));
    assert!(rig.m_tick().is_empty());
    assert_eq!(rig.host.server_tick(), 1);
    assert_eq!(rig.host.last_capture(), before);
    assert_eq!(rig.host.capture_acknowledged(), 0);
    rig.assert_authority();
    assert!(rig.saved().starts_with("{\"version\":1,"));
}

// ---------------------------------------------------------------------------
// Crafted journals: the entry, record and byte bounds at the limit.
// ---------------------------------------------------------------------------

fn envelope(seq: u64, level: usize) -> HistoryEnvelope {
    // Level 0..=18 adds exactly `level` bytes: the outcome word (+9) and the
    // damage's digit count (1..=10).
    let (big, digits) = if level <= 9 {
        (false, 1 + level)
    } else {
        (true, level - 8)
    };
    let outcome = if big { "critical_success" } else { "success" };
    let damage = 10u64.pow(digits as u32 - 1);
    let text = format!(
        "{{\"tick\":0,\"seq\":{seq},\"payload\":{{\"type\":\"action_resolved\",\"value\":{{\"actor\":{{\"index\":0,\"generation\":1}},\"target\":{{\"index\":1,\"generation\":1}},\"ability\":\"000000000000000000000000JV\",\"outcome\":\"{outcome}\",\"damage\":{damage}}}}}}}"
    );
    serde_json::from_str(&text).expect("a valid envelope")
}

/// Host records covering `(start, start + levels.len()]`, split into records
/// of at most `MAX_HOST_RECORD_EVENTS` events, numbered from `first_seq`.
fn build_records(first_seq: u64, start: u64, levels: &[usize]) -> Vec<HostRecord> {
    let total = levels.len();
    let records = total.div_ceil(MAX_HOST_RECORD_EVENTS).max(1);
    let mut out = Vec::new();
    let mut at = 0usize;
    for index in 0..records {
        let count = total / records + usize::from(index < total % records);
        let begin = start + at as u64;
        let events: Vec<HistoryEnvelope> = (0..count)
            .map(|k| envelope(begin + 1 + k as u64, levels[at + k]))
            .collect();
        out.push(HostRecord {
            capture_seq: first_seq + index as u64,
            op: HostOp::Tick,
            history_start: begin,
            history_end: begin + count as u64,
            processed_tick: 0,
            events,
        });
        at += count;
    }
    out
}

fn json_len<T: serde::Serialize>(value: &T) -> usize {
    serde_json::to_vec(value).expect("encodes").len()
}

fn records_len(records: &[HostRecord]) -> usize {
    records.iter().map(json_len).sum()
}

/// Host records whose compact lengths sum to exactly `target` bytes.
fn records_with_total(target: usize) -> Vec<HostRecord> {
    let mut count = target / 205;
    loop {
        let base = records_len(&build_records(1, 4, &vec![0; count]));
        if base > target {
            count -= (base - target) / 200 + 1;
            continue;
        }
        let slack = target - base;
        if slack > 18 * count {
            count += (slack - 18 * count) / 200 + 1;
            continue;
        }
        let mut remaining = slack;
        let levels: Vec<usize> = (0..count)
            .map(|_| {
                let take = remaining.min(18);
                remaining -= take;
                take
            })
            .collect();
        let records = build_records(1, 4, &levels);
        assert_eq!(records_len(&records), target, "exact total");
        assert!(records
            .iter()
            .all(|r| json_len(r) <= MAX_HOST_RECORD_BYTES
                && r.events.len() <= MAX_HOST_RECORD_EVENTS));
        return records;
    }
}

/// An `RT3` checkpoint (no commands) carrying exactly `records`.
fn crafted(records: &[HostRecord]) -> Vec<u8> {
    let base = Rig::rt3().saved();
    assert!(base.starts_with("{\"version\":1,"));
    let mut value = parse(&base);
    let last = records.last().map_or(4, |r| r.history_end);
    value["version"] = json!(2);
    value["history"]["next_seq"] = json!(last + 1);
    value["history"]["acknowledged"] = json!(last);
    value["host_records"] = serde_json::to_value(records).expect("records");
    emit(&value)
}

#[test]
fn crafted_control_one_record_loads() {
    let records = build_records(1, 4, &[0]);
    let loaded = load(&crafted(&records)).expect("control loads");
    assert_eq!(loaded.last_capture(), 1);
    assert_eq!(
        loaded.read_capture_entries(0, 4).unwrap(),
        vec![CaptureEntry::Host(records[0].clone())]
    );
}

#[test]
fn crafted_record_event_bound_at_the_limit_and_one_past() {
    let at_limit = build_records(1, 4, &vec![0; MAX_HOST_RECORD_EVENTS]);
    assert_eq!(at_limit.len(), 1, "one record of 4,096 events");
    assert_eq!(at_limit[0].events.len(), MAX_HOST_RECORD_EVENTS);
    assert!(json_len(&at_limit[0]) <= MAX_HOST_RECORD_BYTES);
    let loaded = load(&crafted(&at_limit)).expect("4,096 events load");
    assert_eq!(loaded.last_capture(), 1);

    // One past, as one oversized record (bypassing the splitter).
    let mut over = at_limit[0].clone();
    over.events.push(envelope(over.history_end + 1, 0));
    over.history_end += 1;
    assert_eq!(over.events.len(), MAX_HOST_RECORD_EVENTS + 1);
    assert!(
        json_len(&over) <= MAX_HOST_RECORD_BYTES,
        "only the count is wrong"
    );
    assert_eq!(
        load_err(&crafted(&[over])),
        Some(CheckpointError::Malformed)
    );
}

#[test]
fn crafted_entry_count_at_the_limit_and_one_past() {
    let at_limit: Vec<HostRecord> = (0..MAX_CAPTURE_RECORDS as u64)
        .flat_map(|i| build_records(1 + i, 4 + i, &[0]))
        .collect();
    assert_eq!(at_limit.len(), 4096);
    let loaded = load(&crafted(&at_limit)).expect("4,096 entries load");
    assert_eq!(loaded.last_capture(), 4096);

    let past: Vec<HostRecord> = (0..MAX_CAPTURE_RECORDS as u64 + 1)
        .flat_map(|i| build_records(1 + i, 4 + i, &[0]))
        .collect();
    assert_eq!(
        load_err(&crafted(&past)),
        Some(CheckpointError::Malformed),
        "4,097 entries"
    );
}

#[test]
fn crafted_journal_bytes_at_the_limit_and_one_past() {
    let at_limit = records_with_total(MAX_CAPTURE_BYTES);
    assert!(at_limit.len() <= MAX_CAPTURE_RECORDS);
    let loaded = load(&crafted(&at_limit)).expect("exactly 8 MiB of entries loads");
    assert_eq!(loaded.last_capture(), at_limit.len() as u64);

    let past = records_with_total(MAX_CAPTURE_BYTES + 1);
    assert_eq!(
        load_err(&crafted(&past)),
        Some(CheckpointError::Malformed),
        "8 MiB + 1 byte of entries"
    );
}

#[test]
fn crafted_capture_full_on_the_byte_reservation() {
    // capture_bytes + MAX_HOST_RECORD_BYTES > MAX_CAPTURE_BYTES is full.
    let limit = MAX_CAPTURE_BYTES - MAX_HOST_RECORD_BYTES;
    assert_eq!(limit, 7_274_496);

    let at = records_with_total(limit);
    let mut host = load(&crafted(&at)).expect("loads");
    let last = host.last_capture();
    assert_eq!(host.tick(), Ok(()), "exactly enough room");
    assert_eq!(host.server_tick(), 1);
    assert_eq!(
        host.last_capture(),
        last,
        "a zero-event tick makes no record"
    );

    let over = records_with_total(limit + 1);
    let mut host = load(&crafted(&over)).expect("loads");
    let last = host.last_capture();
    let hash = host.authority_hash();
    assert_eq!(host.tick(), Err(HostError::CaptureFull));
    assert_eq!(host.server_tick(), 0);
    assert_eq!(host.last_capture(), last);
    assert_eq!(host.capture_acknowledged(), 0);
    assert_eq!(host.authority_hash(), hash);
    // Acknowledging the first record frees room; the retry runs.
    host.acknowledge_captures(1).unwrap();
    assert_eq!(host.tick(), Ok(()));
    assert_eq!(host.server_tick(), 1);
}

#[test]
fn crafted_capture_full_on_the_entry_reservation() {
    let build = |count: u64| -> Vec<HostRecord> {
        (0..count)
            .flat_map(|i| build_records(1 + i, 4 + i, &[0]))
            .collect()
    };
    // 4,095 entries leave room for one more record.
    let mut host = load(&crafted(&build(4095))).expect("loads");
    assert_eq!(host.tick(), Ok(()));
    assert_eq!(host.server_tick(), 1);
    // 4,096 entries leave none.
    let mut host = load(&crafted(&build(4096))).expect("loads");
    let hash = host.authority_hash();
    assert_eq!(host.tick(), Err(HostError::CaptureFull));
    assert_eq!(host.server_tick(), 0);
    assert_eq!(host.last_capture(), 4096);
    assert_eq!(host.authority_hash(), hash);
}

// ---------------------------------------------------------------------------
// S8 (v1/v2 sessions see no host operation).
// ---------------------------------------------------------------------------

#[test]
fn s8_v1_and_snapshot_sessions_get_no_frame_from_a_host_operation() {
    let mut host = Host::from_history(
        history_of(&rt3_json()),
        HostConfig {
            protocol: V1,
            incarnation: 1,
        },
        0,
    )
    .unwrap();
    let plain = host.bind_peer(all_actors(), full_grants(), 0).unwrap();
    let snap = host
        .bind_snapshot_peer(all_actors(), full_grants(), 0)
        .unwrap();
    let plain_before = host.take_delivery_entries(plain).unwrap();
    let snap_before = host.take_delivery_entries(snap).unwrap();

    // A peer command sets the order; its delivery is the command path's.
    let epoch = host.epoch(plain).unwrap();
    let bytes = declare_bytes(1, epoch, 1, 0, 1, strike(), 2);
    host.ingest(plain, &bytes, 100).unwrap();
    host.pump(100).unwrap();
    let plain_mid = host.take_delivery_entries(plain).unwrap();
    let snap_mid = host.take_delivery_entries(snap).unwrap();
    assert!(
        plain_mid.len() > plain_before.len(),
        "the command is delivered"
    );
    assert!(snap_mid.len() >= snap_before.len());
    let snap_state = host.transfer_state(snap).unwrap();

    // The tick records its events and delivers nothing to either session.
    assert_eq!(host.tick(), Ok(()));
    assert_eq!(host.last_capture(), 2, "the tick recorded its events");
    assert_eq!(host.take_delivery_entries(plain).unwrap(), plain_mid);
    assert_eq!(host.take_delivery_entries(snap).unwrap(), snap_mid);
    assert_eq!(host.transfer_state(snap).unwrap(), snap_state);
    assert_eq!(host.executions(), 1);
}

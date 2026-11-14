//! T141s acceptance suite (`tasks/T141.md`, tests S1..S6): host records and
//! checkpoint version 2, public API only. (S7 and S8 are private unit tests
//! in `src/host.rs` and `src/checkpoint.rs`.)
//!
//! Oracles are independent of the host: a test-owned mirror `HistoryWorld`
//! loaded from the same JSON and driven with the same sim calls (acknowledged
//! after each), hand-written expected records, and hand-edited checkpoint
//! text. The fixtures are `tests/fixtures/three_combatants.json` as saved-game
//! JSON, edited in memory:
//!
//! - `TB3`: the file as is (A, B, C act in keys 1, 2, 3; A first).
//! - `TB3c`: `TB3` at history version 2 with a round clock, so the encounter
//!   is real-time capable while turn-based.
//! - `RT3`: `TB3c` in real-time mode, no active head, every timeline key 0.

use crpg_core::{EntityId, Ulid};
use crpg_net::codec_v2;
use crpg_net::protocol::{self, IntentBody, IntentFrame, NetId, ReceiptStatus, LANE_COMBAT};
use crpg_net::protocol_v2::{self, DeltaOp};
use crpg_server::capture::{
    CaptureEntry, CapturedRecord, HostOp, HostRecord, MAX_CAPTURE_PAGE, MAX_CAPTURE_RECORDS,
    MAX_CAPTURE_RECORD_BYTES, MAX_NEW_EVENTS_PER_COMMAND, MAX_NEW_EVENTS_PER_COMMAND_RT,
};
use crpg_server::checkpoint::{
    load_checkpoint, CheckpointError, CHECKPOINT_VERSION, CHECKPOINT_VERSION_HOST_RECORDS,
};
use crpg_server::host::{
    AdmissionResult, ControlGrant, DisclosureGrants, EntityDisclosure, Host, HostConfig, HostError,
    IngestDisposition, PeerHandle, ProtocolSelection,
};
use crpg_sim::{
    history_hash, CombatAction, EncounterMode, HistoryEnvelope, HistoryEvent, HistoryWorld,
};
use serde::Serialize;
use serde_json::{json, Value};

const FIXTURE: &str = include_str!("fixtures/three_combatants.json");

// ---------------------------------------------------------------------------
// Identities and fixtures.
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

fn tb3_value() -> Value {
    serde_json::from_str(FIXTURE).expect("fixture parses")
}

fn tb3c_value() -> Value {
    let mut value = tb3_value();
    value["version"] = json!(2);
    value["world"]["combat"]["clock"] = json!({"round_ticks": 4, "elapsed": 0});
    value
}

fn rt3_value() -> Value {
    let mut value = tb3c_value();
    value["world"]["combat"]["mode"] = json!("real_time");
    value["world"]["combat"]["active"] = Value::Null;
    for entry in value["world"]["timeline"].as_array_mut().expect("timeline") {
        entry[0] = json!(0);
    }
    value
}

/// The authority from `value`, its four start envelopes acknowledged.
fn history_of(value: &Value) -> HistoryWorld {
    let mut history: HistoryWorld =
        serde_json::from_value(value.clone()).expect("fixture loads through validated serde");
    history
        .acknowledge(history.last_sequence())
        .expect("acknowledges its own range");
    history
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

fn config() -> HostConfig {
    HostConfig {
        protocol: ProtocolSelection::V2,
        incarnation: 1,
    }
}

// ---------------------------------------------------------------------------
// Rig: one host, its mirror authority and a bound peer `V`.
// ---------------------------------------------------------------------------

struct Rig {
    host: Host,
    oracle: HistoryWorld,
    now: u64,
}

impl Rig {
    fn new(value: &Value) -> Self {
        Self::from_histories(history_of(value), history_of(value))
    }

    fn from_histories(host_history: HistoryWorld, oracle: HistoryWorld) -> Self {
        let host = Host::from_history(host_history, config(), 0).expect("wraps");
        Self {
            host,
            oracle,
            now: 0,
        }
    }

    /// `V`: controls A, B and C, every disclosure flag, first state drained.
    fn bind_v(&mut self) -> PeerHandle {
        let peer = self
            .host
            .bind_peer(all_actors(), full_grants(), self.now)
            .expect("binds");
        self.drain(peer);
        peer
    }

    /// Takes, decodes and acknowledges every pending frame of `peer`.
    fn drain(&mut self, peer: PeerHandle) -> Vec<protocol_v2::DeltaFrame> {
        let frames: Vec<protocol_v2::DeltaFrame> = self
            .host
            .take_delivery(peer)
            .expect("bound")
            .iter()
            .map(|bytes| codec_v2::decode_delta(bytes).expect("v2 delivery decodes"))
            .collect();
        if let Some(last) = frames.last() {
            self.host
                .acknowledge_delivery(peer, last.event_seq)
                .expect("acks");
        }
        frames
    }

    /// One intent through `ingest` then `pump`, +100 ms.
    fn send(
        &mut self,
        peer: PeerHandle,
        seq: u64,
        actor: u64,
        body: IntentBody,
    ) -> AdmissionResult {
        let frame = IntentFrame {
            epoch: self.host.epoch(peer).expect("bound"),
            lane: LANE_COMBAT,
            seq,
            observed_tick: self.host.server_tick(),
            actor: n(actor),
            body,
        };
        let bytes = codec_v2::encode_intent(&frame).expect("encodes");
        self.now += 100;
        assert_eq!(
            self.host.ingest(peer, &bytes, self.now),
            Ok(IngestDisposition::Staged)
        );
        let summary = self.host.pump(self.now).expect("pump");
        assert_eq!(summary.results.len(), 1, "{summary:?}");
        summary.results[0].clone()
    }

    fn declare(
        &mut self,
        peer: PeerHandle,
        seq: u64,
        actor: u64,
        ability: Ulid,
        target: u64,
    ) -> AdmissionResult {
        self.send(
            peer,
            seq,
            actor,
            IntentBody::DeclareAction {
                ability,
                target: n(target),
            },
        )
    }

    fn end_turn(&mut self, peer: PeerHandle, seq: u64, actor: u64) -> AdmissionResult {
        self.send(peer, seq, actor, IntentBody::EndTurn)
    }

    /// Mirrors one accepted action; returns the envelopes it journaled.
    fn mirror(&mut self, action: CombatAction) -> Vec<HistoryEnvelope> {
        let before = self.oracle.last_sequence();
        self.oracle
            .perform_action(&action)
            .expect("the mirror accepts the same action");
        self.mirror_page(before)
    }

    /// Mirrors one tick; returns the envelopes it journaled.
    fn mirror_tick(&mut self) -> Vec<HistoryEnvelope> {
        let before = self.oracle.last_sequence();
        self.oracle.tick().expect("the mirror ticks");
        self.mirror_page(before)
    }

    fn mirror_page(&mut self, before: u64) -> Vec<HistoryEnvelope> {
        let page = if self.oracle.last_sequence() > before {
            self.oracle.read_after(before, 256).expect("readable")
        } else {
            Vec::new()
        };
        self.oracle
            .acknowledge(self.oracle.last_sequence())
            .expect("acks");
        page
    }

    fn assert_authority(&self) {
        assert_eq!(self.host.authority_hash(), history_hash(&self.oracle));
    }

    fn entries(&self) -> Vec<CaptureEntry> {
        let mut all = Vec::new();
        let mut after = self.host.capture_acknowledged();
        loop {
            let page = self
                .host
                .read_capture_entries(after, MAX_CAPTURE_PAGE)
                .expect("readable");
            if page.is_empty() {
                return all;
            }
            after = page.last().expect("non-empty").capture_seq();
            all.extend(page);
        }
    }

    fn save(&self) -> String {
        String::from_utf8(self.host.save_checkpoint().expect("saves")).expect("utf8")
    }
}

fn command(entry: &CaptureEntry) -> &CapturedRecord {
    match entry {
        CaptureEntry::Command(record) => record,
        CaptureEntry::Host(record) => panic!("expected a command entry, got {record:?}"),
    }
}

fn host_record(entry: &CaptureEntry) -> &HostRecord {
    match entry {
        CaptureEntry::Host(record) => record,
        CaptureEntry::Command(record) => panic!("expected a host entry, got {record:?}"),
    }
}

fn load(text: &str) -> Result<Host, CheckpointError> {
    load_checkpoint(text.as_bytes(), 2, 0)
}

fn load_err(text: &str) -> Option<CheckpointError> {
    load(text).err()
}

fn receipt(epoch: [u8; 16], seq: u64, tick: u64, status: ReceiptStatus) -> DeltaOp {
    DeltaOp::Legacy(protocol::DeltaOp::Receipt {
        epoch,
        lane: LANE_COMBAT,
        seq,
        processed_tick: tick,
        status,
    })
}

// ---------------------------------------------------------------------------
// Checkpoint text surgery (compact JSON, no whitespace).
// ---------------------------------------------------------------------------

/// `[start, end)` of the JSON array that is the value of `"key":`.
fn array_span(text: &str, key: &str) -> (usize, usize) {
    let needle = format!("\"{key}\":[");
    let start = text.find(&needle).expect("key present") + needle.len() - 1;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (offset, byte) in text.as_bytes()[start..].iter().enumerate() {
        if in_string {
            if escaped {
                escaped = false;
            } else if *byte == b'\\' {
                escaped = true;
            } else if *byte == b'"' {
                in_string = false;
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'[' | b'{' => depth += 1,
            b']' | b'}' => {
                depth -= 1;
                if depth == 0 {
                    return (start, start + offset + 1);
                }
            }
            _ => {}
        }
    }
    panic!("unterminated array");
}

fn captured_of(text: &str) -> Vec<CapturedRecord> {
    let (start, end) = array_span(text, "captured");
    serde_json::from_str(&text[start..end]).expect("captured list parses")
}

fn host_records_of(text: &str) -> Vec<HostRecord> {
    let (start, end) = array_span(text, "host_records");
    serde_json::from_str(&text[start..end]).expect("host record list parses")
}

fn with_array<T: Serialize>(text: &str, key: &str, items: &[T]) -> String {
    let (start, end) = array_span(text, key);
    format!(
        "{}{}{}",
        &text[..start],
        serde_json::to_string(items).expect("serializes"),
        &text[end..]
    )
}

fn with_captured(text: &str, items: &[CapturedRecord]) -> String {
    with_array(text, "captured", items)
}

fn with_host_records(text: &str, items: &[HostRecord]) -> String {
    with_array(text, "host_records", items)
}

/// The text without its `"host_records":[..],` member.
fn without_host_records(text: &str) -> String {
    let (start, end) = array_span(text, "host_records");
    let key_start = start - "\"host_records\":".len();
    assert_eq!(&text[end..=end], ",");
    format!("{}{}", &text[..key_start], &text[end + 1..])
}

/// `RT3` after `V` orders A→B, a tick, `V` orders B→C, a tick: entries
/// `[Command, Host, Command, Host]`.
fn two_host_records() -> Rig {
    let mut rig = Rig::new(&rt3_value());
    let v = rig.bind_v();
    assert_eq!(
        rig.declare(v, 1, 1, strike(), 2).status,
        ReceiptStatus::Applied
    );
    rig.drain(v);
    rig.mirror(CombatAction::UseAbility {
        actor: a(),
        ability: strike(),
        target: b(),
    });
    rig.host.tick().expect("ticks");
    rig.mirror_tick();
    assert_eq!(
        rig.declare(v, 2, 2, strike(), 3).status,
        ReceiptStatus::Applied
    );
    rig.drain(v);
    rig.mirror(CombatAction::UseAbility {
        actor: b(),
        ability: strike(),
        target: c(),
    });
    rig.host.tick().expect("ticks");
    rig.mirror_tick();
    rig.assert_authority();
    rig
}

// ---------------------------------------------------------------------------
// S1.
// ---------------------------------------------------------------------------

/// A real-time tick's events are recorded, version 2 is written and read,
/// and acknowledging every host record returns the save to version 1.
#[test]
fn rt_tick_is_recorded() {
    let mut rig = Rig::new(&rt3_value());
    let v = rig.bind_v();
    let epoch = rig.host.epoch(v).expect("bound");

    // (1) A's standing order: a zero-event command.
    let result = rig.declare(v, 1, 1, strike(), 2);
    assert_eq!(result.status, ReceiptStatus::Applied);
    assert_eq!(
        rig.mirror(CombatAction::UseAbility {
            actor: a(),
            ability: strike(),
            target: b()
        }),
        Vec::new()
    );
    rig.drain(v);
    let entries = rig.entries();
    assert_eq!(entries.len(), 1);
    let first = command(&entries[0]);
    assert_eq!(first.capture_seq, 1);
    assert_eq!((first.history_start, first.history_end), (4, 4));
    assert!(first.events.is_empty());
    assert_eq!(rig.host.executions(), 1);

    // (2) The tick resolves it: one envelope, recorded as a host record.
    rig.host.tick().expect("ticks");
    let mirrored = rig.mirror_tick();
    assert_eq!(mirrored.len(), 1);
    assert_eq!(mirrored[0].seq, 5);
    assert_eq!(
        mirrored[0].payload,
        HistoryEvent::ActionResolved {
            actor: a(),
            target: b(),
            ability: strike(),
            outcome: "success".to_owned(),
            damage: 3,
        }
    );
    let entries = rig.entries();
    assert_eq!(entries.len(), 2);
    assert_eq!(
        host_record(&entries[1]),
        &HostRecord {
            capture_seq: 2,
            op: HostOp::Tick,
            history_start: 4,
            history_end: 5,
            processed_tick: 1,
            events: mirrored,
        }
    );
    assert_eq!(rig.host.last_capture(), 2);
    assert_eq!(rig.host.executions(), 1, "a tick is not a command");
    assert_eq!(rig.host.server_tick(), 1);
    assert!(
        rig.drain(v).is_empty(),
        "a host operation sends v2 sessions nothing"
    );
    rig.assert_authority();

    // (3) B's order: a command entry after the host record.
    let result = rig.declare(v, 2, 2, strike(), 3);
    assert_eq!(result.status, ReceiptStatus::Applied);
    rig.mirror(CombatAction::UseAbility {
        actor: b(),
        ability: strike(),
        target: c(),
    });
    rig.drain(v);
    let entries = rig.entries();
    assert_eq!(entries.len(), 3);
    let third = command(&entries[2]);
    assert_eq!(third.capture_seq, 3);
    assert_eq!((third.history_start, third.history_end), (5, 5));
    assert_eq!(third.epoch, epoch);
    assert!(matches!(
        entries.as_slice(),
        [
            CaptureEntry::Command(_),
            CaptureEntry::Host(_),
            CaptureEntry::Command(_)
        ]
    ));
    assert_eq!(
        rig.host.read_capture_entries(0, 256).expect("reads"),
        entries
    );
    let commands = rig.host.read_captures(0, 256).expect("reads");
    assert_eq!(commands.len(), 2);
    assert_eq!(commands[0], *command(&entries[0]));
    assert_eq!(commands[1], *command(&entries[2]));
    let page = rig.host.read_captures(0, 2).expect("reads");
    assert_eq!(page.len(), 1, "the page of two entries holds one command");
    assert_eq!(page[0], *command(&entries[0]));
    assert_eq!(
        rig.host.read_capture_entries(0, 2).expect("reads"),
        entries[..2].to_vec()
    );
    assert_eq!(
        rig.host.read_captures(1, 1).expect("reads"),
        Vec::<CapturedRecord>::new(),
        "the one-entry page after entry 1 is the host record"
    );

    // (4) The checkpoint is version 2 and round-trips.
    let text = rig.save();
    assert!(
        text.starts_with("{\"version\":2,\"incarnation\":1,"),
        "{text}"
    );
    assert!(
        text.contains(
            "\"host_records\":[{\"capture_seq\":2,\"op\":\"tick\",\"history_start\":4,\"history_end\":5,\"processed_tick\":1,\"events\":["
        ),
        "{text}"
    );
    assert_eq!(CHECKPOINT_VERSION, 1);
    assert_eq!(CHECKPOINT_VERSION_HOST_RECORDS, 2);
    let restored = load(&text).expect("version 2 loads");
    assert_eq!(restored.authority_hash(), rig.host.authority_hash());
    assert_eq!(restored.last_capture(), 3);
    assert_eq!(restored.capture_acknowledged(), 0);
    assert_eq!(
        restored.read_capture_entries(0, 256).expect("reads"),
        entries
    );

    // (5) Acknowledging the host record's entry returns the save to v1.
    rig.host.acknowledge_captures(2).expect("acks");
    assert_eq!(rig.host.last_capture(), 3);
    let text = rig.save();
    assert!(text.starts_with("{\"version\":1,"), "{text}");
    assert!(!text.contains("host_records"), "{text}");
    let restored = load(&text).expect("version 1 loads");
    assert_eq!(restored.authority_hash(), rig.host.authority_hash());
    assert_eq!(restored.capture_acknowledged(), 2);
    assert_eq!(
        restored.read_capture_entries(2, 256).expect("reads").len(),
        1
    );
}

/// A host operation delivers nothing to any v1/v2 session, snapshot sessions
/// included, and adds no transfer frame.
#[test]
fn host_operation_sends_v2_sessions_nothing() {
    let mut rig = Rig::new(&rt3_value());
    let v = rig.bind_v();
    let snapshot = rig
        .host
        .bind_snapshot_peer(all_actors(), full_grants(), rig.now)
        .expect("binds");
    let before_entries = rig.host.take_delivery_entries(snapshot).expect("bound");
    assert!(
        !before_entries.is_empty(),
        "the bind staged its first state"
    );
    let last = before_entries.last().expect("non-empty").ordinal;
    rig.host.acknowledge_delivery(snapshot, last).expect("acks");
    assert!(rig.declare(v, 1, 1, strike(), 2).status == ReceiptStatus::Applied);
    rig.drain(v);
    rig.mirror(CombatAction::UseAbility {
        actor: a(),
        ability: strike(),
        target: b(),
    });
    // The peer command reached the snapshot session; drain it first.
    let after_command = rig.host.take_delivery_entries(snapshot).expect("bound");
    let last = after_command
        .last()
        .expect("the command reached it")
        .ordinal;
    rig.host.acknowledge_delivery(snapshot, last).expect("acks");

    rig.host.tick().expect("ticks");
    rig.mirror_tick();
    assert_eq!(rig.host.last_capture(), 2);
    assert_eq!(rig.host.take_delivery_entries(snapshot), Ok(Vec::new()));
    assert_eq!(rig.host.take_delivery(v), Ok(Vec::new()));
    rig.assert_authority();
}

// ---------------------------------------------------------------------------
// S2.
// ---------------------------------------------------------------------------

/// Turn-based ticks and zero-event real-time ticks make no record, and the
/// save stays version 1.
#[test]
fn turn_based_hosts_write_v1() {
    let mut rig = Rig::new(&tb3_value());
    let v = rig.bind_v();
    assert_eq!(rig.end_turn(v, 1, 1).status, ReceiptStatus::Applied);
    rig.mirror(CombatAction::EndTurn { actor: a() });
    rig.drain(v);
    for _ in 0..3 {
        rig.host.tick().expect("ticks");
        rig.mirror_tick();
    }
    assert_eq!(rig.host.last_capture(), 1);
    assert_eq!(rig.host.server_tick(), 3);
    assert_eq!(rig.host.executions(), 1);
    rig.assert_authority();
    let text = rig.save();
    assert!(text.starts_with("{\"version\":1,"), "{text}");
    assert!(!text.contains("host_records"), "{text}");
    assert!(load(&text).is_ok());
    assert!(rig.drain(v).is_empty());

    // Real-time encounter with no order: the tick journals nothing.
    let mut rt = Rig::new(&rt3_value());
    let v = rt.bind_v();
    rt.host.tick().expect("ticks");
    assert!(rt.mirror_tick().is_empty());
    assert_eq!(rt.host.last_capture(), 0);
    assert_eq!(rt.host.server_tick(), 1);
    assert_eq!(rt.entries(), Vec::new());
    rt.assert_authority();
    assert!(rt.save().starts_with("{\"version\":1,"));
    assert!(rt.drain(v).is_empty());
}

// ---------------------------------------------------------------------------
// S3.
// ---------------------------------------------------------------------------

/// A killing command that ends the round with a pending real-time switch
/// journals four events and is accepted; the loader's bound is four.
#[test]
fn four_event_command() {
    assert_eq!(MAX_NEW_EVENTS_PER_COMMAND, 3);
    assert_eq!(MAX_NEW_EVENTS_PER_COMMAND_RT, 4);

    let value = tb3c_value();
    let mut host_history = history_of(&value);
    host_history
        .request_mode(EncounterMode::RealTime)
        .expect("requests");
    host_history
        .acknowledge(host_history.last_sequence())
        .expect("acks");
    let mut oracle = history_of(&value);
    oracle
        .request_mode(EncounterMode::RealTime)
        .expect("requests");
    oracle.acknowledge(oracle.last_sequence()).expect("acks");
    let mut rig = Rig::from_histories(host_history, oracle);
    let v = rig.bind_v();
    let epoch = rig.host.epoch(v).expect("bound");

    assert_eq!(
        rig.declare(v, 1, 1, strike(), 2).status,
        ReceiptStatus::Applied
    );
    rig.mirror(CombatAction::UseAbility {
        actor: a(),
        ability: strike(),
        target: b(),
    });
    rig.drain(v);
    assert_eq!(
        rig.declare(v, 2, 2, whiff(), 3).status,
        ReceiptStatus::Applied
    );
    rig.mirror(CombatAction::UseAbility {
        actor: b(),
        ability: whiff(),
        target: c(),
    });
    rig.drain(v);
    let result = rig.declare(v, 3, 3, smite(), 1);
    assert_eq!(result.status, ReceiptStatus::Applied, "{result:?}");
    let mirrored = rig.mirror(CombatAction::UseAbility {
        actor: c(),
        ability: smite(),
        target: a(),
    });
    rig.assert_authority();
    assert_eq!(mirrored.len(), 4);

    let entries = rig.entries();
    assert_eq!(entries.len(), 3);
    let counts: Vec<usize> = entries.iter().map(|e| command(e).events.len()).collect();
    assert_eq!(counts, vec![2, 2, 4]);
    let third = command(&entries[2]);
    assert_eq!((third.history_start, third.history_end), (8, 12));
    let payloads: Vec<HistoryEvent> = third.events.iter().map(|e| e.payload.clone()).collect();
    assert_eq!(
        payloads,
        vec![
            HistoryEvent::ActionResolved {
                actor: c(),
                target: a(),
                ability: smite(),
                outcome: "critical_success".to_owned(),
                damage: 4_000_000_000,
            },
            HistoryEvent::Died { entity: a() },
            HistoryEvent::ModeChanged {
                mode: EncounterMode::RealTime,
                round: 1,
            },
            HistoryEvent::RoundStarted { round: 1 },
        ]
    );
    assert_eq!(third.events, mirrored);
    let encoded = serde_json::to_vec(third).expect("encodes");
    assert!(
        encoded.len() <= MAX_CAPTURE_RECORD_BYTES,
        "{} bytes",
        encoded.len()
    );

    // The originator's v2 frame: ModeChanged and RoundStarted have no v2 form.
    let frames = rig.drain(v);
    assert_eq!(frames.len(), 1);
    assert_eq!(
        frames[0].ops,
        vec![
            receipt(epoch, 3, 0, ReceiptStatus::Applied),
            DeltaOp::ActionResolved {
                actor: n(3),
                target: n(1),
                ability: smite(),
                outcome: "critical_success".to_owned(),
                damage: 4_000_000_000,
            },
            DeltaOp::Legacy(protocol::DeltaOp::Died { entity: n(1) }),
            DeltaOp::Legacy(protocol::DeltaOp::Health {
                entity: n(1),
                health: 0,
                max_health: 10,
                dead: true,
            }),
            DeltaOp::Legacy(protocol::DeltaOp::Health {
                entity: n(3),
                health: 3,
                max_health: 3,
                dead: false,
            }),
            DeltaOp::Legacy(protocol::DeltaOp::Turn {
                active: None,
                round: 1,
            }),
        ]
    );

    // The version-1 checkpoint loads; surgery on its command records.
    let text = rig.save();
    assert!(text.starts_with("{\"version\":1,"));
    assert!(load(&text).is_ok(), "the four-event record loads");
    let records = captured_of(&text);
    assert_eq!(records.len(), 3);
    assert_eq!(
        records
            .iter()
            .map(|r| (r.history_start, r.history_end, r.events.len()))
            .collect::<Vec<_>>(),
        vec![(4, 6, 2), (6, 8, 2), (8, 12, 4)]
    );

    // Records 1 and 2 merged: four events, the bound.
    let mut merged = records[0].clone();
    merged.history_end = records[1].history_end;
    merged.events.extend(records[1].events.clone());
    let mut third_renumbered = records[2].clone();
    third_renumbered.capture_seq = 2;
    let ok = with_captured(&text, &[merged, third_renumbered]);
    assert!(load(&ok).is_ok(), "four events is the bound");

    // Record 2's last envelope moved to the front of record 3: five events.
    let mut second = records[1].clone();
    let moved = second.events.pop().expect("two events");
    second.history_end = 7;
    let mut third = records[2].clone();
    third.history_start = 7;
    third.events.insert(0, moved);
    assert_eq!(third.events.len(), 5);
    let five = with_captured(&text, &[records[0].clone(), second.clone(), third]);
    assert_eq!(load_err(&five), Some(CheckpointError::Malformed));
    // Control: the same move without the extra event is the unedited text.
    assert!(load(&with_captured(&text, &records)).is_ok());

    // Records 2 and 3 merged: six events.
    let mut merged = records[1].clone();
    merged.history_end = records[2].history_end;
    merged.events.extend(records[2].events.clone());
    assert_eq!(merged.events.len(), 6);
    let six = with_captured(&text, &[records[0].clone(), merged]);
    assert_eq!(load_err(&six), Some(CheckpointError::Malformed));
}

// ---------------------------------------------------------------------------
// S4.
// ---------------------------------------------------------------------------

/// S1 step 4's checkpoint text.
fn s1_text() -> String {
    let mut rig = Rig::new(&rt3_value());
    let v = rig.bind_v();
    assert_eq!(
        rig.declare(v, 1, 1, strike(), 2).status,
        ReceiptStatus::Applied
    );
    rig.drain(v);
    rig.host.tick().expect("ticks");
    assert_eq!(
        rig.declare(v, 2, 2, strike(), 3).status,
        ReceiptStatus::Applied
    );
    rig.drain(v);
    rig.save()
}

#[test]
fn checkpoint_v2_refusals() {
    let text = s1_text();
    assert!(text.starts_with("{\"version\":2,"));
    let host_records = host_records_of(&text);
    assert_eq!(host_records.len(), 1);
    let record = host_records[0].clone();
    let commands = captured_of(&text);
    assert_eq!(commands.len(), 2);

    // Control: the unedited text loads.
    let loaded = load(&text).expect("control loads");
    assert_eq!(loaded.last_capture(), 3);

    let malformed = Some(CheckpointError::Malformed);
    let with_record = |edit: &dyn Fn(&mut HostRecord)| {
        let mut edited = record.clone();
        edit(&mut edited);
        with_host_records(&text, &[edited])
    };

    // The `"host_records":[..],` member removed (version 2 requires one).
    let missing = without_host_records(&text);
    assert!(!missing.contains("host_records"));
    assert_eq!(load_err(&missing), malformed);

    // `"host_records":[],` added to a version-1 save.
    let mut tb = Rig::new(&tb3_value());
    let v = tb.bind_v();
    assert_eq!(tb.end_turn(v, 1, 1).status, ReceiptStatus::Applied);
    let v1 = tb.save();
    assert!(v1.starts_with("{\"version\":1,"));
    assert!(load(&v1).is_ok(), "control: the version-1 save loads");
    let added = v1.replacen(
        "\"capture_acknowledged\"",
        "\"host_records\":[],\"capture_acknowledged\"",
        1,
    );
    assert_ne!(added, v1);
    assert_eq!(load_err(&added), malformed);

    // The host record with no events and history_end 4.
    assert_eq!(
        load_err(&with_record(&|r| {
            r.events.clear();
            r.history_end = 4;
        })),
        malformed
    );
    // Its capture_seq 3: a duplicate of the command entry 3.
    assert_eq!(load_err(&with_record(&|r| r.capture_seq = 3)), malformed);
    // Its capture_seq 4: a gap and a number past the last.
    assert_eq!(load_err(&with_record(&|r| r.capture_seq = 4)), malformed);
    // Its op unknown.
    let bogus = text.replacen("\"op\":\"tick\"", "\"op\":\"bogus\"", 1);
    assert_ne!(bogus, text);
    assert_eq!(load_err(&bogus), malformed);
    // Control: the other two ops load.
    for op in ["action", "turn_timer"] {
        let renamed = text.replacen("\"op\":\"tick\"", &format!("\"op\":\"{op}\""), 1);
        assert!(load(&renamed).is_ok(), "{op}");
    }
    // Its processed_tick 999 (past the server tick).
    assert_eq!(
        load_err(&with_record(&|r| r.processed_tick = 999)),
        malformed
    );
    // Control: processed_tick equal to the server tick (1) loads.
    assert!(load(&with_record(&|r| r.processed_tick = 1)).is_ok());
    // An extra key in it.
    let extra = text.replacen(
        "{\"capture_seq\":2,\"op\":\"tick\"",
        "{\"capture_seq\":2,\"x\":0,\"op\":\"tick\"",
        1,
    );
    assert_ne!(extra, text);
    assert_eq!(load_err(&extra), malformed);
    // Its history_start 3 with a copy of its envelope renumbered 4 prepended
    // (overlap with entry 1).
    assert_eq!(
        load_err(&with_record(&|r| {
            let mut copy = r.events[0].clone();
            copy.seq = 4;
            r.events.insert(0, copy);
            r.history_start = 3;
        })),
        malformed
    );
    // The host record removed: a numbering gap.
    assert_eq!(load_err(&with_host_records(&text, &[])), malformed);
    // The host record list emptied and entry 3 renumbered 2: a history gap
    // (4 -> 5).
    let mut renumbered = commands.clone();
    renumbered[1].capture_seq = 2;
    let gap = with_captured(&with_host_records(&text, &[]), &renumbered);
    assert_eq!(load_err(&gap), malformed);
    // Control: the same edit with entry 3 also starting at 4 is contiguous.
    let mut adjacent = renumbered.clone();
    adjacent[1].history_start = 4;
    adjacent[1].history_end = 4;
    assert!(load(&with_captured(&with_host_records(&text, &[]), &adjacent)).is_ok());
    // Version 3.
    let v3 = text.replacen("{\"version\":2,", "{\"version\":3,", 1);
    assert_ne!(v3, text);
    assert_eq!(
        load_err(&v3),
        Some(CheckpointError::UnsupportedVersion { version: 3 })
    );
    // Control: version 1 over the same body (it has a host_records key) is
    // the unknown-key refusal, not an unsupported version.
    let v1_body = text.replacen("{\"version\":2,", "{\"version\":1,", 1);
    assert_eq!(load_err(&v1_body), malformed);
}

/// Each list must ascend, even when the merged numbering is intact.
#[test]
fn checkpoint_lists_must_ascend() {
    let rig = two_host_records();
    let entries = rig.entries();
    assert!(matches!(
        entries.as_slice(),
        [
            CaptureEntry::Command(_),
            CaptureEntry::Host(_),
            CaptureEntry::Command(_),
            CaptureEntry::Host(_)
        ]
    ));
    let text = rig.save();
    let restored = load(&text).expect("the saved text loads");
    assert_eq!(
        restored.read_capture_entries(0, 256).expect("reads"),
        entries
    );
    let records = host_records_of(&text);
    assert_eq!(
        records.iter().map(|r| r.capture_seq).collect::<Vec<_>>(),
        vec![2, 4]
    );
    let swapped = with_host_records(&text, &[records[1].clone(), records[0].clone()]);
    assert_ne!(swapped, text);
    assert_eq!(load_err(&swapped), Some(CheckpointError::Malformed));
    // The command list swapped the same way.
    let commands = captured_of(&text);
    let swapped = with_captured(&text, &[commands[1].clone(), commands[0].clone()]);
    assert_eq!(load_err(&swapped), Some(CheckpointError::Malformed));
}

/// An empty version-2 list loads and is written again as version 1.
#[test]
fn empty_host_record_list_loads() {
    let mut rig = Rig::new(&tb3_value());
    let v = rig.bind_v();
    assert_eq!(rig.end_turn(v, 1, 1).status, ReceiptStatus::Applied);
    for _ in 0..3 {
        rig.host.tick().expect("ticks");
    }
    let v1 = rig.save();
    assert!(v1.starts_with("{\"version\":1,"));
    let v2 = v1
        .replacen("{\"version\":1,", "{\"version\":2,", 1)
        .replacen(
            "\"capture_acknowledged\"",
            "\"host_records\":[],\"capture_acknowledged\"",
            1,
        );
    let restored = load(&v2).expect("an empty version-2 list loads");
    assert_eq!(restored.authority_hash(), rig.host.authority_hash());
    assert_eq!(restored.last_capture(), 1);
    let saved = String::from_utf8(restored.save_checkpoint().expect("saves")).expect("utf8");
    assert!(saved.starts_with("{\"version\":1,"), "{saved}");
    assert!(!saved.contains("host_records"));
    assert_eq!(
        saved,
        v1.replacen("\"incarnation\":1", "\"incarnation\":2", 1)
    );
}

// ---------------------------------------------------------------------------
// S5.
// ---------------------------------------------------------------------------

#[test]
fn history_ranges_contiguous() {
    let mut rig = Rig::new(&tb3_value());
    let v = rig.bind_v();
    assert_eq!(rig.end_turn(v, 1, 1).status, ReceiptStatus::Applied);
    rig.drain(v);
    assert_eq!(rig.end_turn(v, 2, 2).status, ReceiptStatus::Applied);
    let text = rig.save();
    let records = captured_of(&text);
    assert_eq!(records.len(), 2);
    assert_eq!((records[0].history_start, records[0].history_end), (4, 5));
    assert_eq!((records[1].history_start, records[1].history_end), (5, 6));
    assert_eq!(
        records[0].events[0].payload,
        HistoryEvent::TurnStarted {
            actor: b(),
            round: 0
        }
    );
    assert_eq!(
        records[1].events[0].payload,
        HistoryEvent::TurnStarted {
            actor: c(),
            round: 0
        }
    );
    assert!(load(&text).is_ok(), "control: adjacent ranges load");

    // Record 2 with history_start 4 and both envelopes: an overlap.
    let mut overlap = records[1].clone();
    overlap.history_start = 4;
    overlap.events = vec![records[0].events[0].clone(), records[1].events[0].clone()];
    let edited = with_captured(&text, &[records[0].clone(), overlap]);
    assert_eq!(load_err(&edited), Some(CheckpointError::Malformed));

    // A gap: record 2 starting past record 1's end.
    let mut gap = records[1].clone();
    gap.history_start = 6;
    gap.history_end = 6;
    gap.events.clear();
    let edited = with_captured(&text, &[records[0].clone(), gap]);
    assert_eq!(load_err(&edited), Some(CheckpointError::Malformed));
}

// ---------------------------------------------------------------------------
// S6.
// ---------------------------------------------------------------------------

/// Fills the journal with `MAX_CAPTURE_RECORDS` command entries.
fn fill_journal(rig: &mut Rig, v: PeerHandle, mut command: impl FnMut(&mut Rig, u64)) {
    for seq in 1..=MAX_CAPTURE_RECORDS as u64 {
        command(rig, seq);
        rig.drain(v);
    }
    assert_eq!(rig.host.last_capture(), MAX_CAPTURE_RECORDS as u64);
}

fn fill_rt(rig: &mut Rig, v: PeerHandle) {
    fill_journal(rig, v, |rig, seq| {
        assert_eq!(
            rig.declare(v, seq, 1, strike(), 2).status,
            ReceiptStatus::Applied,
            "seq {seq}"
        );
        rig.mirror(CombatAction::UseAbility {
            actor: a(),
            ability: strike(),
            target: b(),
        });
    });
}

#[test]
fn capture_full() {
    assert_eq!(
        HostError::CaptureFull.to_string(),
        "CaptureFull at host/admission"
    );
    assert!(std::error::Error::source(&HostError::CaptureFull).is_none());

    let mut rig = Rig::new(&rt3_value());
    let v = rig.bind_v();
    fill_rt(&mut rig, v);
    assert_eq!(rig.host.executions(), 4096);

    // The journal is full: a real-time tick is refused and changes nothing.
    let hash = rig.host.authority_hash();
    assert_eq!(rig.host.tick(), Err(HostError::CaptureFull));
    assert_eq!(rig.host.server_tick(), 0);
    assert_eq!(rig.host.authority_hash(), hash);
    assert_eq!(rig.host.last_capture(), 4096);
    assert_eq!(rig.host.capture_acknowledged(), 0);
    assert_eq!(rig.host.executions(), 4096, "no order executed");
    assert_eq!(rig.host.take_delivery(v), Ok(Vec::new()));
    rig.assert_authority();

    // Acknowledging one entry makes room: the tick records as entry 4097.
    rig.host.acknowledge_captures(1).expect("acks");
    rig.host.tick().expect("ticks");
    rig.mirror_tick();
    assert_eq!(rig.host.last_capture(), 4097);
    let last = rig
        .host
        .read_capture_entries(4096, 1)
        .expect("reads")
        .pop()
        .expect("one entry");
    let record = host_record(&last);
    assert_eq!(record.capture_seq, 4097);
    assert_eq!(record.op, HostOp::Tick);
    assert_eq!((record.history_start, record.history_end), (4, 5));
    rig.assert_authority();
    assert_eq!(rig.host.executions(), 4096);

    // Turn-based: a full journal still ticks (no reservation).
    let mut tb = Rig::new(&tb3_value());
    let v = tb.bind_v();
    fill_journal(&mut tb, v, |rig, seq| {
        let actor = (seq - 1) % 3;
        assert_eq!(
            rig.end_turn(v, seq, actor + 1).status,
            ReceiptStatus::Applied,
            "seq {seq}"
        );
        rig.mirror(CombatAction::EndTurn {
            actor: eid(actor as u32),
        });
    });
    tb.host.tick().expect("a turn-based tick needs no room");
    tb.mirror_tick();
    assert_eq!(tb.host.last_capture(), 4096);
    assert_eq!(tb.host.server_tick(), 1);
    tb.assert_authority();
}

/// The entry count bound holds over both kinds: 4,096 entries (4,095
/// commands and a host record) load; one more does not.
#[test]
fn journal_entry_count_bound() {
    let mut rig = Rig::new(&rt3_value());
    let v = rig.bind_v();
    fill_rt(&mut rig, v);
    rig.host.acknowledge_captures(1).expect("acks");
    rig.host.tick().expect("ticks");
    rig.mirror_tick();
    assert_eq!(rig.host.last_capture(), 4097);

    let text = rig.save();
    assert!(text.starts_with("{\"version\":2,"));
    let restored = load(&text).expect("4,096 entries load");
    assert_eq!(restored.last_capture(), 4097);
    assert_eq!(restored.capture_acknowledged(), 1);
    assert_eq!(restored.authority_hash(), rig.host.authority_hash());

    let host_end = host_records_of(&text)[0].history_end;
    let mut commands = captured_of(&text);
    assert_eq!(commands.len(), 4095);
    let mut extra = commands.last().expect("a command").clone();
    extra.capture_seq = 4098;
    extra.history_start = host_end;
    extra.history_end = host_end;
    commands.push(extra);
    assert_eq!(commands.len(), 4096);
    let over = with_captured(&text, &commands);
    assert_eq!(load_err(&over), Some(CheckpointError::Malformed));
}

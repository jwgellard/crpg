//! T039 acceptance suite (`tasks/T039.md` §7): public API only.
//!
//! Gate 10 (spec §15.4 step 10) is file-backed host save/load continuation
//! equivalence, per build and per target, in one process. Oracles are
//! independent of `crpg_server::save`:
//!
//! - **Uninterrupted run**, same process: the reference for every
//!   equivalence assertion. It rebinds a fresh session at each restart point
//!   so both runs continue on the D10 new-session path with `seq` from 1.
//! - **Mirror authority**: a test-owned `HistoryWorld` loaded from the same
//!   fixture, driven through public `perform_action`/`acknowledge`/`tick`
//!   with every command the host is expected to accept, on the host's
//!   schedule. `history_hash` of the mirror must equal `Host::authority_hash`.
//! - **Hand-built header bytes** for format and rejection cases, written to
//!   disk with `crpg_persist::save_file`, never with `save.rs`.
//!
//! The schedule: step `k` (0-based) takes `active = 1 + last_capture % 3`
//! and `next = active % 3 + 1`; `k % 10 == 9` sends `EndTurn` for `next`
//! (refused: not that actor's turn), other even steps declare `whiff` on
//! `next`, other odd steps end the active turn. Every step then runs one
//! trusted tick, drains and acknowledges delivery, and every hundredth step
//! acknowledges captures through `last_capture - 8`. Over 1000 steps that is
//! 900 accepted commands, 100 rejections and 1000 ticks; nobody dies because
//! `whiff` always fails for 0 damage.
//!
//! Tests write only under `CARGO_TARGET_TMPDIR/crpg-server/host_save/`, one
//! directory per test, removed and recreated first.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use crpg_core::{EntityId, Ulid};
use crpg_net::codec;
use crpg_net::codec_v2;
use crpg_net::protocol::{
    self, IntentBody, IntentFrame, NetId, ReceiptStatus, RejectionCode, LANE_COMBAT,
};
use crpg_net::protocol_v2::DeltaOp;
use crpg_persist::{
    encode_envelope, load_file, save_file, EnvelopeError, FileError, FileOp, PayloadKind,
    MAX_ENVELOPE_BYTES, MAX_PAYLOAD_BYTES, TEMP_SUFFIX,
};
use crpg_server::capture::CapturedRecord;
use crpg_server::checkpoint::CheckpointError;
use crpg_server::host::{
    ControlGrant, DisclosureGrants, EntityDisclosure, Host, HostConfig, IngestDisposition,
    PeerHandle, ProtocolSelection,
};
use crpg_server::save::{
    decode_host_save, encode_host_save, load_host_file, save_host_file, LoadedSave, SaveError,
    SaveIdentity, ENGINE_VERSION, HOST_SAVE_FIXED_BYTES, HOST_SAVE_KIND, HOST_SAVE_VERSION,
    MAX_HOST_SAVE_HEADER_BYTES, MAX_VERSION_TEXT_BYTES,
};
use crpg_sim::{history_hash, CombatAction, HistoryWorld};
use serde_json::json;

const FIXTURE: &str = include_str!("fixtures/three_combatants.json");

const V1: ProtocolSelection = ProtocolSelection::V1;
const V2: ProtocolSelection = ProtocolSelection::V2;

/// Runs one named case body under both selected versions, as
/// `<case>::v1` and `<case>::v2`.
macro_rules! both_versions {
    ($name:ident) => {
        mod $name {
            #[test]
            fn v1() {
                super::$name(super::V1);
            }
            #[test]
            fn v2() {
                super::$name(super::V2);
            }
        }
    };
}

// ---------------------------------------------------------------------------
// Fixture identities and operator facts.
// ---------------------------------------------------------------------------

fn eid(index: u32) -> EntityId {
    serde_json::from_value(json!({"index": index, "generation": 1})).expect("valid entity id")
}

/// The entity behind replica id 1/2/3 (A/B/C, indices 0/1/2).
fn entity(replica: u64) -> EntityId {
    eid(u32::try_from(replica - 1).expect("small replica id"))
}

fn whiff() -> Ulid {
    Ulid::from_u128(605)
}

fn n(raw: u64) -> NetId {
    NetId::new(raw).expect("nonzero")
}

/// The fixture authority after the embedding acknowledged its start
/// envelopes (the pre-wrap choreography of `host_capture.rs`).
fn fixture() -> HistoryWorld {
    let mut history: HistoryWorld =
        serde_json::from_str(FIXTURE).expect("fixture loads through validated serde");
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
        entities: vec![full(eid(0)), full(eid(1)), full(eid(2))],
        abilities: vec![
            Ulid::from_u128(603),
            Ulid::from_u128(604),
            Ulid::from_u128(605),
        ],
        encounters: vec![Ulid::from_u128(601)],
        turn_round: true,
        encounter_end: true,
        turn_state: true,
    }
}

fn all_actors() -> ControlGrant {
    ControlGrant {
        actors: vec![eid(0), eid(1), eid(2)],
    }
}

/// The suite's trusted campaign identity.
fn identity() -> SaveIdentity {
    SaveIdentity {
        campaign_id: Ulid::from_u128(700),
        campaign_version: "1.0.0".to_owned(),
    }
}

fn identity_with(id: u128, version: &str) -> SaveIdentity {
    SaveIdentity {
        campaign_id: Ulid::from_u128(id),
        campaign_version: version.to_owned(),
    }
}

fn epoch_of(incarnation: u64, handle: u64) -> [u8; 16] {
    let mut epoch = [0u8; 16];
    epoch[..8].copy_from_slice(&incarnation.to_le_bytes());
    epoch[8..].copy_from_slice(&handle.to_le_bytes());
    epoch
}

// ---------------------------------------------------------------------------
// Test directories.
// ---------------------------------------------------------------------------

fn tag(protocol: ProtocolSelection) -> &'static str {
    match protocol {
        ProtocolSelection::V1 => "v1",
        ProtocolSelection::V2 => "v2",
    }
}

/// `CARGO_TARGET_TMPDIR/crpg-server/host_save/<case>_<tag>`, emptied first.
fn case_dir(case: &str, tag: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("crpg-server")
        .join("host_save")
        .join(format!("{case}_{tag}"));
    match fs::remove_dir_all(&dir) {
        Ok(()) => {}
        Err(e) if e.kind() == ErrorKind::NotFound => {}
        Err(e) => panic!("cannot clear {}: {e}", dir.display()),
    }
    fs::create_dir_all(&dir).expect("creates the case directory");
    dir
}

/// Sorted file names in `dir`.
fn entries(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .expect("readable directory")
        .map(|entry| {
            entry
                .expect("readable entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    names
}

fn assert_no_temp(dir: &Path) {
    let temps: Vec<String> = entries(dir)
        .into_iter()
        .filter(|name| name.ends_with(TEMP_SUFFIX))
        .collect();
    assert!(temps.is_empty(), "temporary files remain: {temps:?}");
}

// ---------------------------------------------------------------------------
// Decoded delivery.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
struct Frame {
    event_seq: u64,
    server_tick: u64,
    ops: Vec<DeltaOp>,
}

fn decode_frame(protocol: ProtocolSelection, bytes: &[u8]) -> Frame {
    match protocol {
        ProtocolSelection::V1 => {
            let frame = codec::decode_delta(bytes).expect("v1 delivery decodes");
            Frame {
                event_seq: frame.event_seq,
                server_tick: frame.server_tick,
                ops: frame.ops.into_iter().map(DeltaOp::Legacy).collect(),
            }
        }
        ProtocolSelection::V2 => {
            let frame = codec_v2::decode_delta(bytes).expect("v2 delivery decodes");
            Frame {
                event_seq: frame.event_seq,
                server_tick: frame.server_tick,
                ops: frame.ops,
            }
        }
    }
}

/// Replaces receipt epochs (session identity) with zeros.
fn without_receipt_epochs(ops: &[DeltaOp]) -> Vec<DeltaOp> {
    ops.iter()
        .map(|op| match op {
            DeltaOp::Legacy(protocol::DeltaOp::Receipt {
                lane,
                seq,
                processed_tick,
                status,
                ..
            }) => DeltaOp::Legacy(protocol::DeltaOp::Receipt {
                epoch: [0; 16],
                lane: *lane,
                seq: *seq,
                processed_tick: *processed_tick,
                status: *status,
            }),
            other => other.clone(),
        })
        .collect()
}

/// Strips session identity from a record for cross-run comparison.
fn without_epochs(mut record: CapturedRecord) -> CapturedRecord {
    record.epoch = [0; 16];
    for view in &mut record.views {
        view.epoch = [0; 16];
    }
    record
}

// ---------------------------------------------------------------------------
// The schedule.
// ---------------------------------------------------------------------------

/// One planned schedule step.
struct Planned {
    bytes: Vec<u8>,
    expect: ReceiptStatus,
    action: Option<CombatAction>,
}

/// What one step produced.
struct StepOut {
    action: Option<CombatAction>,
    frames: Vec<Frame>,
}

/// One host driven through the public `ingest → pump` boundary by one peer.
struct Run {
    host: Host,
    protocol: ProtocolSelection,
    peer: PeerHandle,
    seq: u64,
    now: u64,
}

fn bind(host: &mut Host, now: u64) -> PeerHandle {
    host.bind_peer(all_actors(), full_grants(), now)
        .expect("binds")
}

impl Run {
    /// A host over the fixture under incarnation 1, with one bound peer.
    fn new(protocol: ProtocolSelection) -> Self {
        let host = Host::from_history(
            fixture(),
            HostConfig {
                protocol,
                incarnation: 1,
            },
            0,
        )
        .expect("wraps an acknowledged authority");
        Self::from_host(host, protocol, 0)
    }

    /// Binds a fresh peer on `host` (identical operator facts every time).
    fn from_host(mut host: Host, protocol: ProtocolSelection, now: u64) -> Self {
        let peer = bind(&mut host, now);
        Self {
            host,
            protocol,
            peer,
            seq: 1,
            now,
        }
    }

    /// The D10 new-session path: unbind, then bind a fresh peer.
    fn rebind(&mut self) {
        self.host.unbind_peer(self.peer);
        self.peer = bind(&mut self.host, self.now);
        self.seq = 1;
    }

    /// Step `k`'s intent bytes, expected status and expected sim action.
    fn plan(&self, k: u64) -> Planned {
        let active = 1 + self.host.last_capture() % 3;
        let next = active % 3 + 1;
        let (actor, body, expect, action) = if k % 10 == 9 {
            (
                next,
                IntentBody::EndTurn,
                ReceiptStatus::Rejected(RejectionCode::IllegalAction),
                None,
            )
        } else if k.is_multiple_of(2) {
            (
                active,
                IntentBody::DeclareAction {
                    ability: whiff(),
                    target: n(next),
                },
                ReceiptStatus::Applied,
                Some(CombatAction::UseAbility {
                    actor: entity(active),
                    ability: whiff(),
                    target: entity(next),
                }),
            )
        } else {
            (
                active,
                IntentBody::EndTurn,
                ReceiptStatus::Applied,
                Some(CombatAction::EndTurn {
                    actor: entity(active),
                }),
            )
        };
        let frame = IntentFrame {
            epoch: self.host.epoch(self.peer).expect("bound"),
            lane: LANE_COMBAT,
            seq: self.seq,
            observed_tick: self.host.server_tick(),
            actor: n(actor),
            body,
        };
        let bytes = match self.protocol {
            ProtocolSelection::V1 => codec::encode_intent(&frame),
            ProtocolSelection::V2 => codec_v2::encode_intent(&frame),
        }
        .expect("encodes");
        Planned {
            bytes,
            expect,
            action,
        }
    }

    /// Ingests step `k`'s command without pumping it.
    fn stage(&mut self, k: u64) -> Planned {
        let planned = self.plan(k);
        self.seq += 1;
        self.now += 100;
        assert_eq!(
            self.host.ingest(self.peer, &planned.bytes, self.now),
            Ok(IngestDisposition::Staged),
            "step {k}"
        );
        planned
    }

    /// Pumps exactly one staged command and checks its status.
    fn pump_one(&mut self, k: u64, expect: ReceiptStatus) {
        let summary = self.host.pump(self.now).expect("pump");
        assert_eq!(summary.results.len(), 1, "step {k}: {summary:?}");
        assert_eq!(summary.results[0].status, expect, "step {k}");
    }

    /// One full schedule step.
    fn step(&mut self, k: u64) -> StepOut {
        let planned = self.stage(k);
        self.pump_one(k, planned.expect);
        self.host.tick().expect("trusted tick");
        let frames = self.drain();
        if k % 100 == 99 {
            self.host
                .acknowledge_captures(self.host.last_capture() - 8)
                .expect("consumer acks");
        }
        StepOut {
            action: planned.action,
            frames,
        }
    }

    fn steps(&mut self, range: std::ops::Range<u64>) {
        for k in range {
            self.step(k);
        }
    }

    /// Takes, decodes and acknowledges every pending frame.
    fn drain(&mut self) -> Vec<Frame> {
        let frames: Vec<Frame> = self
            .host
            .take_delivery(self.peer)
            .expect("bound")
            .iter()
            .map(|bytes| decode_frame(self.protocol, bytes))
            .collect();
        if let Some(last) = frames.last() {
            self.host
                .acknowledge_delivery(self.peer, last.event_seq)
                .expect("acks");
        }
        frames
    }

    fn records(&self) -> Vec<CapturedRecord> {
        self.host
            .read_captures(self.host.capture_acknowledged(), 256)
            .expect("readable")
    }
}

/// The independent authority oracle.
struct Mirror(HistoryWorld);

impl Mirror {
    fn new() -> Self {
        Self(fixture())
    }

    fn apply(&mut self, action: Option<CombatAction>) {
        if let Some(action) = action {
            self.0
                .perform_action(&action)
                .expect("the mirror accepts the same action");
            self.0
                .acknowledge(self.0.last_sequence())
                .expect("acknowledges");
        }
        self.0.tick().expect("mirror tick");
    }
}

/// Complete-state-unchanged evidence.
#[derive(Debug, PartialEq, Eq)]
struct Snapshot {
    hash: [u8; 32],
    executions: u64,
    last_capture: u64,
    capture_acknowledged: u64,
}

fn snapshot(host: &Host) -> Snapshot {
    Snapshot {
        hash: host.authority_hash(),
        executions: host.executions(),
        last_capture: host.last_capture(),
        capture_acknowledged: host.capture_acknowledged(),
    }
}

/// Runs both hosts through `range` in lockstep, driving the mirror from the
/// uninterrupted run, and asserts per-step equivalence.
fn lockstep(
    uninterrupted: &mut Run,
    interrupted: &mut Run,
    mirror: &mut Mirror,
    range: std::ops::Range<u64>,
) {
    for k in range {
        let left = uninterrupted.step(k);
        let right = interrupted.step(k);
        assert_eq!(left.action, right.action, "step {k}");
        mirror.apply(left.action);
        assert_eq!(
            uninterrupted.host.authority_hash(),
            interrupted.host.authority_hash(),
            "step {k}"
        );
        assert_eq!(
            uninterrupted.host.last_capture(),
            interrupted.host.last_capture(),
            "step {k}"
        );
        assert_eq!(
            uninterrupted.host.capture_acknowledged(),
            interrupted.host.capture_acknowledged(),
            "step {k}"
        );
        assert_eq!(left.frames.len(), right.frames.len(), "step {k}");
        for (l, r) in left.frames.iter().zip(&right.frames) {
            assert_eq!(l.event_seq, r.event_seq, "step {k}");
            assert_eq!(l.server_tick, r.server_tick, "step {k}");
            assert_eq!(
                without_receipt_epochs(&l.ops),
                without_receipt_epochs(&r.ops),
                "step {k}"
            );
        }
    }
}

/// The restart procedure: save to `path`, record the pre-save state, drop
/// the host, load under `incarnation`, check the loaded state, and bind a
/// fresh peer with identical operator facts.
fn restart(run: Run, path: &Path, incarnation: u64) -> Run {
    save_host_file(path, &run.host, &identity()).expect("quiescent save");
    let hash = run.host.authority_hash();
    let last_capture = run.host.last_capture();
    let capture_acknowledged = run.host.capture_acknowledged();
    let records = run.records();
    let Run {
        host,
        protocol,
        now,
        ..
    } = run;
    drop(host);

    let LoadedSave {
        host,
        engine_version,
    } = load_host_file(path, &identity(), incarnation, now).expect("loads");
    assert_eq!(engine_version, ENGINE_VERSION);
    assert_eq!(host.incarnation(), incarnation);
    assert_eq!(host.protocol(), protocol);
    assert_eq!(host.authority_hash(), hash);
    assert_eq!(host.last_capture(), last_capture);
    assert_eq!(host.capture_acknowledged(), capture_acknowledged);
    assert_eq!(
        host.read_captures(capture_acknowledged, 256)
            .expect("readable"),
        records,
        "retained records, epochs included, are history"
    );
    assert_eq!(host.executions(), 0, "per host lifetime");
    let run = Run::from_host(host, protocol, now);
    assert_eq!(
        run.host.epoch(run.peer).expect("bound"),
        epoch_of(incarnation, 1)
    );
    run
}

/// Final-state equivalence shared by the gate-10 cases.
fn assert_final(uninterrupted: &Run, interrupted: &Run, mirror: &Mirror) {
    assert_eq!(
        uninterrupted.host.authority_hash(),
        interrupted.host.authority_hash()
    );
    assert_eq!(interrupted.host.authority_hash(), history_hash(&mirror.0));
    for run in [uninterrupted, interrupted] {
        assert_eq!(run.host.server_tick(), 1000);
        assert_eq!(run.host.last_capture(), 900);
        assert_eq!(run.host.capture_acknowledged(), 892);
    }
    let left: Vec<CapturedRecord> = uninterrupted
        .records()
        .into_iter()
        .map(without_epochs)
        .collect();
    let right: Vec<CapturedRecord> = interrupted
        .records()
        .into_iter()
        .map(without_epochs)
        .collect();
    assert_eq!(left.len(), 8);
    assert_eq!(left, right);
    assert_eq!(uninterrupted.host.executions(), 900);
}

// ---------------------------------------------------------------------------
// Hand-built payloads.
// ---------------------------------------------------------------------------

fn hex(text: &str) -> Vec<u8> {
    let digits: Vec<u8> = text.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    digits
        .chunks(2)
        .map(|pair| {
            u8::from_str_radix(std::str::from_utf8(pair).expect("ascii"), 16).expect("hex digit")
        })
        .collect()
}

/// A hand-built header with explicit length bytes.
fn raw_header(version: u32, id: u128, cv_len: u8, cv: &[u8], ev_len: u8, ev: &[u8]) -> Vec<u8> {
    let mut out = version.to_le_bytes().to_vec();
    out.extend_from_slice(&id.to_be_bytes());
    out.push(cv_len);
    out.extend_from_slice(cv);
    out.push(ev_len);
    out.extend_from_slice(ev);
    out
}

/// A hand-built header whose length bytes match its texts.
fn header(version: u32, id: u128, cv: &[u8], ev: &[u8]) -> Vec<u8> {
    raw_header(
        version,
        id,
        u8::try_from(cv.len()).expect("short text"),
        cv,
        u8::try_from(ev.len()).expect("short text"),
        ev,
    )
}

/// The suite's valid header: id 700, campaign `1.0.0`, this engine.
fn valid_header() -> Vec<u8> {
    header(1, 700, b"1.0.0", ENGINE_VERSION.as_bytes())
}

fn concat(head: &[u8], tail: &[u8]) -> Vec<u8> {
    let mut out = head.to_vec();
    out.extend_from_slice(tail);
    out
}

/// A test-owned reading of the version alphabet.
fn version_text_ok(bytes: &[u8]) -> bool {
    (1..=64).contains(&bytes.len())
        && bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'+' | b'-'))
}

/// A quiescent V2 host after 20 schedule steps, its checkpoint and hash.
fn quiescent() -> (Host, Vec<u8>, [u8; 32]) {
    let mut run = Run::new(V2);
    run.steps(0..20);
    let checkpoint = run.host.save_checkpoint().expect("quiescent");
    let hash = run.host.authority_hash();
    (run.host, checkpoint, hash)
}

/// Writes `payload` under `HOST_SAVE_KIND` with the persist store, then
/// expects `load_host_file` and `decode_host_save` to refuse it with
/// `expected`, leaving the file bytes unchanged.
fn refuse(dir: &Path, name: &str, payload: &[u8], expected: &SaveError) {
    let path = dir.join(name);
    save_file(&path, HOST_SAVE_KIND, payload).expect("persist writes any payload");
    let before = fs::read(&path).expect("readable");
    assert_eq!(
        &load_host_file(&path, &identity(), 2, 0).unwrap_err(),
        expected,
        "{name}"
    );
    assert_eq!(
        &decode_host_save(payload, &identity(), 2, 0).unwrap_err(),
        expected,
        "{name}"
    );
    assert_eq!(fs::read(&path).expect("readable"), before, "{name}");
}

// ---------------------------------------------------------------------------
// Gate 10: file-backed continuation equivalence.
// ---------------------------------------------------------------------------

fn file_save_load_equivalence(protocol: ProtocolSelection) {
    let dir = case_dir("file_save_load_equivalence", tag(protocol));
    let path = dir.join("host.save");
    let mut mirror = Mirror::new();
    let mut uninterrupted = Run::new(protocol);
    let mut interrupted = Run::new(protocol);
    lockstep(&mut uninterrupted, &mut interrupted, &mut mirror, 0..500);

    uninterrupted.rebind();
    let mut interrupted = restart(interrupted, &path, 2);
    lockstep(&mut uninterrupted, &mut interrupted, &mut mirror, 500..1000);

    assert_final(&uninterrupted, &interrupted, &mirror);
    assert_eq!(interrupted.host.executions(), 450);
    assert_no_temp(&dir);
}
both_versions!(file_save_load_equivalence);

fn repeated_restarts_match_uninterrupted_run(protocol: ProtocolSelection) {
    let dir = case_dir("repeated_restarts_match_uninterrupted_run", tag(protocol));
    let path = dir.join("host.save");
    let mut mirror = Mirror::new();
    let mut uninterrupted = Run::new(protocol);
    let mut interrupted = Run::new(protocol);
    lockstep(&mut uninterrupted, &mut interrupted, &mut mirror, 0..250);
    for (start, incarnation) in [(250, 2), (500, 3), (750, 4)] {
        uninterrupted.rebind();
        interrupted = restart(interrupted, &path, incarnation);
        lockstep(
            &mut uninterrupted,
            &mut interrupted,
            &mut mirror,
            start..start + 250,
        );
    }

    assert_final(&uninterrupted, &interrupted, &mirror);
    assert_eq!(interrupted.host.executions(), 225);
    assert_eq!(entries(&dir), vec!["host.save".to_owned()]);
}
both_versions!(repeated_restarts_match_uninterrupted_run);

fn older_save_resumes_from_its_own_point(protocol: ProtocolSelection) {
    let dir = case_dir("older_save_resumes_from_its_own_point", tag(protocol));
    let a = dir.join("a.save");
    let b = dir.join("b.save");

    // One uninterrupted run, saving at 250 (a) and 500 (b) on the way.
    let mut mirror = Mirror::new();
    let mut main = Run::new(protocol);
    let mut h250 = [0u8; 32];
    let mut h500 = [0u8; 32];
    for k in 0..1000 {
        if k == 250 {
            save_host_file(&a, &main.host, &identity()).expect("saves a");
            h250 = main.host.authority_hash();
        }
        if k == 500 {
            save_host_file(&b, &main.host, &identity()).expect("saves b");
            h500 = main.host.authority_hash();
        }
        let out = main.step(k);
        mirror.apply(out.action);
    }
    let final_hash = main.host.authority_hash();
    assert_eq!(final_hash, history_hash(&mirror.0));
    assert_ne!(h250, h500);

    // Load a: its own point, and exact continuation to H500.
    let loaded = load_host_file(&a, &identity(), 2, main.now).expect("loads a");
    let mut resumed = Run::from_host(loaded.host, protocol, main.now);
    assert_eq!(resumed.host.authority_hash(), h250);
    assert_eq!(resumed.host.last_capture(), 225);
    resumed.steps(250..500);
    assert_eq!(resumed.host.authority_hash(), h500);

    // Negative control: a, then 500..1000, skipping 250..500.
    let loaded = load_host_file(&a, &identity(), 3, resumed.now).expect("loads a again");
    let mut skipped = Run::from_host(loaded.host, protocol, resumed.now);
    skipped.steps(500..1000);
    assert_ne!(skipped.host.authority_hash(), final_hash);

    // Positive control: b, then 500..1000.
    let loaded = load_host_file(&b, &identity(), 4, skipped.now).expect("loads b");
    let mut from_b = Run::from_host(loaded.host, protocol, skipped.now);
    assert_eq!(from_b.host.authority_hash(), h500);
    from_b.steps(500..1000);
    assert_eq!(from_b.host.authority_hash(), final_hash);
}
both_versions!(older_save_resumes_from_its_own_point);

// ---------------------------------------------------------------------------
// Format and identity.
// ---------------------------------------------------------------------------

#[test]
fn payload_layout_matches_hand_built_bytes() {
    let id = identity_with(0x0102_0304_0506_0708_090a_0b0c_0d0e_0f10, "1.2.3-rc.1+b7");
    let mut expected =
        hex("01000000 0102030405060708090a0b0c0d0e0f10 0d 312e322e332d72632e312b6237");
    expected.push(u8::try_from(ENGINE_VERSION.len()).expect("short"));
    expected.extend_from_slice(ENGINE_VERSION.as_bytes());
    expected.extend_from_slice(&hex("7b7d"));
    assert_eq!(encode_host_save(&id, b"{}"), Ok(expected));

    // A real checkpoint is carried verbatim after the header.
    let (_host, checkpoint, hash) = quiescent();
    let payload = encode_host_save(&identity(), &checkpoint).expect("encodes");
    let header_len = HOST_SAVE_FIXED_BYTES + 5 + ENGINE_VERSION.len();
    assert!(header_len <= MAX_HOST_SAVE_HEADER_BYTES);
    assert_eq!(MAX_HOST_SAVE_HEADER_BYTES, 150);
    assert_eq!(&payload[..header_len], valid_header().as_slice());
    assert_eq!(&payload[header_len..], checkpoint.as_slice());
    let loaded = decode_host_save(&payload, &identity(), 2, 0).expect("decodes");
    assert_eq!(loaded.host.authority_hash(), hash);
    assert_eq!(loaded.host.incarnation(), 2);
}

#[test]
fn engine_version_is_valid_and_recorded() {
    assert_eq!(ENGINE_VERSION, env!("CARGO_PKG_VERSION"));
    assert!(version_text_ok(ENGINE_VERSION.as_bytes()));
    assert!(ENGINE_VERSION.len() <= MAX_VERSION_TEXT_BYTES);

    let dir = case_dir("engine_version_is_valid_and_recorded", "any");
    let (_host, checkpoint, hash) = quiescent();
    let long = format!("9.9.9-{}", "x".repeat(58));
    assert_eq!(long.len(), 64);
    for (name, engine) in [("other", "9.9.9-other+x"), ("long", long.as_str())] {
        assert!(version_text_ok(engine.as_bytes()));
        let payload = concat(&header(1, 700, b"1.0.0", engine.as_bytes()), &checkpoint);
        let path = dir.join(name);
        save_file(&path, HOST_SAVE_KIND, &payload).expect("writes");
        let loaded =
            load_host_file(&path, &identity(), 2, 0).expect("loads: recorded, not enforced");
        assert_eq!(loaded.engine_version, engine);
        assert_eq!(loaded.host.authority_hash(), hash);
        let decoded = decode_host_save(&payload, &identity(), 2, 0).expect("decodes");
        assert_eq!(decoded.engine_version, engine);
    }
}

#[test]
fn file_bytes_equal_encoded_envelope() {
    let dir = case_dir("file_bytes_equal_encoded_envelope", "any");
    let (host, checkpoint, _hash) = quiescent();
    let first = dir.join("first.save");
    let second = dir.join("second.save");
    save_host_file(&first, &host, &identity()).expect("saves");
    save_host_file(&second, &host, &identity()).expect("saves");
    let first_bytes = fs::read(&first).expect("readable");
    let second_bytes = fs::read(&second).expect("readable");
    assert_eq!(first_bytes, second_bytes);

    let payload = encode_host_save(&identity(), &checkpoint).expect("encodes");
    let envelope = encode_envelope(HOST_SAVE_KIND, &payload).expect("envelope");
    assert_eq!(first_bytes, envelope);
    assert_eq!(&first_bytes[12..20], b"HOSTCKPT");
    assert_eq!(HOST_SAVE_KIND.as_bytes(), b"HOSTCKPT");
    assert_eq!(load_file(&first, HOST_SAVE_KIND), Ok(payload));
    assert_eq!(
        entries(&dir),
        vec!["first.save".to_owned(), "second.save".to_owned()]
    );
    assert_no_temp(&dir);
}

fn resave_after_load_differs_only_in_incarnation(protocol: ProtocolSelection) {
    let dir = case_dir(
        "resave_after_load_differs_only_in_incarnation",
        tag(protocol),
    );
    let first = dir.join("first.save");
    let second = dir.join("second.save");
    let mut run = Run::new(protocol);
    run.steps(0..500);
    save_host_file(&first, &run.host, &identity()).expect("saves");
    let now = run.now;
    drop(run);
    let loaded = load_host_file(&first, &identity(), 2, now).expect("loads");
    save_host_file(&second, &loaded.host, &identity()).expect("re-saves");

    let a = load_file(&first, HOST_SAVE_KIND).expect("first payload");
    let b = load_file(&second, HOST_SAVE_KIND).expect("second payload");
    let header_len = valid_header().len();
    assert_eq!(&a[..header_len], &b[..header_len]);
    assert_eq!(&a[..header_len], valid_header().as_slice());
    let a_text = String::from_utf8(a[header_len..].to_vec()).expect("utf8");
    let b_text = String::from_utf8(b[header_len..].to_vec()).expect("utf8");
    assert_ne!(a_text, b_text);
    assert_eq!(
        b_text,
        a_text.replacen("\"incarnation\":1,", "\"incarnation\":2,", 1)
    );
}
both_versions!(resave_after_load_differs_only_in_incarnation);

#[test]
fn identity_validation() {
    let dir = case_dir("identity_validation", "any");
    let (host, checkpoint, hash) = quiescent();

    let long_valid = format!("1.0.0-{}", "a".repeat(58));
    assert_eq!(long_valid.len(), MAX_VERSION_TEXT_BYTES);
    for (name, version) in [("short", "1"), ("long", long_valid.as_str())] {
        let id = identity_with(700, version);
        let path = dir.join(name);
        save_host_file(&path, &host, &id).expect("valid identity saves");
        let loaded = load_host_file(&path, &id, 2, 0).expect("valid identity loads");
        assert_eq!(loaded.host.authority_hash(), hash);
    }

    let payload = encode_host_save(&identity(), &checkpoint).expect("encodes");
    let missing = dir.join("never-written.save");
    assert_eq!(
        load_host_file(&missing, &identity(), 2, 0).unwrap_err(),
        SaveError::Persist(FileError::Io {
            op: FileOp::Open,
            kind: ErrorKind::NotFound
        }),
        "positive control: a valid identity reaches the open"
    );
    let too_long = "a".repeat(65);
    for (index, version) in ["", too_long.as_str(), "1.0 .0", "1/0", "1_0", "é"]
        .into_iter()
        .enumerate()
    {
        let bad = identity_with(700, version);
        assert_eq!(
            encode_host_save(&bad, &checkpoint),
            Err(SaveError::InvalidIdentity),
            "{version:?}"
        );
        let sub = dir.join(format!("bad_{index}"));
        fs::create_dir(&sub).expect("creates");
        assert_eq!(
            save_host_file(&sub.join("host.save"), &host, &bad),
            Err(SaveError::InvalidIdentity),
            "{version:?}"
        );
        assert!(entries(&sub).is_empty(), "{version:?}: no file, no temp");
        assert_eq!(
            decode_host_save(&payload, &bad, 2, 0).unwrap_err(),
            SaveError::InvalidIdentity,
            "{version:?}"
        );
        assert_eq!(
            load_host_file(&missing, &bad, 2, 0).unwrap_err(),
            SaveError::InvalidIdentity,
            "{version:?}: identity precedes the open"
        );
    }
}

#[test]
fn payload_cap_boundaries() {
    let id = identity();
    assert_eq!(id.campaign_version.len(), 5);
    let header_len = HOST_SAVE_FIXED_BYTES + 5 + ENGINE_VERSION.len();
    let mut checkpoint = vec![b' '; MAX_PAYLOAD_BYTES - header_len];
    let payload = encode_host_save(&id, &checkpoint).expect("exactly at the cap");
    assert_eq!(payload.len(), MAX_PAYLOAD_BYTES);
    assert_eq!(
        decode_host_save(&payload, &id, 2, 0).unwrap_err(),
        SaveError::Checkpoint(CheckpointError::Malformed),
        "every header step passes; T022 refuses the blank checkpoint"
    );

    checkpoint.push(b' ');
    assert_eq!(
        encode_host_save(&id, &checkpoint),
        Err(SaveError::TooLarge {
            len: MAX_PAYLOAD_BYTES + 1
        })
    );
    drop(checkpoint);

    let mut over = payload;
    over.push(b' ');
    assert_eq!(
        decode_host_save(&over, &id, 2, 0).unwrap_err(),
        SaveError::TooLarge {
            len: MAX_PAYLOAD_BYTES + 1
        }
    );
}

#[test]
fn header_refusals() {
    let dir = case_dir("header_refusals", "any");
    let (_host, checkpoint, hash) = quiescent();
    let malformed = SaveError::Malformed;
    let engine = ENGINE_VERSION.as_bytes();
    let ev_len = u8::try_from(engine.len()).expect("short");

    // Every strict prefix of a valid header, then exactly the header.
    let valid = valid_header();
    for len in 0..valid.len() {
        refuse(&dir, &format!("prefix_{len}"), &valid[..len], &malformed);
    }
    refuse(
        &dir,
        "header_only",
        &valid,
        &SaveError::Checkpoint(CheckpointError::Malformed),
    );

    // The version wins over everything else in the header.
    for version in [0, 2, u32::MAX] {
        refuse(
            &dir,
            &format!("version_{version}"),
            &concat(&header(version, 700, b"1.0.0", engine), &checkpoint),
            &SaveError::UnsupportedVersion { version },
        );
    }
    refuse(
        &dir,
        "version_2_cv_len_0",
        &concat(&raw_header(2, 700, 0, b"", ev_len, engine), &checkpoint),
        &SaveError::UnsupportedVersion { version: 2 },
    );

    // Length bytes out of range.
    let a65 = [b'a'; 65];
    let a255 = [b'a'; 255];
    for (name, cv_len, cv) in [
        ("cv_len_0", 0u8, &b""[..]),
        ("cv_len_65", 65, &a65[..]),
        ("cv_len_255", 255, &a255[..]),
    ] {
        refuse(
            &dir,
            name,
            &concat(&raw_header(1, 700, cv_len, cv, ev_len, engine), &checkpoint),
            &malformed,
        );
    }
    for (name, len, ev) in [("ev_len_0", 0u8, &b""[..]), ("ev_len_65", 65, &a65[..])] {
        refuse(
            &dir,
            name,
            &concat(&raw_header(1, 700, 5, b"1.0.0", len, ev), &checkpoint),
            &malformed,
        );
    }

    // Bytes outside the version alphabet, in either text.
    for (index, text) in [
        &b"1.0 0"[..],
        &b"1.0_0"[..],
        &[b'1', b'.', 0x80, b'.', b'0'][..],
    ]
    .into_iter()
    .enumerate()
    {
        refuse(
            &dir,
            &format!("cv_byte_{index}"),
            &concat(&header(1, 700, text, engine), &checkpoint),
            &malformed,
        );
        refuse(
            &dir,
            &format!("ev_byte_{index}"),
            &concat(&header(1, 700, b"1.0.0", text), &checkpoint),
            &malformed,
        );
    }

    // Structure precedes identity: a wrong id with a malformed engine text.
    refuse(
        &dir,
        "wrong_id_bad_engine",
        &concat(&header(1, 701, b"1.0.0", b"0.1_0"), &checkpoint),
        &malformed,
    );

    // Positive control: the same hand-built header, valid fields.
    let payload = concat(&valid, &checkpoint);
    let path = dir.join("control");
    save_file(&path, HOST_SAVE_KIND, &payload).expect("writes");
    let loaded = load_host_file(&path, &identity(), 2, 0).expect("loads");
    assert_eq!(loaded.host.authority_hash(), hash);
    let decoded = decode_host_save(&payload, &identity(), 2, 0).expect("decodes");
    assert_eq!(decoded.host.authority_hash(), hash);
}

#[test]
fn identity_refusals() {
    let dir = case_dir("identity_refusals", "any");
    let (host, _checkpoint, hash) = quiescent();
    let path = dir.join("host.save");
    save_host_file(&path, &host, &identity()).expect("saves");
    let before = fs::read(&path).expect("readable");

    for (expected_identity, error) in [
        (identity_with(701, "1.0.0"), SaveError::CampaignMismatch),
        (
            identity_with(700, "1.0.1"),
            SaveError::CampaignVersionMismatch,
        ),
        (identity_with(701, "1.0.1"), SaveError::CampaignMismatch),
    ] {
        assert_eq!(
            load_host_file(&path, &expected_identity, 2, 0).unwrap_err(),
            error,
            "{expected_identity:?}"
        );
        assert_eq!(fs::read(&path).expect("readable"), before);
    }

    // Identity is checked before the checkpoint is parsed.
    refuse(
        &dir,
        "wrong_id_garbage_checkpoint",
        &concat(
            &header(1, 701, b"1.0.0", ENGINE_VERSION.as_bytes()),
            b"not a checkpoint",
        ),
        &SaveError::CampaignMismatch,
    );

    // Positive control.
    let loaded = load_host_file(&path, &identity(), 2, 0).expect("loads");
    assert_eq!(loaded.host.authority_hash(), hash);
    assert_eq!(fs::read(&path).expect("readable"), before);
}

#[test]
fn envelope_refusals_pass_through() {
    let dir = case_dir("envelope_refusals_pass_through", "any");
    let (_host, checkpoint, hash) = quiescent();
    let payload = encode_host_save(&identity(), &checkpoint).expect("encodes");
    let load = |path: &Path| load_host_file(path, &identity(), 2, 0);
    let control = |name: &str, bytes: &[u8]| {
        let path = dir.join(name);
        fs::write(&path, bytes).expect("writes");
        let loaded = load(&path).expect("positive control loads");
        assert_eq!(loaded.host.authority_hash(), hash);
    };
    let refused = |path: &Path, error: SaveError| {
        let before = fs::read(path).expect("readable");
        assert_eq!(load(path).unwrap_err(), error, "{}", path.display());
        assert_eq!(fs::read(path).expect("readable"), before);
    };

    // A good envelope.
    let good_path = dir.join("good.save");
    save_file(&good_path, HOST_SAVE_KIND, &payload).expect("writes");
    let good = fs::read(&good_path).expect("readable");

    // Another payload kind.
    let other = PayloadKind::new(*b"OTHERKND").expect("valid kind");
    let kind_path = dir.join("kind.save");
    save_file(&kind_path, other, &payload).expect("writes");
    refused(
        &kind_path,
        SaveError::Persist(FileError::Envelope(EnvelopeError::KindMismatch)),
    );
    control("kind_control.save", &good);

    // Another envelope version.
    let mut tampered = good.clone();
    tampered[8..10].copy_from_slice(&[0x02, 0x00]);
    let version_path = dir.join("version.save");
    fs::write(&version_path, &tampered).expect("writes");
    refused(
        &version_path,
        SaveError::Persist(FileError::Envelope(EnvelopeError::UnsupportedVersion)),
    );
    assert_eq!(&good[8..10], &[0x01, 0x00]);
    control("version_control.save", &good);

    // One digest byte flipped.
    let mut tampered = good.clone();
    tampered[36] ^= 0x01;
    let digest_path = dir.join("digest.save");
    fs::write(&digest_path, &tampered).expect("writes");
    refused(
        &digest_path,
        SaveError::Persist(FileError::Envelope(EnvelopeError::ChecksumMismatch)),
    );
    control("digest_control.save", &good);

    // A missing file, then the same path written.
    let missing = dir.join("missing.save");
    assert_eq!(
        load(&missing).unwrap_err(),
        SaveError::Persist(FileError::Io {
            op: FileOp::Open,
            kind: ErrorKind::NotFound
        })
    );
    assert!(!missing.exists());
    control("missing.save", &good);

    // One byte past the input cap.
    let big_path = dir.join("big.save");
    fs::write(&big_path, vec![0u8; MAX_ENVELOPE_BYTES + 1]).expect("writes");
    refused(
        &big_path,
        SaveError::Persist(FileError::Envelope(EnvelopeError::InputTooLarge)),
    );
    assert!(good.len() <= MAX_ENVELOPE_BYTES);
    control("big_control.save", &good);
}

#[test]
fn checkpoint_refusals_pass_through() {
    let dir = case_dir("checkpoint_refusals_pass_through", "any");
    let (host, checkpoint, hash) = quiescent();

    // A checkpoint version T022 does not read.
    let text = String::from_utf8(checkpoint.clone()).expect("utf8");
    let v2 = text.replacen("{\"version\":1,", "{\"version\":2,", 1);
    assert_ne!(v2, text);
    refuse(
        &dir,
        "checkpoint_v2",
        &concat(&valid_header(), v2.as_bytes()),
        &SaveError::Checkpoint(CheckpointError::UnsupportedVersion { version: 2 }),
    );

    // Incarnations not greater than the saved one (1).
    let path = dir.join("host.save");
    save_host_file(&path, &host, &identity()).expect("saves");
    let payload = load_file(&path, HOST_SAVE_KIND).expect("payload");
    for incarnation in [1, 0] {
        assert_eq!(
            load_host_file(&path, &identity(), incarnation, 0).unwrap_err(),
            SaveError::Checkpoint(CheckpointError::EpochReused)
        );
        assert_eq!(
            decode_host_save(&payload, &identity(), incarnation, 0).unwrap_err(),
            SaveError::Checkpoint(CheckpointError::EpochReused)
        );
    }
    let loaded = load_host_file(&path, &identity(), 2, 0).expect("incarnation 2 loads");
    assert_eq!(loaded.host.authority_hash(), hash);

    // Not a checkpoint.
    refuse(
        &dir,
        "empty_object",
        &concat(&valid_header(), b"{}"),
        &SaveError::Checkpoint(CheckpointError::Malformed),
    );
}

fn save_refusals_leave_files_untouched(protocol: ProtocolSelection) {
    let dir = case_dir("save_refusals_leave_files_untouched", tag(protocol));
    let path = dir.join("host.save");
    let mut run = Run::new(protocol);
    run.steps(0..20);
    save_host_file(&path, &run.host, &identity()).expect("good save B");
    let saved = fs::read(&path).expect("readable");
    let unchanged = |run: &Run, before: &Snapshot| {
        assert_eq!(fs::read(&path).expect("readable"), saved);
        assert_eq!(entries(&dir), vec!["host.save".to_owned()]);
        assert_eq!(&snapshot(&run.host), before);
    };

    // Staged ingress: save refuses before touching any file.
    let planned = run.stage(20);
    let before = snapshot(&run.host);
    assert_eq!(
        save_host_file(&path, &run.host, &identity()),
        Err(SaveError::Checkpoint(CheckpointError::PendingIngress))
    );
    unchanged(&run, &before);

    // An invalid identity: nothing read, written or created.
    assert_eq!(
        save_host_file(&path, &run.host, &identity_with(700, "1_0")),
        Err(SaveError::InvalidIdentity)
    );
    unchanged(&run, &before);

    // After the pump the same save succeeds.
    run.pump_one(20, planned.expect);
    save_host_file(&path, &run.host, &identity()).expect("quiescent save");
    let loaded = load_host_file(&path, &identity(), 2, run.now).expect("loads");
    assert_eq!(loaded.host.authority_hash(), run.host.authority_hash());
    assert_no_temp(&dir);

    // A destination that is a non-empty directory.
    let occupied = dir.join("occupied");
    fs::create_dir(&occupied).expect("creates");
    fs::write(occupied.join("keep"), b"keep").expect("writes");
    match save_host_file(&occupied, &run.host, &identity()) {
        Err(SaveError::Persist(FileError::Io {
            op: FileOp::Rename, ..
        })) => {}
        other => panic!("expected a rename failure, got {other:?}"),
    }
    assert!(occupied.is_dir());
    assert_eq!(entries(&occupied), vec!["keep".to_owned()]);
    assert_no_temp(&dir);

    // A missing parent directory.
    assert_eq!(
        save_host_file(
            &dir.join("missing").join("host.save"),
            &run.host,
            &identity()
        ),
        Err(SaveError::Persist(FileError::Io {
            op: FileOp::CreateTemp,
            kind: ErrorKind::NotFound
        }))
    );
    assert!(!dir.join("missing").exists());

    // No file name at all.
    assert_eq!(
        save_host_file(Path::new(""), &run.host, &identity()),
        Err(SaveError::Persist(FileError::InvalidPath))
    );
    assert_eq!(
        entries(&dir),
        vec!["host.save".to_owned(), "occupied".to_owned()]
    );
}
both_versions!(save_refusals_leave_files_untouched);

#[test]
fn error_display_strings_are_pinned() {
    let own = [
        (SaveError::InvalidIdentity, "InvalidIdentity at host/save"),
        (SaveError::TooLarge { len: 7 }, "TooLarge at host/save"),
        (SaveError::Malformed, "Malformed at host/save"),
        (
            SaveError::UnsupportedVersion { version: 2 },
            "UnsupportedVersion at host/save",
        ),
        (SaveError::CampaignMismatch, "CampaignMismatch at host/save"),
        (
            SaveError::CampaignVersionMismatch,
            "CampaignVersionMismatch at host/save",
        ),
    ];
    for (error, text) in &own {
        assert_eq!(error.to_string(), *text);
        assert!(std::error::Error::source(error).is_none());
    }

    let checkpoint_errors = [
        CheckpointError::TooLarge { len: 1 },
        CheckpointError::Malformed,
        CheckpointError::UnsupportedVersion { version: 2 },
        CheckpointError::EpochReused,
        CheckpointError::PendingIngress,
        CheckpointError::AuthorityInvalid,
        CheckpointError::Io,
    ];
    for inner in checkpoint_errors {
        let error = SaveError::from(inner.clone());
        assert_eq!(error, SaveError::Checkpoint(inner.clone()));
        assert_eq!(error.to_string(), inner.to_string());
        assert!(std::error::Error::source(&error).is_none());
    }
    assert_eq!(
        SaveError::Checkpoint(CheckpointError::EpochReused).to_string(),
        "EpochReused at host/checkpoint"
    );

    let file_errors = [
        FileError::Envelope(EnvelopeError::ChecksumMismatch),
        FileError::InvalidPath,
        FileError::Io {
            op: FileOp::Open,
            kind: ErrorKind::NotFound,
        },
    ];
    for inner in file_errors {
        let error = SaveError::from(inner);
        assert_eq!(error, SaveError::Persist(inner));
        assert_eq!(error.to_string(), inner.to_string());
        assert!(std::error::Error::source(&error).is_none());
    }
    assert_eq!(
        SaveError::Persist(FileError::Envelope(EnvelopeError::ChecksumMismatch)).to_string(),
        "save checksum mismatch"
    );
    assert_eq!(HOST_SAVE_VERSION, 1);
}

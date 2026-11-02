#![forbid(unsafe_code)]
//! Godot-free core of the bridge: one embedded authority, the client half,
//! the bounded local link and the closed error table.
//!
//! Nothing here may name the engine, spawn a thread or read a clock. Time is
//! injected as `u64` milliseconds; every map is ordered; no rule lives here.

mod authority;
mod client;
mod errors;
mod link;

use std::any::Any;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;
use std::rc::Rc;
use std::str::FromStr;

use crpg_core::{EntityId, Ulid};
use crpg_data::Document;
use crpg_net::protocol::SessionEpoch;
use crpg_server::host::{ControlGrant, Host, HostConfig, ProtocolSelection};
use crpg_sim::HistoryWorld;

use authority::{
    check_binding, collect_campaign, first_encounter, grants, presentation_tables,
    ruleset_abilities, Authority, Presentation, RealFs, SpecParts,
};
use client::Client;
use errors::Fault;
use link::{LinkEnd, LocalLink, LINK_FRAMES};

pub use client::{SessionView, ViewEvent, ViewReceipt, ViewValue};
pub use errors::BridgeCode;

/// One embedded single-player session; all GDScript-visible behaviour.
#[derive(Default)]
pub struct BridgeCore {
    state: State,
    view: Rc<SessionView>,
    last_error: String,
    #[cfg(test)]
    probe: Option<Probe>,
}

/// The three lifecycle states of a [`BridgeCore`].
#[derive(Default)]
enum State {
    /// No session.
    #[default]
    Closed,
    /// A live authority and client behind one link.
    Open(Box<Live>),
    /// A fatal fault fenced the session; only `close` clears it.
    Fenced(BridgeCode),
}

/// The live halves plus the client's end of the link.
struct Live {
    authority: Authority,
    client: Client,
    link: LinkEnd,
    #[cfg(test)]
    probe: Option<Probe>,
}

/// Test-only fault injection points (K29).
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Probe {
    /// Panic at the top of `open`'s guarded work.
    Open,
    /// Panic at the top of `poll`'s guarded work.
    Poll,
    /// Panic at the top of `submit_option`'s guarded work.
    Submit,
    /// Panic at the top of `close`'s guarded work.
    Close,
    /// Panic when the `Live` value is dropped.
    LiveDrop,
}

#[cfg(test)]
impl Drop for Live {
    fn drop(&mut self) {
        if self.probe == Some(Probe::LiveDrop) {
            panic!("probe");
        }
    }
}

impl BridgeCore {
    /// A closed core with an empty revision-0 view.
    pub fn new() -> Self {
        Self::default()
    }

    /// Opens the named encounter behind a fresh default-capacity link.
    pub fn open(
        &mut self,
        campaign_dir: &str,
        encounter_id: &str,
        player_side: &str,
        seed: i64,
    ) -> BridgeCode {
        self.open_on(
            campaign_dir,
            encounter_id,
            player_side,
            seed,
            &LocalLink::new(LINK_FRAMES),
        )
    }

    /// The current immutable view snapshot.
    pub fn view(&self) -> Rc<SessionView> {
        Rc::clone(&self.view)
    }

    /// The last non-zero result's `NAME` or `NAME: detail` text.
    pub fn last_error(&self) -> &str {
        &self.last_error
    }

    /// Advances the authority and the client once (B2, B3).
    pub fn poll(&mut self, now_ms: u64) -> BridgeCode {
        let result = catch_unwind(AssertUnwindSafe(|| self.poll_guarded(now_ms)));
        match result {
            Ok(code) => code,
            Err(payload) => self.panicked(payload),
        }
    }

    /// Submits one offered option (B14).
    pub fn submit_option(&mut self, revision: i64, index: i64) -> BridgeCode {
        if revision < 0 || index < 0 {
            return self.fail(BridgeCode::BadArgument, Some("negative argument"));
        }
        let result = catch_unwind(AssertUnwindSafe(|| self.submit_guarded(revision, index)));
        match result {
            Ok(code) => code,
            Err(payload) => self.panicked(payload),
        }
    }

    /// Idempotently ends the session, keeping the last error text (B21).
    pub fn close(&mut self) {
        let result = catch_unwind(AssertUnwindSafe(|| {
            #[cfg(test)]
            if self.probe == Some(Probe::Close) {
                panic!("probe");
            }
            let old = std::mem::replace(&mut self.state, State::Closed);
            if let State::Open(live) = old {
                let mut live = live;
                live.authority.shutdown();
                drop(live);
            }
        }));
        self.state = State::Closed;
        self.view = Rc::new(SessionView::default());
        if let Err(payload) = result {
            self.last_error = format!("PANICKED: {}", panic_text(payload));
        }
    }

    /// Opens on a caller-supplied link, so tests can inspect both queues.
    pub(crate) fn open_on(
        &mut self,
        campaign_dir: &str,
        encounter_id: &str,
        player_side: &str,
        seed: i64,
        link: &LocalLink,
    ) -> BridgeCode {
        if seed < 0 || (!encounter_id.is_empty() && Ulid::from_str(encounter_id).is_err()) {
            return self.fail(BridgeCode::BadArgument, Some("bad argument"));
        }
        let result = catch_unwind(AssertUnwindSafe(|| {
            self.open_guarded(campaign_dir, encounter_id, player_side, seed, link)
        }));
        match result {
            Ok(code) => code,
            Err(payload) => self.panicked(payload),
        }
    }

    /// The whole `open` work inside the panic guard (B1).
    fn open_guarded(
        &mut self,
        campaign_dir: &str,
        encounter_id: &str,
        player_side: &str,
        seed: i64,
        link: &LocalLink,
    ) -> BridgeCode {
        #[cfg(test)]
        if self.probe == Some(Probe::Open) {
            panic!("probe");
        }
        match self.state {
            State::Open(_) | State::Fenced(_) => {
                return self.fail(BridgeCode::AlreadyOpen, Some("session already open"))
            }
            State::Closed => {}
        }

        let files = match collect_campaign(&RealFs, Path::new(campaign_dir)) {
            Ok(files) => files,
            Err(fault) => return self.fail_fault(fault),
        };
        let engine = match env!("CARGO_PKG_VERSION").parse() {
            Ok(engine) => engine,
            Err(_) => {
                return self.fail(
                    BridgeCode::CampaignInvalid,
                    Some("engine version did not parse"),
                )
            }
        };
        let loaded = match crpg_data::load_campaign(&files, &engine) {
            Ok(loaded) => loaded,
            Err(error) => return self.fail(BridgeCode::CampaignInvalid, Some(&error.to_string())),
        };
        let findings = crpg_data::validate(&loaded);
        if let Some(finding) = findings.first() {
            return self.fail(BridgeCode::CampaignInvalid, Some(&finding.message));
        }

        let encounter = if encounter_id.is_empty() {
            match first_encounter(&loaded) {
                Some(id) => encounter_document(&loaded, id),
                None => {
                    return self.fail(BridgeCode::NoEncounter, Some("campaign has no encounter"))
                }
            }
        } else {
            let id = match Ulid::from_str(encounter_id) {
                Ok(id) => id,
                Err(_) => return self.fail(BridgeCode::BadArgument, Some("bad encounter id")),
            };
            match encounter_document(&loaded, id) {
                Some(encounter) => Some(encounter),
                None => return self.fail(BridgeCode::UnknownEncounter, Some("unknown encounter")),
            }
        };
        let Some(encounter) = encounter else {
            return self.fail(BridgeCode::NoEncounter, Some("campaign has no encounter"));
        };

        let parts = match SpecParts::load(&loaded, encounter) {
            Ok(parts) => parts,
            Err(fault) => return self.fail_fault(fault),
        };
        let seed_value = match u64::try_from(seed) {
            Ok(value) => value,
            Err(_) => return self.fail(BridgeCode::BadArgument, Some("negative seed")),
        };
        let mut history = HistoryWorld::new(seed_value);
        if let Err(error) = history.start_encounter(&parts.spec(encounter)) {
            return self.fail(BridgeCode::StartFailed, Some(&error.to_string()));
        }
        let bound: Vec<(EntityId, Ulid)> = history
            .world()
            .combatants()
            .iter()
            .map(|(entity, combatant)| (entity, combatant.placement()))
            .collect();
        let participants: Vec<Ulid> = encounter
            .participants
            .iter()
            .map(|participant| participant.placement)
            .collect();
        if let Err(fault) = check_binding(&bound, &participants) {
            return self.fail_fault(fault);
        }

        let presentation = Presentation::from_loaded(&loaded);
        let chosen_side = if player_side.is_empty() {
            match bound.first() {
                Some((_, placement)) => presentation.combatant(*placement).1,
                None => {
                    return self.fail(BridgeCode::UnknownSide, Some("encounter has no combatant"))
                }
            }
        } else {
            if !bound
                .iter()
                .any(|(_, placement)| presentation.combatant(*placement).1 == player_side)
            {
                return self.fail(BridgeCode::UnknownSide, Some("unknown side"));
            }
            player_side.to_owned()
        };

        if let Err(error) = history.acknowledge(history.last_sequence()) {
            return self.fail(BridgeCode::StartFailed, Some(&error.to_string()));
        }
        let mut host = match Host::from_history(
            history,
            HostConfig {
                protocol: ProtocolSelection::V2,
                incarnation: 1,
            },
            0,
        ) {
            Ok(host) => host,
            Err(error) => return self.fail(BridgeCode::StartFailed, Some(&error.to_string())),
        };

        let disclosure = grants(
            &bound,
            &ruleset_abilities(&loaded, encounter.ruleset),
            encounter.id,
        );
        let mut player_actors = Vec::new();
        let mut npc_actors = Vec::new();
        for (entity, placement) in &bound {
            if presentation.combatant(*placement).1 == chosen_side {
                player_actors.push(*entity);
            } else {
                npc_actors.push(*entity);
            }
        }
        let player = match host.bind_snapshot_peer(
            ControlGrant {
                actors: player_actors,
            },
            disclosure.clone(),
            0,
        ) {
            Ok(peer) => peer,
            Err(error) => return self.fail(BridgeCode::StartFailed, Some(&error.to_string())),
        };
        let npc = match host.bind_peer(ControlGrant { actors: npc_actors }, disclosure, 0) {
            Ok(peer) => peer,
            Err(error) => return self.fail(BridgeCode::StartFailed, Some(&error.to_string())),
        };

        let (table, npc_sides) =
            match presentation_tables(&host, player, npc, &bound, &presentation) {
                Ok(tables) => tables,
                Err(fault) => return self.fail_fault(fault),
            };
        let epoch = match host.epoch(player) {
            Ok(epoch) => SessionEpoch(epoch),
            Err(error) => return self.fail(BridgeCode::StartFailed, Some(&error.to_string())),
        };
        let client = Client::new(epoch, table);
        let authority = Authority::new(host, player, npc, link.authority_end(), npc_sides, bound);
        self.state = State::Open(Box::new(Live {
            authority,
            client,
            link: link.client_end(),
            #[cfg(test)]
            probe: self.probe,
        }));
        self.view = Rc::new(SessionView::default());
        self.last_error.clear();
        BridgeCode::Ok
    }

    /// The work behind `poll`'s guard.
    fn poll_guarded(&mut self, now_ms: u64) -> BridgeCode {
        #[cfg(test)]
        if self.probe == Some(Probe::Poll) {
            panic!("probe");
        }
        match &self.state {
            State::Fenced(code) => {
                let code = *code;
                return code;
            }
            State::Closed => return self.fail(BridgeCode::NotOpen, None),
            State::Open(_) => {}
        }
        let outcome = {
            let State::Open(live) = &mut self.state else {
                return self.fail(BridgeCode::NotOpen, None);
            };
            match live.authority.step(now_ms) {
                Ok(report) => {
                    let refused = report.refused.is_some();
                    live.client
                        .step(&mut live.link, &report.options, refused)
                        .map(|()| (live.client.view(), report.refused))
                }
                Err(fault) => Err(fault),
            }
        };
        match outcome {
            Ok((view, refused)) => {
                self.view = view;
                match refused {
                    Some(detail) => self.fail(BridgeCode::IntentRefused, Some(&detail)),
                    None => BridgeCode::Ok,
                }
            }
            Err(fault) => self.fail_fault(fault),
        }
    }

    /// The work behind `submit_option`'s guard.
    fn submit_guarded(&mut self, revision: i64, index: i64) -> BridgeCode {
        #[cfg(test)]
        if self.probe == Some(Probe::Submit) {
            panic!("probe");
        }
        let outcome = {
            match &self.state {
                State::Fenced(code) => {
                    let code = *code;
                    return code;
                }
                State::Closed => {}
                State::Open(_) => {}
            }
            if matches!(self.state, State::Closed) {
                return self.fail(BridgeCode::NotOpen, None);
            }
            let State::Open(live) = &mut self.state else {
                unreachable!("the match above admits only Open");
            };
            live.client.submit(revision, index, &mut live.link)
        };
        match outcome {
            Ok(()) => BridgeCode::Ok,
            Err(fault) => self.fail_fault(fault),
        }
    }

    /// Records a non-zero result and returns its code (B15).
    fn fail(&mut self, code: BridgeCode, detail: Option<&str>) -> BridgeCode {
        self.last_error = match detail {
            Some(text) if !text.is_empty() => format!("{}: {text}", code.name()),
            _ => code.name().to_string(),
        };
        code
    }

    /// Records a fault; a fatal code fences and tears the session down (B16).
    fn fail_fault(&mut self, fault: Fault) -> BridgeCode {
        if fault.code.is_fatal() {
            self.last_error = if fault.detail.is_empty() {
                fault.code.name().to_string()
            } else {
                format!("{}: {}", fault.code.name(), fault.detail)
            };
            self.fence(fault.code);
        } else {
            self.fail(fault.code, Some(&fault.detail));
        }
        fault.code
    }

    /// The fault's code and text already stand; now shut down and drop `Live`
    /// under one ignored guard.
    fn fence(&mut self, code: BridgeCode) {
        let old = std::mem::replace(&mut self.state, State::Fenced(code));
        if let State::Open(live) = old {
            let _ = catch_unwind(AssertUnwindSafe(move || {
                let mut live = live;
                live.authority.shutdown();
                drop(live);
            }));
        }
    }

    /// A panic caught at a guarded boundary becomes `PANICKED` and a fence.
    fn panicked(&mut self, payload: Box<dyn Any + Send>) -> BridgeCode {
        self.last_error = format!("PANICKED: {}", panic_text(payload));
        self.fence(BridgeCode::Panicked);
        BridgeCode::Panicked
    }
}

impl Drop for BridgeCore {
    fn drop(&mut self) {
        self.close();
    }
}

/// The encounter document with this identity, if any.
fn encounter_document(
    loaded: &crpg_data::LoadedCampaign,
    id: Ulid,
) -> Option<&crpg_data::Encounter> {
    loaded
        .documents
        .values()
        .find_map(|document| match document {
            Document::Encounter(encounter) if encounter.id == id => Some(encounter),
            _ => None,
        })
}

/// The panic payload as text, without ever panicking itself.
fn panic_text(payload: Box<dyn Any + Send>) -> String {
    if let Some(text) = payload.downcast_ref::<&str>() {
        (*text).to_string()
    } else if let Some(text) = payload.downcast_ref::<String>() {
        text.clone()
    } else {
        "non-string payload".to_string()
    }
}

#[cfg(test)]
mod tests;

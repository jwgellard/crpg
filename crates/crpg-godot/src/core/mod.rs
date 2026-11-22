#![forbid(unsafe_code)]
//! Godot-free core of the bridge: one embedded authority, the client half,
//! the bounded local link and the closed error table.
//!
//! Nothing here may name the engine; nothing here spawns a thread or reads a
//! clock itself: a v4 session's loop thread and `SystemClock` are
//! `crpg_server::runner`'s. Time is injected as `u64` milliseconds; every map
//! is ordered; no rule lives here.

mod authority;
mod client;
mod client_v4;
mod errors;
mod link;
mod smooth;

use std::any::Any;
use std::num::NonZeroUsize;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;
use std::rc::Rc;
use std::str::FromStr;
use std::sync::mpsc::{Receiver, TryRecvError};

use crpg_core::{EntityId, Ulid};
use crpg_data::Document;
use crpg_net::protocol::{NetId, SessionEpoch};
use crpg_net::protocol_v4::Point;
use crpg_server::host::{
    ControlGrant, DisclosureGrants, FactGrants, Host, HostConfig, HostError, PeerHandle,
    ProtocolSelection,
};
use crpg_server::run::{HostLoop, LocalClient, LocalHost};
use crpg_server::runner::{spawn, Clock, RunnerHandle, RunnerPanicked, SystemClock};
use crpg_sim::{EncounterMode, HistoryWorld};

use authority::v4::{encounter_bounds, read_options, NpcHook, OptionsReply};
use authority::{
    check_binding, collect_campaign, first_encounter, grants, presentation_tables,
    ruleset_abilities, Authority, Presentation, RealFs, SpecParts,
};
use client::{Client, Offered};
use client_v4::ClientV4;
use errors::{host_fault, Fault};
use link::{LinkEnd, LocalLink, LINK_FRAMES};

pub use client::{SessionView, ViewEvent, ViewMoveReceipt, ViewReceipt, ViewValue};
pub use errors::BridgeCode;

/// One embedded single-player session; all GDScript-visible behaviour.
#[derive(Default)]
pub struct BridgeCore {
    state: State,
    view: Rc<SessionView>,
    last_error: String,
    /// The last `poll` time; a v4 `render_positions()` reads it. 0 at `new`.
    last_poll_ms: u64,
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
    /// A live v4 session on the run loop.
    OpenV4(Box<LiveV4>),
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

/// A live v4 session: the runner owns the host; the bridge holds the client half.
struct LiveV4 {
    runner: RunnerHandle<LocalHost>,
    player: PeerHandle,
    #[cfg_attr(not(test), allow(dead_code))]
    npc: PeerHandle,
    client: ClientV4,
    link: LocalClient,
    options: Vec<Offered>,
    reply: Option<Receiver<Result<OptionsReply, HostError>>>,
    seen_refused: u64,
    /// The player's controlled replica ids, ascending (Reading 3).
    controlled: Vec<i64>,
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

/// Why a live v4 session ended, for B-B5's fault mapping.
enum V4Cause {
    /// The loop thread finished on its own; inspect the join.
    Stopped,
    /// The local link closed.
    Link,
    /// The options read failed.
    ReadError(HostError),
}

/// The shared `open` prefix: everything both session kinds do identically up
/// to the host wrap (steps 1-8 of B1), returned as owned values.
struct OpenPrefix {
    loaded: crpg_data::LoadedCampaign,
    encounter_id: Ulid,
    history: HistoryWorld,
    bound: Vec<(EntityId, Ulid)>,
    participants: Vec<Ulid>,
    presentation: Presentation,
    disclosure: DisclosureGrants,
    player_actors: Vec<EntityId>,
    npc_actors: Vec<EntityId>,
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

    /// Opens the named encounter as a protocol v4 session on the run loop (B-B1).
    pub fn open_v4(
        &mut self,
        campaign_dir: &str,
        encounter_id: &str,
        player_side: &str,
        seed: i64,
    ) -> BridgeCode {
        self.open_v4_with(
            campaign_dir,
            encounter_id,
            player_side,
            seed,
            SystemClock::new(),
        )
    }

    /// `open_v4` with the runner's clock injected (tests pass a manual clock).
    pub(crate) fn open_v4_with(
        &mut self,
        campaign_dir: &str,
        encounter_id: &str,
        player_side: &str,
        seed: i64,
        clock: impl Clock,
    ) -> BridgeCode {
        self.open_v4_mode(
            campaign_dir,
            encounter_id,
            player_side,
            seed,
            clock,
            EncounterMode::TurnBased,
        )
    }

    /// Opens the named encounter as a real-time protocol v4 session on the
    /// run loop (W-1).
    pub fn open_v4_rt(
        &mut self,
        campaign_dir: &str,
        encounter_id: &str,
        player_side: &str,
        seed: i64,
    ) -> BridgeCode {
        self.open_v4_rt_with(
            campaign_dir,
            encounter_id,
            player_side,
            seed,
            SystemClock::new(),
        )
    }

    /// `open_v4_rt` with the runner's clock injected.
    pub(crate) fn open_v4_rt_with(
        &mut self,
        campaign_dir: &str,
        encounter_id: &str,
        player_side: &str,
        seed: i64,
        clock: impl Clock,
    ) -> BridgeCode {
        self.open_v4_mode(
            campaign_dir,
            encounter_id,
            player_side,
            seed,
            clock,
            EncounterMode::RealTime,
        )
    }

    /// The replica ids the player may move in an open v4 session, ascending;
    /// empty in every other state (W-2).
    pub fn controlled(&self) -> Vec<i64> {
        match &self.state {
            State::OpenV4(live) => live.controlled.clone(),
            State::Closed | State::Open(_) | State::Fenced(_) => Vec::new(),
        }
    }

    /// Three raw `Fx16_16` integers per row of `view()`, in `net_ids()` order:
    /// where to draw each row this frame (W-3).
    pub fn render_positions(&self) -> Vec<i64> {
        match &self.state {
            State::OpenV4(live) => live.client.render_positions(self.last_poll_ms),
            State::Closed | State::Open(_) | State::Fenced(_) => self.view.positions().to_vec(),
        }
    }

    /// The one body of `open_v4_with` and `open_v4_rt_with`: validation, the
    /// panic guard, then the guarded work in `mode`.
    fn open_v4_mode(
        &mut self,
        campaign_dir: &str,
        encounter_id: &str,
        player_side: &str,
        seed: i64,
        clock: impl Clock,
        mode: EncounterMode,
    ) -> BridgeCode {
        if seed < 0 || (!encounter_id.is_empty() && Ulid::from_str(encounter_id).is_err()) {
            return self.fail(BridgeCode::BadArgument, Some("bad argument"));
        }
        let result = catch_unwind(AssertUnwindSafe(|| {
            self.open_v4_guarded(campaign_dir, encounter_id, player_side, seed, clock, mode)
        }));
        match result {
            Ok(code) => code,
            Err(payload) => self.panicked(payload),
        }
    }

    /// A turn-based move of the current options' actor (B-B8).
    pub fn declare_move(&mut self, revision: i64, x: i64, y: i64, z: i64) -> BridgeCode {
        if revision < 0 {
            return self.fail(BridgeCode::BadArgument, Some("negative argument"));
        }
        let Some(destination) = point_from(x, y, z) else {
            return self.fail(BridgeCode::BadArgument, Some("coordinate out of range"));
        };
        self.move_guarded(move |client, link| client.declare_move(revision, destination, link))
    }

    /// A lane-1 `MoveTo` for `actor` (a `net_ids()` value) (B-B8).
    pub fn move_to(&mut self, actor: i64, x: i64, y: i64, z: i64) -> BridgeCode {
        if actor <= 0 {
            return self.fail(BridgeCode::BadArgument, Some("bad actor"));
        }
        let Some(destination) = point_from(x, y, z) else {
            return self.fail(BridgeCode::BadArgument, Some("coordinate out of range"));
        };
        let actor = NetId::new(actor as u64).expect("a positive actor mints a NetId");
        self.move_guarded(move |client, link| client.move_to(actor, destination, link))
    }

    /// A lane-1 `Halt` for `actor` (B-B8).
    pub fn halt(&mut self, actor: i64) -> BridgeCode {
        if actor <= 0 {
            return self.fail(BridgeCode::BadArgument, Some("bad actor"));
        }
        let actor = NetId::new(actor as u64).expect("a positive actor mints a NetId");
        self.move_guarded(move |client, link| client.halt(actor, link))
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
        self.last_poll_ms = now_ms;
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
            match old {
                State::Open(live) => {
                    let mut live = live;
                    live.authority.shutdown();
                    drop(live);
                }
                State::OpenV4(live) => {
                    let LiveV4 { runner, .. } = *live;
                    let _ = runner.call(|l| l.hosting_mut().host_mut().shutdown());
                    let _ = runner.join();
                }
                State::Closed | State::Fenced(_) => {}
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
            State::Open(_) | State::OpenV4(_) | State::Fenced(_) => {
                return self.fail(BridgeCode::AlreadyOpen, Some("session already open"))
            }
            State::Closed => {}
        }
        let prefix = match self.open_prefix(
            campaign_dir,
            encounter_id,
            player_side,
            seed,
            EncounterMode::TurnBased,
        ) {
            Ok(prefix) => prefix,
            Err(code) => return code,
        };

        let mut host = match Host::from_history(
            prefix.history,
            HostConfig {
                protocol: ProtocolSelection::V2,
                incarnation: 1,
            },
            0,
        ) {
            Ok(host) => host,
            Err(error) => return self.fail(BridgeCode::StartFailed, Some(&error.to_string())),
        };
        let player = match host.bind_snapshot_peer(
            ControlGrant {
                actors: prefix.player_actors,
            },
            prefix.disclosure.clone(),
            0,
        ) {
            Ok(peer) => peer,
            Err(error) => return self.fail(BridgeCode::StartFailed, Some(&error.to_string())),
        };
        let npc = match host.bind_peer(
            ControlGrant {
                actors: prefix.npc_actors,
            },
            prefix.disclosure,
            0,
        ) {
            Ok(peer) => peer,
            Err(error) => return self.fail(BridgeCode::StartFailed, Some(&error.to_string())),
        };

        let (table, npc_sides) =
            match presentation_tables(&host, player, npc, &prefix.bound, &prefix.presentation) {
                Ok(tables) => tables,
                Err(fault) => return self.fail_fault(fault),
            };
        let epoch = match host.epoch(player) {
            Ok(epoch) => SessionEpoch(epoch),
            Err(error) => return self.fail(BridgeCode::StartFailed, Some(&error.to_string())),
        };
        let client = Client::new(epoch, table);
        let authority = Authority::new(
            host,
            player,
            npc,
            link.authority_end(),
            npc_sides,
            prefix.bound,
        );
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

    /// The whole `open_v4` work inside the panic guard (B-B1).
    fn open_v4_guarded(
        &mut self,
        campaign_dir: &str,
        encounter_id: &str,
        player_side: &str,
        seed: i64,
        clock: impl Clock,
        mode: EncounterMode,
    ) -> BridgeCode {
        #[cfg(test)]
        if self.probe == Some(Probe::Open) {
            panic!("probe");
        }
        match self.state {
            State::Open(_) | State::OpenV4(_) | State::Fenced(_) => {
                return self.fail(BridgeCode::AlreadyOpen, Some("session already open"))
            }
            State::Closed => {}
        }
        let prefix = match self.open_prefix(campaign_dir, encounter_id, player_side, seed, mode) {
            Ok(prefix) => prefix,
            Err(code) => return code,
        };

        let Some(encounter) = encounter_document(&prefix.loaded, prefix.encounter_id) else {
            return self.fail(BridgeCode::NoEncounter, Some("campaign has no encounter"));
        };
        let parts = match SpecParts::load(&prefix.loaded, encounter) {
            Ok(parts) => parts,
            Err(fault) => return self.fail_fault(fault),
        };
        let (min, max) = match encounter_bounds(&prefix.loaded, &parts, &prefix.participants) {
            Ok(bounds) => bounds,
            Err(fault) => return self.fail_fault(fault),
        };

        let mut host = match Host::from_history(
            prefix.history,
            HostConfig {
                protocol: ProtocolSelection::V2,
                incarnation: 1,
            },
            0,
        ) {
            Ok(host) => host,
            Err(error) => return self.fail(BridgeCode::StartFailed, Some(&error.to_string())),
        };
        if let Err(error) = host.bind_area_bounds(min, max) {
            return self.fail(BridgeCode::StartFailed, Some(&error.to_string()));
        }
        let player_actors = prefix.player_actors.clone();
        let player = match host.bind_v4_peer(
            ControlGrant {
                actors: prefix.player_actors,
            },
            prefix.disclosure.clone(),
            FactGrants::default(),
            crpg_net::protocol_v3::PauseRights::default(),
            0,
        ) {
            Ok(peer) => peer,
            Err(error) => return self.fail(BridgeCode::StartFailed, Some(&error.to_string())),
        };
        let npc = match host.bind_peer(
            ControlGrant {
                actors: prefix.npc_actors,
            },
            prefix.disclosure,
            0,
        ) {
            Ok(peer) => peer,
            Err(error) => return self.fail(BridgeCode::StartFailed, Some(&error.to_string())),
        };

        let (table, npc_sides) =
            match presentation_tables(&host, player, npc, &prefix.bound, &prefix.presentation) {
                Ok(tables) => tables,
                Err(fault) => return self.fail_fault(fault),
            };
        let epoch = match host.epoch(player) {
            Ok(epoch) => SessionEpoch(epoch),
            Err(error) => return self.fail(BridgeCode::StartFailed, Some(&error.to_string())),
        };
        let client = match ClientV4::new(epoch, table) {
            Ok(client) => client,
            Err(fault) => return self.fail_fault(fault),
        };
        let mut controlled = Vec::with_capacity(player_actors.len());
        for entity in &player_actors {
            match host.replica_id(player, *entity) {
                Ok(Some(id)) => controlled.push(client::wide(id.get())),
                Ok(None) | Err(_) => {
                    return self.fail(BridgeCode::StartFailed, Some("no player replica id"))
                }
            }
        }
        controlled.sort_unstable();

        let mut local = LocalHost::new(host);
        let link = match local.attach(player, NonZeroUsize::new(LINK_FRAMES).expect("nonzero")) {
            Ok(link) => link,
            Err(error) => return self.fail(BridgeCode::StartFailed, Some(&error.to_string())),
        };
        local.set_ai(Some(NpcHook::new(npc, npc_sides).into_hook()));
        let runner = match spawn(HostLoop::new(local, 0), clock) {
            Ok(runner) => runner,
            Err(error) => return self.fail(BridgeCode::StartFailed, Some(&error.to_string())),
        };

        self.state = State::OpenV4(Box::new(LiveV4 {
            runner,
            player,
            npc,
            client,
            link,
            options: Vec::new(),
            reply: None,
            seen_refused: 0,
            controlled,
        }));
        self.view = Rc::new(SessionView::default());
        self.last_error.clear();
        BridgeCode::Ok
    }

    /// The shared steps both `open` and `open_v4` run, up to the host wrap
    /// (B1(1)-(8), B-B1). A failure has already recorded its fault.
    fn open_prefix(
        &mut self,
        campaign_dir: &str,
        encounter_id: &str,
        player_side: &str,
        seed: i64,
        mode: EncounterMode,
    ) -> Result<OpenPrefix, BridgeCode> {
        let files = match collect_campaign(&RealFs, Path::new(campaign_dir)) {
            Ok(files) => files,
            Err(fault) => return Err(self.fail_fault(fault)),
        };
        let engine = match env!("CARGO_PKG_VERSION").parse() {
            Ok(engine) => engine,
            Err(_) => {
                return Err(self.fail(
                    BridgeCode::CampaignInvalid,
                    Some("engine version did not parse"),
                ))
            }
        };
        let loaded = match crpg_data::load_campaign(&files, &engine) {
            Ok(loaded) => loaded,
            Err(error) => {
                return Err(self.fail(BridgeCode::CampaignInvalid, Some(&error.to_string())))
            }
        };
        let findings = crpg_data::validate(&loaded);
        if let Some(finding) = findings.first() {
            return Err(self.fail(BridgeCode::CampaignInvalid, Some(&finding.message)));
        }

        let encounter_id = if encounter_id.is_empty() {
            match first_encounter(&loaded) {
                Some(id) => id,
                None => {
                    return Err(
                        self.fail(BridgeCode::NoEncounter, Some("campaign has no encounter"))
                    )
                }
            }
        } else {
            let id = match Ulid::from_str(encounter_id) {
                Ok(id) => id,
                Err(_) => return Err(self.fail(BridgeCode::BadArgument, Some("bad encounter id"))),
            };
            match encounter_document(&loaded, id) {
                Some(_) => id,
                None => {
                    return Err(self.fail(BridgeCode::UnknownEncounter, Some("unknown encounter")))
                }
            }
        };
        let Some(encounter) = encounter_document(&loaded, encounter_id) else {
            return Err(self.fail(BridgeCode::NoEncounter, Some("campaign has no encounter")));
        };

        let parts = match SpecParts::load(&loaded, encounter) {
            Ok(parts) => parts,
            Err(fault) => return Err(self.fail_fault(fault)),
        };
        let seed_value = match u64::try_from(seed) {
            Ok(value) => value,
            Err(_) => return Err(self.fail(BridgeCode::BadArgument, Some("negative seed"))),
        };
        let mut history = HistoryWorld::new(seed_value);
        if let Err(error) = history.start_encounter_in_mode(&parts.spec(encounter), mode) {
            return Err(self.fail(BridgeCode::StartFailed, Some(&error.to_string())));
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
            return Err(self.fail_fault(fault));
        }

        let presentation = Presentation::from_loaded(&loaded);
        let chosen_side = if player_side.is_empty() {
            match bound.first() {
                Some((_, placement)) => presentation.combatant(*placement).1,
                None => {
                    return Err(
                        self.fail(BridgeCode::UnknownSide, Some("encounter has no combatant"))
                    )
                }
            }
        } else {
            if !bound
                .iter()
                .any(|(_, placement)| presentation.combatant(*placement).1 == player_side)
            {
                return Err(self.fail(BridgeCode::UnknownSide, Some("unknown side")));
            }
            player_side.to_owned()
        };

        if let Err(error) = history.acknowledge(history.last_sequence()) {
            return Err(self.fail(BridgeCode::StartFailed, Some(&error.to_string())));
        }
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
        Ok(OpenPrefix {
            loaded,
            encounter_id,
            history,
            bound,
            participants,
            presentation,
            disclosure,
            player_actors,
            npc_actors,
        })
    }

    /// The common guarded prefix of the three move entry points (B-B8).
    ///
    /// Runs the panic guard, the `Fenced`/`Closed` arms and the v4-session
    /// check, then hands the live v4 client and link to `call`.
    fn move_guarded(
        &mut self,
        call: impl FnOnce(&mut ClientV4, &mut LocalClient) -> Result<(), Fault>,
    ) -> BridgeCode {
        let result = catch_unwind(AssertUnwindSafe(|| {
            #[cfg(test)]
            if self.probe == Some(Probe::Submit) {
                panic!("probe");
            }
            match &self.state {
                State::Fenced(code) => {
                    let code = *code;
                    return code;
                }
                State::Closed => return self.fail(BridgeCode::NotOpen, None),
                State::Open(_) => {
                    return self.fail(BridgeCode::BadArgument, Some("not a v4 session"))
                }
                State::OpenV4(_) => {}
            }
            let outcome = {
                let State::OpenV4(live) = &mut self.state else {
                    unreachable!("the match above admits only OpenV4");
                };
                call(&mut live.client, &mut live.link)
            };
            match outcome {
                Ok(()) => BridgeCode::Ok,
                Err(fault) => self.fail_fault(fault),
            }
        }));
        match result {
            Ok(code) => code,
            Err(payload) => self.panicked(payload),
        }
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
            State::Open(_) | State::OpenV4(_) => {}
        }
        if matches!(self.state, State::OpenV4(_)) {
            return self.poll_v4_guarded(now_ms);
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

    /// The v4 half of `poll` (B-B4): adopt a reply, notice the loop ended,
    /// step the client, then submit the next options read — never waiting on
    /// the loop thread.
    fn poll_v4_guarded(&mut self, now_ms: u64) -> BridgeCode {
        let mut refused_now = false;
        // (1) adopt a pending options read, if one is outstanding.
        {
            let State::OpenV4(live) = &mut self.state else {
                unreachable!("poll_v4_guarded runs only on a live v4 session");
            };
            if let Some(reply) = live.reply.as_mut() {
                match reply.try_recv() {
                    Ok(Ok(reply)) => {
                        live.options = reply.options;
                        refused_now = reply.refused > live.seen_refused;
                        live.seen_refused = reply.refused;
                        live.reply = None;
                    }
                    Ok(Err(error)) => return self.end_v4(V4Cause::ReadError(error)),
                    Err(TryRecvError::Empty) => {}
                    Err(TryRecvError::Disconnected) => return self.end_v4(V4Cause::Stopped),
                }
            }
        }
        // (2) the loop ended.
        {
            let State::OpenV4(live) = &mut self.state else {
                unreachable!("poll_v4_guarded runs only on a live v4 session");
            };
            if live.runner.is_finished() {
                return self.end_v4(V4Cause::Stopped);
            }
        }
        // (3) the client half.
        {
            let State::OpenV4(live) = &mut self.state else {
                unreachable!("poll_v4_guarded runs only on a live v4 session");
            };
            if let Err(fault) = live
                .client
                .step(&mut live.link, &live.options, refused_now, now_ms)
            {
                return self.fail_fault(fault);
            }
        }
        // (4) the link closed.
        {
            let State::OpenV4(live) = &mut self.state else {
                unreachable!("poll_v4_guarded runs only on a live v4 session");
            };
            if live.link.is_closed() {
                return self.end_v4(V4Cause::Link);
            }
        }
        // (5) submit the next options read when none is outstanding.
        {
            let State::OpenV4(live) = &mut self.state else {
                unreachable!("poll_v4_guarded runs only on a live v4 session");
            };
            if live.reply.is_none() {
                let player = live.player;
                live.reply = Some(live.runner.submit(move |l| read_options(l, player)));
            }
        }
        // (6) publish the view and the refusal.
        let view = {
            let State::OpenV4(live) = &mut self.state else {
                unreachable!("poll_v4_guarded runs only on a live v4 session");
            };
            live.client.view()
        };
        self.view = view;
        if refused_now {
            self.fail(BridgeCode::IntentRefused, Some("player intent refused"))
        } else {
            BridgeCode::Ok
        }
    }

    /// B-B5: the loop ended or the link closed. End the runner, then fence
    /// with the first cause that applies.
    fn end_v4(&mut self, cause: V4Cause) -> BridgeCode {
        let live = match std::mem::replace(&mut self.state, State::Closed) {
            State::OpenV4(live) => live,
            _ => unreachable!("end_v4 runs only on a live v4 session"),
        };
        let LiveV4 { runner, .. } = *live;
        let fault = match runner.join() {
            Err(RunnerPanicked) => Fault {
                code: BridgeCode::Panicked,
                detail: "server loop panicked".to_string(),
            },
            Ok(exit) => match exit.error {
                Some(crpg_server::run::LoopError::Hosting(error)) => host_fault(&error),
                Some(crpg_server::run::LoopError::TimeRegression { .. }) => Fault {
                    code: BridgeCode::HostFailed,
                    detail: "server loop stopped: TimeRegression".to_string(),
                },
                None => match cause {
                    V4Cause::ReadError(error) => host_fault(&error),
                    V4Cause::Link => Fault {
                        code: BridgeCode::HostFailed,
                        detail: "server closed the link".to_string(),
                    },
                    V4Cause::Stopped => Fault {
                        code: BridgeCode::HostFailed,
                        detail: "server loop stopped".to_string(),
                    },
                },
            },
        };
        self.last_error = if fault.detail.is_empty() {
            fault.code.name().to_string()
        } else {
            format!("{}: {}", fault.code.name(), fault.detail)
        };
        self.state = State::Fenced(fault.code);
        fault.code
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
                State::Open(_) | State::OpenV4(_) => {}
            }
            if matches!(self.state, State::Closed) {
                return self.fail(BridgeCode::NotOpen, None);
            }
            match &mut self.state {
                State::Open(live) => live.client.submit(revision, index, &mut live.link),
                State::OpenV4(live) => live.client.submit(revision, index, &mut live.link),
                State::Closed | State::Fenced(_) => unreachable!("the match above admits a live"),
            }
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
        match old {
            State::Open(live) => {
                let _ = catch_unwind(AssertUnwindSafe(move || {
                    let mut live = live;
                    live.authority.shutdown();
                    drop(live);
                }));
            }
            State::OpenV4(live) => {
                let _ = catch_unwind(AssertUnwindSafe(move || {
                    let LiveV4 { runner, .. } = *live;
                    let _ = runner.call(|l| l.hosting_mut().host_mut().shutdown());
                    let _ = runner.join();
                }));
            }
            State::Closed | State::Fenced(_) => {}
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

/// Converts a move destination to a [`Point`]; `None` when any coordinate is
/// not an `i32` or `Point::new` refuses it (B-B8).
fn point_from(x: i64, y: i64, z: i64) -> Option<Point> {
    let x = i32::try_from(x).ok()?;
    let y = i32::try_from(y).ok()?;
    let z = i32::try_from(z).ok()?;
    Point::new([x, y, z])
}

#[cfg(test)]
impl BridgeCore {
    /// The runner of a live v4 session.
    pub(crate) fn runner(&self) -> Option<&RunnerHandle<LocalHost>> {
        match &self.state {
            State::OpenV4(live) => Some(&live.runner),
            _ => None,
        }
    }

    /// `(player, npc)` of a live v4 session.
    pub(crate) fn v4_peers(&self) -> Option<(PeerHandle, PeerHandle)> {
        match &self.state {
            State::OpenV4(live) => Some((live.player, live.npc)),
            _ => None,
        }
    }

    /// The client end of a live v4 session's link.
    pub(crate) fn v4_link(&mut self) -> Option<&mut LocalClient> {
        match &mut self.state {
            State::OpenV4(live) => Some(&mut live.link),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests;

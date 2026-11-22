//! The protocol v4 authority half (T151b): the encounter's area bounds, the
//! NPC driver as the run loop's AI hook, and the between-cycles options read.
//!
//! A v4 session's server runs on `crpg_server::run`'s `HostLoop` over
//! `LocalHost`, driven by `crpg_server::runner`; nothing here ticks, pumps or
//! drives lane 1. The NPC acts only through [`NpcHook`], and legal options
//! are read between cycles without blocking the frame (Reading 19).

use std::collections::BTreeMap;

use crpg_core::{Fx16_16, Ulid};
use crpg_data::{Document, LoadedCampaign};
use crpg_net::codec_v2::encode_intent;
use crpg_net::protocol::{IntentFrame, NetId, LANE_COMBAT, SEQ_FIRST};
use crpg_server::host::{Host, HostError, IngestDisposition, PeerHandle};
use crpg_server::run::{AiHook, HostLoop, LocalHost};

use super::super::client::Offered;
use super::super::errors::{BridgeCode, Fault};
use super::{choose_npc, NpcPacer, SpecParts};

/// The bounds of the area owning `participants[0]`'s placement (Reading 4).
///
/// Missing placement or area is `25` `"encounter area missing"`.
pub(crate) fn encounter_bounds(
    loaded: &LoadedCampaign,
    parts: &SpecParts<'_>,
    participants: &[Ulid],
) -> Result<([Fx16_16; 3], [Fx16_16; 3]), Fault> {
    let missing = || Fault {
        code: BridgeCode::StartFailed,
        detail: "encounter area missing".to_string(),
    };
    let Some(first) = participants.first() else {
        return Err(missing());
    };
    let Some(placement) = parts.placements.get(first) else {
        return Err(missing());
    };
    let Some(area) = loaded
        .documents
        .values()
        .find_map(|document| match document {
            Document::Area(area) if area.id == placement.area => Some(area),
            _ => None,
        })
    else {
        return Err(missing());
    };
    Ok((area.bounds.min, area.bounds.max))
}

/// Today's NPC driver as the run loop's AI hook (Reading 20).
pub(crate) struct NpcHook {
    npc: PeerHandle,
    sides: BTreeMap<NetId, String>,
    next_seq: u64,
    pacer: NpcPacer,
}

impl NpcHook {
    /// An unarmed hook for the NPC peer and its sides table.
    pub(crate) fn new(npc: PeerHandle, sides: BTreeMap<NetId, String>) -> Self {
        Self {
            npc,
            sides,
            next_seq: SEQ_FIRST,
            pacer: NpcPacer::default(),
        }
    }

    /// One AI stage (B-B2): a closed host is reported `Closed` before any
    /// other call (a deliberate shutdown is 50, not 51); otherwise drain the
    /// NPC's delivery unread, then fire at most one staged intent through the
    /// unchanged `choose_npc` + pacer.
    pub(crate) fn run(&mut self, host: &mut Host, now_ms: u64) -> Result<(), HostError> {
        if host.is_closed() {
            return Err(HostError::Closed);
        }
        let entries = host.take_delivery_entries(self.npc)?;
        if let Some(last) = entries.last() {
            host.acknowledge_delivery(self.npc, last.ordinal)?;
        }

        let options = host.legal_intents(self.npc)?;
        let fire = self.pacer.step(now_ms, !options.is_empty(), false);
        if fire {
            if let Some(index) = choose_npc(&options, &self.sides) {
                let chosen = options[index];
                let frame = IntentFrame {
                    epoch: host.epoch(self.npc)?,
                    lane: LANE_COMBAT,
                    seq: self.next_seq,
                    observed_tick: host.server_tick(),
                    actor: chosen.actor,
                    body: chosen.body,
                };
                if let Ok(bytes) = encode_intent(&frame) {
                    match host.ingest(self.npc, &bytes, now_ms) {
                        Ok(IngestDisposition::Staged) => {
                            self.next_seq = self.next_seq.saturating_add(1);
                        }
                        Ok(IngestDisposition::Refused { .. }) => {}
                        Err(error) => return Err(error),
                    }
                }
            }
        }
        Ok(())
    }

    /// The boxed hook `LocalHost::set_ai` takes: `run` on the moved `self`.
    pub(crate) fn into_hook(mut self) -> AiHook {
        Box::new(move |host: &mut Host, now_ms: u64| self.run(host, now_ms))
    }
}

/// One between-cycles read for the client half (Reading 19).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OptionsReply {
    /// The player's legal intents, in host order.
    pub(crate) options: Vec<Offered>,
    /// The player's `LinkStats.refused`.
    pub(crate) refused: u64,
}

/// The body of the closure a v4 `poll` submits (B-B3).
pub(crate) fn read_options(
    host_loop: &mut HostLoop<LocalHost>,
    player: PeerHandle,
) -> Result<OptionsReply, HostError> {
    match host_loop.hosting().link_stats(player) {
        None => Err(HostError::UnknownPeer),
        Some(stats) => {
            let options = host_loop
                .hosting()
                .host()
                .legal_intents(player)?
                .into_iter()
                .map(|option| Offered {
                    actor: option.actor,
                    body: option.body,
                })
                .collect();
            Ok(OptionsReply {
                options,
                refused: stats.refused,
            })
        }
    }
}

#[cfg(test)]
#[path = "v4/tests/rt.rs"]
pub(crate) mod rt_tests;
#[cfg(test)]
mod tests;

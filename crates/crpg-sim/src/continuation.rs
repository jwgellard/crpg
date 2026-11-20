//! Persisted script, dialogue and quest state (T052, ADR-0008 note of 2026-11-19).
//!
//! `World` stores three things that must survive save/load and a server
//! restart: a running event graph paused in `Wait` (an entity holding a
//! [`ScriptContinuation`]), a participant's open dialogue ([`OpenDialogue`],
//! keyed by the participant), and each quest's current state
//! ([`QuestProgress`], strictly ascending by quest, not entities).
//!
//! The sim only stores, counts down and reports what is due. It never resumes
//! a graph, walks a dialogue or transitions a quest: that is `crpg-script`,
//! which depends on this crate and never the reverse. No Lua state is stored,
//! and the sim never resolves an authored id: every id is opaque here.

use std::fmt;

use crpg_core::{EntityId, Ulid};
use serde::{Deserialize, Serialize};

use crate::world::World;

/// A running event graph paused in `Wait` (Reading 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScriptContinuation {
    /// Authored `EventGraph.id`.
    pub graph: Ulid,
    /// The authored `Wait` node the graph resumes after.
    pub node: Ulid,
    /// Ticks left; `0` is due.
    pub remaining: u64,
}

/// One participant's open dialogue (Reading 2b).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenDialogue {
    /// Authored `Dialogue.id`.
    pub dialogue: Ulid,
    /// The current `DialogueNode.id`.
    pub node: Ulid,
}

/// One quest's current state (Reading 2c).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestProgress {
    /// Authored `Quest.id`.
    pub quest: Ulid,
    /// The current `QuestState.id`.
    pub state: Ulid,
}

/// A refused script-state operation; the world is unchanged.
///
/// `Display` is `<VariantName> at script/state`, without fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContinuationError {
    /// The entity holds no continuation (absent, dead or never one).
    NotContinuation {
        /// The entity that holds none.
        entity: EntityId,
    },
    /// The participant is not live.
    NotLive {
        /// The participant.
        entity: EntityId,
    },
    /// The participant already has an open dialogue.
    DialogueOpen {
        /// The participant.
        entity: EntityId,
    },
    /// The participant has no open dialogue.
    NoDialogue {
        /// The participant.
        entity: EntityId,
    },
}

impl fmt::Display for ContinuationError {
    /// Renders `<VariantName> at script/state`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::NotContinuation { .. } => "NotContinuation",
            Self::NotLive { .. } => "NotLive",
            Self::DialogueOpen { .. } => "DialogueOpen",
            Self::NoDialogue { .. } => "NoDialogue",
        };
        write!(f, "{name} at script/state")
    }
}

impl std::error::Error for ContinuationError {}

/// Spec §10 stage 2: every positive `remaining` decreases by exactly one.
///
/// Reads and writes only the continuation store. Records at `0` stay `0`;
/// nothing is enqueued, reported or removed.
pub(crate) fn wait_system(world: &mut World) {
    for (_, record) in world.continuations_mut().iter_mut() {
        record.remaining = record.remaining.saturating_sub(1);
    }
}

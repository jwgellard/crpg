//! Tier-1 interest projection (T027b): one viewer, one area, host-supplied facts.
//!
//! [`InterestView`] owns the per-peer `NetId` mapping, the last rendered image
//! and a view cursor, and turns host-authorized rows into existing v1
//! [`DeltaOp`] values or one T024 [`SnapshotBody`]. It reads no `World`, RNG or
//! event state and adds no wire form: the host still keeps frames, chunking,
//! `server_tick` and `event_seq`. Identity is `(area, EntityId)`, because two
//! areas may mint equal `EntityId`s; ids are minted only for disclosed keys and
//! retirement is derived from `next_id`, never stored.

use core::fmt;
use std::collections::{BTreeMap, BTreeSet};

use crpg_core::{EntityId, Ulid};

use crate::protocol::{DeltaOp, NetId, SessionEpoch, MAX_VISIBLE_ENTITIES};
use crate::protocol_transfer::{HealthFact, SnapshotBody, SnapshotEntity, TurnFact};

/// Largest `ops` length of one [`InterestView::project_delta`] result:
/// 1024 enters + 1024 `Health` + 1 `Turn` + 1024 removals.
pub const MAX_PAGE_OPS: usize = 3 * MAX_VISIBLE_ENTITIES + 1;

/// Runtime identity of one entity. Order: area, then `EntityId`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct EntityKey {
    /// The authoritative area the entity is in.
    pub area: Ulid,
    /// The full arena id, generation included.
    pub entity: EntityId,
}

/// Tier-1 interest: exactly one area.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InterestSet {
    /// The viewer's area for this call.
    pub area: Ulid,
}

/// Per-row grants. `Default` denies everything.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FieldMask {
    /// The entity itself may be disclosed.
    pub presence: bool,
    /// The entity's health fact may be disclosed.
    pub health: bool,
}

/// One authoritative row for one viewer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntityFact {
    /// Identity in authority terms.
    pub key: EntityKey,
    /// What this viewer may see of the row.
    pub fields: FieldMask,
    /// The value; disclosure still needs `fields.health`.
    pub health: Option<HealthFact>,
    /// This page only: emit `Health` even if unchanged.
    pub resend_health: bool,
}

/// The encounter's turn fact in authority terms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TurnState {
    /// The area whose encounter this is.
    pub area: Ulid,
    /// The authoritative active actor, if any.
    pub active: Option<EntityId>,
    /// The current round.
    pub round: u64,
}

/// One coherent viewer context for one call. Field order is contract.
#[derive(Debug, Clone, Copy)]
pub struct ViewerContext<'a> {
    /// The session epoch this context belongs to.
    pub epoch: SessionEpoch,
    /// The authoritative tick this context reflects.
    pub tick: u64,
    /// Tier-1 interest: exactly one area.
    pub interest: InterestSet,
    /// At most [`MAX_VISIBLE_ENTITIES`]; order does not matter.
    pub facts: &'a [EntityFact],
    /// Keys whose removal this page may read as `Despawned`. At most
    /// [`MAX_VISIBLE_ENTITIES`]; order and repeats do not matter.
    pub despawned: &'a [EntityKey],
    /// The encounter's turn, if the authority has one.
    pub turn: Option<TurnState>,
    /// Whether the turn fact may be disclosed.
    pub turn_granted: bool,
    /// This page only: emit `Turn` even if unchanged.
    pub resend_turn: bool,
}

/// View position. Not wire and not `event_seq`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ViewCursor {
    /// The context tick of the last non-empty projection.
    pub tick: u64,
    /// How many non-empty projections this view has made, saturating.
    pub revision: u64,
}

/// One filtered delta page plus the view position after the call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateProjection {
    /// At most [`MAX_PAGE_OPS`], in the Behaviour item 5 layout.
    pub ops: Vec<DeltaOp>,
    /// `ops[..enters]` are the `EntityEnter`s; history goes at this index.
    pub enters: usize,
    /// The view position after the call.
    pub cursor: ViewCursor,
}

/// One snapshot body plus the view position after the call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotProjection {
    /// One entity per visible key plus the rendered turn.
    pub body: SnapshotBody,
    /// The view position after the call.
    pub cursor: ViewCursor,
}

/// A rejected interest-projection call. Every variant leaves the view intact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterestError {
    /// `context.epoch` is not this view's epoch.
    WrongEpoch,
    /// `facts` or `despawned` is longer than [`MAX_VISIBLE_ENTITIES`].
    TooManyFacts,
    /// Two rows of `facts` share one [`EntityKey`].
    DuplicateFact,
    /// The epoch's `NetId` space cannot cover this call's entering keys.
    IdExhausted,
}

impl fmt::Display for InterestError {
    /// Renders `<VariantName> at projection/interest`, without payloads.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongEpoch => write!(f, "WrongEpoch at projection/interest"),
            Self::TooManyFacts => write!(f, "TooManyFacts at projection/interest"),
            Self::DuplicateFact => write!(f, "DuplicateFact at projection/interest"),
            Self::IdExhausted => write!(f, "IdExhausted at projection/interest"),
        }
    }
}

impl std::error::Error for InterestError {}

/// Per-peer mapping, remembered image and cursor.
///
/// Not `Serialize`: a saved view would restore stale authorization. There is
/// deliberately no restore constructor and no second view per epoch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterestView {
    epoch: SessionEpoch,
    next_id: u64,
    live: BTreeMap<EntityKey, LiveEntry>,
    index: BTreeMap<NetId, EntityKey>,
    last_turn: Option<TurnFact>,
    cursor: ViewCursor,
}

/// One live mapping with its last rendered health.
#[derive(Debug, Clone, PartialEq, Eq)]
struct LiveEntry {
    net: NetId,
    last_health: Option<HealthFact>,
}

impl InterestView {
    /// Opens an empty view for one session epoch. Ids start at 1.
    pub fn new(epoch: SessionEpoch) -> Self {
        Self {
            epoch,
            next_id: 1,
            live: BTreeMap::new(),
            index: BTreeMap::new(),
            last_turn: None,
            cursor: ViewCursor::default(),
        }
    }

    /// Returns the epoch this view was opened for.
    pub fn epoch(&self) -> SessionEpoch {
        self.epoch
    }

    /// Returns the view position after the last non-empty projection.
    pub fn cursor(&self) -> ViewCursor {
        self.cursor
    }

    /// Returns the number of live mappings.
    pub fn len(&self) -> usize {
        self.live.len()
    }

    /// Returns `true` when no mapping is live.
    pub fn is_empty(&self) -> bool {
        self.live.is_empty()
    }

    /// Resolves a runtime key to its current `NetId`.
    ///
    /// A retired, hidden or foreign key, and one never disclosed, is `None`.
    pub fn net_id(&self, key: EntityKey) -> Option<NetId> {
        self.live.get(&key).map(|entry| entry.net)
    }

    /// Resolves a `NetId` to its current runtime key.
    ///
    /// A retired, hidden or never-issued id, and a foreign area's id, is
    /// `None`.
    pub fn entity(&self, net: NetId) -> Option<EntityKey> {
        self.index.get(&net).copied()
    }

    /// Projects one delta page for this viewer's context.
    ///
    /// Checks epoch, bounds, duplicate rows and id exhaustion before any
    /// mutation, so every `Err` leaves the view unchanged. On success the
    /// layout is: `EntityEnter`s, then `Health`, then at most one `Turn`, then
    /// removals, each section ascending `NetId`.
    pub fn project_delta(
        &mut self,
        context: &ViewerContext<'_>,
    ) -> Result<StateProjection, InterestError> {
        self.validate(context)?;
        let visible = visible_rows(context);
        let entering: Vec<EntityKey> = visible
            .keys()
            .copied()
            .filter(|key| !self.live.contains_key(key))
            .collect();
        let (minted, next_id) = self.plan_mint(&entering)?;
        let ids = self.resolved_ids(&minted);
        let rows = rendered_rows(&visible, &ids);

        let mut ops: Vec<DeltaOp> = Vec::new();
        let mut enters = minted;
        enters.sort_by_key(|(_, net)| *net);
        for (_, net) in &enters {
            ops.push(DeltaOp::EntityEnter { entity: *net });
        }
        let enters_count = enters.len();

        let mut health_updates: Vec<(EntityKey, Option<HealthFact>)> =
            Vec::with_capacity(rows.len());
        for (net, fact) in &rows {
            let rendered = rendered_health(fact);
            let known = self
                .live
                .get(&fact.key)
                .and_then(|entry| entry.last_health.as_ref());
            let changed = rendered.as_ref() != known;
            if let Some(health) = &rendered {
                if changed || fact.resend_health {
                    ops.push(DeltaOp::Health {
                        entity: *net,
                        health: health.health,
                        max_health: health.max_health,
                        dead: health.dead,
                    });
                }
            }
            health_updates.push((fact.key, rendered));
        }

        let rendered_turn = rendered_turn(context, &ids, &visible);
        if let Some(turn) = &rendered_turn {
            if self.last_turn.as_ref() != Some(turn) || context.resend_turn {
                ops.push(DeltaOp::Turn {
                    active: turn.active,
                    round: turn.round,
                });
            }
        }

        let mut removed: Vec<(NetId, EntityKey)> = self
            .live
            .iter()
            .filter(|(key, _)| !visible.contains_key(key))
            .map(|(key, entry)| (entry.net, *key))
            .collect();
        removed.sort_by_key(|(net, _)| *net);
        for (net, key) in &removed {
            if context.despawned.contains(key) {
                ops.push(DeltaOp::Despawned { entity: *net });
            } else {
                ops.push(DeltaOp::EntityLeave { entity: *net });
            }
        }

        for (_, key) in &removed {
            if let Some(entry) = self.live.remove(key) {
                self.index.remove(&entry.net);
            }
        }
        for (key, net) in &enters {
            self.live.insert(
                *key,
                LiveEntry {
                    net: *net,
                    last_health: None,
                },
            );
            self.index.insert(*net, *key);
        }
        for (key, health) in health_updates {
            if let Some(entry) = self.live.get_mut(&key) {
                entry.last_health = health;
            }
        }
        self.last_turn = rendered_turn;
        self.next_id = next_id;
        if !ops.is_empty() {
            self.advance_cursor(context.tick);
        }

        Ok(StateProjection {
            ops,
            enters: enters_count,
            cursor: self.cursor,
        })
    }

    /// Projects one replacement snapshot for this viewer's context.
    ///
    /// Applies the epoch, bounds, duplicate and id-exhaustion checks, keeps
    /// live visible ids, retires removed keys with no op, and records exactly
    /// what the body says so the next delta emits only differences. Always
    /// advances the cursor on success, even for an empty body.
    pub fn project_snapshot(
        &mut self,
        context: &ViewerContext<'_>,
    ) -> Result<SnapshotProjection, InterestError> {
        self.validate(context)?;
        let visible = visible_rows(context);
        let entering: Vec<EntityKey> = visible
            .keys()
            .copied()
            .filter(|key| !self.live.contains_key(key))
            .collect();
        let (minted, next_id) = self.plan_mint(&entering)?;
        let ids = self.resolved_ids(&minted);
        let rows = rendered_rows(&visible, &ids);

        let entities: Vec<SnapshotEntity> = rows
            .iter()
            .map(|(net, fact)| SnapshotEntity {
                entity: *net,
                health: rendered_health(fact),
            })
            .collect();
        let turn = rendered_turn(context, &ids, &visible);
        let body = SnapshotBody {
            entities,
            turn: turn.clone(),
        };

        let retired: Vec<(EntityKey, NetId)> = self
            .live
            .iter()
            .filter(|(key, _)| !visible.contains_key(key))
            .map(|(key, entry)| (*key, entry.net))
            .collect();
        for (key, net) in retired {
            self.live.remove(&key);
            self.index.remove(&net);
        }
        for (key, net) in &minted {
            self.live.insert(
                *key,
                LiveEntry {
                    net: *net,
                    last_health: None,
                },
            );
            self.index.insert(*net, *key);
        }
        for (_, fact) in &rows {
            if let Some(entry) = self.live.get_mut(&fact.key) {
                entry.last_health = rendered_health(fact);
            }
        }
        self.last_turn = turn;
        self.next_id = next_id;
        self.advance_cursor(context.tick);

        Ok(SnapshotProjection {
            body,
            cursor: self.cursor,
        })
    }

    /// Runs the item-1 checks in order. Every failure changes nothing.
    fn validate(&self, context: &ViewerContext<'_>) -> Result<(), InterestError> {
        if context.epoch != self.epoch {
            return Err(InterestError::WrongEpoch);
        }
        if context.facts.len() > MAX_VISIBLE_ENTITIES
            || context.despawned.len() > MAX_VISIBLE_ENTITIES
        {
            return Err(InterestError::TooManyFacts);
        }
        let mut seen = BTreeSet::new();
        for fact in context.facts {
            if !seen.insert(fact.key) {
                return Err(InterestError::DuplicateFact);
            }
        }
        Ok(())
    }

    /// Mints ids for entering keys in ascending [`EntityKey`] order.
    ///
    /// `NetId`s run `1..=u64::MAX - 1`; the call fails whole when fewer ids
    /// remain than entering keys.
    fn plan_mint(
        &self,
        entering: &[EntityKey],
    ) -> Result<(Vec<(EntityKey, NetId)>, u64), InterestError> {
        if entering.len() as u64 > u64::MAX - self.next_id {
            return Err(InterestError::IdExhausted);
        }
        let mut next = self.next_id;
        let mut minted = Vec::with_capacity(entering.len());
        for key in entering {
            let Some(net) = NetId::new(next) else {
                return Err(InterestError::IdExhausted);
            };
            minted.push((*key, net));
            next += 1;
        }
        Ok((minted, next))
    }

    /// Live ids plus this page's minted ids, keyed by runtime identity.
    fn resolved_ids(&self, minted: &[(EntityKey, NetId)]) -> BTreeMap<EntityKey, NetId> {
        let mut ids: BTreeMap<EntityKey, NetId> = self
            .live
            .iter()
            .map(|(key, entry)| (*key, entry.net))
            .collect();
        ids.extend(minted.iter().copied());
        ids
    }

    /// Advances the cursor to `tick` with the next saturating revision.
    fn advance_cursor(&mut self, tick: u64) {
        self.cursor = ViewCursor {
            tick,
            revision: self.cursor.revision.saturating_add(1),
        };
    }
}

/// Collects the visible rows, keyed by ascending [`EntityKey`].
fn visible_rows<'a>(context: &ViewerContext<'a>) -> BTreeMap<EntityKey, &'a EntityFact> {
    context
        .facts
        .iter()
        .filter(|fact| fact.key.area == context.interest.area && fact.fields.presence)
        .map(|fact| (fact.key, fact))
        .collect()
}

/// Pairs visible rows with their post-minting ids, ascending `NetId`.
fn rendered_rows<'a>(
    visible: &BTreeMap<EntityKey, &'a EntityFact>,
    ids: &BTreeMap<EntityKey, NetId>,
) -> Vec<(NetId, &'a EntityFact)> {
    let mut rows: Vec<(NetId, &EntityFact)> = visible
        .iter()
        .map(|(key, fact)| (ids[key], *fact))
        .collect();
    rows.sort_by_key(|(net, _)| *net);
    rows
}

/// The row's health value when the health grant is set.
fn rendered_health(fact: &EntityFact) -> Option<HealthFact> {
    if fact.fields.health {
        fact.health.clone()
    } else {
        None
    }
}

/// The turn as this viewer may see it, with the post-minting active id.
fn rendered_turn(
    context: &ViewerContext<'_>,
    ids: &BTreeMap<EntityKey, NetId>,
    visible: &BTreeMap<EntityKey, &EntityFact>,
) -> Option<TurnFact> {
    if !context.turn_granted {
        return None;
    }
    let turn = context.turn?;
    if turn.area != context.interest.area {
        return None;
    }
    let active = turn.active.and_then(|entity| {
        let key = EntityKey {
            area: turn.area,
            entity,
        };
        if visible.contains_key(&key) {
            ids.get(&key).copied()
        } else {
            None
        }
    });
    Some(TurnFact {
        active,
        round: turn.round,
    })
}

#[cfg(test)]
impl InterestView {
    /// Test-only: reach an id counter `u64` arithmetic cannot practically walk.
    fn force_next_id(&mut self, next_id: u64) {
        self.next_id = next_id;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec_transfer::encode_body;
    use crate::protocol_transfer::MAX_ENCODED_BODY_BYTES;
    use crpg_core::GenerationalArena;

    fn epoch() -> SessionEpoch {
        SessionEpoch([7; 16])
    }

    fn area(seed: u128) -> Ulid {
        Ulid::from_u128(seed)
    }

    fn full_health() -> HealthFact {
        HealthFact {
            health: 3,
            max_health: 5,
            dead: false,
        }
    }

    fn context<'a>(
        epoch: SessionEpoch,
        area: Ulid,
        facts: &'a [EntityFact],
        despawned: &'a [EntityKey],
        turn: Option<TurnState>,
        turn_granted: bool,
    ) -> ViewerContext<'a> {
        ViewerContext {
            epoch,
            tick: 9,
            interest: InterestSet { area },
            facts,
            despawned,
            turn,
            turn_granted,
            resend_turn: false,
        }
    }

    #[test]
    fn i15_exhaustion_is_checked_before_mutation() {
        let mut arena = GenerationalArena::new();
        let one = EntityKey {
            area: area(1),
            entity: arena.insert(()),
        };
        let facts = [EntityFact {
            key: one,
            fields: FieldMask {
                presence: true,
                health: false,
            },
            health: None,
            resend_health: false,
        }];

        let mut view = InterestView::new(epoch());
        view.force_next_id(u64::MAX);
        let before = view.clone();
        let ctx = context(epoch(), area(1), &facts, &[], None, false);
        assert_eq!(view.project_delta(&ctx), Err(InterestError::IdExhausted));
        assert_eq!(view, before);
        assert_eq!(view.project_snapshot(&ctx), Err(InterestError::IdExhausted));
        assert_eq!(view, before);

        // No entering key still succeeds at the very top of the space.
        let empty = InterestView::new(epoch());
        let mut view = empty.clone();
        view.force_next_id(u64::MAX);
        let ctx = context(epoch(), area(1), &[], &[], None, false);
        let page = view.project_delta(&ctx).expect("empty page");
        assert!(page.ops.is_empty());
        assert_eq!(view.cursor(), ViewCursor::default());
        assert!(view
            .project_snapshot(&ctx)
            .expect("empty snapshot")
            .body
            .entities
            .is_empty());
    }

    #[test]
    fn i15_the_last_issuable_id_is_issued_once() {
        let mut arena = GenerationalArena::new();
        let one = arena.insert(());
        let two = arena.insert(());
        let facts = [
            EntityFact {
                key: EntityKey {
                    area: area(1),
                    entity: one,
                },
                fields: FieldMask {
                    presence: true,
                    health: false,
                },
                health: None,
                resend_health: false,
            },
            EntityFact {
                key: EntityKey {
                    area: area(1),
                    entity: two,
                },
                fields: FieldMask {
                    presence: true,
                    health: false,
                },
                health: None,
                resend_health: false,
            },
        ];

        let mut view = InterestView::new(epoch());
        view.force_next_id(u64::MAX - 1);
        let ctx = context(epoch(), area(1), &facts[..1], &[], None, false);
        let page = view.project_delta(&ctx).expect("one id left");
        assert_eq!(
            page.ops,
            vec![DeltaOp::EntityEnter {
                entity: NetId::new(u64::MAX - 1).unwrap(),
            }]
        );
        assert_eq!(view.next_id, u64::MAX);

        let mut view = InterestView::new(epoch());
        view.force_next_id(u64::MAX - 1);
        let before = view.clone();
        let ctx = context(epoch(), area(1), &facts, &[], None, false);
        assert_eq!(view.project_delta(&ctx), Err(InterestError::IdExhausted));
        assert_eq!(view, before);
    }

    #[test]
    fn id_exhausted_is_last_in_precedence() {
        let mut arena = GenerationalArena::new();
        let one = EntityKey {
            area: area(1),
            entity: arena.insert(()),
        };
        let row = |key| EntityFact {
            key,
            fields: FieldMask {
                presence: true,
                health: false,
            },
            health: None,
            resend_health: false,
        };

        // Each context below would exhaust the id space, so every earlier
        // check must win over `IdExhausted`.
        let enter = [row(one)];
        let duplicate = [row(one), row(one)];
        let too_many: Vec<EntityFact> = (0..=MAX_VISIBLE_ENTITIES)
            .map(|_| {
                row(EntityKey {
                    area: area(1),
                    entity: arena.insert(()),
                })
            })
            .collect();

        let valid = context(epoch(), area(1), &enter, &[], None, false);
        let wrong_epoch = context(SessionEpoch([8; 16]), area(1), &enter, &[], None, false);
        let overfull = context(epoch(), area(1), &too_many, &[], None, false);
        let repeated = context(epoch(), area(1), &duplicate, &[], None, false);

        let mut view = InterestView::new(epoch());
        view.force_next_id(u64::MAX);
        let before = view.clone();
        for (ctx, want) in [
            (&wrong_epoch, InterestError::WrongEpoch),
            (&overfull, InterestError::TooManyFacts),
            (&repeated, InterestError::DuplicateFact),
        ] {
            assert_eq!(view.project_delta(ctx), Err(want));
            assert_eq!(view, before, "{want:?} must not mutate through delta");
            assert_eq!(view.project_snapshot(ctx), Err(want));
            assert_eq!(view, before, "{want:?} must not mutate through snapshot");
        }

        // Control: the same exhausted view with a valid context does exhaust.
        assert_eq!(view.project_delta(&valid), Err(InterestError::IdExhausted));
        assert_eq!(view, before);
        assert_eq!(
            view.project_snapshot(&valid),
            Err(InterestError::IdExhausted)
        );
        assert_eq!(view, before);
    }

    #[test]
    fn i15_worst_case_body_fits_the_encoded_bound() {
        let mut arena = GenerationalArena::new();
        let mut facts = Vec::new();
        for _ in 0..MAX_VISIBLE_ENTITIES {
            facts.push(EntityFact {
                key: EntityKey {
                    area: area(1),
                    entity: arena.insert(()),
                },
                fields: FieldMask {
                    presence: true,
                    health: true,
                },
                health: Some(HealthFact {
                    health: u32::MAX,
                    max_health: u32::MAX,
                    dead: true,
                }),
                resend_health: false,
            });
        }
        let turn = TurnState {
            area: area(1),
            active: Some(facts[MAX_VISIBLE_ENTITIES - 1].key.entity),
            round: u64::MAX,
        };

        let mut view = InterestView::new(epoch());
        view.force_next_id(u64::MAX - 1024);
        let ctx = context(epoch(), area(1), &facts, &[], Some(turn), true);
        let snapshot = view.project_snapshot(&ctx).expect("worst-case snapshot");
        assert_eq!(snapshot.body.entities.len(), MAX_VISIBLE_ENTITIES);
        let bytes = encode_body(&snapshot.body).expect("worst-case body encodes");
        assert!(bytes.len() <= MAX_ENCODED_BODY_BYTES);
        assert_eq!(bytes.len(), MAX_ENCODED_BODY_BYTES);
    }

    #[test]
    fn smoke_delta_order_and_retirement() {
        let mut arena = GenerationalArena::new();
        let a = EntityKey {
            area: area(1),
            entity: arena.insert(()),
        };
        let b = EntityKey {
            area: area(1),
            entity: arena.insert(()),
        };
        let foreign = EntityKey {
            area: area(2),
            entity: arena.insert(()),
        };
        let row = |key| EntityFact {
            key,
            fields: FieldMask {
                presence: true,
                health: true,
            },
            health: Some(full_health()),
            resend_health: false,
        };
        // Input order is shuffled and a foreign row is mixed in: ascending
        // EntityKey is a then b, so a takes id 1 and b takes id 2.
        let facts = [row(b), row(foreign), row(a)];
        let turn = TurnState {
            area: area(1),
            active: Some(a.entity),
            round: 2,
        };
        let mut view = InterestView::new(epoch());
        let ctx = context(epoch(), area(1), &facts, &[], Some(turn), true);
        let page = view.project_delta(&ctx).expect("fresh page");
        assert_eq!(
            page.ops,
            vec![
                DeltaOp::EntityEnter {
                    entity: NetId::new(1).unwrap()
                },
                DeltaOp::EntityEnter {
                    entity: NetId::new(2).unwrap()
                },
                DeltaOp::Health {
                    entity: NetId::new(1).unwrap(),
                    health: 3,
                    max_health: 5,
                    dead: false,
                },
                DeltaOp::Health {
                    entity: NetId::new(2).unwrap(),
                    health: 3,
                    max_health: 5,
                    dead: false,
                },
                DeltaOp::Turn {
                    active: Some(NetId::new(1).unwrap()),
                    round: 2,
                },
            ]
        );
        assert_eq!(page.enters, 2);
        assert_eq!(view.len(), 2);
        assert_eq!(view.net_id(a), Some(NetId::new(1).unwrap()));
        assert_eq!(view.net_id(b), Some(NetId::new(2).unwrap()));
        assert_eq!(view.net_id(foreign), None);

        // Unchanged page is empty and leaves the cursor alone.
        let cursor = page.cursor;
        let again = view.project_delta(&ctx).expect("unchanged page");
        assert!(again.ops.is_empty());
        assert_eq!(again.cursor, cursor);

        // b leaves: one removal, its mapping retires, the other id stays.
        let leaving = [row(a)];
        let page = view
            .project_delta(&context(epoch(), area(1), &leaving, &[], Some(turn), true))
            .expect("leave page");
        assert_eq!(
            page.ops,
            vec![DeltaOp::EntityLeave {
                entity: NetId::new(2).unwrap()
            }]
        );
        assert_eq!(view.entity(NetId::new(2).unwrap()), None);
        assert_eq!(view.net_id(b), None);
        assert_eq!(view.len(), 1);
    }

    #[test]
    fn smoke_snapshot_replaces_and_retires() {
        let mut arena = GenerationalArena::new();
        let a = EntityKey {
            area: area(1),
            entity: arena.insert(()),
        };
        let b = EntityKey {
            area: area(1),
            entity: arena.insert(()),
        };
        let row = |key| EntityFact {
            key,
            fields: FieldMask {
                presence: true,
                health: true,
            },
            health: Some(full_health()),
            resend_health: false,
        };
        let mut view = InterestView::new(epoch());
        let entering = [row(a), row(b)];
        let ctx = context(epoch(), area(1), &entering, &[], None, false);
        view.project_delta(&ctx).expect("enter both");

        let id_a = view.net_id(a).expect("a");
        let id_b = view.net_id(b).expect("b");
        let staying = [row(b)];
        let ctx = context(epoch(), area(1), &staying, &[], None, false);
        let snapshot = view.project_snapshot(&ctx).expect("snapshot");
        assert_eq!(
            snapshot.body.entities,
            vec![SnapshotEntity {
                entity: id_b,
                health: Some(full_health()),
            }]
        );
        assert_eq!(view.net_id(a), None);
        assert_eq!(view.net_id(b), Some(id_b));
        assert_ne!(id_a, id_b);

        // Cutover: the next delta re-emits nothing unchanged.
        let page = view.project_delta(&ctx).expect("after snapshot");
        assert!(page.ops.is_empty());
        assert_eq!(page.cursor, snapshot.cursor);
        assert_eq!(view.cursor().revision, 2);
    }
}

//! Resource pools with caller-clocked refresh.
//!
//! A [`ResourcePool`] is plain rules state: a maximum, a current balance,
//! a refresh trigger, and an optional refill amount. Pools do not subscribe
//! to hooks and own no timeline; the host delivers each logical refresh event
//! once and selects the appropriate entity's pools. Spending more than the
//! balance fails without mutation, and a matching refresh refills to maximum
//! or by the pool's refill amount. The maximum can be changed with
//! [`ResourcePool::set_max`], which clamps the balance. There is no
//! cross-pool atomic cost API, no implicit clock advance, and no fractional
//! or negative counts.

use core::num::NonZeroU32;

use crpg_core::{Tick, Ulid};

use crate::error::{RulesError, RulesErrorCode};

/// The authored identity of one resource pool.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(transparent)]
pub struct ResourcePoolId(pub Ulid);

/// The authored identity of one rest capable of refreshing pools.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(transparent)]
pub struct RestId(pub Ulid);

/// When a pool refills.
///
/// Turn and round triggers match their event kinds, rest triggers match one
/// exact rest, tick triggers match positive ticks divisible by their period,
/// and `Never` matches nothing. A tick period must be nonzero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum RefreshTrigger {
    /// Refills on every turn start.
    OnTurnStart,
    /// Refills on every round start.
    OnRoundStart,
    /// Refills on the matching rest only.
    OnRest(RestId),
    /// Refills on positive ticks divisible by the period.
    OnTick(u64),
    /// Never refills.
    Never,
}

/// One host-delivered refresh event.
///
/// Simulation tick zero is the origin but never itself a refresh. Events
/// carry no history. For a full-refill pool, delivering the same logical event
/// twice refills twice only when spending happened between the deliveries. A
/// partial-refill pool gains its refill amount on every matching delivery
/// until it is full, so duplicate deliveries are not idempotent for it and
/// the host must deliver each logical event exactly once.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum RefreshEvent {
    /// A turn started.
    TurnStart,
    /// A round started.
    RoundStart,
    /// A rest completed.
    Rest(RestId),
    /// A simulation tick elapsed.
    Tick(Tick),
}

/// One caller-clocked resource pool.
///
/// A pool is rules state, not a permitted nested hook payload. All counts
/// are whole and nonnegative; the maximum may be zero. The balance never
/// exceeds the maximum.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourcePool {
    id: ResourcePoolId,
    max: u32,
    current: u32,
    refresh: RefreshTrigger,
    /// `None` refills to maximum; `Some(n)` adds `n`, saturating at `max`.
    refill: Option<NonZeroU32>,
}

impl ResourcePool {
    /// Builds a full-refill pool after validating its construction state.
    ///
    /// Equivalent to [`ResourcePool::with_refill`] with no refill amount.
    /// Fails with [`RulesErrorCode::InvalidResource`] at
    /// `/resource/current` when `current` exceeds `max`, and at
    /// `/resource/refresh` for a zero tick period.
    pub fn new(
        id: ResourcePoolId,
        max: u32,
        current: u32,
        refresh: RefreshTrigger,
    ) -> Result<Self, RulesError> {
        Self::with_refill(id, max, current, refresh, None)
    }

    /// Builds a pool with an optional refill amount after validating its
    /// construction state.
    ///
    /// `None` refills to maximum on a matching refresh; `Some(n)` adds `n`,
    /// saturating at `max`. The refill amount adds no failure: it may exceed
    /// `max`, and it may accompany a trigger that never matches. Fails with
    /// [`RulesErrorCode::InvalidResource`] at `/resource/current` when
    /// `current` exceeds `max`, then at `/resource/refresh` for a zero tick
    /// period.
    pub fn with_refill(
        id: ResourcePoolId,
        max: u32,
        current: u32,
        refresh: RefreshTrigger,
        refill: Option<NonZeroU32>,
    ) -> Result<Self, RulesError> {
        if current > max {
            return Err(RulesError::at(
                RulesErrorCode::InvalidResource,
                String::from("/resource/current"),
            ));
        }
        if matches!(refresh, RefreshTrigger::OnTick(0)) {
            return Err(RulesError::at(
                RulesErrorCode::InvalidResource,
                String::from("/resource/refresh"),
            ));
        }
        Ok(Self {
            id,
            max,
            current,
            refresh,
            refill,
        })
    }

    /// Returns the pool identity.
    #[must_use]
    pub const fn id(&self) -> ResourcePoolId {
        self.id
    }

    /// Returns the maximum balance.
    #[must_use]
    pub const fn max(&self) -> u32 {
        self.max
    }

    /// Returns the current balance.
    #[must_use]
    pub const fn current(&self) -> u32 {
        self.current
    }

    /// Returns the refresh trigger.
    #[must_use]
    pub const fn refresh_trigger(&self) -> RefreshTrigger {
        self.refresh
    }

    /// Returns the refill amount; `None` means a matching refresh refills to
    /// maximum.
    #[must_use]
    pub const fn refill(&self) -> Option<NonZeroU32> {
        self.refill
    }

    /// Reports whether the balance covers `amount`.
    #[must_use]
    pub const fn can_afford(&self, amount: u32) -> bool {
        self.current >= amount
    }

    /// Spends `amount` from the balance.
    ///
    /// Spending zero succeeds. Insufficient funds fail with
    /// [`RulesErrorCode::InsufficientResource`] at `/resource/amount`
    /// without mutation.
    pub fn spend(&mut self, amount: u32) -> Result<(), RulesError> {
        if amount > self.current {
            return Err(RulesError::at(
                RulesErrorCode::InsufficientResource,
                String::from("/resource/amount"),
            ));
        }
        self.current -= amount;
        Ok(())
    }

    /// Applies one host-delivered refresh event.
    ///
    /// On a matching event a full-refill pool resets the balance to maximum,
    /// and a pool with refill amount `n` sets the balance to
    /// `min(max, current + n)`. Returns true exactly when the balance
    /// changed; with no match nothing changes and the result is false. A
    /// full-refill pool therefore ignores an immediate repeated event, while
    /// a partial-refill pool gains `n` on every matching delivery until it is
    /// full, so the host must deliver each logical event exactly once. Tick
    /// `n` matches positive ticks divisible by `n`; simulation tick zero
    /// never refreshes.
    pub fn refresh(&mut self, event: RefreshEvent) -> bool {
        let matched = match (self.refresh, event) {
            (RefreshTrigger::OnTurnStart, RefreshEvent::TurnStart) => true,
            (RefreshTrigger::OnRoundStart, RefreshEvent::RoundStart) => true,
            (RefreshTrigger::OnRest(expected), RefreshEvent::Rest(actual)) => expected == actual,
            (RefreshTrigger::OnTick(period), RefreshEvent::Tick(tick)) => {
                let count = tick.get();
                count > 0 && count % period == 0
            }
            _ => false,
        };
        if !matched {
            return false;
        }
        let refilled = match self.refill {
            None => self.max,
            Some(amount) => self.max.min(self.current.saturating_add(amount.get())),
        };
        if refilled == self.current {
            return false;
        }
        self.current = refilled;
        true
    }

    /// Sets the maximum balance, clamping the current balance to it.
    ///
    /// Every `u32` is a valid maximum, zero included. Lowering the maximum
    /// below the balance lowers the balance to it; raising the maximum never
    /// raises the balance. The refill amount and trigger are untouched.
    /// Returns true exactly when the maximum changed, which is the only case
    /// in which the balance can change.
    pub fn set_max(&mut self, max: u32) -> bool {
        if max == self.max {
            return false;
        }
        self.max = max;
        self.current = self.current.min(max);
        true
    }
}

impl serde::Serialize for ResourcePool {
    /// Serializes the `{id, max, current, refresh}` wire object, then
    /// `refill` only when present.
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let len = if self.refill.is_some() { 5 } else { 4 };
        let mut shape = serializer.serialize_struct("ResourcePool", len)?;
        shape.serialize_field("id", &self.id)?;
        shape.serialize_field("max", &self.max)?;
        shape.serialize_field("current", &self.current)?;
        shape.serialize_field("refresh", &self.refresh)?;
        if let Some(refill) = self.refill {
            shape.serialize_field("refill", &refill)?;
        }
        shape.end()
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ResourcePoolRepr {
    id: ResourcePoolId,
    max: u32,
    current: u32,
    refresh: RefreshTrigger,
    #[serde(default, deserialize_with = "present_refill")]
    refill: Option<NonZeroU32>,
}

/// Decodes a present `refill` key as a nonzero `u32`, so `null` and zero are
/// rejected rather than read as an absent amount.
fn present_refill<'de, D>(deserializer: D) -> Result<Option<NonZeroU32>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;
    NonZeroU32::deserialize(deserializer).map(Some)
}

impl<'de> serde::Deserialize<'de> for ResourcePool {
    /// Decodes through [`ResourcePool::with_refill`], rejecting invalid
    /// state.
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let repr = ResourcePoolRepr::deserialize(deserializer)?;
        Self::with_refill(repr.id, repr.max, repr.current, repr.refresh, repr.refill)
            .map_err(serde::de::Error::custom)
    }
}

/// One buffered adjacent `value` field: the wire shapes only ever carry
/// ULID strings and tick counts.
#[derive(Debug)]
enum WireValue {
    Text(String),
    Count(u64),
}

impl<'de> serde::Deserialize<'de> for WireValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct WireVisitor;
        impl serde::de::Visitor<'_> for WireVisitor {
            type Value = WireValue;
            fn expecting(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                formatter.write_str("a ULID string or a tick count")
            }
            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<WireValue, E> {
                Ok(WireValue::Text(value.to_owned()))
            }
            fn visit_string<E: serde::de::Error>(self, value: String) -> Result<WireValue, E> {
                Ok(WireValue::Text(value))
            }
            fn visit_u64<E: serde::de::Error>(self, value: u64) -> Result<WireValue, E> {
                Ok(WireValue::Count(value))
            }
            fn visit_i64<E: serde::de::Error>(self, value: i64) -> Result<WireValue, E> {
                u64::try_from(value).map(WireValue::Count).map_err(|_| {
                    E::invalid_value(serde::de::Unexpected::Signed(value), &"a nonnegative count")
                })
            }
        }
        deserializer.deserialize_any(WireVisitor)
    }
}

/// Collects the `type` discriminator and optional `value` of one adjacent
/// `type`/`value` object, rejecting unknown, duplicated, and missing fields.
fn collect_tagged<'de, A>(mut map: A) -> Result<(String, Option<WireValue>), A::Error>
where
    A: serde::de::MapAccess<'de>,
{
    use serde::de::Error;
    let mut kind: Option<String> = None;
    let mut value: Option<WireValue> = None;
    while let Some(key) = map.next_key::<String>()? {
        match key.as_str() {
            "type" => {
                if kind.is_some() {
                    return Err(A::Error::duplicate_field("type"));
                }
                kind = Some(map.next_value()?);
            }
            "value" => {
                if value.is_some() {
                    return Err(A::Error::duplicate_field("value"));
                }
                value = Some(map.next_value()?);
            }
            unknown => return Err(A::Error::unknown_field(unknown, &["type", "value"])),
        }
    }
    let kind = kind.ok_or_else(|| A::Error::missing_field("type"))?;
    Ok((kind, value))
}

fn reject_unit_value<E: serde::de::Error>(value: Option<WireValue>) -> Result<(), E> {
    if value.is_some() {
        return Err(E::unknown_field("value", &["type"]));
    }
    Ok(())
}

impl<'de> serde::Deserialize<'de> for RefreshTrigger {
    /// Decodes the adjacent `type`/`value` shape with unknown fields
    /// rejected, including on unit variants.
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct TriggerVisitor;
        impl<'de> serde::de::Visitor<'de> for TriggerVisitor {
            type Value = RefreshTrigger;
            fn expecting(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                formatter.write_str("a refresh trigger object")
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                map: A,
            ) -> Result<RefreshTrigger, A::Error> {
                use serde::de::Error;
                let (kind, value) = collect_tagged(map)?;
                match kind.as_str() {
                    "on_turn_start" => {
                        reject_unit_value(value)?;
                        Ok(RefreshTrigger::OnTurnStart)
                    }
                    "on_round_start" => {
                        reject_unit_value(value)?;
                        Ok(RefreshTrigger::OnRoundStart)
                    }
                    "on_rest" => match value {
                        Some(WireValue::Text(text)) => text
                            .parse::<Ulid>()
                            .map(|id| RefreshTrigger::OnRest(RestId(id)))
                            .map_err(A::Error::custom),
                        Some(WireValue::Count(count)) => Err(A::Error::invalid_value(
                            serde::de::Unexpected::Unsigned(count),
                            &"a rest ULID string",
                        )),
                        None => Err(A::Error::missing_field("value")),
                    },
                    "on_tick" => match value {
                        Some(WireValue::Count(period)) if period == 0 => {
                            Err(A::Error::invalid_value(
                                serde::de::Unexpected::Unsigned(period),
                                &"a nonzero tick period",
                            ))
                        }
                        Some(WireValue::Count(period)) => Ok(RefreshTrigger::OnTick(period)),
                        Some(WireValue::Text(_)) => Err(A::Error::invalid_type(
                            serde::de::Unexpected::Str("string"),
                            &"a tick period",
                        )),
                        None => Err(A::Error::missing_field("value")),
                    },
                    "never" => {
                        reject_unit_value(value)?;
                        Ok(RefreshTrigger::Never)
                    }
                    unknown => Err(A::Error::unknown_variant(
                        unknown,
                        &[
                            "on_turn_start",
                            "on_round_start",
                            "on_rest",
                            "on_tick",
                            "never",
                        ],
                    )),
                }
            }
        }
        deserializer.deserialize_map(TriggerVisitor)
    }
}

impl<'de> serde::Deserialize<'de> for RefreshEvent {
    /// Decodes the adjacent `type`/`value` shape with unknown fields
    /// rejected, including on unit variants.
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct EventVisitor;
        impl<'de> serde::de::Visitor<'de> for EventVisitor {
            type Value = RefreshEvent;
            fn expecting(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                formatter.write_str("a refresh event object")
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                map: A,
            ) -> Result<RefreshEvent, A::Error> {
                use serde::de::Error;
                let (kind, value) = collect_tagged(map)?;
                match kind.as_str() {
                    "turn_start" => {
                        reject_unit_value(value)?;
                        Ok(RefreshEvent::TurnStart)
                    }
                    "round_start" => {
                        reject_unit_value(value)?;
                        Ok(RefreshEvent::RoundStart)
                    }
                    "rest" => match value {
                        Some(WireValue::Text(text)) => text
                            .parse::<Ulid>()
                            .map(|id| RefreshEvent::Rest(RestId(id)))
                            .map_err(A::Error::custom),
                        Some(WireValue::Count(count)) => Err(A::Error::invalid_value(
                            serde::de::Unexpected::Unsigned(count),
                            &"a rest ULID string",
                        )),
                        None => Err(A::Error::missing_field("value")),
                    },
                    "tick" => match value {
                        Some(WireValue::Count(count)) => Ok(RefreshEvent::Tick(Tick::new(count))),
                        Some(WireValue::Text(_)) => Err(A::Error::invalid_type(
                            serde::de::Unexpected::Str("string"),
                            &"a tick count",
                        )),
                        None => Err(A::Error::missing_field("value")),
                    },
                    unknown => Err(A::Error::unknown_variant(
                        unknown,
                        &["turn_start", "round_start", "rest", "tick"],
                    )),
                }
            }
        }
        deserializer.deserialize_map(EventVisitor)
    }
}

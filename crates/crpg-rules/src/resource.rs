//! Resource pools with caller-clocked refresh.
//!
//! A [`ResourcePool`] is plain rules state: a maximum, a current balance,
//! and a refresh trigger. Pools do not subscribe to hooks and own no
//! timeline; the host delivers each logical refresh event once and selects
//! the appropriate entity's pools. Spending more than the balance fails
//! without mutation, and a matching refresh refills to maximum. There is no
//! cross-pool atomic cost API, no implicit clock advance, no maximum
//! adjustment API, and no fractional or negative counts.

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

/// When a pool refills to maximum.
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
/// carry no history; delivering the same logical event twice refills twice
/// only when spending happened between the deliveries.
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
/// are whole and nonnegative; the maximum may be zero.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourcePool {
    id: ResourcePoolId,
    max: u32,
    current: u32,
    refresh: RefreshTrigger,
}

impl ResourcePool {
    /// Builds a pool after validating its construction state.
    ///
    /// Fails with [`RulesErrorCode::InvalidResource`] at
    /// `/resource/current` when `current` exceeds `max`, and at
    /// `/resource/refresh` for a zero tick period.
    pub fn new(
        id: ResourcePoolId,
        max: u32,
        current: u32,
        refresh: RefreshTrigger,
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
    /// A matching event resets the balance to maximum. Returns true exactly
    /// when the balance changed, so an immediate repeated event is
    /// idempotent while spending between duplicates allows another refill.
    /// Tick `n` matches positive ticks divisible by `n`; simulation tick
    /// zero never refreshes.
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
        if matched && self.current != self.max {
            self.current = self.max;
            true
        } else {
            false
        }
    }
}

impl serde::Serialize for ResourcePool {
    /// Serializes the `{id, max, current, refresh}` wire object.
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut shape = serializer.serialize_struct("ResourcePool", 4)?;
        shape.serialize_field("id", &self.id)?;
        shape.serialize_field("max", &self.max)?;
        shape.serialize_field("current", &self.current)?;
        shape.serialize_field("refresh", &self.refresh)?;
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
}

impl<'de> serde::Deserialize<'de> for ResourcePool {
    /// Decodes through [`ResourcePool::new`], rejecting invalid state.
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let repr = ResourcePoolRepr::deserialize(deserializer)?;
        Self::new(repr.id, repr.max, repr.current, repr.refresh).map_err(serde::de::Error::custom)
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

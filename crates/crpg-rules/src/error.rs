//! Owned structured errors for the rules kernel.
//!
//! Every fallible entry point reports a [`RulesError`]: a machine-checkable
//! [`RulesErrorCode`], an owned diagnostic path in [`RulesError::location`],
//! and the structured cycle members in [`RulesError::cycle`] for
//! [`RulesErrorCode::Cycle`] only. Paths are runtime diagnostics, never
//! persisted identities.

use core::fmt;

use crpg_core::StatId;

/// Machine-checkable failure code for every rules-kernel domain error.
///
/// The code selects the failure family; [`RulesError::location`] selects the
/// position. See the task contract for which code each validation phase
/// reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RulesErrorCode {
    /// An empty symbolic string where a nonempty name is required.
    InvalidName,
    /// A stated kernel bound from the task contract was exceeded.
    LimitExceeded,
    /// Two stat definitions share one stat identity.
    DuplicateStat,
    /// Two modifiers in one query context share one modifier identity.
    DuplicateModifier,
    /// Two tags in one tag set share one tag identity.
    DuplicateTag,
    /// An unknown enum domain or member, or inconsistent domains for one id.
    InvalidEnum,
    /// A reference to a stat with no definition.
    UnknownStat,
    /// A modifier names a stacking policy that is not configured.
    UnknownModifierType,
    /// A persisted symbolic name has no handle in the supplied interner.
    UnresolvedHandle,
    /// A value, expression, or operand has the wrong kind for its position.
    TypeMismatch,
    /// An operation is not supported on the target kind, or a policy forbids
    /// it, or a policy-required name is absent.
    InvalidOperation,
    /// A clamp interval has its minimum above its maximum.
    InvalidClamp,
    /// A stored base value exists for a stat declared as derived.
    DerivedBase,
    /// A non-derived stat needed by the query has no stored base value.
    MissingBase,
    /// The queried entity is not the context entity.
    EntityMismatch,
    /// The derived-stat dependency graph contains a directed cycle.
    Cycle,
    /// An expression divides by zero.
    DivisionByZero,
    /// A dice expression or its count/sides/selection fails validation.
    InvalidDice,
    /// A named RNG stream name is empty (over-long names report
    /// [`RulesErrorCode::LimitExceeded`] at the same location).
    InvalidStream,
    /// An outcome table's bands or natural rules fail validation.
    InvalidOutcomeTable,
    /// A resolution request or its roll shape fails validation.
    InvalidResolution,
    /// A query targets a modifier-target kind it does not accept.
    InvalidTarget,
    /// A resolution participant has no entity view.
    MissingEntity,
    /// A resource pool's construction state fails validation.
    InvalidResource,
    /// A resource spend exceeds the available balance.
    InsufficientResource,
}

/// An owned, structured rules-kernel failure.
///
/// `Display` renders exactly `<Code> at <location>`, where `Code` is the
/// [`RulesErrorCode`] variant spelling. Only [`RulesErrorCode::Cycle`] carries
/// a nonempty [`RulesError::cycle`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RulesError {
    /// The failure family.
    pub code: RulesErrorCode,
    /// An owned diagnostic path, never a persisted identity.
    pub location: String,
    /// The reported cycle with its repeated closing vertex, rotated to the
    /// smallest stat id; empty for every other code.
    pub cycle: Vec<StatId>,
}

impl RulesError {
    /// Builds an error with an empty cycle list.
    pub(crate) fn at(code: RulesErrorCode, location: String) -> Self {
        Self {
            code,
            location,
            cycle: Vec::new(),
        }
    }

    /// Builds a cycle error with its structured member path.
    pub(crate) fn cycle(location: String, cycle: Vec<StatId>) -> Self {
        Self {
            code: RulesErrorCode::Cycle,
            location,
            cycle,
        }
    }
}

impl fmt::Display for RulesError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?} at {}", self.code, self.location)
    }
}

impl std::error::Error for RulesError {}

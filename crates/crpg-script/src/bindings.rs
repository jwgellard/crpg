//! Immutable trusted action bindings and synchronous transactional dispatch
//! (T029b, D17).
//!
//! An [`ActionBindings`] table pairs every declaration of one T029a
//! [`ActionSignatureStore`] with exactly one trusted Rust [`ActionHandler`].
//! It is assembled once at trusted startup by [`ActionBindings::new`], which
//! fails closed on any bundle, count, duplicate, unknown, missing or
//! signature disagreement before anything can run. After construction the
//! table is immutable: there is no registration, replacement or removal, and
//! authored input (an [`ActionCall`]) can only *select* an already bound
//! handler by its exact symbolic id — it never installs code.
//!
//! [`ActionBindings::dispatch`] runs one invocation synchronously:
//! T029a call validation, T028 actor validation, exactly one handler call
//! with a read-only [`InvocationContext`], the [`MAX_BINDING_PROPOSALS`] and
//! actor-substitution checks, then every returned [`CombatAction`] proposal
//! applied in order through the public `perform_action` controller on a
//! staged clone of the [`World`]. The staged world is published only when
//! every proposal succeeds; any rejection leaves the caller's world —
//! entities, resources, RNG, events and turn state — exactly as it was.
//!
//! Handlers are **trusted** startup input, not a sandbox: a function pointer
//! is not preempted or instruction-metered, and a panicking handler is a bug
//! rather than a budget error. The proposal bound limits applied simulation
//! operations only. There is no wait, yield or continuation in this slice: a
//! handler that would wait returns [`HandlerError::UnsupportedWait`], which is
//! a real error, never a saved continuation.

use std::collections::BTreeMap;
use std::fmt;

use crpg_core::EntityId;
use crpg_data::{
    ActionBundleIdentity, ActionCall, ActionSignature, ActionSignatureStore, SignatureError,
    MAX_ACTION_SIGNATURES,
};
use crpg_sim::{perform_action, validate_action, ActionOutcome, CombatAction, CombatError, World};

/// Maximum combat proposals one handler invocation may return.
pub const MAX_BINDING_PROPOSALS: usize = 32;

/// A trusted Rust implementation of one declared action.
///
/// It must construct its proposals deterministically from its validated call
/// and read-only context alone: no globals, I/O, entropy or interior host
/// mutation, and bounded work before returning. Proposals are applied by
/// [`ActionBindings::dispatch`], never by the handler.
pub type ActionHandler =
    for<'a> fn(&InvocationContext<'a>, &ActionCall) -> Result<Vec<CombatAction>, HandlerError>;

/// The read-only view one handler invocation receives.
///
/// Only [`ActionBindings::dispatch`] constructs it. It exposes a shared
/// reference to the authoritative [`World`] and the validated invocation
/// actor; it offers no world mutator, RNG access or host capability.
///
/// Reading the world compiles:
///
/// ```
/// use crpg_data::ActionCall;
/// use crpg_script::{HandlerError, InvocationContext};
/// use crpg_sim::CombatAction;
///
/// fn observe(
///     context: &InvocationContext<'_>,
///     _call: &ActionCall,
/// ) -> Result<Vec<CombatAction>, HandlerError> {
///     let _live = context.world().len();
///     Ok(vec![CombatAction::EndTurn { actor: context.actor() }])
/// }
/// ```
///
/// Mutating the world through the context does not:
///
/// ```compile_fail,E0596
/// use crpg_data::ActionCall;
/// use crpg_script::{HandlerError, InvocationContext};
/// use crpg_sim::{CombatAction, EntityMeta};
///
/// fn grab(
///     context: &InvocationContext<'_>,
///     _call: &ActionCall,
/// ) -> Result<Vec<CombatAction>, HandlerError> {
///     context.world().spawn(EntityMeta {});
///     Ok(Vec::new())
/// }
/// ```
///
/// Nor does drawing from the world's RNG:
///
/// ```compile_fail,E0596
/// use crpg_data::ActionCall;
/// use crpg_script::{HandlerError, InvocationContext};
/// use crpg_sim::CombatAction;
///
/// fn draw(
///     context: &InvocationContext<'_>,
///     _call: &ActionCall,
/// ) -> Result<Vec<CombatAction>, HandlerError> {
///     let _rng = context.world().rng_mut();
///     Ok(Vec::new())
/// }
/// ```
///
/// Nor can a caller forge a context around a mutable world:
///
/// ```compile_fail,E0451
/// use crpg_core::EntityId;
/// use crpg_script::InvocationContext;
/// use crpg_sim::World;
///
/// fn forge<'a>(world: &'a World, actor: EntityId) -> InvocationContext<'a> {
///     InvocationContext { world, actor }
/// }
/// ```
pub struct InvocationContext<'a> {
    world: &'a World,
    actor: EntityId,
}

impl<'a> InvocationContext<'a> {
    /// The authoritative world as it stood before this invocation, read-only.
    pub fn world(&self) -> &'a World {
        self.world
    }

    /// The invocation actor, already validated as holding the active turn.
    pub fn actor(&self) -> EntityId {
        self.actor
    }
}

/// One trusted startup pairing of a declaration with its implementation.
///
/// `signature` must equal the store's declaration exactly (action id,
/// parameter names, categories, required flags and parameter order).
pub struct ActionBinding {
    /// The declaration this handler implements.
    pub signature: ActionSignature,
    /// The trusted implementation.
    pub handler: ActionHandler,
}

impl fmt::Debug for ActionBinding {
    /// Shows the signature only; function addresses are not semantic.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ActionBinding")
            .field("signature", &self.signature)
            .finish_non_exhaustive()
    }
}

/// An immutable, validated table of trusted action bindings.
///
/// Constructed only by [`ActionBindings::new`]; there is no mutator, no
/// late registration, no hot replacement and no deserialization.
pub struct ActionBindings {
    declarations: ActionSignatureStore,
    handlers: BTreeMap<String, ActionHandler>,
}

impl fmt::Debug for ActionBindings {
    /// Shows the bundle identity and bound ids; never function addresses.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ActionBindings")
            .field("identity", self.declarations.identity())
            .field("actions", &self.handlers.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

/// Why a trusted handler produced no proposals.
///
/// Carries no handler-chosen text, so nothing arbitrary reaches a wire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandlerError {
    /// The handler declined this invocation.
    Refused,
    /// The handler would need to wait or yield, which this synchronous slice
    /// does not support. Never a saved continuation or a success.
    UnsupportedWait,
}

impl fmt::Display for HandlerError {
    /// Renders the variant name.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused => f.write_str("Refused"),
            Self::UnsupportedWait => f.write_str("UnsupportedWait"),
        }
    }
}

impl std::error::Error for HandlerError {}

/// Every startup or dispatch failure, in pinned precedence.
///
/// Startup ([`ActionBindings::new`]) checks `BundleMismatch`,
/// `TooManyBindings`, `DuplicateBinding`, `UnknownBinding`, `MissingBinding`,
/// `SignatureMismatch` in that order. Dispatch checks `Call`, `Actor`,
/// `Handler`, `ProposalLimit`, `ForeignActor`, `Apply` in that order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BindingError {
    /// The supplied identity differs from the declarations' identity.
    BundleMismatch,
    /// More bindings than [`MAX_ACTION_SIGNATURES`] were supplied.
    TooManyBindings,
    /// The lexically first action id bound more than once.
    DuplicateBinding {
        /// The duplicated action id.
        action_id: String,
    },
    /// The lexically first bound action id with no declaration.
    UnknownBinding {
        /// The undeclared action id.
        action_id: String,
    },
    /// The lexically first declared action id with no binding.
    MissingBinding {
        /// The unbound action id.
        action_id: String,
    },
    /// The lexically first binding whose signature differs from its
    /// declaration.
    SignatureMismatch {
        /// The mismatched action id.
        action_id: String,
    },
    /// T029a rejected the identity or call; no handler ran.
    Call(SignatureError),
    /// T028 rejected the invocation actor; no handler ran.
    Actor(CombatError),
    /// The handler returned an error; nothing was applied.
    Handler(HandlerError),
    /// The handler returned more than [`MAX_BINDING_PROPOSALS`] proposals;
    /// nothing was applied.
    ProposalLimit,
    /// The proposal at this zero-based index names an actor other than the
    /// invocation actor; nothing was applied.
    ForeignActor {
        /// The first foreign proposal.
        index: usize,
    },
    /// The proposal at this zero-based index was rejected against the staged
    /// world; every staged change was discarded.
    Apply {
        /// The first rejected proposal.
        index: usize,
        /// Why the simulation rejected it.
        error: CombatError,
    },
}

impl fmt::Display for BindingError {
    /// `Call`, `Actor` and `Handler` render their wrapped error unchanged;
    /// `Apply` renders `Apply at proposals/<index>`; every other variant
    /// renders `<VariantName> at bindings` without fields.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Call(error) => fmt::Display::fmt(error, f),
            Self::Actor(error) => fmt::Display::fmt(error, f),
            Self::Handler(error) => fmt::Display::fmt(error, f),
            Self::Apply { index, .. } => write!(f, "Apply at proposals/{index}"),
            Self::BundleMismatch => f.write_str("BundleMismatch at bindings"),
            Self::TooManyBindings => f.write_str("TooManyBindings at bindings"),
            Self::DuplicateBinding { .. } => f.write_str("DuplicateBinding at bindings"),
            Self::UnknownBinding { .. } => f.write_str("UnknownBinding at bindings"),
            Self::MissingBinding { .. } => f.write_str("MissingBinding at bindings"),
            Self::SignatureMismatch { .. } => f.write_str("SignatureMismatch at bindings"),
            Self::ProposalLimit => f.write_str("ProposalLimit at bindings"),
            Self::ForeignActor { .. } => f.write_str("ForeignActor at bindings"),
        }
    }
}

impl std::error::Error for BindingError {
    /// `Call`, `Actor` and `Handler` delegate to their wrapped error's own
    /// source; `Apply` exposes its [`CombatError`]; nothing otherwise.
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Call(error) => error.source(),
            Self::Actor(error) => error.source(),
            Self::Handler(error) => error.source(),
            Self::Apply { error, .. } => Some(error),
            Self::BundleMismatch
            | Self::TooManyBindings
            | Self::DuplicateBinding { .. }
            | Self::UnknownBinding { .. }
            | Self::MissingBinding { .. }
            | Self::SignatureMismatch { .. }
            | Self::ProposalLimit
            | Self::ForeignActor { .. } => None,
        }
    }
}

/// The acting entity a proposal names.
fn proposal_actor(action: &CombatAction) -> EntityId {
    match *action {
        CombatAction::UseAbility { actor, .. } | CombatAction::EndTurn { actor } => actor,
    }
}

impl ActionBindings {
    /// Validates and freezes one trusted binding table.
    ///
    /// First failure, in order: `identity` differs from
    /// `declarations.identity()` (`BundleMismatch`); more than
    /// [`MAX_ACTION_SIGNATURES`] bindings (`TooManyBindings`, checked before
    /// any sorting or map construction); the lexically first duplicated id
    /// (`DuplicateBinding`); the lexically first undeclared id
    /// (`UnknownBinding`); the lexically first declared but unbound id
    /// (`MissingBinding`); then, in declaration lexical order, the first
    /// binding whose signature is not exactly equal to its declaration,
    /// parameter order included (`SignatureMismatch`). Registration order
    /// changes neither the result nor the diagnostic. Empty declarations
    /// with empty bindings are valid.
    ///
    /// The declaration digest does not attest to handler code: a rebuilt
    /// binary is a new exact-build determinism domain.
    pub fn new(
        declarations: ActionSignatureStore,
        identity: ActionBundleIdentity,
        bindings: Vec<ActionBinding>,
    ) -> Result<Self, BindingError> {
        if identity != *declarations.identity() {
            return Err(BindingError::BundleMismatch);
        }
        if bindings.len() > MAX_ACTION_SIGNATURES {
            return Err(BindingError::TooManyBindings);
        }
        let mut order: Vec<&ActionBinding> = bindings.iter().collect();
        order.sort_by(|left, right| left.signature.action_id.cmp(&right.signature.action_id));
        for pair in order.windows(2) {
            if pair[0].signature.action_id == pair[1].signature.action_id {
                return Err(BindingError::DuplicateBinding {
                    action_id: pair[0].signature.action_id.clone(),
                });
            }
        }
        for binding in &order {
            if declarations.get(&binding.signature.action_id).is_none() {
                return Err(BindingError::UnknownBinding {
                    action_id: binding.signature.action_id.clone(),
                });
            }
        }
        // Bindings are now unique and all declared, so `order` and the
        // store's lexical iteration align exactly when nothing is missing.
        let mut bound = order.iter().peekable();
        for (action_id, _) in declarations.iter() {
            match bound.peek() {
                Some(binding) if binding.signature.action_id == action_id => {
                    bound.next();
                }
                _ => {
                    return Err(BindingError::MissingBinding {
                        action_id: action_id.to_owned(),
                    });
                }
            }
        }
        for (binding, (action_id, declared)) in order.iter().zip(declarations.iter()) {
            if binding.signature != *declared {
                return Err(BindingError::SignatureMismatch {
                    action_id: action_id.to_owned(),
                });
            }
        }
        let handlers: BTreeMap<String, ActionHandler> = bindings
            .into_iter()
            .map(|binding| (binding.signature.action_id, binding.handler))
            .collect();
        Ok(Self {
            declarations,
            handlers,
        })
    }

    /// The immutable declarations this table implements.
    pub fn declarations(&self) -> &ActionSignatureStore {
        &self.declarations
    }

    /// Whether exactly `action_id` is bound. Inert: installs nothing.
    pub fn contains(&self, action_id: &str) -> bool {
        self.handlers.contains_key(action_id)
    }

    /// Runs one synchronous invocation transactionally.
    ///
    /// In order: T029a validates `identity` and `call` (`Call`); T028's
    /// `validate_action(world, EndTurn { actor })` validates the invocation
    /// actor (`Actor`); the matched handler runs exactly once with a
    /// read-only context (`Handler`); more than [`MAX_BINDING_PROPOSALS`]
    /// proposals fail (`ProposalLimit`), then the first proposal naming
    /// another actor (`ForeignActor`). Proposals then apply in returned
    /// order through `perform_action` on a staged clone, each validated
    /// against the state the earlier ones produced; the first rejection
    /// returns `Apply` and discards every staged change. Only complete
    /// success publishes the staged world, once, and returns each outcome in
    /// proposal order (`Some` for `UseAbility`, `None` for `EndTurn`). Empty
    /// proposals succeed with an empty result and an unchanged world.
    ///
    /// Host actor authorization and target disclosure are the caller's
    /// responsibility; the simulation always rechecks legality.
    pub fn dispatch(
        &self,
        world: &mut World,
        actor: EntityId,
        identity: &ActionBundleIdentity,
        call: &ActionCall,
    ) -> Result<Vec<Option<ActionOutcome>>, BindingError> {
        self.declarations
            .validate_call(identity, call)
            .map_err(BindingError::Call)?;
        validate_action(world, &CombatAction::EndTurn { actor }).map_err(BindingError::Actor)?;
        let handler = *self
            .handlers
            .get(&call.action_id)
            .expect("startup binds every declaration T029a validated");
        let context = InvocationContext {
            world: &*world,
            actor,
        };
        let proposals = handler(&context, call).map_err(BindingError::Handler)?;
        if proposals.len() > MAX_BINDING_PROPOSALS {
            return Err(BindingError::ProposalLimit);
        }
        if let Some(index) = proposals
            .iter()
            .position(|proposal| proposal_actor(proposal) != actor)
        {
            return Err(BindingError::ForeignActor { index });
        }
        if proposals.is_empty() {
            return Ok(Vec::new());
        }
        let mut staged = world.clone();
        let mut outcomes = Vec::with_capacity(proposals.len());
        for (index, proposal) in proposals.iter().enumerate() {
            let outcome = perform_action(&mut staged, proposal)
                .map_err(|error| BindingError::Apply { index, error })?;
            outcomes.push(outcome);
        }
        *world = staged;
        Ok(outcomes)
    }
}

#![forbid(unsafe_code)]
#![warn(missing_docs)]
//! Scripting: trusted action bindings today; the Lua 5.4 sandbox, event IR
//! interpreter and serializable continuations are later tasks.
//!
//! What exists is the T029b synchronous slice ([`bindings`], D17): an
//! immutable startup table pairing each T029a action declaration with one
//! trusted Rust handler, and a transactional dispatch that turns a validated
//! call into bounded `CombatAction` proposals applied through the public
//! `crpg-sim` controller. No Lua runtime, graph interpreter, wait/yield,
//! continuation, native loading or host integration is implemented here.

pub mod bindings;

pub use bindings::{
    ActionBinding, ActionBindings, ActionHandler, BindingError, HandlerError, InvocationContext,
    MAX_BINDING_PROPOSALS,
};

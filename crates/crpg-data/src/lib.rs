#![forbid(unsafe_code)]
#![warn(missing_docs)]
//! Portable campaign documents, canonical bytes, package locks and authored IR.
//!
//! Callers own I/O. Structural acceptance is fail-fast; semantic diagnostics,
//! migration, runtime conversion and graph execution belong to later tasks.

pub mod canonical;
pub mod document;
pub mod error;
pub mod introspection;
pub mod ir;
pub mod loader;
pub mod migrations;
pub mod package;
pub mod schema;
pub mod types;
pub mod validation;

mod inventory;

pub use canonical::*;
pub use document::*;
pub use error::*;
pub use introspection::*;
pub use ir::*;
pub use loader::*;
pub use migrations::*;
pub use package::*;
pub use schema::*;
pub use types::*;
pub use validation::*;

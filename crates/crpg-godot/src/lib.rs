#![allow(unsafe_code)]
#![warn(missing_docs)]
//! Godot presentation bridge (ADR-0033). The only crate permitted to depend on
//! the `godot` crate or to carry `unsafe`.
//!
//! This landing opens the Godot-free core: the bridge error table, the local
//! in-memory link, the client half and the complete authority half with
//! `BridgeCore`. The gdext shell arrives with T047b.

pub mod core;

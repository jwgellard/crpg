#![forbid(unsafe_code)]
#![warn(missing_docs)]
//! The authoritative host slice (T022, ADR-0022).
//!
//! `crpg-server` owns world state. This library is the one platform-neutral
//! host that the Windows embedded single-player adapter and the
//! Windows/Linux dedicated adapters reuse; the `crpg-server` binary stays a
//! thin wiring shell over it. Three modules, no glob re-export:
//!
//! - [`host`]: the serial [`host::Host`] privately owning one
//!   `crpg_sim::HistoryWorld` — peer bindings with per-session epochs,
//!   per-entity disclosure and control grants, byte-level ingest, the FIFO
//!   admission pump (decode → epoch → seq/cache → freshness → mapping →
//!   reservation → `perform_action` → commit), per-peer delivery logs, and
//!   shutdown.
//! - [`capture`]: the bounded capture journal shapes — every accepted
//!   command's returned outcome, exact history range, and permitted
//!   per-peer post-state, retired only by a trusted consumer.
//! - [`checkpoint`]: bounded in-memory checkpoint bytes with fresh sessions
//!   on restart.
//!
//! No client receives `&Host`, `&mut World`, or capture access: the
//! client-facing capability is a copyable [`host::PeerHandle`] plus the
//! adapter's byte channel. Authentication, QUIC, and persistence backends
//! are adapter layers above this contract.

pub mod capture;
pub mod checkpoint;
pub mod host;

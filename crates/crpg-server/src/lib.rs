#![forbid(unsafe_code)]
#![warn(missing_docs)]
//! The authoritative host slice (T022, ADR-0022).
//!
//! `crpg-server` owns world state. This library is the one platform-neutral
//! host that the Windows embedded single-player adapter and the
//! Windows/Linux dedicated adapters reuse; the `crpg-server` binary stays a
//! thin wiring shell over it. Five modules, no glob re-export:
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
//! - [`save`]: host save files — a campaign/engine identity header in front
//!   of the verbatim checkpoint, inside a `crpg-persist` envelope written by
//!   its atomic-replace file store (T039).
//! - [`quic`]: the real-transport adapter (T023b) — one [`host::Host`] plus
//!   one `crpg_net_quic::QuicServer` driven by a synchronous pump, with
//!   invitation credentials checked in constant time, per-connection
//!   backpressure, fencing with `SessionFenced`, and D02 shutdown.
//!
//! No client receives `&Host`, `&mut World`, or capture access: the
//! client-facing capability is a copyable [`host::PeerHandle`] plus the
//! adapter's byte channel. Persistence goes through [`save`] over
//! `crpg-persist`; authentication and the QUIC transport go through
//! [`quic`] over `crpg-net-quic`, which owns the socket, runtime and thread.

pub mod capture;
pub mod checkpoint;
pub mod host;
pub mod quic;
pub mod save;

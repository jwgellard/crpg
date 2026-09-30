#![forbid(unsafe_code)]
#![warn(missing_docs)]
//! Lane-0 combat protocol v1: versioned wire, bounded codec, local transport.
//!
//! `crpg-net` owns the authoritative-transport proof for headless combat. In
//! T018a that means the versioned lane-0 combat wire
//! ([`protocol`]: explicit version + lane, per-lane `seq` identity, closed
//! combat and filtered-replica vocabularies, versioned hard maxima, frozen
//! receipt codes, documented host-policy defaults), the bounded `postcard`
//! [`codec`] (canonical encode, staged decode with exact [`codec::CodecError`]
//! mapping, no `World`/RNG/event access), and the local
//! [`transport::Transport`] byte-pipe trait (E003-B: local to net, no
//! contracts trait) — the single-channel endpoint abstraction. The T018b
//! [`sim::InMemoryTransport`] fabric pumps the same bounded queues, injected
//! time, and fault schedules across per-peer channels; a single-channel
//! endpoint `impl` of the trait arrives with later endpoint work. The T018c
//! suite proves codec round-trip, malicious-client rejection without
//! mutation, filtered replica convergence, and the 5,000-tick loss/jitter
//! exercise. The T021 [`protocol_v2`]/[`codec_v2`] pair carries the same
//! vocabulary plus ordered history events beside frozen v1 under explicit
//! version selection, and [`projection_v2`] maps host-authorized per-field
//! candidates to ordered event ops. Real QUIC, movement wire, reconnect,
//! prediction, and host auth/perception are named follow-ons, not here.

pub mod codec;
pub mod codec_v2;
pub mod projection_v2;
pub mod protocol;
pub mod protocol_v2;
pub mod sim;
pub mod transport;

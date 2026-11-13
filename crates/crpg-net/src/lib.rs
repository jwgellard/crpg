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
//! candidates to ordered event ops. The T024 [`protocol_transfer`]/
//! [`codec_transfer`] / [`reassembly`] triple carries the separate versioned
//! snapshot-transfer subprotocol (first byte `0x81`, lane 2), and the T025a1
//! [`protocol_resume`] / [`codec_resume`] pair carries the separate versioned
//! resume subprotocol (first byte `0x91`, lane 3). The T027b [`interest`]
//! module projects host-supplied facts to existing v1 ops or a T024 body.
//! The T046 [`replica`] module applies decoded v2 deltas and published
//! snapshots into the engine-neutral client copy. Real QUIC,
//! movement wire, reconnect policy, prediction, and host auth/perception are
//! named follow-ons, not here. The T140a [`protocol_v3`]/[`codec_v3`] pair
//! adds the explicit v3 version (byte 3): the pause, mode and standing-order
//! intents, native tags 0..=10 with rejection codes 1..=30, and the scalar
//! encounter-event, `Clock` and host-fact ops. The T140r1 [`pool_v3`] module
//! gates `0x82` transfers and v3 deltas into the v3 receiver's FIFO queue.
//! The T140r2 [`replica_v3`] module applies those updates to the client copy,
//! holding the v3 fact, encounter, timeline, pause and host-fact state.

pub mod codec;
pub mod codec_resume;
pub mod codec_transfer;
pub mod codec_v2;
pub mod codec_v3;
pub mod facts_v3;
pub mod interest;
pub mod pool_v3;
pub mod projection_v2;
pub mod projection_v3;
pub mod protocol;
pub mod protocol_resume;
pub mod protocol_transfer;
pub mod protocol_v2;
pub mod protocol_v3;
pub mod reassembly;
pub mod reassembly_v3;
pub mod replica;
pub mod replica_v3;
pub mod resume;
pub mod sim;
pub mod transfer_v3;
pub mod transport;

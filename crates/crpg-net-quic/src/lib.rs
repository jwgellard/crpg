#![forbid(unsafe_code)]
#![warn(missing_docs)]
//! Real lane-0 QUIC transport for `crpg-net`'s byte protocol.
//!
//! This crate is a stub until T023. It exists apart from `crpg-net` so that
//! crates needing only the protocol never link tokio, quinn or rustls
//! (ADR-0024).

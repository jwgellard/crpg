#![forbid(unsafe_code)]
#![warn(missing_docs)]
//! The headless dedicated server binary: a thin wiring shell (D02) over the
//! `crpg_server` library, which owns the authoritative host. Transport
//! (QUIC, T023/T023b), authentication, OS service integration, and
//! persistence backends are adapter work that has not landed, so the binary
//! still refuses to run rather than pretending to serve.

fn main() {
    eprintln!("crpg-server: dedicated adapter not yet implemented");
    std::process::exit(1);
}

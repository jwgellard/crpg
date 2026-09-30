//! Prints the T020 history golden schedule hashes to stdout.
//!
//! Authoring only: this example never chooses or writes a baseline file.
//! Each line is `<step> <64-lowercase-hex-hash>` with one final LF. Run it
//! independently on native Windows/MSVC and genuine Linux/GNU with the
//! pinned toolchain, default features, and the normal test profile, then
//! review each output before adding its target file:
//!
//! ```text
//! cargo run -p crpg-sim --example generate_history_golden --profile test --locked
//! ```

#[path = "../tests/support/history_schedule.rs"]
#[allow(dead_code)]
mod history_schedule;

use history_schedule::run_golden_schedule;

/// Prints one hash line per schedule step, step zero first.
fn main() {
    let run = run_golden_schedule();
    for (step, hash) in run.hashes.iter().enumerate() {
        let mut hex = String::with_capacity(64);
        for byte in hash {
            hex.push_str(&format!("{byte:02x}"));
        }
        println!("{step} {hex}");
    }
}

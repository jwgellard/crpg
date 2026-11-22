//! T049a SR1: `smooth.rs` names nothing of the server, the sim, an entity id,
//! the authority, a float, a hash map or a clock.
//!
//! The scanner drops `//`-to-end-of-line text, splits the rest into maximal
//! `[A-Za-z0-9_]+` tokens, and bans only an equal token.

use std::fs;
use std::path::PathBuf;

fn src_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src")
}

/// Drops `//`-to-end-of-line text.
fn strip_line_comments(text: &str) -> String {
    let mut kept = String::new();
    for line in text.lines() {
        match line.find("//") {
            Some(index) => kept.push_str(&line[..index]),
            None => kept.push_str(line),
        }
        kept.push('\n');
    }
    kept
}

/// Maximal identifier-ish tokens of the comment-free text.
fn tokens(text: &str) -> Vec<&str> {
    text.split(|character: char| !(character.is_ascii_alphanumeric() || character == '_'))
        .filter(|token| !token.is_empty())
        .collect()
}

fn read(path: &std::path::Path) -> String {
    strip_line_comments(&fs::read_to_string(path).expect("read source"))
}

/// SR1: the smoother's source names none of the ten.
#[test]
fn smooth_names_nothing_it_must_not() {
    let text = read(&src_root().join("core").join("smooth.rs"));
    let found = tokens(&text);
    for banned in [
        "crpg_server",
        "crpg_sim",
        "EntityId",
        "authority",
        "f32",
        "f64",
        "HashMap",
        "HashSet",
        "Instant",
        "SystemTime",
    ] {
        assert!(!found.contains(&banned), "smooth.rs names {banned}");
    }

    // Control: the authority half really does name the server, so the scan
    // above is not vacuous.
    let authority = read(&src_root().join("core").join("authority.rs"));
    let authority = tokens(&authority);
    assert!(authority.contains(&"crpg_server"));
}

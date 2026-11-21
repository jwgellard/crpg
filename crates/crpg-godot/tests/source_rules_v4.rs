//! A8 / B-A10: the v4 client half never names the authority.
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

/// A8: `client_v4.rs` names neither the server, the sim, an entity id nor the
/// authority in code.
#[test]
fn client_v4_never_names_the_authority() {
    let text = read(&src_root().join("core").join("client_v4.rs"));
    let found = tokens(&text);
    for banned in ["crpg_server", "crpg_sim", "EntityId", "authority"] {
        assert!(!found.contains(&banned), "client_v4.rs names {banned}");
    }

    // Control: the authority half really does name the server and its ids, so
    // the token scan above is not vacuous.
    let authority = read(&src_root().join("core").join("authority.rs"));
    let authority = tokens(&authority);
    assert!(authority.contains(&"crpg_server"));
    assert!(authority.contains(&"EntityId"));
}

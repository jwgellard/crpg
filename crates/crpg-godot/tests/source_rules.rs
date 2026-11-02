//! K31 / BR-10, B18, D4: the core stays engine-free and clock-free, the two
//! lower halves never name the authority, and `core` is never a bare path.
//!
//! The scanner drops `//`-to-end-of-line text, splits the rest into maximal
//! `[A-Za-z0-9_]+` tokens, and bans only an equal token, so `authority_end`
//! is not `authority`.

use std::fs;
use std::path::{Path, PathBuf};

fn src_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src")
}

fn source_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("read src") {
        let entry = entry.expect("src entry");
        let path = entry.path();
        if path.is_dir() {
            source_files(&path, out);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            out.push(path);
        }
    }
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

fn read(path: &Path) -> String {
    strip_line_comments(&fs::read_to_string(path).expect("read source"))
}

/// Every `core::` not preceded by an identifier character or another colon.
fn bare_core_paths(text: &str) -> Vec<String> {
    let mut hits = Vec::new();
    let bytes = text.as_bytes();
    for index in 0..bytes.len().saturating_sub(6) {
        if &text[index..index + 6] != "core::" {
            continue;
        }
        let previous = index.checked_sub(1).map(|before| bytes[before]);
        let allowed = match previous {
            None => true,
            Some(byte) => !(byte.is_ascii_alphanumeric() || byte == b'_' || byte == b':'),
        };
        if allowed {
            let start = text[..index].rfind('\n').map_or(0, |line| line + 1);
            let end = text[index..]
                .find('\n')
                .map_or(text.len(), |line| index + line);
            hits.push(text[start..end].trim().to_string());
        }
    }
    hits
}

fn is_test_file(path: &Path) -> bool {
    path.components()
        .any(|component| component.as_os_str() == "tests")
        || path.file_name().is_some_and(|name| name == "tests.rs")
}

/// K31: the token bans and the bare-path rule.
#[test]
fn the_core_has_no_engine_clock_or_bare_core_path() {
    let root = src_root();
    let mut files = Vec::new();
    source_files(&root, &mut files);
    assert!(files.len() > 5);

    let mut lower = Vec::new();
    for file in &files {
        if file
            .file_name()
            .is_some_and(|name| name == "client.rs" || name == "link.rs")
        {
            lower.push(file);
        }
    }
    assert_eq!(lower.len(), 2);
    for file in lower {
        let text = read(file);
        let found = tokens(&text);
        for banned in ["crpg_server", "crpg_sim", "EntityId", "authority"] {
            assert!(
                !found.contains(&banned),
                "{} names {banned}",
                file.display()
            );
        }
    }

    for file in &files {
        if is_test_file(file) {
            continue;
        }
        let text = read(file);
        let found = tokens(&text);
        for banned in ["f32", "f64", "HashMap", "HashSet", "Instant", "SystemTime"] {
            assert!(
                !found.contains(&banned),
                "{} names {banned}",
                file.display()
            );
        }
        assert!(
            !text.contains("thread::spawn"),
            "{} spawns a thread",
            file.display()
        );
    }

    let module = read(&root.join("core").join("mod.rs"));
    let first_attribute = module
        .lines()
        .map(str::trim)
        .find(|line| line.starts_with("#!["))
        .expect("core/mod.rs has a crate attribute");
    assert_eq!(first_attribute, "#![forbid(unsafe_code)]");

    for file in &files {
        let hits = bare_core_paths(&read(file));
        assert!(
            hits.is_empty(),
            "{} has a bare core:: path: {hits:?}",
            file.display()
        );
    }

    // Control: the authority half really does name the server and its ids, so
    // the token scan above is not vacuous.
    let authority_text = read(&root.join("core").join("authority.rs"));
    let authority = tokens(&authority_text);
    assert!(authority.contains(&"crpg_server"));
    assert!(authority.contains(&"EntityId"));
}

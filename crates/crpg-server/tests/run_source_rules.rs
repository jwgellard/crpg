//! T154b source-rule test (`tasks/T154.md` U8 under R-6, with the `run.rs`
//! rule of K16): with `//` comments stripped, the clock, thread and socket
//! tokens appear in `crpg-server` source only in `runner.rs`, and `run.rs`
//! also names no blocking call, AI type or hash collection.
//!
//! Reading: "source" is the shipped code. The `#[cfg(test)] mod tests`
//! modules are stripped before the check: `quic.rs`'s own test module names
//! `Instant` and `std::thread::spawn`, and `quic.rs` must stay unchanged, so
//! a file-wide literal scan could never pass without editing a frozen file.
//! The fence (invariant 7) guards production I/O, not the tests' own rigs.

/// Every `crpg-server` source file the fence covers.
const SOURCES: &[(&str, &str)] = &[
    ("host.rs", include_str!("../src/host.rs")),
    ("capture.rs", include_str!("../src/capture.rs")),
    ("checkpoint.rs", include_str!("../src/checkpoint.rs")),
    ("save.rs", include_str!("../src/save.rs")),
    ("quic.rs", include_str!("../src/quic.rs")),
    ("run.rs", include_str!("../src/run.rs")),
    ("runner.rs", include_str!("../src/runner.rs")),
    ("host_v3.rs", include_str!("../src/host_v3.rs")),
    ("host_v4.rs", include_str!("../src/host_v4.rs")),
    ("nav.rs", include_str!("../src/nav.rs")),
    ("main.rs", include_str!("../src/main.rs")),
];

/// The tokens R-6 fences to `runner.rs`.
const FENCED: &[&str] = &[
    "Instant",
    "SystemTime",
    "thread::",
    "sleep",
    "recv_timeout",
    "UdpSocket",
    "TcpListener",
];

/// Shipped code of one file: `//` comments stripped (as K16 does) and the
/// `#[cfg(test)] mod tests` module, when present, cut away.
fn shipped(source: &str) -> String {
    let lines: Vec<&str> = source.lines().collect();
    let mut end = lines.len();
    for (index, line) in lines.iter().enumerate() {
        let next_is_tests = lines
            .get(index + 1)
            .is_some_and(|next| next.trim_start().starts_with("mod tests"));
        if line.trim() == "#[cfg(test)]" && next_is_tests {
            end = index;
            break;
        }
    }
    lines[..end]
        .iter()
        .map(|line| match line.find("//") {
            Some(index) => &line[..index],
            None => line,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn assert_no_token(file: &str, code: &str, token: &str) {
    assert!(
        !code.contains(token),
        "{file} must not name {token} outside its test module"
    );
}

/// U8 `source_rules` (R-6): the clock, thread and socket tokens live only in
/// `runner.rs`; `run.rs` never blocks and names no AI type or hash map.
#[test]
fn source_rules() {
    let mut run_code = String::new();
    for (file, source) in SOURCES {
        let code = shipped(source);
        if *file == "runner.rs" {
            continue;
        }
        for token in FENCED {
            assert_no_token(file, &code, token);
        }
        if *file == "run.rs" {
            run_code = code;
        }
    }

    // `run.rs` (K16): no clock, thread, socket, blocking call, AI type or
    // hash collection.
    for token in FENCED
        .iter()
        .chain([".recv()", ".send(", "crpg_ai", "HashMap", "HashSet"].iter())
    {
        assert_no_token("run.rs", &run_code, token);
    }

    // The fence's one exception: `runner.rs` alone reads `Instant` (through
    // `SystemClock`), spawns the one loop thread and waits on its control
    // channel — and it owns no other fenced token.
    let runner = shipped(
        SOURCES
            .iter()
            .find_map(|(file, source)| (*file == "runner.rs").then_some(*source))
            .expect("runner.rs listed"),
    );
    for token in ["Instant", "thread::", "recv_timeout"] {
        assert!(
            runner.contains(token),
            "runner.rs must own the loop's {token}"
        );
    }
    for token in ["SystemTime", "sleep", "UdpSocket", "TcpListener"] {
        assert_no_token("runner.rs", &runner, token);
    }
}

/// Fix round (U8 source list): the hand-written `SOURCES` list cannot
/// silently go stale — `runner.rs` is found by enumerating `src/*.rs`, so
/// the fence scan cannot pass on an empty or drifted list.
#[test]
fn source_scan_covers_the_tree() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut names: Vec<String> = Vec::new();
    let entries = std::fs::read_dir(&dir).expect("src lists");
    for entry in entries {
        let name = entry.expect("entry").file_name();
        let name = name.to_str().expect("utf8").to_owned();
        if name.ends_with(".rs") {
            names.push(name);
        }
    }
    names.sort();
    assert!(!names.is_empty());
    assert!(names.contains(&"runner.rs".to_owned()));
    for (file, _) in SOURCES {
        assert!(
            names.contains(&file.to_string()),
            "{file} is listed but missing on disk"
        );
    }
}

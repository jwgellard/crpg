//! T038 §8 `tests/file_store.rs`: the atomic-replace file store (§6). Each
//! test owns `CARGO_TARGET_TMPDIR/crpg-persist/<test fn name>`, removed and
//! recreated first, so parallel tests never share a path.

use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use crpg_persist::{
    encode_envelope, load_file, read_capped, save_file, EnvelopeError, FileError, FileOp,
    PayloadKind, MAX_ENVELOPE_BYTES, MAX_PAYLOAD_BYTES, TEMP_SUFFIX,
};

fn testkind() -> PayloadKind {
    PayloadKind::new(*b"TESTKIND").expect("TESTKIND is in the alphabet")
}

fn test_dir(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("crpg-persist")
        .join(name);
    match fs::remove_dir_all(&dir) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => panic!("cannot clear {}: {e}", dir.display()),
    }
    fs::create_dir_all(&dir).expect("create test dir");
    dir
}

fn temp_of(path: &Path) -> PathBuf {
    let mut name = path.file_name().expect("file name").to_os_string();
    name.push(TEMP_SUFFIX);
    path.with_file_name(name)
}

/// T038 §8 compressible pattern.
fn pattern(n: usize) -> Vec<u8> {
    (0..n).map(|i| (i % 251) as u8 ^ (i >> 16) as u8).collect()
}

fn entries(dir: &Path) -> Vec<PathBuf> {
    let mut names: Vec<PathBuf> = fs::read_dir(dir)
        .expect("read dir")
        .map(|e| e.expect("dir entry").path())
        .collect();
    names.sort();
    names
}

#[test]
fn save_then_load_round_trips() {
    let dir = test_dir("save_then_load_round_trips");
    let path = dir.join("slot.sav");
    let payload = pattern(100_000);
    save_file(&path, testkind(), &payload).expect("save");
    assert_eq!(load_file(&path, testkind()).expect("load"), payload);
    assert!(!temp_of(&path).exists());
    assert_eq!(entries(&dir), vec![path]);
}

#[test]
fn file_bytes_equal_encoded_envelope() {
    let dir = test_dir("file_bytes_equal_encoded_envelope");
    let path = dir.join("slot.sav");
    for payload in [b"abc".to_vec(), pattern(100_000)] {
        save_file(&path, testkind(), &payload).expect("save");
        assert_eq!(
            fs::read(&path).expect("read file"),
            encode_envelope(testkind(), &payload).expect("encode")
        );
    }
}

#[test]
fn save_replaces_existing_file() {
    let dir = test_dir("save_replaces_existing_file");
    let path = dir.join("slot.sav");
    save_file(&path, testkind(), b"first save A").expect("save A");
    assert_eq!(
        load_file(&path, testkind()).expect("load A"),
        b"first save A"
    );
    save_file(&path, testkind(), b"second save B").expect("save B");
    assert_eq!(
        load_file(&path, testkind()).expect("load B"),
        b"second save B"
    );
    assert!(!temp_of(&path).exists());
    assert_eq!(entries(&dir), vec![path]);
}

#[test]
fn failed_encode_leaves_previous_save() {
    let dir = test_dir("failed_encode_leaves_previous_save");
    let path = dir.join("slot.sav");
    save_file(&path, testkind(), b"save A").expect("save A");
    let before = fs::read(&path).expect("read A");

    let over = vec![0u8; MAX_PAYLOAD_BYTES + 1];
    assert_eq!(
        save_file(&path, testkind(), &over),
        Err(FileError::Envelope(EnvelopeError::PayloadTooLarge))
    );
    assert_eq!(fs::read(&path).expect("read after"), before);
    assert!(!temp_of(&path).exists());
    assert_eq!(entries(&dir), vec![path]);
}

#[test]
fn failed_rename_keeps_destination_and_removes_temp() {
    let dir = test_dir("failed_rename_keeps_destination_and_removes_temp");
    let path = dir.join("slot.sav");
    // The destination is an existing (non-empty) directory, which no rename
    // of a file may replace on either target.
    fs::create_dir(&path).expect("create destination dir");
    let inner = path.join("keep.txt");
    fs::write(&inner, b"keep").expect("write inner file");

    // Positive control: the same payload saves fine to a plain path.
    let control = dir.join("control.sav");
    save_file(&control, testkind(), b"payload").expect("control save");

    match save_file(&path, testkind(), b"payload") {
        Err(FileError::Io {
            op: FileOp::Rename, ..
        }) => {}
        other => panic!("expected a rename failure, got {other:?}"),
    }
    assert!(path.is_dir());
    assert_eq!(fs::read(&inner).expect("inner file"), b"keep");
    assert!(!temp_of(&path).exists());
}

#[test]
fn missing_parent_directory() {
    let dir = test_dir("missing_parent_directory");
    let path = dir.join("missing").join("slot.sav");
    assert_eq!(
        save_file(&path, testkind(), b"payload"),
        Err(FileError::Io {
            op: FileOp::CreateTemp,
            kind: io::ErrorKind::NotFound
        })
    );
    assert!(entries(&dir).is_empty());
}

#[test]
fn stale_temp_file_is_replaced() {
    let dir = test_dir("stale_temp_file_is_replaced");
    let path = dir.join("slot.sav");
    // A long junk temp file left by an earlier crash.
    fs::write(temp_of(&path), vec![0xa5u8; 10_000]).expect("write junk temp");
    save_file(&path, testkind(), b"payload").expect("save");
    assert_eq!(load_file(&path, testkind()).expect("load"), b"payload");
    assert!(!temp_of(&path).exists());
    assert_eq!(entries(&dir), vec![path]);
}

#[test]
fn invalid_paths_refused() {
    let dir = test_dir("invalid_paths_refused");
    for path in [Path::new(""), Path::new(".."), &dir.join("..")] {
        assert_eq!(
            save_file(path, testkind(), b"payload"),
            Err(FileError::InvalidPath),
            "{}",
            path.display()
        );
    }
    assert!(entries(&dir).is_empty());

    // Positive control: a real file name in the same directory saves.
    save_file(&dir.join("slot.sav"), testkind(), b"payload").expect("control save");
}

#[test]
fn load_missing_file() {
    let dir = test_dir("load_missing_file");
    assert_eq!(
        load_file(&dir.join("absent.sav"), testkind()),
        Err(FileError::Io {
            op: FileOp::Open,
            kind: io::ErrorKind::NotFound
        })
    );
    assert!(entries(&dir).is_empty());
}

#[test]
fn read_capped_bounds_input() {
    let dir = test_dir("read_capped_bounds_input");

    // An endless reader: the cap bounds the read and the test terminates.
    assert_eq!(
        read_capped(io::repeat(0x43)),
        Err(FileError::Envelope(EnvelopeError::InputTooLarge))
    );

    // Exactly at the cap is accepted.
    let exact = read_capped(io::repeat(0x43).take(MAX_ENVELOPE_BYTES as u64)).expect("at cap");
    assert_eq!(exact.len(), MAX_ENVELOPE_BYTES);
    drop(exact);

    // One byte over the cap, on disk, through load_file.
    let path = dir.join("oversized.sav");
    fs::write(&path, vec![0u8; MAX_ENVELOPE_BYTES + 1]).expect("write oversized");
    assert_eq!(
        load_file(&path, testkind()),
        Err(FileError::Envelope(EnvelopeError::InputTooLarge))
    );
}

#[test]
fn load_reports_envelope_errors_and_leaves_file() {
    let dir = test_dir("load_reports_envelope_errors_and_leaves_file");
    let path = dir.join("slot.sav");
    save_file(&path, testkind(), b"payload").expect("save");
    assert_eq!(
        load_file(&path, testkind()).expect("positive control"),
        b"payload"
    );

    let mut bytes = fs::read(&path).expect("read");
    bytes[40] ^= 0x01;
    fs::write(&path, &bytes).expect("write corrupted");
    assert_eq!(
        load_file(&path, testkind()),
        Err(FileError::Envelope(EnvelopeError::ChecksumMismatch))
    );
    assert_eq!(fs::read(&path).expect("read after load"), bytes);
    assert_eq!(entries(&dir), vec![path]);
}

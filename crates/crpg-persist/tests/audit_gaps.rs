//! T122 audit-gap tests for crpg-persist. Tests only; no source changes.

use std::fs;
use std::io::{self, Read};
use std::path::PathBuf;

use crpg_persist::{
    decode_envelope, encode_envelope, load_file, read_capped, save_file, EnvelopeError, FileError,
    FileOp, PayloadKind, DIGEST_CONTEXT, FLAG_ZSTD, HEADER_BYTES, MAX_ENVELOPE_BYTES,
    MAX_PAYLOAD_BYTES, SAVE_FORMAT_VERSION, SAVE_MAGIC, TEMP_SUFFIX, ZSTD_LEVEL,
    ZSTD_WINDOW_LOG_MAX,
};

const LITERAL_CONTEXT: &str = "crpg-persist 2026-10-04 save envelope v1";
const TESTKIND: [u8; 8] = *b"TESTKIND";

fn testkind() -> PayloadKind {
    PayloadKind::new(TESTKIND).expect("TESTKIND is in the alphabet")
}

fn pattern(n: usize) -> Vec<u8> {
    (0..n).map(|i| (i % 251) as u8 ^ (i >> 16) as u8).collect()
}

fn test_dir(name: &str) -> PathBuf {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"))
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

fn temp_of(path: &std::path::Path) -> PathBuf {
    let mut name = path.file_name().expect("file name").to_os_string();
    name.push(TEMP_SUFFIX);
    path.with_file_name(name)
}

fn raw_abc() -> Vec<u8> {
    encode_envelope(testkind(), b"abc").expect("encode abc")
}

fn build(flags: u16, kind: [u8; 8], payload_len: u64, body: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"CRPGSAVE");
    bytes.extend_from_slice(&SAVE_FORMAT_VERSION.to_le_bytes());
    bytes.extend_from_slice(&flags.to_le_bytes());
    bytes.extend_from_slice(&kind);
    bytes.extend_from_slice(&payload_len.to_le_bytes());
    bytes.extend_from_slice(&(body.len() as u64).to_le_bytes());
    bytes.extend_from_slice(&[0u8; 32]);
    bytes.extend_from_slice(body);
    // resign
    let mut hasher = blake3::Hasher::new_derive_key(LITERAL_CONTEXT);
    hasher.update(&bytes[..36]);
    hasher.update(&bytes[HEADER_BYTES..]);
    bytes[36..68].copy_from_slice(hasher.finalize().as_bytes());
    bytes
}

fn set_u16(bytes: &mut [u8], at: usize, value: u16) {
    bytes[at..at + 2].copy_from_slice(&value.to_le_bytes());
}

fn set_u64(bytes: &mut [u8], at: usize, value: u64) {
    bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
}

fn resign(bytes: &mut [u8]) {
    let mut hasher = blake3::Hasher::new_derive_key(LITERAL_CONTEXT);
    hasher.update(&bytes[..36]);
    hasher.update(&bytes[HEADER_BYTES..]);
    bytes[36..68].copy_from_slice(hasher.finalize().as_bytes());
}

fn frame(data: &[u8]) -> Vec<u8> {
    zstd::bulk::compress(data, 3).expect("zstd frame")
}

fn bomb_frame() -> Vec<u8> {
    let mut encoder = zstd::stream::write::Encoder::new(Vec::new(), 3).expect("encoder");
    let mut zeros = io::repeat(0).take(256 << 20);
    io::copy(&mut zeros, &mut encoder).expect("stream zeros");
    encoder.finish().expect("finish bomb")
}

fn decode(bytes: &[u8]) -> Result<Vec<u8>, EnvelopeError> {
    decode_envelope(testkind(), bytes)
}

// #40
#[test]
fn trailing_second_frame_is_decompressed_too_short() {
    let frame1 = frame(b"abc");
    let mut body = frame1.clone();
    body.extend_from_slice(&frame1);
    let payload_len = body.len() as u64 + 1;
    let bytes = build(FLAG_ZSTD, TESTKIND, payload_len, &body);
    assert_eq!(decode(&bytes), Err(EnvelopeError::DecompressedTooShort));
}

// #41
#[test]
fn zstd_frame_header_descriptor_is_pinned() {
    let payload = pattern(800_000);
    let encoded = encode_envelope(testkind(), &payload).expect("encode");
    let body = &encoded[HEADER_BYTES..];
    assert_eq!(&body[..4], &[0x28, 0xb5, 0x2f, 0xfd]);
    let desc = body[4];
    assert_eq!(desc & 0x04, 0, "checksum should be off");
    assert_eq!(desc & 0x03, 0, "dictionary id should be off");
    assert_ne!(desc & 0xC0, 0, "content size should be on");
    assert_eq!(ZSTD_LEVEL, 3);
}

// #42
#[test]
fn compressed_length_equal_payload_length_stored_raw() {
    let payload = vec![0u8; 17];
    let encoded = encode_envelope(testkind(), &payload).expect("encode");
    let flags = u16::from_le_bytes([encoded[10], encoded[11]]);
    assert_eq!(flags, 0);
    assert_eq!(u64::from_le_bytes(encoded[20..28].try_into().unwrap()), 17);
    assert_eq!(u64::from_le_bytes(encoded[28..36].try_into().unwrap()), 17);
    assert_eq!(&encoded[HEADER_BYTES..], &payload[..]);
}

// #45
#[test]
fn load_ignores_stale_temp_file() {
    // destination present
    let dir = test_dir("load_ignores_stale_temp_file_present");
    let path = dir.join("slot.sav");
    save_file(&path, testkind(), b"payload").expect("save");
    let temp = temp_of(&path);
    let junk = vec![0xa5u8; 1000];
    fs::write(&temp, &junk).expect("junk temp");
    assert_eq!(load_file(&path, testkind()).expect("load"), b"payload");
    assert_eq!(fs::read(&temp).expect("temp read"), junk);

    // destination absent: temp is ignored, load fails Open
    let dir2 = test_dir("load_ignores_stale_temp_file_absent");
    let path2 = dir2.join("absent.sav");
    let temp2 = temp_of(&path2);
    fs::write(&temp2, &junk).expect("junk temp2");
    assert_eq!(
        load_file(&path2, testkind()),
        Err(FileError::Io {
            op: FileOp::Open,
            kind: io::ErrorKind::NotFound
        })
    );
    assert_eq!(fs::read(&temp2).expect("temp2 read"), junk);
}

// #46, #50
#[test]
fn read_capped_rejects_cap_plus_one_finite_bytes() {
    let data = vec![0u8; MAX_ENVELOPE_BYTES + 1];
    assert_eq!(
        read_capped(io::Cursor::new(data.clone())),
        Err(FileError::Envelope(EnvelopeError::InputTooLarge))
    );

    let dir = test_dir("read_capped_rejects_cap_plus_one_finite_bytes");
    let path = dir.join("oversized.sav");
    fs::write(&path, &data).expect("write oversized");
    assert_eq!(
        load_file(&path, testkind()),
        Err(FileError::Envelope(EnvelopeError::InputTooLarge))
    );
}

// #47
#[test]
fn bad_magic_beats_non_v1_version() {
    let mut bytes = raw_abc();
    bytes[0] ^= 0x01; // bad magic
    set_u16(&mut bytes, 8, 2); // non-v1 version
    assert_eq!(decode(&bytes), Err(EnvelopeError::BadMagic));
}

// #48
#[test]
fn non_v2_version_beats_unknown_flags() {
    for version in [0u16, 3u16, u16::MAX] {
        let mut bytes = raw_abc();
        set_u16(&mut bytes, 8, version);
        set_u16(&mut bytes, 10, 0x8000); // unknown flag
        resign(&mut bytes);
        assert_eq!(
            decode(&bytes),
            Err(EnvelopeError::UnsupportedVersion),
            "version {version}"
        );
    }
}

// #49
#[test]
fn load_time_envelope_errors_leave_file_unchanged() {
    let dir = test_dir("load_time_envelope_errors_leave_file_unchanged");
    let raw = raw_abc();
    let bomb = bomb_frame();
    let body = frame(&pattern(1000));
    let mut corrupt_body = frame(&pattern(4096));
    corrupt_body.push(0);

    let mut cases: Vec<(Vec<u8>, EnvelopeError)> = Vec::new();
    cases.push((
        vec![0u8; MAX_ENVELOPE_BYTES + 1],
        EnvelopeError::InputTooLarge,
    ));
    cases.push((raw[..raw.len() - 1].to_vec(), EnvelopeError::Truncated));
    let mut bm = raw.clone();
    bm[0] ^= 0x01;
    cases.push((bm, EnvelopeError::BadMagic));
    let mut v2 = raw.clone();
    set_u16(&mut v2, 8, 2);
    resign(&mut v2);
    cases.push((v2, EnvelopeError::UnsupportedVersion));
    let mut fl = raw.clone();
    set_u16(&mut fl, 10, 0x0002);
    resign(&mut fl);
    cases.push((fl, EnvelopeError::UnknownFlags));
    cases.push((
        build(0, *b"OTHERKND", 3, b"abc"),
        EnvelopeError::KindMismatch,
    ));
    cases.push((
        build(0, TESTKIND, MAX_PAYLOAD_BYTES as u64 + 1, b"abc"),
        EnvelopeError::PayloadTooLarge,
    ));
    let mut lm = raw.clone();
    set_u64(&mut lm, 28, 4);
    resign(&mut lm);
    cases.push((lm, EnvelopeError::LengthMismatch));
    let mut tb = raw.clone();
    tb.push(0);
    cases.push((tb, EnvelopeError::TrailingBytes));
    let mut cm = raw.clone();
    cm[40] ^= 0x01;
    cases.push((cm, EnvelopeError::ChecksumMismatch));
    cases.push((
        build(FLAG_ZSTD, TESTKIND, MAX_PAYLOAD_BYTES as u64, &bomb),
        EnvelopeError::DecompressedTooLarge,
    ));
    cases.push((
        build(FLAG_ZSTD, TESTKIND, 1001, &body),
        EnvelopeError::DecompressedTooShort,
    ));
    cases.push((
        build(FLAG_ZSTD, TESTKIND, 4096, &corrupt_body),
        EnvelopeError::CorruptBody,
    ));

    for (i, (bytes, expected)) in cases.iter().enumerate() {
        let path = dir.join(format!("case_{}.sav", i));
        fs::write(&path, bytes).expect("write");
        let before = fs::read(&path).expect("read before");
        let result = load_file(&path, testkind());
        assert_eq!(
            result,
            Err(FileError::Envelope(*expected)),
            "case {} expected {:?} got {:?}",
            i,
            expected,
            result
        );
        let after = fs::read(&path).expect("read after");
        assert_eq!(before, after, "file changed for case {}", i);
    }
}

// #51
#[test]
fn public_constants_match_contract() {
    assert_eq!(SAVE_MAGIC, *b"CRPGSAVE");
    assert_eq!(SAVE_FORMAT_VERSION, 1);
    assert_eq!(HEADER_BYTES, 68);
    assert_eq!(FLAG_ZSTD, 0x0001);
    assert_eq!(MAX_PAYLOAD_BYTES, 16_777_216);
    assert_eq!(MAX_ENVELOPE_BYTES, 16_777_284);
    assert_eq!(ZSTD_LEVEL, 3);
    assert_eq!(ZSTD_WINDOW_LOG_MAX, 24);
    assert_eq!(DIGEST_CONTEXT, "crpg-persist 2026-10-04 save envelope v1");
    assert_eq!(TEMP_SUFFIX, ".crpg-tmp");
}

//! T038 §8 `tests/rejections.rs`: every `decode_envelope` rejection in §4.2
//! precedence order, the streaming decompression caps (§4.3) and the pinned
//! Display strings. Hostile envelopes are assembled by test-local helpers and
//! re-signed with the literal digest context, never by crate-private code.
//! Every rejection is paired with a valid positive control.

use std::io::{self, Read, Write};

use crpg_persist::{
    decode_envelope, encode_envelope, EnvelopeError, FileError, FileOp, PayloadKind, FLAG_ZSTD,
    HEADER_BYTES, MAX_ENVELOPE_BYTES, MAX_PAYLOAD_BYTES,
};

/// The digest context, written literally so the oracle does not depend on
/// the crate constant.
const LITERAL_CONTEXT: &str = "crpg-persist 2026-10-04 save envelope v1";
const TESTKIND: [u8; 8] = *b"TESTKIND";

fn kind(tag: [u8; 8]) -> PayloadKind {
    PayloadKind::new(tag).expect("tag in the alphabet")
}

fn testkind() -> PayloadKind {
    kind(TESTKIND)
}

/// T038 §8 xorshift64 byte stream.
fn xorshift(n: usize) -> Vec<u8> {
    let mut s: u64 = 0x2545_F491_4F6C_DD1D;
    (0..n)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            (s >> 24) as u8
        })
        .collect()
}

/// T038 §8 compressible pattern.
fn pattern(n: usize) -> Vec<u8> {
    (0..n).map(|i| (i % 251) as u8 ^ (i >> 16) as u8).collect()
}

/// Recompute the digest at offsets 36..68 over `bytes[0..36] ‖ body`.
fn resign(bytes: &mut [u8]) {
    let mut hasher = blake3::Hasher::new_derive_key(LITERAL_CONTEXT);
    hasher.update(&bytes[..36]);
    hasher.update(&bytes[HEADER_BYTES..]);
    bytes[36..68].copy_from_slice(hasher.finalize().as_bytes());
}

/// Assemble a v1 envelope literally from its fields, `body_len = body.len()`,
/// and sign it.
fn build(flags: u16, kind: [u8; 8], payload_len: u64, body: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"CRPGSAVE");
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&flags.to_le_bytes());
    bytes.extend_from_slice(&kind);
    bytes.extend_from_slice(&payload_len.to_le_bytes());
    bytes.extend_from_slice(&(body.len() as u64).to_le_bytes());
    bytes.extend_from_slice(&[0u8; 32]);
    bytes.extend_from_slice(body);
    resign(&mut bytes);
    bytes
}

fn set_u16(bytes: &mut [u8], at: usize, value: u16) {
    bytes[at..at + 2].copy_from_slice(&value.to_le_bytes());
}

fn set_u64(bytes: &mut [u8], at: usize, value: u64) {
    bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
}

fn raw_abc() -> Vec<u8> {
    encode_envelope(testkind(), b"abc").expect("encode abc")
}

/// A valid zstd envelope over 4096 pattern bytes.
fn zstd_4096() -> Vec<u8> {
    let bytes = encode_envelope(testkind(), &pattern(4096)).expect("encode pattern");
    assert_eq!(u16::from_le_bytes([bytes[10], bytes[11]]), FLAG_ZSTD);
    bytes
}

fn frame(data: &[u8]) -> Vec<u8> {
    zstd::bulk::compress(data, 3).expect("zstd frame")
}

fn decode(bytes: &[u8]) -> Result<Vec<u8>, EnvelopeError> {
    decode_envelope(testkind(), bytes)
}

/// 256 MiB of zeros streamed into one level-3 frame with no content size.
fn bomb_frame() -> Vec<u8> {
    let mut encoder = zstd::stream::write::Encoder::new(Vec::new(), 3).expect("encoder");
    let mut zeros = io::repeat(0).take(256 << 20);
    io::copy(&mut zeros, &mut encoder).expect("stream zeros");
    encoder.finish().expect("finish bomb")
}

/// The 2 MiB pattern streamed with no content size, optionally with a
/// 2^27 window.
fn streamed_pattern(window_log: Option<u32>) -> Vec<u8> {
    let mut encoder = zstd::stream::write::Encoder::new(Vec::new(), 3).expect("encoder");
    encoder
        .include_contentsize(false)
        .expect("content size off");
    if let Some(log) = window_log {
        encoder
            .set_parameter(zstd::zstd_safe::CParameter::WindowLog(log))
            .expect("window log");
    }
    encoder
        .write_all(&pattern(2 << 20))
        .expect("stream pattern");
    encoder.finish().expect("finish pattern")
}

#[test]
fn input_cap_is_checked_first() {
    // Bad magic too: the cap still wins.
    let oversized = vec![0u8; MAX_ENVELOPE_BYTES + 1];
    assert_eq!(decode(&oversized), Err(EnvelopeError::InputTooLarge));
    drop(oversized);

    // Positive control: the max-size envelope, exactly at the cap.
    let payload = xorshift(MAX_PAYLOAD_BYTES);
    let max = encode_envelope(testkind(), &payload).expect("encode max");
    assert_eq!(max.len(), MAX_ENVELOPE_BYTES);
    assert_eq!(decode(&max).expect("decode max"), payload);
}

#[test]
fn every_prefix_is_truncated() {
    for full in [zstd_4096(), raw_abc()] {
        assert!(decode(&full).is_ok(), "positive control");
        for len in 0..full.len() {
            assert_eq!(
                decode(&full[..len]),
                Err(EnvelopeError::Truncated),
                "prefix of length {len} of {}",
                full.len()
            );
        }
    }
}

#[test]
fn bad_magic_each_byte() {
    let good = raw_abc();
    assert_eq!(decode(&good).expect("positive control"), b"abc");
    for at in 0..8 {
        let mut bytes = good.clone();
        bytes[at] ^= 0x01;
        assert_eq!(decode(&bytes), Err(EnvelopeError::BadMagic), "byte {at}");
    }
    // Step 2 precedes step 3: nine bytes are truncated whatever they hold.
    assert_eq!(decode(&[0xa5u8; 9]), Err(EnvelopeError::Truncated));
}

#[test]
fn cross_version_refused_before_layout() {
    // Positive controls: the version-1 bare prefix gets past step 4 and stops
    // at the header-length check; the full envelope decodes.
    let mut v1_prefix = b"CRPGSAVE".to_vec();
    v1_prefix.extend_from_slice(&1u16.to_le_bytes());
    assert_eq!(decode(&v1_prefix), Err(EnvelopeError::Truncated));
    assert_eq!(decode(&raw_abc()).expect("positive control"), b"abc");

    for version in [0u16, 2, 0x0100, u16::MAX] {
        let mut prefix = b"CRPGSAVE".to_vec();
        prefix.extend_from_slice(&version.to_le_bytes());
        assert_eq!(prefix.len(), 10);
        assert_eq!(
            decode(&prefix),
            Err(EnvelopeError::UnsupportedVersion),
            "bare prefix, version {version:#06x}"
        );

        let mut full = raw_abc();
        set_u16(&mut full, 8, version);
        if version == 2 {
            // Unknown flags and a wrong kind too: the version still wins.
            set_u16(&mut full, 10, 0x8000);
            full[12..20].copy_from_slice(b"OTHERKND");
        }
        resign(&mut full);
        assert_eq!(
            decode(&full),
            Err(EnvelopeError::UnsupportedVersion),
            "full envelope, version {version:#06x}"
        );
    }
}

#[test]
fn unknown_flag_bits() {
    let good = raw_abc();
    assert_eq!(decode(&good).expect("positive control"), b"abc");
    for bit in 1..16 {
        let mut bytes = good.clone();
        set_u16(&mut bytes, 10, 1u16 << bit);
        resign(&mut bytes);
        assert_eq!(
            decode(&bytes),
            Err(EnvelopeError::UnknownFlags),
            "bit {bit}"
        );
    }
}

#[test]
fn kind_mismatch() {
    let good = raw_abc();
    assert_eq!(decode(&good).expect("positive control"), b"abc");
    assert_eq!(
        decode_envelope(kind(*b"OTHERKND"), &good),
        Err(EnvelopeError::KindMismatch)
    );

    // A header kind outside the alphabet can never match.
    let mut lower = good.clone();
    lower[12..20].copy_from_slice(b"abcdefgh");
    resign(&mut lower);
    assert_eq!(decode(&lower), Err(EnvelopeError::KindMismatch));
}

#[test]
fn declared_payload_over_cap() {
    let good = build(0, TESTKIND, 3, b"abc");
    assert_eq!(decode(&good).expect("positive control"), b"abc");
    for declared in [MAX_PAYLOAD_BYTES as u64 + 1, u64::MAX] {
        for flags in [0, FLAG_ZSTD] {
            let bytes = build(flags, TESTKIND, declared, b"abc");
            assert_eq!(
                decode(&bytes),
                Err(EnvelopeError::PayloadTooLarge),
                "declared {declared}, flags {flags}"
            );
        }
    }
}

#[test]
fn inconsistent_lengths() {
    let raw = raw_abc();
    assert_eq!(decode(&raw).expect("raw positive control"), b"abc");
    for body_len in [2u64, 4] {
        let mut bytes = raw.clone();
        set_u64(&mut bytes, 28, body_len);
        resign(&mut bytes);
        assert_eq!(
            decode(&bytes),
            Err(EnvelopeError::LengthMismatch),
            "raw body_len {body_len}"
        );
    }

    let packed = zstd_4096();
    assert_eq!(
        decode(&packed).expect("zstd positive control"),
        pattern(4096)
    );
    for body_len in [0u64, 4096, 4097] {
        let mut bytes = packed.clone();
        set_u64(&mut bytes, 28, body_len);
        resign(&mut bytes);
        assert_eq!(
            decode(&bytes),
            Err(EnvelopeError::LengthMismatch),
            "zstd body_len {body_len}"
        );
    }
}

#[test]
fn declared_body_vs_input() {
    for good in [raw_abc(), zstd_4096()] {
        assert!(decode(&good).is_ok(), "positive control");

        let mut longer = good.clone();
        longer.push(0);
        assert_eq!(decode(&longer), Err(EnvelopeError::TrailingBytes));

        let shorter = &good[..good.len() - 1];
        assert_eq!(decode(shorter), Err(EnvelopeError::Truncated));
    }
}

#[test]
fn digest_covers_header_and_body() {
    for good in [raw_abc(), zstd_4096()] {
        assert!(decode(&good).is_ok(), "positive control");

        // Digest and body bytes.
        for at in (36..68).chain(HEADER_BYTES..good.len()) {
            let mut bytes = good.clone();
            bytes[at] ^= 0x01;
            assert_eq!(
                decode(&bytes),
                Err(EnvelopeError::ChecksumMismatch),
                "byte {at}"
            );
        }

        // Kind bytes, decoded with the matching flipped kind, so only the
        // digest can object.
        for at in 0..8 {
            let mut tag = TESTKIND;
            tag[at] ^= 0x01;
            let flipped = PayloadKind::new(tag).expect("flipped TESTKIND stays in the alphabet");
            let mut bytes = good.clone();
            bytes[12 + at] ^= 0x01;
            assert_eq!(
                decode_envelope(flipped, &bytes),
                Err(EnvelopeError::ChecksumMismatch),
                "kind byte {at}"
            );
        }
    }
}

#[test]
fn corrupt_frame_with_valid_digest() {
    let payload = pattern(4096);
    let good = frame(&payload);
    let declared = payload.len() as u64;
    assert!((good.len() as u64) < declared);
    assert_eq!(
        decode(&build(FLAG_ZSTD, TESTKIND, declared, &good)).expect("positive control"),
        payload
    );

    // Middle frame bytes overwritten. The 4096-byte frame's middle is raw
    // literals, which zstd copies without any structure to check (catching
    // that is the digest's job), so this case uses the multi-block 800,000-byte
    // pattern frame and overwrites its middle half with 0xff. That span holds
    // block headers and sequence sections, which zstd does validate.
    let big = pattern(800_000);
    let big_frame = frame(&big);
    let big_declared = big.len() as u64;
    assert!((big_frame.len() as u64) < big_declared);
    assert_eq!(
        decode(&build(FLAG_ZSTD, TESTKIND, big_declared, &big_frame)).expect("800 KB control"),
        big
    );
    let mut smashed = big_frame.clone();
    let quarter = smashed.len() / 4;
    for byte in &mut smashed[quarter..3 * quarter] {
        *byte = 0xff;
    }
    assert_eq!(
        decode(&build(FLAG_ZSTD, TESTKIND, big_declared, &smashed)),
        Err(EnvelopeError::CorruptBody),
        "middle bytes overwritten"
    );

    // A second frame inside the body; body_len follows the body.
    let mut two = good.clone();
    two.extend_from_slice(&good);
    assert!((two.len() as u64) < declared);
    assert_eq!(
        decode(&build(FLAG_ZSTD, TESTKIND, declared, &two)),
        Err(EnvelopeError::CorruptBody),
        "second frame"
    );

    // One trailing byte after the frame inside the body.
    let mut trailing = good.clone();
    trailing.push(0);
    assert_eq!(
        decode(&build(FLAG_ZSTD, TESTKIND, declared, &trailing)),
        Err(EnvelopeError::CorruptBody),
        "trailing byte"
    );

    // A frame needing a 2^27 window is refused by the window cap; the same
    // pattern with the default window decodes.
    let expected = pattern(2 << 20);
    let declared = expected.len() as u64;
    let default_window = streamed_pattern(None);
    assert!((default_window.len() as u64) < declared);
    assert_eq!(
        decode(&build(FLAG_ZSTD, TESTKIND, declared, &default_window)).expect("default window"),
        expected
    );
    let wide_window = streamed_pattern(Some(27));
    assert!((wide_window.len() as u64) < declared);
    // The wide frame itself is valid: zstd with a 2^27 ceiling decodes it.
    let mut wide_decoder = zstd::stream::read::Decoder::with_buffer(&wide_window[..])
        .expect("decoder")
        .single_frame();
    wide_decoder.window_log_max(27).expect("window 27");
    let mut wide_out = Vec::new();
    wide_decoder
        .read_to_end(&mut wide_out)
        .expect("decode with 2^27 window");
    assert_eq!(wide_out, expected);
    assert_eq!(
        decode(&build(FLAG_ZSTD, TESTKIND, declared, &wide_window)),
        Err(EnvelopeError::CorruptBody),
        "window 2^27"
    );
}

#[test]
fn short_frames() {
    let payload = pattern(1000);
    let body = frame(&payload);
    assert_eq!(
        decode(&build(FLAG_ZSTD, TESTKIND, 1000, &body)).expect("positive control"),
        payload
    );
    assert_eq!(
        decode(&build(FLAG_ZSTD, TESTKIND, 1001, &body)),
        Err(EnvelopeError::DecompressedTooShort)
    );

    let skippable = [0x50, 0x2a, 0x4d, 0x18, 0x00, 0x00, 0x00, 0x00];
    assert_eq!(
        decode(&build(FLAG_ZSTD, TESTKIND, 16, &skippable)),
        Err(EnvelopeError::DecompressedTooShort)
    );
}

#[test]
fn decompression_bomb_stops_at_declared_length() {
    let bomb = bomb_frame();
    assert!(bomb.len() < 64 * 1024, "bomb frame is {} bytes", bomb.len());
    assert_eq!(
        decode(&build(FLAG_ZSTD, TESTKIND, MAX_PAYLOAD_BYTES as u64, &bomb)),
        Err(EnvelopeError::DecompressedTooLarge)
    );

    // Boundary pair.
    let exact = pattern(1024);
    assert_eq!(
        decode(&build(FLAG_ZSTD, TESTKIND, 1024, &frame(&exact))).expect("1024 declared 1024"),
        exact
    );
    assert_eq!(
        decode(&build(FLAG_ZSTD, TESTKIND, 1024, &frame(&pattern(1025)))),
        Err(EnvelopeError::DecompressedTooLarge)
    );
}

#[test]
fn decode_is_pure() {
    let raw = raw_abc();
    let packed = zstd_4096();
    let good_frame = frame(&pattern(4096));

    let mut cases: Vec<(PayloadKind, Vec<u8>, EnvelopeError)> = Vec::new();
    cases.push((
        testkind(),
        vec![0u8; MAX_ENVELOPE_BYTES + 1],
        EnvelopeError::InputTooLarge,
    ));
    cases.push((testkind(), raw[..9].to_vec(), EnvelopeError::Truncated));
    cases.push((
        testkind(),
        packed[..packed.len() - 1].to_vec(),
        EnvelopeError::Truncated,
    ));
    let mut bad_magic = raw.clone();
    bad_magic[0] ^= 0x01;
    cases.push((testkind(), bad_magic, EnvelopeError::BadMagic));
    let mut v2 = raw.clone();
    set_u16(&mut v2, 8, 2);
    resign(&mut v2);
    cases.push((testkind(), v2, EnvelopeError::UnsupportedVersion));
    let mut flag = raw.clone();
    set_u16(&mut flag, 10, 0x0002);
    resign(&mut flag);
    cases.push((testkind(), flag, EnvelopeError::UnknownFlags));
    cases.push((kind(*b"OTHERKND"), raw.clone(), EnvelopeError::KindMismatch));
    cases.push((
        testkind(),
        build(0, TESTKIND, u64::MAX, b"abc"),
        EnvelopeError::PayloadTooLarge,
    ));
    let mut lengths = raw.clone();
    set_u64(&mut lengths, 28, 4);
    resign(&mut lengths);
    cases.push((testkind(), lengths, EnvelopeError::LengthMismatch));
    let mut trailing = raw.clone();
    trailing.push(0);
    cases.push((testkind(), trailing, EnvelopeError::TrailingBytes));
    let mut digest = packed.clone();
    digest[40] ^= 0x01;
    cases.push((testkind(), digest, EnvelopeError::ChecksumMismatch));
    let mut two = good_frame.clone();
    two.extend_from_slice(&good_frame);
    cases.push((
        testkind(),
        build(FLAG_ZSTD, TESTKIND, 4096, &two),
        EnvelopeError::CorruptBody,
    ));
    cases.push((
        testkind(),
        build(FLAG_ZSTD, TESTKIND, 4097, &good_frame),
        EnvelopeError::DecompressedTooShort,
    ));
    cases.push((
        testkind(),
        build(FLAG_ZSTD, TESTKIND, 4095, &good_frame),
        EnvelopeError::DecompressedTooLarge,
    ));
    cases.push((
        testkind(),
        build(FLAG_ZSTD, TESTKIND, MAX_PAYLOAD_BYTES as u64, &bomb_frame()),
        EnvelopeError::DecompressedTooLarge,
    ));

    for (expected, bytes, error) in &cases {
        assert_eq!(
            decode_envelope(*expected, bytes),
            Err(*error),
            "first decode"
        );
        assert_eq!(
            decode_envelope(*expected, bytes),
            Err(*error),
            "second decode"
        );
    }
}

#[test]
fn error_display_strings_are_pinned() {
    let envelope = [
        (
            EnvelopeError::InputTooLarge,
            "save input exceeds its byte cap",
        ),
        (EnvelopeError::Truncated, "save input is truncated"),
        (EnvelopeError::BadMagic, "not a save envelope"),
        (
            EnvelopeError::UnsupportedVersion,
            "unsupported save format version",
        ),
        (EnvelopeError::UnknownFlags, "unknown save envelope flags"),
        (EnvelopeError::KindMismatch, "save payload kind mismatch"),
        (
            EnvelopeError::PayloadTooLarge,
            "save payload exceeds its byte cap",
        ),
        (
            EnvelopeError::LengthMismatch,
            "save envelope lengths are inconsistent",
        ),
        (
            EnvelopeError::TrailingBytes,
            "save input has trailing bytes",
        ),
        (EnvelopeError::ChecksumMismatch, "save checksum mismatch"),
        (
            EnvelopeError::DecompressedTooLarge,
            "decompressed save exceeds its declared length",
        ),
        (
            EnvelopeError::DecompressedTooShort,
            "decompressed save is shorter than its declared length",
        ),
        (
            EnvelopeError::CorruptBody,
            "compressed save body is corrupt",
        ),
        (EnvelopeError::Compressor, "save compressor failed"),
    ];
    for (error, text) in envelope {
        assert_eq!(error.to_string(), text);
        assert_eq!(FileError::Envelope(error).to_string(), error.to_string());
        assert_eq!(FileError::from(error), FileError::Envelope(error));
    }

    let ops = [
        (FileOp::CreateTemp, "create-temp"),
        (FileOp::WriteTemp, "write-temp"),
        (FileOp::SyncTemp, "sync-temp"),
        (FileOp::Rename, "rename"),
        (FileOp::SyncDir, "sync-dir"),
        (FileOp::Open, "open"),
        (FileOp::Read, "read"),
    ];
    for (op, text) in ops {
        assert_eq!(op.to_string(), text);
    }

    assert_eq!(
        FileError::InvalidPath.to_string(),
        "save path has no file name"
    );
    let open = FileError::Io {
        op: FileOp::Open,
        kind: io::ErrorKind::NotFound,
    };
    assert!(open.to_string().starts_with("save open failed: "));
    assert_eq!(
        open.to_string(),
        format!("save open failed: {}", io::ErrorKind::NotFound)
    );
}

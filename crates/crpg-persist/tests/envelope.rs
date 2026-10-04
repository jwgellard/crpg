//! T038 §8 `tests/envelope.rs`: layout, round trips, the compression
//! decision, cap boundaries, determinism, the linked zstd and the
//! `PayloadKind` alphabet. Everything enters through the public API.

use crpg_persist::{
    decode_envelope, encode_envelope, EnvelopeError, PayloadKind, FLAG_ZSTD, HEADER_BYTES,
    MAX_ENVELOPE_BYTES, MAX_PAYLOAD_BYTES,
};

/// The digest context, written literally so the oracle does not depend on
/// the crate constant.
const LITERAL_CONTEXT: &str = "crpg-persist 2026-10-04 save envelope v1";

fn testkind() -> PayloadKind {
    PayloadKind::new(*b"TESTKIND").expect("TESTKIND is in the alphabet")
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

fn hex(text: &str) -> Vec<u8> {
    let digits: Vec<u8> = text.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    assert_eq!(digits.len() % 2, 0, "odd hex length");
    digits
        .chunks(2)
        .map(|pair| {
            let s = std::str::from_utf8(pair).expect("ascii");
            u8::from_str_radix(s, 16).expect("hex digit")
        })
        .collect()
}

fn flags(bytes: &[u8]) -> u16 {
    u16::from_le_bytes([bytes[10], bytes[11]])
}

fn payload_len(bytes: &[u8]) -> u64 {
    u64::from_le_bytes(bytes[20..28].try_into().expect("8 bytes"))
}

fn body_len(bytes: &[u8]) -> u64 {
    u64::from_le_bytes(bytes[28..36].try_into().expect("8 bytes"))
}

/// Independent digest oracle over `bytes[0..36] ‖ body`.
fn oracle_digest(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new_derive_key(LITERAL_CONTEXT);
    hasher.update(&bytes[..36]);
    hasher.update(&bytes[HEADER_BYTES..]);
    *hasher.finalize().as_bytes()
}

#[test]
fn raw_envelope_matches_hand_built_layout() {
    let literal = hex(
        "4352504753415645 0100 0000 544553544b494e44 0300000000000000 0300000000000000
         6cc46ec6f5dd5d790128e477e33004b396f0574fa33cff5796050dc0b3bb3f47 616263",
    );
    assert_eq!(literal.len(), 71);

    let encoded = encode_envelope(testkind(), b"abc").expect("encode abc");
    assert_eq!(encoded, literal);

    // The literal digest is reproduced by an independent BLAKE3 computation.
    assert_eq!(oracle_digest(&literal)[..], literal[36..68]);

    assert_eq!(
        decode_envelope(testkind(), &encoded).expect("decode abc"),
        b"abc"
    );
}

#[test]
fn empty_payload_round_trips_raw() {
    let encoded = encode_envelope(testkind(), b"").expect("encode empty");
    assert_eq!(encoded.len(), 68);
    assert_eq!(flags(&encoded), 0);
    assert_eq!(payload_len(&encoded), 0);
    assert_eq!(body_len(&encoded), 0);
    assert_eq!(
        encoded[36..68],
        hex("39afd20fc7950456a211ba0d3a16c9726a20baa99501da8e660ab469719745ea")[..]
    );
    assert_eq!(oracle_digest(&encoded)[..], encoded[36..68]);
    assert_eq!(
        decode_envelope(testkind(), &encoded).expect("decode empty"),
        b""
    );
}

#[test]
fn compressible_payload_uses_zstd() {
    let payload = pattern(800_000);
    let encoded = encode_envelope(testkind(), &payload).expect("encode pattern");
    assert_eq!(flags(&encoded), FLAG_ZSTD);
    assert_eq!(payload_len(&encoded), 800_000);
    assert!(body_len(&encoded) < payload_len(&encoded));
    assert_eq!(
        encoded.len() as u64,
        HEADER_BYTES as u64 + body_len(&encoded)
    );
    assert_eq!(
        encoded[HEADER_BYTES..HEADER_BYTES + 4],
        [0x28, 0xb5, 0x2f, 0xfd]
    );
    assert_eq!(
        decode_envelope(testkind(), &encoded).expect("decode pattern"),
        payload
    );
}

#[test]
fn incompressible_payload_stored_raw() {
    let payload = xorshift(1 << 20);
    let encoded = encode_envelope(testkind(), &payload).expect("encode xorshift");
    assert_eq!(flags(&encoded), 0);
    assert_eq!(payload_len(&encoded), 1 << 20);
    assert_eq!(body_len(&encoded), 1 << 20);
    assert_eq!(encoded[HEADER_BYTES..], payload[..]);
    assert_eq!(
        decode_envelope(testkind(), &encoded).expect("decode xorshift"),
        payload
    );
}

#[test]
fn payload_cap_boundaries() {
    // The largest compressible payload round-trips through zstd.
    let max_pattern = pattern(MAX_PAYLOAD_BYTES);
    let encoded = encode_envelope(testkind(), &max_pattern).expect("encode max pattern");
    assert_eq!(flags(&encoded), FLAG_ZSTD);
    assert_eq!(
        decode_envelope(testkind(), &encoded).expect("decode max pattern"),
        max_pattern
    );
    drop(max_pattern);

    // The largest incompressible payload is stored raw and exactly fills the
    // input cap, which is accepted.
    let max_random = xorshift(MAX_PAYLOAD_BYTES);
    let encoded = encode_envelope(testkind(), &max_random).expect("encode max xorshift");
    assert_eq!(flags(&encoded), 0);
    assert_eq!(encoded.len(), MAX_ENVELOPE_BYTES);
    assert_eq!(
        decode_envelope(testkind(), &encoded).expect("decode max xorshift"),
        max_random
    );
    drop(max_random);

    // One byte over the payload cap is refused before any compression.
    let over = vec![0u8; MAX_PAYLOAD_BYTES + 1];
    assert_eq!(
        encode_envelope(testkind(), &over),
        Err(EnvelopeError::PayloadTooLarge)
    );
}

#[test]
fn encode_is_deterministic() {
    let payloads: [Vec<u8>; 4] = [
        Vec::new(),
        b"abc".to_vec(),
        pattern(800_000),
        xorshift(1 << 20),
    ];
    for payload in &payloads {
        let first = encode_envelope(testkind(), payload).expect("first encode");
        let second = encode_envelope(testkind(), payload).expect("second encode");
        assert_eq!(
            first,
            second,
            "two encodes of {} bytes differ",
            payload.len()
        );
        let decoded = decode_envelope(testkind(), &first).expect("decode");
        assert_eq!(&decoded, payload);
        let again = encode_envelope(testkind(), &decoded).expect("re-encode");
        assert_eq!(again, first, "encode/decode/encode is not a fixed point");
    }
}

#[test]
fn linked_zstd_is_bundled_1_5_7() {
    assert_eq!(zstd::zstd_safe::version_number(), 10507);
}

#[test]
fn payload_kind_alphabet() {
    for tag in [*b"HOSTCKPT", *b"A_9_____", *b"ZZZZZZZZ"] {
        let kind = PayloadKind::new(tag).expect("tag in the alphabet");
        assert_eq!(kind.as_bytes(), &tag);
    }
    for bad in [b'a', b' ', 0x00, 0x7f, 0x80, b'-'] {
        for at in 0..8 {
            let mut tag = *b"HOSTCKPT";
            tag[at] = bad;
            assert_eq!(
                PayloadKind::new(tag),
                None,
                "byte {bad:#04x} at {at} was accepted"
            );
        }
    }
}

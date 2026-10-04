# crpg-persist — agent contract

Read the root rules, [POST-T018-RULES](../../tasks/POST-T018-RULES.md) and
[T038](../../tasks/T038.md) (exact public API, envelope v1 byte layout,
decode precedence, streaming decompression bound, file-store steps, the §9
dependency audit and the approval). Architecture:
[crpg-persist](../../docs/architecture/crpg-persist.md). This document
describes the T038 save envelope and atomic file store.

## Public surface

Modules `envelope` and `file`. Every item below is re-exported at the root by
name (no glob re-export). T038 §2 is exhaustive: adding a public item, a
trait, a header struct, an inspection API, a streaming writer or a
configurable level or cap needs a new approved contract.

- `envelope`: `SAVE_MAGIC` (`CRPGSAVE`), `SAVE_FORMAT_VERSION` (1),
  `HEADER_BYTES` (68), `FLAG_ZSTD` (bit 0), `MAX_PAYLOAD_BYTES` (16,777,216),
  `MAX_ENVELOPE_BYTES` (16,777,284), `ZSTD_LEVEL` (3), `ZSTD_WINDOW_LOG_MAX`
  (24), `DIGEST_CONTEXT`; `PayloadKind` (`new` accepts only `A-Z`, `0-9` and
  `_`; `as_bytes`); `encode_envelope`, `decode_envelope`; `EnvelopeError`
  (14 variants, pinned lowercase `Display`, no source).
- `file`: `TEMP_SUFFIX` (`.crpg-tmp`), `save_file`, `load_file`,
  `read_capped`; `FileOp` (7 steps, pinned `Display`); `FileError`
  (`Envelope`, `InvalidPath`, `Io { op, kind }`), `From<EnvelopeError>`.

## Invariants

1. **Payload-neutral.** The envelope never encodes or inspects a `World`,
   `HistoryWorld` or checkpoint, and the crate has no workspace path
   dependency. The unit of save belongs to the caller (T038 §1).
2. **Frozen prefix.** Offsets 0..10 (magic and version) mean the same in
   every version. The version is checked before the header length, flags,
   lengths or digest, so a foreign version of any length is
   `UnsupportedVersion`.
3. **Precedence is contract.** Decode order is input cap, prefix length,
   magic, version, header length, flags, kind, declared payload cap, lengths
   against flags, body against input, digest, then the payload. First
   failure wins. Do not reorder the steps. Do not merge two variants.
4. **Both caps hold.** Encode refuses `payload.len() > MAX_PAYLOAD_BYTES`
   before compressing. Decode refuses input over `MAX_ENVELOPE_BYTES` before
   reading anything else. `read_capped` reads at most cap + 1 bytes and
   never consults file metadata. The decompressed cap is the declared
   `payload_len`, enforced chunk by chunk *before* appending.
5. **No sizing from untrusted fields.** The output vector starts empty and
   grows by appending. The one scratch buffer is `DECODE_CHUNK_BYTES`
   (64 KiB). Neither `payload_len` nor the frame's content size ever reaches
   a `with_capacity`.
6. **Encoder invariants match the decoder.** Raw body iff the frame is not
   strictly shorter, so `body_len == payload_len` (raw) or
   `1 <= body_len < payload_len` (zstd). An envelope is never longer than
   `HEADER_BYTES + payload.len()`.
7. **Pinned zstd.** Bundled zstd 1.5.7 through `zstd =0.14.0` with default
   features off; `linked_zstd_is_bundled_1_5_7` asserts
   `version_number() == 10507`. A fresh compression context per call, level
   3, no zstd checksum, no dictionary id, content size on, everything else
   default.
8. **Atomic replace.** `save_file` encodes before touching the filesystem.
   It writes, syncs and renames a sibling temp file, and removes the temp
   (best effort) when write, sync or rename fails. Directory sync is
   `#[cfg(unix)]` only, the one OS-specific branch. `load_file` never
   writes, renames or deletes.
9. **Determinism scope.** Byte equality is per build and per target. No
   clock, environment read, randomness, threads or global state, in source
   or tests. Test scratch paths come from the compile-time
   `env!("CARGO_TARGET_TMPDIR")`.

## Allowed dependencies

Exactly T038 §7, and nothing else:

```toml
[dependencies]
blake3 = { workspace = true }
zstd = { version = "=0.14.0", default-features = false }
```

No `[dev-dependencies]`, no `[build-dependencies]`, no `build.rs`. Tests reach
`blake3`, `zstd` and `zstd::zstd_safe` through these normal edges.
`crpg-core`/`crpg-data`/`crpg-sim` are permitted by `deps.py` but are not
approved edges. Any new edge, version change or zstd feature needs a
T018a-style record and explicit approval first, and `deny.toml` is never
widened to pass. The crate root keeps the root `AGENTS.md` forbid attribute.
Containers are ordered or plain `Vec`, and arithmetic is integer only.

## Definition of done for any change

```text
cargo fmt --all
cargo test -p crpg-persist --test envelope --locked
cargo test -p crpg-persist --test rejections --locked
cargo test -p crpg-persist --test file_store --locked
cargo clippy -p crpg-persist --all-targets --locked -- -D warnings
cargo test -p crpg-persist --locked
rg -n "<T038 §10 pattern>" crates/crpg-persist
```

Then run the root/POST-T018 common gates on both native targets, including
`cargo deny check` with `deny.toml` unchanged. The §10 scan must print only
the crate root's forbid line. `determinism.py` does not cover this crate, so
the scan is the check. This file must not trip that scan either.

## Known traps

- **Never** `Vec::with_capacity(payload_len)` or a capacity taken from the
  frame header. A 68-byte header must not reserve 16 MiB.
- **Never** enable the zstd features `zstdmt`, `pkg-config`, `legacy`,
  `bindgen`, `cmake`, `experimental` or `zdict_builder`, and never set
  `ZSTD_SYS_USE_PKG_CONFIG`. A system `libzstd` can change compressed bytes,
  and legacy decoders widen the attack surface.
- **Never** reuse a compressor or decompressor context across calls. That
  would make output depend on call history.
- **Never** compare compressed bytes across targets, or add a cross-target
  golden. The two raw-envelope digests in `tests/envelope.rs` are
  specification vectors (no zstd output), so they hold on every target.
- The test digest oracle uses the **literal** context string, not
  `DIGEST_CONTEXT`. Keep it independent.
- `zstd::stream::read::Decoder::finish` hands back the remaining body. Bytes
  left after the single frame are `CorruptBody`, and that check runs only
  after the short-output check, so a second frame with a larger declared
  length reads as `DecompressedTooShort`.
- A frame's raw-literal section can be altered without zstd noticing. That
  is the digest's job. `corrupt_frame_with_valid_digest` therefore corrupts
  block structure in a multi-block frame, not literals.
- Two concurrent `save_file` calls to one path share a temp name. Single
  writer per path is the caller's obligation.

## Agent log

- 2026-10-04 (UTC) · claude-code + T038 · Wrote the crate's first working contract with its first code: the exact surface, the precedence and cap invariants, the pinned zstd tree and the traps, so T039 and later backends extend the envelope rather than reinterpret it.

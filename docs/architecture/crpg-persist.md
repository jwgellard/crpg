# crpg-persist architecture

## Scope

`crpg-persist` turns caller bytes into a save file and back. Spec §8 gives the
crate save/load. D10 and T022 §7 hand it the production half: the filesystem,
compression, and separate compressed and decompressed size caps.

What exists today is the first slice, specified by [T038](../../tasks/T038.md):

- a versioned, checksummed, `zstd`-compressed **save envelope** around opaque
  payload bytes;
- the S001 **input-size and decompressed-size caps**, with the decompressed
  cap enforced while zstd is still producing bytes;
- an **atomic-replace file store** over that envelope.

There is no `PersistenceBackend` trait, no `SqliteBackend`, no save-slot
policy and no typed `World`/`HistoryWorld` helper. Each of those is planned
work (see below), not a placeholder.

## Where it sits

Normal dependencies are workspace `blake3` and `zstd =0.14.0` with default
features off, pinned in the crate manifest. That pin, its tree
(`zstd-safe` 8.0.0, `zstd-sys` 2.1.0 bundling zstd 1.5.7 C, plus build-only
`pkg-config` and `jobserver`) and the build-time C compilation were approved
under D27b with the T038 §9 audit ([approval](../../tasks/T038.md#approval--2026-10-04-utc)).
There are no dev-dependencies and no build script of its own.

`tools/lint/deps.py`'s `ALLOWED` row permits `crpg-core`, `crpg-data` and
`crpg-sim`. That is a ceiling, not a current edge: T038 deliberately takes no
workspace path dependency (next section). The first consumer is planned to be
T039, a `crpg-server` adapter.

## The payload-neutral boundary

The envelope carries **opaque caller bytes** and never encodes, decodes or
inspects a `World`, `HistoryWorld` or checkpoint. T038 §1 gives the reasons
in full. In short:

- T022 §7 owns the unit of save. That is the whole host checkpoint: the
  `HistoryWorld` wrapper plus the capture journal, watermark, incarnation and
  protocol selection, as compact JSON built in `crpg-server`. Persist cannot
  import server or net, so it can carry that unit but cannot build it.
- A `World`-only helper would invite a round trip that T022 says is not
  continuation evidence.
- The sim serializers and their host pre-parse cap already exist
  ([crpg-sim](crpg-sim.md)). A second encoding here could drift from them.

**Spec §8 deviation.** Spec §8 says snapshots are "serialized with
`postcard`". T038 does not do this: the payload encoding belongs to the
caller, and T022 chose compact JSON. The user accepted this deviation at T038
approval (§12 Q7). The spec text is unchanged; a later documentation task may
append a dated note to it.

Save identity (engine version, campaign id and version, schema version) also
stays inside the caller's payload (§12 Q5). The envelope records only its own
format version and an 8-byte `PayloadKind` tag.

## Module flow

Two modules, `envelope` and `file`. Every public item is re-exported at the
crate root by name.

- **`envelope`.** `encode_envelope(kind, payload)` refuses a payload over
  `MAX_PAYLOAD_BYTES`. It then compresses with a fresh level-3 zstd context
  (no zstd checksum, no dictionary id, content size on) and keeps the frame
  only if it is strictly shorter than the payload. Last, it writes the
  68-byte header and the body. `decode_envelope(expected, bytes)` checks the
  input cap, the frozen magic-and-version prefix, the header, flags, kind,
  declared lengths, the body length against the input, and then the BLAKE3
  digest. A zstd body is decompressed by a streaming loop that stops as soon
  as output would pass the declared `payload_len`. The byte layout is
  T038 §3. The precedence is §4.2 and the streaming bound is §4.3.
- **`file`.** `save_file` encodes first, so a refused payload never touches
  the filesystem. It then writes `<name>.crpg-tmp` beside the destination,
  syncs it, renames it over the destination and, on Unix only, syncs the
  parent directory. `load_file` opens the file, reads through `read_capped`
  (at most `MAX_ENVELOPE_BYTES + 1` bytes; metadata length is never trusted)
  and decodes. The exact steps are T038 §6.

The digest is an integrity check, not authentication: anyone who can write
the file can re-sign it. The decompression caps are the safety boundary.

## Determinism

Encoding is a pure function of `(kind, payload)` for a fixed build: the
bundled zstd 1.5.7 (a test asserts the linked version), level 3 and the
pinned parameters. Byte equality is claimed **per build and per target**
only (ADR-0012). Windows/MSVC and Linux/GNU compressed bytes are never
compared. The two literal vectors in T038 §8 are raw envelopes whose bytes
follow from the layout and BLAKE3 alone, so they are specification vectors,
not goldens, and both targets assert them.

## Guarantees and residuals

- **Atomic replace on both targets.** A later load sees the previous complete
  save or the new one, never a mix. The new contents are fsync'd on both
  targets.
- **Directory durability on Unix only.** On Windows, std cannot open a
  directory to flush it, so the rename's directory entry is not explicitly
  made durable. The user accepted this residual (§12 Q9).
- **Not claimed:** crash-durable exactly-once, write-ahead logging, rolling
  slots, or "never overwrite the only save". Those are T039/host policy
  (spec §8, D10). Each path has one writer; that is the caller's obligation.

## Governing decisions

- D27b ([decisions](../../tasks/DECISIONS-2026-10-04.md)) — `zstd`, with both
  S001 caps mandatory and the decompressed cap enforced during decompression.
- D10 ([decisions](../../tasks/POST-T018-DECISIONS.md)) and T022 §7 — persist
  owns the filesystem and compression; the host owns the unit of save.
- [T038](../../tasks/T038.md) — the exact API, layout, precedence and file
  steps, and the approval of all twelve §12 recommendations.
- ADR-0012 — exact-build, per-target determinism.

## Planned

- **T039** (`crpg-server`): put T022's checkpoint bytes on disk through
  `save_file`/`load_file` under a `PayloadKind` it chooses (for example
  `HOSTCKPT`), decide the checkpoint identity fields and slot policy, and
  turn on capability gate 10. T038 makes no gate-10 claim.
- A `PersistenceBackend` trait in `crpg-contracts`, only once a second backend
  exists (§12 Q6). That is a human-owned change.
- A later `SqliteBackend` and the testkit `assert_persistence_backend` suite
  (spec §8, §15.3).
- A v2 envelope, if ever needed, adds its own decoder and a compatibility rule
  by decision. Offsets 0..10 keep their meaning so any build can refuse it.

## What consumers inherit

A payload-neutral envelope with a frozen version prefix and pinned
first-failure-wins error precedence. Both caps hold: 16 MiB of payload, and
16 MiB + 68 B of input. A zstd window above 2^24 is refused. No allocation is
sized from an untrusted field. On top of that, an atomic-replace file store
with honest durability claims. The envelope's crate-level contract is
[crates/crpg-persist/AGENTS.md](../../crates/crpg-persist/AGENTS.md).

## Agent log

- 2026-10-04 (UTC) · claude-code + T038 · Wrote the crate's first architecture doc with its first code. It covers the envelope, the caps, the file store, the payload-neutral boundary and the spec §8 `postcard` deviation, and lists what T039 and later backends inherit.

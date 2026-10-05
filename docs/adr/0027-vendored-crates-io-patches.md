# ADR-0027: Vendored crates.io patches under `third_party/`

Date: 2026-10-05 (UTC)
Status: **Accepted**. The user approved T023v's contract and dependency
change on 2026-10-05, including Q5, "ADR-0027 records the `third_party/`
vendoring convention". The approval is recorded in
[tasks/T023v.md "Decisions — 2026-10-05
(UTC)"](../../tasks/T023v.md#decisions--2026-10-05-utc).

## Context

[D23](../../tasks/DECISIONS-2026-09-30.md) chose a checksum-pinned vendoring
mechanism for patching quinn-proto. It is used first for the dedup window
patch prepared by T030 (`docs/reviews/T030-quinn/`). D04's rules carry
forward: no git source, no fork URL and no `deny.toml` widening.

T023c found that quinn-proto 0.11.19 holds a locally closed connection's
`CONNECTION_CLOSE` behind its congestion gate. On Windows the peer then sees
a stateless reset instead of the close code (T023c C§1.4). Upstream fixed
this in commit `e556fde` (quinn-rs/quinn#2787, which fixes #2785), but no
crates.io release carries the fix. The user chose to vendor the fix (T023c
Q6, path b), and [T023v](../../tasks/T023v.md) is the first use of the
mechanism.

D23 chose the mechanism. Nothing yet records the rules that must hold every
time it is used. T023v measured those rules (T023v V§1.4), and the deferred
dedup patch will reuse them.

## Decision

1. **Layout.** A vendored crate lives in `third_party/<crate>/`:
   - `VENDOR.md`: provenance, hashes, the re-derivation script, the
     evidence to re-run, rules and retirement, with a signed agent log;
   - `patches/NNNN-*.patch`: numbered patches, applied in file-name order
     with `patch -p1 --no-backup-if-mismatch --fuzz=0`;
   - `crate/`: the crates.io archive, verified against its registry index
     `cksum`, extracted, and changed only by `patches/`. It must reproduce
     byte-for-byte by VENDOR.md's script.

   The workspace uses `crate/` through a `[patch.crates-io]` path entry.
   `[patch.crates-io]` is the last table of the root `Cargo.toml`. The
   dependency's version pin in the consuming crate does not change. No git
   source, no fork URL and no `deny.toml` change (D04, D23).
2. **Admission.**
   - Each patch needs its own H-dep approval, with a T018a-style record.
   - A patch taken from an upstream commit is preferred to a local change.
     Its provenance is recorded and checked against that commit.
3. **Non-scope.** A vendored crate is not a workspace member and gets no
   `exclude` entry, so cargo refuses to run inside `crate/` in the
   repository. Its own suite is run only in scratch copies. Measured:
   - `cargo fmt --all` and clippy do not scan it;
   - `tools/lint/deps.py` and `tools/lint/determinism.py` do not scan it;
   - it is third-party code, like any registry crate.

   Never edit `crate/` by hand, and never run `rustfmt` on it.
4. **Advisories.** `cargo deny` does not report RustSec advisories against a
   crate replaced by a path patch (measured: T023v V§1.4). So the CI `deny`
   job also strips `[patch.crates-io]` in its throwaway checkout and runs
   `cargo deny check advisories` again on the registry graph. This uses the
   same pinned action and adds no job.
5. **Byte-exact storage.** `.gitattributes` sets
   `third_party/** -text -whitespace`. Line endings are never converted, so
   VENDOR.md's hashes hold on Windows checkouts. Patch context lines may be
   whitespace-only.
6. **Retirement.**
   - A patch is removed when the workspace moves, by its own approved
     dependency task, to a release that contains the fix, and the
     patch's evidence passes unpatched.
   - When no patch remains, delete the directory, the `[patch.crates-io]`
     entry and the CI advisories re-check. `Cargo.lock` regains the
     registry `source` and `checksum` lines.
   - D23's dedup patch is planned as quinn-proto's `0002`, a byte-identical
     copy of `docs/reviews/T030-quinn/dedup-window.patch`. It has its own
     H-dep and D23's timing. The two patches retire independently.

## Authority and affected surface

- Decided by: the user, approving T023v (Q4–Q8) on 2026-10-05 under D23.
- Authoritative contract: [tasks/T023v.md](../../tasks/T023v.md) V§3, and
  `third_party/quinn-proto/VENDOR.md` for the vendored crate itself.
- Owning files: `third_party/`, the root `Cargo.toml` `[patch.crates-io]`
  table, `Cargo.lock`, `.gitattributes`, and the `deny` job in
  `.github/workflows/ci.yml`.

## Compatibility, hash and dependency impact

- Persisted/serialized shapes: unchanged.
- Replay, `state_hash`, goldens: unchanged. No workspace member below
  `crpg-net-quic` depends on quinn-proto.
- Dependency graph: no edge added or removed. `tools/lint/deps.py`
  `ALLOWED` is unchanged. For quinn-proto 0.11.19, the source changes from
  the registry to a path. The H-dep record is T023v V§3.9.
- Platform scope (ADR-0012): the same vendored source on Windows/MSVC and
  Linux/GNU. Its byte-exactness is enforced by `.gitattributes`.

## Alternatives considered

- **Git source or fork URL**: rejected by D04 and D23, and `deny.toml`
  denies unknown git sources.
- **Wait for an upstream release**: the 0.11.x branch has no backport, and
  `main` is the unreleased 0.12. Windows stays red until then.
- **A flat `third_party/<crate>/` with VENDOR.md inside the crate** (T030's
  draft): `diff -r` against the archive would then need exclusions.
- **A workspace `exclude`**: it would allow cargo inside `crate/`, which can
  create an in-tree `target/` and rewrite the archive's own `Cargo.lock`.
- **Advisories check documented in VENDOR.md only**: a manual step does not
  run on the weekly schedule that exists to catch new advisories.

## Consequences

- The repository carries third-party source, about 1.3 MB for quinn-proto.
  It must never be reformatted.
- Updating a vendored crate is a re-derivation plus evidence, not a
  `cargo update`.
- The `deny` job's advisories check runs twice.
- Upstream fixes can be adopted ahead of a release, without forks.

## Acceptance and evidence

Recorded in T023v's completion record (`tasks/T023v.md`): VENDOR.md's
re-derivation, the exact `Cargo.lock` diff, `cargo deny check` on both
graphs, and the T023v-a and T023v-b gates on Linux/GNU, plus Windows/MSVC
through CI.

## Supersession and corrections

## Agent log

- 2026-10-05 (UTC) · claude-code + T023v-a · Filed the `third_party/` vendoring convention as Accepted on the user's T023v approval (Q5), so the layout, non-membership, advisories re-check, byte-exact storage and retirement rule D23's later dedup patch will reuse are recorded once, apart from the quinn-proto specifics in VENDOR.md.

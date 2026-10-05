# Vendored quinn-proto 0.11.19

`crate/` is the crates.io archive of quinn-proto 0.11.19, unmodified except
by the patches in `patches/`, applied in file-name order. The workspace
uses it through `[patch.crates-io]` in the root `Cargo.toml` (T023v,
decision D23's vendoring mechanism). It is not a workspace member;
`cargo fmt --all`, clippy, `tools/lint/deps.py` and
`tools/lint/determinism.py` do not scan it. Never edit `crate/` by hand,
never run `rustfmt` on it, and never run cargo inside it in this
repository.

## Source

| Item | Value |
|---|---|
| Package | `quinn-proto` 0.11.19; licence MIT OR Apache-2.0 (`crate/LICENSE-MIT`, `crate/LICENSE-APACHE`, verbatim) |
| Archive | https://static.crates.io/crates/quinn-proto/quinn-proto-0.11.19.crate |
| Archive SHA-256 (= crates.io index `cksum`) | `0e750cca55fe4f0439a15d0bb529da9651e79993e8e72c61a899a36d462befbe` |
| Upstream | https://github.com/quinn-rs/quinn, tag `quinn-proto-0.11.19` → `8192ed399a26a6e8f0aba43d10ec8badaa5b905e` (= `crate/.cargo_vcs_info.json`) |
| Tree digest of `crate/` | `19f5943ca22d8235d3b60365a3622822f9d5ed08c1602e765780381264c21f31` (unpatched archive: `34957658107d6e5d1401e5a84cbdde8008411bf402798c48e21310f9e1ccf37d`) |

## Patches

| Patch (SHA-256) | Change | Upstream | Files (SHA-256 before → after) |
|---|---|---|---|
| `0001-send-connection-close-when-congestion-blocked.patch` (`d4abd4b622ce0b33ec8d12a8bbbeebd06303319986c70a16b3a75d9b7a430450`) | A close packet skips the congestion and pacing gate in `poll_transmit` (it carries only ACK and `CONNECTION_CLOSE`, which are not congestion controlled) | quinn-rs/quinn `e556fde4b5287842e60b26b10bb225ea6174385b` (#2787, fixes #2785), on `main` since 2026-08-27; not in 0.11.19 or the `0.11.x` branch | `src/connection/mod.rs` `dc94645f5ada5beca1ebab0f435eb8ea682fc5f60797db3c75fe43e9a2b451ab` → `ac80f470f3cb8a84439817a16581fca89594115d99a028b188195b4924fbdd3f`; `src/tests/mod.rs` (test only) `178a35edaedb5165175b54bc45f08d95d285eab1677265efc104bcba46fafe5a` → `8ce7157b00c484e0681438767f227d2ca34327b8e9ae438603f210761fffde9e` |

## Re-derive and verify

POSIX `sh` (on Windows, Git Bash). It must print `vendored tree
reproduced`.

```sh
set -eu
REPO=$(git rev-parse --show-toplevel)
WORK=$(mktemp -d) && cd "$WORK"
curl -sSf https://index.crates.io/qu/in/quinn-proto | grep '"vers":"0.11.19"' | grep -o '"cksum":"[0-9a-f]*"'
curl -sSfL -o quinn-proto-0.11.19.crate https://static.crates.io/crates/quinn-proto/quinn-proto-0.11.19.crate
echo "0e750cca55fe4f0439a15d0bb529da9651e79993e8e72c61a899a36d462befbe  quinn-proto-0.11.19.crate" | sha256sum -c -
tar xzf quinn-proto-0.11.19.crate
cd quinn-proto-0.11.19
for p in "$REPO"/third_party/quinn-proto/patches/*.patch; do
  patch -p1 --no-backup-if-mismatch --fuzz=0 -i "$p"
done
cd ..
diff -r quinn-proto-0.11.19 "$REPO/third_party/quinn-proto/crate"
(cd "$REPO/third_party/quinn-proto/crate" && find . -type f -print0 | LC_ALL=C sort -z | xargs -0 sha256sum | sha256sum)
echo "vendored tree reproduced"
```

## Evidence to re-run whenever `crate/` or `patches/` changes

1. The vendored crate's own suite, in a scratch copy, with
   `RUSTUP_TOOLCHAIN=1.98.0` and a fresh `CARGO_TARGET_DIR` per tree. Run
   `cargo test --locked`:
   - patched: 314 passed, doctests 3;
   - with `patch -R` of the code hunk of 0001 only:
     `connection_close_while_congestion_blocked` fails.
2. `cargo test -p crpg-net-quic --test quic --locked`: case 28
   `discard_close_reaches_peer_when_congestion_blocked` passes.
   - With the `[patch.crates-io]` table removed, in a scratch copy, it
     fails with `Reset`.
3. `cargo deny check`, and the advisories re-check on the graph with the
   `[patch.crates-io]` table removed (cargo-deny does not see advisories
   against a path crate).

## Rules

- A change to the vendored code is a new numbered patch plus an updated
  table above, never an edit to `crate/`.
- `patch --no-backup-if-mismatch` always: the archive ships its own
  `Cargo.toml.orig`, so never delete `*.orig` files.
- Keep `[patch.crates-io]` the last table of the root `Cargo.toml`; the
  CI advisories re-check strips from it to the end.
- A RustSec advisory against quinn-proto 0.11.19 is handled like any
  other: upgrade to a fixed release with the full evidence set, and
  re-derive or retire this directory.

## Retirement

Retire patch 0001 when the workspace moves (by its own approved dependency
task) to a quinn-proto release that contains `e556fde` or an equivalent
fix:

- That release's suite passes `connection_close_while_congestion_blocked`
  unpatched.
- Case 28 passes with no `[patch.crates-io]` entry, in 10 Linux runs and in
  the four-run Windows gate.

When no patch remains, delete this directory, the `[patch.crates-io]`
table and the CI advisories re-check. `Cargo.lock` regains the registry
`source` and `checksum` lines. Record the retirement as an addendum to the
ADR.

## Agent log

- 2026-10-05 (UTC) · claude-code + T023v-a · Vendored quinn-proto 0.11.19 with upstream's congestion-gated `CONNECTION_CLOSE` fix as patch 0001, recording the hashes, re-derivation, evidence and retirement rule so the copy can be reproduced and dropped once a release carries the fix.

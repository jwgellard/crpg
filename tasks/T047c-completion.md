# T047c — completion record (local part)

Branch `task/T047c-s1`, Windows, cargo 1.98.0, cargo-deny 0.20.2, Godot 4.7.2.stable.official.ed1daf0bf.

## Status

One contract defect (W4) was found and fixed with the user's approval (see "Defect"). Every local row passes.

## Local results

| Id | Result |
|---|---|
| W1 | BR-21 block: exit 0, "BR-21 passed: no godot package by default; the engine control resolves godot v0.5.5" |
| W2 | first call with `--features crpg-godot/engine`: exit 1, printed the godot family lines and "BR-21 failed" |
| W3 | control pattern `^godot v9`: exit 1, "BR-21 control failed" |
| W4 | as written in the contract: FAIL (exit 1, see Defect). With the fix in the committed ci.yml: exit 0, eight lines |
| W5 | with the fix: `godot-ffi` removed: exit 1 with diff; replaced by `godot-extra v0.5.5`: exit 1 with diff |
| W6 | exit 0; `GITHUB_ENV` holds `GODOT=...\godot-4.7.2\Godot_v4.7.2-stable_win64_console.exe`; version string matched |
| W7 | SHA-256 digit changed: throws `is not the pinned`; SHA-512 digit changed: same; in both, `godot-4.7.2` dir not created |
| W8 | G1 0, G2 0, G3 3, G4 0, G5 3 (all as expected) |
| W9 | `scope blocks identical`; one character changed in the copy: 2 blocks, not equal |
| W10 | `engine build only in client`; `features engine` added to a `check` comment: `bad ['check']` |
| W11 | `cargo deny check`: `advisories ok, bans ok, licenses ok, sources ok` |
| W12 | `godot-ffi` exception removed: `error[rejected]: failed to satisfy license requirements`, `godot-ffi v0.5.5`, `license = "MPL-2.0"`, `licenses FAILED` (exit 4) |
| W13 | `godot-ffi` exception and `[graph]` removed: `licenses ok` (exit 0), with `license-exception-not-encountered` warnings (the exceptions are unmatched) |
| W14 | pending CI |
| W15 | pending CI |

Also: `cargo fmt --all -- --check`, `deps.py`, `determinism.py`, the lint self-tests (81 tests) and
`powershell -File tools/preflight.ps1 -Crate crpg-godot` all pass; `git diff --check` is clean.
G1 left untracked `*.uid` files, deleted before commit. G1's output includes a harmless
Godot warning about shortening `crpg_godot.pdb`; exit 0.

## Defect and fix

`cargo tree --format '{p}'` prints a proc-macro package as `godot-macros v0.5.5 (proc-macro)`.
The contract's notice step stripped only a trailing ` (*)`, so the resolved set held that line and
never equalled the notice (W4 exit 1 although the count was eight). Fix, approved by the user:
strip any trailing parenthesised annotation, `sed -E 's/ \([^)]*\)$//'`, replacing
`sed 's/ (\*)$//'`. After the fix: W4 exit 0 with eight lines; W5 godot-ffi removed exit 1,
godot-extra swapped in exit 1. `tasks/T047c.md` is untouched; the manager adds the contract note.
The BR-21 step is unaffected.

## Pending CI (W14/W15)

Per-step wall times of the `client` job, artifact file list and size, run id: to be filled by the
manager after the push.

## Agent log

- 2026-11-02 (UTC) · claude-code/sonnet-5.5 + T047c · Recorded the local W1-W13 results and the W4 defect found in the contract's notice-check step, with a verified one-line fix.

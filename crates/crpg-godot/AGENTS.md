# crpg-godot — agent contract

Design: [`docs/architecture/crpg-godot.md`](../../docs/architecture/crpg-godot.md)
— what the crate is and how its pieces fit. This file is the working contract.
Read ADR-0033 and [T047](../../tasks/T047.md) §"Exact contract" before editing.
T047-1 landed `core/errors.rs`, `core/link.rs` and `core/client.rs`; T047-2
landed `core/authority.rs`, `BridgeCore`, the campaign walk and the
integration tests; T047b owns the shell.

## Invariants

1. **The core is Godot-free.** No `godot` import, no FFI, no `unsafe` below
   `shell.rs`; `src/core/mod.rs` begins with `#![forbid(unsafe_code)]`. The
   crate root keeps `#![allow(unsafe_code)]` only for T047b's macro expansion,
   plus `#![warn(missing_docs)]`.
2. **One authority, one in-memory link.** The client half only reads bytes
   delivered by `LocalLink` and sends encoded intents back. It never touches a
   `World`, host state or another peer.
3. **`std::`, never `core::`.** The `core` module shadows the `core` crate at
   the crate root; a bare `core::` path is a bug.
4. **A bad frame is fatal, never skipped.** Decode failure is 52, replica
   refusal 53, an unexpected transfer 54. The view keeps its last revision;
   only `close` clears the fence.
5. **Acknowledge only what the link accepted.** Frames are whole and bounded;
   a full direction refuses without changing the queue. The client never
   acknowledges. `hand_off` returns the last ordinal actually queued and stops
   at the first `LinkFull`.
6. **The revision bumps at most once per poll.** Reuse the current
   `Rc<SessionView>` when rows, active, round, ended and options are unchanged
   and the step collected no event or receipt. The new view carries only that
   step's events and receipts, oldest first.
7. **`Died` is a notice.** It never sets the `dead` column; only a health fact
   does. `encounter_ended` latches `ended`.
8. **No float, `HashMap`/`HashSet`, `Instant` or `SystemTime` in the core.**
   Time is an injected `u64` millisecond value. Maps are `BTreeMap`.
9. **Error codes are the closed ADR-0033 §11 table.** Fatal is exactly 50–55.
   Diagnostics go in `Fault::detail`; `last_error` prefixes the code name and
   is never control flow.
10. **Options are the host's.** `legal_intents` is the only source; the client
    formats labels and sends the chosen body. It decides no rule.
11. **Every poll watermarks captures.** `acknowledge_captures(last_capture())`
    runs before the options are read, or admission eventually stops.
12. **The NPC is a peer.** It reads `legal_intents`, encodes the same
    `IntentFrame` a player would, and fires at most once per poll after 600 ms.
    A retained `QueueFull` is a hold, not a rejection (`npc_results`); every
    other NPC rejection is fatal 51.
13. **One guard per entry point.** `open`/`poll`/`submit_option`/`close` wrap
    their `Live` work in `catch_unwind(AssertUnwindSafe(..))`. A panic fences
    as 55 with `PANICKED: …`; a teardown panic during a fatal fence is ignored
    and the fault's code and text stand. `Drop` runs `close`, so no panic
    escapes.

## Definition of done for any change

```
cargo fmt --all
cargo clippy -p crpg-godot --all-targets --locked -j 2 -- -D warnings
cargo test -p crpg-godot --locked -j 2
python tools/lint/deps.py
python tools/lint/determinism.py
python -m unittest discover -s tools/lint -p "test_*.py"
```

(T047b adds the `--features engine` build and the Godot load script; those
never run under `cargo test`.)

## Known traps

- **The test seams are compiled out of the library.** `Probe` and the
  `#[cfg(test)]` `Authority` accessors (`host`, `player`, `npc`, `bound`,
  `npc_submitted`) exist only for `src/core/tests.rs` and its children; do not
  add a crate-level accessor or widen them to `pub`.
- **`client.rs` and `link.rs` never name `crpg_server`, `crpg_sim`,
  `EntityId` or `authority` in code.** Keep the dependency direction and the
  K31 source rule intact.
- **`submit` is a client prefix, not the full B14 order.** `BridgeCore` must
  check negative arguments, `Fenced` and `Closed` first; otherwise `close`
  plus `submit_option(-1, 0)` could return 1 instead of 3.
- **`LocalLink` capacity is per direction.** Both ends share two `Rc` queues;
  do not clone a `LinkEnd` to get two senders, and add no loss or timing policy
  here.
- **`entries()` is the only engine-facing key list.** Adding a field means
  updating the shell getters in T047b and the K9/K10 expectations together.
- **The campaign walk is a copy of the CLI's.** `crpg-cli` is bin-only;
  reusing it would pull testkit and `same-file` into the DLL. Keep the
  refusal shapes and sorted-first failure aligned with K34.
- **The fixture's opening actor is the goblin.** Initiative keys are ascending
  and the goblin's 9 sorts before the hero's 10. The default player side is
  still the first combatant's (Hero), so the default view has no player
  options on the first poll; tests that need options open with `"Goblin"`.

Agent log:

- 2026-11-02 (UTC) · opencode-go/deepseek-v4.1-flash + T047-1 · Wrote the working contract for the first core modules: Godot-free/one-authority invariants, gate list, and the temporary dead-code allowance plus B14-prefix trap, so T047-2 extends rather than reopens them.
- 2026-11-02 (UTC) · opencode-go/deepseek-v4.1-flash + T047-2 · Completed the working contract: removed the temporary dead-code allowance, added the capture watermark, NPC-peer and panic-guard invariants, the campaign-walk copy and fixture-opening traps, so the finished core is not extended as if the authority half were still missing.

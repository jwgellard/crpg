# crpg-godot — architecture

The Godot presentation bridge (spec §9, ADR-0033). It is the only crate allowed
to depend on `godot` or to carry `unsafe`. The design splits in two: a
Godot-free core that builds and tests headless on Windows and Linux, and a thin
gdext shell that converts values and calls the core.

**State after T047-2 (2026-11-02):** the Godot-free core is complete.
`core/errors.rs` (the closed code table and host-fault mapping), `core/link.rs`
(the bounded in-process two-ended link), `core/client.rs` (replica
application, immutable view revisions, rows, turn, options, receipts and
intent submission), `core/authority.rs` (the campaign walk, the embedded
`Host`, the NPC driver, the delivery hand-off and the presentation tables) and
`core/mod.rs` (`BridgeCore`, `open`/`poll`/`submit_option`/`close`, the panic
guard and the fatal fence) exist. The gdext shell is T047b.

## Position

`crpg-godot` sits above `crpg-server`, `crpg-net`, `crpg-sim` and `crpg-core`.
It owns presentation only: no rule, wire format, host policy, campaign loader
or engine type may move into it. The authority runs in-process behind
`LocalLink`; the client cannot mutate it.

## Modules

- `core/mod.rs` — `#![forbid(unsafe_code)]`; private module declarations and
  public re-exports (`BridgeCode`, `SessionView`, `ViewEvent`, `ViewReceipt`,
  `ViewValue`, `BridgeCore`). Never write a bare `core::` path: the module
  shadows the `core` crate. `BridgeCore` holds one `State` (`Closed`,
  `Open(Box<Live>)`, `Fenced(code)`), the current `Rc<SessionView>` and the
  `last_error` text. Every `open`/`poll`/`submit_option`/`close` runs its
  `Live` work inside `catch_unwind(AssertUnwindSafe(..))`; `open_on` takes a
  caller link so tests can inspect both queues.
- `core/errors.rs` — `BridgeCode` (20 stable numbers from ADR-0033 §11,
  `name()` upper-case, `is_fatal()` exactly 50–55), `Fault` (code + detail)
  and `host_fault` (`Closed` → 50, every other `HostError` → 51).
- `core/link.rs` — `LocalLink` owns two independent FIFOs of whole frames, each
  capped at `cap` (default `LINK_FRAMES`, the host's per-peer egress depth,
  128). `authority_end()` sends down and receives up; `client_end()` sends up
  and receives down. A full queue returns `LinkFull` and changes nothing;
  `recv` is FIFO or `None`.
- `core/client.rs` — `Client` owns one `ReplicaWorld`, the current
  `Rc<SessionView>`, the offered options, the pending sequence and the next
  sequence. `step` drains the down queue, decodes (`52` on failure), applies
  (`53` on refusal), collects events/receipts and rebuilds the view at most
  once per call. `submit` validates the client-owned prefix of B14 and sends
  at most one encoded intent.
- `core/authority.rs` — `WalkFs`/`RealFs`/`collect_campaign` (a private copy
  of the CLI walker: no link following, sorted encoded-byte DFS, non-UTF-8
  names refused, classifier errors `21`, filesystem errors `20`);
  `check_binding` (`25`); `Presentation` (D8's name/side/ability resolution);
  `Authority` (the `Host`, both peer handles, the NPC side map, the pacer, the
  in-flight sequence and the recorded `(EntityId, placement)` order);
  `hand_off`, `npc_results`, `choose_npc` and `NpcPacer`; and the open helpers
  `SpecParts`, `grants`, `ruleset_abilities`, `first_encounter` and
  `presentation_tables`.

## Flow in this half

1. `BridgeCore::open` checks the arguments (`3`) and the state (`2`), then
   walks the campaign (`20`/`21`), parses the compile-time engine version,
   loads and validates (`21`), selects the encounter (`22`/`23`), assembles
   the `EncounterSpec`, starts a `HistoryWorld`, checks the placement binding
   (`25`), picks or validates the player side (`24`), acknowledges the start
   journal, wraps the host, binds both peers with full disclosure (player as a
   snapshot peer, NPC as a plain peer), builds both presentation tables and
   creates the client. On success the view is empty revision 0 and
   `last_error` is cleared; nothing is retained on failure.
2. `BridgeCore::poll` runs the authority step, then the client step, then
   publishes the client's view. `0` is a clean poll, `30` means an upstream
   frame was refused or a failure-budget reply was dropped, and `50`–`55`
   fences.
3. The authority step drains the player's frames into `ingest`, drives the NPC
   (`legal_intents` → pacer → `choose_npc` → one encoded intent), pumps,
   folds the pump's NPC results, hands the player's delivery to the link,
   acknowledges the NPC's delivery unread, watermarks captures and finally
   reads the player's next options.
4. The client step decodes and applies every queued frame; a bad frame is
   fatal and is never skipped.
5. The view is rebuilt from `replica.entities()` and `turn()`: rows ascending
   by `NetId`, health-known flags, live active filter, host-ordered labels.
   The revision becomes `previous + 1` when this step collected any event or
   receipt, or when rows/active/round/ended/options differ. Otherwise the
   previous `Rc`, with its own events and receipts, is kept unchanged.
6. `submit_option` checks negative arguments (`3`), the fence (`c`), `Closed`
   (`1`), stale revision (`4`), index (`6`) and pending (`5`), encodes, then
   sends. A full link is `30`; only success advances `seq` and sets pending.

## View contract

Every slice has `net_ids().len()` entries. Missing presentation rows produce
`""`, never a panic. Unknown health is `(0, 0, known 0, dead 0)`; a `Died`
notice never sets `dead`. `encounter_ended` latches `ended`. `entries()` keys
start with `"kind"` for events and are `seq, applied, code` for receipts.

## Errors, fence and panic policy

Every non-zero result sets `last_error` to `NAME` or `NAME: detail`; only a
successful `open` clears it. A fatal fault (`50`–`55`) records its code and
text, replaces the state with `Fenced(code)` and shuts the host down and drops
the `Live` inside one ignored `catch_unwind`, so a panic during teardown
cannot change the fault. The view keeps its last revision. `close` is
idempotent, always ends `Closed` with the empty revision-0 view, keeps the
last error text, and turns a teardown panic into `PANICKED: …` without
fencing. A panic caught in `open`/`poll`/`submit_option` fences as `55` with
`PANICKED: <payload>`; `Drop for BridgeCore` runs `close`, so no panic
escapes. `#[cfg(test)] Probe` injects a panic at each boundary and at
`Live::drop`; it is compiled out of the library.

## Decisions and traps

Governed by ADR-0033 and ADR-0012. The core holds no float,
`HashMap`/`HashSet`, clock or thread. Only `shell.rs` will carry `unsafe`
(T047b); this crate root allows it for macro expansion, `core` forbids it.
Errors never carry control flow: tests assert codes and stable integers, not
diagnostic prose.

- **Acknowledge only what the link accepted.** `hand_off` stops at the first
  `LinkFull` and returns the last ordinal actually queued; acknowledging a
  refused frame makes the host drop it silently.
- **The NPC is a peer, not a second rule path.** It reads `legal_intents` and
  encodes the same `IntentFrame` a player would; a retained `QueueFull` is a
  hold, not a rejection.
- **Every poll watermarks captures.** Without it admission eventually stops
  with `backpressured: 1`.
- **The walker is a copy, not a reuse.** `crpg-cli` is bin-only; importing it
  would drag testkit and `same-file` into the DLL. Keep the two walkers'
  refusal shapes aligned by the K34 cases.

Agent log:

- 2026-11-02 (UTC) · opencode-go/deepseek-v4.1-flash + T047-1 · Opened the crate document for the first real code: the Godot-free errors, link and client modules, their flow and view contract, with the authority and shell parts marked as later T047 packages.
- 2026-11-02 (UTC) · opencode-go/deepseek-v4.1-flash + T047-2 · Completed the core document with the authority half, `BridgeCore`, the campaign walk, the NPC driver, the exact open/poll flow, the fence and panic policy, and the traps found while implementing them; the shell remains T047b.

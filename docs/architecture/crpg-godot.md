# crpg-godot — architecture

The Godot presentation bridge (spec §9, ADR-0033). It is the only crate allowed
to depend on `godot` or to carry `unsafe`. The design splits in two: a
Godot-free core that builds and tests headless on Windows and Linux, and a thin
gdext shell that converts values and calls the core.

**State after T047-1 (2026-11-02):** `core/errors.rs` (the closed code table
and host-fault mapping), `core/link.rs` (the bounded in-process two-ended
link) and `core/client.rs` (replica application, immutable view revisions,
rows, turn, options, receipts and intent submission) exist. `core/mod.rs`
declares them, forbids `unsafe` and re-exports the public items. `BridgeCore`,
`Authority`, the campaign walk, integration tests and the gdext shell are not
here yet (T047-2/T047b).

## Position

`crpg-godot` sits above `crpg-server`, `crpg-net` and `crpg-core`. It owns
presentation only: no rule, wire format, host policy, campaign loader or engine
type may move into it. The authority runs in-process behind `LocalLink`; the
client cannot mutate it.

## Modules

- `core/mod.rs` — `#![forbid(unsafe_code)]`; private module declarations and
  public re-exports (`BridgeCode`, `SessionView`, `ViewEvent`, `ViewReceipt`,
  `ViewValue`). Never write a bare `core::` path: the module shadows the
  `core` crate.
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

## Flow in this half

1. The authority calls `Client::step(link, options, refused)` once per poll.
2. Every queued frame is decoded and applied in order; a bad frame is fatal
   and is never skipped.
3. The view is rebuilt from `replica.entities()` and `turn()`: rows ascending
   by `NetId`, health-known flags, live active filter, host-ordered labels.
4. The revision becomes `previous + 1` when this step collected any event or
   receipt, or when rows/active/round/ended/options differ. Otherwise the
   previous `Rc`, with its own events and receipts, is kept unchanged.
5. `Client::submit` checks negative arguments (`3`), stale revision (`4`),
   index (`6`) and pending (`5`), encodes, then sends. A full link is `30`;
   only success advances `seq` and sets pending.

## View contract

Every slice has `net_ids().len()` entries. Missing presentation rows produce
`""`, never a panic. Unknown health is `(0, 0, known 0, dead 0)`; a `Died`
notice never sets `dead`. `encounter_ended` latches `ended`. `entries()` keys
start with `"kind"` for events and are `seq, applied, code` for receipts.

## Decisions and traps

Governed by ADR-0033 and ADR-0012. The core holds no float,
`HashMap`/`HashSet`, clock or thread. Only `shell.rs` will carry `unsafe`
(T047b); this crate root allows it for macro expansion, `core` forbids it.
Errors never carry control flow: tests assert codes and stable integers, not
diagnostic prose.

Agent log:

- 2026-11-02 (UTC) · opencode-go/deepseek-v4.1-flash + T047-1 · Opened the crate document for the first real code: the Godot-free errors, link and client modules, their flow and view contract, with the authority and shell parts marked as later T047 packages.

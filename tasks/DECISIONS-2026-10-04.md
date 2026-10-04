# Decision D27 — 2026-10-04 (UTC)

Status: **Decided by the user, 2026-10-04.**

## Authority

`docs/IMPLEMENTATION_PLAN.md` §2 asked the user six questions (D27 a–f). The
agent explained (c), (e) and (f) on request. The user's answers are quoted
below. Where the user deferred to "if it is needed" or "whatever the current
one is", the record names the condition and how it is resolved. These extend
D01–D26 and do not waive any task's exact contract, tests, native gates or
completion record.

## D27a — merges stay human

User: "No merges, but PR is okay."

Agents may open pull requests. They never merge them. Every merge is H-merge.
Plan §1.6's merge-delegation proposal is **declined**.

## D27b — save compression: `zstd`

User: "Compression".

`crpg-persist` (T038) may add `zstd` (current stable at contract time, exact
pin, `crpg-persist` only). The S001 caps are mandatory:
- an input-size ceiling;
- a decompressed-size ceiling, enforced while decompressing, not after.

The T038 contract carries the T018a-style dependency audit and a clean
`cargo deny check`.

## D27c — navigation: custom, deterministic, in `crpg-nav`

User: "Has to be custom, unless there is another mature Rust library. I am
happy for this to be its own dedicated repo if nothing like this exists for
rust."

Survey (crates.io, 2026-10-04):
- **Rejected — floating-point geometry.** `landmass` 0.9.2, `polyanya`
  0.17.1 and `rerecast` 0.4.0 (a pure-Rust Recast port, still pre-1.0) are
  all built on `glam` `f32` geometry. Their results are not integer-exact, so
  they cannot feed authoritative movement, replay goldens or per-target hash
  comparison.
- **Rejected — bindings and Bevy.** `recastnavigation-sys` is raw C++ FFI.
  `oxidized_navigation` is a Bevy plugin.
- **Allowed for search only.** The one mature pure-Rust library is
  `pathfinding` 4.16.0 (generic A*/Dijkstra over integer costs, MIT or
  Apache-2.0, about 3M downloads). It covers search only, not navmesh
  representation or baking.

Decision:
- The navigation representation and bake are custom, with integer or
  `Fx16_16` coordinates.
- They live in the existing `crpg-nav` workspace crate. Staying in the
  workspace keeps the determinism lint, the dependency lint and dual-native
  CI on it, which a separate repository would lose.
- The user's offer of a dedicated repository is recorded as an **option to
  extract later** once the API is stable, not as the starting point.
- The T035 contract may propose `pathfinding` (exact pin) for search. That
  edge needs the user's approval at contract review. Without it, search is
  written in-house.

## D27d — Godot: current versions

User: "Whatever the current one is".

- **Rust binding:** the `godot` crate, current stable when T047's contract is
  written (0.5.5 on 2026-10-04), exact pin, `crpg-godot` only.
- **Engine:** the Godot 4 build installed on the maintainer's Windows
  machine (`Godot4 --version`), recorded in T047's completion record and
  pinned per ADR-0001.

## D27e — Lua: approved when needed

User: "If it is needed, approve it."

- **Approved:** `mlua` (current stable at contract time; 0.12.2 on
  2026-10-04), exact pin, with vendored Lua 5.4 (ADR-0005), for
  `crpg-script` only.
- **Condition:** used only from M8's sandbox task (T055) on. The MVP does not
  need it (D27f).
- The sandbox deny-list and budget enforcement remain H-sandbox (read line by
  line). The dependency audit and a clean `cargo deny check` are still
  required.

## D27f — MVP scope confirmed

User: "confirm both."

1. The MVP quest completes through a built-in event-IR rule ("on this
   creature's death, set the quest to completed"), not Lua.
2. MVP enemy AI is only "move to the nearest hostile and attack", in
   `crpg-ai`, consuming `legal_actions` (T028). There is no scoring,
   behaviour trees or party AI before M9.

## Agent log

- 2026-10-04 (UTC) · claude-code + D27 · Recorded the user's six answers with their own words, the navigation library survey behind D27c, and the conditions on the deferred-to-agent answers, so Wave 1+ tasks cite a dated authority.

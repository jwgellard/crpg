# Decision D45 — session 39: a T139b2 gap, and two revised contracts to approve

Status: **Answered by the user, 2026-11-07: D45a (a), D45b (a), D45c (a).** Drafted by the manager in session 39 (2026-11-07). D45a holds back one T139b2 code fix; D45b and D45c each hold back an implementation package. Nothing else in the wave waits on them.

## D45a — T139b2: a creature waiting to join when the fight ends

In a turn-based fight, a creature that joins in round 0 under the ruleset's `late_join: after_active` setting is put on a waiting list (`pending_joins`) until the current turn ends. The Claude review found a case the contract does not cover. If an attack leaves fewer than two creatures alive, the fight ends on the spot (`active` becomes empty) without going through the end-of-turn step that empties the waiting list. The finished fight is then saved with a creature still on the list. Load check Q1 says a waiting list may only exist while a turn is active, so **that save refuses to load**. The contract also says such saves must round-trip (B33), so it contradicts itself here. The reviewer reproduced it.

- **(a) Recommended: empty the waiting list whenever the fight ends.** Once the fight is over, a waiting creature will never get a turn, so the list means nothing. Q1 stays strict, so the loader still accepts only states the engine can actually reach. The waiting creature is still a combatant and is released like the others. No golden or replay changes: the list is saved only when it is non-empty, and no golden contains one.
- (b) Relax Q1 so a finished fight may keep a waiting list. The loader then also has to skip Q5 (which compares each waiting creature with the active turn), because a finished fight has no active turn. That is two load rules loosened to keep a list nobody reads.

## D45b — approve the revised T098h contract (`tasks/T098h.md`)

This is D44c (a) written out as a contract. The three QUIC close sites (client stop, single-connection server close, server shutdown) must build their flush request through one shared constructor. The request's fields become private, so passing the wrong deadline is a compile error rather than something a test has to catch. One unit-test module pins the 200 ms reserve rule exactly for budgets of 2 s, 800 ms, 400 ms and 0. There are no wall-clock tests. It changes `src/` in one crate only, with no public API, wire, config or `CloseReport` change. The draft lists no open questions. The manager checked its code citations.

- **(a) Recommended: approve as written.** It then goes to an OpenCode implementer with a Claude review that re-runs its four planted bugs (mutants).
- (b) Send it back with changes (name them).

## D45c — approve the revised T048b contract (`tasks/T048b.md`)

This is D44b (a) written out: only the mesh inside each proxy is lifted, and the proxy and its label offset stay where T048 put them. None of the approved scene tests S1–S14 changes. The draft carries two questions of its own:

1. **Side colours.** The four opaque colours are blue `(0.20, 0.60, 1.00)`, orange-red `(1.00, 0.35, 0.20)`, green `(0.25, 0.85, 0.45)` and purple `(0.75, 0.45, 0.95)`. A fifth side reuses the first colour, a sixth the second, and so on. Dead stays greyed as T048 approved.
2. **Proof and records.** A rendered-frame check (a Godot movie of 90 frames), plus updates to the three implementation records. `Main.tscn`, the HUD and Rust are left untouched.

- **(a) Recommended: approve both as drafted.**
- (b) Change the palette or the wrap rule (name the change); the proof and records stay as drafted.

## Agent log

- 2026-11-07 (UTC) · claude-code/opus-5.5 (manager) + session 39 · Drafted D45 from the T139b2 core review's terminal-with-pending-joiner finding and the two contract revisions made to D44, so the user can rule on all three at once.
- 2026-11-07 (UTC) · claude-code/opus-5.5 (manager) + session 39 + D45 answered · Recorded the user's answers: (a) empty the waiting list whenever the fight ends, (a) approve the T098h contract, (a) approve the T048b contract with its palette, wrap, frame check and records.

# Decision D44 — three small rulings from session 38 (T139b, T048b, T098h)

Status: **Answered by the user, 2026-11-07: D44a (a), D44b (a), D44c (a).** Drafted by the manager in session 38 (2026-11-07). None of the three blocks the wave: T139b1 is in its fix round, and the two contract drafts wait on the answers.

## D44a — T139b member checks: which fault wins when two members are both bad

T139b lets an encounter start with creatures that already exist in the world ("members"). Before starting, it checks each member (M1 "is the entity alive", M2 "is it listed twice", M3 "does it have a creature record", up to M8). `tasks/T139.md` B21 says the checks run member by member, in list order. Test E2 in the same contract says a creature with no record that is listed twice must fail as "listed twice" (M2), not "no record" (M3). Member by member, that can't happen: the first listing already fails M3 before the duplicate is seen. The contract contradicts itself.

- **(a) Recommended: run M1 and M2 across the whole list first, then M3–M8 member by member.** This is what the implementer built. It satisfies E2 and changes nothing when only one member is bad. The visible difference: with two bad members, an absent id later in the list is reported before an earlier member's dead record. The contract's B21 text is amended to say so.
- (b) Keep B21's literal per-member order and change E2's "listed twice beats no record" pair to a case that order can meet (a creature with a record listed twice). The implementer reworks the check loop.

Either way the world is left untouched on any failure; only which error is reported changes.

## D44b — T048b: how the Godot proxies sit on the ground, and their colours

The proxies in the Godot client are centred on the ground plane, so half of each is underground, and all are the ground's grey. The T048b draft (`tasks/T048b.md`) lifts each one by half its height and gives each side a colour (blue, orange-red, green, purple, wrapping round after four); dead stays greyed in place as approved in T048.

- **(a) Recommended: lift only the mesh inside each proxy.** The proxy's own position is unchanged, so none of the six approved scene tests (S1, S2, S3, S5, S8, S9) changes. The name label then sits at the top of the mesh rather than floating above it.
- (b) Lift the whole proxy (the draft's recommendation). `proxy.position.y` then reads directly as "grounded", but the six existing tests change their expected y values (y-only edits that you approve), and labels float about 0.6 above the mesh.

Both choices also take the four-colour palette and the four new tests S15–S18 as drafted.

## D44c — T098h: how to prove the close reserve is wired in all three places

T098g reserved the last 200 ms of a QUIC close budget for the "goodbye" packet, in three places: client close, single-connection server close, and server shutdown. The review showed that if any one place were changed back to use the whole budget, no test would notice. The goodbye still gets out in the tests, because of a separate fix in the same change. T098h is meant to close that gap. The draft (`tasks/T098h.md`) adds three timing tests: a close against a stalled peer must return before the full 2 s budget. Correct code returns at about 1.8 s and the broken version at 2.0 s.

- **(a) Recommended: a small code change instead of timing tests.** All three places build their flush request through one shared constructor that takes the computed deadlines, so no call site can pick the wrong deadline, and one unit test pins the constructor. No timing, so it can't flake. It touches `src/` in one crate (no public API, wire or config change), so it is a code package rather than tests only.
- (b) The timing tests as drafted. They need no product change, but they have at most 200 ms of margin, the size of the reserve. A loaded CI machine can stall a test thread longer than that (session 37 saw a QUIC test take 34 s instead of 6 s), so a correct build could fail.
- (c) Drop T098h and keep T098g's accepted position: the wiring is covered by its test 1 plus review.

## Agent log

- 2026-11-07 (UTC) · claude-code/opus-5.5 (manager) + session 38 · Drafted D44 from the T139b1 implementer's contract reading, the T048b draft's proxy-lift choice and the T098h draft's 200 ms timing margin, so the user can rule on all three at once.
- 2026-11-07 (UTC) · claude-code/opus-5.5 (manager) + session 38 + D44 answered · Recorded the user's answers: (a) M1–M2 pass first, (a) lift the mesh only, (a) shared flush-request constructor instead of timing tests.

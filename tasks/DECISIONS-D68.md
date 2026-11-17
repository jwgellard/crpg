# Decision D68 — session 70: the reconnect contract (T025b) questions and approval

Status: **open, drafted 2026-11-17 (session 70).**

T025b lets a player whose connection drops get the same seat back within 30 seconds, instead of rejoining as a new player. The contract is on `task/T025b-c1` (`00fa070`, Opus architect; review fixes `4d13749`).

## D68a — where the "reconnect password" comes from (T025b Question 1)

When a player joins, the server gives their game a secret, called a "ticket". If the connection drops, the game shows the ticket to get the same seat back. The server has no source of random numbers of its own, so the ticket has to be made from something it already has.

- (a) **Reuse the player's invitation secret as the ticket.** No new library. But then the ticket is the invitation itself. Anyone who copies it can join as that player at any time, not just reclaim one dropped seat.
- (b) **Mix the invitation secret with the session number using a standard keyed hash (BLAKE3).** The engine already uses BLAKE3 for save-state hashes, so nothing new is downloaded. `crpg-server` gains a dependency line on the copy the workspace already has, and that edge is what needs your approval. A copied ticket reclaims only that one session, and only within its 30 seconds.
- (c) **Use the operating system's random numbers.** Needs a new library, and tickets then differ from run to run. It gains nothing over (b) here.

**Recommend (b).** The decision on record (D12) asks for a ticket bound to one session, and (b) gives one with a library already in the build. Choosing (b) also approves the `blake3` line in `crates/crpg-server/Cargo.toml` and its one-line `Cargo.lock` change (part T025b-q only).

## D68b — how the server says "no" to a reconnect (T025b Question 2)

The reconnect design (ADR-0032) says the server sends a short "rejected" message, then closes the connection once that message has arrived. Waiting for it to arrive can stall the server's game loop for up to 2 seconds per refusal (the network layer's flush timeout). Anyone can send fake reconnects, so this is an easy way to freeze the server.

- (a) **Send the message, then wait**, as the ADR says. The loop can stall.
- (b) **Hang up at once with a reason code**: 5 means "start a new session", 7 means "busy, try again". The client reads the code instead of a message. This is instant. ADR-0032 gets an addendum, and the client task (T046) maps the two codes.

**Recommend (b).** A server that one fake packet can freeze for 2 seconds breaks the engine's "the loop never blocks" rule. The message (a) would carry is no more useful than the close code.

## D68c — approve the T025b contract

T025b has two parts, done one after the other, both in `crpg-server`. Neither part is S-set (protected).

- **T025b-i** (`host.rs`): the seat-holding logic (fence, grace period, resume, expiry). About 280 lines of code and 750 of tests, with a second, independent test writer (from another model family) on four cases.
- **T025b-q** (`quic.rs`): the network side. About 240 lines of code and 600 of tests.

Movement-protocol (v4) sessions can't be resumed yet. A backlog row covers that.

A free first-pass review found four defects, all fixed in `4d13749`:

- a copied test harness that would crash on the reconnect message;
- one unbound name in a test;
- a missing import list;
- a wrong line citation.

**Recommend approve**, with the answers to D68a and D68b written into the contract.

## Agent log
- 2026-11-17 (UTC) · claude-code/opus-5.5 (manager) + session 70 · Drafted the T025b decision batch from the contract's two questions and its review, with a recommendation for each.

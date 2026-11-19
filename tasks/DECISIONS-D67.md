# Decision D67 — session 68: the malicious-client suite (T042) questions and approval

Status: **answered by the user, 2026-11-17 (session 69): D67a (a), D67b (a) now plus a backlog row for (b), D67c approve — all as recommended.**

## D67a — who writes the log line for a rejected client message (T042 Question 1)

The spec says every malicious message must get "a clean rejection and a log line". Today the engine already rejects bad messages cleanly and gives each kind of rejection a fixed text, but no server code writes a log anywhere: no server crate has a logging library yet.

- (a) **T042 pins the rejection and its exact text now.** Actually writing the line becomes the server program's job (T040 or T041), filed as a backlog row with a small "where do server logs go" decision there.
- (b) **Add a logging library now.** That is a new dependency touching every server crate, well beyond a test task.

**Recommend (a).** The text exists, and where logs go (a file, the console, size limits) is a product decision for the server program, not for a test suite. With (b), T042 grows from a test-only task into a cross-crate change that needs your dependency approval.

## D67b — should the client's own decoders get the same hostile-input suite (T042 Question 2)

T042 covers what a cheating client can send to the server. The client also decodes what the server sends, and a hostile server (or a bug) could send it bad bytes.

- (a) **Not in T042.** The server is trusted and pinned by certificate, and the client decoders already have their own bad-input tests.
- (b) **Yes, as a later task** (T042b, the same shape, about 500 test lines), done before players host games for each other.

**Recommend (a) now, plus a backlog row for (b)** tied to player-hosted games (D65b), when a server can no longer be assumed honest.

## D67c — approve the T042 contract

T042 is one part, T042a: about 650 lines of tests in `crpg-net` (`tests/malicious_client.rs`, cases X1–X14 over the eight decoders a client can reach). It also corrects the `crpg-net` architecture doc, which states the decode order wrongly. The Opus architect wrote it on `task/T042-c1` (`4533d54`). A Go first-pass review found three defects: two wording contradictions about the order checks run in, and a fuzz case that could pass without ever testing an accepted message. All three are fixed in revision 1 (`691c0d2`); a fuzz pass now has at least 4,352 accepted inputs.

**Recommend approve**, with the answers to D67a and D67b written into the contract.

## Agent log
- 2026-11-17 (UTC) · claude-code/opus-5.5 (manager) + session 68 · Drafted the T042 decision batch from the contract's two questions and its review, with a recommendation for each.
- 2026-11-17 (UTC) · claude-code/opus-5.5 (manager) + session 69 · Recorded the user's answers: all three as recommended.

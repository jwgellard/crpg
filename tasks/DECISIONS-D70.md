# Decision D70 — the dedicated server program (T040) questions and approval

Status: **open, 2026-11-19. Waiting for the user.**

T040 turns `crpg-server` into a program you can actually run as a
dedicated server on Windows, and on Linux without Godot. It reads one
settings file, loads a saved world, lets invited players connect over the
internet, and runs the same game loop single-player already uses. It writes
a log line every time a player's message is refused, and on `quit` it saves
the world and exits. A CI job builds the server for both systems so every
change proves it still builds (gate 12). The contract is `tasks/T040.md` on
`task/T040-c1` (Opus architect, adjudicated from two blind drafts). No new
library is added in any answer marked recommended.

## D70a — how does an operator stop the server cleanly? (T040 Reading 13)

A clean stop matters because only a clean stop saves the world; anything
else loses the play since the last save. Rust's standard library cannot
catch Ctrl-C or a service manager's "stop" request. Catching them needs
either unsafe code (forbidden outside the Godot crate) or a small extra
library.

- (a) **Type `quit` in the server console (recommended).** No new library.
  `quit` stops the loop, closes every connection politely and saves. Ctrl-C
  or a kill still ends the server at once, without saving. A server started
  with no console (as a background service) keeps running; it can only be
  killed, so a service setup loses unsaved play on stop until (b) is done.
- (b) **Add the `ctrlc` library now.** Ctrl-C and a service "stop" would
  also save and close cleanly. It is a new dependency (it brings a couple of
  small platform crates for Windows and Linux) that needs your approval and
  a licence/advisory check.

**Recommend (a) now**, plus a backlog item to do (b) before public server
releases (T089), when servers will really run as services. Today nobody
runs a dedicated server as a service.

## D70b — what file format for the server's certificate and key? (T040 Reading 7)

Players' games trust the server by its certificate (a fingerprint they
are given with their invitation). The server reads the certificate and its
private key from two files. There are two common formats: DER (raw binary)
and PEM (the same bytes as text between `-----BEGIN ...-----` lines).

- (a) **DER only (recommended).** The networking code already takes DER, and
  the tests already use DER files, so no new code or library is needed. An
  operator with PEM files converts them once with a standard tool (for
  example `openssl x509 -outform der`).
- (b) **Also accept PEM.** Friendlier, since most tools produce PEM. But
  reading PEM needs a base64 decoder: either a new library (approval) or
  about 40 lines of hand-written decoding with its own tests.

**Recommend (a).** It is the smallest correct thing; PEM can be added later
without changing anything else.

## D70c — should the dedicated server run the monsters' AI? (T040 Reading 12)

In single-player and multiplayer alike, the server decides what the
monsters do (your earlier rule: single-player is the same server, and the
server does the AI). To do that it must know which creatures are on which
side and which ones the AI controls. That information lives in the
campaign's authored data, which the server is deliberately not allowed to
read.

- (a) **Yes, from a small "sides" file (recommended).** The settings file can
  name a JSON file listing each creature's side and which creatures the AI
  controls; the server calls the AI once per game-loop cycle. About 40 more
  lines. Someone (an operator, or a later `crpgc` command, or T041's tests)
  writes that file.
- (b) **No AI in T040.** Smaller, but on a dedicated server monsters never
  act, so a fight can never finish there. T041's end-to-end server test
  (gate 13) would then need its own way to drive the AI, or a later task
  must add (a) anyway.

**Recommend (a).** Without it the dedicated server cannot host a real fight,
and gate 13 needs one.

## D70d — approve the T040 contract

T040 is in the protected (S-set) category: it is the server product and it
changes the CI workflow. It has three parts, one after another:

- **T040a** (`crpg-server` library): a list of "refused client messages"
  read from what the network layer already reports, and a hook so the
  program can see that report every cycle. About 120 lines of code and 260
  of tests.
- **T040b** (`crpg-server` program): the command line (`--config`, `--help`,
  `--version`), the JSON settings file, invitations, the start-up order,
  the log lines, the console `quit`, the save on exit. About 420 lines of
  code and 520 of tests that run the real program.
- **T040c** (CI): one new job that builds the server on Windows and Linux,
  checks Linux pulls in nothing from Godot, runs it once, and keeps both
  builds as downloadable artifacts for 14 days. No existing job changes.

Nothing in the game simulation, the save format, the network protocol or
any existing test changes. No new library is added (with D70a–c answered as
recommended).

**Recommend approve**, with the answers to D70a–c written into the contract.

## Agent log
- 2026-11-19 (UTC) · claude-code/opus-5.5 (architect) + T040 contract · Drafted the T040 decision batch: how the server is stopped, the certificate format, AI on the dedicated server, and contract approval, each explained plainly with a recommendation.

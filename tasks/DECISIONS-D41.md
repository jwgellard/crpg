# Decision D41 — Godot demo gates

Status: **Answered by the user, 2026-11-02: D41a (a), D41b (a), D41c (a),
D41d (a), all as recommended.** On D41c the user first asked whether MPL-2.0
posed any problem; after the file-level obligations were explained, they chose
the eight-package exception.

## D41a — What a player is allowed to see

The server sends each player only what that player may know, so a hacked client
cannot read hidden information. `tasks/T027b.md` §"Decisions for the user" item 1
lists the rules in plain words, for example: you see a character only if you
share an area and the server marks it visible, and a character who leaves and
returns looks like a new one, so it cannot be tracked.

- **Recommend:** (a) approve as written. The rules match the T022 server
  behaviour already approved and the D14 choices.
- Alternative: (b) name a rule to change. The contract changes, and possibly T027c.

## D41b — How the demo knows names, and which character is yours

The network sends anonymous numbers, health and turn, with no names and no "this
one is yours". The demo still has to show them.

- **Recommend:** (a) the Godot bridge builds a number-to-name table from the
  campaign data when the game starts. No network change, and it works for one
  local player. ADR-0033 already assumes this. Real multiplayer needs (b) later,
  as its own task.
- Alternative: (b) add a network message for it now. This is right long term, but
  it needs a reviewed protocol version and its own filtering rule, which delays
  the demo.

## D41c — The licence on the Godot binding

Our licence rule (`deny.toml`) allows only permissive licences. The Rust library
that connects our code to Godot is under MPL-2.0: if we edit one of its files and
ship it, we must publish that edited file. Our own files are unaffected. D27d
approved the library's version, not its licence. Without an answer T047 may not
add it. Details: ADR-0033 U1.

- **Recommend:** (a) allow MPL-2.0 for the library's eight packages only. Every
  other MPL package still fails the check. Cost: a licence notice in the shipped
  game.
- Alternatives: (b) allow MPL-2.0 for everything, which removes the check for
  future packages; (c) refuse it and write the bridge in C++, which costs months
  before any Godot demo.

## D41d — Accept the Godot bridge design

`docs/adr/0033-godot-bridge.md` sets how the game engine connects to the server.
The whole Rust side lives in `crpg-godot`, Godot calls it from its main thread,
and the client only asks for actions by choosing from a list the server offers.
It adds one small server task first (T047a), and it drops the art-sourcing
(T105) and UX-research (T104, T108) prerequisites for this demo only: the demo
uses plain shapes and a basic combat screen.

- **Recommend:** (a) accept. T047 and T047c, CI changes included, still come to
  you for the S-set review, and the single `unsafe` line for H-unsafe review.
- Alternative: (b) amend the ADR first.

## Agent log

- 2026-11-01 (UTC) · claude-code/opus-5.5 (manager) + session 29 · Collected the four user gates on the Godot demo path in one batch, each with a recommendation.
- 2026-11-02 (UTC) · claude-code/opus-5.5 (manager) + D41 answered · Recorded the user's answers: all four as recommended, D41c after a clarifying question on MPL-2.0.

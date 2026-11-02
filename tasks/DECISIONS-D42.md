# Decision D42 — Godot slice: client CI contract and two contract corrections

Status: **open**, drafted by the manager in session 32 (2026-11-02). Nothing waits on these yet: T047b (shell) and T047d (GDScript client) land first, and T047c's implementation needs D42a.

## D42a — Approve the T047c contract (the Windows `client` CI job)

`tasks/T047c.md` says how CI will build the Godot demo on Windows, check it loads, and offer it as a download. It also says how CI will prove the normal build contains no Godot code. It is S-set (it changes `.github/workflows/ci.yml` and `deny.toml`), so it needs your approval before a worker builds it. The architect checked the Godot 4.7.2 download itself. It hashed a fresh copy, and the result matched both GitHub's SHA-256 and Godot's published SHA-512, so CI checks both.

Its four questions, which the contract's last section explains in full:
1. May the Linux CI machine *look up* (not build) the Godot packages, so the "no Godot in the normal build" check has a self-check there too? **Recommend (a): yes.** ADR C4's "no Linux job enables `engine`" then reads as "never builds", and the `deny` job already does the same lookup.
2. May the download include the test campaign (14 small files) and our two licence files? Without the campaign, the download cannot run. **Recommend (a): yes.**
3. Pin the new upload step as `actions/upload-artifact@v7` (a major tag, like `checkout`) rather than an exact version? **Recommend (a): `@v7`.**
4. Should the notice name only the eight MPL-2.0 Godot packages for now, with a backlog item for a full generated notice before any build leaves the project? **Recommend (a): eight only now, plus the backlog item.**

- **Recommend:** approve the contract with 1–4 as recommended.
- Alternative: name a change. The architect revises the contract before anyone implements it.

## D42b — Two corrections to the approved T047 contract (record only)

Building the core showed two places where the approved T047 text does not match the code it describes. The reviewer accepted both readings. Nothing in the code changes; this only corrects the text.
1. **Who acts first.** K15 says the Hero (NetId 1) is active at the start. In the test campaign the Goblin's initiative puts it first (NetId 2), as the CLI's own tests show. The default player is the Hero, so on the first frame the player has no choices. Tests that need a choice open as "Goblin".
2. **One private field.** `Live` holds the client's end of the local link, which the Interface sketch left out.

- **Recommend:** (a) the manager adds a dated correction note under T047.md's Adjudication section, so the text matches what landed.
- Alternative: (b) leave the text as approved and record the corrections only in the completion record.

## Agent log

- 2026-11-02 (UTC) · claude-code/opus-5.5 (manager) + session 32 · Drafted the decisions for the T047c contract and the two T047 text corrections found in the T047-2 review, so the user can answer them in one pass.

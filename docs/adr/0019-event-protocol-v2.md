# ADR-0019: Explicit v2 history projection beside frozen v1

Date: 2026-09-29 (UTC)
Status: Selected under recorded D09 delegation for specification; source and
native conformance remain T021 work. No independent human review is claimed.

## Context

T020 supplies authoritative ordered history. v1 has a closed eight-op delta
vocabulary and must continue rejecting unknown versions/tags. Net currently
depends on sim only for tests; making a codec import sim at runtime is neither
necessary nor approved.

## Decision

Adopt [T021's specification revision](../../tasks/T021.md#specification-revision--2026-09-29)
as the exact API/wire/error/disclosure contract. Add explicit protocol_v2,
codec_v2 and projection_v2 modules. v2 version is 2, tags 8/9/10 are
ActionResolved/TurnStarted/EncounterEnded; v1 remains frozen and callable.
The Rust Legacy wrapper flattens to existing tags 0..7 with unchanged payloads.

Production projection consumes host-supplied per-field optional values, never
raw SimEvent serialization or an implicit new sim dependency. Suppress an event
unless every field is permitted, preserve order, and let the host independently
publish permitted state and assign gapless per-client frame sequences. Native
tests consume real T020 history through the existing dev-only edge.

## Consequences

No negotiation/downgrade policy is hidden in decoding. New strings are bounded
before copying; caps and receipts remain unchanged. Candidate grants are a
projection input, not a replacement for T027's production interest policy or
T022 authentication. Wire fixtures are independent byte oracles, not replay
rebaselines. No dependency approval is needed for the specified API.

## Agent log

- 2026-09-29 (UTC) · opencode/gpt-6-astra + T021/D09 specification · Filed the exact version/tag and disclosure decision while preserving the existing dependency boundary. Linked the normative task contract rather than creating a second copy of the wire layout.

## Status update — 2026-09-30 (UTC)

**Accepted** — approved by the user on 2026-09-30 ("Approved.", decision D19). Implemented by T021 and merged in PR #18 with `events_v2` green on Windows/MSVC and Linux/GNU CI. See [tasks/DECISIONS-2026-09-30.md](../../tasks/DECISIONS-2026-09-30.md).

- 2026-09-30 (UTC) · claude-code + D19 status update · Appended the user-approved status change as a dated section instead of editing the original status line, per the E007 append-only policy.

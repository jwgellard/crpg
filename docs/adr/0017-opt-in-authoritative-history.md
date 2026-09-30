# ADR-0017: Opt-in authoritative history and full-wrapper hashing

Date: 2026-09-29 (UTC)
Status: Selected under recorded D08 delegation for specification. Golden artifacts
require independent native generation/review; implementation gates remain open.

## Context

Unconditional richer SimEvent emissions would alter legacy queue bytes and
combat replay hashes. Host capture needs bounded durable history, but legacy
World exposes mutable queues/stores/RNG and cannot enforce capture on all callers.
ADRs 0008/0011 keep payload fields core-closed; ADR-0012 requires native goldens.

## Decision

Adopt [T020's specification revision](../../tasks/T020.md#specification-revision--2026-09-29)
as the exact API, serialization, errors, emission and acceptance contract.
Add HistoryWorld owning a private World and a bounded, single-consumer journal.
It offers typed transactional methods and immutable World queries only. Reuse
existing controllers on staged state; consume legacy events once through drain.
Private transition facts identify logical turn starts even when the actor or
saturating round number repeats. Keep the persisted combat round as u32 with
its existing saturation semantics; convert to u64 only for history payloads.
Keep SimEvent and all legacy APIs/bytes intact.

HistoryEvent is a separate six-variant sim-owned, core-closed vocabulary. No
net identities or rules values enter it. T022's server capture/checkpoint layer
is the named real consumer; acknowledgement follows host capture, not slow-peer
delivery. Full wrapper hash includes every serialized authoritative field,
including pending history, acknowledgements and sequence state. Consumer ack
schedule is therefore part of the deterministic input.

## Compatibility and new goldens

No legacy hash exclusion, format change or rebaseline. New native sim-local
history_v1 fixtures have explicit permission to be authored under T020, each
on its own ADR-0012 target with the pinned toolchain/default features and normal
test profile. Authoring is distinct from reviewed acceptance; comparisons never
regenerate or skip a missing file. Testkit/CLI integration is separate work.

## Consequences

Full cloning is an acceptable first implementation of rollback, not a performance
claim. Importing arbitrary legacy mid-encounter state and general mutable World
callbacks are intentionally unavailable. This mode's queue is bounded; the World
itself is not made a bounded network payload. Persistence parsing limits are a
mandatory host concern: T022 must cap checkpoint input bytes before JSON parsing,
including reader-based input and separate compressed/decompressed limits when
applicable. Sim's visitors bound retained journal storage and collection growth,
not parser scratch allocation; escaped strings may allocate parser scratch
before a string visitor runs. Generic Deserialize alone therefore carries no
parser-wide memory-bound guarantee. Adjacent-tag history payloads require `type`
before `value` on input and reject the reverse order before decoding the value,
avoiding whole-payload buffering. Explicit release does not manufacture natural
EncounterEnded.

## Agent log

- 2026-09-29 (UTC) · opencode/gpt-6-astra + T020/D08 specification · Filed the selected opt-in history and full-hash decision with explicit native golden scope. Preserved historical ADRs and distinguished delegated design authority from unperformed artifact review and implementation verification.
- 2026-09-29 (UTC) · opencode/gpt-6-astra + ADR-0017 review corrections · Clarified retained-history versus parser-memory bounds and the mandatory host pre-parse limit, adopting streaming type-before-value payload decoding. Made the event-only round conversion explicit so the legacy persisted counter and saturation semantics remain unchanged.

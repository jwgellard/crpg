# C0 — post-T018 human decision bundle

**D01–D03 selected under user delegation; no source authorization.**
Current choices: [decision record](POST-T018-DECISIONS.md#d01--contracts-and-e017-disposition-selected).
The options below are retained as historical analysis. Read E003/E012/E017/E018/E022 and ADR-0012.
Appendix B direction was approved; those E-tasks were not closed by T018.

## E003 — contracts placement and Transport

Recommendation: ratify E003 B (definitions only) with consumer-owned local
interfaces where the graph requires them. T018b's Transport remains in net;
B6 explicitly says this is not debt requiring a later move. A higher consumer
may later propose a narrow adapter. Reconcile the spec promise of cross-crate
implementations through an approved superseding decision, not source migration.

Genuine options: A, graph relaxation for contracts dev dependencies (does not
solve production implementation placement); C, consumer implementations of
contracts traits (net→contracts is currently forbidden). Both require explicit
graph/self-test/governance work in separately scoped tasks. No such change is
authorized. **Call:** approve reconciliation B or reopen one of those options.

## E012 + E022 — one authoritative host and product/API placement

Recommendation: reusable library target in existing `crpg-server`, thin
dedicated executable in that package, Windows client/editor Godot projects
over `crpg-godot`. Embedded and dedicated hosts use the same authority through
transport. Keep OS process/filesystem/service adapters above sim. Put shared
filtered replica mechanisms in net and presentation queries in the bridge,
subject to exact API review; no client World mutation access.

Alternative: another existing crate owns the reusable host, with explicit
proof that its graph admits every dependency and no authority cycle appears.
A new crate is not implicitly approved by either option.

**Call:** choose package/targets and assign exact lifecycle, ownership/threading,
shutdown, error, auth binding and replica API contracts. Define checkpoint
ownership and transport adapter interface before T022. E022's remaining ledger
(EditCommand, FFI objects, bulk scene sync, signing/discovery, crpgc apply)
must remain separately owned; accepting host placement cannot close it all.
Acceptance requires Windows embedded + dedicated and Linux dedicated product
smokes when those adapters exist, not an in-memory double relabeled as product.

## E018 — privileged capabilities and trust enforcement

Recommendation: ordinary-player, scoped GM and administrative capabilities
bound by the server to authenticated sessions; distinct privileged protocol
surface. GM operations authorize operation + campaign/session/entity scope,
not merely possession of an intent flag. Revoke before the next operation;
embedded sessions have no bypass. Keep ordinary combat v1 unchanged.

Alternative: authenticated multiplexed connection with a separate typed
privileged message namespace and identical capability checks; less connection
machinery, but greater risk of accidental routing across privilege boundaries.

**Call:** choose credential provisioning/proof/rotation, grant/revoke authority,
scope rules, channel separation, reconnect reauthentication, signing trust
roots/offline failure behavior and client package allowlist/strip policy.
Map every T0/T1/T2 promise to an actual verifier, sandbox or filter with an
owner. T0 signing must coordinate E023; Lua stripping must coordinate E010.
No accounts service, LAN-only trust assumption or native signing implementation
is invented here. Future tests must include player-forged GM messages, expired
and revoked credentials and valid authorized controls through public transport.

## E017 — close or supersede

Recommendation: supersede the original T018-blocking task with a closure
record limited to executed Appendix B lane-0 scope and links to T019–T029.
Keep C1/history, host retention, snapshot wire, lanes, reconnect and IR as
open children. Preserve the original non-binding appendices and approval receipt.

Alternative: keep E017 open as umbrella until the evolution queue is complete;
or close it now only after the maintainer accepts all residual owner assignments.
**Call:** select lifecycle and approve residual ownership; no claim that T018
implemented host capture, richer events or reconnect. Its two recorded
deviations remain visible: postcard feature narrowing and the sim spend defect.

## Decision output

An append-only approval receipt with chosen options, exact remaining scope,
ADR/spec reconciliation owner and dates. Until then T022 and privileged/server
endpoint work stay blocked; T019 does not need C0.

## Agent log

- 2026-09-28 (UTC) · opencode/gpt-6-astra + C0 decision brief · Separated already-approved local Transport from unresolved contracts wording, host placement and privilege policy. Proposed E017 supersession without closing unimplemented evolution work.
- 2026-09-28 (UTC) · opencode/gpt-6-astra + delegated C0 resolution · Selected definitions-only contracts with net-local Transport, crpg-server shared authority, explicit invitation/capability policy and E017 supersession. Exact APIs and ADR/spec reconciliation remain named deliverables rather than repeated human option gates.

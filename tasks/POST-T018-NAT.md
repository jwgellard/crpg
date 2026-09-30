# NAT reachability — first real server scope

**D05 selected: documented manual forwarding for the first real server.**
See [decision record](POST-T018-DECISIONS.md#d05--nat-manual-forwarding-first).
No IGD dependency or router modification is authorized; alternatives below are history.
ADR-0004/T002b established that bind-and-listen behind an unmodified home
router was unreachable. UPnP being enabled did not create an application mapping.

## Recommendation and options

Recommend documented manual UDP port forwarding for the first real dedicated
server: explicit listening port, firewall/operator prerequisites, external
address verification and an honest CGNAT limitation. This has no new crate
dependency and is testable with two separately owned networks.

Alternative: approve `igd` (exact version/features/source still a dependency
decision) and a host-adapter-only mapping lifecycle: opt-in, discover, request,
renew, bounded timeout, remove on clean shutdown and actionable failure.
UPnP must not be described as general NAT punching or relay support.

## Call and handoff

Choose manual or IGD and the operator UX/failure contract. If IGD is selected,
write a separate task in the selected host-adapter crate after E012/E022; do
not add NAT code to core/rules/sim or bundle it with net transport. Specify
real external-network success and unreachable-router controls for both native
dedicated products. This is **not critical path until a real server exists**;
it does not block T019, codec, or in-memory host conformance.

## Agent log

- 2026-09-28 (UTC) · opencode/gpt-6-astra + NAT scope brief · Recommended a bounded manual-forwarding first-server scope while retaining IGD as a real choice. Kept reachability proof separate from QUIC correctness and CGNAT claims.
- 2026-09-28 (UTC) · opencode/gpt-6-astra + delegated NAT resolution · Selected manual forwarding and deferred IGD/CGNAT traversal, leaving real two-network reachability acceptance with the dedicated-server task.

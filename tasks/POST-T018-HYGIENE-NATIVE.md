# E021 and E023 — separate governance briefs

**D07 selected: illustrative-only example plus retrospective T004; defer native loading.**
See [decision record](POST-T018-DECISIONS.md#d07--hygiene-and-native-extensions-selected).
No ABI, unsafe or native dependency permission is granted; alternatives below are history.

## E021 brief — embedded contract hygiene

Recommendation: label the in-spec rules contract example illustrative-only,
point to root/crate AGENTS and authoritative lints, and backfill a one-page
T004 historical record from git evidence. Do not forge retrospective test
results. Alternative: remove the duplicate example in a separately approved
spec edit and annotate T004's index instead of creating a task file.

**Call:** choose example treatment and retrospective file versus annotation.
These future edits are documentation-only; this session does not edit the
spec, root rules or existing attribution. No copied example can authorize
cross-crate tests/dependencies contrary to the live rules.

## E023 brief — native extension ABI and unsafe governance

Recommendation: keep dynamic native loading blocked until an explicit ABI,
artifact and governance decision; prefer a versioned narrow boundary with
target/toolchain-specific artifacts and no unload of live objects. No stable
Rust ABI promise. Bind extension trust to E018 and authority to E012/E022.

Genuine options:

- Approve a narrowly owned in-process loader/FFI boundary with an explicit
  unsafe-policy amendment, exact ownership/threading/error and lifetime rules,
  artifact identity, signing/discovery and rejection behavior. The existing
  only-crpg-godot-unsafe rule currently prevents a headless host loader.
- Use a process-isolated extension protocol, avoiding an in-process native
  ABI at the authority boundary, with explicit IPC/latency/crash semantics.
- Defer native extensions and support portable data/sandboxed scripting first.

**Call:** select mechanism and owner, supported target/toolchain negotiation,
trust root, packaging/discovery, offline failures, unload policy and unsafe
governance **before any implementation**. Never put Godot into Linux headless
to obtain an unsafe exception. Any linter/governance change is separately
reviewed, self-tested work; no dependency or deny widening is implicit.

Future implementation splits loader, host adapter and packaging by their
single owning crates, with genuine Windows/MSVC and Linux/GNU compatibility,
wrong-target/version/signature and valid-artifact controls. This brief grants
no ABI, loader, library or signing implementation permission.

## Agent log

- 2026-09-28 (UTC) · opencode/gpt-6-astra + E021/E023 governance briefs · Presented independent hygiene and native-extension choices while retaining explicit human gates. Kept ABI/unsafe ownership unresolved rather than borrowing the Godot exception for headless servers.
- 2026-09-28 (UTC) · opencode/gpt-6-astra + delegated hygiene/native resolution · Selected E021's documentation treatment and explicitly deferred native-extension implementation beyond the queue. Existing unsafe governance remains intact, with no ABI promised.

# T047c fact sheet — client CI job (`client`) and `deny.toml`

Facts gathered from the repository at branch `task/T047c-facts-b1`, 2026-11-02.
No recommendations, no proposed YAML. Every `path:line` below resolves on this
branch.

## 1. `.github/workflows/ci.yml` today

File: `.github/workflows/ci.yml` (221 lines). Workflow `name: ci` (line 1).
Triggers (lines 3–12): `push` on `master`; `pull_request` (no path filter);
`schedule` cron `"0 6 * * 1"` (weekly Monday 06:00). Permissions `contents: read`
(lines 33–34). Concurrency group `${{ github.workflow }}-${{ github.ref }}`,
cancel-in-progress unless master (lines 38–40).

Jobs (in file order):

| Job | lines | runs-on | timeout | notes |
|---|---|---|---|---|
| `guard` | 43–87 | `ubuntu-latest` | 10 | `if: github.event_name == 'pull_request'` (line 51). Fork-PR protected-paths + trusted-lint gate over `tools/lint/**` and `.github/workflows/**` (lines 59–86). |
| `secrets` | 88–101 | `ubuntu-latest` | 10 | gitleaks. |
| `check` | 102–161 | matrix `os: [ubuntu-latest, windows-latest]` (lines 103–105), `${{ matrix.os }}` | 30 | Rust gates. |
| `deny` | 162–181 | `ubuntu-latest` | 15 | cargo-deny. |
| `lint-architecture` | 182–193 | `ubuntu-latest` | 10 | `python tools/lint/deps.py`. |
| `lint-determinism` | 194–203 | `ubuntu-latest` | 10 | `python tools/lint/determinism.py`. |
| `lint-selftest` | 204–221 | matrix `os: [ubuntu-latest, windows-latest]` | 10 | `python -m unittest discover -s tools/lint -p "test_*.py"` and `tools/tests/test_preflight.py`. |

Actions pinned (lines 53–217):
- `actions/checkout@v7` (lines 53, 92, 109, 166, 186, 198, 213)
- `actions/setup-python@v7` with `python-version: "3.11"` (lines 56–58, 189–191, 199–201, 214–216)
- `gitleaks/gitleaks-action@v3.0.0` (line 98)
- `Swatinem/rust-cache@v2` (line 148)
- `EmbarkStudios/cargo-deny-action@v2.1.1` (lines 171, 178)

Windows toolchain install: there is no explicit Rust install step. The workspace
`rust-toolchain.toml` (channel `1.98.0`, components `clippy`, `rustfmt`,
lines 1–3) drives rustup on both OSes. The `check` job's Rust steps (lines
150, 157, 159) run only when `steps.scope.outputs.rust == 'true'`.

Docs-only classification logic (lines 112–147): job `check` always runs and
reports (lines 112–115: "a workflow-level path filter would leave them pending
for ever"). Step `Classify the change …` id `scope` (lines 124–147). On non-PR
events, `rust=true` (lines 129–132). On PR, `BASE_SHA` fetched, `MERGE_BASE`
computed, `CHANGED` from `git diff --name-only --no-renames`, then
`CODE="$(… | grep -vE '^(docs|tasks)/|^[^/]+\.md$|^crates/[^/]+/AGENTS\.md$' || true)"`
(line 138). If `CODE` non-empty → `rust=true` (lines 139–142); else
`rust=false` (lines 143–146). Allowlist: `docs/**`, `tasks/**`, root-level
`*.md`, and `crates/*/AGENTS.md` (lines 116–117). `--no-renames` lists both
sides of a move (line 119). Pushes to master and the weekly schedule always run
every gate (line 121).

Caching (line 148): `Swatinem/rust-cache@v2` gated on
`if: steps.scope.outputs.rust == 'true'` (line 149). No other cache step.

Required checks recorded in-repo: `ci.yml` comments lines 114–115 state the
master-protection ruleset requires both `check` contexts; `tasks/PROJECT_STATE.md`
and `docs/adr/0033-godot-bridge.md:451,589` record that `client` must be added
to required checks once T047c lands (C7). No branch-protection config file exists
in the tree.

## 2. Other workflows under `.github/workflows/`

Only `.github/workflows/ci.yml` exists. No other workflow files are present.

## 3. `deny.toml` today and T047.md's exact text

`deny.toml` (103 lines). Relevant sections verbatim:

`[licenses]` allow list (lines 20–29):
```toml
allow = [
    "MIT",
    "Apache-2.0",
    "Apache-2.0 WITH LLVM-exception",
    "BSD-2-Clause",
    "BSD-3-Clause",
    "ISC",
    "Unicode-3.0",
    "Zlib",
]
```
`unused-allowed-license = "allow"` (line 41). No `exceptions` key exists today.
`include-dev = true` (line 15).

`[bans]` (lines 43–79): `multiple-versions = "deny"` (line 44),
`multiple-versions-include-dev = true` (line 47), `skip = [ … ]` (lines 66–79)
listing `getrandom@0.3.4`, `getrandom@0.4.3`, `r-efi@5.3.0`, `r-efi@6.0.0`,
`rand@0.9.5`, `rand_core@0.9.5`, `windows-sys@0.52.0`.

`[advisories]` (lines 81–94): `unmaintained = "all"` (line 83),
`yanked = "deny"` (line 94).

`[sources]` (lines 96–103): `unknown-registry = "deny"` (line 97),
`unknown-git = "deny"` (line 98), `allow-git = []` (line 99),
`allow-registry = ["https://github.com/rust-lang/crates.io-index"]` (lines 101–103).

`[graph]`: **absent** from `deny.toml` today (no section).

`tasks/T047.md`'s exact `deny.toml` text (Interface block, lines 312–345 of
`tasks/T047.md`): insert after `unused-allowed-license = "allow"`
(`deny.toml:41`):

```toml
# ADR-0033 U1, answered (a) by the user in D41c (2026-11-02): MPL-2.0 is
# allowed for exactly these eight godot-rust (gdext) packages, by name, and
# for nothing else; the `allow` list above is unchanged. The version pin is
# `godot = "=0.5.5"` in crates/crpg-godot/Cargo.toml.
exceptions = [
    { crate = "godot", allow = ["MPL-2.0"] },
    { crate = "godot-bindings", allow = ["MPL-2.0"] },
    { crate = "godot-cell", allow = ["MPL-2.0"] },
    { crate = "godot-codegen", allow = ["MPL-2.0"] },
    { crate = "godot-core", allow = ["MPL-2.0"] },
    { crate = "godot-ffi", allow = ["MPL-2.0"] },
    { crate = "godot-macros", allow = ["MPL-2.0"] },
    { crate = "gdextension-api", allow = ["MPL-2.0"] },
]
```

Then add before `[licenses]`:

```toml
# ADR-0033 C5: `engine` is off by default, so cargo-deny resolves the
# godot packages only when asked to. Resolution only; nothing is built.
[graph]
features = ["crpg-godot/engine"]
```

C5 negative control (from `tasks/T047.md:344–345`): removing one exception must
fail `deny` with `licenses`. `crate =` is the current PackageSpec form; the old
`name`/`version` table form is deprecated (`tasks/T047.md:340–341`).

## 4. What `tasks/T047.md` says T047c owns or must pin

Lines mentioning T047c (`tasks/T047.md`):
- `tasks/T047.md:29` — `| 4 | **T047c** | `.github/workflows/ci.yml`, `deny.toml` (text in Interface) | **yes** | Claude Sonnet |`
- `tasks/T047.md:56` — `(T047c only). Workspace `Cargo.toml`: **untouched**.`
- `tasks/T047.md:312` — `**`deny.toml` (T047c, exact text).** Insert after `unused-allowed-license = "allow"` (`deny.toml:41`):`
- `tasks/T047.md:344` — `` `cargo metadata` is **unverified**. T047c proves it with C5's negative ``
- `tasks/T047.md:573` — `- **Left to T047c:** BR-21 (no `godot` in the default graph on either target),`
- `tasks/T047.md:663` — `about 350 lines of GDScript, one scene. **T047c gets its own C package (`tasks/T047c.md`) before it is`
- `tasks/T047.md:671` — `the user's S-set review covers T047b and T047c.`
- `tasks/T047.md:685` — `(`tasks/DECISIONS-D41.md:41-43`). T047c's C3 uploads the DLL as a build`
- `tasks/T047.md:686` — `artifact, and no task owns the notice. **Recommend:** T047c adds a short`
- `tasks/T047.md:709` — `` `[graph]` it likely does not, so licence coverage starts only with T047c.``
- `tasks/T047.md:741` — `` `crpg-godot/engine` and godot-tree advisories (T047c); whether removing the ``
- `tasks/T047.md:748` — `… and T047c's own contract obligations; …`

BR-21 (`tasks/T047.md`):
- `tasks/T047.md:573` — `- **Left to T047c:** BR-21 (no `godot` in the default graph on either target),`
- `tasks/T047.md:664` — `implemented.** That contract must pin: the BR-21 mechanism (a command on both`

`client` job / C1–C7 (`tasks/T047.md`):
- `tasks/T047.md:574` — `the C5 `deny` negative control, the Windows `client` job, the artifact and`
- `tasks/T047.md:575` — `C7.`
- `tasks/T047.md:668` — `shape (C1 commands, docs-only classification, Windows only, `--import` before`
- `tasks/T047.md:669` — `` `-s`); the C3 artifact contents; this file's exact `deny.toml` text and C5's ``

Artifact (`tasks/T047.md`):
- `tasks/T047.md:669` — `` `-s`); the C3 artifact contents; …``
- `tasks/T047.md:685` — `T047c's C3 uploads the DLL as a build`

Licence notice / open question 2 (`tasks/T047.md`):
- `tasks/T047.md:670` — `negative control; and the licence notice (question 2).`
- `tasks/T047.md:683` — `2. **Must the eight-package licence notice ship with the CI build?** D41c's`
- `tasks/T047.md:684` — `cost was "a licence notice in the shipped game"`
- `tasks/T047.md:686` — `artifact, and no task owns the notice. **Recommend:** T047c adds a short`
- `tasks/T047.md:687` — `` `THIRD_PARTY_NOTICES` file to the artifact. It would name the eight ``

Also `tasks/T047.md:661–671` (Routing and size budget) states the T047c contract
must pin: BR-21 mechanism on both targets with its control (same command with
`--features crpg-godot/engine`); Godot 4.7.2 Windows download URL, SHA-256 and
verify step; `client` job shape (C1 commands, docs-only classification, Windows
only, `--import` before `-s`); C3 artifact contents; exact `deny.toml` text and
C5 negative control; and the licence notice (question 2).

## 5. `docs/adr/0033-godot-bridge.md` (§15 and related)

Status line (`docs/adr/0033-godot-bridge.md:4`): `Status: **Proposed**`.

Windows build / CI (`docs/adr/0033-godot-bridge.md:426–454`):
- `docs/adr/0033-godot-bridge.md:426` — `### 15. Windows build and CI`
- `docs/adr/0033-godot-bridge.md:428` — `Local: `cargo build -p crpg-godot --features engine --locked -j 2`, then`
- `docs/adr/0033-godot-bridge.md:429` — `` `Godot4 --path apps/client`. Keep `-j 2`: commit memory is tight on this machine, ``
- `docs/adr/0033-godot-bridge.md:432` — `.github/workflows/`, `deny.toml` and the rulesets are S-set.`
- `docs/adr/0033-godot-bridge.md:433` — `T047 changes none of them. A separate S package, here **T047c**, must do exactly this, …`
- `docs/adr/0033-godot-bridge.md:436` — `- **C1.** Add a Windows-only job `client`, reporting on every run with the same`
- `docs/adr/0033-godot-bridge.md:437` — `docs-only classification as `check`, that runs:`
- `docs/adr/0033-godot-bridge.md:438` — `` `cargo clippy -p crpg-godot --features engine --all-targets --locked -- -D warnings`, ``
- `docs/adr/0033-godot-bridge.md:439` — `` `cargo build -p crpg-godot --features engine --locked`, ``
- `docs/adr/0033-godot-bridge.md:440` — `` `--headless --path apps/client --import`, and BR-22. ``
- `docs/adr/0033-godot-bridge.md:441` — `- **C2.** Fetch Godot 4.7.2 at the exact version with a pinned SHA-256, no`
- `docs/adr/0033-godot-bridge.md:442` — `floating download.`
- `docs/adr/0033-godot-bridge.md:443` — `- **C3.** Upload `crpg_godot.dll` plus `apps/client` as the gate 12 slice artifact.`
- `docs/adr/0033-godot-bridge.md:444` — `- **C4.** On Linux and Windows `check`, prove BR-21: the default-feature graph has`
- `docs/adr/0033-godot-bridge.md:445` — `no `godot` package. No Linux job ever enables `engine`.`
- `docs/adr/0033-godot-bridge.md:446` — `- **C5.** Make `cargo deny` see the engine graph. `deny.toml` has no `[graph]``
- `docs/adr/0033-godot-bridge.md:448` — `` `godot`. The task proves coverage with a negative control: remove the licence ``
- `docs/adr/0033-godot-bridge.md:449` — `exception and watch `deny` fail, on `licenses`.`
- `docs/adr/0033-godot-bridge.md:450` — `- **C6.** Add the per-crate licence exceptions, if the user accepts decision U1.`
- `docs/adr/0033-godot-bridge.md:451` — `- **C7.** The user adds `client` to the master ruleset's required checks.`
- `docs/adr/0033-godot-bridge.md:453` — `The existing `check`, `deny` and lint jobs are otherwise unchanged. `apps/` is`
- `docs/adr/0033-godot-bridge.md:454` — `outside the docs-only allowlist, so a client change runs every gate.`

`engine` feature (`docs/adr/0033-godot-bridge.md:380–404`):
- `docs/adr/0033-godot-bridge.md:380` — `[features]`
- `docs/adr/0033-godot-bridge.md:381` — `engine = ["dep:godot"]`
- `docs/adr/0033-godot-bridge.md:389` — `godot = { version = "=0.5.5", optional = true, features = ["api-4-7"] }`
- `docs/adr/0033-godot-bridge.md:400` — `` `engine` is off by default. `cargo clippy --workspace` and ``
- `docs/adr/0033-godot-bridge.md:401` — `` `cargo test --workspace` compile and test the whole of `core` on both targets ``
- `docs/adr/0033-godot-bridge.md:403` — `` with no Godot code. Linux headless stays Godot-free, and every behaviour except ``
- `docs/adr/0033-godot-bridge.md:404` — `` the shell gets both-target coverage. `Cargo.lock` lists the `godot` packages ``

Client artifact / layout (`docs/adr/0033-godot-bridge.md:406–424`):
- `docs/adr/0033-godot-bridge.md:406` — `### 14. `apps/client` layout`
- `docs/adr/0033-godot-bridge.md:409` — `apps/client/`
- `docs/adr/0033-godot-bridge.md:411` — `crpg.gdextension       Windows x86_64 debug and release only`
- `docs/adr/0033-godot-bridge.md:417` — `tests/smoke.gd         headless playthrough (BR-22)`
- `docs/adr/0033-godot-bridge.md:421` — `The DLL is loaded from the workspace `target/` directory; nothing is copied`
- `docs/adr/0033-godot-bridge.md:422` — `in. Campaign path for the slice: `campaigns/fixtures/combat_basic` through`

Licence notice (U1, `docs/adr/0033-godot-bridge.md:556–584`):
- `docs/adr/0033-godot-bridge.md:556` — `### U1. May the client ship with a dependency under the MPL-2.0 licence?`
- `docs/adr/0033-godot-bridge.md:561` — `` the `godot` crate, is under MPL-2.0. All eight packages in its family ``
- `docs/adr/0033-godot-bridge.md:562` — `` are: `godot`, `godot-bindings`, `godot-cell`, `godot-codegen`, `godot-core`, ``
- `docs/adr/0033-godot-bridge.md:563` — `` `godot-ffi`, `godot-macros` and `gdextension-api` (checked in each package's manifest). ``
- `docs/adr/0033-godot-bridge.md:575` — `Cost: a licence notice in the shipped product, and`
- `docs/adr/0033-godot-bridge.md:576` — `publishing any edits we make to those files (we plan none).`

BR-21 (`docs/adr/0033-godot-bridge.md:630, :654`):
- `docs/adr/0033-godot-bridge.md:630` — `features on both native targets; BR-21 runs in CI; BR-22 runs on Windows.`
- `docs/adr/0033-godot-bridge.md:654` — `| BR-21 | The default-feature graph on both targets has no `godot` package |`

Platform scope (`docs/adr/0033-godot-bridge.md:471–473`):
- `docs/adr/0033-godot-bridge.md:471` — `Platform scope (ADR-0012): `engine` and `apps/client` are Windows only. `core` is`
- `docs/adr/0033-godot-bridge.md:472` — `platform-neutral and tested on both targets. The DLL bundles `crpg-server`, so it`
- `docs/adr/0033-godot-bridge.md:473` — `also bundles the QUIC tree, which the remote client will need anyway.`

## 6. How the repo already verifies a downloaded binary

No workflow or tool in this repository verifies a downloaded binary. Search of
`.github/workflows/` and `tools/` for `sha256`, `Get-FileHash`, `sha256sum`
returns no hits (only `BRIEF.md` itself). The closest existing practice is
verification of a downloaded crate archive, not a binary:

- `third_party/quinn-proto/VENDOR.md:39` —
  `echo "0e75…befbe  quinn-proto-0.11.19.crate" | sha256sum -c -`
- `tasks/T023v.md:436` — same `sha256sum -c -` line.
- `docs/reviews/T030-quinn/README.md:49` — `sha256sum quinn-proto-0.11.19.crate`
  (manual, no `-c`).
- `Cargo.toml:35` — comment recording the archive SHA-256
  `0e750cca…befbe`.

## 7. How existing tools or docs run `Godot4` headless

Canonical commands (from `docs/adr/0033-godot-bridge.md` and `tasks/T047.md`):
- `docs/adr/0033-godot-bridge.md:429` — `Godot4 --path apps/client` (local run).
- `docs/adr/0033-godot-bridge.md:440` — `` `--headless --path apps/client --import`, and BR-22. ``
- `docs/adr/0033-godot-bridge.md:655` — `Godot4 --headless --path apps/client -s tests/smoke.gd` (BR-22).
- `tasks/T047.md:607` — `Godot4 --headless --path apps/client --import`
- `tasks/T047.md:608` — `Godot4 --headless --path apps/client -s res://tests/load.gd`
- `tasks/T047.md:622` — `Godot4 --headless --path apps/client --import` (T047b)
- `tasks/T047.md:623` — `Godot4 --headless --path apps/client -s res://tests/smoke.gd` (T047d)
- `tasks/T047.md:108` — D9: `--import` writes `.godot/` (needed before any `-s` run, unverified).
- `tasks/T047.md:111` — scripts run as `-s res://tests/<name>.gd`.
- `tasks/T045-facts.md:209` — `Godot4 --version` output `4.7.2.stable.official.ed1daf0bf`.
- `tasks/DECISIONS-2026-10-04.md:71` — engine version pin reference.

Drafts (`tasks/T047-draft-a.md`, `tasks/T047-draft-b.md`) repeat the same
`--headless … --import` / `-s` commands; see their lines 341–343, 417, 445,
561, 564, 581–582. `AGENTS.md:88` notes `"Godot4" is available in PATH CLI`.

## 8. `cargo tree` facts

Command run (default features, normal edges, Windows target):
`cargo tree -p crpg-godot -e normal --target x86_64-pc-windows-msvc`

Result: only the root crate name contains "godot":
`crpg-godot v0.1.0 (…\crates\crpg-godot)`. No dependency package named
`godot*` appears (no `godot`, `godot-core`, `godot-ffi`, etc.) — expected,
because T047b has not landed and `engine` is off by default. Full graph has 146
lines.

Same command with `--target x86_64-unknown-linux-gnu`: same result — only the
root crate name contains "godot"; no `godot*` dependency package. 145 lines.

`cargo tree --version` output:
`cargo 1.98.0 (797e8a9bc 2026-08-05)`

## Agent log

- 2026-11-02 (UTC) · opencode-go/beam-free + T047c facts · Fact sheet for the T047c contract: gathered CI/deny.toml/ADR/T047 facts, ran the two cargo tree commands, recorded absence of godot* packages.
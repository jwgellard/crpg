# Day One

**Goal for today:** a Rust GDExtension compiled on your laptop and loaded by Godot, a public repo with green CI, and one task completed by an agent end to end.

**Not today:** the rules kernel, the campaign format, PF2e, a subscription purchase, or any design work.

**Time:** 4–6 hours, roughly half of it downloads. Start step 1 immediately and do the reading while it runs.

---

## Two decisions first (5 minutes)

**Windows or Linux?** Use **Windows, natively.** Not WSL2.

Reasons: the client and editor need real Vulkan access to a real GPU, and Godot plus GDExtension is smoothest on native Windows. Rust builds inside WSL2 that touch `/mnt/c` are painfully slow, and keeping the project inside the WSL filesystem means Godot cannot see it properly. You are also going to spend a lot of this project *playing* the thing you are building, on a gaming laptop, on Windows.

The cost is platform drift, since your CI and your eventual dedicated server run Linux. That is handled by putting both `ubuntu-latest` and `windows-latest` in the CI matrix from day one, which costs nothing on a public repo. Revisit only if Windows becomes a genuine obstacle.

> **Platform supersession note (2026-09-07):** The setup text above is
> historical. Its Linux-only eventual-server assumption is superseded by
> [ADR-0012](adr/0012-windows-primary-platform.md). Windows/MSVC
> (`x86_64-pc-windows-msvc`) is primary for development, product, release
> gates, and behavioural baselines, including client, editor, embedded
> single-player server, dedicated server, and CLI. Linux/GNU
> (`x86_64-unknown-linux-gnu`) remains fully supported for dedicated server,
> headless tooling, server-side extensibility, and CI/testing; failures are
> defects, not best effort. One platform-neutral authoritative server runs
> in-process for Windows single-player behind an in-memory transport and in
> Windows/Linux dedicated processes; the client cannot mutate authority.
> Each target must reproduce its own independently generated replay golden,
> not the other target's hashes. Linux GUI support is not promised and Linux
> headless support must not depend on Godot. All required T009c gates passed
> on native Windows/MSVC and genuine Linux/GNU in WSL Ubuntu 24.04; see the
> [completion record](../tasks/T009c.md). T009a and T009c are merged on
> `master` as of 2026-09-07, and T009b's thin `crpgc replay` wrapper is
> next.
> The native Windows development
> recommendation is not a ban on Linux testing in WSL.

**Buy a subscription today?** Not required for today — nothing in day one needs a frontier model, and you should not spend $20 before you have a repo.

But be realistic about what you are choosing between. The free agentic tier is much thinner than it was a year ago: Alibaba closed Qwen Code's free login on 15 April 2026, Google ended free Gemini CLI serving on 18 June, and the student Copilot plan came back in June as roughly 200 monthly credits rather than full Pro. What remains free is real but small and unstable. Plan to decide at the end of week one, and expect the answer to be yes.

---

## Step 1 — Start the big download now (2 minutes of typing, 30–45 minutes of waiting)

Rust on Windows needs the MSVC linker, which means Visual Studio Build Tools. It is the longest download, so it goes first and runs in the background.

Open **PowerShell as Administrator**:

```powershell
winget install --id Microsoft.VisualStudio.2022.BuildTools --override "--wait --quiet --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
```

If that command misbehaves, install `Microsoft.VisualStudio.2022.BuildTools` normally, then open the Visual Studio Installer GUI and tick **Desktop development with C++**. That workload is the only thing you need; ignore everything else.

While it downloads, check you have room. You want **60 GB free** for the toolchain, Godot, a local model, and Rust's `target/` directories, which grow alarmingly.

```powershell
Get-PSDrive C | Select-Object Used,Free
```

---

## Step 2 — Everything else, in one go (10 minutes)

Still in an Administrator PowerShell:

```powershell
winget install Git.Git
winget install GitHub.cli
winget install Rustlang.Rustup
winget install OpenJS.NodeJS.LTS
winget install Python.Python.3.12
winget install Ollama.Ollama
winget install Microsoft.VisualStudioCode
```

Then **close PowerShell and open a new one** so the PATH changes take effect. This trips up almost everyone; do it now rather than debugging a missing command in twenty minutes.

Verify:

```powershell
git --version
rustc --version
cargo --version
node --version
python --version
gh --version
```

If `rustc` is missing, run `rustup default stable`.

Add the components you will need:

```powershell
rustup component add clippy rustfmt
cargo install cargo-deny
```

`cargo-deny` compiles from source and takes a few minutes. Let it run while you do step 3.

**Editor:** VS Code with the `rust-analyzer` extension is fine and is what most agent CLIs assume. If you want something better, the GitHub Student Developer Pack includes **JetBrains RustRover** free, and its analyser handles a large Cargo workspace noticeably better. Either works. Do not spend an hour configuring an editor today.

---

> **Version-note (2026-09-05):** the `4.6.x` / `1.XX.0` strings below are the day-one bootstrap snapshot. Live pins are Godot 4.7.2 / rustc 1.98.0 — see `docs/PROJECT_STATE.md`. The embedded agent-rules template is likewise superseded by root `AGENTS.md:17-21`.

## Step 3 — Godot (10 minutes)

Download **Godot 4.6.x, the standard build, not the .NET build**, from godotengine.org. The .NET build only matters if you intend to write C#, and per the architecture you do not.

```powershell
mkdir C:\tools\godot
```

Extract the zip there. Rename the executable to **`godot4.exe`** — godot-rust looks for a binary by that name on PATH, or for a `GODOT4_BIN` environment variable.

Add it to your user PATH:

```powershell
[Environment]::SetEnvironmentVariable(
  "Path",
  [Environment]::GetEnvironmentVariable("Path","User") + ";C:\tools\godot",
  "User"
)
```

New terminal, then:

```powershell
godot4 --version
```

Write the exact version down. You are pinning this, and the version goes in an ADR later.

---

## Step 4 — Prove the toolchain (45–60 minutes)

**This is the actual milestone of the day.** Everything else is installation. This step answers the only question that matters right now: can this machine build a Rust GDExtension that Godot loads?

```powershell
mkdir C:\CRPG\dev\spike-gdext
cd C:\CRPG\dev\spike-gdext
cargo init --lib
cargo add godot
```

`cargo add godot` picks the current release, which is what you want — do not hardcode a version from a tutorial.

Edit **`Cargo.toml`** so the crate builds as a dynamic library:

```toml
[lib]
crate-type = ["cdylib"]
```

Replace **`src/lib.rs`** entirely:

```rust
use godot::prelude::*;

struct SpikeExtension;

#[gdextension]
unsafe impl ExtensionLibrary for SpikeExtension {}

#[derive(GodotClass)]
#[class(base=Node)]
struct HelloProbe {
    base: Base<Node>,
}

#[godot_api]
impl INode for HelloProbe {
    fn init(base: Base<Node>) -> Self {
        Self { base }
    }

    fn ready(&mut self) {
        godot_print!("Rust core online. Toolchain proven.");
    }
}
```

Build it:

```powershell
cargo build
```

The first build pulls and compiles the whole `godot` crate and will take several minutes. Subsequent builds are seconds.

Now the Godot side. Create a folder `godot/` inside the project, and in it a file **`spike.gdextension`**:

```ini
[configuration]
entry_symbol = "gdext_rust_init"
compatibility_minimum = 4.2
reloadable = true

[libraries]
windows.debug.x86_64   = "res://../target/debug/spike_gdext.dll"
windows.release.x86_64 = "res://../target/release/spike_gdext.dll"
```

Check the DLL name matches what Cargo actually produced. Look in `target\debug\` — if your crate is named `spike-gdext`, the DLL is `spike_gdext.dll` with an underscore.

Also create `godot/project.godot` containing just:

```ini
config_version=5

[application]
config/name="spike"
```

Then:

1. Open Godot, **Import** the `godot/` folder.
2. In the scene tree, add a new node. Search for **`HelloProbe`**. If it appears in the list, the extension loaded and **you are done — the hard part of today worked.**
3. Save the scene, press F5, and check the Output panel for the print.

### If it fails

| Symptom | Cause | Fix |
|---|---|---|
| `link.exe not found` | Build Tools missing or incomplete | Re-run step 1, confirm the C++ workload is ticked |
| `HelloProbe` not in the node list | Godot cannot find the DLL | Check the path in `.gdextension` against the real filename; paths are relative to the `godot/` folder |
| `libclang` / bindgen errors | You enabled the `api-custom` feature | You do not need it. Remove it. The default path uses prebuilt bindings and needs no LLVM. |
| Godot crashes on load | API/runtime version mismatch | Set `compatibility_minimum` to your Godot minor version, rebuild |
| Editor holds the DLL, rebuild fails | Godot has the library open | Close Godot, rebuild, reopen. `reloadable = true` helps but is not perfect. |

**Do not move past this step.** If it does not work, that is today's remaining work, and it is worth all of it. This is the load-bearing assumption of the entire architecture.

---

## Step 5 — The real repository (45 minutes)

Public from day one: unlimited GitHub Actions minutes, and the licence is permissive anyway.

```powershell
cd C:\CRPG\dev
gh auth login
gh repo create crpg --public --clone
cd crpg
```

Create the workspace manifest, **`Cargo.toml`**:

```toml
[workspace]
resolver = "2"
members = ["crates/*"]

[workspace.package]
edition = "2021"
license = "MIT OR Apache-2.0"

[workspace.dependencies]
serde = { version = "1", features = ["derive"] }
thiserror = "2"
indexmap = { version = "2", features = ["serde"] }
```

Create the stub crates:

```powershell
mkdir crates
$libs = @("crpg-core","crpg-data","crpg-rules","crpg-sim","crpg-script",
          "crpg-ai","crpg-nav","crpg-net","crpg-persist","crpg-edit",
          "crpg-contracts","crpg-testkit")
foreach ($c in $libs) { cargo new --lib "crates/$c" --vcs none }
foreach ($c in @("crpg-server","crpg-cli")) { cargo new --bin "crates/$c" --vcs none }
cargo new --lib crates/crpg-godot --vcs none
```

Put this at the top of every library crate's `lib.rs`:

```rust
#![forbid(unsafe_code)]
#![warn(missing_docs)]
//! TODO: crate purpose.
```

Exception: `crpg-godot` gets `#![allow(unsafe_code)]`, because it is the only crate permitted to have any. That asymmetry is the point.

Pin the toolchain — **`rust-toolchain.toml`**:

```toml
[toolchain]
channel = "1.XX.0"     # paste the output of `rustc --version`
components = ["clippy", "rustfmt"]
```

Pinning matters more than usual here: it means an agent cannot silently "fix" a build by changing compiler versions, and CI reproduces your machine exactly.

**`.gitignore`**:

```
/target
/build
.godot/
*.dll
*.pdb
.env
```

Verify and commit:

```powershell
cargo build
cargo fmt --all
git add -A
git commit -m "Bootstrap workspace with stub crates"
git push
```

---

## Step 6 — CI, before any real code (30 minutes)

Create **`.github/workflows/ci.yml`**:

```yaml
name: ci
on: [push, pull_request]

jobs:
  check:
    strategy:
      matrix:
        os: [ubuntu-latest, windows-latest]
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - uses: Swatinem/rust-cache@v2
      - run: cargo fmt --all -- --check
      - run: cargo clippy --workspace --all-targets -- -D warnings
      - run: cargo test --workspace

  lint-architecture:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - run: python tools/lint/deps.py
```

The `lint-architecture` job will fail until step 7 creates that script. That is deliberate: a red CI is a task, and this is your first one.

Push and watch it run. Fix whatever is red except the missing lint.

---

## Step 7 — Agent setup, and your first delegated task (60 minutes)

### Install the agent harness

Install **OpenCode**. It is MIT-licensed, model-agnostic, and the specific reason to choose it is that free model providers keep dying — the harness outlives them.

```powershell
npm install -g opencode-ai
opencode --version
```

Now configure **three providers**, so that any one of them disappearing costs you a config line rather than a week:

1. **OpenCode Zen.** Sign up at `opencode.ai/auth` and add the key. Zen rotates a set of genuinely free models while their teams gather feedback. Check `opencode.ai/docs/zen` for which are free *today* — the list changes monthly and any list I give you will be stale. Free models may train on your prompts; your repo is public, so this does not matter here.
2. **OpenRouter.** Free key at `openrouter.ai`, then use the `:free` model variants. Roughly 50 requests/day by default.
3. **Local Ollama** (next section) as an OpenAI-compatible endpoint. Slow and weak, but it never has an outage and never changes its pricing.

Do not skip providers 2 and 3 because provider 1 is working. That is precisely the mistake that leaves you stranded on a Tuesday morning.

### Install a local model

```powershell
ollama pull qwen2.5-coder:7b
ollama run qwen2.5-coder:7b --verbose
```

Ask it something trivial and read the tokens/second at the end. Write the number down. On your 4060 at Q4 you should see comfortably usable speed. This model is not for architecture; it is for commit messages, doc comments, and eventually campaign JSON.

### Write the instruction files

**`AGENTS.md`** at the repo root. Keep it short — it is loaded on every session and cached, so it should be stable:

```markdown
# CRPG engine — agent rules

Rust workspace. Simulation core has NO game-engine dependency.

## Non-negotiable
- Only `crpg-godot` may depend on `godot`. Only `crpg-godot` may use `unsafe`.
- Dependency direction: core <- data <- rules <- sim <- {net, ai, script} <- server.
  Never import upward. Never create a cycle.
- No `HashMap` iteration and no `f32`/`f64` in `crpg-rules` or `crpg-sim` rules paths.
  Use `IndexMap`/`BTreeMap` and integers or fixed-point.
- Do not add dependencies without being asked.
- Do not modify `crpg-contracts` or `rust-toolchain.toml`.
- Do not weaken or delete an existing test to make a build pass. Stop and say so.

## Working rules
- One task = one crate. If a task needs two crates, stop and say so.
- Finish only when the task file's stated command passes.
- Before finishing: `cargo fmt --all`, `cargo clippy -p <crate> -- -D warnings`,
  `cargo test -p <crate>`.
```

**`tasks/T005.md`** — your first delegated task:

```markdown
## Task
Write the dependency-direction lint.

## Crate(s)
None. Creates `tools/lint/deps.py` only.

## Purpose
Machine-enforce the layering rule in AGENTS.md so it cannot be violated silently.
CI job `lint-architecture` currently fails because this file does not exist.

## Interface
`python tools/lint/deps.py` from the repo root.
Exit 0 if clean, exit 1 otherwise. On failure, print one line per violation:
`VIOLATION <from-crate> -> <to-crate> (<rule>)`

## Behaviour
- Parse every `crates/*/Cargo.toml`.
- Build the graph of workspace-internal dependencies only. Ignore external crates
  except for the `godot` rule below.
- Fail if any crate other than `crpg-godot` depends on `godot`.
- Fail on any cycle.
- Fail on any edge not in this allowed-edges table:
    crpg-core       -> (none)
    crpg-data       -> crpg-core
    crpg-rules      -> crpg-core, crpg-data
    crpg-sim        -> crpg-core, crpg-data, crpg-rules
    crpg-nav        -> crpg-core
    crpg-script     -> crpg-core, crpg-data, crpg-rules, crpg-sim
    crpg-ai         -> crpg-core, crpg-rules, crpg-sim, crpg-nav
    crpg-net        -> crpg-core, crpg-data, crpg-sim
    crpg-persist    -> crpg-core, crpg-data, crpg-sim
    crpg-edit       -> crpg-core, crpg-data, crpg-rules
    crpg-contracts  -> crpg-core
    crpg-testkit    -> any
    crpg-server     -> any except crpg-godot, crpg-edit
    crpg-cli        -> any except crpg-godot
    crpg-godot      -> any

## Constraints
- Standard library only. No pip installs. Use `tomllib`.
- Under 150 lines.

## Test
Add `tools/lint/test_deps.py` with fixture Cargo.toml trees covering: a clean graph,
a cycle, an upward edge, and a non-crpg-godot crate depending on godot.
`python tools/lint/test_deps.py` must pass.

## Definition of done
- `python tools/lint/deps.py` exits 0 on the current repo.
- The self-tests pass.
- CI job `lint-architecture` is green.

## Out of scope
Do not touch any Cargo.toml. Do not touch any Rust file. Do not add a
determinism lint. Do not modify the CI workflow.
```

### Run it

```powershell
git worktree add ..\crpg-t005 -b task/T005
cd ..\crpg-t005
opencode
```

Then:

> Read `AGENTS.md` and `tasks/T005.md`. Implement exactly that task and nothing else. Stop when `python tools/lint/deps.py` exits 0 and `python tools/lint/test_deps.py` passes. If anything in the task is ambiguous, stop and tell me rather than guessing.

**Use the worktree.** Get the habit today, on a task where it does not matter, so that it is automatic later when running two agents at once would otherwise corrupt your working directory.

When it finishes, read the whole diff. It is 150 lines and you should understand every one. Then:

```powershell
git add -A
git commit -m "T005: dependency-direction lint"
git push -u origin task/T005
gh pr create --fill
```

Merge when CI is green.

---

## End-of-day checklist

- [ ] `godot4 --version` works and the version is written down
- [ ] A Rust GDExtension builds and Godot loads it — **the one that matters**
- [ ] Public `crpg` repo, workspace builds, 15 stub crates
- [ ] `rust-toolchain.toml` pinned
- [ ] CI runs on Linux and Windows and is green
- [ ] `AGENTS.md` written
- [ ] One task specified by you, implemented by an agent, reviewed by you, merged
- [ ] OpenCode installed with three providers configured, local 7B model pulled and benchmarked

If you got through step 4 and nothing else, today was still a success. If step 4 failed, tomorrow is step 4 again.

---

## What comes next

**Tomorrow:** spec task T5's second half — the determinism lint (bans `HashMap` iteration and float types in the rules crates). Same shape as T005, same workflow, slightly harder. Then `docs/adr/0001-godot-relationship.md` recording what you decided and why, with the Godot version you pinned.

**This week:** T6 (`crpg-core`: `EntityId`, `Fx16_16`, `DeterministicRng`) and T7 (the world store). Both are pure Rust, heavily testable, and ideal for the free agent tier.

**Next weekend:** spike T1 properly — 200 skinned characters driven from an external Rust array, measured. Today's `HelloProbe` proves the toolchain; T1 proves the architecture. Do not conflate them.

**Do not do this week:** design the campaign format, think about PF2e, open the rules kernel, or install anything else. The lints and `crpg-core` first. They are boring and they are what makes the next two years possible.

---

## One warning

The temptation on day two will be to skip ahead to something visible, because lints and id types are not fun and a rendering demo is. Resist it for about three weeks. The determinism lint and the replay harness are the instruments you will use to debug everything else, and building them after the code they measure is how projects end up with a simulation nobody can reason about.

---

## Agent log

- 2026-09-05 (UTC) · opencode/muse-spark + stale-pin hygiene · Added a version banner pointing at the live Godot/rustc pins in PROJECT_STATE.md; historical 4.6.x/1.XX.0 strings left intact as bootstrap snapshot.
- 2026-09-07 (UTC) · opencode/gpt-6-astra + T009c documentation alignment · Appended a visible ADR-0012 supersession note beside the Linux-only server assumption without rewriting the dated setup text. Clarified shared Windows/Linux server authority and pending native verification, including WSL Ubuntu availability.
- 2026-09-07 (UTC) · opencode/gpt-6-astra + T009c verification status · Updated only the active supersession note to reflect the reported passing native gates and link the completion record, preserving the historical setup text. T009c remains awaiting review/merge before T009b, with T009a also uncommitted and final audit still running.
- 2026-09-07 (UTC) · opencode/gpt-6-astra + T009c final audit · Updated the active supersession note to the reported completed final audit and working-tree implementation/verification completion, preserving historical setup text. Review/merge remains outstanding and T009c stays ahead of T009b, with T009a also uncommitted.
- 2026-09-07 (UTC) · opencode/big-pickle + T009a/T009c merged · Updated the active supersession note to the merged state; T009b is next.

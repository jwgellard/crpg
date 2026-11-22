# Project state

Updated: 2026-11-22

## Handover
- **In flight (session 101, 2026-11-22 UTC):** T157a (sides, `crpg-sim`, S-set) on slot 1 branch `task/T157a-i1`, commit `9e0c9d7` on base `386dff1`; the Claude implementer hit its 80-turn cap after committing, while its own acceptance run may still have been going. Its final report never arrived: nothing is verified yet. PR #105 merged (`332bd8b`). Batch `mgr/session-101-batch` (pushed) holds the T157 contract revision 5 + manager fix (three Sonnet review rounds, last verdict 1 low, fixed), D79/D80 answered, and ADR-0029/0036 Amendment 1 Accepted by the user. Brief: `.agents/briefs/T157a.md`.
- **Next:** (1) `python tools/agents/verify_task.py T157a --slot C:/crpg/dev/crpg-slots/slot-1 --brief C:/crpg/dev/crpg/.agents/briefs/T157a.md`; on a fail, fix round to a Claude implementer with the output. (2) Mutant run (Claude reviewer), then the user's H-review (S-set). (3) T157b then T157c (S-set, serialized), T157d, e, f (S-set), g per `tasks/T157.md` §Parts. (4) Follow-ups T158–T162 (engage trigger kinds), the CI `client` job running every engine gate (walk, hud, movement, combat, enemy_ai; S-set workflow), AI sides (D78a row), items (4)–(7) of session 99.
- **Blocked:** OpenCode free and Go on `quota` until about 2026-11-23. Claude usage limit reached at the end of session 101.
- Session 100 facts: free DeepSeek (`quota`), free Muse Spark (`model`) and Go DeepSeek (`quota`) all refused T156a at start. Claude implementers: T156c 140k subagent tokens/8 min, T156a 144k/8 min (in parallel, `-j 2`, 36 GB disk was enough), T156b 98k/5 min, fix A 84k/5.5 min; mutant reviewer 103k/11 min. The architect's contract assumed a separate end-turn step after the AI's Strike, but `combat_basic`'s Strike has `ends_turn`: A1/A3/E3 were corrected to one AI step (erratum in `tasks/T156.md`). `brief`-owns lists must give every path in full: `Feedback.gd` written as a bare name failed scope. In Git Bash, `python3` may be the Microsoft Store shim; workers put a wrapper on PATH for `preflight.sh`. Frame strip: `Godot4 --write-movie` at 1280x720, 240 frames, ~1 min.
- Session 99 facts: Beam wrote three fact sheets in parallel (slots 3–5) in 2.5–4 min each; the architect found the T156 sheet's GDScript line numbers mostly off by 10–60 lines and its omission that `Main.gd` opens v2 with a stand-in already playing the enemy. The user rejected an `AskUserQuestion` twice when it came before they could see/run the thing in question: tell them how to try it first, then ask.
- Session 98 facts: `verify_task.py` rebuilds `crpg-godot` without `engine`, so after it the Godot scene shows "CrpgSession is not available" until `cargo build -p crpg-godot --features engine` runs again; a slot with no `apps/client/.godot` needs `Godot4 --headless --path apps/client --import` (segfault 139 is harmless) before any `-s`, or `walk.gd` exits 2. A quota-cut OpenCode run can leave a complete draft uncommitted: commit it in the slot and hand it to the Claude implementer to check (T049a: 2 min, 60k subagent tokens, nothing to change). Claude Sonnet writers again used a cp1252 `·` (0xB7) in agent logs, failing docsig: briefs now say U+00B7. Subagent cost this session: reviewer mutant runs 82k (7 min) and 81k (8 min); T050b implementer 100k (7 min).
- Session 97 facts: Beam wrote both fact sheets in 4–5 min with verify PASS; the architect found two wrong citations in sheet A (a `#[cfg(test)]` helper cited as the dedicated start path; a short dependency list) and none in sheet B. Opus architect: 338k subagent tokens/23 min for three contracts, then 390k/6 min for revision 2; Sonnet contract review 225k/10 min found 8 real defects (one compile blocker, a false parallel-safety claim, untested GDScript input mapping). The user asked why smoothing lives in Rust; the plain answer (the samples and tick numbers arrive in the Rust client half; exact integer tests in cargo; GDScript only draws) settled D77c. The free Zen quota did not come back at 00:00 UTC.
- Session 96 facts: free and Go tiers still out, so T155e ran on the Claude implementer (9 min, 181k subagent tokens) and its plant review on the Claude reviewer (2.5 min, 52k). gitleaks scans every commit in a PR's range, so a false positive cannot be fixed by a later commit; `.gitleaksignore` takes `commit:file:rule:line` fingerprints, and a comment in it that quotes the flagged text is itself flagged. WSL five-run script: `crpg-slots/manager/t155e-linux.sh`, run from PowerShell.
- Session 95 facts: free and Go tiers still out (session 94's refusals), so all four T155c/T155d runs and both mutant runs went to Claude (implementers 121k–201k subagent tokens; mutant reviewers 74k–108k, 45–74 min; fix rounds 139k–160k). A mutant run over `main.rs` must run the binary tests (`--lib` alone skips them). The `host_quic.rs` `quic_admitted_schedule_matches_in_memory` flake (T147, BACKLOG flake row) hit 4 full-suite mutant runs and passed on rerun. The T155c implementer first handed back before its full test run finished: tell Claude workers not to hand back while cargo runs.
- Session 94 facts: free DeepSeek V4.1 Flash refused on `quota` at 09:00 UTC; Go still at its weekly limit; all T155 work ran on Claude (architect 460k subagent tokens/18 min; implementers 147k–174k each; mutant reviewers ~76k each, T155a's 46 min; fix rounds 48k–79k). Two Sonnet implementers built in parallel at 12 GB free RAM with `-j 2` without a reap, but C: filled to 0 bytes mid-run (both deleted their own slot's `target/debug/incremental` or `deps/*.pdb`/`*.exe`; 50 GB free after): check `df` before parallel cargo. A mutant driver over `crpg-server` needs `--no-fail-fast`, or a failing `--lib` hides the integration killers. T155b's R4 cannot catch the `accept_resume` lane-1 claim (loopback too fast), as the contract foresaw. Follow-ups for the backlog: make `"movement": true` the dedicated server default once terminal play and the test clients speak `/3` (D76c); T155b M22 (`open_bi` failure after the answer maps to `Handshake`, not `lost_error`; low).
- Session 93 facts: free Muse Spark (`model`), free DeepSeek V4.1 Flash (`quota`) and Go DeepSeek V4.1 Flash (`quota`) all refused T100f at start, so it went to the Claude implementer (64 s, 29k subagent tokens). WSL five-run script: clone from the main repo into `~/t041d-linux`, `CARGO_BUILD_JOBS=2`, read the `finished in` times; timing with `date +%s.%N` through `awk` printed garbage (one negative).
- Session 92 facts: free DeepSeek V4.1 Flash (`quota`), free Muse Spark (`model`) and free GLM-5.3-Flash (`model`) all refused at start: the free tier was unusable all session, so T041d went to the Claude implementer (8 min, 152k subagent tokens) and its mutant run to the Claude reviewer (6 min, 72k). `verify_task.py` gates failed once with `os error 1455` (paging file too small) at 14.5 GB free RAM after the reviewer's cargo run: rerun with `CARGO_BUILD_JOBS=2` passed. A fix round's verify needs `--base <mutant-run commit>` and a fix brief whose owns list adds `reviews/<ID>/` (session 82 rule). Cleanup at the user's request: 105 merged `task/*` branches deleted, landed slots 1/3/4 detached, the stale OpenCode `review-t151` worktree removed. The T041d contract's own G1 let a skipped save pass, because setup writes the same file the quit should: for any save-on-exit check, compare against what setup wrote.
- Session 91 facts: a `verify_task.py` brief grants protected files only under a `## Permissions` heading (`- protected: <path>`), not in "Files you own"; `.agents/` is gitignored, so pass `--brief` as an absolute path into the main repo. Free Muse Spark refused at start (`model`) again; free DeepSeek V4.1 Flash hit `quota` after finishing both row edits (manager added the log line). The T040b chain (implement 20 min, mutant run 30 min, fix round 8 min, T040c 1 min) ran on Claude alone because Go is over its weekly limit. `git push -f` is blocked by auto mode: fix-ups go in a new commit. Heredoc text through the Bash tool turned a literal `\r\n` into a newline twice: use the Edit tool for escapes in docs.
- Session 90 facts: the Go weekly limit stopped the baseline mid-batch; quota-cut runs are not excluded by T100-E2 if OpenCode logs a retry step after the error (D75); authorbench `run` needs `--models`, and its `--crpgc` must not be the out dir's own bin/crpgc.exe (WinError 32); T151c went through Claude implementer, then reviewer mutant run, then fix round, all on Claude alone, in about 20 minutes of agent time; the `Godot4 --import` crash (exit 139) recurred, harmless per crpg-godot AGENTS.md.
- Session 89 facts: an account switch mid-session (5-hour limit on the old account) stopped the Claude reviewer mid-run; `SendMessage` to it resumed it with its work intact (check the slot for a planted mutant first). Free Muse Spark refused at start again (`model`, infrastructure); Beam took the T102 prototype (new-file draft work) in 6 minutes. `verify_task.py` on a fix round reads the original brief's owns list: pass `--brief <fix brief>`. The original session-86 service-fault logs no longer exist (re-run in session 87), so T100-E2 is pinned by test fixtures, not by a real log.
- Session 88 facts: the T100 run chained from session 87 survived `/clear` and finished; its `score` stdout was lost (scores.json was written), and re-scoring is byte-identical. Errata written by the manager without reading the transport layer failed twice (T040-E1's oversized frame is refused client-side by `QuicClient::try_send_channel`): for an erratum about what reaches the server, check the client send path. Free Muse Spark refused bookkeeping with `model` at start again (infrastructure). The T101 architect, not a script, caught the scorer's zero `errors` column: after any new benchmark, check that each column varies.
- Session 87 facts: the Claude reviewer mutant run on T052 (45 rows, union of 8 crpg-sim suites, survivors re-run on the full suite) took 64 min and 87k subagent tokens. Claude Code's low-memory reaper killed both background shells (the 3-way authorbench baseline and a Go cargo worker) while the session was idle at about 15 GB free: the Go worker had written its test and killed M45 but not gated or committed; the manager ran `cargo fmt` + `verify_task.py --brief --base` (PASS) and committed. Run a cargo worker and the baseline at `--par 3` not at once; `--par 2` for the baseline beside nothing else. `oc.py run` doc-only bookkeeping on Go Muse Spark again had no shell to commit (commit by hand after PASS). The T052 mutant driver's ids are renumbered after pruning (`_DROPPED`): name the driver's id in fix briefs, not the draft id. Linux CI caught a poll-count race Windows never showed: contracts must not promise a fixed number of polls for an asynchronous end.
- Session 86 facts: session 85 was cut off at ~16:31 local with two fix rounds (T154d, T151b) written but not driven; both `oc.py` logs said `INFRA verify-error`. The manager drove the remaining rows itself and committed: check slots for uncommitted fix-round tests at start when a handover is missing. The T151b fix round's `fa_2` failed on the unmutated tree, so it "killed" every mutant: the driver has no baseline gate; run `cargo test --lib` first and treat a mutant whose only killer fails unmutated as unproven. An `oc.py run --role tests` with `--base` at the pre-implementation commit left the branch at base + the tests commit, without the implementation (`341fced` lost from the branch, verify FAIL with 131 compile errors): rebuilt by `reset --hard 341fced` + cherry-pick; for a two-keys run, pass `--base` but check `git log` of the branch after, or set the base to the implementation tip. `oc.py run` refused slot 4 still on landed `task/T100a-i1` (`slot-occupied`): detach landed slots first. Go Muse Spark bookkeeping (PASS) and Go DeepSeek fact sheet (120-step cap) both left edits uncommitted: commit leftovers after every PASS. Claude Sonnet T052 implementer: 168k subagent tokens, ~15 min, 80 tool uses.
- Session 84 facts: three Go DeepSeek V4.1 Flash runs in parallel (two cargo, one doc) at about 16 GB free RAM, no reap. Mutant briefs derived by script from a sibling's (T042b4 from T042b3, T042b2 from T042b4) with "run in three foreground batches, never end your turn mid-batch" both finished all rows (13 and 10 min). A ~460-line source-plus-suite part (T154d) used all 120 steps on DeepSeek V4.1 Flash and left the suite uncommitted; verify passed on the working tree, so commit leftovers after every PASS (session 62 rule held). A small fix-round diff (T100b, 125 lines) was cheaper for the manager to judge directly than a re-review.
- Session 83 facts: T042b3 (free DeepSeek V4.1 Flash, 13 min) and T041b (Go DeepSeek V4.1 Flash, 11 min) passed first attempt in parallel. A Go mutant driver run under `oc.py` died with its worker when the worker ended its turn mid-run (19/42 rows done) and left a mutant planted in `headless.rs`: check for a dirty `src`/support file after any mutant run and `git checkout --` it; the manager finished the rows with the driver's ids argument. T041b's D1–D5 kill only 16/42 support-module mutants alone, but with `headless_harness` 39/42, the rest equivalent per T041a fix B: run a later part's mutant table against the union of the crate's suites before calling a support mutant a gap. T042b3's MC2 (Begin's consistency check removed) is equivalent because `check_begin` repeats it. A `sed`-derived brief from a sibling (T042b4 from T042b3) is quick; check the owns line and Behaviour heading.
- Session 82 facts: the T041a mutant run had finished all 113 mutants before the session-81 cut-off; only the classification was missing. Folding classification into the fix round (kill each survivor, or call it equivalent or later-part with a reason) and re-running the driver on the claimed kills worked: fix A 27/27, fix B 27/28. Free DeepSeek V4.1 Flash hit its 120-step cap twice on fix rounds (T150c fix B: no edit; T041a fix B: work done, uncommitted, manager committed): give fix rounds over ~10 items to Go IDs. Fix rounds after a committed `reviews/` run always fail scope: re-verify with `--base <mutant-run commit>`. Brief owns lists must name files literally (a `<name>` placeholder failed T100b's scope); `verify_task.py --brief` takes the corrected brief. `oc.py run` refuses a slot whose branch was landed by cherry-pick: check the slot out to the next branch first. Free Muse Spark refused with `model` at start (infrastructure).
- Session 81 facts: three Go workers (record, T100a, T041a) all passed first attempt in parallel (3, 35 and 30 min; T041a 92 steps for ~1,800 lines). T042b1 passed first attempt in 18 min. Two Go cargo workers beside one Claude mutant driver ran at 18 GB free RAM without a reap. A Claude mutant run over T150c took ~38 min for 108 mutants.
- Session 80 facts: `oc.py wave` takes any clean slot, and put a Go draft into slot 4 while a just-started Claude architect had created its branch there but not yet written (moved by SendMessage before any file was lost); before a wave, check out each Claude subagent's branch in its slot so the slot is not clean-and-free, or pass explicit slots via `oc.py run --slot`. T151b attempt 2 stopped rightly on two contract contradictions (B6: the host stages a truncated frame and refuses it at pump, so `LinkStats.refused` stays 0; B10(b): `shutdown()` cleared sessions so the hook got `UnknownPeer`, 51 not 50); an Opus architect erratum (T151-E2, ~5 min) fixed both, and attempt 3 passed in 11 min. A T150c full part (~330 source + 1,900 test lines) did not fit 120 steps on DeepSeek V4.1 Flash: give large parts a split brief or a planned continuation. Sonnet architects (model override) adjudicated T100 and T042b at about 280–420k subagent tokens each; Opus T041 took ~26 min. T042b's architect found the suite is ~3,000 lines and split it into four test-only parts.
- Session 79 facts: a Claude architect started by the previous session (T040) kept running after `/clear` and committed; the new session re-dispatched the same job and had to stop it. Before re-running a Claude subagent named in the handover, wait a few minutes for a commit in its slot (or check `ps` for its processes). The T153 fix round closed all 8 targeted survivors (driver: 27/28 killed, a14 equivalent; T153 landed as `4c2e22d`, `5c7142c`, `35a1354`). D70 rulings by the user: ctrlc now (not console-only); PEM accepted long term. Two limits of the ctrlc choice are written into T040: closing the Windows console window does not guarantee a save, and a native Windows service stop saves only through a wrapper that sends Ctrl-C (backlog row for native service control before T089). The ctrlc pin adds `nix` 0.31.3 (build script) on Linux and four Apple-only crates to the lock that are never built; `cargo deny` passes unchanged.
- Session 78 facts: Go Muse Spark lost its shell on a test-only fix round again (T151a fix B; tests were right): run and commit by hand, or route test fix rounds to Go DeepSeek V4.1 Flash (T151a fix C passed in 5 min). A manager mutant driver for T153 (28 mutants) took about 13 minutes beside a Go worker's build. The `.agents/` folder is gitignored: briefs and run records stay local.
- Session 77 facts: PR #98 merged at 08:00 UTC (`d1a7c04`). The user asked for as much parallel work as possible (2026-11-19): keep doc slots 3–6 busy with fact sheets and drafts for the next contracts while Rust slots build. Free Zen still refused for 2026-11-19 UTC: Go IDs. Bash heredocs holding briefs with backticks still fail to parse: write a generator script with the Write tool (scratchpad `mkfacts.py`).
- Session 76 facts: the session-75 `git cherry` sweep rule found T152, a verified lint package with no backlog row, nine sessions after its run; most other unmerged `task/*` branches are drafts, fact sheets or superseded tips (their files differ only because later work changed them). Go Muse Spark lost its shell again on a test-only fix round (8 tests written, none run or committed): commit and run its tests by hand. A manager mutant driver must build `--lib` whenever a fix round adds `#[cfg(test)]` tests in `src` (h03/h04/h07 were killed only by them).
- Session 75 facts: four verified packages (T056a/b, T150a/b) sat on task branches for nine sessions because the session-67 handover's cherry-pick list named only T037b–d; at start, run `git cherry <batch> task/<ID>-*` for every `ready`/`done` branch, not just the handover's list. A manager mutant driver for T154c (44 mutants) took about 12 minutes of cargo.
- Session 74 facts: `oc.py wave` reported T154b `gates` FAIL while the worker's own gates were green; a hand rerun of `verify_task.py` passed and the reviewer's 35 runs (25 under CPU load) found no flake: rerun verify before charging a worker. A fix round's test for "Stop runs no further step" can race the thread's first clock read: pin the first cycle with a `call` rendezvous before moving the clock. `git cherry-pick` has no `-q`. Free DeepSeek V4.1 Flash refused on quota at 03:45 UTC: Go IDs for the rest of the UTC day.
- Session 73 facts: a session after the session-72 handover ran the T025b-q mutant driver (01:14–01:35 UTC) and ended without a handover; read `reviews/*/` timestamps against the handover commit when the state looks behind. Two mutant-driver bugs made killed mutants look like survivors: stage 1 lacked `--lib`, so `#[cfg(test)]` unit tests never ran; and the `after` anchor was not CRLF-converted, so a branch checkout that rewrote `quic.rs` with CRLF gave NO-ANCHOR. Both fixed in `reviews/T025b/qdriver.py` and `reviews/T037c/driver.py`; copy those for the next driver. A mutant pattern can go stale when a later package edits the same lines (T037c n09 after T025b-i added `graces.clear()`): run `--check` before every re-run. Free Muse Spark returned 402 at start (infrastructure).
- Session 71 facts: Claude Code's memory reap stopped three background runs at once (a Go worker, its waiter and a doc grunt) while the session sat idle; the worker had already committed. Keep background waiters to one, and expect a reaped `oc.py run` to leave its commit unverified: run `verify_task.py` by hand. `oc.py run` refuses a slot with uncommitted changes from an earlier attempt of the same task: commit the WIP on the task branch, then retry with notes. A worker that stops on a wrong brief fact (S71 bookkeeping: "T154 rows exist") is right, not a failure: fix the brief. A manager-written mutant driver (copy of T037d's `driver.py`, a 16-line `mutants.py`) ran the whole T042 table in under 2 minutes of cargo in place of a reviewer run.
- Session 70 facts: the handover had x03/e04 swapped (e04 survived, x03 was the compile error); read `results.jsonl`, not the prose. `oc.py run` fix rounds fail scope against the task base when earlier rounds touched other files; re-verify with `--base <pre-round tip>` (PASS). Integrating stacked task branches by cherry-pick in a free Rust slot on a temp branch, then `merge --ff-only` into the batch, kept the main checkout cold; add-only conflicts resolved by keeping ours and re-inserting the commit's added hunk after its context (scratchpad `addpick.py`). The free DeepSeek quota ran out again at ~11:30 UTC.
- Session 69 facts: memory, not disk, now limits Rust: 23.7 GB RAM, page file 1.5 GB (system-managed, C: ~20 GB free), idle commit 13.4 GB with Chrome/VS Code open. Two cargo builds (a worker's gates beside a mutant driver) gave a linker LNK1102 out-of-memory, and later Claude Code's memory reap stopped a Go worker (after its commit) and an `oc.py ask` review: run **one cargo job at a time**. Deleted both slots' `target/**/incremental` (stale under `CARGO_INCREMENTAL=0`). A Go Muse Spark worker stopped after one refused shell command claiming "shell denied" (no edit): fix-round notes now say a refusal is one command, never the shell. T154a runs fail verify scope by design (the run brief lacks `run_loop_contract.rs`): re-verify with the tests brief. One verify gate failure did not reproduce (full suite then passed; likely the QUIC flake or memory).
- Session 68 facts: free Muse Spark refused at start a third session running (`cause: model`, 3 s): skip it until it is seen working. `oc.py ask` cannot read slot paths (sandboxed to the main checkout): pass a contract branch with `--diff <branch> --base <batch>`. Free GLM 5.3 Flash spent a 25-min review hand-computing PRNG values and timed out; telling the reviewer "do not hand-compute; ~40 steps" got Go DeepSeek V4.1 Flash to finish in 10 min with three real findings. C: was at 2.9 GB free at start; deleting slot 2's `target/` (27 GB) and slot 1's incrementals freed 30 GB; a cold slot-2 build pushed free DeepSeek's T154a past its 30-min timeout at the final gates. `verify_task.py` reads the slot's task brief (`.agents/briefs/T154a.md`) for a tests run, so the two-keys file fails scope: pass `--brief` of the tests brief that lists both. Two Claude Sonnet mutant reviewers in parallel hit the account's weekly limit; `/login` to another account and `SendMessage` resumed both; their background drivers kept running meanwhile.
- Session 67 facts: free Muse Spark (`opencode/muse-spark-1.3-contributor-free`) refused at start twice (INFRA `cause: model`, 0 steps); Go Muse Spark took T037b's three fix parts at 79–96 steps each. Splitting a 13-gap fix round into 7+6 gaps fit the step limit where one run of 13 spent all 120 steps reading. A fix round that says "tests only, no `src`" cannot kill mutants in arms the public API cannot reach: allow `#[cfg(test)]` unit tests in the owned `src` file from the start. `oc.py run` refuses a slot that still has another task's branch checked out: `git checkout --detach` it first. The free DeepSeek quota ran out late on 2026-11-16 UTC and was back after the date change. Review answers sit in `.agents/runs/ask/<n>-<model>.jsonl`; a small extractor took the last text part (session scratchpad `ans.py`).
- Session 66 facts: the routing table listed free Zen models only "when Go limits run out", so sessions 64–66 sent every OpenCode run to paid Go; the user caught it and the table now leads with free IDs. Two cargo jobs (a Claude mutant driver and a Go implementer, both `-j 2`) ran side by side without a reap at 16 GB free RAM. A careless `git checkout <branch> --` in the main worktree switched its branch (restored); run branch checks in slots only. Muse Spark closed T150b's K11 first attempt after DeepSeek V4.1 Flash missed it twice.
- Session 65 facts: a memory-pressure reap stopped three background runs at once (a cargo verify, a mutant driver and a Go `ask`); the driver left one mutated line in slot 2 (restored by `git checkout`); run at most one cargo job beside one Go worker. `oc.py run` on a slug's first oc-attempt (T150a, after a Claude attempt 1) recreated `task/T150a-i1` from `--base`, dropping two commits; the worker rebuilt them byte-identical; before such a run tag or note the branch tip. A brief that owns only a fix round's files fails verify scope against the task base: list the earlier attempts' files as "already committed, read-only" with full paths, or pass `verify_task.py --base <pre-fix tip>`. DeepSeek V4 Pro hung on T150b (13 steps, no edit; `stats` shows DEMOTE? for implement) but passed T150a first attempt.
- Session 64 facts: the user asked "Why so much Claude?" and "why has it been forgotten?": outage-week habits (Sonnet as main implementer, 2026-11-07 to 11-13) and handovers naming Claude routes carried over after Go Plus; the Go-first check is now in MANAGER.md §3.3. Stopping a Claude worker does not stop its background shell: the stopped T150a worker's matrix kept running beside the manager's rerun, both wrote one jsonl (33 lines, one corrupt); check for its processes before rerunning. A Go fact sheet took 16 min (120-step limit, all sections committed) and two Go blind drafts 8 and 19 min; Go `ask` first-pass reviews took 8-9 min on DeepSeek V4.1 Flash and each found a real test defect (T037b B9 vacuous; T037c C3 wrong-reason pass). `oc.py record` on an `ask` review writes a minimal FAIL record against the reviewer.
- Session 63 facts: starting five Claude workers within ~40 minutes (two Sonnet reviewers, one Sonnet S-set implementer, two Opus architects) hit the Claude 5-hour limit and stopped four of them; the user restated Go-first. Resuming each by SendMessage after `/login` worked (T151 and T153 contracts finished on resume). Of the five, the T056 mutant re-check and T150b (contract allows DeepSeek V4 Pro) did not need Claude. OpenCode's `crpg-review` cannot edit, so mutant runs cannot go to it; a read-only Go review can precede the Claude mutant run. `oc.py wave` has no `--dry-run`; use `oc.py run --dry-run`. A doc slot holding a finished draft branch is refused as slot-occupied: `checkout --detach` it first.
- Session 62 facts: `oc.py run` reported PASS for T037c while the worker, out of steps, had left its whole 1,400-line suite untracked and its docs uncommitted: verify checks the working tree, so after every PASS check `git status` in the slot and commit leftovers before a retry (a retry on a dirty slot is refused as slot-occupied). T056a+b mutant review: 141 mutants, no code defect, 17 real test gaps; same-lab suites (DeepSeek wrote both) stay weak without a two-keys writer. C: reached 0 bytes during the T056b fix verify (slot 2 `target/` 19 GB); deleting both slots' incrementals freed 24 GB. A fact sheet on Go takes 6–13 minutes; Muse Spark committed its doc work itself this session.
- Session 58 facts: Stacking worked across all five parts: each part was cut from its predecessor's implementation commit while the predecessor's review ran, and every fix round was test-only, so cherry-picks were clean except appended `lib.rs` lines and doc sections (keep both). Two parts can build in parallel when the contract's "one at a time" reason is only appended files. A narrowed review of a transliteration (T026p4: normaliser plus v4-specific mutants, 34) still found one real gap. C: was at 7.5 GB free at start; deleting slot incrementals and slot 2's `target/` freed 30 GB.
- Session 59 facts: cherry-picking by subject skipped a worker's final commit because an earlier commit had the same subject — check file equality against each branch tip after a batch cherry-pick loop; a contract reviewer ran `git checkout --detach` despite read-only instructions — check the slot's branch after a review; a resumed reviewer's revision check (about 3 minutes) found 4 more test defects after the first fix round.
- Session 60 facts: a Muse doc draft can leave an uncommitted rewrite after its verified commit — check `git status` in the slot and keep the verified commit unless the rewrite is clearly better; the reviewer's mutant driver (`mut.py`, session scratchpad) re-ran just the real-gap mutants after a test-only fix round in about 10 minutes, with no extra Claude review. The user upgraded to OpenCode Go Plus (4× limits): Go is the main implementer tier again and the Claude-only review relaxations (no two-keys writer for non-S-set parts) lapse.
- Session 57 facts: the T141c first run (session 56) stopped with all work uncommitted; briefs now say commit the source once it gates green, then the suite as it grows. A part cut from an unreviewed predecessor can be stacked: T141d was cut from T141c before T141c's fix round; cherry-picking it onto the fix conflicted only where both appended to the end of `host.rs` `mod tests` (keep both). A later part's behaviour can break an earlier part's unit tests that the contract's test rules did not cover (T141e's throttle vs two T141d unit tests); the implementer stopped and the manager authorised a setup-only edit. Contract erratum: T141 D14(b)'s fixture (Q controlling only B) cannot yield Q's capture view (D4 step 1, H-3, D-10); tested with Q controlling A and B.
- Session 56 facts: C: was at 7 GB free at start; deleting both slots' `target/debug/incremental` (13 GB) freed it to 20 GB; two parallel `-j 2` builds ran all session without OOM. A Sonnet worker wrote `·` as a Latin-1 byte (0xB7) in agent-log lines and docsig failed; briefs now say write it as UTF-8. `verify_task.py` asserts does not follow method calls (`Sweep::run`); tests must call a free `assert_*` helper. Mutant reviews (25–125 mutants each) found 1–5 test gaps per part and no production defect in five parts; each fix round took 3–9 minutes on the resumed implementer. A brief's owns list must include every file the contract tells the part to touch (T143c's `count.rs` allow deletion).
- Session 55 facts: Contract reviews were worth their cost (T141: a high design flaw; T143: a cost formula s times too low and four unowned test edits; T026p: a test port that could not be done by rename). A resumed architect closes a review round in 2–3 minutes. Mutant reviews of verified suites still find 3–5 test gaps each, never production defects. The PowerShell variables $S and $s are the same variable (case-insensitive): a loop over $s clobbered a path in $S.
- Dispatch facts: commit memory is tight; two parallel cargo builds plus a third OOM'd ("paging file too small"); briefs now say rerun with `-j 2`. Brief owns-lists need full repo paths and a `## Permissions` `goldens:` line, or `verify_task.py` fails scope/goldens. Linux goldens: `wsl -d Ubuntu-24.04` has Rust 1.98.0 (`~/.cargo/bin`); run a script file, not an inline `bash -c` (quoting breaks). Stale worktrees and 100+ merged local branches removed 2026-10-24; strays archived in `crpg-slots/manager/archive-2026-10-24/`. PR #70 merged 2026-10-24 after the user approved the history_v2 baselines (D34). Slot guard: a retry on a slot with uncommitted worker files is refused (slot-occupied); commit the WIP on its task branch first. Clone WSL baselines from the main repo, not a slot worktree (its .git file holds a Windows path). A brief adapted from a sibling must have its owns line updated too: a stale one made `verify_task.py` fail scope on T137b-tests. Pass `--brief` and `--base` to `verify_task.py` for a Claude worker's slot. Parallel `oc.py ask` runs collide: they get the same attempt number and write one jsonl, so run free reviews one at a time until fixed (L5). Beam may report empty test stubs as passing: check named tests and assertion counts before trusting a pass (L6). A Beam ready-for-pr is not test-strength evidence: on T024 the Claude core review found six untested behaviours Beam passed (L7). Beam doc work: run `crpg-slots/manager/lost.py` on any trim, and check file ends and agent-log lines (L8). A brief's goldens must be in its own owns list as well as its Permissions, or `verify_task.py` fails scope; run WSL scripts from PowerShell, because Git Bash rewrites `/mnt/c` paths; DeepSeek V4 Pro review timed out at 570 s; give V4.1 Flash reviews 1500 s. Pass `oc.py run --brief` an absolute path. Slots keep stale finished branches: with a clean tree, `git -C <slot> checkout --detach <base>` frees them. Fact-sheet then blind-draft then architect ran in about 45 minutes on session 29. Session 30: `oc.py run --role tests` reports `verify-error` (INFRA) every time; rerun `verify_task.py <ID> --slot <slot> --brief <tests brief> --base <impl commit>` by hand, and it passes. `oc.py ask --diff` takes a branch ref plus `--base`, not a file. Muse's first two-keys suites (T027b, T046) passed but were weak: a Claude review found 10 and 9 tests that could not fail, so keep the Claude review on two-keys suites; one fix round closed both, and on T046 the reviewer's planted-bug runs (mutants) proved the fixes. DeepSeek on a contract revision hit its step limit before committing: check `git status` in the slot. Session 31: `host_quic` crashed once with STATUS_ACCESS_VIOLATION in preflight and passed 25/25 on rerun (flake, not charged); a DeepSeek blind draft hit its step limit before committing again (manager committed); `oc.py record` without `--slug` can attach to the wrong run file. Brief `## Permissions` grants need a leading `- ` (`- protected: Cargo.lock`), or `verify_task.py` ignores them. DeepSeek's self-written tests check lengths, not values: T047-1 let 7 of 32 mutants through, so put "assert exact values; cover every branch and both directions" in implement briefs that include tests.
- Session 32 facts: the local `Godot4` is windowed, so PowerShell neither waits for it nor sees its exit code; run `Godot4 ... -s` from Git Bash. `--import` writes untracked `*.uid` files that fail `verify_task.py` scope: delete them before verifying. `oc.py record` takes no `--slug`. Godot-family packages: T047b added exactly 13 lock entries; `cargo deny` passes until T047c's `[graph]` line makes it resolve them.
- Session 35 facts: `oc.py run` output can be empty with exit 1 when a worker runs out (T098f a1): check the slot and `.agents/runs/<ID>/`. A worker that hangs after its edits: commit them, run `verify_task.py` and the Godot runner yourself rather than a blind retry. Muse doc drafts may report "no shell" and leave the file uncommitted: commit it in the slot. A Claude review of DeepSeek's self-written Godot cases still finds survivors (5/36): keep it. The architect's Godot probes live in `C:/tmp/t051probe`.
- Session 36 facts: DeepSeek (V4.1 Flash twice, V4 Pro once) hung early on 2026-11-07 with no edits; V4 Pro passed T048-fix in between. Muse's `implement` runs report "Permission denied: shell", so it cannot commit or run Godot: give it new-file doc drafts only and commit for it. A brief that leaves a caller of a changed contract out of scope makes a good worker stop (T048-fix attempt 2, `smoke.gd`): grep callers before writing owns lists. The architect found the lost QUIC close needs a `yield_now` before `wait_idle`, not only a time reserve (`tasks/T098g.md`).
- Session 37 facts: C: hit 100% during parallel builds (LNK1318); about 26 GB free after clearing `target/debug/incremental`; clear incrementals in the slots before a wave. A verify run under load can fail QUIC timing tests (`ordering_preserved_under_concurrent_traffic` took 34 s against 6 s): rerun `verify_task.py` by hand when the machine is quiet before charging a worker. A worker's `git checkout -- <owned file>` is denied by the installed permission config although briefs allow it: workers undo planted bugs with inverse edits. Check a scene change by rendering frames from Git Bash: `Godot4 --path apps/client --resolution 1280x720 --write-movie <dir>/f.png --quit-after 90`.
- Session 38 facts: A slot holding an already-merged (cherry-picked) branch is still refused as slot-occupied: `git cherry` it, then `checkout --detach` the batch. DeepSeek V4.1 Flash timed out again on a doc draft (25 min, no edit); V4 Pro did the same draft in 10 min. DeepSeek V4 Pro committed tests before `cargo fmt`: briefs now say to run fmt before committing. A 200 ms close reserve cannot be pinned by wall-clock tests safely; prefer a construction that makes the wrong wiring impossible (D44c).
- Session 39 facts: Run `git fetch` and move local `master` to `origin/master` before any `--base master`; a stale local master (98aac38) made `verify_task.py` fail scope on 37 files. On 2026-11-07, Go ran out mid-run twice, then DeepSeek Flash Free, then the whole free Zen tier (GLM refused at 0 steps); each was an infra stop, so commit the slot's WIP before retrying. GLM-5.3-Flash Free completed a contract revision but hit 120 steps before committing. A brief that prescribes the agent-log model string is copied verbatim by a substitute model, so write "your model ID" instead. The Claude implementer hit its 80-turn cap on a full T139 half; resuming it with "finish, gate, commit" worked in 7 turns.
- Session 40 facts: Claude worker briefs live in `crpg-slots/manager/briefs/` (e.g. `BRIEF-T139c.md`); OpenCode briefs in `.agents/briefs/`. `brief.py` fails (`no-task`) for `-tests` IDs; pass `--brief` (the T139a/T139d-tests briefs were recovered into `.agents/briefs/`). C: hit 0 bytes during T139c's testkit run (LNK1180); slot 1's `target/` (18 GB) was deleted, so about 24 GB is free; build with `CARGO_INCREMENTAL=0`; slot 1's next build is cold. `verify_task.py` run against the main repo fails scope/protected/docsig on its untracked files (`.claude/agent-memory/`, `.github/workflows/opencode.yml`, `extras/`); verify in a slot. `oc.py` treats Kimi K3 as scarce: one run per 5 h window (`CRPG_GO_SPEND_OVERRIDE=1` overrides). Go quota ran out again on 2026-11-07 after about 1.5 h of DeepSeek V4 Pro work.
- Session 41 facts: The UTC date lags local time in the morning (UTC+8), and a Beam worker dated its log entry locally, failing docsig; fix the date line and re-verify. A two-keys suite cut from an old base must be rebuilt on the batch head before landing: T139b added `Lifecycle::join_after_active` and broke five literals. Slots holding cherry-picked branches are refused as slot-occupied: compare subjects with `git log master --format=%s`, then `checkout --detach`. On 2026-11-07 (UTC) Go ran out again and the whole free tier with it. The user said "Don't wait on Go, use Sonnet": Go's weekly quota resets in about a day and the monthly quota is nearly used up, so expect Sonnet as the main implementer until the monthly reset. A Beam fact sheet had 5 wrong citations, caught by the Sonnet blind drafter (L7/D38 hold). Session 41 ruling (this session only, not a standing decision): while Go is out, a blind Sonnet two-keys writer may test a Sonnet implementation, provided a Claude reviewer then plants mutants and the suite kills them.
- Session 42/43 facts: session 42 ended without a handover; session 43 rebuilt state from git and `.agents/runs` result records. When a two-keys suite alone is weak, one fix round on it (with the reviewer's mutant list as notes) closed both its own gaps and the implementer suite's gaps. A Godot frame render needs `cargo build -p crpg-godot --features engine` first, because a worker's plain cargo test leaves a DLL without CrpgSession. A manager note can be wrong: the T142-tests writer correctly followed B5's declaration order over the notes. On 2026-11-08 Go and the whole free tier ran out again mid-session.
- Session 44 facts: Go and the whole free Zen tier were out of quota all day on 2026-11-08, so every implementer, two-keys writer and reviewer was Claude Sonnet. C: has about 17 GB free. The Godot demo needs `cargo build -p crpg-godot --features engine` in the main repo before `Godot4 --path apps/client`. A blind two-keys Sonnet writer plus a Claude mutant review plus one fix round on the same writer (resumed with the survivor list) closed every gap in about 1–2 minutes of worker time.
- Session 45/46 facts: session 45 ended without a handover again; session 46 rebuilt it from git and `.agents/runs`. C: reached 0 MB when the WSL golden build joined two slot builds; deleting an idle slot's `target/` (23 GB) freed it, and WSL needed `wsl --shutdown` after its I/O error. An `oc.py` tests fix round with `--base` at the implementation commit re-cuts the branch and folds the earlier two-keys commit into the fix commit (content kept): land the original commit, then the diff. DeepSeek V4 Pro and Muse Spark both timed out with no file on 450-line blind drafts; a note saying "write a skeleton within 10 steps, then one section per edit" fixed both on retry.
- Session 47 facts: a contract's "no golden moves" list can miss a golden created earlier in the same contract (T036c moved T036b's `movement_v1`); list every golden on the branch in the brief's unchanged check, and ask the user before re-baselining. C: hit 0 MB again with slot 1 (37 GB `target/`) plus slot 2; deleting the idle slot's `target/` freed it. Kimi K3 and DeepSeek both stopped on quota or steps after writing but before or between commits: always check the slot and commit for them.
- Session 48 facts: `.claude/agent-memory/` is now in the shared `.git/info/exclude`, so a slot holding a reviewer's memory files no longer fails `verify_task.py` scope/docsig. Task IDs without their own task file (`T149b`, `T140-draft-a`) make `brief.py` print `no-task`: write the brief by hand and pass `--brief`. Worker `sed`/inverse edits under `core.autocrlf=true` leave `src` files stat-dirty with no content diff: `git diff --stat` is empty; `git checkout -- <file>` clears it. Starting an S-set implementer (T149b) while the previous S-set package's review was still running worked: both reviews found test gaps only, so nothing needed a rebase. A Claude worker told to resume with a survivor list (SendMessage to the same two-keys writer) closed three survivors in 10 minutes. An architect that writes 1,200 lines in one pass runs out of context before re-reading: plan a Sonnet contract review and one fix round. Reviewer mutant drivers: scratchpad `t036c_mutants.py` and `mutants.py` (session-scoped; gone after the session). C: fell from 35 GB to 21 GB free with slot 2's `target/` at 14 GB.
- Session 49 facts: the user asked for higher per-creature limits rather than the recommended ones; asking "what should give" (fewer creatures vs a bigger package) before the architect pass avoided a second revision. Resuming the same Sonnet reviewer by SendMessage for the revision check reused its Python byte model (scratchpad, session-scoped). Linux run script: `crpg-slots/manager/s49-linux.sh` (clone, regenerate goldens and cmp, test sim+testkit).
- Session 34 facts: Two `oc.py run`s started together race on `.git/config` when the base is a remote ref; stagger them or use a local base. Two runs sharing one `--task` ID report `verify-error`; verify each by hand. Workers date agent-log entries in local time; the brief must say UTC (`date -u +%F`).
- Session 50 facts: The Sonnet implementer left out the crate AGENTS.md agent-log entry while writing the content; verify_task's docsig caught it. Put "sign both docs' agent logs" as its own line in Acceptance. Both T140a suites missed flag bytes tested only at end of input (a truncation fails first): briefs for codec parts should say "test each bad byte with a full valid remainder after it". A reviewer mutant run (63 mutants) plus one targeted re-check cost about 2 Sonnet turns more than a plain review and found 5 survivors.
- Session 51 facts: The mutant reviews found test-only gaps in all three parts; the code had no defect in any of them. Flag-byte-02 cases masked by truncation recurred in T140b and T140d even though the brief said to use full remainders, so briefs should require each 02 case to have a 01 control that decodes Ok. Filter packages need explicit tests for revoking a grant after disclosure; T140c's six survivors were all of this kind. Resuming the same writer by SendMessage with the survivor list closed each fix round in 1–2 minutes. Starting the next S-set implementer (T140d, cut from the T140c implementation commit) while the previous part was still in review cherry-picked cleanly. Reviewer mutant harnesses live in the session scratchpad and are gone after the session.
- Session 52 facts: a Git Bash heredoc holding a brief with backticks and apostrophes failed to parse; write briefs with the Write tool. T140e (implementer, two-keys, review, fix) took about 35 minutes of wall time on Claude workers alone.
- Session 54 facts: `git cherry-pick A B C^..D` is one revision walk, not a list: it skipped A and B and conflicted; pick one commit at a time in a loop. A 403 'Attention Required | Cloudflare' authentication error stops Claude subagents mid-run; after the user's /login, resume them with SendMessage (state kept), checking the slot for a planted mutant first. Free DeepSeek's first T140r1 run was right to stop on a contract defect (P9) rather than design around it.

## Phase
Phase 4 — server and networking (T022 host, T023/QUIC chain through T023b,
T029b bindings, T038/T039 persistence, and T058 headless edit API landed;
next: T024 snapshots and the reconnect/movement/interest queue per
`docs/IMPLEMENTATION_PLAN.md`).

## Branch state
All merged work is on `master` through `16f58c8` (PR #28, 2026-10-06).
Since `f3d1560`: PR #18 (T020 history, T021 v2 protocol), PR #20 (T027a,
T028, T029a, T030–T032), PR #21 (D19–D26, ADR-0023, T033), PR #23 (T022
host, T029b bindings, T038 save envelope), PR #24 (T058a pointer edit),
PR #25 (T058 headless edit API, T023s lint/ADR step), PR #26 (T023 QUIC
transport, T023s stub), PR #27 (T039 checkpoint adapter, T023c window
limits, T023v vendored quinn-proto fix), PR #28 (T023d close-delivery
fix, T023b QUIC host adapter). Per-task detail lives in
`tasks/BACKLOG.md`; the Done history below (through T019 plus the Phase
0–3 spikes) is retained as history.
The pre-`f3d1560` record follows. T010's campaign
format, T011a's positioned validation plus its slug/graph coverage follow-up,
T011b's thin `crpgc validate` wrapper, T012a's data-half migration
framework (plus the T012/T012b Stage-2 specs), T012b's thin `crpgc migrate`
wrapper (PR #5), S001's public-repo hardening (guard/trusted-lint jobs,
`build.rs` ban, `gitleaks` scan, runner rule, `yanked = "deny"`,
branch-protection ruleset, PRs #2/#4/#6), T013a's data-owned
introspection prerequisite (PR #7), T013's scaffolding/introspection
 CLI (PR #9), and T014's stat/modifier kernel (PR #11) are merged. T015's
  dice, outcome tables, and resolution (PR #13, `76a48da`, 2026-09-26) is
  merged; T016's headless combat with `rulesets/minimal-d6` (content, sim
  adapter/controller, lifecycle/release, replay goldens, CLI proof) and
  T017's second-ruleset abstraction proof with `rulesets/srd-lite`
  (multi-pool/effect/defense/turn generalization, replay goldens, CLI
   proof) are merged (PR #14, `4686a73`, 2026-09-27). T018's lane-0 combat
   protocol with bounded postcard codec, deterministic simulated transport,
   and conformance suite (T018a/b/c, 75 net tests per native target) is
   merged (PR #16, implementation `2fc54b7`, merge `bc54896`, 2026-09-28)
   with both native gates green; T019's same-pool affordability repair
   (`crpg-sim`: summed per-pool costs, checked overflow, 11-test
   regression, unchanged replay/goldens) is merged (PR #17,
   implementation `56931eb`, merge `f3d1560`, 2026-09-29) with both
   native gates green; the post-T018 follow-on queue
   (T019–T029b, D01–D18 decided under delegation) is specified in
   `tasks/POST-T018.md`. The Done history below is
   merged work only.

## Done
- T004 workspace, 15 stub crates, CI green on Linux and Windows
- T005 dependency-direction lint
- T005b determinism lint (bans HashMap/HashSet iteration, wall-clock,
  threads, and external RNG in `crpg-rules`/`crpg-sim`, plus floats in
  `crpg-rules`; `// determinism-ok: <reason>` escape hatch). Wired into CI
  as `lint-determinism`.
- T005c `deny.toml` (licences, advisories, bans, sources) plus a `cargo deny`
  job in CI (pinned `EmbarkStudios/cargo-deny-action@v2.1.1`, cargo-deny
  0.20.2), and CI narrowed to push-on-master + pull-request so a PR branch no
  longer runs every job twice.
- T006a `crpg-core`: `CoreError`, `EntityId`, `GenerationalArena<T>`, plus the
  crate's `Cargo.toml`, module layout and `AGENTS.md`. Arena semantics are
  ADR-0006 Decision 1: generations start at 1, lowest-index slot reuse,
  ascending-index iteration as a documented invariant, and a slot whose
  generation would overflow is retired rather than wrapped (the exact
  boundary was corrected by review 2 below). The free list is
  serialized so a loaded arena allocates the ids the saved one would have, and
  deserialization rejects an arena whose slots and free list disagree. 23
  tests pass: 4 property tests (id-reuse safety, arena invariants,
  iteration order, serde round trip including next-allocation), 18 unit tests
  and a doctest. (Now 26: the boundary fixes below added three.)
- Review follow-up 1 (whole-project review, 2026-09-04). Closed the
  gaps it found, none of which any gate was catching:
  - The arena's deserialization guard accepted a *retired* slot (generation
    `u32::MAX`) that was on the free list, and would then issue an id at
    `u32::MAX` from it. Now `CoreError::CorruptArena(RETIRED_BUT_FREE)`.
  - `deps.py` read only `[dependencies]`. It now checks `[dev-dependencies]`
    and `[build-dependencies]` too (a dev-dep is still an import, and ADR-0006
    has just made dev-deps a live concept), fails closed on a crate missing
    from the allowed-edges table, and enforces the unsafe rule directly by
    requiring `#![forbid(unsafe_code)]` on every crate root but `crpg-godot`'s.
    That check immediately found `crpg-cli/src/main.rs` missing it.
  - `determinism.py` now covers `crpg-core` as well. `crpg-sim` stays exempt
    from the float ban on purpose — spec §2.4 puts positions in `f32` — and
    README/AGENTS.md now state what is actually enforced rather than the
    vaguer "rules paths".
  - The two lint self-test files used incompatible conventions, so
    `unittest discover` silently ran 5 of 11 tests and reported OK, and CI ran
    neither. Both are `unittest` now, 22 tests, with a `lint-selftest` CI job.
  - `deny.toml`: dropped the unused `MPL-2.0` allowance (weak copyleft
    pre-approved for nothing) and silenced the unused-allowance warnings.
  - Stripped the UTF-8 BOM from 14 tracked files; both lints now read
    `utf-8-sig` so a reintroduced BOM cannot mask a line-1 violation.
- Review follow-up 2 (second whole-project review, 2026-09-04).
  Same shape as the first — every finding was a **partial enumeration**, a
  check that knew about some of its cases and not the rest:
  - **The arena's exhaustion boundary was off by one, and the runtime and the
    loader disagreed about it.** `remove` bumped a slot at `u32::MAX - 1` to
    `u32::MAX` *and* returned it to the free list, so a live arena could reach
    the exact state its own `TryFrom` rejects as `RETIRED_BUT_FREE` — it
    serialized to a save it could not load — and would then issue an id at
    `u32::MAX`. `u32::MAX` is now a reserved tombstone that is never issued: a
    slot retires on *reaching* it. `remove` and `clear` share one
    `retire_or_free` so they cannot drift, and the guard gained a fourth defect
    (`OCCUPIED_AT_RETIRED`) now that an occupied slot at the tombstone is also
    impossible. The old test forced a slot to `u32::MAX` and removed, which
    tests a state no arena reaches; the boundary case one below it was the
    missing test, and is now two. Within ADR-0006 Decision 1 ("retired
    permanently rather than wrapped"), which this pins the exact edge of rather
    than reverses.
  - **`deps.py` read three dependency tables and trusted table keys as crate
    names.** A `[target.'cfg(windows)'.dependencies]` block was invisible, and
    so was any renamed dependency (`x = { package = "godot" }`). Both together
    put a godot dependency and an upward `crpg-core -> crpg-sim` edge in one
    manifest with the lint green. It now walks every `[target.*]` block and
    resolves `package` over the key, and names the offending table in the
    violation. Also: `src/bin/*.rs` are crate roots for the unsafe check, and
    `crpg-testkit` may no longer depend on `crpg-godot` (higher crates may
    eventually dev-depend on testkit, and that must not pull in the engine).
  - **`determinism.py` treated `///` as prose.** Doctests are compiled and run,
    and `crpg-core/AGENTS.md` says the bans hold "anywhere, including tests" —
    so a doctest using `HashMap` or `f64` passed. Fence bodies are scanned now
    (`text`/`ignore` fences excepted), `/* */` block comments no longer produce
    false positives, and an unterminated one is reported rather than silently
    swallowing the rest of a file. That change immediately exposed a second
    hole: `\bf(?:32|64)\b` never matched a suffixed literal like `1.5f64`,
    because there is no word boundary after a digit.
  - `EntityId` and `Slot` are now closed shapes (`deny_unknown_fields`), and an
    `EntityId` deserializes only at a generation an arena issues —
    `CoreError::InvalidEntityId`. That is a well-formedness check, not an
    authority check; authority belongs to whoever knows the sender.
  - CI: `--locked` on clippy and test (the committed lockfile was not the
    tested one), a weekly schedule so a new advisory does not wait for a push,
    `permissions`, `concurrency`, `timeout-minutes`, and a pinned Python 3.11
    for the lint jobs (`tomllib`).
  - `LICENSE-MIT` and `LICENSE-APACHE` added — the workspace has declared
    `MIT OR Apache-2.0` since T004 with neither text shipped. `ADR-0001.md`
    renamed to `0001-godot-pinned-not-forked.md`, matching every other ADR.
  - Root `AGENTS.md` was weaker than CI: it asked for `clippy -p <crate>`
    without `--all-targets`, and never mentioned the lints or their self-tests.
  - Lint self-tests: 22 -> 49.
- T006b `crpg-core`: `Fx16_16`, a 16-fractional-bit integer fixed point.
  Saturating (never panicking or wrapping) `+ - * /` plus checked and
  saturating variants, floor division for either divisor sign, `floor`/`ceil`/
  `round` (round halves away from zero) and `abs` with `MIN.abs() == MAX`.
  `Display` prints the shortest exact decimal; `FromStr` rejects inexact or
  out-of-range input rather than rounding; serde is the raw `i32`. 19 new tests
  (10 property tests) prove arithmetic against i64 oracles, so no floats
  anywhere; 45 core tests pass in debug and release. Review found only doc
  gaps (rounding-mode wording, Display/FromStr impl docs), fixed before
  landing.
- T006c `crpg-core`: `DeterministicRng` owns lazily-created named PCG32-XSH-RR
  streams in canonical `BTreeMap` order. Stream parameters derive only from the
  master seed and length-separated name bytes through SplitMix64, so first-use
  order and draws from other streams cannot shift a sequence. Range generation
  is rejection-sampled, inclusive signed ranges cover the full `i32` domain,
  and serde resumes every stream exactly. Twelve new tests pin the 16-value
  golden vector, independence/order properties, range bounds and distribution,
  and serde continuation. All required local gates pass: 57 core tests
  including the doctest, both lints, and all 49 lint self-tests.
- T006d `crpg-core`: `Tick(u64)` and `RoundCount(u32)` provide explicit
  saturating/checked simulation-time arithmetic with transparent integer serde
  and no seconds conversion. `Ulid(u128)` masks caller-supplied 48-bit timestamp
  and 80-bit randomness fields, uses canonical uppercase Crockford base32 for
  display and string serde, accepts lowercase plus `I`/`L`/`O` aliases, and
  reports length, character and overflow parse failures separately. Seventeen
  tests, including six property tests, brought core to 74 tests including its
  doctest. All required local gates passed; no proptest regression file was
  produced.
- T006e `crpg-core`: `Interner` assigns dense `u32` handles in first-intern
  order and serializes as an ordered string list. `Interners` owns distinct stat
  and tag namespaces whose private-field `StatId`/`TagId` handles are
  runtime-only and deliberately do not implement serde or `Display`. A full
  review made equality order-sensitive and made deserialization reject duplicate
  strings, then hardened RNG deserialization against invalid stream parameters
  and added a direct rejection-sampling regression. Nine interner tests and two
  RNG regressions bring core to 85 tests including its doctest.
- T007 `crpg-sim` skeleton plus the ADR-0008 core substrate: `World` reuses
  `GenerationalArena<EntityMeta>` (spawn with explicit meta, total despawn
  with auto `Spawned`/`Despawned` events), `ComponentStore<T>`,
  `Timeline` container (`BTreeMap<(InitiativeKey, EntityId)>`,
  advance rules reserved for T008), `f32` `Transform`, and generic
  `EventEnvelope`/`EventQueue` in core. Seven sim tests including two
  10,000-case property tests (skeleton round-trip, replay determinism) and
  four substrate tests. Implementation findings now pinned in code and docs:
  stores/timeline serialize as pair lists (struct keys are not JSON keys),
  `EntityMeta` is braced (a unit serializes as `null`, i.e. a vacant slot).
- T008a measurement + loop in `crpg-sim`: BLAKE3 `state_hash` over canonical
  JSON (queue bytes in, exclusions none, NaN panics by design), fixed-step
  `tick` (counter then one-system list), `end_turn` pop primitive. Seven
  tick tests including the 10,000-tick hash-identity backbone plus seed
  sensitivity. New runtime deps `blake3` + promoted `serde_json`; `deny`
  clean.
- T008b harness in `crpg-testkit` (first testkit code): fixed
  script→tick→hash interleaving, line-hex goldens with scope headers,
  exact-tick `Mismatch` vs `Io` errors, hand-rolled hex (zero new deps).
  Four wiring tests including the 10,000-tick golden round trip. Sim
  untouched; direction followed the then-unratified E005 policy, now ratified.
- Review follow-up 3 (third whole-project review, 2026-09-06). Hardened the
  T007/T008a/T008b seams with no behavior change, in four commits:
  `crpg-sim` asserts `is_finite` up front in `state_hash` (`serde_json`
  emits `null` for NaN/infinity instead of failing), `Timeline::from`
  replaces while `Deserialize` rejects duplicate entities, and `World`
  loading rejects dangling ids; `crpg-core` `EventQueue` loading rejects
  duplicate `seq` and stale `next_seq`; `crpg-testkit` `Mismatch` is now a
  truthful enum (ADR-0010: only present sides, non-UTF-8 as content
  divergence); ADR-0011 clarifies the event payload bound as fields, not
  the enum, affirming `EventQueue<SimEvent>`. Sim now 24 tests (11 tick +
  13 world, incl. interleaved 10k ops), core event 7, testkit 6. All gates
  green each time.
- T009a replay in `crpg-testkit` (merged 2026-09-07): versioned
  `.replay` format (format 1: seed, campaign/engine identity, tick count,
  ordered inputs with opaque `serde_json::Value` payloads), validation
  before playback in fixed check order, input→tick→hash playback through
  public `World` APIs only, typed `ReplayError` reusing the ADR-0010
  `Mismatch` via a boxed `ReplayDivergence`, and `play_and_verify` as the
  end-to-end gate. New runtime deps `serde` + `serde_json` via the existing
  workspace dependencies (both already in the graph; `deny` clean, lockfile
  gains two edges, no new package). Original T009a fixture/golden pair:
  `fixtures/replay_basic.replay` (8 ticks, 5 inputs: a same-tick pair, a
  gap, trailing ticks) and
  `goldens/replay_basic_rust-1.98.0_linux-x86_64_debug.golden` (8 hashes,
  generated on genuine Ubuntu 24.04 / Rust 1.98.0 Linux by a deleted
  temporary example driving the production path). T009a verification had 19 tests
  (6 harness + 12 portable replay + 1 canonical-Linux gate doing the real
  comparison); the full suite passed on Windows and on that Linux
  environment under T009a's original policy. This is the historical T009a
  verification record, not verification of T009c's two target-scoped gates;
  ADR-0012 supersedes its Linux-only selection. Sim and CLI untouched.
- T009c platform correction in `crpg-testkit` (merged 2026-09-07):
  ADR-0012 makes `x86_64-pc-windows-msvc` the primary development, product,
  release-gate, and behavioural-baseline target while `x86_64-unknown-linux-gnu`
  stays fully supported for dedicated server, headless CLI/tooling,
  server-side extensibility, and tests. Each target compares against its own
  independently generated golden under pinned Rust 1.98.0 — exact-build
  determinism, never cross-platform lockstep. Goldens: Windows
  `replay_basic_rust-1.98.0_x86_64-pc-windows-msvc_test-default.golden`
  (570 B, sha `f2385639...21edb2`) and Linux
  `replay_basic_rust-1.98.0_x86_64-unknown-linux-gnu_test-default.golden`
  (568 B, sha `31998a38...68278d`); fixture `replay_basic.replay`
  (750 B, sha `2b65e7be...c8bd0`), all LF/no-BOM with `.gitattributes`
  pinning `*.replay`/`*.golden` to `eol=lf`. Verified on native
  Windows/MSVC and genuine WSL Ubuntu 24.04 Linux/GNU: 19 testkit tests
  (6 harness + 12 portable + 1 native golden), 135 workspace tests
  (134 unit/integration + 1 doctest), and 65 lint self-tests per target,
  plus the release-profile fallback test and withheld-golden failure proof
  on each. Final audit reran the gates and byte-audited LF/no-BOM. See the
  [T009c completion record](../tasks/T009c.md).
- T009b `crpgc replay` in `crpg-cli` (merged 2026-09-07): a thin
  `crpgc replay <path> [--golden <path>]` over `crpg_testkit::play_and_verify`
  with the clap-convention exit codes 0/1/2 (hand-rolled args for one
  subcommand; clap revisited at T013), a provisional reference-intents apply
  (`spawn_at`/`timeline`/`draw`/`despawn` over public `World` APIs,
  superseded when T014/T016 land), and no replay semantics in the crate. The
  task opened the crate: arch doc + crate `AGENTS.md` + architecture README
  row from "due with T013". 23 CLI tests (10 unit + 13 integration) pass on
  Windows/MSVC and genuine WSL Ubuntu 24.04 Linux/GNU, including real
  comparisons against each target's golden; workspace 158 and 65 lint
  self-tests pass per target. No new package in the lockfile; strictly
   verify-only (no rebless command).
- T010 campaign data format and schema gate in `crpg-data` (merged
  `15ac9fb`): typed entity/aggregate documents, 17 schemars-generated schemas
  with a real read-only drift test (gate 7), canonical JSON writer, flat
  package resolver/lock APIs, loader/index, tick-wait event IR, and the
  ten-file `one_area_one_creature` fixture that loads and reserializes
  byte-identically.
- T011a positioned validation in `crpg-data` (merged `2f801b5`): collected
  `validate`/`validate_files` over the `Diagnostic` model, shared path
  classifier, exact 15-diagnostic `broken_references` snapshot plus the
  data-owned `expected.json` gate manifest; followed by a test-only hardening
  (`ab7d208`) pinning duplicate slugs for every slug-bearing kind and
  embedded-graph reachability.
- T011b `crpgc validate` in `crpg-cli` (merged `d45f1b1`): thin read-only
  traversal over T011a's API with plain/canonical-JSON rendering and 0/1/2
  exits, plus the authorized replay first-position-flag usage fix; gate 8 is
  live as an ordinary manifest-driven black-box test. Native Windows/MSVC
  gates green (data 46, CLI 75, full workspace, 65 lint self-tests, deny
  clean); Linux results recorded in the task files.
- T012a migration framework data half in `crpg-data` (committed `3a62487`,
  with the T012/T012b Stage-2 specs): single-registry per-type version
  chains with clone-then-publish dispatch, Item-only `crpg.item/1 → /2`
  dummy edge through the loader, `migration_v1` golden campaign plus
  `migrations.json` manifest, and the non-vacuous `migration_coverage`
  gate. 17 schemas unchanged in count (one-line item tag bump); `Cargo.lock`,
  workflows, deny policy and every other crate untouched. Native
  Windows/MSVC gates green (data 69, full workspace 279, 65 lint
   self-tests, deny clean); Linux results live in `tasks/T012.md`.
   `crpgc migrate` itself is still absent — that is T012b in `crpg-cli`.
- T012b thin `crpgc migrate` wrapper in `crpg-cli` (merged `382cd4c`, PR #5,
  2026-09-18): explicit-save command over the T012a loader/writer with
  preflight-then-lexical-rewrite, bounded partial-write contract, and 0/1/2
  exits; 23 portable black-box tests (27 on Linux) plus 26 new unit seams;
  dual-native green (Windows 68 bin + 23 migrate + 20 validate + 13 replay;
  Linux 68 + 27 + 25 + 13) with gates 7/8/9 and migration coverage live.
  Supersedes T012a's "`crpgc migrate` still absent" note above.
- S001 public-repo hardening (merged `f704111`, PR #2, 2026-09-17): fork-PR
  guard over `tools/lint/` and `.github/workflows/` plus the trusted-lint
  gate in CI, the `build.rs`/external `[build-dependencies]` ban in
  `deps.py` (70 lint self-tests), the pinned `gitleaks` secret scan, the
  enforced self-hosted-runner rule, and `yanked = "deny"`. All 9 PR checks
  green; the `master-protection.` ruleset is active with all nine checks
  required, PR-only merging, and no bypass (human-done, API-verified). The
   follow-up revert PR #3 was closed unmerged. Deferred input caps live with
   their owners: T010 loader caps, T018 postcard caps, persist zstd bomb cap.
- T013a data-owned `explain_object` introspection prerequisite in `crpg-data`
  (merged `e84b13e`, PR #7, 2026-09-18): canonical report bytes, single
  shared reference inventory for loader/validation/introspection,
  subtree/ordering rules; 11 introspection tests plus a review-added
  stale-index pin; dual-native green with all 9 PR checks passing.
  Unblocks the CLI-only T013.
- T013 scaffolding/introspection CLI in `crpg-cli` (merged `9a6304f`,
  PR #9, 2026-09-26): six thin commands (`new`/`schema`/`explain`/`fmt`/
  `lock`/`run`) over the landed data/harness APIs with hand-rolled
  per-command parsers and 0/1/2 exits; five binary suites plus literal
  schema-only LLM acceptance (four types, R1 + R2 rounds with provenance
  record); dual-native green (Windows 231 / Linux 243 `crpg-cli` tests)
  with all 9 PR checks passing, plus a line-ending follow-up pinning the
  trial inputs to LF via a local `.gitattributes` after PR Windows CI
  caught an autocrlf conversion. Unblocks the rules-kernel T014.
- T014 stat/modifier kernel in `crpg-rules` (merged `18145d4`,
  PR #11, 2026-09-26): typed stats (Int/Fixed/Bool/Enum/Tags),
  data-selected stacking policies, always-present breakdowns, derived-stat
  validation with canonical cycle paths, saturating/floor arithmetic,
  symbolic string persistence, and five kernel lifecycle hooks; a 122-row
  modifier table, three 1024-case properties plus a deterministic reversal
  companion, boundary tests for every limit, and cross-namespace round
  trips; dual-native green with all 9 PR checks passing, plus a review
  follow-up closing the enum/tag-literal/capacity boundary and
  reversal-occurrence gaps test-only. Unblocks T015 dice and resolution.
- T015 dice, outcome tables, and resolution in `crpg-rules` (merged
  `76a48da`, PR #13, 2026-09-26): dice-valued stats and `DiceExpr`,
  roll/DC modifier targets, and `BeforeRoll`/`AfterRoll`/`BeforeDamage`/
  `AfterDamage` hooks. Unblocks T016 headless combat.
- T016 headless combat with `rulesets/minimal-d6` (merged `4686a73`,
  PR #14, 2026-09-27): authored combat vocabulary, data-to-runtime
  adapter with authoritative action/turn/damage operations, combat
  invariant repair and save/load coherence, encounter release with
  retained summary, cross-layer replay with independent native goldens,
  and the thin `crpgc replay --campaign` wrapper; ADRs 0013/0014.
  Dual-native green with all 9 PR checks passing. Unblocks T017.
- T017 second-ruleset abstraction proof with `rulesets/srd-lite`
  (merged `4686a73`, PR #14, 2026-09-27): versioned multi-pool/cost/
  defense/turn/effect vocabulary with migration edges, generalized
  sim controller (per-ability definitions, plural pools, target-stat
  defense, natural selection, lifetime-bearing effects, explicit
  `EndTurn`), cross-layer `combat_srd` replay with independent native
   goldens, and the CLI `end`-op proof; ADRs 0015/0016. Dual-native
   green with all 9 PR checks passing. Unblocks T018 `crpg-net`.
- T018 lane-0 combat protocol + simulated transport + conformance in
  `crpg-net` (merged `bc54896`, PR #16, 2026-09-28): versioned lane-0
  combat wire with bounded postcard codec (T018a), deterministic in-memory
  simulated transport with fault schedules (T018b), and the conformance
  gate — codec round-trip, malicious-client rejection without mutation,
  filtered replica convergence, and the 5,000-tick loss/jitter exercise
  (T018c); 28 + 25 + 22 net tests per native target. Dual-native green
   with all 9 PR checks passing. Unblocks the post-T018 queue
   (`tasks/POST-T018.md`).
- T019 `crpg-sim` same-pool affordability repair (merged `f3d1560`,
  PR #17, 2026-09-29): per-pool summed costs in authored template order
  with checked `u32` addition (`ValueOverflow`) and `InsufficientAction`
  carrying the total and original balance; 11-test public-API regression
  with byte/hash rollback evidence; existing replay fixtures and goldens
  unchanged per native target. Dual-native green with all 9 PR checks
  passing. Unblocks T020 history (specification review).
- T020 `crpg-sim` opt-in authoritative history and T021 `crpg-net`
  explicit v2 event protocol (merged `a991fe9`, PR #18, 2026-09-30,
  implementations `ea681c6`/`e25d0fd`, with the post-T018 specification
  frontier `81b73a2`): `HistoryWorld` with transactional typed mutations,
  bounded read/ack journal and full-wrapper `history_hash`, per-target
  `history_v1` goldens (ADR-0017); `protocol_v2`/`codec_v2`/`projection_v2`
  beside frozen v1 (ADR-0019). T020's completion record reports dual-native
  green and leaves independent review of its golden artifacts open;
  `tasks/T021.md` has no completion record. ADR-0017/0019 still read
  "Selected".
- T001 GDExtension rendering spike — go (ADR-0003), 200 chars @ 231.7 fps,
  FFI cost 87.4 µs/frame, on the RTX 4060 laptop. Spike lives in
  `C:\CRPG\Dev\spike-gdext`, not this workspace.
- T003 Lua sandbox spike — go (ADR-0005). All 10 escape-attempt fixtures
  blocked, instruction budget aborts an infinite loop, memory ceiling
  blocks unbounded allocation, `pairs`/`math.random` substitutions are
  reproducible under a seed and change with a different one. Found that
  `mlua`'s `StdLib` bitset does not gate the base library — `load`,
  `loadfile`, `dofile` are loaded regardless and must be stripped from
  globals by hand; recorded so `crpg-script` doesn't rediscover it the
  hard way. Spike lives in `C:\CRPG\Dev\spike-lua-sandbox`, not this
  workspace.
- T002/T002b QUIC movement spike — go (ADR-0004, updated 2026-09-04).
  Prediction/reconciliation validated (small, non-compounding corrections
  even under ~30% effective loss). NAT leg closed: a human-run two-machine
  test (server behind a normal home router, no port forward/DMZ; client on
  a separate network) failed to connect, as spec §7.7 anticipated. Local
  Windows Firewall and router UPnP capability were both confirmed present
  and ruled out as the cause; the failure is the NAT layer having no port
  mapping, which is exactly what §7.7 already deferred handling for.
  Known, scoped future fix: give the real server a UPnP/IGD client (`igd`
  crate) or document manual port-forwarding for operators. Still flagged
  as a risk, unresolved: a real quinn 0.11.11 defect
  (quinn-rs/quinn#2710: 129-packet dedup window silently discards
  reordered-but-delivered datagrams) that inflates effective loss well
  past the shim's configured rate under realistic jitter — needs a
  decision before crpg-net's snapshot channel depends on raw datagrams.
  Spike lives in `C:\CRPG\Dev\spike-quic`, not this workspace.

## Next
- T022 `crpg-server` host capture/checkpoint: specification inputs missing
  (authentication/dependency evidence, admission/checkpoint limits,
  lifecycle API) per `tasks/SPECIFICATION-READINESS.md`.
- Specified and implementable now: T028 legality (`crpg-sim`), then T027a
  area identity (`crpg-sim`, serialized after T028), T029a IR declarations
  (`crpg-data`), T030 quinn dossier, T031 preflight runners, T032
  documentation reconciliation.
- *Update 2026-09-30:* all six are implemented on branch
  `claude/blissful-rubin-59repd` (unmerged) with Linux/GNU gates green; each
  still needs its native Windows/MSVC run and review/merge.
- *Update 2026-09-30 (later):* PR #20 merged all six (`c1bc6ab`) with CI green
  on Windows/MSVC and Linux/GNU; T031's `tools/tests` suite has not yet run on
  native Windows. Next: T022 host and T029b bindings, each waiting on its
  dependency approval; ADR-0017–0022 acceptance, the T023 `windows-sys` policy
  call and the remaining E-task wording are open decisions for the maintainer.
- *Update 2026-09-30 (decisions):* the user approved D19–D26
  (`tasks/DECISIONS-2026-09-30.md`): ADR-0017/0018/0019/0020/0022 accepted,
  ADR-0021 superseded, T022 and T029b dependency edges approved (both now
  **open and ready**), `windows-sys` resolved by ADR-0023 (applied by T023),
  T023 scoped to lane-0 reliable streams without vendoring the dedup patch,
  movement scheduled after T024, remaining E-task wording closed by T033
  (E023 deferred), and preflight tests added to CI. Next: T022, then T029b. T030 found the
  ADR-0004 symptom not reproduced end-to-end and keeps T023 blocked on native
  Windows evidence, a `cargo deny` bans failure (`windows-sys` split) and
  pending dependency approval. Details: `tasks/BACKLOG.md` rows T027a–T032.
- *Update 2026-10-01:* PR #21 merged (`ac137ac`) with all nine checks green:
  decisions D19–D26, ADR-0023, T033, and the D26 preflight CI step, whose
  `lint-selftest (windows-latest)` run is T031's native Windows evidence. The
  T030 blockers named in the previous update are resolved by D22/D23; T023 now
  waits only on its own contract and T022. Next: T022 and T029b (both ready),
  per the delegation plan in `docs/IMPLEMENTATION_PLAN.md`.
- *Update 2026-10-06:* PRs #23–#28 merged on `master` through `16f58c8`:
  T022/T029b/T038, T058a/T058, T023s/T023, T039/T023c/T023v, and T023d/T023b
  (ADR-0026 host adapter over the `crpg-net-quic` path edge; `host_quic`
  25/25 ×5 and workspace 1,164 passed on Linux/GNU per its completion
  record; the PR #28 Windows/MSVC run at `5431bfa` was green, `quic` 30/30
  and `host_quic` 25/25). Next: T024 snapshot transfer (contract drafted on
  `claude/blissful-rubin-59repd`, awaiting approval), then T025a/b reconnect,
  T026p movement acks and T027b/c interest per the plan's Wave 4–6 order.

## Platform decision and verification
- [ADR-0012](adr/0012-windows-primary-platform.md) is Accepted, recording the
  maintainer's 2026-09-06 decision. `x86_64-pc-windows-msvc` is the primary
  development, product, release-gating, and behavioural-baseline target for
  client, editor, embedded single-player server, dedicated server, and CLI.
- `x86_64-unknown-linux-gnu` is fully supported for dedicated server, headless
  CLI/tooling, server-side extensibility, and CI/testing. A target-specific
  failure is a defect. Linux GUI client/editor builds are not promised;
  Linux headless support must not depend on Godot.
- One platform-neutral authoritative server implementation is hosted
  in-process for Windows single-player behind an in-memory transport and by
  dedicated processes on Windows and Linux. Clients never mutate authority
  directly. OS-specific concerns remain above core/rules/sim.
- Each target must reproduce its independently generated golden under pinned
  Rust 1.98.0, normal test profile, and default features. Windows owns the
  primary behavioural baseline, Linux the supported server regression
  baseline; neither is compared to the other. Exact-build determinism, not
  cross-platform lockstep, remains the promise.
- All required T009c gates passed on native Windows/MSVC and genuine
  Linux/GNU in WSL Ubuntu 24.04. Each native run passed 19 testkit tests
  (6 harness, 12 portable replay, 1 native golden), 135 workspace tests total
  (134 unit/integration + 1 doctest), and 65 lint self-tests. See the
  [T009c completion record](../tasks/T009c.md) for native commands and
  provenance. This is post-change verification, separate from the retained
  T009a Linux history above. The independent Linux re-verification run
  confirmed provenance by regenerating both build and goldens on genuine
  WSL Ubuntu 24.04 with zero tolerance.
- Final audit fixed code/documentation issues and reran the Windows/Linux
  gates with the totals above. The release-profile portable fallback test
  passed on each native target (1 test each). Byte audit confirmed LF and no
  BOM on replay/golden artifacts; a temporary `GIT_INDEX_FILE` audit
  confirmed `i/lf w/lf` without changing the user index. The scoped source
  diff was empty. See the
   [completion record](../tasks/T009c.md) for the pre-merge audit results;
   T009a and T009c are both merged on `master` as of 2026-09-07; T009b's
    `crpgc replay` wrapper is merged too (2026-09-07). T010/T011a/T011b,
    T012a's data-half migration framework, T012b's `crpgc migrate` wrapper
    (PR #5), S001 hardening (PRs #2/#4/#6), T013a data-owned
    introspection (PR #7), and T013 scaffolding/introspection CLI (PR #9)
    are merged on `master` through `9a6304f`; T014's stat/modifier kernel
    (PR #11) is merged on `master` through `18145d4`; T015's dice,
    outcome tables, and resolution (PR #13) is merged on `master` through
    `76a48da`; T016's headless combat and T017's second-ruleset abstraction
     proof (PR #14) are merged on `master` through `4686a73`; T018's
     lane-0 combat protocol, simulated transport and conformance (PR #16)
     are merged on `master` through `bc54896`.

## Future platform obligations
- [E012](../tasks/E012-binary-crate-naming.md) and
  [E022](../tasks/E022-server-editor-api-shapes.md) track assigning one reusable
  authoritative host to Windows embedded,
  Windows dedicated, and Linux dedicated adapters. They own whether it is a
  library target in `crpg-server` or code in another existing crate; no
  package/API choice is made here.
- The open, planning-only
  [E023](../tasks/E023-native-extension-loading-and-packaging.md) tracks T0 native
  extension loading/packaging on Windows/MSVC and Linux/GNU, reconciling
  target artifacts and ABI/loading with only `crpg-godot` permitting `unsafe`.
  No loader, dependency, unsafe exception, or stable ABI is authorized.
  T1 campaign data and sandboxed Lua/ruleset content remain portable.
- Future product CI must require Windows client/editor/server and Linux
  headless server artifacts, plus real Windows embedded-server and
  dedicated-server smoke tests and a Linux dedicated-server smoke test when
  those capabilities exist. E020 forbids unavailable placeholder jobs.
- E012/E022/E023 track these future host, extension, and capability-gated
  product obligations; no server hosting, extension loading, packaging, or
  product CI implementation is claimed by T009c verification.

## Task backlog
`tasks/BACKLOG.md` is the index of every numbered task with its status, plus
the carried blockers and the throughput log.

## Decisions
- ADR-0001 Godot consumed as a pinned dependency, not forked
- ADR-0002 Rust below the presentation layer
- ADR-0003 T1 spike go/no-go: go
- ADR-0004 T2 spike go/no-go: go — local reconciliation validated, NAT
  leg closed (fails without manual port forward/UPnP request, as
  expected per §7.7; known future fix is a UPnP/IGD client or documented
  manual forwarding), quinn dedup-window defect still flagged as a risk
  to resolve before crpg-net depends on raw QUIC datagrams
- ADR-0005 T3 spike go/no-go: go — mlua sandbox holds against all 10
  scripted escape attempts; base-library `load`/`loadfile`/`dofile` are
  not gated by `StdLib` and must be stripped explicitly, carry that
  forward into crpg-script
- ADR-0006 crpg-core primitive semantics — **Accepted** on 2026-09-04:
  generational arena in core, `Fx16_16` saturating/floor, PCG32 sub-streams in
  a `BTreeMap`, interned ids runtime-only (persist the string). Authorises
  `proptest` + `serde_json` as workspace dev-dependencies.
- ADR-0007 reserves `u32::MAX` as the arena's never-issued retirement
  tombstone, superseding ADR-0006 Decision 1's original overflow boundary.
- E006-A (2026-09-06): `f64` banned in `crpg-sim`; the determinism lint
  enforces it as `no-f64`, `f32`-spatial-only per spec §2.4.
- E009/E014/E015 (2026-09-06): spec carries the ADR-0008 residue fixes, the
  skeleton-only `World` serde caveat (conversion pair owned by T014), and the
  replica/`Timeline` ownership (prediction buffer outside sim, container in
  T007, advance rules in T008). T007 scope is locked.
- ADR-0009 (2026-09-06): determinism scope is replay-over-exact-build, not
  lockstep; goldens filename-scoped; hash exclusion list governed, starting
  empty. Only Decision 3's canonical-Linux-only selection is superseded by
  ADR-0012's independent Windows/MSVC and Linux/GNU comparisons.
- ADR-0010 (2026-09-06): testkit `Mismatch` is an enum carrying only present
  sides (no zeroed-hash sentinels); non-UTF-8 goldens are content
  divergence, not I/O failures.
- ADR-0011 (2026-09-06): supersedes ADR-0008 Decision 1's wording — the
  core-closed event payload bound applies to payload fields, not the
  vocabulary enum; `World.events: EventQueue<SimEvent>` affirmed as the
  sanctioned instance. Docs aligned, no code changes.
- E005 (2026-09-06): testkit is a one-way integration consumer. Core, data,
  rules, and sim do not depend on it even for tests; their cross-layer tests
  live in testkit. Higher crates may dev-depend only when all transitive edges
  remain legal and testkit does not depend back.
- E020 (2026-09-06): integration gates 7–13 activate only when their
  capabilities exist, never as skipped green placeholders. T009a's historical
  Linux comparison passed on genuine Rust 1.98.0 Linux, 2026-09-06. ADR-0012
now requires real target-scoped comparisons in both existing Windows and
  Linux workspace-test jobs; T009a and T009c are merged as of 2026-09-07.
  See the [completion record](../tasks/T009c.md).
- E016 (2026-09-06): campaign JSON distinguishes entity and named aggregate
  documents; object references use ULIDs while dependencies use immutable
  package ids; assets/campaign/package manifests have separate hash authority;
  event-IR waits count ticks. T010 owns data behavior, T013 the thin lock CLI.
- Godot pinned at 4.7.2
- Toolchain pinned at rustc 1.98.0

## Open questions
- Whether to carry a patch for quinn-proto's dedup window (quinn#2710) or
  wait/track upstream, once crpg-net design starts (see ADR-0004). Not on the
  critical path until after T018 — the in-memory transport comes first.

## Known problems
- Scaffolding from workflow plan §15 still missing, none of it blocking:
  `tools/preflight.ps1`, `docs/adr/0000-template.md`, per-crate `AGENTS.md`
  for every crate except `crpg-core` (T006a), `crpg-sim` (T007),
  `crpg-testkit` (T008b), `crpg-cli` (T009b) and `crpg-data`
  (T010, extended through T012a).
- Spec §14's `docs/contracts/` and `docs/guides/` still do not exist. Neither
  has a gate depending on it: contracts matter once `crpg-contracts` holds
  traits, guides once there is a campaign format to author against.
   (`docs/architecture/` was the one with a gate — §15.6 — and now exists.)
- S001 public-repo attack surface (merged `f704111`, PR #2, 2026-09-17): the
  security review's four gaps — defangable lints, no `build.rs` ban, no
  secret scan, and unenforced self-hosted-runner restriction — plus
  `yanked = "warn"` are closed by S001's guard/trusted-lint jobs,
  `build.rs`/external `[build-dependencies]` ban, `gitleaks` scan, enforced
  runner rule, and `yanked = "deny"`. Human-done: the `master-protection.`
  ruleset is active on `master` with all nine CI checks required, PR-only
  merging, and no bypass actors (API-verified); the follow-up revert PR #3
  was closed unmerged. Path-scoped review stays procedural for a solo
  maintainer; enforcing it by path needs CODEOWNERS, which S001 keeps out
  of scope.

---

## Merged-task log (moved from `AGENTS.md`, 2026-10-16)

Kept verbatim from the root `AGENTS.md` Note (D28f), with link paths adjusted;
newer status is in the handover above and in `tasks/BACKLOG.md`.

- T009a (typed replay) and T009c (Windows-primary/Linux-supported native
  golden policy, ADR-0012) are merged on `master` as of 2026-09-07; native
  Windows/MSVC and genuine Linux/GNU in WSL Ubuntu 24.04 gates passed and the
  final audit is complete. T009b's thin `crpgc replay` wrapper (`crpg-cli`)
  is merged on `master` as of 2026-09-07 with both native gates green; T010
  campaign data format, T011a positioned validation, T011b's `crpgc validate`
  wrapper, and T012a migration framework (data half) are merged on `master`;
  T012b's thin `crpgc migrate` wrapper (`crpg-cli`) is merged on `master`
  (PR #5) with both native gates green; S001 hardening is merged with branch
  protection human-done (PRs #2/#4/#6). T013a data-owned introspection
  prerequisite (`crpg-data`) is merged on `master` (PR #7, `e84b13e`) with
  both native gates green, reviewed with a stale-index test fix; T013
  scaffolding/introspection CLI (`crpg-cli`: `new`/`schema`/`explain`/`fmt`/
  `lock`/`run`) is merged on `master` (PR #9, `9a6304f`) with both native
   gates green, plus a line-ending follow-up pinning the trial inputs to LF;
   T014 stat/modifier kernel (`crpg-rules`: typed stats, stacking pipeline,
   derived validation, symbolic persistence, kernel hooks) is merged on
   `master` (PR #11, `18145d4`) with both native gates green, plus a
   test-only review follow-up closing boundary/reversal gaps; T015
   dice, outcome tables, and resolution (`crpg-rules`) is merged on
   `master` (PR #13, `76a48da`) with both native gates green; T016
   headless combat (`rulesets/minimal-d6`, sim adapter/controller,
   lifecycle/release, replay goldens, CLI proof) and T017 second-ruleset
   abstraction proof (`rulesets/srd-lite`, multi-pool/effect/defense/turn
   generalization, replay goldens, CLI proof) are merged on `master`
   (PR #14, `4686a73`) with both native gates green; T018 `crpg-net`
   lane-0 combat protocol + simulated transport + conformance is merged on
   `master` (PR #16, implementation `2fc54b7`, merge `bc54896`,
   2026-09-28) with both native gates green; T019 `crpg-sim` same-pool
   affordability repair is merged on `master` (PR #17, implementation
   `56931eb`, merge `f3d1560`, 2026-09-29) with both native gates green;
   T020 C1-sim opt-in history (`crpg-sim`, ADR-0017) and T021 v2 event
   protocol (`crpg-net`, ADR-0019) are merged on `master` (PR #18, merge
   `a991fe9`, 2026-09-30); T027a, T028, T029a, T030, T031 and T032 are
   merged on `master` (PR #20, merge `c1bc6ab`, 2026-09-30) with CI green on
   both OSes; decisions D19–D26, ADR-0023 and T033 are merged on `master`
   (PR #21, merge `ac137ac`, 2026-10-01) with CI green on both OSes; the
   delegated plan to completion is `docs/IMPLEMENTATION_PLAN.md`; see
   `tasks/BACKLOG.md` for open review items and next tasks.
   See the [completion record](../tasks/T009c.md),
   [T009b](../tasks/T009b.md), [T010](../tasks/T010.md), [T011a](../tasks/T011a.md),
   [T011b](../tasks/T011b.md), [T012](../tasks/T012.md), [T012b](../tasks/T012b.md),
   [T013a](../tasks/T013a.md), [T013](../tasks/T013.md), [T014](../tasks/T014.md),
   [T015](../tasks/T015.md), [T016](../tasks/T016.md), [T017](../tasks/T017.md),
   [T018](../tasks/T018.md) and [POST-T018](../tasks/POST-T018.md).

## Agent log

- 2026-09-06 (UTC) · opencode/muse-spark + hygiene + T007-unblock · Folded T006e into Done (it landed in 8f2e38b; "complete in working tree" was stale) and recorded E006-A/E009/E014/E015 so T007 can be specified.
- 2026-09-06 (UTC) · opencode/muse-spark + T007 merged · Recorded the skeleton above; next is T008 with the open E004 split question flagged.
- 2026-09-06 (UTC) · opencode/muse-spark + E004 decided (Option A) · T008 is now T008a (sim, specify first) + T008b (testkit); E011 acceptance criterion sits with T008a.
- 2026-09-06 (UTC) · opencode/muse-spark + E011 filed · ADR-0009 accepted; T008a's scope input is now ratified, leaving only the `blake3` choice for its task file.
- 2026-09-06 (UTC) · opencode/muse-spark + T008a merged · Recorded the loop and instrument above; next is T008b (specify first), then T009 with E005/E020.
- 2026-09-06 (UTC) · opencode/muse-spark + T008b merged · Recorded the harness above; next is T009 (specify after E005 + E020), then T010 (needs E016).
- 2026-09-06 (UTC) · opencode/muse-spark + review 3 follow-up · Recorded the sim/core/testkit hardening, ADR-0010/0011, and the AGENTS.md correction above; next is still T009 (blocked: E005 + E020).
- 2026-09-06 (UTC) · opencode/gpt-5.6-sol + E005/E016/E020 decisions · Recorded the testkit boundary, capability-gated CI sequence, and campaign-envelope policy; T009a is now the unblocked next task.
- 2026-09-06 (UTC) · opencode/muse-spark + T009a · Recorded the replay implementation (format, opaque payloads, typed divergence), the 19-test suite green on Windows and genuine Rust 1.98.0 Linux, and CI step 9 going live; next is T009b, then T010.
- 2026-09-06 (UTC) · opencode/gpt-5.6-sol + T009c platform correction plan · Made the Windows-primary/Linux-supported correction the next priority ahead of T009b; T009a remains unmerged until both target-scoped replay baselines and the superseding ADR are in place.
- 2026-09-07 (UTC) · opencode/gpt-6-astra + T009c documentation alignment · Recorded the accepted platform policy and future host, extension, and product-gate obligations while retaining T009a's genuine Linux verification history. T009a remains uncommitted and T009c in progress pending Windows and WSL Ubuntu verification; no completion or merge is claimed.
- 2026-09-07 (UTC) · opencode/gpt-6-astra + T009c verification status and audit · Recorded the reported passing native gates with 19 testkit tests, 144 workspace tests including one doctest, and 65 lint self-tests per environment, preserving T009a's historical provenance. Linked the completion record and tracked E012/E022/E023 obligations; T009c awaits review/merge with final audit running and T009a also uncommitted.
- 2026-09-07 (UTC) · opencode/gpt-6-astra + T009c count correction · Corrected the active workspace total from the revised native report to 135 (134 unit/integration + 1 doctest): core 91, sim 24, testkit 19, and core doctest 1. The earlier attribution's 144 is superseded, not rewritten; final audit remains running and review/merge is outstanding.
- 2026-09-07 (UTC) · opencode/gpt-6-astra + T009c final audit · Recorded the reported completed final audit, passing native reruns and release fallback tests, LF/no-BOM byte and temporary-index checks, and empty scoped source diff. Implementation/verification is complete in the working tree awaiting review/merge; T009a remains uncommitted and T009c retains priority before T009b.
- 2026-09-07 (UTC) · opencode/big-pickle + T009a/T009c merged · Recorded the merge of T009a's typed replay and T009c's Windows-primary/Linux-supported native golden policy onto `master` (commit `bb9a702`, pushed to `origin`). Next is T009b's thin `crpgc replay` wrapper, then T010.
- 2026-09-07 (UTC) · opencode/big-pickle + T009b merged · Recorded the `crpgc replay` wrapper (thin consumer of `play_and_verify`, exit codes 0/1/2, provisional reference apply, crate-opened docs) landed on `master` with both native gates green; next is T010 campaign data in `crpg-data`.
- 2026-09-17 (UTC) · opencode/muse-spark + T010/T011a/T011b merged · Recorded the pushed campaign format, positioned validation with its slug/graph test hardening, and thin `crpgc validate` wrapper with live gate 8; next is T012 migration framework.
- 2026-09-18 (UTC) · opencode/muse-spark + T012a merged/doc-status catch-up · Recorded the committed data-half migration framework (registry, Item dummy edge, `migration_v1` golden, coverage gate) with T012b next; branch state moved past the stale `d45f1b1` pin and Known problems now lists all five live per-crate `AGENTS.md` files.
- 2026-09-17 (UTC) · opencode/muse-spark + S001 known-problems · Noted S001's open attack-surface closure and the remaining human branch-protection step, so the guard jobs are not mistaken for the boundary itself.
- 2026-09-18 (UTC) · opencode/muse-spark + S001 merged · Recorded the PR #2 merge (`f704111`, all 9 checks green), the API-verified active ruleset, and the revert PR #3 closed unmerged; Done history, branch state, and Known problems moved to the merged state with T012b next.
- 2026-09-18 (UTC) · opencode/muse-spark + T012b merged/T013a on-branch catch-up · Recorded the T012b `crpgc migrate` merge (`382cd4c`, PR #5) and the Task/s001 merge (`289e7db`, PR #6) in branch state, Done history, Phase and Next; T013a noted as implemented and dual-native verified on `docs/S001-merged-bookkeeping`, awaiting landing.
- 2026-09-18 (UTC) · opencode/muse-spark + T013a landed · Recorded the T013a data-owned introspection merge (`e84b13e`, PR #7, all 9 checks green) in branch state, Done history, Phase, Next, and verification history; T013 CLI is next.
- 2026-09-26 (UTC) · opencode/muse-spark + T013 landed · Recorded the T013 scaffolding/introspection CLI merge (`9a6304f`, PR #9, all 9 checks green after a trial-input line-ending fix) in branch state, Done history, Phase, Next, and verification history; T014 rules kernel is next.
- 2026-09-26 (UTC) · opencode/muse-spark + T014 landed · Recorded the T014 stat/modifier kernel merge (`18145d4`, PR #11, all 9 checks green after a test-only review follow-up) in branch state, Done history, Phase, Next, and verification history; T015 dice/resolution is next.
- 2026-09-27 (UTC) · opencode/muse-spark + A0 status reconciliation · Recorded the T015 dice/resolution merge (`76a48da`, PR #13, 2026-09-26) in branch state, Done history, Phase, Next, and verification history; T016a–e noted as implemented and natively verified in the working tree, unmerged, with the A2 lifecycle decision and milestone merge outstanding.
- 2026-09-27 (UTC) · opencode/muse-spark + T016/T017 landed · Recorded the T016 headless combat and T017 second-ruleset abstraction proof merge (`4686a73`, PR #14, all 9 checks green) in branch state, Done history, Phase, Next, and verification history; T018 `crpg-net` protocol is next.
- 2026-09-28 (UTC) · opencode/muse-spark + T018 landed · Recorded the T018 lane-0 protocol + simulated transport + conformance merge (`bc54896`, PR #16, all 9 checks green, verified against `origin/master`) in branch state, Done history, Phase, Next, and verification history; T019 is implemented and dual-native verified in the working tree with review/merge outstanding, T020 specification next.
- 2026-09-30 (UTC) · opencode/muse-spark + T019 landed · Recorded the T019 same-pool affordability repair merge (`f3d1560`, PR #17, all 9 checks green) in branch state, Done history, Phase, and Next; T020 specification review is next.
- 2026-09-30 (UTC) · claude-code + T020/T021 landed bookkeeping · Recorded the PR #18 merge of T020/T021 in Phase, Done and Next, replacing the stale "T020 spec-review next" pointer; the open golden review, missing T021 record and "Selected" ADR statuses are stated rather than resolved.
- 2026-09-30 (UTC) · claude-code + post-T018 batch on branch · Recorded that T027a, T028, T029a and T030–T032 are implemented on branch with Linux-only verification, keeping the Windows gates, review/merge and T030's T023 blockers explicit.
- 2026-09-30 (UTC) · claude-code + PR #20 landed · Recorded the merge of the six post-T018 tasks with dual-OS CI and listed the maintainer decisions that now gate T022, T029b and T023.
- 2026-09-30 (UTC) · claude-code + D19–D26 recorded · Summarized the user-approved decisions and the resulting next tasks (T022, then T029b).
- 2026-10-01 (UTC) · claude-code + PR #21 landed / implementation plan · Recorded the PR #21 merge, superseded the stale T023-blocker sentence by appending rather than rewriting it, and pointed at the new delegation plan.
- 2026-10-06 (UTC) · opencode/muse-spark + status-docs housekeeping · Moved Updated/Phase/branch-state past the stale `f3d1560` pin to the merged PR #23–#28 frontier (`16f58c8`) with per-record gate honesty; per-task detail stays in `tasks/BACKLOG.md`, Done history retained as history. No code, dependency, or gate change.
- 2026-10-16 (UTC) · claude-code + D28 handover · Added the handover block the adopted agent-team plan reads first, corrected the 2026-10-06 update's "Windows run pending" to the green PR #28 Windows run recorded on `claude/blissful-rubin-59repd`, and received the merged-task log moved out of `AGENTS.md` (D28f).
- 2026-10-16 (UTC) · claude-code + handover after S002 · Moved the handover past the S002 merge, added PR #31 (T096) and T097 for the manager, and cleared the deny blocker.
- 2026-10-16 (UTC) · claude-code + manager handover · Rewrote the handover for the next manager session, naming every run still going with its worktree, model and log, plus where the interim runner and the A0-6b brief live, because the first manager session's context passed the new 200k limit.
- 2026-10-16 (UTC) · claude-code + manager handover (second session) · Rewrote the handover for the next session: what PR #33 gained, the numbered next steps, the Go-route rule, and every run still going with its model and worktree.
- 2026-10-16 (UTC) · claude-code + final handover (second session) · Brought the handover to the session's end: wiring the context meter comes first, after Claude's usage report showed 88% of usage above 150k context; two runs are named as still going.
- 2026-10-16 (UTC) · claude-code + manager handover · Replaced the Handover block with the second manager session close-out (PR #33 in flight, the numbered next steps, and the Go-only blocker) so the next session starts from the current state, not the earlier handover.
- 2026-10-16 (UTC) · claude-code + user approvals · Recorded the user's approval of the T097 contract and D29d–f, making the T097 I package the next dispatch, and noted that the hook wiring must be done by hand because the classifier refuses it even with approval.
- 2026-10-16 (UTC) · claude-code + hooks wired · Marked next step 2 done after the user applied the four hooks to `.claude/settings.json` by hand, so the next session knows the context meter is live.
- 2026-10-16 (UTC) · claude-code (manager, Opus 5.5) + third manager session handover · Replaced the handover: six packages stopped by the Go cap with their slots, branches and rerun commands, and the user's new rules (cut Claude usage, keep slots full, design in parallel), so the next session can resume without this one.
- 2026-10-16 (UTC) · claude-code (manager, Opus 5.5) + user concern on review cost · Added Next item 5, the cheaper-review plan (cheap-first review, smaller review input, A0-7 catches-per-dollar fixtures), because Kimi K3 and Gemini 4 Argon runs exhaust the Go cap.
- 2026-10-16 (UTC) · claude-code (manager, Opus 5.5) + research wave · Recorded the R16–R18 research runs left going at handover, with their folders, logs and next steps, so the next session can collect them.
- 2026-10-16 (UTC) · claude-code (manager, Opus 5.5) + Beam on Go · Recorded the T108 drafts rerun on the uncapped Beam and LongCat Go models, so the next session can collect them and add Beam to MODELS.md.
- 2026-10-16 (UTC) · claude-code (manager, Opus 5.5) + phone restart · Added the oc.py environment and base rules for dispatching from the mgr-a0 worktree, because the user will restart this session with /clear from a phone.
- 2026-10-16 (UTC) · claude-code (manager, Opus 5.5) + session 4 handover · Rewrote the handover: nothing running, the execute-tool fix and its stale-slot gap, the audit and T108 results, and the 02:30 UTC reruns.
- 2026-10-16 (UTC) · claude-code (manager, Opus 5.5) + redirect hole · Added the read-only roles' shell-redirection deny to the handover security note.
- 2026-10-17 (UTC) · claude-code (manager, Opus 5.5) + session 5 handover · Recorded the OpenCode Go weekly limit and its reset (about 2026-10-18 23:15 UTC), the PR #33 merge that resolves the stale-slot execute gap, and how to read local OpenCode spend.
- 2026-10-17 (UTC) · claude-code (manager, Opus 5.5) + session 6 handover · Recorded the Beam implementation trial (eight PRs, T097 as a draft awaiting the user's review choice), the two packages still to finish, and the dispatch facts and tool defects found, so the next manager can continue without this session.
- 2026-10-17 (UTC) · claude-code (manager, Opus 5.5) + session 7 handover · Replaced the handover with the finished test-writer pass, the open PR queue #36–#55, the D29g–D29k questions and four A0-3d tool fixes, so the next session starts from the merge queue and the Go reset.
- 2026-10-17 (UTC) · claude-code (manager, Opus 5.5) + session 7 PR CI · Updated the handover's CI line with the three PR failures of case 13 and the re-runs, and raised T098 ahead of the next PR batch.
- 2026-10-17 (UTC) · claude-code (manager, Opus 5.5) + user request · Pointed the handover at the single session-7 batch PR that replaced #45–#56, and recorded the one-PR-per-wave rule after the user objected to the number of PRs.
- 2026-10-17 (UTC) · claude-code (manager, Opus 5.5) + user request · Folded session 6's PRs #36 and #38–#44 into batch PR #57 at the user's word, leaving two open PRs, and updated the handover to match.
- 2026-10-17 (UTC) · opencode/gpt-6-luna (grunt) + session 8 bookkeeping · Replaced the handover and update time with the supplied session 8 facts, including the closed audit and current PR queue.
- 2026-10-21 (UTC) · claude-code (manager, Opus 5.5) + session 9 handover · Rewrote the handover at the 187k context mark: A0-3c done, A0-7a and three T097 reviews still running with their logs named, and the T098 contention data saved for the next attempt.
- 2026-10-21 (UTC) · claude-code (manager, Opus 5.5) + session 10 handover · Rewrote the handover at the 150k context mark: #37 and #59 merged at the user's word with the advisory lockfile bump, T098/T098b/A0-3e and A0-7a attempts 1–3 on the session-10 batch, A0-7a attempt 4 and the tool fixes next.
- 2026-10-21 (UTC) · claude-code (manager, Opus 5.5) + user answers · Recorded the user's answers to D29g–D29k, A0-10 and T108 in the handover, with the fix tasks they create.
- 2026-10-21 (UTC) · opencode-go/muse-spark-1.3-contributor + S11 bookkeeping · Replaced the handover with the session 11 facts: PR #60 is now "Sessions 10–11" with T130/T131/T132/T133/T098c/A0-3f/A0-7a/T108 judged, and D30 is the next user gate.
- 2026-10-21 (UTC) · claude-code/opus-5.5 + session 11 PR number · Pointed the handover at PR #61 on `mgr/session-11-batch`, because PR #60 merged before the session's batch was pushed.
- 2026-10-21 (UTC) · claude-code/opus-5.5 + D30 answered · Recorded the user's D30 answers and ADR-0029's acceptance in the handover's first Next item.
- 2026-10-21 (UTC) · opencode-go/muse-spark-1.3-contributor + S12 bookkeeping · Replaced the handover with the session 12 facts (PR #61 green awaiting merge; S12-docs, T098d and T144 judged on `mgr/session-12-batch`; A0-8 trim correction), so the next session starts from the session 12 close-out.
- 2026-10-22 (UTC) · opencode-go/muse-spark-1.3-contributor + S13 bookkeeping · Replaced the handover with the session 13 facts (PRs #61 and #62 green awaiting merge; T144b, A0-7b, T136-facts, T136 and T135 contracts judged), so the next session starts from the D31 gate.
- 2026-10-22 (UTC) · claude-code + S13 handover · Added to Next item 4 that grunt runs left their files uncommitted twice this session, so A0-3g covers that as well as the false PASS.
- 2026-10-23 (UTC) · claude-code + D31 answered · Next item 1 now records the D31 answer and the manager's merges of #61–#63, so the next session dispatches T136a and T135 directly.
- 2026-10-23 (UTC) · opencode-go/muse-spark-1.3-contributor (grunt) + S14 bookkeeping · Replaced Updated and the Handover block with the session 14 facts (PR #64, the S-set review gate, tool defects and dispatch facts), so the next session starts from the session 14 close-out.
- 2026-10-23 (UTC) · opencode-go/muse-spark-1.3-contributor (grunt) + S15 bookkeeping · Replaced the Handover block with the session 15 facts (PR #65 stacked on PR #64, T136b/T138 contracts awaiting D32, tool gaps and dispatch facts), so the next session starts from the session 15 close-out.
- 2026-10-23 (UTC) · opencode-go/muse-spark-1.3-contributor (grunt) + S16 bookkeeping · Replaced the Handover block with the session 16 facts (PR #66 re-land on master, A0-11/A0-4c in the session 16 batch, D32 gate), so the next session starts from the session 16 close-out.
- 2026-10-23 (UTC) · claude-code (manager, Opus 5.5) + D32 answered · Recorded the D32 answers in the handover, so the next session dispatches T136b and T138 directly.
- 2026-10-24 (UTC) · claude-code (manager, Opus 5.5) + session 17 handover · Replaced the session 16 handover with the session 17 state at the 150k context limit, naming the PR #69 blocker and the unverified T138a WIP.
- 2026-10-24 (UTC) · opencode-go/muse-spark-1.3-contributor + S18-bookkeeping · Replaced the handover with the session 18 state (batch holds T136b+T136c, T137 approved D33, T138a+T138c with review fixes, and A0-12; PR #69 merged; batch PR waits on T138b/d/e), so the next session starts from the current close-out.
- 2026-10-24 (UTC) · claude-code (manager, Opus 5.5) + session 19 handover · Replaced the handover with the PR #70 state and the golden gate it waits on, so the next session starts from the open PR.
- 2026-10-24 (UTC) · opencode-go/muse-spark-1.3-contributor + S20-bookkeeping · Replaced the handover with the session 20 state (from `master` `e680e3c` after PR #70 merged; T137a running in slot 1) and its numbered next steps, so the next session starts from the batch close-out.
- 2026-10-24 (UTC) · opencode-go/beam-free + S20-bookkeeping-2 · Recorded D35 T024 approval, T137b/T137c merge and D36 surface_v1 gate, and the T137b-tests handover.
- 2026-10-24 (UTC) · claude-code (manager, Opus 5.5) + session 20 close · Updated the handover now that T137b-tests merged into the batch (Claude Sonnet finished Beam's file at the user's go-ahead), and added the stale-owns-line and verify-flags dispatch facts.
- 2026-10-25 (UTC) · opencode-go/beam-free + S21-bookkeeping · Recorded session 21 handover: T137-gaps and T024/T024-fix on mgr/session-21-batch, T024-tests WIP on task/T024-tests-k1; lessons L5/L6; PR #71 merged.
- 2026-10-25 (UTC) · opencode-go/beam-free + S22-bookkeeping · Recorded session 22 handover: T024 core review returned fix (six test gaps), T024-docs and T025a-facts merged into the batch, lesson L7, and the two-keys notes-3 rerun plan.
- 2026-10-25 (UTC) · opencode-go/beam-free + D37-record · Recorded decision batch D37 answered 2026-10-25 (`tasks/DECISIONS-D37.md`) and replaced the two carried decision-batch questions in handover item 4.
- 2026-10-25 (UTC) · opencode-go/beam-free + S22-bookkeeping-2 · Recorded T024 two-keys landing (46 tests, Sonnet 5.5 attempt 7) and handover before the batch PR.
- 2026-10-25 (UTC) · opencode-go/beam-free + S22-bookkeeping-3 · PR #72 merged; handover updated to in-flight nothing and next renumbered.
- 2026-10-25 (UTC) · opencode-go/beam-free + S23-bookkeeping · Session 23 records and handover.
- 2026-10-25 (UTC) · claude-code/opus-5.5 (manager) + D38 · Pointed the handover at open PR #73 and at D38, so the next manager routes the remaining A0-8 trims away from Beam.
- 2026-10-25 (UTC) · claude-code/opus-5.5 (manager) + A0-8 approval · Updated the handover: crpg-net trim approved, three crates held for a tidy pass.
- 2026-10-30 (UTC) · claude-code/opus-5.5 (manager) + session 25 handover · Replaced the session-23 handover with session 25's state: four contracts reviewed and fixed, D39/D40 answered, A0-13 to A0-16 merged into the batch, T025a1 blocked only on the zstd-sys advisory, and T024h1 and the T139 tags revision next.
- 2026-10-30 (UTC) · claude-code/opus-5.5 (manager) + session 26 handover · Updated the handover for the D40e zstd upgrade, the T025a1 merge, T024h1's committed WIP after the Go quota stop, and the T139 architect run still going in slot 6.
- 2026-10-30 (UTC) · claude-code/opus-5.5 (manager) + session 27 handover · Replaced the session-26 handover: T139 contract, T024h1, T139d and T139a1 merged into the batch, Go exhausted with no reset, T024q WIP parked, disk nearly full.
- 2026-11-01 (UTC) · opencode-go/muse-spark-1.3-contributor + S28-bookkeeping · Replaced the handover with the session 28 state (batch `mgr/session-28-batch` PR open, T147 measurement in slot 1, demo-first Godot slice next, S-set review of T139a2 blocked on the user) and extended the dispatch facts with the goldens-owns, WSL-from-PowerShell and review-timeout notes.
- 2026-11-01 (UTC) · opencode-go/muse-spark-1.3-contributor + S29-bookkeeping · Moved the handover to the session 29 batch (T027b contract, ADR-0033, T146b, D41) and extended the dispatch facts with the session 29 notes.
- 2026-11-02 (UTC) · claude-code/opus-5.5 (manager) + session 30 handover · Moved the handover to the session 30 state (D41 answered, T027b and T046 implemented and reviewed, T047a contract proposed) and added the session 30 dispatch facts on the tests-role verify error and on weak first-pass two-keys suites.
- 2026-11-02 (UTC) · claude-code/opus-5.5 (manager) + session 31 handover · Moved the handover to session 31 (T047a and the T027b follow-up merged into the batch, the T047 contract approved, T047-1 running) and added the host_quic flake and the T047 split to the dispatch facts.
- 2026-11-02 (UTC) · claude-code/opus-5.5 (manager) + session 31 final handover · Recorded the PR #77 merge, the T047-1 fix round still running, the grant-syntax trap and the weak self-written tests, so the next session resumes at T047-1's re-check.
- 2026-11-02 (UTC) · claude-code/opus-5.5 (manager) + session 31 close · Recorded T047-1 merged into the session 32 batch after its fix round, so the next session starts at T047-2.
- 2026-11-02 (UTC) · claude-code/opus-5.5 (manager) + session 32 handover · Rewrote the handover for the T047 batch (T047-2, T047b, T047d merged into the batch; T047c running at handover) and added the Godot exit-code and `.uid` facts.
- 2026-11-02 (UTC) · claude-code/opus-5.5 (manager) + session 32 final handover · Pointed the handover at PR #78 with T047c's CI evidence and S-set review still to do.
- 2026-11-02 (UTC) · claude-code/opus-5.5 (manager) + session 33 handover · Moved the handover to PR #78 green with T047c-fix1 and the T047c review done, leaving only the user's reviews before merge.
- 2026-11-02 (UTC) · opencode-go/muse-spark-1.3-contributor + S34-bookkeeping · Moved the handover past T098e and the T048 contract docs onto PR #78 and recorded the session 34 dispatch facts; nothing below the handover changed.
- 2026-11-03 (UTC) · claude-code/opus-5.5 (manager) + session 35 handover · Rewrote the handover for PR #79 and the held T048 sentinel fix.
- 2026-11-07 (UTC) · opencode-go/gpt-6-luna + S36 bookkeeping · Replaced the handover with the supplied session 36 state and appended its new dispatch facts.
- 2026-11-07 (UTC) · claude-code/opus-5.5 (manager) + session 36 · Recorded the user's passing by-hand Godot play for T048 and T051, and the fresh-checkout setup it needed.
- 2026-11-07 (UTC) · claude-code/opus-5.5 (manager) + session 36 · Recorded the upward camera/sun defect the user's play exposed, its fix, and the box-proxy follow-up.
- 2026-11-07 (UTC) · opencode-go/gpt-6-luna + S37 bookkeeping · Replaced the handover with the supplied session 37 status and recorded its dispatch facts.
- 2026-11-07 (UTC) · opencode-go/gpt-6-luna + S38-bookkeeping · Updated the handover and session facts to the session 38 batch state and its next packages.
- 2026-11-07 (UTC) · claude-code/haiku-5.5 (bookkeeper) + S39-bookkeeping · Replaced the handover with the session 39 state and added the session 39 facts.
- 2026-11-07 (UTC) · claude-code/opus-5.5 (manager) + S39-bookkeeping check · Restored the Dispatch facts bullet that the session 39 bookkeeping commit dropped by mistake.
- 2026-11-07 (UTC) · claude-code/haiku-5.5 (bookkeeper) + S40-bookkeeping · Replaced the handover with the session 40 batch state and next steps, and added the session 40 facts.

- 2026-11-07 (UTC) · claude-code/haiku-5.5 (bookkeeper) + S41-bookkeeping · Replaced the handover with the session 41 batch state and next steps, and added the session 41 facts including the session-only Sonnet two-keys ruling.
- 2026-11-08 (UTC) · claude-code/haiku-5.5 (bookkeeper) + S42-43-bookkeeping · Replaced the handover with the session 42/43 batch state and next steps, and added the session 42/43 facts.
- 2026-11-08 (UTC) · claude-code/haiku-5.5 (bookkeeper) + S44-bookkeeping · Replaced the handover with the session 44 batch state and next steps, and added the session 44 facts.
- 2026-11-09 (UTC) · opencode-go/deepseek-v4.1-flash + S46-bookkeeping · Moved the handover to the sessions 45–46 batch with T036c, T149a, T149b and T037 next, and added the session 45/46 facts.
- 2026-11-09 (UTC) · claude-code/opus-5.5 (manager) + session 47 handover · Moved the handover to the session 47 batch (T149a, T036c with re-baselined `movement_v1`, T140 facts) and recorded the session's dispatch facts.
- 2026-11-10 (UTC) · claude-code/fable-5.2 (manager) + session 48 handover · Rewrote the handover for the session 48 state (T036c and T149b reviewed with fix rounds, T140 contract drafted, reviewed and fixed, Go monthly limit, free tier out) and added the session 48 dispatch facts.
- 2026-11-10 (UTC) · claude-code/haiku-5.5 (bookkeeper) + S49-bookkeeping · Replaced the handover's in-flight, next and blocked bullets with the session 49 batch state and added the session 49 facts, leaving every earlier facts bullet as written.
- 2026-11-10 (UTC) · claude-code/opus-5.5 (manager) + S49 handover · Set the handover to the merged PR #89 state with T140a next.
- 2026-11-10 (UTC) · claude-code/opus-5.5 (manager) + user: Go reset date · Recorded the user's estimate that Go's monthly quota resets in about 17 days (about 2026-11-27).
- 2026-11-10 (UTC) · claude-code/haiku-5.5 (bookkeeper) + S50-bookkeeping · Set the handover to the session 50 state: T140a in review on the session 50 PR (number still to fill), T140b after the user's merge, and the session 50 facts line.
- 2026-11-10 (UTC) · claude-code/opus-5.5 (manager) + S50 handover · Filled in the session 50 PR number (#90) the bookkeeper left as a placeholder.
- 2026-11-10 (UTC) · claude-code/opus-5.5 (manager) + S51 handover · Set the handover to session 51's in-flight T140b review so a lost session can resume from it.
- 2026-11-10 (UTC) · claude-code/opus-5.5 (manager) + S51 handover · Moved the handover to T140c in review and T140d in tests.
- 2026-11-10 (UTC) · claude-code/haiku-5.5 (bookkeeper) + S51-bookkeeping · Set the handover to the session 51 state: PR #91 waits on the user for T140b and T140d with T140c riding along, T140e is the next package, and the session 51 facts line is added.
- 2026-11-10 (UTC) · claude-code/claude-haiku-5-5 (bookkeeper) + S52-bookkeeping · Set the handover to the session 52 state: PR #91 now holds T140e in review, T140r is the next contract, and T141 waits on T140d's merge.
- 2026-11-13 (UTC) · claude-code/claude-haiku-5-5 (bookkeeper) + S54-bookkeeping · Set the handover to the session 54 state: PR #91 still waits on the user, the session 53–54 PR holds T140r, T140r1 and T140r2 in review, and the next steps are the merges, T141's contract and the crpg-godot bridge.
- 2026-11-13 (UTC) · claude-code/claude-haiku-5-5 (bookkeeper) + S55-bookkeeping · Set the handover to the session 55 state: PR #91 and PR #92 merged, three contracts (T141, T143, T026p) and the T137b-tests continuation running on Claude workers, with the next steps and blocks.
- 2026-11-14 (UTC) · claude-code/claude-opus-5-5 (manager) + S55 handover · Set the handover to the session 55 stop point: three contracts approved, T141s and T143b in final review rounds with their slots and branches named, and the queue and filings that follow.
- 2026-11-14 (UTC) · claude-code/claude-opus-5-5 (manager) + S55 T143b landed · Marked T143b verified and landed on the batch in the handover, so the next session starts T143a instead of re-verifying.
- 2026-11-14 (UTC) · claude-code/claude-opus-5-5 (manager) + S55 T141s-tests fix · Recorded the committed T141s-tests fix round (`53e54bb`) in the handover, so the next session verifies and lands it without checking the slot.
- 2026-11-14 (UTC) · claude-code/claude-haiku-5-5 (bookkeeper) + S57-bookkeeping · Replaced the handover's in-flight, next and blocked lines with the session 57 batch state (PR #93 open, T141 and T143 complete, T026p1 next once T141s merges) and added the session 57 facts line, so the next session starts from the PR review.
- 2026-11-14 (UTC) · claude-code/claude-opus-5-5 (manager) + S56 handover · Set the handover to the session 56 stop point: T141s, T141a, T141b and all of T143 landed on the batch with mutant reviews closed, T141c briefed and next, and the PR text items and contract errata listed for the session 55 PR.
- 2026-11-15 (UTC) · claude-code/claude-haiku-5-5 (bookkeeper) + S58-bookkeeping · Replaced the handover with the session 58 batch (PR #94 open, waiting on the user's review) and the next steps after it, and added the session 58 facts line.
- 2026-11-15 (UTC) · claude-code/claude-haiku-5-5 (bookkeeper) + S59-bookkeeping · Replaced the handover's in-flight, next and blocked lines with the session 59 batch (T037 contract D60 approved, T037a next, T056 contract next), removed the owed ADR notes that the architect has now written, and added the session 59 facts.
- 2026-11-15 (UTC) · opencode-go/deepseek-v4.1-flash + S60-bookkeeping · Replaced the handover with the session 60 state (batch PR open with T037a, T037b next, the T056 contract review, the T150/T151 contracts), recorded Go Plus restoring Go as the main implementer tier, and added the session 60 facts.
- 2026-11-15 (UTC) · claude-code/opus-5.5 (manager) + S62-handover · Replaced the handover with session 62's in-flight runs, verified commits and next steps at the context limit, because session 61 had ended without one.
- 2026-11-15 (UTC) · claude-code/opus-5.5 (manager) + S62-facts · Added the session 62 facts line: a PASS can hide untracked worker files, so check the slot after every run.
- 2026-11-15 (UTC) · claude-code/opus-5.5 (manager) + S62-handover-T150 · Recorded T150's second fix round and that it now waits only on the user's answers.
- 2026-11-15 (UTC) · claude-code/opus-5.5 (manager) + S62-handover-T025b · Moved T025b draft A from in flight to verified.
- 2026-11-15 (UTC) · claude-code/opus-5.5 (manager) + S62-handover-T037c · Moved T037c from in flight to verified after its finishing run.
- 2026-11-15 (UTC) · claude-code/opus-5.5 (manager) + S62-handover-T153a · Moved T153 draft A from in flight to verified.
- 2026-11-15 (UTC) · claude-code/opus-5.5 (manager) + S62-handover-T056b · Recorded the T056b fix commit, its disk-full verify and the pending hand re-run.
- 2026-11-15 (UTC) · claude-code/opus-5.5 (manager) + S62-handover-T153b · Moved T153 draft B from in flight to verified.
- 2026-11-15 (UTC) · claude-code/opus-5.5 (manager) + S62-handover-final · Recorded the T056b fix re-verify pass; no run is in flight at handover.
- 2026-11-15 (UTC) · claude-code/opus-5.5 (manager) + D64 · Recorded the user's approval of the T150 contract and unblocked it.
- 2026-11-15 (UTC) · claude-code/opus-5.5 (manager) + S63-handover · Handed over at the 150k context limit with two Go runs in flight (T150b continuation, T037b first-pass review), the T151 and T153 contracts written, and the user's Go-first routing rule recorded.
- 2026-11-15 (UTC) · claude-code/opus-5.5 (manager) + S63-handover-T037b · Recorded that the Go first-pass review of T037b hung and is to be retried on DeepSeek V4.1 Flash.
- 2026-11-15 (UTC) · claude-code/opus-5.5 (manager) + S63-handover-T150b · Recorded T150b verified (hand re-run PASS) and waiting for its Claude mutant review; no run is in flight.
- 2026-11-16 (UTC) · claude-code/opus-5.5 (manager) + D65 · Pointed Next at D65: the run-loop contract and the T151 revision come before the T151 review.
- 2026-11-16 (UTC) · claude-code/opus-5.5 (manager) + D65 answers · Recorded in Next that D65c–h are answered.
- 2026-11-16 (UTC) · claude-code/opus-5.5 (manager) + session 64 handover · Recorded session 64 results and a Go-first Next line after the user asked why Claude had displaced Go.
- 2026-11-16 (UTC) · claude-code/opus-5.5 (manager) + session 65 handover · Recorded session 65 results (T150a and T037c ready, T150b round 2 and the T154 review running) and the Next line.
- 2026-11-16 (UTC) · claude-code/opus-5.5 (manager) + T154 Go review · Recorded the finished T154 first-pass review's four findings in the handover so the next manager can hand them to the architect.
- 2026-11-16 (UTC) · claude-code/opus-5.5 (manager) + session 66 handover · Recorded session 66 results (T150b complete, T154 rev1/rev2, T151/T153 revised, T037b mutant gaps, T037d WIP) and the runs still going, with free-first routing as the standing rule.
- 2026-11-17 (UTC) · claude-code/opus-5.5 (manager) + session 67 handover · Recorded session 67 results (D66 answered, three contracts approved, T037d fixed and verified, T037b fix round 2 with 16 mutants left, T154a blocked on the K17 peer bound) and the T042 architect still running, so the next manager starts from the K17 fix.
- 2026-11-17 (UTC) · claude-code/opus-5.5 (manager) + T042 contract finished · Replaced the handover's "T042 running" with its result and two user questions, so the next manager does not wait on a finished run.
- 2026-11-17 (UTC) · claude-code/opus-5.5 (manager) + session 68 handover · Recorded session 68 results (T037b mutant round closed, T154 rev3/rev4 on the batch, T154a built with its two-keys suite, T042 rev1, D67 drafted) and the two Claude mutant reviews still running.
- 2026-11-17 (UTC) · claude-code/opus-5.5 (manager) + session 68 reviews finished · Replaced the handover's running reviews with their finished counts and the saved findings files, so the next session starts the two fix rounds.
- 2026-11-17 (UTC) · claude-code/opus-5.5 (manager) + session 69 handover · Recorded session 69 results (D67 answered, T037d and T154a fix rounds, T025b contract) and the two runs still going, so the next manager starts from them; added the one-cargo-job memory rule after two out-of-memory stops.
- 2026-11-17 (UTC) · claude-code/opus-5.5 (manager) + session 69 T025b review · Replaced the running T025b review in the handover with its four findings and where they are saved, since it finished after the handover commit.
- 2026-11-17 (UTC) · claude-code/opus-5.5 (manager) + session 69 T037d re-plant · Replaced the running T037d re-plant in the handover with its result (27 of 32 killed, four survivors, one compile error), since it finished after the handover commit.
- 2026-11-17 (UTC) · claude-code/opus-5.5 (manager) + session 70 handover · Recorded session 70 results (D68 answered, T037d done, T042a implemented, T037b/c/d and T154a integrated on the batch) and the T042a review still running, so the next manager starts from the integrated batch.
- 2026-11-17 (UTC) · claude-code/opus-5.5 (manager) + session 71 handover · Recorded session 71 results (T042a mutant table 16/16 killed and on the batch, S71 bookkeeping on the batch, T025b-i implemented and verified) and the two-keys run still going in slot 1.
- 2026-11-17 (UTC) · claude-code/opus-5.5 (manager) + session 71 two-keys result · Replaced the running two-keys run in the handover with its verified result.

- 2026-11-18 (UTC) · claude-code/opus-5.5 (manager) + session 72 pause · Rewrote the handover for a user-requested PC restart: T025b-i landed on the batch, T025b-q implemented but unverified, T037c mutant gaps briefed.
- 2026-11-19 (UTC) · claude-code/opus-5.5 (manager) + session 73 handover · Recorded T025b-q and the T037c fix round landing on the batch, the open batch PR as next, and the mutant-driver lessons, so the next manager merges rather than re-verifies.
- 2026-11-19 (UTC) · claude-code/opus-5.5 (manager) + session 74 handover · Recorded the PR #97 merge, T154b and T037e landing on the new batch with their errata and mutant results, and the bookkeeping and batch PR as next, because the session reached the 150k context mark.
- 2026-11-19 (UTC) · claude-code/opus-5.5 (manager) + session 75 handover · Recorded T154c and the four recovered packages (T056a/b, T150a/b) on PR #98, the T154c mutant survivors, and the user's S-set gate on T150a/b.
- 2026-11-19 (UTC) · claude-code/opus-5.5 (manager) + session 76 handover · Recorded the user's T150a/T150b approval, T152 and its fix landing, the T154c fix rounds and re-plant, and T153 as the next package.
- 2026-11-19 (UTC) · claude-code/opus-5.5 (manager) + session 77 handover · Recorded the PR #98 merge, the six runs started on the new batch (T153, T151a and four fact sheets) with their slots and briefs, and the user's request for maximum parallel work, because the session handed over at the context limit with runs still going.
- 2026-11-19 (UTC) · claude-code/opus-5.5 (manager) + session 78 start · Named the six runs started this session with slots, branches and briefs, so a session that ends early can collect them.
- 2026-11-19 (UTC) · claude-code/opus-5.5 (manager) + T151a review · Recorded the T151a mutant review result and the two fix rounds now running, so the next session knows what to re-check.
- 2026-11-19 (UTC) · claude-code/opus-5.5 (manager) + session 78 progress · Recorded the four landed drafts, the T153 two-keys pass and mutant driver, and the T052 architect and two draft runs now going.
- 2026-11-19 (UTC) · claude-code/opus-5.5 (manager) + session 78 progress · Recorded T151a landing, T151b dispatch, the T052 contract and its D69 questions, and the T040 architect run.
- 2026-11-19 (UTC) · claude-code/opus-5.5 (manager) + session 78 handover · Handed over at the context limit with four runs named by slot, branch and record, the T153 mutant survivors and how to re-run them, and the D69 decisions waiting on the user.
- 2026-11-19 (UTC) · claude-code/opus-5.5 (manager) + session 78 handover update · Recorded T100 draft B landing after the handover.
- 2026-11-19 (UTC) · claude-code/opus-5.5 (manager) + session 79 handover · Handed over at the 150k context mark with two runs named by slot, branch and record, after landing T153, the T040 contract and its D70 revision, the D69 record, and T041 facts and draft B.
- 2026-11-19 (UTC) · claude-code/opus-5.5 (manager) + session 80 handover · Handed over at the 150k context mark with T150c's continuation named by slot, branch and record, the user's D71/D72 answers recorded here until a doc worker writes them to the decision files, and the T041, T100 and T042b contracts landed.
- 2026-11-19 (UTC) · claude-code/opus-5.5 (manager) + session 80 handover update · Recorded T150c attempt 2 passing verify after the handover, so the next session starts with nothing running.
- 2026-11-19 (UTC) · claude-code/opus-5.5 (manager) + session 81 handover · Recorded the runs in flight and the next steps at the usage limit, so the next session can pick up T150c, T041a, T100a and T042b1 without rebuilding state.
- 2026-11-20 (UTC) · claude-code/opus-5.5 (manager) + session 82 handover · Handed over at the 150k context mark with nothing running, after landing T100a, T150c, T042b1 and T041a and leaving T100b for its contract review, so the next session starts from T100b's review and T042b2–b4.
- 2026-11-20 (UTC) · claude-code/opus-5.5 (manager) + session 83 handover · Handed over at the 150k context mark with T042b4 and the T100b fix round still running, after landing T042b3 and T041b, so the next session can verify both and continue with T042b2 and T041c.
- 2026-11-20 (UTC) · claude-code/opus-5.5 (manager) + session 83 handover update · Recorded that the T100b fix round finished and verified, so the next session goes straight to its re-review.
- 2026-11-20 (UTC) · claude-code/opus-5.5 (manager) + session 83 handover update · Recorded that T042b4 finished and verified, so nothing is left running.
- 2026-11-20 (UTC) · claude-code/opus-5.5 (manager) + session 84 handover · Handed over at the 150k context mark with nothing running, after landing T100b, T042b2, T042b4 and the T042b docs and verifying T154d, so the next session starts with T154d's mutant review.
- 2026-11-20 (UTC) · claude-code/opus-5.5 (manager) + session 86 handover · Handed over at the 165k context mark with the T100 baseline still running, after finishing session 85's cut-off fix rounds, landing T154d, T151b and the T042c fact sheet, and verifying T052 with its two-keys suite.
- 2026-11-21 (UTC) · claude-code/opus-5.5 (manager) + session 87 handover · Handed over at the 156k context mark with the T100 baseline remainder running, after landing T052 and T052b, opening PR #99 and fixing its Linux CI race through erratum T151-E3.
- 2026-11-21 (UTC) · claude-code/opus-5.5 (manager) + session 88 handover · Handed over at the 170k context mark with nothing running, after PR #99's merge, the T100 baseline and its scorer fix, T101's proposed ADR with the user's answers, and T040a on the batch.
- 2026-11-21 (UTC) · claude-code/opus-5.5 (manager) + session 89 handover · Handed over at the 165k context mark with nothing running, after the T040a mutant run and fix round, errata T100-E2/E3 with T100d/T100e, and the T102 prototype, all on PR #100.
- 2026-11-21 (UTC) · claude-code/haiku-5.5 (bookkeeper) + S90 bookkeeping · Set the handover to the session 90 state (T151c on the batch, D75 waiting on the user, baseline re-runs after the Go reset, editor work on hold) and added the session 90 facts, so the next session starts from the current state.
- 2026-11-21 (UTC) · claude-code/opus-5.5 (manager) + session 91 handover · Replaced the handover with session 91's state: T040b and T040c done on PR #101 with erratum T040-E2, the user's reviews and Ctrl-C check next, T041d after the merge.
- 2026-11-21 (UTC) · claude-code/opus-5.5 (manager) + session 92 handover · Replaced the handover with session 92's state: T041d done on PR #101 with its errata, D75 and T040-E2 answered, T100f waiting for a model.
- 2026-11-21 (UTC) · claude-code/opus-5.5 (manager) + session 93 handover · Replaced the handover with session 93's state: T100f done and the baseline re-scored, T041d's Linux leg done, and PR #101 waiting on the user, because no OpenCode tier was available for bookkeeping.
- 2026-11-21 (UTC) · claude-code/opus-5.5 (manager) + session 94 handover · Replaced the handover with session 94's state: T155 approved (D76), T155a and T155b done on `mgr/session-94-batch`, T155c briefed, the Ctrl-C check passed.
- 2026-11-21 (UTC) · claude-code/opus-5.5 (manager) + session 95 handover · Recorded T155c and T155d done on the session-95 batch, PR #102 merged, and T155e next behind the user's T155d review.
- 2026-11-21 (UTC) · claude-code/opus-5.5 (manager) + session 96 handover · Recorded T155e done and T155 complete on PR #103, the user's T155d approval and the gitleaks fingerprint ignore, and pointed Next at the merge and the next wave.
- 2026-11-22 (UTC) · claude-code/opus-5.5 (manager) + S97 handover · Rewrote the handover for the walk-demo wave (D77 decided, T050a on the Claude implementer in slot 2), because the context meter reached 150k.
- 2026-11-22 (UTC) · claude-code/opus-5.5 (manager) + S97 handover update · Recorded T050a's implementation (`469f7d2`, verify PASS) so the next session starts at its mutant review instead of waiting for a run that has ended.
- 2026-11-22 (UTC) · claude-code/opus-5.5 (manager) + S98 handover · Rewrote the handover for the finished walk-demo wave on PR #104 and recorded the launch line, because T050b, T049a and the erratum landed on the batch this session.
- 2026-11-22 (UTC) · claude-code/opus-5.5 (manager) + S99 handover · Rewrote the handover for the user's combat-demo-first order, the session 99 batch (T156 contract draft, T157 and M4 fact sheets, T102 walkthrough rewrite) and the D78 questions awaiting the user.
- 2026-11-22 (UTC) · claude-code/opus-5.5 (manager) + session 100 handover · Recorded the T156 combat demo built on the batch, the user's play-test approval of T156b, and the T157 contract as next.
- 2026-11-22 (UTC) · claude-code/opus-5.5 (manager) + session 101 handover · Recorded the merged PR #105, the reviewed T157 contract and accepted amendments, and T157a built but unverified when the usage limit hit.

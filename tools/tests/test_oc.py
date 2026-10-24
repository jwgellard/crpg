#!/usr/bin/env python3
"""Subprocess tests for tools/agents/oc.py (A0-3).

Each test runs ``oc.py`` as a real process against a fake ``opencode``
executable (``tools/tests/fake_opencode.py``) and a fake verifier
(``tools/tests/fixtures/oc/fake_verify.py``) inside temporary git
repositories with real worktree slots. The real ``opencode`` and the network
are never touched; start gaps and lock waits are shortened through the
environment so the suite finishes in under 60 seconds.

Coverage maps to docs/AGENT_TEAM_PLAN.md section 5.1 and the A0-3 brief:
stdin closed, ``--dir``/``PWD``, start staggering, locked retry and
exhaustion, prompt intactness (angle brackets and pipes), the timeout
tree-kill, no ``--auto``, the stall retry and ``no-edit``, the infra cause
table, verify FAIL/ERROR plumbing, the summary shape and 40-line cap,
dry-run purity, attempt and branch discipline, result records, ``record``,
``stats`` demote/promote boundaries, ``wave`` fan-out, and the idle
watchdog (``hang`` and the OpenCode-log quota path), plus the attempt-3
review fixes: a non-signalling PID probe, infra classification scoped to
provider/process errors with a stop-finish exemption, stale-lock reclaim
on the same slot, ``is_failure`` exactly per MODELS.md section 4.1,
dry-run brief validation, records for slot-busy/no-exe, the git-clean
check, ask/grunt record and skip behaviour, overlapping-process stagger,
UTF-8 summaries, role-aware next lines, wave model caps, renamed-model
refusal, and filed-brief/placeholder/same-crate handling, plus the
attempt-4 OpenCode 2.x support: version detection, the 2.x argv
(``--standalone``, model ``#variant``, no ``--dir``) against the kept 1.x
argv, the stall retry in both generations, top-level ``error`` events, the
removed Cat Jam IDs, and dry-run output per version, plus the attempt-5
second-review fixes: tool errors are never classified as infra (only
top-level error events, the process's own output and the OpenCode log),
exact fake-mode matching so the ``error-quota`` event branch is live,
``ask`` recording a ``no-exe`` exit, and the Windows liveness probe
counting a PID alive when in doubt, plus A0-3b: the run's base reaching the
verifier (``run`` and ``wave``), the agent-declaration fallback into a slot
prepared from a base that predates A0-2 with its ``info/exclude`` entries,
and ``no-agent`` as both a setup exit and an event classification, plus
A0-3c: the Go spend guard (window sums from injected records, the ``spend``
view and ``--json``, the 5-hour/weekly/scarce refusals with their cost-0
record, ``ask`` and ``wave`` refusals, the override and ``--dry-run``).
A0-3g: a grunt run that changed nothing is ``no-edit`` (never PASS), a
verified grunt run is committed once by ``oc.py`` with its hash in the
summary and ``--json``, a failed grunt verification commits nothing, and an
``implement`` run still gets no commit from ``oc.py``.
A0-4c: ``ask --diff/--base`` writes a slot branch's diff beside the run's
log, appends its path to the question, and records ``diff``; an unresolved
ref or an empty diff exits ``bad-ref``/``empty-diff`` before OpenCode starts
and never counts against the model.
"""

from __future__ import annotations

import ctypes
import importlib.util
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from datetime import datetime, timedelta, timezone
from pathlib import Path

HERE = Path(__file__).resolve().parent
TOOLS = HERE.parent
OC = TOOLS / "agents" / "oc.py"
FAKE_OPENCODE = HERE / "fake_opencode.py"
FAKE_VERIFY = HERE / "fixtures" / "oc" / "fake_verify.py"
PROMPT = "Read BRIEF.md in this directory and do what it says."
STALL_SENTENCE = (
    "Write the code in small edits; do not plan the whole solution in one step."
)
MODEL = "go/fake-a"


def load_oc():
    spec = importlib.util.spec_from_file_location("oc_under_test", OC)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


OCMOD = load_oc()


def sh(args: list[str], cwd: Path) -> subprocess.CompletedProcess:
    proc = subprocess.run(
        args, cwd=cwd, capture_output=True, text=True, timeout=60
    )
    if proc.returncode != 0:
        raise AssertionError(f"{args} failed: {proc.stdout} {proc.stderr}")
    return proc


BRIEF_STUB = """#!/usr/bin/env python3
import sys
from pathlib import Path
args = sys.argv[1:]
task = args[0]
out = Path(args[args.index("--out") + 1])
out.write_text("# Brief %s\\n\\nGenerated stub.\\n" % task, encoding="utf-8")
"""


class DispatchCase(unittest.TestCase):
    """Runs against one temp repo with two real worktree slots."""

    @classmethod
    def setUpClass(cls) -> None:
        cls.tmp = tempfile.TemporaryDirectory(prefix="oc test ")
        cls.addClassCleanup(cls.tmp.cleanup)
        root = Path(cls.tmp.name)
        cls.repo = root / "repo"
        cls.slots = root / "slots"
        cls.repo.mkdir()
        cls.slots.mkdir()
        sh(["git", "init", "-b", "master", str(cls.repo)], root)
        sh(["git", "config", "user.email", "test@example.invalid"], cls.repo)
        sh(["git", "config", "user.name", "oc test"], cls.repo)
        (cls.repo / "README.md").write_text("# test repo\n", encoding="utf-8")
        sh(["git", "add", "README.md"], cls.repo)
        sh(
            ["git", "-c", "commit.gpgsign=false", "commit", "-m", "init"],
            cls.repo,
        )
        # `no-agents` pins the commit before OpenCode was configured, so a
        # slot prepared from it is exactly the pre-A0-2 slot the fallback
        # targets; `master` does define every agent, so the other tests keep
        # their ordinary path.
        cls.old_base = sh(["git", "rev-parse", "HEAD"], cls.repo).stdout.strip()
        agents = cls.repo / "tools" / "agents"
        agents.mkdir(parents=True)
        (agents / "brief.py").write_text(BRIEF_STUB, encoding="utf-8")
        opencode_agents = cls.repo / ".opencode" / "agents"
        opencode_agents.mkdir(parents=True)
        for role in ("implement", "tests", "review", "scout", "grunt"):
            (opencode_agents / f"crpg-{role}.md").write_text(
                f"# crpg-{role}\n", encoding="utf-8"
            )
        (cls.repo / "opencode.json").write_text(
            '{"agent": {}}\n', encoding="utf-8"
        )
        sh(["git", "add", "tools", ".opencode", "opencode.json"], cls.repo)
        sh(
            ["git", "-c", "commit.gpgsign=false", "commit", "-m", "agents"],
            cls.repo,
        )
        sh(["git", "branch", "no-agents", cls.old_base], cls.repo)
        sh(["git", "worktree", "add", str(cls.slots / "slot-1")], cls.repo)
        sh(["git", "worktree", "add", str(cls.slots / "slot-2")], cls.repo)
        cls.slot1 = (cls.slots / "slot-1").resolve()
        cls.slot2 = (cls.slots / "slot-2").resolve()
        cls.base_env = {
            "CRPG_REPO": str(cls.repo),
            "CRPG_SLOTS": str(cls.slots),
            "CRPG_OPENCODE": str(FAKE_OPENCODE),
            "CRPG_VERIFY": str(FAKE_VERIFY),
            "CRPG_OC_START_GAP": "0",
            "CRPG_OC_LOCK_WAIT": "0.05",
            "FAKE_OC_MODE": "ok",
            "FAKE_VERIFY_MODE": "pass",
        }

    def setUp(self) -> None:
        self.count = 0
        self.case_dir = Path(tempfile.mkdtemp(prefix="oc case "))
        self.addCleanup(shutil.rmtree, self.case_dir, True)
        self.reset_slots()

    def reset_slots(self) -> None:
        """Put the shared worktrees back on their own branches, clean, at master.

        One test's leftover WIP or commits must never make the next test's
        slot look occupied (A0-12), so every test starts from the same two
        free slots.
        """
        for name, slot in (("slot-1", self.slot1), ("slot-2", self.slot2)):
            sh(["git", "checkout", "-f", "-B", name, "master"], slot)
            sh(["git", "clean", "-ffdx"], slot)

    # -- helpers -----------------------------------------------------

    def tid(self) -> str:
        self.count += 1
        name = re.sub(r"[^A-Za-z0-9]+", "-", self._testMethodName).strip("-")
        return f"T-{name}-{self.count}"

    def brief(self, text: str = "# Brief\n") -> str:
        path = self.case_dir / f"BRIEF-{self.count}.md"
        path.write_text(text, encoding="utf-8")
        return str(path)

    def fresh_file(self, name: str) -> Path:
        return self.case_dir / name

    def fresh_dir(self, name: str) -> Path:
        path = self.case_dir / name
        path.mkdir()
        return path

    def oc(self, *argv: str, env: dict | None = None, timeout: int = 90):
        merged = dict(os.environ)
        merged.update(self.base_env)
        for key, value in (env or {}).items():
            if value is None:
                merged.pop(key, None)
            else:
                merged[key] = value
        return subprocess.run(
            [sys.executable, str(OC), *argv],
            cwd=self.repo,
            capture_output=True,
            text=True,
            timeout=timeout,
            env=merged,
        )

    def oc_run(self, task, role="implement", model=MODEL, extra=None,
               env=None, timeout=90, brief=True, base="master"):
        argv = [
            "run", "--task", task, "--role", role, "--model", model,
            "--base", base, "--timeout", "60", "--idle", "10",
        ]
        if brief:
            argv += ["--brief", self.brief()]
        argv += extra or []
        return self.oc(*argv, env=env, timeout=timeout)

    def exclude_file(self, slot: Path) -> Path:
        """The worktree's info/exclude file, resolved the way oc.py does."""
        out = sh(["git", "rev-parse", "--git-path", "info/exclude"], slot).stdout
        path = Path(out.strip())
        return path if path.is_absolute() else slot / path

    def record_path(self, task, model=MODEL, attempt=1) -> Path:
        safe = model.replace("/", "_")
        return (
            self.repo / ".agents" / "runs" / task / f"{attempt}-{safe}.result.json"
        )

    def log_path(self, task, model=MODEL, attempt=1) -> Path:
        safe = model.replace("/", "_")
        return self.repo / ".agents" / "runs" / task / f"{attempt}-{safe}.jsonl"

    def read_record(self, task, model=MODEL, attempt=1) -> dict:
        return json.loads(self.record_path(task, model, attempt).read_text())

    def fake_calls(self, log: Path) -> list[dict]:
        if not log.exists():
            return []
        return [
            json.loads(line)
            for line in log.read_text(encoding="utf-8").splitlines()
            if line.strip()
        ]

    # -- happy path and record schema --------------------------------

    def test_pass_run_verifies_and_records(self) -> None:
        task = self.tid()
        result = self.oc_run(task)
        self.assertEqual(result.returncode, 0, result.stderr)
        lines = result.stdout.splitlines()
        self.assertTrue(lines[0].startswith(f"PASS {task} implement"), lines[0])
        self.assertIn("files:", result.stdout)
        self.assertIn("scope:", result.stdout)
        self.assertIn("fake worker report line 1", result.stdout)
        self.assertEqual(lines[-1], "next: open the PR")
        self.assertLessEqual(len(lines), 40)
        record = self.read_record(task)
        for key in (
            "version", "task", "role", "model", "attempt", "started_utc",
            "elapsed_s", "verdict", "cause", "failed_checks", "steps",
            "tokens", "cost", "retries", "outcomes",
        ):
            self.assertIn(key, record)
        self.assertEqual(record["verdict"], "PASS")
        self.assertIsNone(record["cause"])
        self.assertEqual(record["steps"], 2)
        self.assertEqual(
            record["tokens"],
            {"input": 1600, "cached": 1200, "output": 460},
        )
        self.assertEqual(record["retries"], [])
        self.assertEqual(record["outcomes"], [])
        self.assertTrue(self.log_path(task).is_file())

    def test_run_json_prints_one_object(self) -> None:
        task = self.tid()
        result = self.oc_run(task, role="review", extra=["--json"])
        self.assertEqual(result.returncode, 0, result.stderr)
        payload = json.loads(result.stdout)
        self.assertEqual(payload["verdict"], "PASS")
        self.assertEqual(payload["task"], task)
        self.assertEqual(payload["attempt"], 1)
        self.assertIn("steps", payload)
        self.assertIn("tokens", payload)

    # -- process fixes from MODELS.md section 1 ----------------------

    def test_stdin_is_closed(self) -> None:
        task = self.tid()
        result = self.oc_run(task, role="review", env={"FAKE_OC_STDIN": "check"})
        log = self.log_path(task, model=MODEL).read_text(encoding="utf-8")
        self.assertIn("STDIN-CLOSED", log)

    def test_dir_pwd_prompt_and_no_auto(self) -> None:
        # Default fake is OpenCode 2.x: no --dir, --standalone set, the slot
        # is the process cwd plus PWD, and the prompt arrives intact.
        task = self.tid()
        call_log = self.fresh_file("calls.jsonl")
        result = self.oc_run(task, env={"FAKE_OC_LOG": str(call_log)})
        self.assertEqual(result.returncode, 0, result.stderr)
        calls = self.fake_calls(call_log)
        self.assertGreaterEqual(len(calls), 1)
        first = calls[0]
        self.assertNotIn("--dir", first["argv"])
        self.assertIn("--standalone", first["argv"])
        self.assertNotIn("--variant", first["argv"])
        self.assertEqual(Path(first["cwd"]).resolve(), self.slot1)
        self.assertEqual(Path(first["pwd"]).resolve(), self.slot1)
        self.assertIn("--agent", first["argv"])
        self.assertNotIn("--auto", first["argv"])
        self.assertEqual(first["argv"][-1], PROMPT)

    def test_1x_argv_keeps_dir_and_variant_flag(self) -> None:
        # When the executable reports 1.x, oc.py keeps the --dir form.
        task = self.tid()
        call_log = self.fresh_file("calls.jsonl")
        result = self.oc_run(
            task,
            extra=["--variant", "high"],
            env={"FAKE_OC_LOG": str(call_log),
                 "FAKE_OC_VERSION": "opencode v1.18.35"},
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        calls = self.fake_calls(call_log)
        self.assertGreaterEqual(len(calls), 1)
        first = calls[0]
        self.assertIn("--dir", first["argv"])
        self.assertEqual(
            first["argv"][first["argv"].index("--dir") + 1], str(self.slot1)
        )
        self.assertNotIn("--standalone", first["argv"])
        self.assertIn("--variant", first["argv"])
        self.assertEqual(
            first["argv"][first["argv"].index("--variant") + 1], "high"
        )
        self.assertEqual(Path(first["cwd"]).resolve(), self.slot1)
        self.assertEqual(Path(first["pwd"]).resolve(), self.slot1)
        self.assertNotIn("--auto", first["argv"])
        self.assertEqual(first["argv"][-1], PROMPT)

    def test_2x_argv_carries_variant_in_model(self) -> None:
        task = self.tid()
        call_log = self.fresh_file("calls.jsonl")
        result = self.oc_run(
            task,
            extra=["--variant", "high"],
            env={"FAKE_OC_LOG": str(call_log)},
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        calls = self.fake_calls(call_log)
        self.assertGreaterEqual(len(calls), 1)
        first = calls[0]
        self.assertNotIn("--dir", first["argv"])
        self.assertNotIn("--variant", first["argv"])
        self.assertIn("--standalone", first["argv"])
        flag = first["argv"][first["argv"].index("-m") + 1]
        self.assertEqual(flag, f"{MODEL}#high")

    def test_special_chars_reach_worker_intact(self) -> None:
        question = "On which line is <line number> | & > honoured exactly?"
        prompt_file = self.fresh_file("prompt.txt")
        result = self.oc(
            "ask", "--role", "scout", "--model", MODEL,
            "--timeout", "60", "--idle", "10", question,
            env={
                "FAKE_OC_MODE": "prompt-file,ok",
                "FAKE_OC_PROMPT_FILE": str(prompt_file),
            },
        )
        self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
        self.assertEqual(prompt_file.read_text(encoding="utf-8"), question)

    def test_starts_are_staggered(self) -> None:
        # Two overlapping processes contend on the shared stamp file, so a
        # per-process sleep (no cross-process coordination) fails this test:
        # both would sleep side by side and start together.
        stamp = self.repo / ".agents" / "last_start"
        if stamp.exists():
            stamp.unlink()
        starts = self.fresh_file("starts.txt")
        env = {"CRPG_OC_START_GAP": "1.0", "FAKE_OC_STARTS": str(starts)}
        results: dict[int, subprocess.CompletedProcess] = {}

        def run_ask(key: int, question: str) -> None:
            results[key] = self.oc(
                "ask", "--role", "scout", "--model", MODEL,
                "--timeout", "60", "--idle", "10", question, env=env,
            )

        threads = [
            threading.Thread(target=run_ask, args=(0, "first question")),
            threading.Thread(target=run_ask, args=(1, "second question")),
        ]
        for thread in threads:
            thread.start()
        for thread in threads:
            thread.join(90)
        self.assertEqual(set(results), {0, 1})
        self.assertEqual(results[0].returncode, 0, results[0].stderr)
        self.assertEqual(results[1].returncode, 0, results[1].stderr)
        marks = sorted(float(line) for line in starts.read_text().splitlines())
        self.assertEqual(len(marks), 2)
        self.assertGreaterEqual(marks[1] - marks[0], 0.9)

    # -- database lock -----------------------------------------------

    def test_locked_retry_then_success_stays_one_attempt(self) -> None:
        task = self.tid()
        call_log = self.fresh_file("calls.jsonl")
        state = self.fresh_dir("state")
        result = self.oc_run(
            task,
            env={
                "FAKE_OC_MODE": "locked:2",
                "FAKE_OC_STATE": str(state),
                "FAKE_OC_LOG": str(call_log),
            },
        )
        self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
        self.assertEqual(len(self.fake_calls(call_log)), 3)
        record = self.read_record(task)
        self.assertEqual(record["attempt"], 1)
        self.assertEqual(
            [entry["cause"] for entry in record["retries"]],
            ["locked", "locked"],
        )
        self.assertFalse(self.log_path(task, attempt=2).exists())
        self.assertFalse(self.record_path(task, attempt=2).exists())

    def test_locked_exhausted_is_infra(self) -> None:
        task = self.tid()
        state = self.fresh_dir("state")
        result = self.oc_run(
            task,
            env={"FAKE_OC_MODE": "locked", "FAKE_OC_STATE": str(state)},
        )
        self.assertEqual(result.returncode, 2, result.stdout)
        self.assertIn("locked", result.stdout + result.stderr)
        record = self.read_record(task)
        self.assertEqual(record["cause"], "locked")
        self.assertEqual(len(record["retries"]), 2)

    # -- reasoning stall ----------------------------------------------

    def test_stall_retries_once_with_low_variant_then_succeeds(self) -> None:
        # Default fake is 2.x: the stall retry carries `low` as model#low.
        task = self.tid()
        call_log = self.fresh_file("calls.jsonl")
        state = self.fresh_dir("state")
        result = self.oc_run(
            task,
            env={
                "FAKE_OC_MODE": "stall-once",
                "FAKE_OC_STATE": str(state),
                "FAKE_OC_LOG": str(call_log),
            },
        )
        self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
        calls = self.fake_calls(call_log)
        self.assertEqual(len(calls), 2)
        self.assertNotIn("--variant", calls[0]["argv"])
        self.assertNotIn("--dir", calls[0]["argv"])
        second = calls[1]["argv"]
        self.assertNotIn("--variant", second)
        self.assertIn("--standalone", second)
        self.assertEqual(
            second[second.index("-m") + 1], f"{MODEL}#low"
        )
        self.assertIn(STALL_SENTENCE, second[-1])
        record = self.read_record(task)
        self.assertEqual(record["attempt"], 1)
        self.assertEqual(
            record["retries"], [{"cause": "stall", "variant": "low"}]
        )

    def test_stall_retry_uses_variant_flag_on_1x(self) -> None:
        task = self.tid()
        call_log = self.fresh_file("calls.jsonl")
        state = self.fresh_dir("state")
        result = self.oc_run(
            task,
            env={
                "FAKE_OC_MODE": "stall-once",
                "FAKE_OC_STATE": str(state),
                "FAKE_OC_LOG": str(call_log),
                "FAKE_OC_VERSION": "opencode v1.18.35",
            },
        )
        self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
        calls = self.fake_calls(call_log)
        self.assertEqual(len(calls), 2)
        self.assertNotIn("--variant", calls[0]["argv"])
        second = calls[1]["argv"]
        self.assertIn("--variant", second)
        self.assertEqual(second[second.index("--variant") + 1], "low")
        self.assertEqual(second[second.index("-m") + 1], MODEL)
        self.assertIn(STALL_SENTENCE, second[-1])

    def test_stall_twice_is_no_edit(self) -> None:
        task = self.tid()
        call_log = self.fresh_file("calls.jsonl")
        result = self.oc_run(
            task, env={"FAKE_OC_MODE": "stall", "FAKE_OC_LOG": str(call_log)}
        )
        self.assertEqual(result.returncode, 2, result.stdout)
        self.assertIn("no-edit", result.stdout + result.stderr)
        self.assertEqual(len(self.fake_calls(call_log)), 2)
        self.assertEqual(self.read_record(task)["cause"], "no-edit")

    # -- timeout -------------------------------------------------------

    def test_timeout_kills_the_tree(self) -> None:
        task = self.tid()
        beat = self.fresh_file("beat.txt")
        state = self.fresh_dir("state")
        result = self.oc_run(
            task,
            extra=["--timeout", "2"],
            env={
                "FAKE_OC_MODE": "spawn-child",
                "FAKE_OC_HEARTBEAT": str(beat),
                "FAKE_OC_STATE": str(state),
            },
            timeout=60,
        )
        self.assertEqual(result.returncode, 2, result.stdout)
        self.assertIn("timeout", result.stdout + result.stderr)
        self.assertEqual(self.read_record(task)["cause"], "timeout")
        time.sleep(1.5)
        first = beat.stat().st_size
        time.sleep(1.5)
        self.assertEqual(beat.stat().st_size, first)

    # -- infra cause table ----------------------------------------------

    def test_infra_causes(self) -> None:
        cases = [
            ("fail402", "402"),
            ("quota", "quota"),
            ("headers", "timeout"),
            ("sig-model", "model"),
        ]
        for mode, cause in cases:
            with self.subTest(mode=mode):
                self.count += 1
                task = self.tid()
                result = self.oc_run(task, env={"FAKE_OC_MODE": mode})
                self.assertEqual(result.returncode, 2, result.stdout)
                self.assertIn(cause, result.stdout + result.stderr)
                self.assertEqual(self.read_record(task)["cause"], cause)

    def test_classify_infra_table(self) -> None:
        classify = OCMOD.classify_infra
        self.assertEqual(classify("ERROR 402 Payment Required"), "402")
        self.assertEqual(
            classify(" FAILED: insufficient account funds "), "402"
        )
        self.assertEqual(classify("Rate Limit hit, slow down"), "quota")
        self.assertEqual(classify("quota exhausted"), "quota")
        self.assertEqual(
            classify("AI_APICallError: Go usage limit exceeded"), "quota"
        )
        self.assertEqual(classify("Too Many Requests, retry"), "quota")
        self.assertEqual(classify("HTTP  429 returned"), "quota")
        self.assertEqual(classify("stream error: HEADERS TIMED OUT"), "timeout")
        self.assertEqual(classify("Model Not Found: x"), "model")
        self.assertEqual(classify("unknown model 'x'"), "model")
        self.assertEqual(
            classify('{"type":"provider.no-route","message":"Model unavailable: x"}'),
            "model",
        )
        self.assertIsNone(classify("all done, nothing to report"))
        self.assertEqual(OCMOD.DEMOTE_WINDOW, 4)
        self.assertEqual(OCMOD.DEMOTE_FAILURES, 2)
        self.assertEqual(OCMOD.PROMOTE_MIN_RUNS, 5)

    def test_error_event_is_model(self) -> None:
        # A 2.x top-level error event (provider.no-route, no steps)
        # exits 2 with cause model.
        task = self.tid()
        result = self.oc_run(task, env={"FAKE_OC_MODE": "no-route"})
        self.assertEqual(result.returncode, 2, result.stdout)
        self.assertIn("model", result.stdout + result.stderr)
        self.assertEqual(self.read_record(task)["cause"], "model")

    def test_error_event_message_uses_cause_table(self) -> None:
        # The top-level error event's message goes through the cause table:
        # a rate-limit message classifies as quota, not model, and the mode
        # is matched exactly so this really is the error-event branch (not
        # the plain stdout-quota branch).
        task = self.tid()
        result = self.oc_run(task, env={"FAKE_OC_MODE": "error-quota"})
        self.assertEqual(result.returncode, 2, result.stdout)
        self.assertIn("quota", result.stdout + result.stderr)
        self.assertEqual(self.read_record(task)["cause"], "quota")
        events = [
            json.loads(line)
            for line in self.log_path(task).read_text(encoding="utf-8").splitlines()
            if line.strip()
        ]
        self.assertTrue(events, "the fake wrote no events")
        self.assertEqual(events[0].get("type"), "error")
        self.assertEqual(events[0]["error"]["type"], "provider.apiError")

    def test_tool_error_is_never_classified(self) -> None:
        # A tool_use state.error echoes command output, so it must not
        # classify as infra: this run ends on `tool-calls` and exits 1 but
        # keeps its normal verdict instead of exiting 2 quota.
        task = self.tid()
        result = self.oc_run(task, env={"FAKE_OC_MODE": "tool-error-quota"})
        self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
        self.assertTrue(result.stdout.splitlines()[0].startswith(f"PASS {task}"))
        record = self.read_record(task)
        self.assertEqual(record["verdict"], "PASS")
        self.assertIsNone(record["cause"])

    def test_stall_retry_after_tool_error(self) -> None:
        # A stalled run (reason `length`, no edit) whose earlier tool error
        # says "rate limit" must still get its variant-low retry: the tool
        # error is not infra text, so it cannot pre-empt the stall check.
        task = self.tid()
        call_log = self.fresh_file("calls.jsonl")
        state = self.fresh_dir("state")
        result = self.oc_run(
            task,
            env={
                "FAKE_OC_MODE": "stall-tool-error",
                "FAKE_OC_STATE": str(state),
                "FAKE_OC_LOG": str(call_log),
            },
        )
        self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
        calls = self.fake_calls(call_log)
        self.assertEqual(len(calls), 2)
        second = calls[1]["argv"]
        self.assertEqual(second[second.index("-m") + 1], f"{MODEL}#low")
        self.assertIn(STALL_SENTENCE, second[-1])
        record = self.read_record(task)
        self.assertEqual(
            record["retries"], [{"cause": "stall", "variant": "low"}]
        )

    # -- verifier plumbing -------------------------------------------------

    def test_verify_fail_is_exit_1_with_output(self) -> None:
        task = self.tid()
        result = self.oc_run(task, env={"FAKE_VERIFY_MODE": "fail"})
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertTrue(result.stdout.splitlines()[0].startswith(f"FAIL {task}"))
        self.assertIn("clippy", result.stdout)
        self.assertIn("error line 29", result.stdout)
        record = self.read_record(task)
        self.assertEqual(record["verdict"], "FAIL")
        self.assertEqual(record["failed_checks"], ["clippy"])

    def test_verify_error_is_exit_2(self) -> None:
        task = self.tid()
        result = self.oc_run(task, env={"FAKE_VERIFY_MODE": "error"})
        self.assertEqual(result.returncode, 2, result.stdout)
        self.assertIn("INFRA", result.stdout.splitlines()[0])

    def test_missing_verifier_is_exit_2(self) -> None:
        task = self.tid()
        result = self.oc_run(task, env={"CRPG_VERIFY": None})
        self.assertEqual(result.returncode, 2, result.stderr)
        self.assertIn("no-verify", result.stderr)

    def test_verify_timeout_is_exit_2_and_not_a_model_fault(self) -> None:
        # A passing draft once exceeded a fixed 900 s verify budget and was
        # reported as `verify-error`; the budget is now explicit, and its
        # expiry is a non-fault cause (A0-4b).
        task = self.tid()
        slow = self.fresh_file("slow_verify.py")
        slow.write_text(
            "import time\nfrom pathlib import Path\n"
            "time.sleep(30)\nPath(__file__).with_suffix('.done').write_text('x')\n",
            encoding="utf-8",
        )
        result = self.oc_run(
            task,
            extra=["--verify-timeout", "1"],
            env={"CRPG_VERIFY": str(slow)},
            timeout=60,
        )
        self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
        self.assertIn("cause: verify-timeout", result.stdout)
        record = self.read_record(task)
        self.assertEqual(record["cause"], "verify-timeout")
        self.assertEqual(record["verdict"], "INFRA")
        self.assertFalse(slow.with_suffix(".done").exists())
        self.assertFalse(OCMOD.is_failure(record))

    def test_wave_passes_its_verify_timeout_to_members(self) -> None:
        filed_dir = self.repo / ".agents" / "briefs"
        filed_dir.mkdir(parents=True, exist_ok=True)
        task = self.tid()
        (filed_dir / f"{task}.md").write_text(
            "# Brief\n\n## Files you own\n\n- `crates/wave-timeout/src/lib.rs`\n",
            encoding="utf-8",
        )
        slow = self.fresh_file("slow_wave_verify.py")
        slow.write_text("import time\ntime.sleep(30)\n", encoding="utf-8")
        result = self.oc(
            "wave", f"{task}:implement:{MODEL}", "--base", "master",
            "--verify-timeout", "1", timeout=60,
            env={"CRPG_VERIFY": str(slow)},
        )
        self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
        self.assertIn("verify-timeout", result.stdout + result.stderr)

    def test_review_role_needs_text_not_verify(self) -> None:
        task = self.tid()
        result = self.oc_run(
            task, role="review", env={"FAKE_VERIFY_MODE": "error"}
        )
        self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
        self.assertTrue(result.stdout.splitlines()[0].startswith(f"PASS {task}"))

    def test_tests_role_runs_gates(self) -> None:
        task = self.tid()
        verify_log = self.fresh_file("verify.jsonl")
        result = self.oc_run(
            task, role="tests", env={"FAKE_VERIFY_LOG": str(verify_log)}
        )
        self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
        logged = [
            json.loads(line)
            for line in verify_log.read_text(encoding="utf-8").splitlines()
        ]
        self.assertEqual(len(logged), 1)
        # A test package's fmt/clippy/test failures must not be hidden.
        self.assertNotIn("--skip", logged[0])
        self.assertNotIn("gates", logged[0])

    # -- full report file (A0-3d) -------------------------------------

    def test_full_report_is_written_beside_the_log(self) -> None:
        task = self.tid()
        result = self.oc_run(task, env={"FAKE_OC_MODE": "long-report"})
        self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
        report = self.log_path(task).with_suffix(".report.md")
        self.assertEqual(report.name, "1-go_fake-a.report.md")
        text = report.read_text(encoding="utf-8")
        self.assertEqual(text.splitlines()[-1], "report line 30")
        self.assertEqual(len(text.splitlines()), 30)
        lines = result.stdout.splitlines()
        self.assertIn(f"report: {report}", lines)
        self.assertIn("--- worker report (first 14 lines) ---", lines)
        self.assertIn("report line 14", lines)
        self.assertNotIn("report line 15", lines)
        self.assertLessEqual(len(lines), 40)

    def slot_from_verify_log(self, log: Path) -> Path:
        """The slot the fake verifier was handed in its `--slot` argument."""
        argv = json.loads(log.read_text(encoding="utf-8").splitlines()[0])
        return Path(argv[argv.index("--slot") + 1]).resolve()

    def commits_since_base(self, slot: Path, base: str = "master") -> int:
        return int(sh(["git", "rev-list", "--count", f"{base}..HEAD"], slot).stdout)

    def status_paths(self, slot: Path) -> str:
        """Porcelain status with untracked files listed one path per line."""
        return sh(
            ["git", "status", "--porcelain", "--untracked-files=all"], slot
        ).stdout

    # -- a run that changed nothing is not a PASS (A0-3d) -------------

    def test_empty_change_set_is_no_edit_not_pass(self) -> None:
        for role in ("implement", "tests"):
            task = self.tid()
            result = self.oc_run(task, role=role, env={"FAKE_VERIFY_MODE": "empty"})
            self.assertEqual(result.returncode, 2, result.stdout)
            self.assertNotIn("PASS", result.stdout.splitlines()[0])
            self.assertIn("cause: no-edit", result.stdout)
            record = self.read_record(task)
            self.assertEqual(record["cause"], "no-edit")
            self.assertEqual(record["failed_checks"], ["no-edit"])
            self.assertTrue(OCMOD.is_failure(record))

    def test_empty_change_set_is_fine_for_review(self) -> None:
        task = self.tid()
        result = self.oc_run(task, role="review", env={"FAKE_VERIFY_MODE": "empty"})
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertTrue(result.stdout.splitlines()[0].startswith(f"PASS {task}"))

    # -- grunt: empty runs and dispatcher commits (A0-3g) --------------

    def test_grunt_run_with_no_change_is_no_edit(self) -> None:
        # The default fake worker writes nothing and the fake verifier
        # passes: an untouched slot is no-edit, not PASS (session 13's
        # T136-facts run hit its step limit and still printed PASS).
        task = self.tid()
        result = self.oc_run(task, role="grunt")
        self.assertEqual(result.returncode, 2, result.stdout)
        self.assertNotIn("PASS", result.stdout.splitlines()[0])
        self.assertIn("cause: no-edit", result.stdout)
        record = self.read_record(task)
        self.assertEqual(record["cause"], "no-edit")
        self.assertEqual(record["verdict"], "INFRA")
        self.assertEqual(record["failed_checks"], ["no-edit"])
        self.assertTrue(OCMOD.is_failure(record))

    def test_verified_grunt_run_is_committed_by_oc(self) -> None:
        task = self.tid()
        verify_log = self.fresh_file("verify.jsonl")
        result = self.oc_run(
            task,
            role="grunt",
            extra=["--json"],
            env={"FAKE_OC_MODE": "grunt-edit", "FAKE_VERIFY_LOG": str(verify_log)},
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        payload = json.loads(result.stdout)
        slot = self.slot_from_verify_log(verify_log)
        self.assertEqual(self.commits_since_base(slot), 1)
        subject = sh(["git", "log", "-1", "--format=%s"], slot).stdout.strip()
        self.assertEqual(subject, f"{task}: grunt edits ({MODEL})")
        committed = sh(
            ["git", "show", "--name-only", "--format=", "HEAD"], slot
        ).stdout.split()
        self.assertEqual(sorted(committed), ["src/lib.rs", "tests/t.rs"])
        commit = sh(["git", "rev-parse", "--short", "HEAD"], slot).stdout.strip()
        self.assertEqual(payload["commit"], commit)
        self.assertIn(f"committed {commit}", payload["summary"].splitlines())
        # Everything reported was committed: no work is left for the manager.
        status = self.status_paths(slot)
        self.assertNotIn("src/lib.rs", status)
        self.assertNotIn("tests/t.rs", status)

    def test_failed_grunt_verification_commits_nothing(self) -> None:
        task = self.tid()
        verify_log = self.fresh_file("verify.jsonl")
        result = self.oc_run(
            task,
            role="grunt",
            env={"FAKE_OC_MODE": "grunt-edit", "FAKE_VERIFY_MODE": "fail",
                 "FAKE_VERIFY_LOG": str(verify_log)},
        )
        self.assertEqual(result.returncode, 1, result.stdout)
        self.assertNotIn("committed ", result.stdout)
        slot = self.slot_from_verify_log(verify_log)
        self.assertEqual(self.commits_since_base(slot), 0)
        status = self.status_paths(slot)
        self.assertIn("src/lib.rs", status)
        self.assertIn("tests/t.rs", status)

    def test_implement_run_is_not_committed_by_oc(self) -> None:
        task = self.tid()
        verify_log = self.fresh_file("verify.jsonl")
        result = self.oc_run(
            task,
            extra=["--json"],
            env={"FAKE_OC_MODE": "grunt-edit", "FAKE_VERIFY_LOG": str(verify_log)},
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        payload = json.loads(result.stdout)
        self.assertIsNone(payload["commit"])
        self.assertNotIn("committed ", payload["summary"])
        slot = self.slot_from_verify_log(verify_log)
        self.assertEqual(self.commits_since_base(slot), 0)
        self.assertIn("src/lib.rs", self.status_paths(slot))

    def test_provider_error_with_no_change_is_infra_not_pass(self) -> None:
        task = self.tid()
        result = self.oc_run(
            task,
            env={"FAKE_OC_MODE": "provider-fail", "FAKE_VERIFY_MODE": "empty"},
        )
        self.assertEqual(result.returncode, 2, result.stdout)
        self.assertIn("cause: model", result.stdout)
        record = self.read_record(task)
        self.assertEqual(record["cause"], "model")
        self.assertEqual(record["verdict"], "INFRA")
        self.assertEqual(record["failed_checks"], [])
        self.assertFalse(OCMOD.is_failure(record))

    def test_ask_without_text_is_exit_1(self) -> None:
        result = self.oc(
            "ask", "--role", "scout", "--model", MODEL,
            "--timeout", "60", "--idle", "10", "is anyone there?",
            env={"FAKE_OC_MODE": "no-text"},
        )
        self.assertEqual(result.returncode, 1, result.stdout)

    # -- the run's base reaches the verifier (A0-3b) -----------------------

    def test_default_base_is_origin_master(self) -> None:
        # Running without --base defaults the verifier's base to origin/master.
        task = self.tid()
        # Make origin/master fetchable in this synthetic repo.
        sh(["git", "remote", "add", "origin", str(self.repo)], self.repo)
        sh(["git", "update-ref", "refs/remotes/origin/master", "HEAD"], self.repo)
        verify_log = self.fresh_file("verify-default.jsonl")
        result = self.oc(
            "run", "--task", task, "--role", "implement", "--model", MODEL,
            "--timeout", "60", "--idle", "10",
            "--brief", self.brief(),
            env={"FAKE_VERIFY_LOG": str(verify_log)},
            timeout=120,
        )
        # remove remote to keep repo clean for other tests
        sh(["git", "remote", "remove", "origin"], self.repo)
        self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
        logged = [json.loads(line) for line in verify_log.read_text(encoding="utf-8").splitlines()]
        self.assertEqual(len(logged), 1)
        self.assertIn("--base", logged[0])
        self.assertEqual(logged[0][logged[0].index("--base") + 1], "origin/master")

    def test_verify_receives_the_runs_base(self) -> None:
        # A slot prepared from `--base X` is verified against X. Without this
        # the verifier falls back to `merge-base HEAD origin/master` and
        # reports every commit of X as an out-of-scope change.
        task = self.tid()
        verify_log = self.fresh_file("verify.jsonl")
        result = self.oc_run(
            task, base="no-agents", env={"FAKE_VERIFY_LOG": str(verify_log)}
        )
        self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
        logged = [
            json.loads(line)
            for line in verify_log.read_text(encoding="utf-8").splitlines()
        ]
        self.assertEqual(len(logged), 1)
        self.assertIn("--base", logged[0])
        self.assertEqual(logged[0][logged[0].index("--base") + 1], "no-agents")

    def test_wave_passes_its_base_to_the_verifier(self) -> None:
        task = self.tid()
        verify_log = self.fresh_file("verify.jsonl")
        result = self.oc(
            "wave", f"{task}:implement:{MODEL}", "--base", "no-agents",
            env={"FAKE_VERIFY_LOG": str(verify_log)}, timeout=120,
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        logged = [
            json.loads(line)
            for line in verify_log.read_text(encoding="utf-8").splitlines()
        ]
        self.assertEqual(len(logged), 1)
        self.assertEqual(logged[0][logged[0].index("--base") + 1], "no-agents")

    # -- agent declaration fallback (A0-3b) --------------------------------

    def test_agent_fallback_copies_and_excludes(self) -> None:
        # A slot from a base that predates A0-2 has no agent declaration:
        # OpenCode cannot start and fails in seconds with `Agent not found`.
        # oc.py copies the declaration and opencode.json from the repository
        # root and excludes the copies from the worker's changed set.
        task = self.tid()
        result = self.oc_run(task, base="no-agents")
        self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
        source = self.repo / ".opencode" / "agents"
        self.assertEqual(
            (self.slot1 / ".opencode" / "agents" / "crpg-implement.md").read_text(
                encoding="utf-8"
            ),
            (source / "crpg-implement.md").read_text(encoding="utf-8"),
        )
        self.assertTrue((self.slot1 / "opencode.json").is_file())
        listed = self.exclude_file(self.slot1).read_text(encoding="utf-8").splitlines()
        self.assertIn(".opencode/agents/crpg-implement.md", listed)
        self.assertIn("opencode.json", listed)
        # The verifier's scope check reads untracked files with git's own
        # exclude rules (verify_task.py collect_changes), so a clean run
        # reports no scope failure for the copies.
        status = sh(["git", "status", "--porcelain"], self.slot1).stdout
        self.assertNotIn(".opencode", status)
        self.assertNotIn("opencode.json", status)
        untracked = sh(
            ["git", "ls-files", "--others", "--exclude-standard"], self.slot1
        ).stdout
        self.assertNotIn(".opencode", untracked)
        self.assertNotIn("opencode.json", untracked)
        self.assertIn("scope: ok", result.stdout)

    def test_agent_fallback_never_overwrites_an_existing_file(self) -> None:
        slot = self.fresh_dir("fallback-slot")
        (slot / ".opencode" / "agents").mkdir(parents=True)
        sentinel = slot / ".opencode" / "agents" / "crpg-tests.md"
        sentinel.write_text("# sentinel\n", encoding="utf-8")
        copied = OCMOD.ensure_agent_files(self.repo, slot, "implement")
        self.assertIn(".opencode/agents/crpg-implement.md", copied)
        self.assertIn(".opencode/agents/crpg-grunt.md", copied)
        self.assertIn("opencode.json", copied)
        self.assertNotIn(".opencode/agents/crpg-tests.md", copied)
        self.assertEqual(sentinel.read_text(encoding="utf-8"), "# sentinel\n")

    def test_add_to_exclude_is_idempotent(self) -> None:
        slot = self.fresh_dir("exclude-slot")
        # ensure git worktree? exclude_file uses git rev-parse in slot.
        # Use an existing slot instead.
        slot = self.slot1
        OCMOD.add_to_exclude(slot, [".opencode/agents/crpg-implement.md", "opencode.json"])
        OCMOD.add_to_exclude(slot, [".opencode/agents/crpg-implement.md", "opencode.json"])
        listed = self.exclude_file(slot).read_text(encoding="utf-8").splitlines()
        self.assertEqual(listed.count(".opencode/agents/crpg-implement.md"), 1)
        self.assertEqual(listed.count("opencode.json"), 1)

    def test_agent_fallback_needs_the_declaration_in_the_slot_or_the_root(self) -> None:
        bare = self.fresh_dir("bare-root")
        slot = self.fresh_dir("bare-slot")
        with self.assertRaises(OCMOD.Exit) as caught:
            OCMOD.ensure_agent_files(bare, slot, "implement")
        self.assertEqual(caught.exception.code, 2)
        self.assertTrue(str(caught.exception.cause).startswith("no-agent"))

    def test_missing_agent_declaration_is_no_agent_before_opencode(self) -> None:
        bare = self.fresh_dir("no-agent-root")
        task = self.tid()
        call_log = self.fresh_file("calls.jsonl")
        result = self.oc_run(
            task,
            base="no-agents",
            env={"CRPG_REPO": str(bare), "FAKE_OC_LOG": str(call_log)},
        )
        self.assertEqual(result.returncode, 2, result.stdout)
        self.assertIn("no-agent", result.stdout + result.stderr)
        record = json.loads(
            (bare / ".agents" / "runs" / task / f"1-{MODEL.replace('/', '_')}"
             ".result.json").read_text(encoding="utf-8")
        )
        self.assertTrue(str(record["cause"]).startswith("no-agent"))
        # The failure is reported before OpenCode starts, not as a worker
        # error: the fake was never invoked.
        self.assertFalse(call_log.exists(), "opencode was started anyway")

    def test_agent_not_found_event_is_no_agent(self) -> None:
        task = self.tid()
        result = self.oc_run(task, env={"FAKE_OC_MODE": "no-agent"})
        self.assertEqual(result.returncode, 2, result.stdout)
        self.assertIn("no-agent", result.stdout + result.stderr)
        self.assertEqual(self.read_record(task)["cause"], "no-agent")
        self.assertEqual(
            OCMOD.classify_infra(
                '{"type":"error","error":{"message":"Agent not found: \\"x\\""}}'
            ),
            "no-agent",
        )

    # -- summary shape -----------------------------------------------

    def test_summary_truncates_to_40_lines(self) -> None:
        task = self.tid()
        result = self.oc_run(task, env={"FAKE_VERIFY_MODE": "fail-big"})
        self.assertEqual(result.returncode, 1)
        lines = result.stdout.splitlines()
        self.assertLessEqual(len(lines), 40)
        self.assertTrue(lines[0].startswith(f"FAIL {task}"))
        self.assertIn("truncated to 40 lines", lines[-1])
        second = self.tid()
        full = self.oc_run(
            second, env={"FAKE_VERIFY_MODE": "fail-big"}, extra=["--full"]
        )
        self.assertEqual(full.returncode, 1)
        self.assertGreater(len(full.stdout.splitlines()), 40)

    # -- setup failures --------------------------------------------------

    def test_no_brief_is_exit_2(self) -> None:
        empty = self.fresh_dir("empty-repo")
        task = self.tid()
        result = self.oc_run(task, brief=False, env={"CRPG_REPO": str(empty)})
        self.assertEqual(result.returncode, 2, result.stderr)
        self.assertIn("no-brief", result.stderr)

    def test_missing_brief_file_is_exit_2(self) -> None:
        task = self.tid()
        result = self.oc(
            "run", "--task", task, "--role", "implement", "--model", MODEL,
            "--base", "master", "--brief", str(self.case_dir / "absent.md"),
            env={},
        )
        self.assertEqual(result.returncode, 2, result.stderr)
        self.assertIn("no-brief", result.stderr)

    def test_fetch_failure_is_exit_2(self) -> None:
        task = self.tid()
        result = self.oc(
            "run", "--task", task, "--role", "implement", "--model", MODEL,
            "--base", "origin/master", "--brief", self.brief(), env={},
        )
        self.assertEqual(result.returncode, 2, result.stderr)
        self.assertIn("fetch", result.stderr)

    def test_dry_run_changes_nothing(self) -> None:
        task = self.tid()
        before = sh(
            ["git", "rev-parse", "--abbrev-ref", "HEAD"], self.slot1
        ).stdout.strip()
        result = self.oc(
            "run", "--task", task, "--role", "implement", "--model", MODEL,
            "--base", "master", "--brief", self.brief(), "--dry-run", env={},
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("command:", result.stdout)
        self.assertIn("opencode", result.stdout)
        # Default fake is 2.x: the dry-run command uses --standalone and
        # no --dir.
        self.assertIn("--standalone", result.stdout)
        self.assertNotIn("--dir", result.stdout)
        after = sh(
            ["git", "rev-parse", "--abbrev-ref", "HEAD"], self.slot1
        ).stdout.strip()
        self.assertEqual(before, after)
        self.assertFalse(self.log_path(task).exists())
        self.assertFalse(self.record_path(task).exists())

    def test_dry_run_prints_1x_command(self) -> None:
        task = self.tid()
        result = self.oc(
            "run", "--task", task, "--role", "implement", "--model", MODEL,
            "--base", "master", "--brief", self.brief(), "--dry-run",
            env={"FAKE_OC_VERSION": "opencode v1.18.35"},
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("command:", result.stdout)
        self.assertIn("--dir", result.stdout)
        self.assertNotIn("--standalone", result.stdout)
        self.assertFalse(self.log_path(task).exists())
        self.assertFalse(self.record_path(task).exists())

    def test_dry_run_prints_2x_hash_variant(self) -> None:
        task = self.tid()
        result = self.oc(
            "run", "--task", task, "--role", "implement", "--model", MODEL,
            "--base", "master", "--brief", self.brief(), "--dry-run",
            "--variant", "low", env={},
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(f"{MODEL}#low", result.stdout)
        self.assertNotIn("--variant", result.stdout)

    def test_attempt_continues_on_its_branch(self) -> None:
        task = self.tid()
        first = self.oc_run(task)
        self.assertEqual(first.returncode, 0, first.stderr)
        second = self.oc_run(task)
        self.assertEqual(second.returncode, 0, second.stderr)
        self.assertEqual(self.read_record(task, attempt=2)["attempt"], 2)
        sh(["git", "checkout", "-b", "detour"], self.slot1)
        try:
            # A slot left on another branch is put back on the task branch
            # instead of refusing and making the manager do it by hand.
            third = self.oc_run(task)
            self.assertEqual(third.returncode, 0, third.stdout + third.stderr)
            self.assertEqual(
                sh(["git", "rev-parse", "--abbrev-ref", "HEAD"],
                   self.slot1).stdout.strip(),
                f"task/{task}",
            )
            fresh = self.oc_run(task, extra=["--fresh"])
            self.assertEqual(fresh.returncode, 0, fresh.stderr + fresh.stdout)
        finally:
            sh(["git", "checkout", f"task/{task}"], self.slot1)
            sh(["git", "branch", "-D", "detour"], self.slot1)

    def test_attempt_counter_is_per_slug(self) -> None:
        # Two drafts of one task are separate pieces of work (T136b's
        # draft-a and draft-b), so each starts at attempt 1; rerunning one
        # slug is its attempt 2 (A0-4b).
        task = self.tid()
        safe = MODEL.replace("/", "_")
        first = self.oc_run(task, extra=["--slug", "draft-a"])
        self.assertEqual(first.returncode, 0, first.stdout + first.stderr)
        second = self.oc_run(task, extra=["--slug", "draft-b"])
        self.assertEqual(second.returncode, 0, second.stdout + second.stderr)
        third = self.oc_run(task, extra=["--slug", "draft-a"])
        self.assertEqual(third.returncode, 0, third.stdout + third.stderr)
        task_dir = self.repo / ".agents" / "runs" / task
        self.assertEqual(
            json.loads((task_dir / f"1-draft-a-{safe}.result.json")
                       .read_text(encoding="utf-8"))["attempt"],
            1,
        )
        self.assertEqual(
            json.loads((task_dir / f"1-draft-b-{safe}.result.json")
                       .read_text(encoding="utf-8"))["attempt"],
            1,
        )
        self.assertEqual(
            json.loads((task_dir / f"2-draft-a-{safe}.result.json")
                       .read_text(encoding="utf-8"))["attempt"],
            2,
        )
        self.assertFalse((task_dir / f"2-{safe}.result.json").exists())

    def test_attempt_creates_a_missing_task_branch(self) -> None:
        task = self.tid()
        first = self.oc_run(task)
        self.assertEqual(first.returncode, 0, first.stderr)
        # Drop the branch so attempt 2 finds no task branch in any slot.
        sh(["git", "checkout", "slot-1"], self.slot1)
        sh(["git", "branch", "-D", f"task/{task}"], self.slot1)
        second = self.oc_run(task, extra=["--slot", "2"])
        self.assertEqual(second.returncode, 0, second.stdout + second.stderr)
        self.assertEqual(
            sh(["git", "rev-parse", "--abbrev-ref", "HEAD"], self.slot2).stdout.strip(),
            f"task/{task}",
        )
        self.assertEqual(
            sh(["git", "rev-parse", f"task/{task}"], self.slot2).stdout,
            sh(["git", "rev-parse", "master"], self.slot2).stdout,
            "the new branch starts at --base",
        )

    def test_attempt_refuses_uncommitted_changes(self) -> None:
        # A dirty slot is another task's unmerged work (A0-12): the run is
        # refused before it can read those files, and the refusal leaves the
        # slot exactly as it found it, with no lock behind.
        task = self.tid()
        first = self.oc_run(task)
        self.assertEqual(first.returncode, 0, first.stderr)
        sh(["git", "checkout", "slot-1"], self.slot1)
        dirty = self.slot1 / "dirty-probe.txt"
        dirty.write_text("uncommitted\n", encoding="utf-8")
        head = sh(["git", "rev-parse", "HEAD"], self.slot1).stdout
        lock = self.repo / ".agents" / "slots" / "slot-1.lock"
        try:
            second = self.oc_run(task, extra=["--slot", "1"])
            self.assertEqual(second.returncode, 2, second.stdout)
            self.assertIn("slot-occupied", second.stdout + second.stderr)
            self.assertIn("uncommitted", second.stdout + second.stderr)
            self.assertTrue(
                str(self.read_record(task, attempt=2)["cause"]).startswith(
                    "slot-occupied"
                )
            )
            self.assertEqual(
                sh(["git", "rev-parse", "--abbrev-ref", "HEAD"],
                   self.slot1).stdout.strip(),
                "slot-1",
            )
            self.assertEqual(dirty.read_text(encoding="utf-8"), "uncommitted\n")
            self.assertEqual(
                sh(["git", "rev-parse", "HEAD"], self.slot1).stdout, head
            )
            self.assertFalse(lock.exists(), "a refused slot was left locked")
        finally:
            dirty.unlink()

    # -- slot occupancy: another task's work (A0-12) ----------------------

    def foreign_branch(self, slot: Path, branch: str, name: str) -> str:
        """Leave `slot` on `branch` with one committed change master lacks."""
        sh(["git", "checkout", "-B", branch, "master"], slot)
        (slot / name).write_text(f"{branch} work\n", encoding="utf-8")
        sh(["git", "add", name], slot)
        sh(
            ["git", "-c", "commit.gpgsign=false", "commit", "-m", f"{branch} work"],
            slot,
        )
        return sh(["git", "rev-parse", "HEAD"], slot).stdout.strip()

    def slot_used_by(self, call_log: Path) -> Path:
        """The slot the fake worker was started in."""
        calls = [
            json.loads(line)
            for line in call_log.read_text(encoding="utf-8").splitlines()
            if line.strip()
        ]
        return Path(calls[0]["cwd"]).resolve()

    def test_auto_pick_skips_a_foreign_branch_ahead_of_base(self) -> None:
        # A branch whose commit no other ref holds is another task's unmerged
        # work, even with a clean tree: the run must not adopt it as base.
        head = self.foreign_branch(self.slot1, "task/other-work", "other.txt")
        task = self.tid()
        call_log = self.fresh_file("calls.jsonl")
        result = self.oc_run(task, env={"FAKE_OC_LOG": str(call_log)})
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(self.slot_used_by(call_log), self.slot2)
        self.assertEqual(
            sh(["git", "rev-parse", "--abbrev-ref", "HEAD"],
               self.slot1).stdout.strip(),
            "task/other-work",
        )
        self.assertEqual(sh(["git", "rev-parse", "HEAD"], self.slot1).stdout.strip(),
                         head)
        self.assertTrue((self.slot1 / "other.txt").is_file())
        self.assertEqual(self.status_paths(self.slot1), "")

    def test_auto_pick_skips_a_dirty_slot(self) -> None:
        wip = self.slot1 / "wip.txt"
        wip.write_text("another task's work\n", encoding="utf-8")
        task = self.tid()
        call_log = self.fresh_file("calls.jsonl")
        result = self.oc_run(task, env={"FAKE_OC_LOG": str(call_log)})
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(self.slot_used_by(call_log), self.slot2)
        self.assertEqual(wip.read_text(encoding="utf-8"), "another task's work\n")
        self.assertEqual(
            sh(["git", "rev-parse", "--abbrev-ref", "HEAD"],
               self.slot1).stdout.strip(),
            "slot-1",
        )
        # Nothing was prepared in the dirty slot, not even a brief.
        self.assertFalse((self.slot1 / "BRIEF.md").exists())

    def test_detached_head_counts_only_the_dirty_test(self) -> None:
        # A detached HEAD has no branch whose commits could be another task's
        # unmerged work, so only uncommitted changes make it occupied.
        sh(["git", "checkout", "--detach", "master"], self.slot1)
        (self.slot1 / "detached.txt").write_text("detached work\n",
                                                 encoding="utf-8")
        sh(["git", "add", "detached.txt"], self.slot1)
        sh(["git", "-c", "commit.gpgsign=false", "commit", "-m", "detached"],
           self.slot1)
        task = self.tid()
        call_log = self.fresh_file("calls.jsonl")
        result = self.oc_run(task, env={"FAKE_OC_LOG": str(call_log)})
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(self.slot_used_by(call_log), self.slot1)

    def test_explicit_slot_on_occupied_slot_is_refused(self) -> None:
        head = self.foreign_branch(self.slot1, "task/other-work", "other.txt")
        task = self.tid()
        result = self.oc_run(task, extra=["--slot", "1"])
        self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
        text = result.stdout + result.stderr
        self.assertIn("slot-occupied: slot-1 holds task/other-work", text)
        self.assertIn("1 commits not on master", text)
        record = self.read_record(task)
        self.assertTrue(str(record["cause"]).startswith("slot-occupied"))
        self.assertFalse(OCMOD.is_failure(record), "charged to the model")
        # The refusal touched nothing: same branch and commit, no brief, and
        # the lock it took to look was released again.
        self.assertEqual(sh(["git", "rev-parse", "HEAD"], self.slot1).stdout.strip(),
                         head)
        self.assertEqual(
            sh(["git", "rev-parse", "--abbrev-ref", "HEAD"],
               self.slot1).stdout.strip(),
            "task/other-work",
        )
        self.assertTrue((self.slot1 / "other.txt").is_file())
        self.assertFalse((self.slot1 / "BRIEF.md").exists())
        self.assertFalse((self.repo / ".agents" / "slots" / "slot-1.lock").exists())

    def test_retry_on_its_own_task_branch_is_allowed(self) -> None:
        # An attempt 2 finds its own task branch ahead of base: that is this
        # run's work, not a neighbour's, so the slot is reused.
        task = self.tid()
        first = self.oc_run(task, extra=["--slot", "1"])
        self.assertEqual(first.returncode, 0, first.stdout + first.stderr)
        (self.slot1 / "own.txt").write_text("own work\n", encoding="utf-8")
        sh(["git", "add", "own.txt"], self.slot1)
        sh(["git", "-c", "commit.gpgsign=false", "commit", "-m", "own work"],
           self.slot1)
        self.assertEqual(self.commits_since_base(self.slot1), 1)
        second = self.oc_run(task, extra=["--slot", "1"])
        self.assertEqual(second.returncode, 0, second.stdout + second.stderr)
        self.assertEqual(self.read_record(task, attempt=2)["attempt"], 2)
        self.assertEqual(
            sh(["git", "rev-parse", "--abbrev-ref", "HEAD"],
               self.slot1).stdout.strip(),
            f"task/{task}",
        )
        self.assertEqual(self.commits_since_base(self.slot1), 1)

    def test_stacked_base_on_the_same_branch_is_allowed(self) -> None:
        # A stacked run: the slot already sits on the run's --base branch,
        # clean and with no commit beyond it, so nothing is in the way.
        base_head = self.foreign_branch(self.slot1, "task/T136c-bridge",
                                        "bridge.txt")
        task = self.tid()
        result = self.oc_run(task, base="task/T136c-bridge",
                             extra=["--slot", "1"])
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(
            sh(["git", "rev-parse", "--abbrev-ref", "HEAD"],
               self.slot1).stdout.strip(),
            f"task/{task}",
        )
        self.assertEqual(
            sh(["git", "rev-parse", f"task/{task}"], self.slot1).stdout.strip(),
            base_head,
            "the run starts at its stacked base",
        )
        self.assertEqual(self.commits_since_base(self.slot1, "task/T136c-bridge"),
                         0)

    def test_all_slots_occupied_names_each_branch(self) -> None:
        self.foreign_branch(self.slot1, "task/other-one", "one.txt")
        self.foreign_branch(self.slot2, "task/other-two", "two.txt")
        task = self.tid()
        result = self.oc_run(task)
        self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
        text = result.stdout + result.stderr
        self.assertIn("slot-busy", text)
        self.assertIn("task/other-one", text)
        self.assertIn("task/other-two", text)
        record = self.read_record(task)
        self.assertEqual(record["verdict"], "INFRA")
        self.assertTrue(str(record["cause"]).startswith("slot-busy"))
        for name in ("slot-1.lock", "slot-2.lock"):
            self.assertFalse((self.repo / ".agents" / "slots" / name).exists())

    def test_wave_refuses_when_every_slot_is_occupied(self) -> None:
        self.foreign_branch(self.slot1, "task/other-one", "one.txt")
        self.foreign_branch(self.slot2, "task/other-two", "two.txt")
        task = self.tid()
        call_log = self.fresh_file("calls.jsonl")
        result = self.oc(
            "wave", f"{task}:implement:{MODEL}", "--base", "master",
            env={"FAKE_OC_LOG": str(call_log)}, timeout=120,
        )
        self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
        text = result.stdout + result.stderr
        self.assertIn("slot-busy", text)
        self.assertIn("task/other-one", text)
        self.assertIn("task/other-two", text)
        self.assertFalse(call_log.exists(), "a worker started without a free slot")
        self.assertFalse(self.record_path(task).exists())

    # -- idle watchdog ---------------------------------------------------

    def test_idle_silence_is_hang(self) -> None:
        task = self.tid()
        result = self.oc_run(task, extra=["--idle", "1"],
                             env={"FAKE_OC_MODE": "silent"}, timeout=60)
        self.assertEqual(result.returncode, 2, result.stdout)
        self.assertIn("hang", result.stdout + result.stderr)
        record = self.read_record(task)
        self.assertEqual(record["cause"], "hang")
        self.assertEqual(record["failed_checks"], ["hang"])

    def test_idle_error_log_maps_to_no_agent(self) -> None:
        task = self.tid()
        oc_log = self.fresh_file("opencode-noagent.log")
        # Pre-create an empty log so run_worker's baseline size is 0; a
        # background thread appends the Agent-not-found line before the idle
        # watchdog fires, mirroring a real OpenCode log growing mid-run.
        oc_log.write_text("", encoding="utf-8")

        def append() -> None:
            time.sleep(0.3)
            with oc_log.open("a", encoding="utf-8") as handle:
                handle.write(
                    'timestamp=2026-10-16T10:27:27.101Z level=ERROR message="stream error" '
                    'providerID=opencode-go modelID=fake-a session.id=ses_fake '
                    'agent=crpg-implement error.error="Agent not found: \\"crpg-implement\\""\n'
                )

        thread = threading.Thread(target=append, daemon=True)
        thread.start()
        result = self.oc_run(
            task,
            extra=["--idle", "1"],
            env={
                "FAKE_OC_MODE": "silent",
                "CRPG_OPENCODE_LOG": str(oc_log),
            },
            timeout=60,
        )
        thread.join(timeout=5)
        self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
        self.assertIn("no-agent", result.stdout + result.stderr)
        self.assertEqual(self.read_record(task)["cause"], "no-agent")

    def test_idle_error_log_maps_to_quota(self) -> None:
        task = self.tid()
        oc_log = self.fresh_file("opencode.log")
        result = self.oc_run(
            task,
            extra=["--idle", "1"],
            env={
                "FAKE_OC_MODE": "error-log",
                "CRPG_OPENCODE_LOG": str(oc_log),
            },
            timeout=60,
        )
        self.assertEqual(result.returncode, 2, result.stdout)
        self.assertIn("quota", result.stdout + result.stderr)
        self.assertEqual(self.read_record(task)["cause"], "quota")
        self.assertIn("level=ERROR", oc_log.read_text(encoding="utf-8"))

    # -- wave ------------------------------------------------------------

    def test_wave_reports_worst_exit(self) -> None:
        env = {"FAKE_VERIFY_FAIL_TASKS": "T-wave-fail"}
        result = self.oc(
            "wave", "T-wave-pass:implement:go/fake-a",
            "T-wave-fail:implement:go/fake-b",
            "--base", "master", env=env, timeout=120,
        )
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("T-wave-pass", result.stdout)
        self.assertIn("T-wave-fail", result.stdout)
        self.assertIn("clippy", result.stdout)
        self.assertIn("next:", result.stdout)
        self.assertLessEqual(len(result.stdout.splitlines()), 40)
        self.assertEqual(self.read_record("T-wave-pass", "go/fake-a")["verdict"], "PASS")
        self.assertEqual(self.read_record("T-wave-fail", "go/fake-b")["verdict"], "FAIL")

    # -- attempt-3 review fixes ----------------------------------------

    def test_markers_in_tool_output_keep_verdict(self) -> None:
        # A clean run ending in stop whose tool output and report text
        # contain every infra-table marker keeps its normal verdict.
        task = self.tid()
        result = self.oc_run(task, env={"FAKE_OC_MODE": "markers"})
        self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
        self.assertTrue(result.stdout.splitlines()[0].startswith(f"PASS {task}"))
        record = self.read_record(task)
        self.assertEqual(record["verdict"], "PASS")
        self.assertIsNone(record["cause"])

    def test_dry_run_validates_brief(self) -> None:
        task = self.tid()
        missing = str(self.case_dir / "absent.md")
        result = self.oc(
            "run", "--task", task, "--role", "implement", "--model", MODEL,
            "--base", "master", "--brief", missing, "--dry-run", env={},
        )
        self.assertEqual(result.returncode, 2, result.stdout)
        self.assertIn("no-brief", result.stderr)
        self.assertFalse(self.record_path(task).exists())
        empty = self.fresh_dir("empty-repo")
        generated = self.tid()
        result = self.oc(
            "run", "--task", generated, "--role", "implement",
            "--model", MODEL, "--base", "master", "--dry-run",
            env={"CRPG_REPO": str(empty)},
        )
        self.assertEqual(result.returncode, 2, result.stdout)
        self.assertIn("no-brief", result.stderr)

    def test_slot_busy_writes_record(self) -> None:
        sleeper = subprocess.Popen(
            [sys.executable, "-c", "import time; time.sleep(60)"],
            stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        try:
            lock_dir = self.repo / ".agents" / "slots"
            lock_dir.mkdir(parents=True, exist_ok=True)
            for name in ("slot-1.lock", "slot-2.lock"):
                (lock_dir / name).write_text(f"{sleeper.pid}\n", encoding="utf-8")
            self.addCleanup(shutil.rmtree, lock_dir, True)
            task = self.tid()
            result = self.oc_run(task)
            self.assertEqual(result.returncode, 2, result.stdout)
            self.assertIn("slot-busy", result.stdout + result.stderr)
            record = self.read_record(task)
            self.assertEqual(record["verdict"], "INFRA")
            self.assertTrue(str(record["cause"]).startswith("slot-busy"))
        finally:
            try:
                sleeper.kill()
            except OSError:
                pass
            sleeper.wait(timeout=30)

    def test_no_exe_writes_record(self) -> None:
        bare = self.fresh_dir("bare-path")
        task = self.tid()
        result = self.oc_run(
            task,
            env={"CRPG_OPENCODE": None, "PATH": str(bare),
                 "APPDATA": str(bare)},
        )
        self.assertEqual(result.returncode, 2, result.stdout)
        self.assertIn("no-exe", result.stdout + result.stderr)
        record = self.read_record(task)
        self.assertEqual(record["verdict"], "INFRA")
        self.assertTrue(str(record["cause"]).startswith("no-exe"))

    def test_ask_no_exe_writes_record(self) -> None:
        # ask resolves the executable inside its try block, so a missing
        # OpenCode still leaves a result record (attempt-5 finding 3).
        bare = self.fresh_dir("bare-path")
        result = self.oc(
            "ask", "--role", "scout", "--model", MODEL,
            "--timeout", "60", "--idle", "10", "who are you?",
            env={"CRPG_OPENCODE": None, "PATH": str(bare),
                 "APPDATA": str(bare)},
        )
        self.assertEqual(result.returncode, 2, result.stdout)
        self.assertIn("no-exe", result.stdout + result.stderr)
        # Other ask tests share this repo, so take the newest ask record
        # rather than assuming attempt 1.
        runs = list((self.repo / ".agents" / "runs" / "ask").glob("*.result.json"))
        self.assertTrue(runs)
        path = max(runs, key=lambda p: int(p.name.split("-", 1)[0]))
        record = json.loads(path.read_text(encoding="utf-8"))
        self.assertEqual(record["verdict"], "INFRA")
        self.assertTrue(str(record["cause"]).startswith("no-exe"))

    def test_git_clean_failure_is_branch(self) -> None:
        task = self.tid()
        # BRIEF.md is the one path the A0-12 occupancy check ignores (the
        # dispatcher writes it itself), so a held-open brief still reaches
        # `git clean` and proves its failure is the `branch` cause.
        held = self.slot1 / "BRIEF.md"
        handle = open(held, "w", encoding="utf-8")
        handle.write("held open so git clean cannot remove it\n")
        handle.flush()

        def release() -> None:
            handle.close()
            if held.exists():
                held.unlink()

        self.addCleanup(release)
        result = self.oc_run(task)
        self.assertEqual(result.returncode, 2, result.stdout)
        self.assertIn("branch", result.stdout + result.stderr)
        self.assertTrue(str(self.read_record(task)["cause"]).startswith("branch"))

    def test_ask_writes_result_record(self) -> None:
        result = self.oc(
            "ask", "--role", "scout", "--model", MODEL,
            "--timeout", "60", "--idle", "10", "where is the lint?",
        )
        self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
        path = self.repo / ".agents" / "runs" / "ask" / "1-go_fake-a.result.json"
        self.assertTrue(path.is_file())
        record = json.loads(path.read_text(encoding="utf-8"))
        self.assertEqual(record["verdict"], "PASS")
        self.assertEqual(record["task"], "ask")

    def test_ask_next_line_reads_answer(self) -> None:
        result = self.oc(
            "ask", "--role", "scout", "--model", MODEL,
            "--timeout", "60", "--idle", "10", "where is the lint?",
        )
        self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
        self.assertEqual(result.stdout.splitlines()[-1], "next: read the answer")

    def test_review_next_line_acts_on_verdict(self) -> None:
        task = self.tid()
        result = self.oc_run(task, role="review", extra=["--json"])
        self.assertEqual(result.returncode, 0, result.stderr)
        payload = json.loads(result.stdout)
        self.assertTrue(payload["summary"].splitlines()[-1].startswith(
            "next: act on the verdict"))

    def test_grunt_role_skips_gates(self) -> None:
        task = self.tid()
        verify_log = self.fresh_file("verify.jsonl")
        result = self.oc_run(
            task, role="grunt",
            env={"FAKE_OC_MODE": "grunt-edit", "FAKE_VERIFY_LOG": str(verify_log)},
        )
        self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
        logged = [
            json.loads(line)
            for line in verify_log.read_text(encoding="utf-8").splitlines()
        ]
        self.assertEqual(len(logged), 1)
        self.assertIn("--skip", logged[0])
        self.assertIn("gates", logged[0])

    def test_summary_uses_utf8_middle_dot(self) -> None:
        merged = dict(os.environ)
        merged.update(self.base_env)
        merged.pop("PYTHONIOENCODING", None)
        proc = subprocess.run(
            [sys.executable, str(OC), "ask", "--role", "scout",
             "--model", MODEL, "--timeout", "60", "--idle", "10",
             "where is the lint?"],
            cwd=self.repo,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            timeout=90,
            env=merged,
        )
        self.assertEqual(proc.returncode, 0, proc.stdout[-500:])
        self.assertIn("·".encode("utf-8"), proc.stdout)

    def test_renamed_model_is_refused(self) -> None:
        task = self.tid()
        result = self.oc_run(task, model="opencode/longcat-2.5-preview-free")
        self.assertEqual(result.returncode, 2, result.stdout)
        self.assertIn("usage", result.stdout + result.stderr)
        self.assertIn("opencode-go/longcat-2.5-preview-free",
                      result.stdout + result.stderr)
        self.assertFalse(self.record_path(
            task, model="opencode/longcat-2.5-preview-free").exists())
        asked = self.oc(
            "ask", "--role", "scout", "--model", "opencode/longcat-2.5-preview-free",
            "--timeout", "60", "a question",
        )
        self.assertEqual(asked.returncode, 2, asked.stdout)
        self.assertIn("opencode-go/longcat-2.5-preview-free",
                      asked.stdout + asked.stderr)

    def test_cat_jam_and_zen_beam_name_go_beam(self) -> None:
        # Cat Jam is Beam Free after the 2.x upgrade; Beam runs on Go only.
        for model in ("opencode/cat-jam-free", "opencode-go/cat-jam-free",
                      "opencode/beam-free"):
            with self.subTest(model=model):
                self.count += 1
                task = self.tid()
                result = self.oc_run(task, model=model)
                self.assertEqual(result.returncode, 2, result.stdout)
                self.assertIn("usage", result.stdout + result.stderr)
                self.assertIn("moved to", result.stdout + result.stderr)
                self.assertIn("opencode-go/beam-free",
                              result.stdout + result.stderr)
                self.assertFalse(self.record_path(task, model=model).exists())

    def test_filed_brief_is_used(self) -> None:
        task = self.tid()
        filed_dir = self.repo / ".agents" / "briefs"
        filed_dir.mkdir(parents=True, exist_ok=True)
        filed = filed_dir / f"{task}.md"
        filed.write_text("# Filed brief\n\nUse this text.\n", encoding="utf-8")
        result = self.oc_run(task, brief=False)
        self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
        self.assertEqual(
            (self.slot1 / "BRIEF.md").read_text(encoding="utf-8"),
            "# Filed brief\n\nUse this text.\n",
        )

    def test_placeholder_brief_is_refused(self) -> None:
        task = self.tid()
        path = self.case_dir / "placeholder.md"
        path.write_text("# Brief\n\n<MANAGER: fill in the goal>\n", encoding="utf-8")
        result = self.oc(
            "run", "--task", task, "--role", "implement", "--model", MODEL,
            "--base", "master", "--brief", str(path), env={},
        )
        self.assertEqual(result.returncode, 2, result.stdout)
        self.assertIn("no-brief", result.stdout + result.stderr)
        self.assertIn("placeholder", result.stdout + result.stderr)

    def test_wave_same_crate_is_refused(self) -> None:
        filed_dir = self.repo / ".agents" / "briefs"
        filed_dir.mkdir(parents=True, exist_ok=True)
        first = self.tid()
        second = self.tid()
        for task in (first, second):
            (filed_dir / f"{task}.md").write_text(
                "# Brief\n\n## Files you own\n\n"
                "- `crates/same-crate/src/lib.rs`\n",
                encoding="utf-8",
            )
        result = self.oc(
            "wave", f"{first}:implement:{MODEL}", f"{second}:implement:{MODEL}",
            "--base", "master", timeout=120,
        )
        self.assertEqual(result.returncode, 2, result.stdout)
        self.assertIn("usage", result.stdout + result.stderr)
        self.assertIn("crates/same-crate", result.stdout + result.stderr)

    def test_wave_distinct_crates_pass(self) -> None:
        filed_dir = self.repo / ".agents" / "briefs"
        filed_dir.mkdir(parents=True, exist_ok=True)
        specs = []
        for crate in ("wave-crate-a", "wave-crate-b"):
            task = self.tid()
            (filed_dir / f"{task}.md").write_text(
                "# Brief\n\n## Files you own\n\n"
                f"- `crates/{crate}/src/lib.rs`\n",
                encoding="utf-8",
            )
            specs.append(f"{task}:implement:{MODEL}")
        result = self.oc("wave", *specs, "--base", "master", timeout=120)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        for spec in specs:
            task = spec.split(":")[0]
            self.assertEqual(self.read_record(task)["verdict"], "PASS")

    def test_wave_serializes_one_free_model(self) -> None:
        conc = self.fresh_file("conc.txt")
        first = self.tid()
        second = self.tid()
        model = "opencode/fake-free"
        result = self.oc(
            "wave", f"{first}:implement:{model}", f"{second}:implement:{model}",
            "--base", "master",
            env={"FAKE_OC_MODE": "slow", "FAKE_OC_SLOW": "1.0",
                 "FAKE_OC_CONC": str(conc)},
            timeout=120,
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(self.read_record(first, model)["verdict"], "PASS")
        self.assertEqual(self.read_record(second, model)["verdict"], "PASS")
        events = []
        for line in conc.read_text(encoding="utf-8").splitlines():
            kind, _, stamp = line.partition(" ")
            events.append((float(stamp), kind))
        events.sort()
        live = 0
        peak = 0
        for _, kind in events:
            live += 1 if kind == "start" else -1
            peak = max(peak, live)
        self.assertEqual(len(events), 4)
        self.assertEqual(peak, 1)


class RecordStatsCase(unittest.TestCase):
    """record/stats against a bare directory: no git, no worker."""

    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory(prefix="oc stats ")
        self.addCleanup(self.tmp.cleanup)
        self.repo = Path(self.tmp.name)
        self.env = {"CRPG_REPO": str(self.repo)}

    def oc(self, *argv: str, env: dict | None = None):
        merged = dict(os.environ)
        merged.update(self.env)
        merged.update(env or {})
        return subprocess.run(
            [sys.executable, str(OC), *argv],
            cwd=self.repo,
            capture_output=True,
            text=True,
            timeout=60,
            env=merged,
        )

    def write_record(self, task, model, role, attempt, verdict, started,
                     cause=None, outcomes=(), steps=4, elapsed=30.0, cost=0.02):
        path = (
            self.repo / ".agents" / "runs" / task
            / f"{attempt}-{model.replace('/', '_')}.result.json"
        )
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps({
            "version": 1, "task": task, "role": role, "model": model,
            "attempt": attempt, "started_utc": started, "elapsed_s": elapsed,
            "verdict": verdict, "cause": cause, "failed_checks": [],
            "steps": steps,
            "tokens": {"input": 100, "cached": 50, "output": 20},
            "cost": cost, "retries": [],
            "outcomes": [
                {"outcome": name, "note": "", "utc": started}
                for name in outcomes
            ],
        }), encoding="utf-8")
        return path

    # -- record ----------------------------------------------------------

    def test_record_appends_to_existing(self) -> None:
        path = self.write_record(
            "T-rec", "go/m", "implement", 1, "FAIL",
            "2026-10-10T00:00:00Z",
        )
        result = self.oc(
            "record", "--task", "T-rec", "--role", "implement",
            "--model", "go/m", "--outcome", "review-fix",
            "--note", "clippy findings",
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        payload = json.loads(path.read_text(encoding="utf-8"))
        self.assertEqual(payload["verdict"], "FAIL")
        self.assertEqual(len(payload["outcomes"]), 1)
        self.assertEqual(payload["outcomes"][0]["outcome"], "review-fix")

    def test_record_creates_minimal_for_unknown(self) -> None:
        merged = self.oc(
            "record", "--task", "T-new", "--role", "implement",
            "--model", "go/m", "--outcome", "merged",
        )
        self.assertEqual(merged.returncode, 0, merged.stderr)
        payload = json.loads(
            (self.repo / ".agents" / "runs" / "T-new"
             / "1-go_m.result.json").read_text(encoding="utf-8")
        )
        self.assertEqual(payload["verdict"], "PASS")
        rejected = self.oc(
            "record", "--task", "T-new2", "--role", "implement",
            "--model", "go/m", "--outcome", "review-reject",
        )
        self.assertEqual(rejected.returncode, 0, rejected.stderr)
        payload = json.loads(
            (self.repo / ".agents" / "runs" / "T-new2"
             / "1-go_m.result.json").read_text(encoding="utf-8")
        )
        self.assertEqual(payload["verdict"], "FAIL")

    def test_recorded_review_fix_is_not_a_failure(self) -> None:
        # Two synthesized review-fix records alone must not mark DEMOTE?;
        # two review-reject ones must.
        for day in (1, 2):
            self.oc(
                "record", "--task", f"T-fix-{day}", "--role", "implement",
                "--model", "go/fix", "--outcome", "review-fix",
            )
            self.oc(
                "record", "--task", f"T-rej-{day}", "--role", "implement",
                "--model", "go/rej", "--outcome", "review-reject",
            )
        rows = self.rows()
        by_key = {(row["model"], row["role"]): row for row in rows}
        self.assertFalse(by_key[("go/fix", "implement")]["demote"])
        self.assertTrue(by_key[("go/rej", "implement")]["demote"])

    def test_recorded_minimal_carries_recorded_cause(self) -> None:
        self.oc(
            "record", "--task", "T-min", "--role", "implement",
            "--model", "go/m", "--outcome", "review-fix",
        )
        payload = json.loads(
            (self.repo / ".agents" / "runs" / "T-min"
             / "1-go_m.result.json").read_text(encoding="utf-8")
        )
        self.assertEqual(payload["cause"], "recorded")
        self.assertFalse(OCMOD.is_failure(payload))

    # -- stats -----------------------------------------------------------

    def build_table(self) -> None:
        for day in range(1, 6):
            self.write_record(
                f"S-best-{day}", "m-best", "implement", 1, "PASS",
                f"2026-10-{day:02d}T00:00:00Z",
            )
        verdicts = ["PASS", "PASS", "PASS", "PASS", "FAIL"]
        for day, verdict in enumerate(verdicts, start=1):
            self.write_record(
                f"S-mid-{day}", "m-mid", "implement", 1, verdict,
                f"2026-10-{day:02d}T00:00:00Z",
            )
        for day in range(1, 5):
            self.write_record(
                f"S-few-{day}", "m-few", "implement", 1, "PASS",
                f"2026-10-{day:02d}T00:00:00Z",
            )
        for day, verdict in enumerate(["FAIL", "FAIL", "PASS", "PASS"], start=1):
            self.write_record(
                f"S-dem-{day}", "m-dem", "tests", 1, verdict,
                f"2026-10-{day:02d}T00:00:00Z",
            )
        for day, verdict in enumerate(["FAIL", "PASS", "PASS", "PASS"], start=1):
            self.write_record(
                f"S-ok-{day}", "m-ok", "tests", 1, verdict,
                f"2026-10-{day:02d}T00:00:00Z",
            )
        for day, outcomes in enumerate(
            [["review-reject"], [], ["review-reject"], []], start=1
        ):
            self.write_record(
                f"S-rev-{day}", "m-rev", "review", 1, "PASS",
                f"2026-10-{day:02d}T00:00:00Z", outcomes=outcomes,
            )
        self.write_record(
            "S-fix-1", "m-fix", "implement", 1, "PASS",
            "2026-10-01T00:00:00Z", outcomes=["review-fix"],
        )
        self.write_record(
            "S-fix-2", "m-fix", "implement", 1, "PASS",
            "2026-10-02T00:00:00Z",
        )
        self.write_record(
            "S-quota-1", "m-quota", "implement", 1, "INFRA",
            "2026-10-01T00:00:00Z", cause="quota",
        )
        self.write_record(
            "S-quota-2", "m-quota", "implement", 1, "PASS",
            "2026-10-02T00:00:00Z",
        )

    def rows(self, *argv: str) -> list[dict]:
        result = self.oc("stats", "--json", *argv)
        self.assertEqual(result.returncode, 0, result.stderr)
        return json.loads(result.stdout)["rows"]

    def test_stats_ranking_and_marks(self) -> None:
        self.build_table()
        rows = self.rows()
        by_key = {(row["model"], row["role"]): row for row in rows}
        self.assertEqual(
            [row["model"] for row in rows[:3]], ["m-best", "m-few", "m-quota"]
        )
        self.assertTrue(by_key[("m-best", "implement")]["promote"])
        self.assertFalse(by_key[("m-mid", "implement")]["promote"])
        self.assertFalse(by_key[("m-few", "implement")]["promote"])
        self.assertTrue(by_key[("m-dem", "tests")]["demote"])
        self.assertFalse(by_key[("m-ok", "tests")]["demote"])
        self.assertTrue(by_key[("m-rev", "review")]["demote"])
        fix = by_key[("m-fix", "implement")]
        self.assertEqual(
            (fix["first_attempt_passes"], fix["first_attempt_eligible"]), (1, 2)
        )
        quota = by_key[("m-quota", "implement")]
        self.assertEqual(
            (quota["first_attempt_passes"], quota["first_attempt_eligible"]),
            (1, 1),
        )
        text = self.oc("stats")
        self.assertEqual(text.returncode, 0, text.stderr)
        self.assertIn("DEMOTE?", text.stdout)
        self.assertIn("PROMOTE?", text.stdout)
        self.assertLessEqual(len(text.stdout.splitlines()), 40)

    def test_stats_filters(self) -> None:
        self.build_table()
        tests_only = self.rows("--role", "tests")
        self.assertTrue(tests_only)
        self.assertTrue(all(row["role"] == "tests" for row in tests_only))
        recent = self.rows("--since", "2026-10-05")
        best = next(row for row in recent if row["model"] == "m-best")
        self.assertEqual(best["runs"], 1)
        bad = self.oc("stats", "--since", "October")
        self.assertEqual(bad.returncode, 2)

    def test_stats_does_not_count_a_verify_timeout(self) -> None:
        self.write_record(
            "S-vt-1", "m-vt", "implement", 1, "INFRA",
            "2026-10-16T00:00:00Z", cause="verify-timeout", elapsed=None,
        )
        rows = self.rows()
        row = next(row for row in rows if row["model"] == "m-vt")
        self.assertEqual(row["first_attempt_eligible"], 0)
        self.assertEqual(row["first_attempt_passes"], 0)
        self.assertEqual(row["infra"], {"verify-timeout": 1})
        self.assertFalse(row["demote"])

    def test_stats_treats_null_numbers_as_absent(self) -> None:
        # A record can carry `"elapsed_s": null` (an interrupted run); stats
        # leaves it out of averages and sums instead of crashing on it.
        self.write_record(
            "S-null-1", "m-null", "implement", 1, "PASS",
            "2026-10-16T00:00:00Z", steps=None, elapsed=None, cost=None,
        )
        text = self.oc("stats", "--since", "2026-10-16")
        self.assertEqual(text.returncode, 0, text.stderr + text.stdout)
        self.assertIn("model x role", text.stdout)
        row = next(row for row in self.rows() if row["model"] == "m-null")
        self.assertEqual(row["median_steps"], 0)
        self.assertEqual(row["median_elapsed_s"], 0)
        self.assertEqual(row["total_cost"], 0)


class AskDiffCase(unittest.TestCase):
    """`ask --diff/--base` in a fresh temporary git repository (A0-4c)."""

    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory(prefix="oc ask diff ")
        self.addCleanup(self.tmp.cleanup)
        root = Path(self.tmp.name)
        self.repo = root / "repo"
        self.repo.mkdir()
        self.prompt_file = root / "prompt.txt"
        self.calls = root / "calls.jsonl"
        self.starts = root / "starts.txt"
        sh(["git", "init", "-b", "master", str(self.repo)], root)
        sh(["git", "config", "user.email", "test@example.invalid"], self.repo)
        sh(["git", "config", "user.name", "oc test"], self.repo)
        (self.repo / "README.md").write_text(
            "# ask diff repo\n", encoding="utf-8")
        sh(["git", "add", "README.md"], self.repo)
        sh(["git", "-c", "commit.gpgsign=false", "commit", "-m", "init"],
           self.repo)
        self.env = {
            "CRPG_REPO": str(self.repo),
            "CRPG_OPENCODE": str(FAKE_OPENCODE),
            "CRPG_OC_START_GAP": "0",
            "CRPG_OC_LOCK_WAIT": "0.05",
            "FAKE_OC_MODE": "prompt-file,ok",
            "FAKE_OC_LOG": str(self.calls),
            "FAKE_OC_STARTS": str(self.starts),
            "FAKE_OC_PROMPT_FILE": str(self.prompt_file),
        }

    # -- helpers -----------------------------------------------------

    def branch(self, name: str, text: str | None = None) -> None:
        """Create a branch off master, with one committed change if asked."""
        sh(["git", "checkout", "-b", name], self.repo)
        if text is not None:
            (self.repo / "CHANGE.md").write_text(text, encoding="utf-8")
            sh(["git", "add", "CHANGE.md"], self.repo)
            sh(["git", "-c", "commit.gpgsign=false", "commit", "-m", name],
               self.repo)
        sh(["git", "checkout", "master"], self.repo)

    def ask(self, *extra: str, question: str = "What changed?"):
        merged = dict(os.environ)
        merged.update(self.env)
        return subprocess.run(
            [sys.executable, str(OC), "ask", "--role", "scout",
             "--model", MODEL, "--timeout", "60", "--idle", "10",
             *extra, question],
            cwd=self.repo, capture_output=True, text=True, timeout=90,
            env=merged,
        )

    def runs_dir(self) -> Path:
        return self.repo / ".agents" / "runs" / "ask"

    def record_path(self, attempt: int = 1) -> Path:
        return self.runs_dir() / f"{attempt}-{MODEL.replace('/', '_')}.result.json"

    def diff_path(self, attempt: int = 1) -> Path:
        return self.runs_dir() / f"{attempt}-{MODEL.replace('/', '_')}.diff"

    def read_record(self, attempt: int = 1) -> dict:
        return json.loads(self.record_path(attempt).read_text())

    # -- behaviours --------------------------------------------------

    def test_diff_file_and_question_sentence(self) -> None:
        self.branch("changed", "a changed line\n")
        result = self.ask("--diff", "changed", "--json")
        self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
        diff = self.diff_path()
        self.assertTrue(diff.is_file())
        text = diff.read_text(encoding="utf-8")
        self.assertIn("diff --git a/CHANGE.md b/CHANGE.md", text)
        self.assertIn("+a changed line", text)
        rel = diff.relative_to(self.repo).as_posix()
        sentence = (
            f"The diff of `changed` against `master` is in `{rel}`; "
            "read it there."
        )
        self.assertEqual(
            self.prompt_file.read_text(encoding="utf-8"),
            "What changed?\n\n" + sentence,
        )
        record = self.read_record()
        self.assertEqual(record["diff"], rel)
        self.assertEqual(json.loads(result.stdout)["diff"], rel)

    def test_bad_ref_exits_before_opencode_and_writes_the_record(self) -> None:
        result = self.ask("--diff", "no-such-branch")
        self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
        self.assertIn("cause: bad-ref", result.stdout)
        self.assertFalse(self.starts.exists())
        self.assertFalse(self.diff_path().exists())
        record = self.read_record()
        self.assertEqual((record["verdict"], record["cause"]),
                         ("INFRA", "bad-ref"))
        self.assertNotIn("diff", record)

    def test_empty_diff_exits_2(self) -> None:
        self.branch("same")
        result = self.ask("--diff", "same")
        self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
        self.assertIn("cause: empty-diff", result.stdout)
        self.assertFalse(self.starts.exists())
        self.assertEqual(self.read_record()["cause"], "empty-diff")

    def test_base_without_diff_is_a_usage_error(self) -> None:
        result = self.ask("--base", "master")
        self.assertEqual(result.returncode, 2)
        self.assertIn("--base requires --diff", result.stderr)
        self.assertFalse(self.starts.exists())

    def test_ask_without_diff_leaves_the_question_unchanged(self) -> None:
        question = "On which line is <line number> | & > honoured exactly?"
        result = self.ask(question=question)
        self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
        self.assertEqual(
            self.prompt_file.read_text(encoding="utf-8"), question)
        self.assertNotIn("diff", self.read_record())

    def test_stats_does_not_count_bad_ref_or_empty_diff(self) -> None:
        self.branch("same")
        empty = self.ask("--diff", "same")
        self.assertEqual(empty.returncode, 2, empty.stderr)
        bad = self.ask("--diff", "no-such-branch")
        self.assertEqual(bad.returncode, 2, bad.stderr)
        first, second = self.read_record(1), self.read_record(2)
        self.assertEqual(first["cause"], "empty-diff")
        self.assertEqual(second["cause"], "bad-ref")
        self.assertIn("empty-diff", OCMOD.NON_FAULT_CAUSES)
        self.assertIn("bad-ref", OCMOD.NON_FAULT_CAUSES)
        for record in (first, second):
            self.assertFalse(OCMOD.is_failure(record))
            self.assertFalse(OCMOD.first_attempt_eligible(record))
        merged = dict(os.environ)
        merged.update(self.env)
        stats = subprocess.run(
            [sys.executable, str(OC), "stats", "--json"],
            cwd=self.repo, capture_output=True, text=True, timeout=90,
            env=merged,
        )
        self.assertEqual(stats.returncode, 0, stats.stderr)
        row = next(r for r in json.loads(stats.stdout)["rows"]
                   if r["model"] == MODEL and r["role"] == "scout")
        self.assertEqual(row["infra"], {"bad-ref": 1, "empty-diff": 1})
        self.assertEqual((row["first_attempt_passes"],
                          row["first_attempt_eligible"]), (0, 0))
        self.assertFalse(row["demote"])


class SpendGuardCase(unittest.TestCase):
    """The Go spend guard against a fresh temp repo per test.

    Records are injected with chosen ``started_utc`` and ``cost``; the worker
    is the fake OpenCode and the verifier the fake one, so nothing here
    touches the network or the real executable.
    """

    GO = "opencode-go/deepseek-v4.1-flash"
    KIMI = "opencode-go/kimi-k3"

    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory(prefix="oc spend ")
        self.addCleanup(self.tmp.cleanup)
        root = Path(self.tmp.name)
        self.repo = root / "repo"
        self.slots = root / "slots"
        self.calls = root / "calls.jsonl"
        self.repo.mkdir()
        self.slots.mkdir()
        sh(["git", "init", "-b", "master", str(self.repo)], root)
        sh(["git", "config", "user.email", "test@example.invalid"], self.repo)
        sh(["git", "config", "user.name", "oc test"], self.repo)
        (self.repo / "README.md").write_text("# spend repo\n", encoding="utf-8")
        agents = self.repo / ".opencode" / "agents"
        agents.mkdir(parents=True)
        for role in ("implement", "tests", "review", "scout", "grunt"):
            (agents / f"crpg-{role}.md").write_text(
                f"# crpg-{role}\n", encoding="utf-8")
        (self.repo / "opencode.json").write_text('{"agent": {}}\n',
                                                 encoding="utf-8")
        sh(["git", "add", "."], self.repo)
        sh(["git", "-c", "commit.gpgsign=false", "commit", "-m", "init"],
           self.repo)
        sh(["git", "worktree", "add", str(self.slots / "slot-1")], self.repo)
        sh(["git", "worktree", "add", str(self.slots / "slot-2")], self.repo)
        self.slot1 = (self.slots / "slot-1").resolve()
        self.env = {
            "CRPG_REPO": str(self.repo),
            "CRPG_SLOTS": str(self.slots),
            "CRPG_OPENCODE": str(FAKE_OPENCODE),
            "CRPG_VERIFY": str(FAKE_VERIFY),
            "CRPG_OC_START_GAP": "0",
            "CRPG_OC_LOCK_WAIT": "0.05",
            "FAKE_OC_MODE": "ok",
            "FAKE_OC_LOG": str(self.calls),
            "FAKE_VERIFY_MODE": "pass",
        }

    # -- helpers -----------------------------------------------------

    def oc(self, *argv: str, env: dict | None = None, timeout: int = 120):
        merged = dict(os.environ)
        merged.update(self.env)
        for name in ("CRPG_GO_SPEND_OVERRIDE", "CRPG_GO_CAP_5H",
                     "CRPG_GO_CAP_WEEK"):
            merged.pop(name, None)
        merged.update(env or {})
        return subprocess.run(
            [sys.executable, str(OC), *argv],
            cwd=self.repo, capture_output=True, text=True,
            timeout=timeout, env=merged,
        )

    def brief(self, task: str) -> Path:
        path = self.repo / ".agents" / "briefs" / f"{task}.md"
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(
            f"# Brief {task}\n\n## Files you own\n\n- `tools/x.py`\n",
            encoding="utf-8")
        return path

    def run_go(self, task: str, model: str | None = None,
               extra: list[str] | None = None, env: dict | None = None):
        return self.oc(
            "run", "--task", task, "--role", "implement",
            "--model", model or self.GO, "--base", "master",
            "--timeout", "60", "--idle", "10", "--brief", str(self.brief(task)),
            *(extra or []), env=env,
        )

    def inject(self, task: str, model: str, cost=0.1, hours_ago: float = 1.0,
               started: object = "auto") -> Path:
        if started == "auto":
            stamp = datetime.now(timezone.utc) - timedelta(hours=hours_ago)
            started = stamp.strftime("%Y-%m-%dT%H:%M:%SZ")
        payload = {
            "version": 1, "task": task, "role": "implement", "model": model,
            "attempt": 1, "elapsed_s": 20.0, "verdict": "PASS", "cause": None,
            "failed_checks": [], "steps": 3,
            "tokens": {"input": 10, "cached": 0, "output": 5},
            "retries": [], "outcomes": [],
        }
        if started is not None:
            payload["started_utc"] = started
        if cost is not None:
            payload["cost"] = cost
        path = (self.repo / ".agents" / "runs" / task
                / f"1-{model.replace('/', '_')}.result.json")
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(payload), encoding="utf-8")
        return path

    def record_path(self, task: str, model: str) -> Path:
        return (self.repo / ".agents" / "runs" / task
                / f"1-{model.replace('/', '_')}.result.json")

    def read_record(self, task: str, model: str) -> dict:
        return json.loads(self.record_path(task, model).read_text())

    def lock_files(self) -> list[Path]:
        directory = self.repo / ".agents" / "slots"
        return sorted(directory.glob("*.lock")) if directory.is_dir() else []

    # -- T1 ----------------------------------------------------------

    def test_1_sums_only_go_records_inside_the_windows(self) -> None:
        self.inject("A-old", self.GO, cost=5.0, hours_ago=24 * 8)
        self.inject("B-mid", self.GO, cost=2.0, hours_ago=10)
        self.inject("C-fresh", self.GO, cost=1.0, hours_ago=1)
        self.inject("D-zen", "opencode/big-pickle", cost=3.0, hours_ago=1)
        self.inject("E-nostamp", self.GO, cost=7.0, started=None)
        self.inject("F-badcost", self.GO, cost="lots", hours_ago=1)
        result = self.oc("spend", "--json")
        self.assertEqual(result.returncode, 0, result.stderr)
        payload = json.loads(result.stdout)
        self.assertEqual(payload["windows"]["5h"]["total"], 1.0)
        self.assertEqual(payload["windows"]["week"]["total"], 3.0)
        self.assertEqual(
            [item["model"] for item in payload["windows"]["5h"]["models"]],
            [self.GO])

    # -- T2 ----------------------------------------------------------

    def test_2_spend_output_and_json(self) -> None:
        self.inject("A", self.KIMI, cost=0.30, hours_ago=1)
        self.inject("B", self.GO, cost=0.55, hours_ago=2)
        self.inject("C", self.GO, cost=0.10, hours_ago=1)
        result = self.oc("spend")
        self.assertEqual(result.returncode, 0, result.stderr)
        lines = result.stdout.splitlines()
        self.assertTrue(lines[0].startswith("SPEND"), lines[0])
        self.assertIn("5h $0.95 of $4.50", lines[0])
        self.assertIn("week $0.95 of $12.00", lines[0])
        five = next(line for line in lines if line.startswith("5h: "))
        self.assertLess(five.index(self.GO), five.index(self.KIMI))
        self.assertLessEqual(len(lines), 40)
        payload = json.loads(self.oc("spend", "--json").stdout)
        self.assertEqual(payload["windows"]["5h"]["total"], 0.95)
        self.assertEqual(payload["windows"]["week"]["total"], 0.95)
        top = payload["windows"]["5h"]["models"][0]
        self.assertEqual((top["model"], top["cost"]), (self.GO, 0.65))
        tightened = json.loads(self.oc(
            "spend", "--json", env={"CRPG_GO_CAP_5H": "1.25"}).stdout)
        self.assertEqual(tightened["caps"], {"5h": 1.25, "week": 12.0})

    # -- T3 ----------------------------------------------------------

    def test_3_run_over_the_5h_cap_is_refused(self) -> None:
        self.inject("A-old", self.GO, cost=4.45, hours_ago=1)
        result = self.run_go("T-blocked")
        self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
        self.assertTrue(result.stdout.startswith("INFRA T-blocked"),
                        result.stdout)
        self.assertIn("cause: spend", result.stdout)
        self.assertIn("guard: 5h cap:", result.stdout)
        self.assertIn("CRPG_GO_SPEND_OVERRIDE=1", result.stdout)
        record = self.read_record("T-blocked", self.GO)
        self.assertEqual(
            (record["verdict"], record["cause"], record["cost"]),
            ("INFRA", "spend", 0))
        self.assertEqual(self.lock_files(), [])
        self.assertFalse((self.slot1 / "BRIEF.md").exists())
        self.assertFalse(self.calls.exists())

    # -- T4 ----------------------------------------------------------

    def test_4_run_over_the_weekly_cap_is_refused(self) -> None:
        self.inject("A", self.GO, cost=6.00, hours_ago=30)
        self.inject("B", self.GO, cost=5.95, hours_ago=40)
        result = self.run_go("T-week")
        self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
        self.assertIn("cause: spend", result.stdout)
        self.assertIn("guard: week cap:", result.stdout)
        self.assertIn("5h $0.00 of $4.50", result.stdout)
        record = self.read_record("T-week", self.GO)
        self.assertEqual((record["verdict"], record["cause"]), ("INFRA", "spend"))

    # -- T5 ----------------------------------------------------------

    def test_5_scarce_model_is_one_per_5h_window(self) -> None:
        self.inject("A-old", self.KIMI, cost=0.05, hours_ago=6)
        first = self.run_go("T-kimi-1", model=self.KIMI)
        self.assertEqual(first.returncode, 0, first.stdout + first.stderr)
        self.inject("A-fresh", self.KIMI, cost=0.05, hours_ago=1)
        second = self.run_go("T-kimi-2", model=self.KIMI)
        self.assertEqual(second.returncode, 2,
                         second.stdout + second.stderr)
        self.assertIn("guard: scarce", second.stdout)
        self.assertIn("at most one per 5h", second.stdout)
        record = self.read_record("T-kimi-2", self.KIMI)
        self.assertEqual((record["verdict"], record["cause"]), ("INFRA", "spend"))

    # -- T6 ----------------------------------------------------------

    def test_6_non_go_model_is_never_refused(self) -> None:
        self.inject("A", self.KIMI, cost=100.0, hours_ago=1)
        result = self.run_go("T-free", model="opencode/big-pickle")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(
            self.read_record("T-free", "opencode/big-pickle")["verdict"], "PASS")

    # -- T7 ----------------------------------------------------------

    def test_7_override_lets_the_run_start(self) -> None:
        self.inject("A-old", self.GO, cost=4.45, hours_ago=1)
        result = self.run_go("T-over", env={"CRPG_GO_SPEND_OVERRIDE": "1"})
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("spend guard overridden", result.stdout)
        record = self.read_record("T-over", self.GO)
        self.assertEqual((record["verdict"], record["cause"]), ("PASS", None))

    # -- T8 ----------------------------------------------------------

    def test_8_ask_is_guarded(self) -> None:
        self.inject("A-old", self.GO, cost=4.45, hours_ago=1)
        result = self.oc("ask", "--role", "scout", "--model", self.GO,
                         "what changed?")
        self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
        self.assertIn("cause: spend", result.stdout)
        self.assertIn("guard:", result.stdout)
        record = self.read_record("ask", self.GO)
        self.assertEqual(
            (record["verdict"], record["cause"], record["cost"]),
            ("INFRA", "spend", 0))
        self.assertFalse(self.calls.exists())

    # -- T9 ----------------------------------------------------------

    def test_9_wave_refusal_leaves_the_other_members_running(self) -> None:
        self.inject("A-old", self.GO, cost=4.45, hours_ago=1)
        refused, fine = "T-wave-blocked", "T-wave-ok"
        self.brief(refused)
        self.brief(fine)
        result = self.oc(
            "wave", f"{refused}:implement:{self.GO}",
            f"{fine}:implement:opencode/big-pickle", "--base", "master")
        self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
        lines = result.stdout.splitlines()
        refused_line = next(line for line in lines
                            if self.GO in line and line.startswith("INFRA"))
        self.assertIn("spend", refused_line)
        self.assertTrue(any(line.startswith(f"PASS {fine}")
                            for line in lines), result.stdout)
        self.assertEqual(
            self.read_record(fine, "opencode/big-pickle")["verdict"], "PASS")

    # -- T10 ---------------------------------------------------------

    def test_10_dry_run_reports_the_refusal(self) -> None:
        self.inject("A-old", self.GO, cost=4.45, hours_ago=1)
        result = self.run_go("T-dry", extra=["--dry-run"])
        self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
        self.assertIn("cause: spend", result.stdout)
        self.assertIn("guard: 5h cap:", result.stdout)
        self.assertFalse(self.record_path("T-dry", self.GO).exists())
        self.assertEqual(self.lock_files(), [])
        self.assertFalse(self.calls.exists())

    # -- T11 ---------------------------------------------------------

    def test_11_stats_does_not_count_a_spend_exit_against_the_model(self) -> None:
        injected = self.inject("A-old", self.GO, cost=4.45, hours_ago=1)
        result = self.run_go("T-stats")
        self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
        record = self.read_record("T-stats", self.GO)
        self.assertFalse(OCMOD.is_failure(record))
        injected.unlink()
        rows = json.loads(self.oc("stats", "--json").stdout)["rows"]
        row = next(r for r in rows
                   if r["model"] == self.GO and r["role"] == "implement")
        self.assertEqual(row["infra"], {"spend": 1})
        self.assertEqual(
            (row["first_attempt_passes"], row["first_attempt_eligible"]),
            (0, 0))
        self.assertFalse(row["demote"])


class GoSpendWindowCase(unittest.TestCase):
    """Direct tests for ``go_spend`` window boundaries (A0-3e).

    Every call passes a fixed ``now`` so edges are exact to the second.
    """

    GO = "opencode-go/deepseek-v4.1-flash"
    KIMI = "opencode-go/kimi-k3"

    def _fmt(self, dt):
        return dt.strftime("%Y-%m-%dT%H:%M:%SZ")

    def test_b1_five_hour_boundary(self):
        now = datetime(2026, 10, 21, 12, 0, 0, tzinfo=timezone.utc)
        exact = now - timedelta(hours=5)
        over = now - timedelta(hours=5, seconds=1)
        records = [
            {"model": self.GO, "started_utc": self._fmt(exact), "cost": 1.0},
            {"model": self.GO, "started_utc": self._fmt(over), "cost": 2.0},
        ]
        data = OCMOD.go_spend(records, now)
        self.assertEqual(data["totals"]["5h"], 1.0)
        self.assertEqual(data["totals"]["week"], 3.0)

    def test_b2_week_boundary(self):
        now = datetime(2026, 10, 21, 12, 0, 0, tzinfo=timezone.utc)
        exact = now - timedelta(days=7)
        over = now - timedelta(days=7, seconds=1)
        records = [
            {"model": self.GO, "started_utc": self._fmt(exact), "cost": 1.0},
            {"model": self.GO, "started_utc": self._fmt(over), "cost": 2.0},
        ]
        data = OCMOD.go_spend(records, now)
        self.assertEqual(data["totals"]["5h"], 0.0)
        self.assertEqual(data["totals"]["week"], 1.0)

    def test_b3_clock_skew_not_counted_nor_scarce(self):
        now = datetime(2026, 10, 21, 12, 0, 0, tzinfo=timezone.utc)
        future = now + timedelta(seconds=1)
        records = [
            {"model": self.KIMI, "started_utc": self._fmt(future), "cost": 1.0},
        ]
        data = OCMOD.go_spend(records, now)
        self.assertEqual(data["totals"]["5h"], 0.0)
        self.assertEqual(data["totals"]["week"], 0.0)
        self.assertNotIn(self.KIMI, data["scarce"])

    def test_b4_cost_coercion(self):
        now = datetime(2026, 10, 21, 12, 0, 0, tzinfo=timezone.utc)
        started = self._fmt(now)
        records = [
            {"model": self.GO, "started_utc": started, "cost": True},
            {"model": self.GO, "started_utc": started, "cost": None},
            {"model": self.GO, "started_utc": started, "cost": "lots"},
            {"model": self.GO, "started_utc": started},
            {"model": self.GO, "started_utc": started, "cost": 7},
        ]
        data = OCMOD.go_spend(records, now)
        self.assertEqual(data["totals"]["5h"], 7.0)
        self.assertEqual(data["totals"]["week"], 7.0)

    def test_b5_newest_scarce_start_order_independent(self):
        now = datetime(2026, 10, 21, 12, 0, 0, tzinfo=timezone.utc)
        t0 = now - timedelta(hours=4)
        t1 = now - timedelta(hours=2)
        t2 = now - timedelta(hours=1)
        latest = t2
        records = [
            {"model": self.KIMI, "started_utc": self._fmt(t0), "cost": 0.1},
            {"model": self.KIMI, "started_utc": self._fmt(t1), "cost": 0.1},
            {"model": self.KIMI, "started_utc": self._fmt(t2), "cost": 0.1},
        ]
        data_fwd = OCMOD.go_spend(records, now)
        data_rev = OCMOD.go_spend(list(reversed(records)), now)
        self.assertEqual(data_fwd["scarce"][self.KIMI], latest)
        self.assertEqual(data_rev["scarce"][self.KIMI], latest)


class FetchBaseCase(unittest.TestCase):
    """`--base` names a remote only when the slot configures one."""

    def setUp(self) -> None:
        self.tmp = Path(tempfile.mkdtemp(prefix="oc fetch "))
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.remote = self.tmp / "remote"
        self.slot = self.tmp / "slot"
        sh(["git", "init", "-q", "-b", "master", str(self.remote)], self.tmp)
        sh(["git", "config", "user.email", "test@example.invalid"], self.remote)
        sh(["git", "config", "user.name", "oc test"], self.remote)
        (self.remote / "README.md").write_text("# remote\n", encoding="utf-8")
        sh(["git", "add", "README.md"], self.remote)
        sh(["git", "-c", "commit.gpgsign=false", "commit", "-m", "init"], self.remote)
        # Clone before the extra branch exists, so its remote-tracking ref
        # is created only by an actual fetch.
        sh(["git", "clone", "-q", str(self.remote), str(self.slot)], self.tmp)
        sh(["git", "branch", "mgr/session-10-batch"], self.remote)

    def remote_ref(self) -> subprocess.CompletedProcess:
        return subprocess.run(
            ["git", "rev-parse", "--verify", "--quiet",
             "refs/remotes/origin/mgr/session-10-batch"],
            cwd=self.slot, capture_output=True, text=True,
        )

    def test_remote_prefixed_base_fetches_the_nested_branch(self) -> None:
        OCMOD.fetch_base(self.slot, "origin/mgr/session-10-batch")
        self.assertEqual(self.remote_ref().returncode, 0)

    def test_slash_base_without_a_remote_fetches_nothing(self) -> None:
        # `mgr` is not a configured remote: the whole value is a local ref.
        OCMOD.fetch_base(self.slot, "mgr/session-10-batch")
        self.assertNotEqual(self.remote_ref().returncode, 0)

    def test_a_configured_remote_with_a_missing_branch_still_raises(self) -> None:
        with self.assertRaises(OCMOD.Exit) as caught:
            OCMOD.fetch_base(self.slot, "origin/no-such-branch")
        self.assertEqual(caught.exception.code, 2)
        self.assertTrue(str(caught.exception.cause).startswith("fetch"))


class HarnessUnitCase(unittest.TestCase):
    """Direct unit tests for oc.py helpers: no git, no worker."""

    def test_next_attempt_counts_only_the_same_slug(self) -> None:
        with tempfile.TemporaryDirectory(prefix="oc attempts ") as tmp:
            task_dir = Path(tmp)
            (task_dir / "1-go_m.result.json").write_text(
                json.dumps({"attempt": 1, "slug": None}), encoding="utf-8"
            )
            self.assertEqual(OCMOD.next_attempt_number(task_dir), 2)
            self.assertEqual(OCMOD.next_attempt_number(task_dir, "draft-a"), 1)
            (task_dir / "1-draft-a-go_m.result.json").write_text(
                json.dumps({"attempt": 1, "slug": "draft-a"}), encoding="utf-8"
            )
            self.assertEqual(OCMOD.next_attempt_number(task_dir, "draft-a"), 2)
            self.assertEqual(OCMOD.next_attempt_number(task_dir, "draft-b"), 1)
            # A record written before the slug field existed is slug-less.
            (task_dir / "2-go_m.result.json").write_text(
                json.dumps({"attempt": 2}), encoding="utf-8"
            )
            self.assertEqual(OCMOD.next_attempt_number(task_dir), 3)

    def test_pid_probe_never_kills(self) -> None:
        sleeper = subprocess.Popen(
            [sys.executable, "-c", "import time; time.sleep(60)"],
            stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        try:
            self.assertTrue(OCMOD._pid_alive(sleeper.pid))
            # The probe must not signal: the sleeper is still running.
            self.assertIsNone(sleeper.poll())
        finally:
            sleeper.kill()
            sleeper.wait(timeout=30)
        self.assertFalse(OCMOD._pid_alive(sleeper.pid))

    def test_windows_liveness_probe_counts_alive_when_in_doubt(self) -> None:
        # The Windows probe reads only; a kernel32 shim proves the doubt
        # rule on any OS (attempt-5 finding 4).
        class Kernel32Shim:
            def __init__(self, handle, last_error=0, exit_ok=True,
                         exit_code=259):
                self.handle = handle
                self.error = last_error
                self.exit_ok = exit_ok
                self.exit_code = exit_code
                self.closed: list[int] = []

            def OpenProcess(self, access, inherit, pid):
                return self.handle

            def GetLastError(self):
                return self.error

            def GetExitCodeProcess(self, handle, out):
                if not self.exit_ok:
                    return 0
                ctypes.cast(
                    out, ctypes.POINTER(ctypes.c_uint32)
                )[0] = self.exit_code
                return 1

            def CloseHandle(self, handle):
                self.closed.append(handle)
                return 1

        # ERROR_ACCESS_DENIED (5): a live process we may not open is alive.
        denied = Kernel32Shim(0, last_error=5)
        self.assertTrue(OCMOD._win_pid_alive(4242, denied))
        # ERROR_INVALID_PARAMETER (87): no such process, the only dead case.
        invalid = Kernel32Shim(0, last_error=87)
        self.assertFalse(OCMOD._win_pid_alive(4242, invalid))
        # Any other error code is uncertain: alive.
        unknown = Kernel32Shim(0, last_error=1008)
        self.assertTrue(OCMOD._win_pid_alive(4242, unknown))
        # A failed GetExitCodeProcess is uncertain: alive.
        blink = Kernel32Shim(1, exit_ok=False)
        self.assertTrue(OCMOD._win_pid_alive(4242, blink))
        # A still-active process is alive, and its handle is closed.
        live = Kernel32Shim(1, exit_code=259)
        self.assertTrue(OCMOD._win_pid_alive(4242, live))
        self.assertEqual(live.closed, [1])
        # An already-exited process (exit code 0) is dead.
        gone = Kernel32Shim(1, exit_code=0)
        self.assertFalse(OCMOD._win_pid_alive(4242, gone))

    def lock_path(self, repo: Path, number: int) -> Path:
        directory = repo / ".agents" / "slots"
        directory.mkdir(parents=True, exist_ok=True)
        return directory / f"slot-{number}.lock"

    def test_stale_lock_is_reclaimed_on_the_same_slot(self) -> None:
        tmp = Path(tempfile.mkdtemp(prefix="oc lock "))
        self.addCleanup(shutil.rmtree, tmp, True)
        placement = tmp / "slot-7"
        placement.mkdir()
        lock = self.lock_path(tmp, 7)
        dead = subprocess.Popen([sys.executable, "-c", "pass"])
        dead.wait(timeout=30)
        lock.write_text(f"{dead.pid}\n", encoding="utf-8")
        try:
            number, path = OCMOD.acquire_slot(tmp, [(7, placement)], None, False)
            self.assertEqual((number, path), (7, placement))
            self.assertEqual(lock.read_text(encoding="utf-8").split()[0],
                             str(os.getpid()))
        finally:
            OCMOD.release_slot(tmp, 7)
        self.assertFalse(lock.exists())

    def test_garbage_lock_is_stale(self) -> None:
        tmp = Path(tempfile.mkdtemp(prefix="oc lock "))
        self.addCleanup(shutil.rmtree, tmp, True)
        placement = tmp / "slot-7"
        placement.mkdir()
        self.lock_path(tmp, 7).write_text("not-a-pid\n", encoding="utf-8")
        try:
            number, _ = OCMOD.acquire_slot(tmp, [(7, placement)], None, False)
            self.assertEqual(number, 7)
        finally:
            OCMOD.release_slot(tmp, 7)

    def test_live_lock_is_busy(self) -> None:
        sleeper = subprocess.Popen(
            [sys.executable, "-c", "import time; time.sleep(60)"],
            stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        tmp = Path(tempfile.mkdtemp(prefix="oc lock "))
        self.addCleanup(shutil.rmtree, tmp, True)
        placement = tmp / "slot-7"
        placement.mkdir()
        self.lock_path(tmp, 7).write_text(f"{sleeper.pid}\n", encoding="utf-8")
        try:
            with self.assertRaises(Exception) as caught:
                OCMOD.acquire_slot(tmp, [(7, placement)], None, False)
            self.assertIn("slot-busy", str(caught.exception))
        finally:
            sleeper.kill()
            sleeper.wait(timeout=30)

    def test_wave_fit_caps(self) -> None:
        fits = OCMOD.wave_fits
        # At most one run per free Zen model.
        self.assertFalse(fits("opencode/a-free", ["opencode/a-free"]))
        self.assertTrue(fits("opencode/b-free", ["opencode/a-free"]))
        # At most three free Zen runs in total; paid models are unaffected.
        self.assertFalse(fits("opencode/d-free",
                              ["opencode/a-free", "opencode/b-free",
                               "opencode/c-free"]))
        self.assertTrue(fits("opencode/d-free",
                             ["opencode/a-free", "opencode/b-free"]))
        self.assertTrue(fits("go/m",
                             ["opencode/a-free", "opencode/b-free",
                              "opencode/c-free"]))
        # At most three runs per model of any kind.
        self.assertFalse(fits("go/m", ["go/m", "go/m", "go/m"]))
        self.assertTrue(fits("go/m", ["go/m", "go/m"]))

    def test_model_flag_forms(self) -> None:
        flag = OCMOD.model_flag
        self.assertEqual(flag("go/m", None, 2), ["-m", "go/m"])
        self.assertEqual(flag("go/m", "low", 2), ["-m", "go/m#low"])
        self.assertEqual(flag("go/m", None, 1), ["-m", "go/m"])
        self.assertEqual(
            flag("go/m", "low", 1), ["-m", "go/m", "--variant", "low"]
        )

    def test_brief_owned_crates(self) -> None:
        owned = OCMOD.brief_owned_crates(
            "# Goal\n\n- `crates/elsewhere/x.rs`\n\n"
            "## Files you own\n\n"
            "- `tools/agents/oc.py`\n"
            "- crates/alpha/src/lib.rs\n"
            "- `crates/beta/tests/t.rs`\n\n"
            "## Out of scope\n\n- `crates/gamma/`\n"
        )
        self.assertEqual(owned, {"alpha", "beta"})

    def test_is_failure_follows_models_4_1(self) -> None:
        base = {"verdict": "PASS", "cause": None, "outcomes": []}
        verify_fail = dict(base, verdict="FAIL")
        self.assertTrue(OCMOD.is_failure(verify_fail))
        rejected = dict(base, outcomes=[{"outcome": "review-reject"}])
        self.assertTrue(OCMOD.is_failure(rejected))
        for cause in ("no-edit", "timeout", "hang"):
            self.assertTrue(OCMOD.is_failure(dict(base, cause=cause)))
        for cause in ("quota", "402", "locked"):
            self.assertFalse(OCMOD.is_failure(dict(base, cause=cause)))
        self.assertFalse(OCMOD.is_failure(dict(base, verdict="FAIL",
                                              cause="recorded")))
        self.assertFalse(OCMOD.is_failure(dict(base, verdict="FAIL",
                                              cause="no-text")))
        fixed = dict(base, outcomes=[{"outcome": "review-fix"}])
        self.assertFalse(OCMOD.is_failure(fixed))


if __name__ == "__main__":
    unittest.main()

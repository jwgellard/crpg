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
refusal, and filed-brief/placeholder/same-crate handling.
"""

from __future__ import annotations

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
        agents = cls.repo / "tools" / "agents"
        agents.mkdir(parents=True)
        (agents / "brief.py").write_text(BRIEF_STUB, encoding="utf-8")
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
               env=None, timeout=90, brief=True):
        argv = [
            "run", "--task", task, "--role", role, "--model", model,
            "--base", "master", "--timeout", "60", "--idle", "10",
        ]
        if brief:
            argv += ["--brief", self.brief()]
        argv += extra or []
        return self.oc(*argv, env=env, timeout=timeout)

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
        task = self.tid()
        call_log = self.fresh_file("calls.jsonl")
        result = self.oc_run(task, env={"FAKE_OC_LOG": str(call_log)})
        self.assertEqual(result.returncode, 0, result.stderr)
        calls = self.fake_calls(call_log)
        self.assertGreaterEqual(len(calls), 1)
        first = calls[0]
        self.assertIn("--dir", first["argv"])
        self.assertEqual(
            first["argv"][first["argv"].index("--dir") + 1], str(self.slot1)
        )
        self.assertEqual(Path(first["cwd"]).resolve(), self.slot1)
        self.assertEqual(Path(first["pwd"]).resolve(), self.slot1)
        self.assertNotIn("--auto", first["argv"])
        self.assertEqual(first["argv"][-1], PROMPT)

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
        second = calls[1]["argv"]
        self.assertIn("--variant", second)
        self.assertEqual(second[second.index("--variant") + 1], "low")
        self.assertIn(STALL_SENTENCE, second[-1])
        record = self.read_record(task)
        self.assertEqual(record["attempt"], 1)
        self.assertEqual(
            record["retries"], [{"cause": "stall", "variant": "low"}]
        )

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
        self.assertIsNone(classify("all done, nothing to report"))
        self.assertEqual(OCMOD.DEMOTE_WINDOW, 4)
        self.assertEqual(OCMOD.DEMOTE_FAILURES, 2)
        self.assertEqual(OCMOD.PROMOTE_MIN_RUNS, 5)

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

    def test_review_role_needs_text_not_verify(self) -> None:
        task = self.tid()
        result = self.oc_run(
            task, role="review", env={"FAKE_VERIFY_MODE": "error"}
        )
        self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
        self.assertTrue(result.stdout.splitlines()[0].startswith(f"PASS {task}"))

    def test_tests_role_skips_gates(self) -> None:
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
        self.assertIn("--skip", logged[0])
        self.assertIn("gates", logged[0])

    def test_ask_without_text_is_exit_1(self) -> None:
        result = self.oc(
            "ask", "--role", "scout", "--model", MODEL,
            "--timeout", "60", "--idle", "10", "is anyone there?",
            env={"FAKE_OC_MODE": "no-text"},
        )
        self.assertEqual(result.returncode, 1, result.stdout)

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
        after = sh(
            ["git", "rev-parse", "--abbrev-ref", "HEAD"], self.slot1
        ).stdout.strip()
        self.assertEqual(before, after)
        self.assertFalse(self.log_path(task).exists())
        self.assertFalse(self.record_path(task).exists())

    def test_attempt_continues_on_its_branch(self) -> None:
        task = self.tid()
        first = self.oc_run(task)
        self.assertEqual(first.returncode, 0, first.stderr)
        second = self.oc_run(task)
        self.assertEqual(second.returncode, 0, second.stderr)
        self.assertEqual(self.read_record(task, attempt=2)["attempt"], 2)
        sh(["git", "checkout", "-b", "detour"], self.slot1)
        try:
            third = self.oc_run(task)
            self.assertEqual(third.returncode, 2, third.stdout)
            self.assertIn(f"task/{task}", third.stderr)
            fresh = self.oc_run(task, extra=["--fresh"])
            self.assertEqual(fresh.returncode, 0, fresh.stderr + fresh.stdout)
        finally:
            sh(["git", "checkout", f"task/{task}"], self.slot1)
            sh(["git", "branch", "-D", "detour"], self.slot1)

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

    def test_git_clean_failure_is_branch(self) -> None:
        task = self.tid()
        held = self.slot1 / "oc-clean-probe.txt"
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
            task, role="grunt", env={"FAKE_VERIFY_LOG": str(verify_log)}
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
        result = self.oc_run(task, model="opencode/cat-jam-free")
        self.assertEqual(result.returncode, 2, result.stdout)
        self.assertIn("usage", result.stdout + result.stderr)
        self.assertIn("opencode-go/cat-jam-free", result.stdout + result.stderr)
        self.assertFalse(self.record_path(task, model="opencode/cat-jam-free").exists())
        asked = self.oc(
            "ask", "--role", "scout", "--model", "opencode/longcat-2.5-preview-free",
            "--timeout", "60", "a question",
        )
        self.assertEqual(asked.returncode, 2, asked.stdout)
        self.assertIn("opencode-go/longcat-2.5-preview-free",
                      asked.stdout + asked.stderr)

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


class HarnessUnitCase(unittest.TestCase):
    """Direct unit tests for oc.py helpers: no git, no worker."""

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

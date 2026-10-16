#!/usr/bin/env python3
"""Subprocess tests for the context meter hook (A0-6b).

Each test plants a fake repository with a ``.git`` marker and a JSONL
transcript, runs the hook as a real process with one hook JSON object on stdin,
and checks the exit code, stdout and stderr. The transcript size is expressed
with small token numbers, so no test needs a genuinely large context; only the
tail-reading case writes a file over 4 MB.
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

AGENTS = Path(__file__).resolve().parent.parent / "agents"
CONTEXT_METER = AGENTS / "context_meter.py"

WARN = 150000
STOP = 200000
REPEAT_STEP = 25000

WARN_MESSAGE = (
    "Context meter: about {n}k tokens (warn at {warn}k). Finish the package in "
    "hand, start no new one, and plan the handover (docs/AGENT_TEAM_PLAN.md §8)."
)
STOP_MESSAGE = (
    "Context meter: about {n}k tokens (stop at {stop}k). Hand over now: update "
    "the handover in docs/PROJECT_STATE.md, naming every run still going, "
    "commit, and tell the user to start a fresh session "
    "(docs/AGENT_TEAM_PLAN.md §8)."
)


class ContextMeterTests(unittest.TestCase):
    """Every interface bullet, as a process with a planted transcript."""

    def setUp(self) -> None:
        self.tmp = Path(tempfile.mkdtemp(prefix="context meter "))
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.root = self.tmp / "repo"
        (self.root / ".git").mkdir(parents=True)
        self.transcript = self.root / "transcript.jsonl"
        self._sessions = 0

    # -- fixtures ---------------------------------------------------------

    def session(self) -> str:
        self._sessions += 1
        return f"session-{self._sessions}"

    def tokens(self, total: int) -> str:
        return json.dumps(
            {"cwd": str(self.root), "message": {"usage": {"input_tokens": total}}}
        )

    def usage(self, **fields: int) -> str:
        return json.dumps(
            {"cwd": str(self.root), "message": {"usage": dict(fields)}}
        )

    def no_usage(self) -> str:
        return json.dumps({"cwd": str(self.root), "type": "summary"})

    def write(self, lines: list[str], path: Path | None = None) -> Path:
        path = path or self.transcript
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text("\n".join(lines) + "\n", encoding="utf-8")
        return path

    def state_file(self, session: str, root: Path | None = None) -> Path:
        return (root or self.root) / ".agents" / "context-meter" / f"{session}.json"

    def warn_message(self, n: int, warn: int = WARN) -> str:
        return WARN_MESSAGE.format(n=n, warn=warn // 1000)

    def stop_message(self, n: int, stop: int = STOP) -> str:
        return STOP_MESSAGE.format(n=n, stop=stop // 1000)

    # -- runner -----------------------------------------------------------

    def invoke(
        self,
        session: str,
        transcript: Path | None = None,
        env: dict | None = None,
        cwd: Path | None = None,
        payload: dict | None = None,
        text_input: str | None = None,
    ) -> subprocess.CompletedProcess:
        if payload is None:
            payload = {
                "hook_event_name": "PostToolUse",
                "session_id": session,
                "transcript_path": str(transcript or self.transcript),
                "cwd": str(self.root),
            }
        stdin = text_input if text_input is not None else json.dumps(payload)
        full_env = dict(os.environ)
        full_env.pop("CRPG_CONTEXT_WARN", None)
        full_env.pop("CRPG_CONTEXT_STOP", None)
        if env:
            full_env.update(env)
        return subprocess.run(
            [sys.executable, str(CONTEXT_METER)],
            input=stdin,
            capture_output=True,
            text=True,
            cwd=str(cwd or self.root),
            env=full_env,
            timeout=30,
        )

    def notice(self, result: subprocess.CompletedProcess) -> str:
        self.assertEqual(result.returncode, 0, result.stderr)
        return json.loads(result.stdout)["hookSpecificOutput"]["additionalContext"]

    # -- behaviours -------------------------------------------------------

    def test_below_warn_is_silent(self) -> None:
        self.write([self.tokens(WARN - 1)])
        result = self.invoke(self.session())
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "")
        self.assertEqual(result.stderr, "")

    def test_crossing_warn_notifies_once(self) -> None:
        session = self.session()
        self.write([self.tokens(WARN)])
        self.assertEqual(self.notice(self.invoke(session)), self.warn_message(150))
        # Same level on the next call is silent, and the level was recorded.
        second = self.invoke(session)
        self.assertEqual(second.returncode, 0, second.stderr)
        self.assertEqual(second.stdout, "")
        state = json.loads(self.state_file(session).read_text(encoding="utf-8"))
        self.assertEqual(state["level"], 1)

    def test_crossing_stop_notifies(self) -> None:
        self.write([self.tokens(STOP)])
        self.assertEqual(self.notice(self.invoke(self.session())), self.stop_message(200))

    def test_repeats_past_stop_each_25k(self) -> None:
        session = self.session()
        self.write([self.tokens(STOP)])
        self.assertEqual(self.notice(self.invoke(session)), self.stop_message(200))
        self.assertEqual(self.invoke(session).stdout, "")
        # One token short of the next repeat is still the same level.
        self.write([self.tokens(STOP + REPEAT_STEP - 1)])
        self.assertEqual(self.invoke(session).stdout, "")
        self.write([self.tokens(STOP + REPEAT_STEP)])
        self.assertEqual(self.notice(self.invoke(session)), self.stop_message(225))
        self.assertEqual(self.invoke(session).stdout, "")
        self.write([self.tokens(STOP + 2 * REPEAT_STEP)])
        self.assertEqual(self.notice(self.invoke(session)), self.stop_message(250))

    def test_uses_last_usage_line_not_largest(self) -> None:
        # The largest usage is earlier; the last usage (100k) wins and is silent.
        self.write([self.tokens(300000), self.tokens(100000), self.no_usage()])
        self.assertEqual(self.invoke(self.session()).stdout, "")
        # The last usage's own value, not the earlier 300k, is what is reported.
        self.write([self.tokens(300000), self.tokens(160000), self.no_usage()])
        self.assertEqual(self.notice(self.invoke(self.session())), self.warn_message(160))

    def test_missing_usage_fields_count_as_zero(self) -> None:
        self.write([self.usage(input_tokens=150000)])
        self.assertEqual(self.notice(self.invoke(self.session())), self.warn_message(150))
        self.write([self.usage(cache_read_input_tokens=100000, cache_creation_input_tokens=50000)])
        self.assertEqual(self.notice(self.invoke(self.session())), self.warn_message(150))
        self.write([self.usage()])
        self.assertEqual(self.invoke(self.session()).stdout, "")

    def test_transcript_over_4mb_is_read_from_the_tail(self) -> None:
        padding = json.dumps({"cwd": str(self.root), "padding": "x" * (5 * 1024 * 1024)})
        self.write([self.tokens(300000), padding, self.tokens(STOP)])
        self.assertGreater(self.transcript.stat().st_size, 4 * 1024 * 1024)
        self.assertEqual(self.notice(self.invoke(self.session())), self.stop_message(200))

    def test_malformed_lines_are_skipped(self) -> None:
        self.write(["not json {", "", self.tokens(WARN)])
        self.assertEqual(self.notice(self.invoke(self.session())), self.warn_message(150))

    def test_message_without_usage_is_ignored(self) -> None:
        self.write(
            [
                self.tokens(WARN),
                json.dumps({"cwd": str(self.root), "message": {"role": "assistant"}}),
            ]
        )
        self.assertEqual(self.notice(self.invoke(self.session())), self.warn_message(150))

    def test_env_thresholds_are_honoured(self) -> None:
        env = {"CRPG_CONTEXT_WARN": "100000", "CRPG_CONTEXT_STOP": "120000"}
        session = self.session()
        self.write([self.tokens(100000)])
        self.assertEqual(
            self.notice(self.invoke(session, env=env)), self.warn_message(100, warn=100000)
        )
        self.write([self.tokens(119999)])
        self.assertEqual(self.invoke(session, env=env).stdout, "")
        self.write([self.tokens(120000)])
        self.assertEqual(
            self.notice(self.invoke(session, env=env)), self.stop_message(120, stop=120000)
        )
        self.write([self.tokens(145000)])
        self.assertEqual(
            self.notice(self.invoke(session, env=env)), self.stop_message(145, stop=120000)
        )

    def test_unusable_env_falls_back_to_defaults(self) -> None:
        env = {"CRPG_CONTEXT_WARN": "abc", "CRPG_CONTEXT_STOP": ""}
        self.write([self.tokens(WARN - 1)])
        self.assertEqual(self.invoke(self.session(), env=env).stdout, "")
        self.write([self.tokens(WARN)])
        self.assertEqual(self.notice(self.invoke(self.session(), env=env)), self.warn_message(150))

    def test_stdout_is_one_exact_json_object(self) -> None:
        self.write([self.tokens(WARN)])
        result = self.invoke(self.session())
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(result.stdout.endswith("\n"), repr(result.stdout))
        self.assertEqual(result.stdout.count("\n"), 1)
        payload = json.loads(result.stdout)
        self.assertEqual(list(payload.keys()), ["hookSpecificOutput"])
        inner = payload["hookSpecificOutput"]
        self.assertEqual(set(inner.keys()), {"hookEventName", "additionalContext"})
        self.assertEqual(inner["hookEventName"], "PostToolUse")
        self.assertEqual(inner["additionalContext"], self.warn_message(150))

    def test_bad_stdin_exits_zero_without_stdout(self) -> None:
        result = self.invoke(self.session(), text_input="not json")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "")
        self.assertLessEqual(len(result.stderr.strip().splitlines()), 1)

    def test_missing_transcript_exits_zero_without_stdout(self) -> None:
        payload = {
            "session_id": self.session(),
            "transcript_path": str(self.tmp / "nope.jsonl"),
        }
        result = self.invoke("unused", payload=payload)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "")
        self.assertLessEqual(len(result.stderr.strip().splitlines()), 1)

    def test_missing_keys_exit_zero_without_stdout(self) -> None:
        for payload in ({"session_id": "s"}, {"transcript_path": "x"}, {}):
            with self.subTest(payload=payload):
                result = self.invoke("unused", payload=payload)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(result.stdout, "")

    def test_unwritable_state_still_notifies(self) -> None:
        # ``.agents`` is a file, so the state directory cannot be created.
        (self.root / ".agents").write_text("not a directory\n", encoding="utf-8")
        self.write([self.tokens(WARN)])
        self.assertEqual(self.notice(self.invoke(self.session())), self.warn_message(150))

    def test_corrupt_state_is_ignored_and_rewritten(self) -> None:
        session = self.session()
        self.write([self.tokens(WARN)])
        state = self.state_file(session)
        state.parent.mkdir(parents=True, exist_ok=True)
        state.write_text("{not json", encoding="utf-8")
        self.assertEqual(self.notice(self.invoke(session)), self.warn_message(150))
        self.assertEqual(
            json.loads(state.read_text(encoding="utf-8"))["level"], 1
        )

    def test_sessions_are_independent(self) -> None:
        self.write([self.tokens(WARN)])
        self.assertNotEqual(self.invoke(self.session()).stdout, "")
        self.assertNotEqual(self.invoke(self.session()).stdout, "")

    def test_repo_root_comes_from_transcript_cwd(self) -> None:
        loose = self.tmp / "loose"
        loose.mkdir()
        session = self.session()
        self.write([self.tokens(WARN)])  # every line carries cwd = self.root
        self.assertNotEqual(self.invoke(session, cwd=loose).stdout, "")
        self.assertTrue(self.state_file(session).exists())
        # The state was found under the transcript's repo, not the process cwd.
        self.assertEqual(self.invoke(session, cwd=loose).stdout, "")

    def test_repo_root_falls_back_to_process_cwd(self) -> None:
        session = self.session()
        line = json.dumps({"message": {"usage": {"input_tokens": WARN}}})
        self.write([line])  # no cwd field anywhere in the transcript
        self.assertNotEqual(self.invoke(session).stdout, "")
        self.assertTrue(self.state_file(session).exists())

    def test_no_repository_still_notifies(self) -> None:
        outside = self.tmp / "elsewhere"
        outside.mkdir()
        transcript = outside / "t.jsonl"
        line = json.dumps({"cwd": str(outside), "message": {"usage": {"input_tokens": WARN}}})
        self.write([line], path=transcript)
        result = self.invoke(self.session(), transcript=transcript)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotEqual(result.stdout, "")


if __name__ == "__main__":
    unittest.main()

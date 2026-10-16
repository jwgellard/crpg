#!/usr/bin/env python3
"""Planted-fault tests for tools/agents/verify_task.py (A0-4).

Each test builds a throwaway git repository with the tiny layout the brief
describes, plants exactly one fault, runs the verifier as a real process, and
inspects its text or JSON output and exit code. Gate and lint commands are
stubbed through CRPG_PREFLIGHT and CRPG_LINTS, so no test runs cargo, a real
lint or the network.
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
TOOLS = HERE.parent
SCRIPT = TOOLS / "agents" / "verify_task.py"

CHECKS = [
    "scope",
    "gates",
    "protected",
    "goldens",
    "weakening",
    "unsafe",
    "lints",
    "docsig",
    "todo",
]
TODAY = "2026-10-16"

STUB = "import sys\nsys.exit(int(sys.argv[1]))\n"

BASIC_TEST = textwrap.dedent(
    """\
    #[test]
    fn a() { assert!(true); }

    #[test]
    fn b() { assert_eq!(1, 1); }
    """
)

BASE_FILES = {
    "Cargo.toml": "[workspace]\nmembers = []\n",
    "Cargo.lock": "# lock\n",
    "deny.toml": "[bans]\n",
    "rust-toolchain.toml": "[toolchain]\n",
    "crates/demo/src/lib.rs": "pub fn value() -> i32 { 1 }\n",
    "crates/demo/tests/basic.rs": BASIC_TEST,
    "crates/demo/goldens/out.golden": "gold\n",
    "crates/crpg-contracts/src/lib.rs": "pub struct Contract;\n",
    ".github/workflows/ci.yml": "name: ci\n",
    "docs/notes.md": "# Notes\n\nsome text\n",
}

DEFAULT_BRIEF = """# Brief — DEMO
Route: x
## Files you own
- `crates/demo/**`
- `crates/crpg-contracts/**`
- `docs/**`
## Out of scope
nothing
"""


def write_file(path: Path, content: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content, encoding="utf-8")


def run_git(repo: Path, *args: str, check: bool = True) -> subprocess.CompletedProcess:
    result = subprocess.run(
        ["git", "-C", str(repo), *args], capture_output=True, text=True, encoding="utf-8"
    )
    if check and result.returncode != 0:
        raise AssertionError(f"git {args} failed: {result.stderr}")
    return result


class VerifyCase(unittest.TestCase):
    """A temp git repo with the brief's layout, plus helpers to run the tool."""

    def setUp(self) -> None:
        self.tmp = Path(tempfile.mkdtemp(prefix="verify_task_"))
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.repo = self.tmp / "slot"
        self.repo.mkdir()
        self.stub = self.tmp / "stub.py"
        self.stub.write_text(STUB, encoding="utf-8")
        self.git("init", "-q")
        for path, content in BASE_FILES.items():
            write_file(self.repo / path, content)
        self.git("add", "-A")
        self.commit("base")
        self.base = self.git("rev-parse", "HEAD").stdout.strip()
        self.write_brief(DEFAULT_BRIEF)

    # -- plumbing ------------------------------------------------------

    def git(self, *args: str, check: bool = True) -> subprocess.CompletedProcess:
        return run_git(self.repo, *args, check=check)

    def commit(self, message: str) -> None:
        self.git(
            "-c", "user.name=T", "-c", "user.email=t@example.com",
            "commit", "-q", "-m", message,
        )

    def write_brief(self, text: str) -> None:
        (self.repo / "BRIEF.md").write_text(text, encoding="utf-8")

    def add(self, relative: str, content: str) -> None:
        write_file(self.repo / relative, content)

    def env(self, **overrides: str) -> dict[str, str]:
        env = os.environ.copy()
        env["CRPG_PREFLIGHT"] = f'"{sys.executable}" "{self.stub}" 0'
        env["CRPG_LINTS"] = f'"{sys.executable}" "{self.stub}" 0'
        env["CRPG_TODAY"] = TODAY
        env["PYTHONIOENCODING"] = "utf-8"
        env.update({key: str(value) for key, value in overrides.items()})
        return env

    def verify(
        self,
        *args: str,
        base: str | None = None,
        brief: Path | None = None,
        slot: Path | None = None,
        env_extra: dict[str, str] | None = None,
    ) -> subprocess.CompletedProcess:
        command = [
            sys.executable,
            str(SCRIPT),
            "A0-4",
            "--slot",
            str(slot or self.repo),
            "--base",
            base or self.base,
        ]
        if brief is not None:
            command += ["--brief", str(brief)]
        command += list(args)
        return subprocess.run(
            command,
            capture_output=True,
            text=True,
            encoding="utf-8",
            env=self.env(**(env_extra or {})),
            timeout=60,
        )

    # -- assertions ----------------------------------------------------

    def check_lines(self, result: subprocess.CompletedProcess) -> tuple[str, list[str]]:
        lines = result.stdout.splitlines()
        self.assertTrue(lines, result.stdout)
        self.assertTrue(lines[0].startswith(("PASS", "FAIL", "ERROR")), lines[0])
        return lines[0], lines[1:10]

    def assert_only_failed(self, result: subprocess.CompletedProcess, target: str) -> None:
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        verdict, checks = self.check_lines(result)
        self.assertTrue(verdict.startswith("FAIL "), verdict)
        by_name = {line.split(": ", 1)[0]: line.split(": ", 1)[1] for line in checks}
        self.assertEqual(sorted(by_name), sorted(CHECKS), result.stdout)
        for name in CHECKS:
            if name == target:
                self.assertTrue(by_name[name].startswith("FAILED"), (name, result.stdout))
            else:
                self.assertEqual(by_name[name], "ok", (name, result.stdout))

    def clean_change(self) -> None:
        self.add("crates/demo/src/clean.rs", "pub fn clean() -> i32 { 1 }\n")


class CleanChange(VerifyCase):
    def test_clean_change_passes_all_nine(self) -> None:
        self.clean_change()
        result = self.verify()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        _, checks = self.check_lines(result)
        statuses = [line.split(": ", 1)[1] for line in checks]
        self.assertEqual(statuses, ["ok"] * 9, result.stdout)


class PlantedFaults(VerifyCase):
    def test_scope_fault(self) -> None:
        self.clean_change()
        self.add("outside/thing.txt", "x\n")
        self.assert_only_failed(self.verify(), "scope")

    def test_gates_fault(self) -> None:
        self.clean_change()
        env = {"CRPG_PREFLIGHT": f'"{sys.executable}" "{self.stub}" 1'}
        self.assert_only_failed(self.verify(env_extra=env), "gates")

    def test_protected_fault(self) -> None:
        self.clean_change()
        self.add("crates/crpg-contracts/src/lib.rs", "pub struct Contract;\n// touched\n")
        self.assert_only_failed(self.verify(), "protected")

    def test_goldens_fault(self) -> None:
        self.clean_change()
        self.add("crates/demo/goldens/out.golden", "gold2\n")
        self.assert_only_failed(self.verify(), "goldens")

    def test_weakening_fault_removed_test(self) -> None:
        self.clean_change()
        self.add("crates/demo/tests/basic.rs", "#[test]\nfn a() { assert!(true); }\n")
        self.assert_only_failed(self.verify(), "weakening")

    def test_weakening_fault_deleted_file(self) -> None:
        self.clean_change()
        (self.repo / "crates/demo/tests/basic.rs").unlink()
        self.assert_only_failed(self.verify(), "weakening")

    def test_weakening_fault_ignore_added(self) -> None:
        self.clean_change()
        self.add("crates/demo/tests/basic.rs", BASIC_TEST + "#[ignore]\nfn c() {}\n")
        self.assert_only_failed(self.verify(), "weakening")

    def test_unsafe_fault(self) -> None:
        self.clean_change()
        self.add("crates/demo/src/danger.rs", "pub unsafe fn danger() {}\n")
        self.assert_only_failed(self.verify(), "unsafe")

    def test_lints_fault(self) -> None:
        self.clean_change()
        env = {"CRPG_LINTS": f'"{sys.executable}" "{self.stub}" 1'}
        self.assert_only_failed(self.verify(env_extra=env), "lints")

    def test_docsig_fault(self) -> None:
        self.clean_change()
        self.add("docs/notes.md", "# Notes\n\nchanged text\n")
        self.assert_only_failed(self.verify(), "docsig")

    def test_todo_fault(self) -> None:
        self.clean_change()
        self.add("crates/demo/src/stub_body.rs", "pub fn stub() { todo!() }\n")
        self.assert_only_failed(self.verify(), "todo")


class Grants(VerifyCase):
    def test_protected_grant_lifts_the_check(self) -> None:
        self.write_brief(
            DEFAULT_BRIEF + "## Permissions\n- protected: `crates/crpg-contracts/**`\n"
        )
        self.clean_change()
        self.add("crates/crpg-contracts/src/lib.rs", "pub struct Contract;\n// touched\n")
        result = self.verify()
        self.assertEqual(result.returncode, 0, result.stdout)

    def test_goldens_grant_lifts_the_check(self) -> None:
        self.write_brief(
            DEFAULT_BRIEF + "## Permissions\n- goldens: `crates/demo/goldens/**`\n"
        )
        self.clean_change()
        self.add("crates/demo/goldens/out.golden", "gold2\n")
        result = self.verify()
        self.assertEqual(result.returncode, 0, result.stdout)


class BriefParsing(VerifyCase):
    def test_colon_text_after_an_owned_path_is_ignored(self) -> None:
        self.write_brief(
            "# Brief\n## Files you own\n"
            "- `crates/demo/**`\n"
            "- `docs/notes.md`: the completion record section only\n"
        )
        self.clean_change()
        self.add(
            "docs/notes.md",
            "# Notes\n\n- 2026-10-16 (UTC) · test agent + x · signed now\n",
        )
        result = self.verify()
        self.assertEqual(result.returncode, 0, result.stdout)

    def test_a_matching_docsig_signature_passes(self) -> None:
        self.clean_change()
        self.add(
            "docs/notes.md",
            "# Notes\n\n- 2026-10-16 (UTC) · test agent + x · signed now\n",
        )
        result = self.verify()
        self.assertEqual(result.returncode, 0, result.stdout)


class ChangeSet(VerifyCase):
    def test_committed_change_is_in_the_changed_set(self) -> None:
        self.clean_change()
        self.git("add", "-A")
        self.commit("change")
        result = self.verify()
        self.assertEqual(result.returncode, 0, result.stdout)

    def test_staged_change_is_in_the_changed_set(self) -> None:
        self.clean_change()
        self.git("add", "crates/demo/src/clean.rs")
        result = self.verify()
        self.assertEqual(result.returncode, 0, result.stdout)

    def test_base_ahead_of_head_does_not_report_base_commits(self) -> None:
        # A commit reachable from the base but not from HEAD is not part of
        # base..HEAD; only the branch's own work may be reported.
        self.add("docs/notes.md", "# Notes\n\nbase-side change\n")
        self.git("add", "docs/notes.md")
        self.commit("base side")
        base_ahead = self.git("rev-parse", "HEAD").stdout.strip()
        self.git("reset", "--hard", "HEAD~1")
        self.clean_change()
        result = self.verify(base=base_ahead)
        self.assertEqual(result.returncode, 0, result.stdout)

    def test_brief_md_is_never_a_changed_file(self) -> None:
        self.clean_change()
        result = self.verify("--json")
        document = json.loads(result.stdout)
        self.assertNotIn("BRIEF.md", [item["path"] for item in document["files"]])


class SkipAndJson(VerifyCase):
    def test_skip_reports_a_check_as_skipped(self) -> None:
        self.clean_change()
        self.add("outside/thing.txt", "x\n")
        result = self.verify("--skip", "scope")
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertIn("scope: skipped", result.stdout)

    def test_skip_gates_avoids_running_the_gate(self) -> None:
        self.clean_change()
        env = {"CRPG_PREFLIGHT": f'"{sys.executable}" "{self.stub}" 1'}
        result = self.verify("--skip", "gates", env_extra=env)
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertIn("gates: skipped", result.stdout)

    def test_unknown_skip_name_is_exit_two(self) -> None:
        self.clean_change()
        result = self.verify("--skip", "bogus")
        self.assertEqual(result.returncode, 2, result.stdout)
        self.assertTrue(result.stdout.startswith("ERROR "), result.stdout)

    def test_json_is_one_valid_object(self) -> None:
        self.clean_change()
        result = self.verify("--json")
        self.assertEqual(result.returncode, 0, result.stdout)
        document = json.loads(result.stdout)
        self.assertEqual(document["tool"], "verify_task")
        self.assertEqual(document["version"], 1)
        self.assertEqual(document["task"], "A0-4")
        self.assertEqual(document["verdict"], "PASS")
        self.assertIsNone(document["error"])
        self.assertEqual(len(document["checks"]), 9)
        for check in document["checks"]:
            self.assertEqual(set(check), {"name", "status", "detail", "output"})
            self.assertEqual(check["status"], "ok")
            self.assertIsInstance(check["output"], list)
        changed = {item["path"]: item for item in document["files"]}
        self.assertIn("crates/demo/src/clean.rs", changed)
        self.assertEqual(changed["crates/demo/src/clean.rs"]["status"], "?")
        for item in document["files"]:
            self.assertIn(item["status"], ["A", "M", "D", "?"])
            self.assertIsInstance(item["added"], int)
            self.assertIsInstance(item["deleted"], int)

    def test_json_reports_a_failure(self) -> None:
        self.clean_change()
        self.add("outside/thing.txt", "x\n")
        result = self.verify("--json")
        self.assertEqual(result.returncode, 1, result.stdout)
        document = json.loads(result.stdout)
        self.assertEqual(document["verdict"], "FAIL")
        scope = next(c for c in document["checks"] if c["name"] == "scope")
        self.assertEqual(scope["status"], "failed")
        self.assertTrue(scope["output"])

    def test_json_reports_an_error(self) -> None:
        self.clean_change()
        result = self.verify("--json", "--base", "no-such-ref")
        self.assertEqual(result.returncode, 2, result.stdout)
        document = json.loads(result.stdout)
        self.assertEqual(document["verdict"], "ERROR")
        self.assertIsInstance(document["error"], str)


class OutputCap(VerifyCase):
    def test_caps_output_at_40_lines_unless_full(self) -> None:
        self.clean_change()
        for index in range(1, 6):
            self.add(f"outside/x{index}.txt", "x\n")
            self.add(f"crates/demo/src/f{index}.rs", "pub unsafe fn f() { todo!() }\n")
        self.add("docs/notes.md", "# Notes\n\nchanged\n")
        gate = self.tmp / "gate.py"
        gate.write_text(
            "for i in range(1, 51):\n    print(f'gate-line-{i}')\nraise SystemExit(1)\n",
            encoding="utf-8",
        )
        env = {"CRPG_PREFLIGHT": f'"{sys.executable}" "{gate}"'}
        result = self.verify(env_extra=env)
        self.assertEqual(result.returncode, 1, result.stdout)
        self.assertLessEqual(len(result.stdout.splitlines()), 40)
        self.assertNotIn("gate-line-50", result.stdout)
        full = self.verify("--full", env_extra=env)
        self.assertEqual(full.returncode, 1, full.stdout)
        self.assertGreater(len(full.stdout.splitlines()), 40)
        self.assertIn("gate-line-50", full.stdout)


class ErrorCases(VerifyCase):
    def assert_exit_two(self, result: subprocess.CompletedProcess) -> None:
        self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
        self.assertTrue(result.stdout.startswith("ERROR "), result.stdout)

    def test_slot_is_not_a_worktree(self) -> None:
        plain = self.tmp / "plain"
        plain.mkdir()
        self.assert_exit_two(self.verify(slot=plain))

    def test_brief_missing(self) -> None:
        (self.repo / "BRIEF.md").unlink()
        self.assert_exit_two(self.verify())

    def test_brief_without_owned_bullets(self) -> None:
        self.write_brief("# Brief\n## Files you own\n\n## Out of scope\nnothing\n")
        self.assert_exit_two(self.verify())

    def test_unknown_base(self) -> None:
        self.clean_change()
        self.assert_exit_two(self.verify(base="no-such-ref"))

    def test_gate_that_cannot_start(self) -> None:
        self.clean_change()
        env = {"CRPG_PREFLIGHT": "definitely-not-a-real-command-xyz"}
        self.assert_exit_two(self.verify(env_extra=env))

    def test_lint_that_cannot_start(self) -> None:
        self.clean_change()
        env = {"CRPG_LINTS": "definitely-not-a-real-command-xyz"}
        self.assert_exit_two(self.verify(env_extra=env))


if __name__ == "__main__":
    unittest.main()

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
    "doc-only",
    "protected",
    "goldens",
    "weakening",
    "unsafe",
    "lints",
    "docsig",
    "todo",
    "asserts",
]
TODAY = "2026-10-16"

STUB = "import sys\nsys.exit(int(sys.argv[1]))\n"

# A gate command that records that it ran; a doc-only slot must never start
# it, so the marker file is the assertion (no test ever runs cargo).
MARKER_STUB = (
    "import sys\nfrom pathlib import Path\n"
    "Path(sys.argv[1]).write_text('ran', encoding='utf-8')\n"
)

# A tiny unittest suite planted in a temp slot: its marker proves the gates'
# `unittest discover` command really ran, without cargo or the real tools tests.
PROBE_PASS = (
    "import unittest\nfrom pathlib import Path\n\n"
    "class Probe(unittest.TestCase):\n"
    "    def test_probe(self):\n"
    "        Path(%r).write_text('ran', encoding='utf-8')\n"
)

PROBE_FAIL = (
    "import unittest\n\n"
    "class Probe(unittest.TestCase):\n"
    "    def test_probe(self):\n"
    "        self.fail('planted tool gate fault')\n"
)

TOOL_BRIEF = """# Brief — TOOLS
Route: x
## Files you own
- `tools/**`
- `tasks/**`
## Out of scope
nothing
"""

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

    SLOT_NAME = "slot"

    def setUp(self) -> None:
        self.tmp = Path(tempfile.mkdtemp(prefix="verify_task_"))
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.repo = self.tmp / self.SLOT_NAME
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
        return lines[0], lines[1 : 1 + len(CHECKS)]

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

    def marker_command(self, name: str) -> tuple[str, Path]:
        """A command override that records that it ran, and its marker file."""
        script = self.tmp / f"{name}.py"
        script.write_text(MARKER_STUB, encoding="utf-8")
        marker = self.tmp / f"{name}.ran"
        return f'"{sys.executable}" "{script}" "{marker}"', marker


class CleanChange(VerifyCase):
    def test_clean_change_passes_every_check(self) -> None:
        self.clean_change()
        result = self.verify()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        _, checks = self.check_lines(result)
        statuses = [line.split(": ", 1)[1] for line in checks]
        self.assertEqual(statuses, ["ok"] * len(CHECKS), result.stdout)


class DocOnlySlots(VerifyCase):
    """Slots 3-6 are doc-only: no cargo gates, but the lints still run."""

    SLOT_NAME = "slot-4"

    def test_a_doc_change_passes_without_invoking_the_gates(self) -> None:
        self.add(
            "docs/notes.md",
            "# Notes\n\n- 2026-10-16 (UTC) · test agent + doc-only · signed\n",
        )
        gates, gate_marker = self.marker_command("gates")
        lints, lint_marker = self.marker_command("lints")
        env = {"CRPG_PREFLIGHT": gates, "CRPG_LINTS": lints}
        result = self.verify(env_extra=env)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertFalse(gate_marker.exists(), result.stdout)
        self.assertTrue(lint_marker.exists(), result.stdout)
        document = json.loads(self.verify("--json", env_extra=env).stdout)
        checks = {check["name"]: check for check in document["checks"]}
        self.assertEqual(checks["gates"]["status"], "ok", result.stdout)
        self.assertEqual(checks["gates"]["detail"], "skipped (doc-only slot)")
        self.assertEqual(checks["doc-only"]["status"], "ok", result.stdout)

    def test_a_code_change_fails_doc_only(self) -> None:
        self.clean_change()
        gates, gate_marker = self.marker_command("gates")
        result = self.verify(env_extra={"CRPG_PREFLIGHT": gates})
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertFalse(gate_marker.exists(), result.stdout)
        document = json.loads(self.verify("--json").stdout)
        checks = {check["name"]: check for check in document["checks"]}
        self.assertEqual(checks["doc-only"]["status"], "failed", result.stdout)
        self.assertIn("code: crates/demo/src/clean.rs", checks["doc-only"]["output"])

    def test_a_manifest_change_fails_doc_only(self) -> None:
        self.add("Cargo.toml", "[workspace]\nmembers = ['demo']\n")
        result = self.verify()
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("Cargo.toml", result.stdout.split("doc-only: ", 1)[1])

    def test_doc_only_is_skippable_like_the_other_checks(self) -> None:
        self.clean_change()
        result = self.verify("--skip", "doc-only")
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertIn("doc-only: skipped", result.stdout)


class DocOnlyToolTask(VerifyCase):
    """A doc-only slot with no crate owned still runs the tool unittests."""

    SLOT_NAME = "slot-4"

    def test_a_tool_change_runs_the_unittest_gates(self) -> None:
        self.write_brief(TOOL_BRIEF)
        self.add("tools/agents/x.py", "VALUE = 1\n")
        tests_marker = self.tmp / "tool_tests.ran"
        lint_marker = self.tmp / "tool_lint.ran"
        self.add("tools/tests/test_probe.py", PROBE_PASS % str(tests_marker))
        self.add("tools/lint/test_probe.py", PROBE_PASS % str(lint_marker))
        # No gate override: the real unittest commands must be the ones run.
        env = {"CRPG_PREFLIGHT": ""}
        result = self.verify(env_extra=env)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertTrue(tests_marker.exists(), result.stdout)
        self.assertTrue(lint_marker.exists(), result.stdout)
        document = json.loads(self.verify("--json", env_extra=env).stdout)
        checks = {check["name"]: check for check in document["checks"]}
        self.assertEqual(checks["gates"]["status"], "ok", result.stdout)
        self.assertEqual(checks["gates"]["detail"], "", result.stdout)

    def test_a_failing_tool_unittest_fails_the_gates(self) -> None:
        self.write_brief(TOOL_BRIEF)
        self.add("tools/agents/x.py", "VALUE = 1\n")
        self.add("tools/tests/test_probe.py", PROBE_FAIL)
        self.add("tools/lint/test_probe.py", PROBE_FAIL)
        env = {"CRPG_PREFLIGHT": ""}
        result = self.verify(env_extra=env)
        self.assert_only_failed(result, "gates")

    def test_a_doc_only_change_reports_no_tool_change(self) -> None:
        self.write_brief(TOOL_BRIEF)
        self.add(
            "tasks/x.md",
            "# Task\n\n- 2026-10-16 (UTC) · test agent + doc-only · signed\n",
        )
        gates, gate_marker = self.marker_command("tool_gates")
        result = self.verify(env_extra={"CRPG_PREFLIGHT": gates})
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertFalse(gate_marker.exists(), result.stdout)
        document = json.loads(self.verify("--json", env_extra={"CRPG_PREFLIGHT": gates}).stdout)
        checks = {check["name"]: check for check in document["checks"]}
        self.assertEqual(checks["gates"]["status"], "ok", result.stdout)
        self.assertEqual(checks["gates"]["detail"], "skipped (no tool change)")

    def test_a_crate_owning_doc_only_brief_runs_no_preflight(self) -> None:
        self.add(
            "docs/notes.md",
            "# Notes\n\n- 2026-10-16 (UTC) · test agent + doc-only · signed\n",
        )
        gates, gate_marker = self.marker_command("crate_gates")
        result = self.verify(env_extra={"CRPG_PREFLIGHT": gates})
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertFalse(gate_marker.exists(), result.stdout)
        document = json.loads(self.verify("--json", env_extra={"CRPG_PREFLIGHT": gates}).stdout)
        checks = {check["name"]: check for check in document["checks"]}
        self.assertEqual(checks["gates"]["status"], "ok", result.stdout)
        self.assertEqual(checks["gates"]["detail"], "skipped (doc-only slot)")


class RouteDocOnlySlot(VerifyCase):
    """A slot whose brief says `doc-only` is doc-only whatever it is called."""

    SLOT_NAME = "slot-1"

    def test_the_route_line_makes_a_slot_doc_only(self) -> None:
        self.write_brief(DEFAULT_BRIEF.replace("Route: x", "Route: doc-only"))
        self.add(
            "docs/notes.md",
            "# Notes\n\n- 2026-10-16 (UTC) · test agent + doc-only · signed\n",
        )
        gates, marker = self.marker_command("gates")
        result = self.verify(env_extra={"CRPG_PREFLIGHT": gates})
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertFalse(marker.exists(), result.stdout)
        self.assertIn("skipped (doc-only slot)", result.stdout)


class NonDocOnlySlot(VerifyCase):
    """Slot 1 is not doc-only: its gates command still runs (stubbed)."""

    SLOT_NAME = "slot-1"

    def test_slot_one_still_runs_the_gates(self) -> None:
        self.clean_change()
        gates, marker = self.marker_command("gates")
        result = self.verify(env_extra={"CRPG_PREFLIGHT": gates})
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertTrue(marker.exists(), result.stdout)
        self.assertIn("gates: ok", result.stdout)


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


class TodoScope(VerifyCase):
    def test_owned_fixture_todo_is_not_checked(self) -> None:
        # A bake-off fixture's unimplemented stubs are its point: only
        # crate sources are checked.
        self.write_brief(
            "# Brief\n## Files you own\n"
            "- `tools/agents/bakeoff/fixtures/**`\n"
        )
        self.add(
            "tools/agents/bakeoff/fixtures/x/src/lib.rs",
            "pub fn stub() { todo!() }\n",
        )
        result = self.verify()
        self.assertEqual(result.returncode, 0, result.stdout)

    def test_owned_crate_todo_still_fails(self) -> None:
        self.write_brief("# Brief\n## Files you own\n- `crates/crpg-x/**`\n")
        self.add("crates/crpg-x/src/lib.rs", "pub fn stub() { todo!() }\n")
        self.assert_only_failed(self.verify(), "todo")


class Docsig(VerifyCase):
    def test_deleted_md_file_needs_no_signature(self) -> None:
        self.clean_change()
        (self.repo / "docs/notes.md").unlink()
        result = self.verify()
        self.assertEqual(result.returncode, 0, result.stdout)


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
    def test_a_bullet_owns_every_backticked_path(self) -> None:
        # ``- `a`, `b`, and `c` `` must own all three paths, not just the first.
        self.write_brief(
            "# Brief\n## Files you own\n"
            "- `tools/tests/test_oc.py`, `tools/tests/fake_opencode.py`, "
            "and `tools/tests/fixtures/oc/**`\n"
        )
        self.add("tools/tests/test_oc.py", "# a\n")
        self.add("tools/tests/fake_opencode.py", "# b\n")
        self.add("tools/tests/fixtures/oc/case.txt", "c\n")
        result = self.verify()
        self.assertEqual(result.returncode, 0, result.stdout)

    def test_a_bullet_with_two_backticked_paths_owns_both(self) -> None:
        self.write_brief(
            "# Brief\n## Files you own\n"
            "- `tools/tests/test_verify_task.py` and "
            "`tools/tests/fixtures/verify/**`\n"
        )
        self.add("tools/tests/test_verify_task.py", "# a\n")
        self.add("tools/tests/fixtures/verify/case.txt", "c\n")
        result = self.verify()
        self.assertEqual(result.returncode, 0, result.stdout)

    def test_an_unbackticked_bullet_keeps_the_first_token_rule(self) -> None:
        self.write_brief(
            "# Brief\n## Files you own\n"
            "- tasks/T024.md: the completion record section only\n"
        )
        self.add(
            "tasks/T024.md",
            "# T024\n\n- 2026-10-16 (UTC) · test + x · signed now\n",
        )
        result = self.verify()
        self.assertEqual(result.returncode, 0, result.stdout)

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

    def test_a_long_note_after_the_path_still_owns_it(self) -> None:
        # The session-14 shape: the note follows the path in parentheses.
        self.write_brief(
            "# Brief\n## Files you own\n"
            "- `crates/demo/tests/a.rs` (new; the only file you create or change)\n"
        )
        self.add("crates/demo/tests/a.rs", BASIC_TEST)
        result = self.verify()
        self.assertEqual(result.returncode, 0, result.stdout)

    def test_a_note_inside_the_code_span_does_not_hide_the_path(self) -> None:
        # Prose in the same backticks as the path is not part of the path.
        self.write_brief(
            "# Brief\n## Files you own\n"
            "- `crates/demo/tests/a.rs (new; the only file you create or change)`\n"
        )
        self.add("crates/demo/tests/a.rs", BASIC_TEST)
        result = self.verify()
        self.assertEqual(result.returncode, 0, result.stdout)

    def test_a_full_stop_inside_the_span_does_not_alter_the_path(self) -> None:
        # A sentence's full stop may land inside the backticks; it is not
        # part of the path, or scope would see a different filename.
        self.write_brief(
            "# Brief\n## Files you own\n"
            "- `crates/demo/tests/a.rs.` (new; the only file you create or change)\n"
        )
        self.add("crates/demo/tests/a.rs", BASIC_TEST)
        result = self.verify()
        self.assertEqual(result.returncode, 0, result.stdout)

    def test_the_long_note_bullet_owns_exactly_its_path(self) -> None:
        self.write_brief(
            "# Brief\n## Files you own\n"
            "- `crates/demo/tests/a.rs` (new; the only file you create or change)\n"
        )
        self.add("crates/demo/tests/a.rs", BASIC_TEST)
        self.add("crates/demo/tests/b.rs", BASIC_TEST)
        result = self.verify("--json")
        self.assertEqual(result.returncode, 1, result.stdout)
        document = json.loads(result.stdout)
        scope = next(c for c in document["checks"] if c["name"] == "scope")
        self.assertIn("unowned: crates/demo/tests/b.rs", scope["output"])
        self.assertNotIn("unowned: crates/demo/tests/a.rs", scope["output"])

    def test_two_backticked_paths_and_a_note_own_both(self) -> None:
        self.write_brief(
            "# Brief\n## Files you own\n"
            "- `crates/demo/tests/a.rs` and `crates/demo/tests/b.rs` "
            "(new; the only files you create or change)\n"
        )
        self.add("crates/demo/tests/a.rs", BASIC_TEST)
        self.add("crates/demo/tests/b.rs", BASIC_TEST)
        result = self.verify()
        self.assertEqual(result.returncode, 0, result.stdout)

    def test_prose_outside_the_backticks_is_never_owned(self) -> None:
        # `and/or` in the note reads like a path, but it is prose: a change
        # to a file actually named `and/or` must stay out of scope.
        self.write_brief(
            "# Brief\n## Files you own\n"
            "- `crates/demo/tests/a.rs` (new; see and/or the notes beside it)\n"
        )
        self.add("crates/demo/tests/a.rs", BASIC_TEST)
        self.add("and/or", "not a path\n")
        result = self.verify()
        self.assertEqual(result.returncode, 1, result.stdout)
        self.assertIn("unowned: and/or", result.stdout)

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
        self.assertEqual(len(document["checks"]), len(CHECKS))
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

    def test_per_check_cap_binds_before_the_total_cap(self) -> None:
        # A single failed check with 30 lines proves the 12-line per-check cap;
        # with one check the total-cap path cannot be what truncates it.
        self.clean_change()
        gate = self.tmp / "gate30.py"
        gate.write_text(
            "for i in range(1, 31):\n    print(f'gate-line-{i}')\nraise SystemExit(1)\n",
            encoding="utf-8",
        )
        env = {"CRPG_PREFLIGHT": f'"{sys.executable}" "{gate}"'}
        result = self.verify(env_extra=env)
        self.assertEqual(result.returncode, 1, result.stdout)
        lines = result.stdout.splitlines()
        self.assertLessEqual(len(lines), 40)
        self.assertIn("gate-line-12", result.stdout)
        self.assertNotIn("gate-line-13", result.stdout)
        self.assertNotIn("gate-line-30", result.stdout)


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

    def test_malformed_preflight_override(self) -> None:
        self.clean_change()
        env = {"CRPG_PREFLIGHT": '"unbalanced'}
        self.assert_exit_two(self.verify(env_extra=env))

    def test_malformed_lints_override(self) -> None:
        self.clean_change()
        env = {"CRPG_LINTS": '"unbalanced'}
        self.assert_exit_two(self.verify(env_extra=env))


OTHER_BRIEF = """# Brief — A0-9 · another task
Route: x
## Files you own
- `docs/other/**`
## Out of scope
nothing
"""


class ReusedSlotBrief(VerifyCase):
    """A0-3d: a slot's BRIEF.md for another task must not decide scope."""

    def filed(self, text: str | None) -> dict[str, str]:
        repo = self.tmp / "repo"
        if text is not None:
            write_file(repo / ".agents" / "briefs" / "A0-4.md", text)
        return {"CRPG_REPO": str(repo)}

    def test_other_tasks_slot_brief_is_replaced_by_the_filed_brief(self) -> None:
        self.write_brief(OTHER_BRIEF)
        self.clean_change()
        result = self.verify(env_extra=self.filed(DEFAULT_BRIEF))
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_other_tasks_slot_brief_without_a_filed_brief_is_no_brief(self) -> None:
        self.write_brief(OTHER_BRIEF)
        self.clean_change()
        result = self.verify(env_extra=self.filed(None))
        self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
        self.assertTrue(result.stdout.startswith("ERROR "), result.stdout)
        self.assertIn("no-brief", result.stdout)
        self.assertIn("BRIEF.md", result.stdout)
        self.assertIn("A0-4.md", result.stdout)

    def test_review_brief_header_is_read_too(self) -> None:
        self.write_brief(OTHER_BRIEF.replace("# Brief —", "# Review brief —"))
        self.clean_change()
        result = self.verify(env_extra=self.filed(None))
        self.assertEqual(result.returncode, 2, result.stdout)
        self.assertIn("no-brief", result.stdout)

    def test_slot_brief_for_this_task_is_kept(self) -> None:
        self.write_brief(DEFAULT_BRIEF.replace("DEMO", "A0-4 ·"))
        self.clean_change()
        result = self.verify(env_extra=self.filed(None))
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_explicit_brief_wins(self) -> None:
        self.write_brief(OTHER_BRIEF)
        explicit = self.tmp / "explicit.md"
        explicit.write_text(DEFAULT_BRIEF, encoding="utf-8")
        self.clean_change()
        result = self.verify(brief=explicit, env_extra=self.filed(None))
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)


class DocsigDateRange(VerifyCase):
    """A0-3d: an entry written when the work began survives a UTC midnight."""

    def commit_on(self, day: str) -> None:
        stamp = f"{day}T12:00:00+00:00"
        self.git("add", "-A")
        subprocess.run(
            ["git", "-C", str(self.repo), "-c", "user.name=T",
             "-c", "user.email=t@example.com", "commit", "-q", "-m", "work"],
            check=True, capture_output=True,
            env={**os.environ, "GIT_COMMITTER_DATE": stamp, "GIT_AUTHOR_DATE": stamp},
        )

    def note(self, day: str) -> None:
        self.add("docs/notes.md", f"# Notes\n\n- {day} (UTC) · test agent + x · why\n")

    def test_entry_from_the_day_the_work_began_passes(self) -> None:
        self.note("2026-10-15")
        self.commit_on("2026-10-15")
        result = self.verify()  # CRPG_TODAY is 2026-10-16
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_entry_older_than_the_oldest_commit_fails_and_names_the_range(self) -> None:
        self.note("2026-10-14")
        self.commit_on("2026-10-15")
        result = self.verify()
        self.assertEqual(result.returncode, 1, result.stdout)
        self.assertIn("docsig: FAILED", result.stdout)
        self.assertIn("2026-10-15..2026-10-16", result.stdout)

    def test_entry_dated_after_today_fails(self) -> None:
        self.note("2026-10-17")
        self.commit_on("2026-10-15")
        self.assertEqual(self.verify().returncode, 1)

    def test_uncommitted_work_still_needs_todays_entry(self) -> None:
        self.note("2026-10-15")
        self.assertEqual(self.verify().returncode, 1)


class AssertsScope(VerifyCase):
    """A0-14: only new/changed tests count, in tests dirs and cfg(test) mods."""

    def test_empty_stub_file_fails(self) -> None:
        self.add("crates/demo/tests/empty.rs", "#[test]\nfn t01() {}\n")
        self.assert_only_failed(self.verify(), "asserts")

    def test_comment_only_body_fails(self) -> None:
        self.add(
            "crates/demo/tests/comments.rs",
            "#[test]\nfn t01() {\n    // nothing here\n    /* nor here */\n}\n",
        )
        self.assert_only_failed(self.verify(), "asserts")

    def test_block_comment_hiding_assert_fails(self) -> None:
        self.add(
            "crates/demo/tests/hidden.rs",
            "#[test]\nfn t01() {\n    // assert_eq!(1, 1);\n}\n",
        )
        self.assert_only_failed(self.verify(), "asserts")

    def test_string_literal_does_not_count(self) -> None:
        self.add(
            "crates/demo/tests/strings.rs",
            '#[test]\nfn t01() {\n    let _ = "assert_eq!(1, 1)";\n}\n',
        )
        self.assert_only_failed(self.verify(), "asserts")

    def test_empty_should_panic_still_fails(self) -> None:
        self.add(
            "crates/demo/tests/panicky.rs",
            "#[test]\n#[should_panic]\nfn t01() {}\n",
        )
        self.assert_only_failed(self.verify(), "asserts")

    def test_plain_function_without_test_is_ignored(self) -> None:
        self.add("crates/demo/tests/helper.rs", "fn t01() {}\n")
        result = self.verify()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_test_outside_cfg_test_is_ignored(self) -> None:
        self.add("crates/demo/src/loose.rs", "#[test]\nfn t01() {}\n")
        result = self.verify()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_cfg_test_module_empty_stub_fails(self) -> None:
        self.add(
            "crates/demo/src/inside.rs",
            "#[cfg(test)]\nmod tests {\n    #[test]\n    fn t01() {}\n}\n",
        )
        self.assert_only_failed(self.verify(), "asserts")

    def test_tokio_test_empty_stub_fails(self) -> None:
        self.add(
            "crates/demo/tests/async_empty.rs",
            "#[tokio::test]\nasync fn t01() {}\n",
        )
        self.assert_only_failed(self.verify(), "asserts")

    def test_tokio_test_with_assert_passes(self) -> None:
        self.add(
            "crates/demo/tests/async_ok.rs",
            "#[tokio::test]\nasync fn t01() { assert!(true); }\n",
        )
        result = self.verify()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_proptest_empty_block_fails(self) -> None:
        self.add(
            "crates/demo/tests/prop_empty.rs",
            "proptest! {\n    #test\n    fn t01(a in 0..10u8) {}\n}\n",
        )
        self.assert_only_failed(self.verify(), "asserts")

    def test_proptest_with_prop_assert_passes(self) -> None:
        self.add(
            "crates/demo/tests/prop_ok.rs",
            "proptest! {\n"
            "    #test\n"
            "    fn t01(a in 0..10u8) { prop_assert!(a < 20); }\n"
            "}\n",
        )
        result = self.verify()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_unchanged_legacy_empty_test_is_not_flagged(self) -> None:
        self.add("crates/demo/tests/legacy.rs", "#[test]\nfn legacy_empty() {}\n")
        self.git("add", "-A")
        self.commit("legacy empty stub")
        self.base = self.git("rev-parse", "HEAD").stdout.strip()
        self.clean_change()
        result = self.verify()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)


class AssertsMarkers(VerifyCase):
    """A0-14: every passing marker in Behaviour 2 lets a new test through."""

    def plant(self, body: str, prefix: str = "#[test]\n") -> None:
        self.add("crates/demo/tests/markers.rs", f"{prefix}fn t01() {{ {body} }}\n")

    def assert_passes(self, result: subprocess.CompletedProcess) -> None:
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_assert_eq_passes(self) -> None:
        self.plant("assert_eq!(1, 1);")
        self.assert_passes(self.verify())

    def test_each_assert_macro_passes(self) -> None:
        bodies = [
            "assert!(true);",
            "assert_ne!(1, 2);",
            "assert_matches!(Some(1), Some(_));",
            "debug_assert!(true);",
        ]
        for index, body in enumerate(bodies):
            with self.subTest(body=body):
                path = f"crates/demo/tests/assert_{index}.rs"
                self.add(path, f"#[test]\nfn t{index:02d}() {{ {body} }}\n")
        self.assert_passes(self.verify())

    def test_panic_and_unreachable_pass(self) -> None:
        self.add("crates/demo/tests/panic.rs", "#[test]\nfn t01() { panic!(\"boom\"); }\n")
        self.add(
            "crates/demo/tests/unreach.rs",
            "#[test]\nfn t02(v: Option<i32>) { unreachable!(); }\n",
        )
        self.assert_passes(self.verify())

    def test_unwrap_err_and_expect_err_pass(self) -> None:
        self.add(
            "crates/demo/tests/errs.rs",
            "#[test]\nfn t01() { Err::<i32, _>(1).unwrap_err(); }\n"
            "#[test]\nfn t02() { Err::<i32, _>(1).expect_err(\"must fail\"); }\n",
        )
        self.assert_passes(self.verify())

    def test_should_panic_passes(self) -> None:
        self.plant('panic!("boom");', prefix="#[test]\n#[should_panic]\n")
        self.assert_passes(self.verify())

    def test_helper_prefix_calls_pass(self) -> None:
        self.add(
            "crates/demo/tests/helpers.rs",
            "#[test]\nfn t01() { assert_roundtrip(&v); }\n"
            "#[test]\nfn t02() { check_invariants(&v); }\n"
            "#[test]\nfn t03() { expect_valid(&v); }\n",
        )
        self.assert_passes(self.verify())


class AssertsDetail(VerifyCase):
    """A0-14: detail names each failing test, capped at 20 plus a tail line."""

    def test_detail_names_path_line_and_fn(self) -> None:
        self.add(
            "crates/demo/tests/two.rs",
            "#[test]\nfn t01() {}\n#[test]\nfn t02() { assert!(true); }\n",
        )
        result = self.verify("--json")
        self.assertEqual(result.returncode, 1, result.stdout)
        document = json.loads(result.stdout)
        asserts = next(c for c in document["checks"] if c["name"] == "asserts")
        self.assertEqual(asserts["status"], "failed", result.stdout)
        self.assertIn("crates/demo/tests/two.rs:2 t01", asserts["output"])

    def test_detail_caps_at_twenty_plus_tail(self) -> None:
        stubs = "".join(f"#[test]\nfn t{i:02d}() {{}}\n" for i in range(21))
        self.add("crates/demo/tests/many.rs", stubs)
        result = self.verify("--json")
        document = json.loads(result.stdout)
        asserts = next(c for c in document["checks"] if c["name"] == "asserts")
        self.assertEqual(len(asserts["output"]), 21, result.stdout)
        self.assertEqual(asserts["output"][-1], "… and 1 more")


class AssertsSkipAndDocOnly(VerifyCase):
    SLOT_NAME = "slot-1"

    def test_skip_asserts_skips(self) -> None:
        self.add("crates/demo/tests/empty.rs", "#[test]\nfn t01() {}\n")
        result = self.verify("--skip", "asserts")
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertIn("asserts: skipped", result.stdout)

    def test_doc_only_diff_skips(self) -> None:
        self.write_brief(DEFAULT_BRIEF.replace("Route: x", "Route: doc-only"))
        self.add("crates/demo/tests/empty.rs", "#[test]\nfn t01() {}\n")
        result = self.verify("--json")
        document = json.loads(result.stdout)
        asserts = next(c for c in document["checks"] if c["name"] == "asserts")
        self.assertEqual(asserts["status"], "ok", result.stdout)
        self.assertEqual(asserts["detail"], "skipped (doc-only slot)")


if __name__ == "__main__":
    unittest.main()

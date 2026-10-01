#!/usr/bin/env python3
"""Subprocess tests for tools/preflight.sh and tools/preflight.ps1 (T031).

Each test copies a launcher into a temporary fake repository whose path
contains spaces, puts fake ``rustc``/``cargo``/``python3``/``python``/``git``
executables first (and alone) on PATH, runs the launcher as a real process,
and inspects the fake tools' JSON-lines log plus the process exit code. No
real gate, lint file or toolchain is ever invoked.

The fake Python forwards ``-c`` snippets (the launchers' own prerequisite and
metadata helpers) to the real interpreter so their logic is exercised; every
other invocation is only logged.

Launchers: Bash runs on POSIX hosts. PowerShell runs wherever ``pwsh`` is
found and is mandatory on Windows, where it is the native launcher.
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

TOOLS = Path(__file__).resolve().parent.parent
WINDOWS = os.name == "nt"
PWSH = shutil.which("pwsh")
BASH = None if WINDOWS else shutil.which("bash")

FAKE_TOOL = textwrap.dedent(
    r'''
    import json, os, subprocess, sys

    tool = sys.argv[1]
    argv = sys.argv[2:]
    with open(os.environ["FAKE_LOG"], "a", encoding="utf-8") as log:
        log.write(json.dumps({"tool": tool, "argv": argv, "cwd": os.getcwd()}) + "\n")

    if tool in ("python", "python3") and argv[:1] == ["-c"]:
        if "version_info" in argv[1] and os.environ.get("FAKE_PY_OLD"):
            sys.exit(1)
        sys.exit(subprocess.run([sys.executable, *argv]).returncode)

    failure = os.environ.get("FAKE_FAIL", "")
    if failure:
        words, _, code = failure.rpartition(":")
        expected = words.split(" ")
        if [tool, *argv][: len(expected)] == expected:
            print(f"fake {tool} failing as requested", file=sys.stderr)
            sys.exit(int(code))

    if tool == "cargo" and argv[:1] == ["metadata"]:
        with open(os.environ["FAKE_METADATA"], encoding="utf-8") as source:
            sys.stdout.write(source.read())
        sys.exit(0)
    print(f"fake {tool} ok")
    '''
)

METADATA = {
    "packages": [
        {"name": "crpg-core", "id": "path+file:///ws/crates/crpg-core#0.1.0"},
        {"name": "crpg-sim", "id": "path+file:///ws/crates/crpg-sim#0.1.0"},
        # A plausible crate name that is not a workspace member.
        {"name": "crpg-extra", "id": "registry+https://example.invalid#crpg-extra@1.0.0"},
    ],
    "workspace_members": [
        "path+file:///ws/crates/crpg-core#0.1.0",
        "path+file:///ws/crates/crpg-sim#0.1.0",
    ],
    "version": 1,
}


class Launcher:
    """One launcher variant: how to invoke it and spell its options."""

    def __init__(self, name: str, script: str, python: str) -> None:
        self.name = name
        self.script = script
        self.python = python

    def command(self, script_path: Path, args: list[str]) -> list[str]:
        if self.name == "bash":
            return [BASH, str(script_path), *args]
        return [PWSH, "-NoProfile", "-File", str(script_path), *args]

    def crate(self, package: str) -> list[str]:
        return ["--crate", package] if self.name == "bash" else ["-Crate", package]

    def check_only(self) -> list[str]:
        return ["--check-only"] if self.name == "bash" else ["-CheckOnly"]

    def help(self) -> list[str]:
        return ["--help"] if self.name == "bash" else ["-Help"]


def expected_gates(python: str, crate: str | None, check_only: bool) -> list[list[str]]:
    """The literal gate sequence, one argv per gate, tool name first."""
    gates = [["rustc", "-vV"], ["cargo", "metadata", "--no-deps", "--format-version", "1", "--locked"]]
    if not check_only:
        gates.append(["cargo", "fmt", "--all"])
    gates.append(["cargo", "fmt", "--all", "--", "--check"])
    if crate:
        gates.append(["cargo", "clippy", "-p", crate, "--all-targets", "--locked", "--", "-D", "warnings"])
        gates.append(["cargo", "test", "-p", crate, "--locked"])
    else:
        gates.append(["cargo", "clippy", "--workspace", "--all-targets", "--locked", "--", "-D", "warnings"])
        gates.append(["cargo", "test", "--workspace", "--locked"])
    gates += [
        [python, "tools/lint/deps.py"],
        [python, "tools/lint/determinism.py"],
        [python, "-m", "unittest", "discover", "-s", "tools/lint", "-p", "test_*.py"],
        ["cargo", "deny", "check"],
        ["git", "diff", "--check"],
    ]
    return gates


class PreflightCase:
    """Shared tests; mixed into one TestCase per available launcher."""

    launcher: Launcher

    def setUp(self) -> None:
        self.tmp = Path(tempfile.mkdtemp(prefix="preflight test "))
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.root = self.tmp / "repo with spaces"
        (self.root / "tools" / "lint").mkdir(parents=True)
        for name in ("Cargo.toml", "rust-toolchain.toml"):
            (self.root / name).write_text("# fake\n", encoding="utf-8")
        for name in ("deps.py", "determinism.py"):
            (self.root / "tools" / "lint" / name).write_text("raise SystemExit(99)\n", encoding="utf-8")
        self.script = self.root / "tools" / self.launcher.script
        shutil.copyfile(TOOLS / self.launcher.script, self.script)
        self.bin = self.tmp / "fake bin"
        self.bin.mkdir()
        fake = self.tmp / "fake_tool.py"
        fake.write_text(FAKE_TOOL, encoding="utf-8")
        for tool in ("rustc", "cargo", "python3", "python", "git"):
            self.write_shim(tool, fake)
        self.log = self.tmp / "log.jsonl"
        self.metadata = self.tmp / "metadata.json"
        self.metadata.write_text(json.dumps(METADATA), encoding="utf-8")
        self.elsewhere = self.tmp / "elsewhere"
        self.elsewhere.mkdir()

    def write_shim(self, tool: str, fake: Path) -> None:
        if WINDOWS:
            shim = self.bin / f"{tool}.cmd"
            shim.write_text(
                f'@echo off\r\n"{sys.executable}" "{fake}" {tool} %*\r\nexit /b %ERRORLEVEL%\r\n',
                encoding="utf-8",
            )
        else:
            shim = self.bin / tool
            shim.write_text(f'#!/bin/sh\nexec "{sys.executable}" "{fake}" {tool} "$@"\n', encoding="utf-8")
            shim.chmod(0o755)

    def remove_shim(self, tool: str) -> None:
        for candidate in self.bin.glob(f"{tool}*"):
            if candidate.stem == tool or candidate.name == tool:
                candidate.unlink()

    def run_launcher(self, args: list[str], **env_extra: str) -> subprocess.CompletedProcess:
        env = {
            "PATH": str(self.bin),
            "FAKE_LOG": str(self.log),
            "FAKE_METADATA": str(self.metadata),
        }
        passthrough = (
            "SYSTEMROOT", "WINDIR", "COMSPEC", "TEMP", "TMP", "HOME", "USERPROFILE",
            "APPDATA", "LOCALAPPDATA", "PROGRAMFILES", "PSModulePath",
        )
        for key in passthrough:
            if key in os.environ:
                env[key] = os.environ[key]
        if WINDOWS:
            env["PATHEXT"] = ".COM;.EXE;.BAT;.CMD"
            # PowerShell runs .cmd shims through cmd.exe, found via COMSPEC or
            # System32; System32 holds none of the faked tool names.
            system32 = Path(os.environ.get("SYSTEMROOT", r"C:\Windows")) / "System32"
            env["PATH"] = os.pathsep.join([str(self.bin), str(system32)])
        env.update(env_extra)
        return subprocess.run(
            self.launcher.command(self.script, args),
            cwd=self.elsewhere,
            env=env,
            capture_output=True,
            text=True,
            timeout=120,
        )

    def calls(self) -> list[dict]:
        if not self.log.exists():
            return []
        return [json.loads(line) for line in self.log.read_text(encoding="utf-8").splitlines()]

    def gates(self) -> list[list[str]]:
        """Logged gate invocations, excluding the launchers' own -c helpers."""
        return [[c["tool"], *c["argv"]] for c in self.calls() if c["argv"][:1] != ["-c"]]

    # -- success paths -------------------------------------------------

    def test_workspace_success_runs_every_gate_in_order(self) -> None:
        result = self.run_launcher([])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.gates(), expected_gates(self.launcher.python, None, False))
        self.assertTrue(result.stdout.rstrip().endswith("preflight: all workspace gates passed"))
        self.assertNotIn("crate gates", result.stdout)
        # Every tool ran from the resolved root, not the caller's directory.
        for call in self.calls():
            self.assertEqual(Path(call["cwd"]).resolve(), self.root.resolve())
        # The Python version prerequisite ran before any formatting gate.
        tools = [(c["tool"], c["argv"][:1]) for c in self.calls()]
        version = next(i for i, c in enumerate(self.calls()) if "version_info" in " ".join(c["argv"]))
        fmt = tools.index(("cargo", ["fmt"]))
        self.assertLess(version, fmt)

    def test_check_only_skips_rewrite_but_checks(self) -> None:
        result = self.run_launcher(self.launcher.check_only())
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.gates(), expected_gates(self.launcher.python, None, True))
        self.assertNotIn(["cargo", "fmt", "--all"], self.gates())

    def test_crate_mode_validates_membership_and_says_crate(self) -> None:
        result = self.run_launcher(self.launcher.crate("crpg-sim"))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.gates(), expected_gates(self.launcher.python, "crpg-sim", False))
        self.assertIn("all crate gates passed for crpg-sim", result.stdout)
        self.assertNotIn("workspace gates", result.stdout)
        # cargo deny still checks the whole graph in crate mode.
        self.assertIn(["cargo", "deny", "check"], self.gates())

    # -- argument and prerequisite errors (exit 2, no modifying gate) ---

    def assert_rejected_before_gates(self, result: subprocess.CompletedProcess) -> None:
        self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
        gates = self.gates()
        self.assertFalse(any(g[:2] == ["cargo", "fmt"] for g in gates), gates)
        self.assertNotIn("passed", result.stdout)

    def test_help_exits_zero_without_tools(self) -> None:
        result = self.run_launcher(self.launcher.help())
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("Usage", result.stdout)
        self.assertEqual(self.calls(), [])

    def test_invalid_arguments_exit_two_without_tools(self) -> None:
        crate = self.launcher.crate("crpg-sim")
        cases = [
            ["--bogus"],
            ["extra-positional"],
            [crate[0]],
            [crate[0], ""],
            [*crate, *crate],
            [*self.launcher.check_only(), *self.launcher.check_only()],
        ]
        for args in cases:
            with self.subTest(args=args):
                if self.log.exists():
                    self.log.unlink()
                result = self.run_launcher(args)
                self.assertEqual(result.returncode, 2, (args, result.stdout, result.stderr))
                self.assertEqual(self.calls(), [], args)

    def test_nonmember_and_unknown_packages_are_rejected(self) -> None:
        for package in ("crpg-extra", "crpg-nope", "crates/crpg-sim", "crpg-*"):
            with self.subTest(package=package):
                if self.log.exists():
                    self.log.unlink()
                result = self.run_launcher(self.launcher.crate(package))
                self.assert_rejected_before_gates(result)
                self.assertIn(["cargo", "metadata", "--no-deps", "--format-version", "1", "--locked"], self.gates())

    def test_malformed_metadata_exits_two(self) -> None:
        for text in ("not json", json.dumps({"packages": []}), json.dumps({"packages": [], "workspace_members": []})):
            with self.subTest(text=text):
                if self.log.exists():
                    self.log.unlink()
                self.metadata.write_text(text, encoding="utf-8")
                result = self.run_launcher([])
                self.assert_rejected_before_gates(result)

    def test_missing_prerequisites_exit_two(self) -> None:
        (self.root / "rust-toolchain.toml").unlink()
        result = self.run_launcher([])
        self.assert_rejected_before_gates(result)
        self.assertEqual(self.calls(), [])
        (self.root / "rust-toolchain.toml").write_text("# fake\n", encoding="utf-8")
        self.remove_shim("git")
        result = self.run_launcher([])
        self.assert_rejected_before_gates(result)
        self.assertEqual(self.calls(), [])

    def test_old_python_exits_two_before_formatting(self) -> None:
        result = self.run_launcher([], FAKE_PY_OLD="1")
        self.assert_rejected_before_gates(result)

    # -- gate failures (exit 1, nothing later runs) ---------------------

    def test_failures_stop_at_first_failed_gate(self) -> None:
        python = self.launcher.python
        sequence = expected_gates(python, None, False)
        for position in (0, 2, 4, 5, 6, 8, 9, 10):
            gate = sequence[position]
            with self.subTest(gate=gate):
                if self.log.exists():
                    self.log.unlink()
                result = self.run_launcher([], FAKE_FAIL=" ".join(gate) + ":7")
                self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
                self.assertEqual(self.gates(), sequence[: position + 1])
                self.assertIn("FAILED (exit 7)", result.stderr)
                self.assertIn(" ".join(gate[1:]), result.stderr)
                self.assertNotIn("gates passed", result.stdout)

    def test_metadata_command_failure_is_a_gate_failure(self) -> None:
        result = self.run_launcher([], FAKE_FAIL="cargo metadata:5")
        self.assertEqual(result.returncode, 1)
        self.assertIn("FAILED (exit 5)", result.stderr)
        self.assertEqual(self.gates(), expected_gates(self.launcher.python, None, False)[:2])


if BASH:

    class BashPreflightTest(PreflightCase, unittest.TestCase):
        launcher = Launcher("bash", "preflight.sh", "python3")


if PWSH:

    class PowerShellPreflightTest(PreflightCase, unittest.TestCase):
        launcher = Launcher("pwsh", "preflight.ps1", "python")

elif WINDOWS:

    class PowerShellRequiredOnWindows(unittest.TestCase):
        def test_pwsh_is_available(self) -> None:
            self.fail("pwsh (PowerShell 7) is required to test the native Windows launcher")


if __name__ == "__main__":
    unittest.main()

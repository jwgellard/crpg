#!/usr/bin/env python3
"""Subprocess tests for the Claude Code hooks (A0-6).

Each test runs the hook as a real process inside a temporary fake repository,
pipes one hook JSON object on stdin, and checks the exit code, stdout and
stderr. ``protect.py`` and ``handover.py`` resolve paths from files they never
have to touch, so the planted repository only needs a ``.git`` marker and the
files the case names. ``fmt_one.py`` needs a real rustfmt; that one case is
skipped when rustfmt is not on PATH.
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
PROTECT = AGENTS / "protect.py"
FMT_ONE = AGENTS / "fmt_one.py"
HANDOVER = AGENTS / "handover.py"

WINDOWS = os.name == "nt"
RUSTFMT = shutil.which("rustfmt")

HANDOVER_PREFIX = (
    "Handover from docs/PROJECT_STATE.md (re-injected after compaction):"
)


def run_hook(script: Path, payload, cwd: str, env: dict | None = None, text_input: str | None = None) -> subprocess.CompletedProcess:
    """Run one hook as a process, feeding *payload* as JSON on stdin."""
    stdin = text_input if text_input is not None else json.dumps(payload)
    return subprocess.run(
        [sys.executable, str(script)],
        input=stdin,
        capture_output=True,
        text=True,
        cwd=cwd,
        env=dict(os.environ) if env is None else env,
        timeout=30,
    )


class ProtectTests(unittest.TestCase):
    """Each protected rule and each escape hatch, as processes."""

    def setUp(self) -> None:
        self.tmp = Path(tempfile.mkdtemp(prefix="hooks test "))
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.root = self.tmp / "repo"
        (self.root / ".git").mkdir(parents=True)

    def write(self, rel: str) -> Path:
        path = self.root / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text("planted\n", encoding="utf-8")
        return path

    def protect(self, path, allow: str | None = None) -> subprocess.CompletedProcess:
        env = dict(os.environ)
        env.pop("CRPG_ALLOW_PROTECTED", None)
        if allow is not None:
            env["CRPG_ALLOW_PROTECTED"] = allow
        payload = {
            "hook_event_name": "PreToolUse",
            "tool_name": "Edit",
            "tool_input": {"file_path": str(path)},
        }
        return run_hook(PROTECT, payload, str(self.root), env=env)

    def test_every_protected_rule_blocks_and_names_its_rule(self) -> None:
        cases = [
            ("rust-toolchain.toml", "rust-toolchain.toml"),
            ("Cargo.lock", "Cargo.lock"),
            ("deny.toml", "deny.toml"),
            ("Cargo.toml", "Cargo.toml"),
            ("crates/crpg-contracts/src/lib.rs", "crpg-contracts"),
            (".github/workflows/ci.yml", ".github/workflows"),
            ("assets/goldens/level.bin", "goldens"),
            ("data/thing.golden", "*.golden"),
            ("replays/run.replay", "*.replay"),
        ]
        for rel, rule in cases:
            with self.subTest(rel=rel):
                path = self.write(rel)
                result = self.protect(path)
                self.assertEqual(result.returncode, 2, result.stderr)
                self.assertIn(rel, result.stderr)
                self.assertIn(rule, result.stderr)
                self.assertIn("is protected", result.stderr)
                self.assertIn("CRPG_ALLOW_PROTECTED", result.stderr)

    def test_crate_manifest_and_ordinary_source_pass(self) -> None:
        allowed = ["crates/crpg-core/Cargo.toml", "crates/crpg-core/src/lib.rs"]
        for rel in allowed:
            with self.subTest(rel=rel):
                path = self.write(rel)
                result = self.protect(path)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(result.stderr, "")

    def test_allowlist_lifts_exactly_the_listed_path(self) -> None:
        lock = self.write("Cargo.lock")
        manifest = self.write("Cargo.toml")
        result = self.protect(lock, allow="Cargo.lock")
        self.assertEqual(result.returncode, 0, result.stderr)
        # The other protected path is still blocked with the same allowlist.
        result = self.protect(manifest, allow="Cargo.lock")
        self.assertEqual(result.returncode, 2, result.stderr)

    def test_allowlist_accepts_a_comma_separated_list(self) -> None:
        manifest = self.write("Cargo.toml")
        lock = self.write("Cargo.lock")
        result = self.protect(manifest, allow="Cargo.lock, Cargo.toml")
        self.assertEqual(result.returncode, 0, result.stderr)
        result = self.protect(lock, allow="Cargo.lock, Cargo.toml")
        self.assertEqual(result.returncode, 0, result.stderr)

    @unittest.skipUnless(WINDOWS, "backslash paths only resolve on Windows")
    def test_backslash_path_is_protected(self) -> None:
        path = self.write("Cargo.lock")
        backslash = str(path).replace("/", "\\")
        result = self.protect(backslash)
        self.assertEqual(result.returncode, 2, result.stderr)
        self.assertIn("Cargo.lock", result.stderr)

    def test_sibling_worktree_path_is_protected(self) -> None:
        sibling = self.tmp / "slot-1"
        sibling.mkdir()
        # A worktree's .git is a file pointing at the main checkout.
        (sibling / ".git").write_text("gitdir: ../repo/.git/worktrees/slot-1\n", encoding="utf-8")
        planted = sibling / "Cargo.lock"
        planted.write_text("planted\n", encoding="utf-8")
        result = self.protect(planted)
        self.assertEqual(result.returncode, 2, result.stderr)
        self.assertIn("Cargo.lock", result.stderr)

    def test_malformed_stdin_exits_zero(self) -> None:
        result = run_hook(PROTECT, None, str(self.root), text_input="not json")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("hook error", result.stderr)

    def test_missing_file_path_exits_zero(self) -> None:
        payload = {"hook_event_name": "PreToolUse", "tool_input": {}}
        result = run_hook(PROTECT, payload, str(self.root))
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_path_outside_any_repository_exits_zero(self) -> None:
        outside = self.tmp / "loose"
        outside.mkdir()
        target = outside / "Cargo.lock"
        target.write_text("planted\n", encoding="utf-8")
        result = self.protect(target)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stderr, "")


class FmtOneTests(unittest.TestCase):
    """PostToolUse formatting, as processes."""

    def setUp(self) -> None:
        self.tmp = Path(tempfile.mkdtemp(prefix="hooks test "))
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.root = self.tmp / "repo"
        (self.root / ".git").mkdir(parents=True)

    def fmt(self, path) -> subprocess.CompletedProcess:
        payload = {
            "hook_event_name": "PostToolUse",
            "tool_name": "Edit",
            "tool_input": {"file_path": str(path)},
        }
        return run_hook(FMT_ONE, payload, str(self.root))

    @unittest.skipUnless(RUSTFMT, "rustfmt is not on PATH")
    def test_reformats_badly_formatted_rust_file(self) -> None:
        source = self.root / "messy.rs"
        source.write_text("fn main(){let x=1;println!(\"{}\",x);}\n", encoding="utf-8")
        before = source.read_text(encoding="utf-8")
        result = self.fmt(source)
        self.assertEqual(result.returncode, 0, result.stderr)
        after = source.read_text(encoding="utf-8")
        self.assertNotEqual(after, before)
        self.assertIn("fn main() {\n", after)

    def test_ignores_markdown_file(self) -> None:
        notes = self.root / "notes.md"
        notes.write_text("# not rust\nfn main(){}\n", encoding="utf-8")
        result = self.fmt(notes)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(notes.read_text(encoding="utf-8"), "# not rust\nfn main(){}\n")
        self.assertEqual(result.stderr, "")

    def test_malformed_stdin_exits_zero(self) -> None:
        result = run_hook(FMT_ONE, None, str(self.root), text_input="{")
        self.assertEqual(result.returncode, 0, result.stderr)


class HandoverTests(unittest.TestCase):
    """SessionStart handover re-injection, as processes."""

    def setUp(self) -> None:
        self.tmp = Path(tempfile.mkdtemp(prefix="hooks test "))
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.root = self.tmp / "repo"
        (self.root / ".git").mkdir(parents=True)
        self.state = self.root / "docs" / "PROJECT_STATE.md"

    def start(self, cwd=None, text_input: str | None = None) -> subprocess.CompletedProcess:
        payload = {"hook_event_name": "SessionStart", "cwd": str(cwd or self.root)}
        return run_hook(HANDOVER, payload, str(cwd or self.root), text_input=text_input)

    def test_prints_exactly_the_handover_block(self) -> None:
        self.state.parent.mkdir(parents=True, exist_ok=True)
        self.state.write_text(
            "# Project state\n\n## Handover\n- alpha\n- beta\n\n## Phase\nPhase body\n",
            encoding="utf-8",
        )
        result = self.start()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, HANDOVER_PREFIX + "\n## Handover\n- alpha\n- beta\n")

    def test_missing_block_prints_nothing(self) -> None:
        self.state.parent.mkdir(parents=True, exist_ok=True)
        self.state.write_text("# Project state\n\n## Phase\nPhase body\n", encoding="utf-8")
        result = self.start()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "")

    def test_missing_file_prints_nothing(self) -> None:
        result = self.start()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "")

    def test_malformed_stdin_falls_back_to_process_cwd(self) -> None:
        self.state.parent.mkdir(parents=True, exist_ok=True)
        self.state.write_text("## Handover\n- only\n## Phase\n", encoding="utf-8")
        result = self.start(text_input="not json")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, HANDOVER_PREFIX + "\n## Handover\n- only\n")


if __name__ == "__main__":
    unittest.main()

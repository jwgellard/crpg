#!/usr/bin/env python3
"""Tests for tools/agents/lost.py (A0-16, lesson L8).

Each test builds a throwaway git repository, commits a base, writes the head
state either to the working tree or to a commit, and runs the tool as a real
process. No network, no OpenCode, no cargo.
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

HERE = Path(__file__).resolve().parent
SCRIPT = HERE.parent / "agents" / "lost.py"

# A prose block long enough to clear the default 25-character floor.
BLOCK_A = (
    "- Keep the simulation core free of engine dependencies and platform "
    "branches."
)
BLOCK_B = (
    "- Use the file edit tool for documentation changes and never use a "
    "heredoc."
)


def write_file(path: Path, content: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content, encoding="utf-8")


class LostCase(unittest.TestCase):
    """A temp git repo plus helpers to commit fixtures and run the tool."""

    def setUp(self) -> None:
        self.tmp = Path(tempfile.mkdtemp(prefix="lost_"))
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.repo = self.tmp / "repo"
        self.repo.mkdir()
        self.git("init", "-q")

    # -- plumbing ------------------------------------------------------

    def git(self, *args: str, check: bool = True) -> subprocess.CompletedProcess:
        result = subprocess.run(
            ["git", "-C", str(self.repo), *args],
            capture_output=True,
            text=True,
            encoding="utf-8",
        )
        if check and result.returncode != 0:
            raise AssertionError(f"git {args} failed: {result.stderr}")
        return result

    def commit(self, message: str) -> None:
        self.git("add", "-A")
        self.git(
            "-c",
            "user.name=T",
            "-c",
            "user.email=t@example.com",
            "commit",
            "-q",
            "-m",
            message,
        )

    def write(self, relative: str, content: str) -> None:
        write_file(self.repo / relative, content)

    def base(self, files: dict[str, str]) -> str:
        """Write *files*, commit them, and return the new base hash."""
        for relative, content in files.items():
            self.write(relative, content)
        self.commit("base")
        return self.git("rev-parse", "HEAD").stdout.strip()

    def head(self, files: dict[str, str]) -> None:
        for relative, content in files.items():
            self.write(relative, content)
        self.commit("head")

    def lost(self, *args: str) -> subprocess.CompletedProcess:
        env = os.environ.copy()
        env["PYTHONIOENCODING"] = "utf-8"
        return subprocess.run(
            [sys.executable, str(SCRIPT), *args],
            cwd=self.repo,
            capture_output=True,
            text=True,
            encoding="utf-8",
            env=env,
            timeout=60,
        )

    def verdict(self, result: subprocess.CompletedProcess) -> str:
        lines = result.stdout.splitlines()
        self.assertTrue(lines, result.stdout + result.stderr)
        return lines[0]


class MoveAndDelete(LostCase):
    def test_pure_move_from_agents_to_architecture_doc_passes(self) -> None:
        base = self.base({"AGENTS.md": f"# AGENTS\n\n{BLOCK_A}\n"})
        self.head(
            {
                "AGENTS.md": "# AGENTS\n",
                "docs/architecture/demo.md": f"# Demo\n\n{BLOCK_A}\n",
            }
        )
        result = self.lost(
            "--base",
            base,
            "--from",
            "AGENTS.md",
            "--to",
            "docs/architecture/demo.md",
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(self.verdict(result), "PASS lost: 1 blocks checked, 0 lost")

    def test_deleted_block_fails_and_names_path_and_line(self) -> None:
        base = self.base({"AGENTS.md": f"# AGENTS\n\n{BLOCK_A}\n"})
        self.head({"AGENTS.md": "# AGENTS\n"})
        result = self.lost(
            "--base",
            base,
            "--from",
            "AGENTS.md",
            "--to",
            "docs/architecture/demo.md",
        )
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertEqual(self.verdict(result), "FAIL lost: 1 blocks checked, 1 lost")
        self.assertIn("0.00  AGENTS.md:3", result.stdout)
        self.assertIn(BLOCK_A[:40], result.stdout)

    def test_missing_paths_count_as_empty_text(self) -> None:
        base = self.base({"AGENTS.md": f"# AGENTS\n\n{BLOCK_A}\n"})
        # Missing on the base side: nothing to check.
        result = self.lost("--base", base, "--from", "docs/absent.md")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(self.verdict(result), "PASS lost: 0 blocks checked, 0 lost")
        # Missing on the head side: the base block is gone.
        self.head({"AGENTS.md": "# AGENTS\n"})
        result = self.lost("--base", base, "--from", "AGENTS.md")
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertEqual(self.verdict(result), "FAIL lost: 1 blocks checked, 1 lost")


class Rewording(LostCase):
    def test_reworded_block_fails_but_lightly_rewrapped_block_passes(self) -> None:
        words = (
            "The replay determinism baseline is compared with its own hash on "
            "each platform and never across platforms."
        )
        base = self.base({"AGENTS.md": f"# AGENTS\n\n{words}\n"})
        rewritten = (
            "Deterministic replay compares generated hashes only inside one "
            "operating system family, by design, forever."
        )
        self.head({"AGENTS.md": "# AGENTS\n", "docs/arch.md": rewritten + "\n"})
        result = self.lost(
            "--base", base, "--from", "AGENTS.md", "--to", "docs/arch.md"
        )
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("FAIL lost: 1 blocks checked, 1 lost", result.stdout)

        rewrapped = (
            "The replay determinism baseline is compared\n"
            "with its own hash on each platform\n"
            "and never across platforms.\n"
        )
        self.head({"AGENTS.md": "# AGENTS\n", "docs/arch.md": rewrapped})
        result = self.lost(
            "--base", base, "--from", "AGENTS.md", "--to", "docs/arch.md"
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(self.verdict(result), "PASS lost: 1 blocks checked, 0 lost")


class MinCharsAndThreshold(LostCase):
    def test_short_blocks_under_min_chars_are_ignored(self) -> None:
        base = self.base({"AGENTS.md": "# AGENTS\n\n- Run `cargo fmt`.\n"})
        self.head({"AGENTS.md": "# AGENTS\n"})
        result = self.lost("--base", base, "--from", "AGENTS.md")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(self.verdict(result), "PASS lost: 0 blocks checked, 0 lost")
        result = self.lost("--base", base, "--from", "AGENTS.md", "--min-chars", "5")
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertEqual(self.verdict(result), "FAIL lost: 1 blocks checked, 1 lost")

    def test_threshold_changes_the_verdict(self) -> None:
        words = (
            "alpha bravo charlie delta echo foxtrot golf hotel india juliet "
            "kilo lima mike november oscar papa quebec romeo sierra tango "
            "uniform victor whiskey xray yankee"
        )
        base = self.base({"AGENTS.md": f"# AGENTS\n\n{words}\n"})
        head_words = " ".join(words.split()[:12])
        self.head({"AGENTS.md": "# AGENTS\n", "docs/arch.md": head_words + "\n"})
        args = ("--base", base, "--from", "AGENTS.md", "--to", "docs/arch.md")
        result = self.lost(*args)
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("0.40  AGENTS.md:3", result.stdout)
        result = self.lost(*args, "--threshold", "0.4")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(self.verdict(result), "PASS lost: 1 blocks checked, 0 lost")


class Sources(LostCase):
    def test_to_paths_other_than_from_are_concatenated(self) -> None:
        base = self.base(
            {"AGENTS.md": f"# AGENTS\n\n{BLOCK_A}\n", "docs/notes.md": f"# Notes\n\n{BLOCK_B}\n"}
        )
        self.head(
            {
                "AGENTS.md": "# AGENTS\n",
                "docs/notes.md": "# Notes\n",
                "docs/architecture/one.md": f"# One\n\n{BLOCK_A}\n",
                "docs/architecture/two.md": f"# Two\n\n{BLOCK_B}\n",
            }
        )
        result = self.lost(
            "--base",
            base,
            "--from",
            "AGENTS.md",
            "--from",
            "docs/notes.md",
            "--to",
            "docs/architecture/one.md",
            "--to",
            "docs/architecture/two.md",
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(self.verdict(result), "PASS lost: 2 blocks checked, 0 lost")

    def test_to_defaults_to_from(self) -> None:
        base = self.base({"AGENTS.md": f"# AGENTS\n\n{BLOCK_A}\n"})
        result = self.lost("--base", base, "--from", "AGENTS.md")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(self.verdict(result), "PASS lost: 1 blocks checked, 0 lost")

    def test_head_working_tree_and_head_ref_agree(self) -> None:
        base = self.base({"AGENTS.md": f"# AGENTS\n\n{BLOCK_A}\n"})
        self.head(
            {
                "AGENTS.md": "# AGENTS\n",
                "docs/arch.md": f"# Arch\n\n{BLOCK_A}\n",
            }
        )
        args = ("--base", base, "--from", "AGENTS.md", "--to", "docs/arch.md")
        working = self.lost(*args)
        self.assertEqual(working.returncode, 0, working.stdout + working.stderr)
        reference = self.lost(*args, "--head", "HEAD")
        self.assertEqual(reference.returncode, 0, reference.stdout + reference.stderr)
        self.assertEqual(self.verdict(working), self.verdict(reference))
        self.assertEqual(self.verdict(working), "PASS lost: 1 blocks checked, 0 lost")

    def test_bad_ref_exits_2(self) -> None:
        base = self.base({"AGENTS.md": f"# AGENTS\n\n{BLOCK_A}\n"})
        result = self.lost("--base", "no-such-ref", "--from", "AGENTS.md")
        self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
        self.assertIn("bad ref", result.stderr)
        result = self.lost(
            "--base", base, "--head", "no-such-ref", "--from", "AGENTS.md"
        )
        self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
        self.assertIn("bad ref", result.stderr)


class BlocksAndOutput(LostCase):
    def test_block_boundaries_follow_headings_lists_and_table_rows(self) -> None:
        prose = (
            "Paragraph one keeps the replay baseline deterministic on each "
            "platform."
        )
        item = "- List item keeps documentation changes signed with a dated entry."
        numbered = (
            "1. Numbered item keeps the toolchain pinned for every release gate."
        )
        row_one = "| row one | keeps the protocol lane free of engine dependencies |"
        row_two = (
            "| row two | keeps authoritative state behind the transport boundary |"
        )
        base = self.base(
            {
                "AGENTS.md": (
                    "# AGENTS\n\n"
                    f"{prose}\n\n{item}\n{numbered}\n{row_one}\n{row_two}\n"
                )
            }
        )
        self.head(
            {
                "AGENTS.md": "# AGENTS\n",
                "docs/arch.md": f"# Arch\n\n{item}\n{numbered}\n{row_one}\n{row_two}\n",
            }
        )
        result = self.lost(
            "--base", base, "--from", "AGENTS.md", "--to", "docs/arch.md"
        )
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertEqual(self.verdict(result), "FAIL lost: 5 blocks checked, 1 lost")
        self.assertIn("AGENTS.md:3", result.stdout)

    def test_detail_text_is_capped_at_100_chars(self) -> None:
        block = "- " + "documentation " * 20 + "end."
        base = self.base({"AGENTS.md": f"# AGENTS\n\n{block}\n"})
        self.head({"AGENTS.md": "# AGENTS\n"})
        result = self.lost(
            "--base", base, "--from", "AGENTS.md", "--to", "docs/arch.md"
        )
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        detail = result.stdout.splitlines()[1]
        self.assertEqual(detail, f"  0.00  AGENTS.md:3  {block[:100]}")
        self.assertEqual(len(detail.rsplit("  ", 1)[-1]), 100)


class Output(LostCase):
    def test_json_shape(self) -> None:
        base = self.base({"AGENTS.md": f"# AGENTS\n\n{BLOCK_A}\n"})
        self.head({"AGENTS.md": "# AGENTS\n"})
        args = ("--base", base, "--from", "AGENTS.md", "--to", "docs/arch.md")
        result = self.lost(*args, "--json")
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        document = json.loads(result.stdout)
        self.assertEqual(set(document), {"verdict", "checked", "lost"})
        self.assertEqual(document["verdict"], "FAIL")
        self.assertEqual(document["checked"], 1)
        self.assertEqual(len(document["lost"]), 1)
        entry = document["lost"][0]
        self.assertEqual(set(entry), {"path", "line", "ratio", "text"})
        self.assertEqual(entry["path"], "AGENTS.md")
        self.assertEqual(entry["line"], 3)
        self.assertEqual(entry["ratio"], 0.0)
        self.assertTrue(entry["text"].startswith("- Keep"))

        self.write("AGENTS.md", "# AGENTS\n")
        self.write("docs/arch.md", f"# Arch\n\n{BLOCK_A}\n")
        result = self.lost(*args, "--json")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        document = json.loads(result.stdout)
        self.assertEqual(document, {"verdict": "PASS", "checked": 1, "lost": []})

    def test_forty_detail_lines_then_more(self) -> None:
        blocks = [
            f"- Block {number:02d} keeps a distinct sentence about documentation "
            "changes and signed agent log entries."
            for number in range(1, 46)
        ]
        base = self.base({"AGENTS.md": "# AGENTS\n\n" + "\n".join(blocks) + "\n"})
        self.head({"AGENTS.md": "# AGENTS\n"})
        result = self.lost(
            "--base", base, "--from", "AGENTS.md", "--to", "docs/arch.md"
        )
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        lines = result.stdout.splitlines()
        self.assertEqual(lines[0], "FAIL lost: 45 blocks checked, 45 lost")
        details = [line for line in lines if line.startswith("  ")]
        self.assertEqual(len(details), 40, result.stdout)
        self.assertEqual(lines[-1], "… and 5 more")


if __name__ == "__main__":
    unittest.main()

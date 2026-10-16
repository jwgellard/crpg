#!/usr/bin/env python3
"""Tests for tools/agents/recall.py (A0-5).

Builds a small throwaway git repository with one file per source weight, then
checks ranking, whole-word matching, determinism, the text caps and the JSON
shape. CLI cases copy the scripts in so the tool's own-checkout root resolves
to the throwaway repository.
"""

from __future__ import annotations

import json
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

HERE = Path(__file__).resolve().parent
AGENTS = HERE.parent / "agents"

sys.path.insert(0, str(AGENTS))
import recall  # noqa: E402

BRIEF_PY = AGENTS / "brief.py"
RECALL_PY = AGENTS / "recall.py"


def write(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8", newline="")


def git(root: Path, *args: str) -> subprocess.CompletedProcess:
    return subprocess.run(
        ["git", "-C", str(root), *args],
        capture_output=True,
        text=True,
        encoding="utf-8",
    )


class RecallRepo:
    """A temp git repo with copies of the scripts and one file per source."""

    def __init__(self, commits: bool = True) -> None:
        self.tmp = Path(tempfile.mkdtemp(prefix="recall_"))
        self.root = self.tmp / "repo"
        (self.root / "tools" / "agents").mkdir(parents=True)
        for source in (BRIEF_PY, RECALL_PY):
            shutil.copy(source, self.root / "tools" / "agents" / source.name)
        subprocess.run(["git", "init", "-q", str(self.root)], check=True)
        if commits:
            git(self.root, "config", "user.name", "T")
            git(self.root, "config", "user.email", "t@example.com")

    def close(self) -> None:
        shutil.rmtree(self.tmp, True)

    def add(self, relative: str, text: str) -> None:
        write(self.root / relative, text)

    def commit(self, message: str) -> None:
        git(self.root, "add", "-A")
        git(
            self.root,
            "-c",
            "user.name=T",
            "-c",
            "user.email=t@example.com",
            "commit",
            "-q",
            "-m",
            message,
        )

    def run(self, *args: str) -> subprocess.CompletedProcess:
        return subprocess.run(
            [sys.executable, str(self.root / "tools" / "agents" / "recall.py"), *args],
            capture_output=True,
            text=True,
            encoding="utf-8",
            cwd=str(self.root),
        )

    def one_of_each(self) -> None:
        self.add("docs/agents/LESSONS.md", "# Lessons\n\n- a zebra lesson\n")
        self.add("tasks/E001-events.md", "# E001\n\n## Task\n\nzebra record\n")
        self.add("docs/adr/0001-demo.md", "# ADR-0001\n\n## Decision\n\nzebra decision\n")
        self.add("tasks/DECISIONS-2026-01-01.md", "# Decisions\n\n## D1\n\nzebra batch\n")
        self.add("tasks/T001-demo.md", "# T001\n\n## Task\n\nzebra task\n")
        self.commit("zebra commit")


class Ranking(unittest.TestCase):
    def setUp(self) -> None:
        self.repo = RecallRepo()
        self.addCleanup(self.repo.close)
        self.repo.one_of_each()

    def test_weights_and_tie_break_order(self) -> None:
        hits = recall.search(["zebra"], str(self.repo.root))
        ordered = [(hit["path"], hit["score"]) for hit in hits]
        self.assertEqual(
            ordered,
            [
                ("docs/agents/LESSONS.md", 3),
                ("tasks/E001-events.md", 3),
                ("docs/adr/0001-demo.md", 2),
                ("tasks/DECISIONS-2026-01-01.md", 2),
                ("git", 1),
                ("tasks/T001-demo.md", 1),
            ],
        )
        self.assertEqual(hits[4]["kind"], "git")
        self.assertEqual(hits[4]["subject"], "zebra commit")

    def test_distinct_words_multiply_the_score(self) -> None:
        self.repo.add("docs/agents/LESSONS.md", "# Lessons\n\n- a zebra yak lesson\n")
        hits = recall.search(["zebra", "yak"], str(self.repo.root))
        self.assertEqual(hits[0]["path"], "docs/agents/LESSONS.md")
        self.assertEqual(hits[0]["score"], 6)
        # A one-word line in the same source still scores its single weight.
        self.assertEqual(recall.search(["yak"], str(self.repo.root))[0]["score"], 3)

    def test_whole_word_matching(self) -> None:
        self.repo.add("tasks/T002-demo.md", "# T002\n\n## Task\n\nnetwork of networks\n")
        hits = recall.search(["net"], str(self.repo.root))
        self.assertEqual(hits, [])

    def test_deterministic_order(self) -> None:
        first = recall.search(["zebra"], str(self.repo.root))
        second = recall.search(["zebra"], str(self.repo.root))
        self.assertEqual(first, second)

    def test_none_is_an_empty_list(self) -> None:
        self.assertEqual(recall.search(["nonexistentword"], str(self.repo.root)), [])

    def test_words_are_deduplicated_case_insensitively(self) -> None:
        once = recall.search(["zebra"], str(self.repo.root))
        twice = recall.search(["Zebra", "zebra"], str(self.repo.root))
        self.assertEqual(len(once), 6)
        self.assertEqual(len(twice), len(once))

    def test_git_matches_the_subject_only(self) -> None:
        # A hash is hexadecimal: `abc1234` must not hit the abbreviated hash.
        log = subprocess.CompletedProcess(
            args=[], returncode=0, stdout="abc1234 a plain subject\n", stderr=""
        )
        with mock.patch.object(recall, "run_git", return_value=log):
            self.assertEqual(recall.search(["abc1234"], str(self.repo.root)), [])
            hits = recall.search(["plain"], str(self.repo.root))
        self.assertEqual(len(hits), 1)
        self.assertEqual(hits[0]["hash"], "abc1234")
        self.assertEqual(hits[0]["subject"], "a plain subject")


class HitLineFormat(unittest.TestCase):
    def setUp(self) -> None:
        self.repo = RecallRepo(commits=False)
        self.addCleanup(self.repo.close)

    def test_file_hit_is_path_line_heading_text(self) -> None:
        self.repo.add("tasks/T002-demo.md", "# T002\n\n## Task\n\nquokka " + "x" * 300 + "\n")
        hits = recall.search(["quokka"], str(self.repo.root))
        self.assertEqual(len(hits), 1)
        line = recall.format_hit(hits[0])
        self.assertTrue(line.startswith("tasks/T002-demo.md:5 · Task · quokka "), line)
        self.assertEqual(len(line.split(" · ")[-1]), 120)
        self.assertEqual(line, recall.format_hit(hits[0]))

    def test_git_hit_is_hash_then_subject(self) -> None:
        hit = {
            "kind": "git",
            "path": "git",
            "line": 0,
            "heading": "",
            "text": "abc1234 zebra subject",
            "weight": 1,
            "score": 1,
            "hash": "abc1234",
            "subject": "zebra subject",
        }
        self.assertEqual(recall.format_hit(hit), "git abc1234 · zebra subject")

    def test_printed_line_matches_the_hit(self) -> None:
        self.repo.add("tasks/T002-demo.md", "# T002\n\n## Task\n\nquokka here\n")
        self.repo.commit("quokka commit")
        result = self.repo.run("quokka")
        self.assertEqual(result.returncode, 0, result.stderr)
        hits = recall.search(["quokka"], str(self.repo.root))
        for hit, line in zip(hits, result.stdout.splitlines()[1:]):
            self.assertEqual(line, recall.format_hit(hit))


class CliBehaviour(unittest.TestCase):
    def setUp(self) -> None:
        self.repo = RecallRepo()
        self.addCleanup(self.repo.close)
        self.repo.one_of_each()

    def test_found_and_none_exit_zero(self) -> None:
        found = self.repo.run("zebra")
        self.assertEqual(found.returncode, 0, found.stderr)
        self.assertTrue(found.stdout.startswith("FOUND 6 hits for zebra"), found.stdout)
        none = self.repo.run("nonexistentword")
        self.assertEqual(none.returncode, 0, none.stderr)
        self.assertTrue(none.stdout.startswith("NONE for nonexistentword"), none.stdout)

    def test_repeated_words_score_once(self) -> None:
        result = self.repo.run("Zebra", "zebra")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(result.stdout.startswith("FOUND 6 hits for Zebra"), result.stdout)
        document = json.loads(self.repo.run("Zebra", "zebra", "--json").stdout)
        self.assertEqual(document["words"], ["Zebra"])
        self.assertEqual(document["count"], 6)

    def test_default_limit_and_json(self) -> None:
        self.repo.add("tasks/T002-demo.md", "# T002\n\n## Task\n\n" + "quokka\n" * 50)
        limited = self.repo.run("quokka")
        self.assertEqual(limited.returncode, 0, limited.stderr)
        self.assertEqual(len(limited.stdout.splitlines()), 16)
        document = json.loads(self.repo.run("quokka", "--json").stdout)
        self.assertEqual(document["tool"], "recall")
        self.assertEqual(document["version"], 1)
        self.assertEqual(document["words"], ["quokka"])
        self.assertEqual(document["count"], len(document["hits"]))
        self.assertEqual(document["count"], 50)
        for hit in document["hits"]:
            self.assertEqual(
                set(hit),
                {"kind", "path", "line", "heading", "text", "weight", "score"},
            )

    def test_40_line_cap_and_full(self) -> None:
        self.repo.add("tasks/T002-demo.md", "# T002\n\n## Task\n\n" + "quokka\n" * 100)
        capped = self.repo.run("quokka", "--limit", "100")
        self.assertEqual(capped.returncode, 0, capped.stderr)
        self.assertLessEqual(len(capped.stdout.splitlines()), 40)
        full = self.repo.run("quokka", "--limit", "100", "--full")
        self.assertEqual(full.returncode, 0, full.stderr)
        self.assertGreater(len(full.stdout.splitlines()), 40)
        self.assertIn("quokka", full.stdout)

    def test_usage_error_exits_two(self) -> None:
        result = self.repo.run()
        self.assertEqual(result.returncode, 2, result.stdout + result.stderr)


class OutsideGit(unittest.TestCase):
    def test_outside_a_checkout_exits_two(self) -> None:
        tmp = Path(tempfile.mkdtemp(prefix="recall_plain_"))
        self.addCleanup(shutil.rmtree, tmp, True)
        (tmp / "tools" / "agents").mkdir(parents=True)
        for source in (BRIEF_PY, RECALL_PY):
            shutil.copy(source, tmp / "tools" / "agents" / source.name)
        result = subprocess.run(
            [sys.executable, str(tmp / "tools" / "agents" / "recall.py"), "zebra"],
            capture_output=True,
            text=True,
            encoding="utf-8",
            cwd=str(tmp),
        )
        self.assertEqual(result.returncode, 2, result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main()

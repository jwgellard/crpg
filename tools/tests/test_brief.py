#!/usr/bin/env python3
"""Tests for tools/agents/brief.py (A0-5).

Unit tests call the parser and builder directly on synthetic text; CLI tests
copy the two scripts into a throwaway git repository so the tool's own-checkout
root resolves there, then run it as a real process and inspect text and JSON.
The size-property test iterates the real `tasks/` directory.
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
from unittest import mock

HERE = Path(__file__).resolve().parent
TOOLS = HERE.parent
AGENTS = TOOLS / "agents"
REPO = TOOLS.parent
FIXTURE = HERE / "fixtures" / "brief" / "T900.md"

sys.path.insert(0, str(AGENTS))
import brief  # noqa: E402
import recall  # noqa: E402

BRIEF_PY = AGENTS / "brief.py"
RECALL_PY = AGENTS / "recall.py"


def run_git(root: Path, *args: str) -> subprocess.CompletedProcess:
    return subprocess.run(
        ["git", "-C", str(root), *args],
        capture_output=True,
        text=True,
        encoding="utf-8",
    )


class TempRepo:
    """A temp git repo with copies of the scripts under tools/agents."""

    def __init__(self) -> None:
        self.tmp = Path(tempfile.mkdtemp(prefix="brief_"))
        self.root = self.tmp / "repo"
        (self.root / "tools" / "agents").mkdir(parents=True)
        for source in (BRIEF_PY, RECALL_PY):
            shutil.copy(source, self.root / "tools" / "agents" / source.name)
        subprocess.run(["git", "init", "-q", str(self.root)], check=True)

    def close(self) -> None:
        shutil.rmtree(self.tmp, True)

    def write_task(self, task_id: str, text: str) -> Path:
        path = self.root / "tasks" / f"{task_id}.md"
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8", newline="")
        return path

    def run_brief(self, *args: str) -> subprocess.CompletedProcess:
        return subprocess.run(
            [sys.executable, str(self.root / "tools" / "agents" / "brief.py"), *args],
            capture_output=True,
            text=True,
            encoding="utf-8",
            cwd=str(self.root),
        )


class Classification(unittest.TestCase):
    CONTRACT = [
        "Exact contract — 2026-10-05 (UTC)",
        "Re-contract — 2026-10-04 (UTC): the `crpg-net-quic` crate",
        "Contract amendment — 2026-10-06 (UTC): stall cases",
        "Amendment — 2026-01-01 (UTC)",
        "Task",
        "Task / Crate(s) / Purpose",
        "Crate(s)",
        "Purpose",
        "Purpose (as recorded in spec §24 and the backlog)",
        "Interface",
        "Interface — exact transfer/version gate",
        "2. Interface (exact public Rust)",
        "Behaviour",
        "Behaviour to preserve in the later executable contract",
        "Behaviour / suites (`crates/crpg-net/tests/conformance.rs` + support)",
        "Constraints",
        "Constraints on all children",
        "Test",
        "Test — requirements for the executable contract, not runnable today",
        "Tests",
        "Tests (`crates/crpg-data/tests/combat_content.rs`; reserved by T016)",
        "Tests (reserved by T016: `combat` and `combat_persistence`)",
        "C§8. Tests (`crates/crpg-net-quic/tests/quic.rs`, public API only)",
        "7. Test",
        "10. Definition of done",
        "Definition of done",
        "Definition of done — later implementation",
        "11. Out of scope",
        "Out of scope (later work; do not expand into it)",
        "Read first",
        "Read first (all children)",
        "Decision",
        "What must not change",
        "1. Decisions this contract makes",
    ]
    HISTORY = [
        "Agent log",
        "Amendment decisions — 2026-10-06 (UTC)",
        "Decisions — 2026-10-05 (UTC)",
        "Decisions on the revised questions — 2026-10-04 (UTC)",
        "Reading decisions",
        "Implementation blocker and decision — 2026-10-05 (UTC)",
        "Completion record — 2026-10-06 (UTC)",
        "CI follow-up — 2026-10-05 (UTC): Windows relay timing",
        "Open questions (revised) — 2026-10-04",
        "Next contract needs",
        "Downstream — 2026-10-04 (UTC): the `crpg-net-quic` split",
        "History",
        "Dependency record and approval request (H-dep)",
        "3. Host save payload v1 layout (normative)",
        "5. Determinism and the gate-10 claim",
        "Whatever unknown heading",
    ]

    # Every `##` section of the fixture, so a wrong kind anywhere fails.
    FIXTURE_KINDS = {
        "Task / Crate(s) / Purpose": "contract",
        "Crate(s)": "contract",
        "Purpose": "contract",
        "2. Interface (exact public Rust)": "contract",
        "4. Behaviour": "contract",
        "5. Determinism and the gate-10 claim": "history",
        "7. Test": "contract",
        "Tests": "contract",
        "10. Definition of done": "contract",
        "11. Out of scope": "contract",
        "12. Open questions for the user": "history",
        "Agent log": "history",
        "Exact contract — 2026-10-05 (UTC)": "contract",
        "Decisions — 2026-10-05 (UTC)": "history",
        "Re-contract — 2026-10-06 (UTC): the second contract": "contract",
        "Contract amendment — 2026-10-07 (UTC)": "contract",
        "Amendment decisions — 2026-10-07 (UTC)": "history",
        "Reading decisions": "history",
        "Implementation blocker — 2026-10-07 (UTC)": "history",
        "Completion record — 2026-10-07 (UTC)": "history",
        "CI follow-up — 2026-10-07 (UTC): timing": "history",
        "Open questions (revised) — 2026-10-04": "history",
        "Next contract needs": "history",
        "Downstream — 2026-10-04 (UTC): the split": "history",
        "History": "history",
        "Dependency record — 2026-10-05 (UTC)": "history",
        "Whatever unknown heading": "history",
    }

    def test_every_contract_heading(self) -> None:
        for heading in self.CONTRACT:
            self.assertEqual(brief.classify_heading(heading), "contract", heading)

    def test_every_history_heading(self) -> None:
        for heading in self.HISTORY:
            self.assertEqual(brief.classify_heading(heading), "history", heading)

    def test_fixture_sections_are_classified(self) -> None:
        text = FIXTURE.read_text(encoding="utf-8")
        _, sections, _ = brief.parse(text)
        kinds = {section["text"]: brief.classify_heading(section["text"]) for section in sections}
        self.assertEqual(kinds, self.FIXTURE_KINDS)

    def test_plural_tests_in_the_real_task_files(self) -> None:
        # `## Tests` became a contract heading in review item 1; these two
        # real lines are the manager's examples.
        root = Path(recall.find_repo_root())
        for name, number in (("T009c.md", 254), ("T016a.md", 379)):
            lines = (root / "tasks" / name).read_text(encoding="utf-8").splitlines()
            heading = lines[number - 1]
            self.assertTrue(heading.startswith("## Tests"), f"{name}:{number}: {heading!r}")
            self.assertEqual(brief.classify_heading(heading[3:].strip()), "contract", heading)


class DecisionSections(unittest.TestCase):
    """The narrowed decision rule of A0-3b attempt 2.

    `Decision` (exactly), `Decisions this contract makes…` and
    `What must not change…` are base contract; every other `…decision…`
    heading stays history, so a task's decision log is not mistaken for its
    contract.
    """

    CONTRACT = (
        "Decision",
        "What must not change",
        "1. Decisions this contract makes",
        "Decision: the journal cache",
        "What must not change (ADR-0017)",
    )
    HISTORY = (
        "Decision already made",
        "Decision output",
        "Decision to make",
        "Decisions — 2026-10-04 (UTC)",
        "Decisions (D19–D26)",
        "Amendment decisions",
        "Amendment decisions — 2026-10-06 (UTC)",
        "Reading decisions",
        "Implementation blocker and decision — 2026-10-05 (UTC)",
    )

    def test_narrow_rule_pins_the_headings(self) -> None:
        for heading in self.CONTRACT:
            self.assertEqual(brief.classify_heading(heading), "contract", heading)
        for heading in self.HISTORY:
            self.assertEqual(brief.classify_heading(heading), "history", heading)

    def test_t097_brief_names_both_sections_in_file_order(self) -> None:
        root = recall.find_repo_root()
        with open(Path(root) / "tasks" / "T097.md", encoding="utf-8", newline="") as handle:
            text = handle.read()
        headings, sections, _ = brief.parse(text)
        current = brief.select_current(sections)
        by_line = {section["line"]: section for section in current}
        self.assertIn(44, by_line, [section["text"] for section in current])
        self.assertIn(63, by_line)

        built, document = brief.build("T097", text, root)
        contract_line = next(line for line in built.splitlines() if line.startswith("Contract:"))
        decision = brief.section_reference("T097", by_line[44])
        unchanged = brief.section_reference("T097", by_line[63])
        self.assertIn(decision, contract_line)
        self.assertIn(unchanged, contract_line)
        self.assertLess(contract_line.index(decision), contract_line.index(unchanged))
        self.assertIn(f"- {decision}", built)
        self.assertIn(f"- {unchanged}", built)
        self.assertLess(built.index(f"- {decision}"), built.index(f"- {unchanged}"))
        for section in document["sections"]:
            if section["heading"] in ("Decision", "What must not change"):
                self.assertEqual(section["kind"], "contract")
                self.assertTrue(section["current"])


class CurrentContract(unittest.TestCase):
    TEXT = textwrap.dedent(
        """\
        # T900 — fixture

        ## Task

        base task

        ## Exact contract — 2026-01-01 (UTC)

        old exact

        ## Amendment — 2026-01-02 (UTC)

        old amendment

        ## Re-contract — 2026-01-03 (UTC): second

        new exact

        ## Contract amendment — 2026-01-04 (UTC)

        new amendment

        ## Agent log

        history
        """
    )

    def test_last_exact_wins_and_only_later_amendments_join(self) -> None:
        _, sections, _ = brief.parse(self.TEXT)
        current = [section["text"] for section in brief.select_current(sections)]
        self.assertEqual(
            current,
            ["Re-contract — 2026-01-03 (UTC): second", "Contract amendment — 2026-01-04 (UTC)"],
        )

    def test_base_sections_fallback(self) -> None:
        text = textwrap.dedent(
            """\
            # T900 — base fixture

            ## Task

            base task

            ## Interface

            base interface

            ## Behaviour

            base behaviour

            ## Agent log

            history

            ## Decisions — 2026-01-01 (UTC)

            history
            """
        )
        _, sections, _ = brief.parse(text)
        current = [section["text"] for section in brief.select_current(sections)]
        self.assertEqual(current, ["Task", "Interface", "Behaviour"])

    def test_fixture_current_is_last_exact_plus_amendment(self) -> None:
        text = FIXTURE.read_text(encoding="utf-8")
        _, sections, _ = brief.parse(text)
        current = [section["text"] for section in brief.select_current(sections)]
        self.assertEqual(
            current,
            ["Re-contract — 2026-10-06 (UTC): the second contract", "Contract amendment — 2026-10-07 (UTC)"],
        )


class InterfaceExtraction(unittest.TestCase):
    def setUp(self) -> None:
        self.text = FIXTURE.read_text(encoding="utf-8")
        self.headings, self.sections, _ = brief.parse(self.text)
        self.current = brief.select_current(self.sections)

    def test_interface_is_byte_exact_and_amendment_appended(self) -> None:
        interface = brief.build_interface(self.text, self.headings, self.current)
        self.assertTrue(
            interface.startswith("\n```rust\npub fn demo1() -> i32 { 11 }\n```\n\n"),
            repr(interface[:120]),
        )
        self.assertIn("From Contract amendment — 2026-10-07 (UTC):\n", interface)
        self.assertTrue(
            interface.endswith("\n```rust\npub fn demo2() -> i32 { 2 }\n```\n\n"),
            repr(interface[-120:]),
        )
        # The earlier, superseded Exact contract's Interface must not appear.
        self.assertNotIn("pub fn demo() -> i32 { 1 }", interface)

    def test_brief_copies_the_rust_block(self) -> None:
        built, _ = brief.build("T900", self.text, str(REPO))
        self.assertIn("```rust\npub fn demo1() -> i32 { 11 }\n```", built)

    def test_no_interface_placeholder(self) -> None:
        text = "# T900 — x\n\n## Task\n\nbase task\n"
        headings, sections, _ = brief.parse(text)
        current = brief.select_current(sections)
        self.assertEqual(
            brief.build_interface(text, headings, current),
            "<MANAGER: no Interface section found>",
        )

    def test_only_an_amendment_appends_an_interface(self) -> None:
        # Base sections are the current contract here, and `## Behaviour`
        # carries an `Interface` subsection: it must not be appended.
        text = textwrap.dedent(
            """\
            # T900 — append fixture

            ## Interface

            ```rust
            pub fn base() -> i32 { 0 }
            ```

            ## Behaviour

            ### Loader integration (V Interface)

            prose

            ## Tests

            ```text
            cargo test -p crpg-demo --locked
            ```

            ## Agent log

            history
            """
        )
        headings, sections, _ = brief.parse(text)
        current = brief.select_current(sections)
        interface = brief.build_interface(text, headings, current)
        self.assertIn("pub fn base() -> i32 { 0 }", interface)
        self.assertNotIn("From ", interface)
        self.assertNotIn("Loader integration", interface)

    def test_t012_brief_does_not_append_a_base_section(self) -> None:
        root = recall.find_repo_root()
        with open(Path(root) / "tasks" / "T012.md", encoding="utf-8", newline="") as handle:
            text = handle.read()
        built, _ = brief.build("T012", text, root)
        self.assertNotIn("From Behaviour:", built)
        self.assertNotIn("Loader integration and authority", built)


class AcceptanceExtraction(unittest.TestCase):
    def test_commands_dedup_and_preflight(self) -> None:
        text = FIXTURE.read_text(encoding="utf-8")
        headings, sections, _ = brief.parse(text)
        current = brief.select_current(sections)
        body = brief.extract_acceptance(text, headings, current, "crpg-demo")
        self.assertEqual(
            body.splitlines(),
            [
                "- cargo clippy -p crpg-demo --all-targets --locked -- -D warnings",
                "- python tools/lint/deps.py",
                "- bash tools/preflight.sh --crate crpg-demo   "
                "(Windows: tools/preflight.ps1 -Crate crpg-demo)",
                "Passing the tests that exist is not done. Done is the contract's "
                "Behaviour, with a",
                "test for each numbered item.",
            ],
        )
        self.assertNotIn("git diff --check", body)
        self.assertNotIn("host_quic", body)

    def test_no_commands_placeholder_keeps_preflight(self) -> None:
        text = "# T900 — x\n\n## Task\n\nbase task\n\n## Test\n\nno fence here\n"
        headings, sections, _ = brief.parse(text)
        current = brief.select_current(sections)
        body = brief.extract_acceptance(text, headings, current, None)
        self.assertTrue(body.startswith("- <MANAGER: acceptance commands>\n"), body)
        self.assertIn("bash tools/preflight.sh --crate <crate>", body)

    def test_t023c_nested_tests_subsection(self) -> None:
        # `#### C§8.7 Commands` sits under `### C§8. Tests` inside the exact
        # contract; both need a numbered/lettered prefix to be recognised.
        root = recall.find_repo_root()
        with open(Path(root) / "tasks" / "T023c.md", encoding="utf-8", newline="") as handle:
            text = handle.read()
        built, _ = brief.build("T023c", text, root)
        self.assertNotIn("<MANAGER: acceptance commands>", built)
        self.assertIn("- cargo test -p crpg-net-quic --test quic --locked\n", built)
        self.assertIn("- python tools/lint/deps.py\n", built)
        self.assertIn("bash tools/preflight.sh --crate crpg-net-quic", built)

    def test_definition_of_done_subsection_with_bash_and_pwsh(self) -> None:
        text = (HERE / "fixtures" / "brief" / "T901.md").read_text(encoding="utf-8")
        headings, sections, _ = brief.parse(text)
        current = brief.select_current(sections)
        body = brief.extract_acceptance(text, headings, current, "crpg-demo")
        lines = body.splitlines()
        self.assertNotIn("- <MANAGER: acceptance commands>", lines)
        # The lettered-prefix `Definition of done` subsection.
        self.assertIn("- bash tools/preflight.sh --crate crpg-demo", lines)
        self.assertIn("- pwsh -File tools/preflight.ps1 -Crate crpg-demo", lines)
        # A plural `Tests` heading and the commands nested inside it.
        self.assertIn("- cargo test -p crpg-demo --locked", lines)
        self.assertIn("- python tools/lint/deps.py", lines)
        self.assertEqual(lines.count("- cargo test -p crpg-demo --locked"), 1)
        self.assertNotIn("- git diff --check", lines)
        self.assertNotIn("- pub fn demo() -> i32 { 9 }", lines)


class Placeholders(unittest.TestCase):
    @staticmethod
    def section_body(built: str, heading: str) -> str:
        lines = built.splitlines()
        body: list[str] = []
        for line in lines[lines.index(heading) + 1 :]:
            if line.startswith("## "):
                break
            body.append(line)
        while body and not body[-1].strip():
            body.pop()
        return "\n".join(body)

    def test_placeholders_present_and_no_invented_goal(self) -> None:
        text = FIXTURE.read_text(encoding="utf-8")
        built, _ = brief.build("T900", text, str(REPO))
        for heading in ("## Goal", "## Files you own", "## Out of scope", "## Size budget"):
            self.assertIn(heading, built)
        self.assertIn("## Example to follow (follow-on tasks)", built)
        self.assertEqual(self.section_body(built, "## Goal"), "<MANAGER: goal>")

    def test_placeholder_bodies_are_exact_and_distinct(self) -> None:
        text = FIXTURE.read_text(encoding="utf-8")
        built, _ = brief.build("T900", text, str(REPO))
        self.assertEqual(self.section_body(built, "## Files you own"), "<MANAGER: files you own>")
        self.assertEqual(
            self.section_body(built, "## Example to follow (follow-on tasks)"),
            "<MANAGER: example to follow>",
        )
        self.assertEqual(self.section_body(built, "## Out of scope"), "<MANAGER: out of scope>")
        self.assertEqual(self.section_body(built, "## Size budget"), "<MANAGER: size budget>")
        self.assertEqual(self.section_body(built, "## Goal"), "<MANAGER: goal>")

    def test_fixed_sections_substitute_the_id(self) -> None:
        text = FIXTURE.read_text(encoding="utf-8")
        built, _ = brief.build("T900", text, str(REPO))
        self.assertIn("commit once as `T900: <summary>`", built)
        self.assertIn("Reply with the report format in docs/agents/TEMPLATES.md §2", built)
        self.assertIn("crpg-godot · an edit to crpg-contracts or rust-toolchain.toml.", built)


class RelatedMemory(unittest.TestCase):
    def test_title_words_are_deduplicated_case_insensitively(self) -> None:
        text = "# T900 — Zebra zebra memory\n\n## Task\n\nbase task\n"
        captured: list[list[str]] = []

        def fake_search(words, root):  # type: ignore[no-untyped-def]
            captured.append(list(words))
            return []

        with mock.patch.object(brief.recall, "search", side_effect=fake_search):
            brief.build("T900", text, str(REPO))
        self.assertEqual(captured[0], ["Zebra", "memory"])


class CliBehaviour(unittest.TestCase):
    def setUp(self) -> None:
        self.repo = TempRepo()
        self.addCleanup(self.repo.close)
        self.fixture_text = FIXTURE.read_text(encoding="utf-8")
        self.repo.write_task("T900", self.fixture_text)

    def test_generates_a_brief(self) -> None:
        result = self.repo.run_brief("T900")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(result.stdout.startswith("# Brief — T900 · synthetic brief fixture"))
        self.assertIn('§"Re-contract — 2026-10-06 (UTC): the second contract"', result.stdout)

    def test_out_writes_the_file(self) -> None:
        target = self.repo.tmp / "out.md"
        result = self.repo.run_brief("T900", "--out", str(target))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "")
        self.assertTrue(target.read_text(encoding="utf-8").startswith("# Brief — T900"))

    def test_missing_task_is_no_task_exit_two(self) -> None:
        result = self.repo.run_brief("T999")
        self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
        self.assertIn("no-task", result.stdout)

    def test_json_shape(self) -> None:
        result = self.repo.run_brief("T900", "--json")
        self.assertEqual(result.returncode, 0, result.stderr)
        document = json.loads(result.stdout)
        self.assertEqual(document["tool"], "brief")
        self.assertEqual(document["version"], 1)
        self.assertEqual(document["task"], "T900")
        self.assertEqual(document["bytes_in"], len(self.fixture_text.encode("utf-8")))
        self.assertGreater(document["bytes_out"], 0)
        by_heading = {section["heading"]: section for section in document["sections"]}
        for section in document["sections"]:
            self.assertEqual(set(section), {"heading", "start", "end", "kind", "current"})
            self.assertIn(section["kind"], ("contract", "history"))
            self.assertIsInstance(section["current"], bool)
            self.assertLessEqual(section["start"], section["end"])
        self.assertTrue(by_heading["Exact contract — 2026-10-05 (UTC)"]["kind"] == "contract")
        self.assertFalse(by_heading["Exact contract — 2026-10-05 (UTC)"]["current"])
        self.assertTrue(by_heading["Contract amendment — 2026-10-07 (UTC)"]["current"])
        self.assertEqual(by_heading["Agent log"]["kind"], "history")

    def test_json_missing_task(self) -> None:
        result = self.repo.run_brief("T999", "--json")
        self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
        document = json.loads(result.stdout)
        self.assertEqual(document["error"], "no-task")
        self.assertEqual(document["task"], "T999")


class SizeProperty(unittest.TestCase):
    def test_brief_is_small_against_the_real_task_files(self) -> None:
        root = recall.find_repo_root()
        tasks = sorted((Path(root) / "tasks").glob("T*.md"))
        self.assertTrue(tasks, "no task files found")
        checked = 0
        for path in tasks:
            size = path.stat().st_size
            if size <= 40 * 1024:
                continue
            checked += 1
            with open(path, encoding="utf-8", newline="") as handle:
                text = handle.read()
            heads, sections, _ = brief.parse(text)
            current = brief.select_current(sections)
            interface = brief.build_interface(text, heads, current)
            built, document = brief.build(path.stem, text, root)
            full = document["bytes_out"]
            skeleton = len(built.encode("utf-8")) - len(interface.encode("utf-8"))
            if full < size / 4:
                continue
            # A base-sections task may copy an Interface section that is itself
            # over a quarter (T010); the generated skeleton must still be small.
            self.assertLess(skeleton, size / 4, f"{path.name} skeleton too large")
        self.assertGreater(checked, 5)

    def test_t023b_full_brief_is_under_a_quarter(self) -> None:
        root = recall.find_repo_root()
        with open(Path(root) / "tasks" / "T023b.md", encoding="utf-8", newline="") as handle:
            text = handle.read()
        _, document = brief.build("T023b", text, root)
        size = (Path(root) / "tasks" / "T023b.md").stat().st_size
        self.assertLess(document["bytes_out"], size / 4)

    def test_t023b_reference_matches_the_documented_example(self) -> None:
        fixture = HERE / "fixtures" / "brief" / "T023b.md"
        with open(fixture, encoding="utf-8", newline="") as handle:
            text = handle.read()
        root = recall.find_repo_root()
        built, document = brief.build("T023b", text, root)
        self.assertIn(
            'tasks/T023b.md §"Exact contract — 2026-10-05 (UTC)" (lines 125–1401)',
            built,
        )
        self.assertIn(
            'tasks/T023b.md §"Contract amendment — 2026-10-06 (UTC): stall cases on '
            'T023c windows" (lines 1454–2056)',
            built,
        )
        self.assertEqual(document["bytes_in"], 125054)


class DecisionSectionsPinned(unittest.TestCase):
    """Pins the narrowed decision rule of A0-3b (manager's fixture headings)."""

    def test_contract_headings(self) -> None:
        for heading in ("Decision", "What must not change", "1. Decisions this contract makes"):
            self.assertEqual(brief.classify_heading(heading), "contract", heading)

    def test_history_headings(self) -> None:
        for heading in (
            "Decision already made",
            "Decision output",
            "Decision to make",
            "Decisions — 2026-10-04 (UTC)",
            "Decisions (D19–D26)",
            "Amendment decisions",
        ):
            self.assertEqual(brief.classify_heading(heading), "history", heading)




if __name__ == "__main__":
    unittest.main()

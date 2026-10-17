"""OpenCode's `execute` tool stays denied for every worker.

OpenCode 2.x gives agents an `execute` tool that runs JavaScript with
`fs`, `child_process` and `tools.opencode.session_move`. On 2026-10-16 two
review workers used it to move their sessions into other slots, and others
to run `git` through `execSync`, past every `bash`, `edit`, `webfetch` and
`external_directory` rule in their agent files. Denying the tool by name is
the only control that held when probed (docs/agents/MODELS.md section 1).
"""

from __future__ import annotations

import json
import re
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]


class ExecuteToolDenied(unittest.TestCase):
    def test_project_config_denies_execute(self) -> None:
        config = json.loads((REPO / "opencode.json").read_text(encoding="utf-8"))
        self.assertEqual(config["permission"].get("execute"), "deny")

    def test_every_agent_denies_execute(self) -> None:
        agents = sorted((REPO / ".opencode" / "agents").glob("*.md"))
        self.assertTrue(agents)
        for agent in agents:
            with self.subTest(agent=agent.name):
                text = agent.read_text(encoding="utf-8")
                front = text.split("---", 2)[1]
                self.assertRegex(front, re.compile(r"^  execute: deny$", re.M))

    def test_read_only_roles_deny_shell_redirection(self) -> None:
        # "git show*": allow also matched `git show X > file`, so a review
        # with `edit: deny` wrote files (2026-10-16). The last rule wins.
        for name in ("crpg-review.md", "crpg-scout.md", "crpg-grunt.md"):
            with self.subTest(agent=name):
                front = (REPO / ".opencode" / "agents" / name).read_text(
                    encoding="utf-8").split("---", 2)[1]
                bash = front.split("  bash:\n", 1)[1].split("\n")
                rules = [line for line in bash if line.startswith("    ")]
                self.assertEqual(rules[-1].strip(), '"*>*": deny')


if __name__ == "__main__":
    unittest.main()

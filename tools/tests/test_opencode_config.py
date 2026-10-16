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


if __name__ == "__main__":
    unittest.main()

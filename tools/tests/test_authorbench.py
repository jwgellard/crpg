#!/usr/bin/env python3
"""Tests for tools/agents/authorbench/authorbench.py (T100b)."""

from __future__ import annotations

import hashlib
import importlib.util
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]
AUTHORBENCH = REPO / "tools" / "agents" / "authorbench" / "authorbench.py"
AUTHORBENCH_DIR = AUTHORBENCH.parent
SCORING = AUTHORBENCH_DIR / "authorbench_scoring.py"
FAKE = HERE / "fixtures" / "authorbench" / "fake_crpgc.py"
FAKE_OPENCODE = HERE / "fake_opencode.py"

BRIEF_ORDER = ["campaign-extend", "campaign-small", "campaign-medium",
               "campaign-large", "ruleset-small"]

PROMPT_TEXT = "Read brief.md in this directory and do exactly what it says."

ROW_KEYS = [
    "brief", "model", "repeat", "variant", "design", "rc", "secs", "excluded",
    "steps", "tools", "crpgc_calls", "errors", "commands", "tok", "cost", "err",
    "campaign_present", "files", "kinds", "valid", "validate", "complete",
    "play", "playable", "first_error",
]

META_KEYS = ["brief", "model", "repeat", "variant", "design", "rc", "secs",
             "dir", "log", "tools_sha256", "crpgc_sha256"]


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


authorbench = load("authorbench_under_test", AUTHORBENCH)
import authorbench_scoring as scoring  # noqa: E402

HEAD = (
    "# Authoring task\n"
    "\n"
    "You are building game content for a CRPG engine. Work only inside this directory.\n"
    "\n"
    "## What to build\n"
    "\n"
)

TAIL = (
    "\n"
    "## What you have\n"
    "\n"
    "{{TOOLS}}\n"
    "\n"
    "## Rules of the exercise\n"
    "\n"
    "- Use only `crpgc` and the files in `schemas/`. Do not look for other documentation and do not read anything outside this directory.\n"
    "- Put the finished campaign in the `campaign/` directory here. Every document is a JSON file.\n"
    "- Check your work with `crpgc validate campaign --json` and keep going until it exits 0.\n"
    "- Then check that it plays: give `crpgc play --campaign campaign` its choices on standard input, one number per line.\n"
    "- When you are finished, reply with the single word DONE. If you cannot finish, reply with the word STUCK and one sentence saying why.\n"
)

BUILDS = {
    "campaign-extend": "The directory `campaign/` already holds a small valid campaign: a hero and a goblin fight in one encounter. Add a third creature, a wolf with the slug `wolf`. It has the same four attributes and the same health stat as the other two, with less health than the hero. Place it in the starting area and add it to the existing encounter. The campaign must stay valid, and the fight must start with three combatants.\n",
    "campaign-small": "Build a new campaign named `ember-hollow` from nothing, in `campaign/`. It has one world and one area with the slug `start`. Its rule system is your own, named `plain-d6`: three attributes (might, guile, resolve) and a health stat; a creature has one action each turn. It has two creatures, a hero and a goblin, each placed in the area. It has one ability, `strike`, that costs one action. A strike rolls two six-sided dice against the attacker's might and succeeds when the roll is no higher than the might; a success deals 2 damage and a failure deals none. It has one encounter, `first-blood`, in which the hero and the goblin fight. The fight must start with two combatants.\n",
    "campaign-medium": "Build a new campaign named `ashen-vale` from nothing, in `campaign/`. It has one world and one area with the slug `start`. Its rule system is your own, named `focus-d6`: three attributes (might, guile, resolve) and a health stat; a creature has one action each turn. It has three creatures, a hero, a goblin and an archer, each placed in the area. It has two abilities. `strike` costs one action, rolls two six-sided dice against the attacker's might, and on a success deals 2 damage. `focus` costs one action, deals no damage, and on a success gives its user an effect named `focusing`. The effect `focusing` lasts two turns and adds 2 to its bearer's might. Each ability has its own outcome table, `strike-table` and `focus-table`. It has one encounter, `skirmish`, in which all three creatures fight. The fight must start with three combatants.\n",
    "campaign-large": "Build a new campaign named `ashen-reach` from nothing, in `campaign/`. It has one world with two areas, `start` and `ridge`. Its rule system is your own, named `focus-d6`: three attributes (might, guile, resolve) and a health stat; a creature has one action each turn. It has four creatures, a hero, a goblin, an archer and a wolf; place the hero, the goblin and the archer in `start` and the wolf in `ridge`. It has two abilities. `strike` costs one action, rolls two six-sided dice against the attacker's might, and on a success deals 2 damage. `focus` costs one action, deals no damage, and on a success gives its user an effect named `focusing`, which lasts two turns and adds 2 to its bearer's might. Each ability has its own outcome table, `strike-table` and `focus-table`. It has two encounters: `skirmish`, in `start`, in which the hero, the goblin and the archer fight, and `ridge-ambush`, in `ridge`, in which the hero and the wolf fight. It has an item, `healing-draught`, and a faction, `vale-folk`, that the hero and the archer belong to. It has a dialogue, `elder-greeting`, of at least three nodes that ends, and a quest, `clear-the-ridge`, with at least two states, the last of them terminal. Either encounter may be the one that starts first; it must start with at least two combatants.\n",
    "ruleset-small": "Define a small rule system and show that it plays. The rule system is named `tiny-d8` (slug `tiny-d8`). It has three attributes (vigor, finesse, wit) and a health stat; a creature has one action each turn. It has one ability, `cut`, that costs one action and rolls one eight-sided die against the attacker's vigor; it succeeds when the roll is no higher than the vigor, a success deals 3 damage and a failure deals none. It has one outcome table of your own, named `d8-under`. Then build the smallest campaign in `campaign/` that uses it: a campaign named `tiny-test`, one world, one area, two creatures (`squire` and `rat`) placed in the area, and one encounter in which they fight. The fight must start with two combatants.\n",
}

EXPECTED_BRIEFS = {
    "campaign-extend": {"kind": "extend", "starter": "campaigns/fixtures/combat_basic",
                        "minimum": {"creature": 3}, "ruleset_slug": None,
                        "min_combatants": 3, "timeout_s": 900},
    "campaign-small": {"kind": "campaign", "starter": None,
                       "minimum": {"creature": 2, "ability": 1, "outcome-table": 1, "ruleset": 1, "encounter": 1},
                       "ruleset_slug": None, "min_combatants": 2, "timeout_s": 1800},
    "campaign-medium": {"kind": "campaign", "starter": None,
                        "minimum": {"creature": 3, "ability": 2, "outcome-table": 2, "effect": 1, "ruleset": 1, "encounter": 1},
                        "ruleset_slug": None, "min_combatants": 3, "timeout_s": 2400},
    "campaign-large": {"kind": "campaign", "starter": None,
                       "minimum": {"creature": 4, "ability": 2, "outcome-table": 2, "effect": 1, "ruleset": 1,
                                   "encounter": 2, "area": 2, "item": 1, "faction": 1, "dialogue": 1, "quest": 1},
                       "ruleset_slug": None, "min_combatants": 2, "timeout_s": 3600},
    "ruleset-small": {"kind": "ruleset", "starter": None,
                      "minimum": {"creature": 2, "ability": 1, "outcome-table": 1, "ruleset": 1, "encounter": 1},
                      "ruleset_slug": "tiny-d8", "min_combatants": 2, "timeout_s": 1800},
}

TOOLS_TEXT = (
    "`crpgc` is the engine's command-line tool; it is on your PATH.\n"
    "\n"
    "  crpgc validate <campaign-root> [--json]\n"
    "      Checks a campaign directory. Exit 0 means no error was found. With --json it prints a JSON array of diagnostics.\n"
    "  crpgc new <type> --slug <s> --id <id> [--entry-id <id>]\n"
    "      Prints one minimal valid document of that type to standard output. <type> is creature, item, dialogue or quest.\n"
    "  crpgc schema <type>\n"
    "      Prints one JSON Schema. <type> is campaign, world, area, creature, item, dialogue, quest, faction, graph, placements, triggers, locale, variables, campaign-lock, assets-lock, placement or action-signature.\n"
    "  crpgc explain <id> [--root <campaign-root>]\n"
    "      Prints one object of a campaign by its id.\n"
    "  crpgc fmt [<campaign-root>] [--check]\n"
    "      Rewrites documents in canonical form; with --check it lists the ones that are not.\n"
    "  crpgc play --campaign <campaign-root> [--encounter <ulid>] [--side <name>] [--seed N]\n"
    "      Plays one encounter in the terminal. It reads one number per line from standard input to pick an option; `q` quits.\n"
    "\n"
    "The directory `schemas/` holds all 23 JSON Schema files, including the six that `crpgc schema` does not print: ability, effect, encounter, outcome-table, ruleset and ui.\n"
)

MODELS_TEXT = (
    "# Models the authoring benchmark runs by default (T100). Edit freely.\n"
    "opencode-go/deepseek-v4.1-flash\n"
    "opencode-go/deepseek-v4-pro\n"
    "opencode-go/muse-spark-1.3-contributor\n"
    "opencode-go/glm-5.3\n"
    "opencode-go/claude-haiku-5-5\n"
)

_BANNED_RE = re.compile(r"[A-Za-z]:[\\/]|/home/|/Users/|@|http")


class AuthorbenchCase(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="authorbench-"))
        self.addCleanup(shutil.rmtree, self.tmp, ignore_errors=True)
        self.scratch = self.tmp / "scratch"

    def env(self, **overrides):
        e = dict(os.environ)
        e.update({
            "CRPG_OPENCODE": str(FAKE_OPENCODE),
            "CRPG_OC_START_GAP": "0",
            "CRPG_OC_LOCK_WAIT": "0.05",
            "FAKE_OC_MODE": "ok",
            "CRPGC": str(FAKE),
        })
        e.update(overrides)
        return e

    def run_ab(self, *argv, env=None):
        return subprocess.run(
            [sys.executable, str(AUTHORBENCH), *argv],
            capture_output=True, text=True, env=env or self.env())

    # CL1
    def test_cl1_setup_builds_templates(self):
        r = self.run_ab("setup", str(self.scratch))
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("templates ready in", r.stdout)
        tpl = self.scratch / "tpl"
        schemas_src = {p.name for p in (REPO / "schemas").glob("*.json")}
        self.assertEqual(len(schemas_src), 23)
        for name in BRIEF_ORDER:
            brief_dir = AUTHORBENCH_DIR / "briefs" / name
            self.assertEqual((tpl / name / "brief.md").read_text(encoding="utf-8"),
                             (brief_dir / "prompt.txt").read_text(encoding="utf-8"))
            got_schemas = {p.name for p in (tpl / name / "schemas").iterdir()}
            self.assertEqual(got_schemas, schemas_src)
            self.assertFalse((tpl / name / "prompt.txt").exists())
            self.assertFalse((tpl / name / "brief.json").exists())
        starter = REPO / "campaigns" / "fixtures" / "combat_basic"
        starter_files = {p.relative_to(starter).as_posix() for p in starter.rglob("*") if p.is_file()}
        camp_root = tpl / "campaign-extend" / "campaign"
        camp_files = {p.relative_to(camp_root).as_posix() for p in camp_root.rglob("*") if p.is_file()}
        self.assertEqual(camp_files, starter_files)
        self.assertTrue((camp_root / "campaign.json").is_file())
        for name in BRIEF_ORDER[1:]:
            self.assertFalse((tpl / name / "campaign").exists())
        # a second setup gives the same listing (a stale file does not survive)
        (tpl / "campaign-small" / "stale.txt").write_text("stale", encoding="utf-8")
        r2 = self.run_ab("setup", str(self.scratch))
        self.assertEqual(r2.returncode, 0, r2.stderr)
        self.assertFalse((tpl / "campaign-small" / "stale.txt").exists())
        self.assertEqual({p.name for p in (tpl / "campaign-small" / "schemas").iterdir()}, schemas_src)

    # CL2
    def test_cl2_refuses_inside_repo(self):
        inner = REPO / "authorbench-refused-scratch"
        cases = [
            ("setup", [str(inner)]),
            ("run", [str(inner), "--models", "m1"]),
            ("score", [str(inner)]),
            ("compare", [str(inner), "a", "b"]),
            ("all", [str(inner)]),
        ]
        for cmd, argv in cases:
            r = self.run_ab(cmd, *argv)
            self.assertEqual(r.returncode, 2, (cmd, r.stderr))
            self.assertIn("scratch directory is inside the repository; refused", r.stderr)
        self.assertFalse(inner.exists())

    # CL3
    def test_cl3_run_happy_path(self):
        self.run_ab("setup", str(self.scratch))
        oc_log = self.tmp / "oc.log"
        r = self.run_ab("run", str(self.scratch), "--models", "m/a#x",
                        "--briefs", "campaign-extend", "--design", "baseline",
                        env=self.env(FAKE_OC_LOG=str(oc_log)))
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("campaign-extend m/a#x rc=0", r.stdout)
        out = self.scratch / "baseline"
        run_dir = out / "runs" / "campaign-extend" / "m_a_x"
        self.assertTrue(run_dir.is_dir())
        tools_text = (AUTHORBENCH_DIR / "tools-baseline.txt").read_text(encoding="utf-8")
        brief_md = (run_dir / "brief.md").read_text(encoding="utf-8")
        self.assertIn(tools_text.rstrip("\n"), brief_md)
        self.assertNotIn("{{TOOLS}}", brief_md)
        self.assertTrue((run_dir / "schemas").is_dir())
        self.assertTrue((run_dir / "campaign").is_dir())
        expected_cfg = json.dumps({
            "$schema": "https://opencode.ai/config.json",
            "autoupdate": False,
            "share": "disabled",
            "permission": {"execute": "deny", "webfetch": "deny", "websearch": "deny",
                           "external_directory": "deny", "edit": "allow", "bash": "allow"},
        }, indent=2)
        self.assertEqual((run_dir / "opencode.json").read_text(encoding="utf-8"), expected_cfg)
        bin_crpgc = out / "bin" / "crpgc.py"
        self.assertTrue(bin_crpgc.is_file())
        self.assertEqual(bin_crpgc.read_bytes(), FAKE.read_bytes())
        meta_path = out / "logs" / "campaign-extend__m_a_x.jsonl.meta.json"
        meta = json.loads(meta_path.read_text(encoding="utf-8"))
        self.assertEqual(list(meta.keys()), META_KEYS)
        self.assertEqual(meta["rc"], 0)
        self.assertEqual(meta["design"], "baseline")
        self.assertIsNone(meta["variant"])
        self.assertEqual(meta["dir"], "runs/campaign-extend/m_a_x")
        self.assertEqual(meta["log"], "logs/campaign-extend__m_a_x.jsonl")
        self.assertEqual(meta["tools_sha256"], hashlib.sha256(tools_text.encode("utf-8")).hexdigest())
        self.assertEqual(meta["crpgc_sha256"], hashlib.sha256(FAKE.read_bytes()).hexdigest())
        records = [json.loads(line) for line in oc_log.read_text(encoding="utf-8").splitlines()]
        self.assertEqual(len(records), 1)
        rec = records[0]
        self.assertEqual(rec["argv"][-3:], ["--format", "json", PROMPT_TEXT])
        self.assertIn("-m", rec["argv"])
        self.assertIn("m/a#x", rec["argv"])
        self.assertEqual(os.path.normcase(rec["cwd"]), os.path.normcase(str(run_dir)))

        # control: --repeat 2 splits run dirs and logs with an rN suffix
        r2 = self.run_ab("run", str(self.scratch), "--models", "m/a#x",
                         "--briefs", "campaign-extend", "--design", "rep", "--repeat", "2",
                         env=self.env(FAKE_OC_LOG=str(oc_log)))
        self.assertEqual(r2.returncode, 0, r2.stderr)
        out2 = self.scratch / "rep"
        self.assertTrue((out2 / "runs" / "campaign-extend" / "m_a_x" / "r0").is_dir())
        self.assertTrue((out2 / "runs" / "campaign-extend" / "m_a_x" / "r1").is_dir())
        self.assertTrue((out2 / "logs" / "campaign-extend__m_a_x__r0.jsonl").is_file())
        self.assertTrue((out2 / "logs" / "campaign-extend__m_a_x__r1.jsonl").is_file())

        # control: --tools F puts F in brief.md and its SHA-256 in the meta
        f = self.tmp / "tools.txt"
        f.write_text("X", encoding="utf-8")
        r3 = self.run_ab("run", str(self.scratch), "--models", "m/a#x",
                         "--briefs", "campaign-extend", "--design", "tools", "--tools", str(f),
                         env=self.env(FAKE_OC_LOG=str(oc_log)))
        self.assertEqual(r3.returncode, 0, r3.stderr)
        out3 = self.scratch / "tools"
        brief_md3 = (out3 / "runs" / "campaign-extend" / "m_a_x" / "brief.md").read_text(encoding="utf-8")
        self.assertIn("X", brief_md3)
        meta3 = json.loads((out3 / "logs" / "campaign-extend__m_a_x.jsonl.meta.json").read_text(encoding="utf-8"))
        self.assertEqual(meta3["tools_sha256"], hashlib.sha256(b"X").hexdigest())

    # CL4
    def test_cl4_run_refusals(self):
        self.run_ab("setup", str(self.scratch))
        oc_log = self.tmp / "oc.log"
        cases = [
            (["--models", ""], "usage"),
            (["--models", "@/no/file"], None),
            (["--models", "m1", "--briefs", "nope"], "unknown brief nope"),
            (["--models", "m1", "--repeat", "0"], None),
            (["--models", "m1", "--tools", "/no/file"], None),
            (["--models", "m1", "--crpgc", "/no/file"], "no-crpgc: build it or pass --crpgc"),
        ]
        for extra, frag in cases:
            r = self.run_ab("run", str(self.scratch), *extra, env=self.env(FAKE_OC_LOG=str(oc_log)))
            self.assertEqual(r.returncode, 2, (extra, r.stderr))
            if frag:
                self.assertIn(frag, r.stderr)
        self.assertFalse(oc_log.exists())
        # a fresh scratch (before setup): the first brief's template is missing
        fresh = self.tmp / "fresh"
        fresh.mkdir()
        r = self.run_ab("run", str(fresh), "--models", "m1", env=self.env(FAKE_OC_LOG=str(oc_log)))
        self.assertEqual(r.returncode, 2, r.stderr)
        self.assertIn("template missing for campaign-extend; run setup first", r.stderr)
        # resolve_crpgc, in-process
        self.assertEqual(authorbench.resolve_crpgc(str(FAKE)), str(FAKE.resolve()))
        self.assertIsNone(authorbench.resolve_crpgc("/no/file"))
        old = os.environ.get("CRPGC")
        try:
            os.environ["CRPGC"] = str(FAKE)
            self.assertIsNone(authorbench.resolve_crpgc("/no/file"))
            self.assertEqual(authorbench.resolve_crpgc(None), str(FAKE.resolve()))
            os.environ["CRPGC"] = "/no/file"
            self.assertIsNone(authorbench.resolve_crpgc(None))
        finally:
            if old is None:
                os.environ.pop("CRPGC", None)
            else:
                os.environ["CRPGC"] = old

    # CL5
    def test_cl5_run_failure_exit(self):
        self.run_ab("setup", str(self.scratch))
        r = self.run_ab("run", str(self.scratch), "--models", "m1",
                        "--briefs", "campaign-extend", "--design", "bad",
                        env=self.env(FAKE_OC_MODE="rc1"))
        self.assertEqual(r.returncode, 1, r.stderr)
        self.assertIn("campaign-extend m1 rc=1", r.stdout)
        meta = json.loads((self.scratch / "bad" / "logs" / "campaign-extend__m1.jsonl.meta.json").read_text(encoding="utf-8"))
        self.assertEqual(meta["rc"], 1)

    # CL6
    def test_cl6_score_end_to_end(self):
        self.run_ab("setup", str(self.scratch))
        self.run_ab("run", str(self.scratch), "--models", "m/a#x",
                    "--briefs", "campaign-extend", "--design", "baseline", env=self.env())
        r = self.run_ab("score", str(self.scratch), "--out", "baseline")
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(r.stdout.splitlines()[0],
                         "SCORED baseline: 1 runs, 1 valid, 0 playable, 0 excluded (smoke ok)")
        first = (self.scratch / "baseline" / "scores.json").read_bytes()
        scores = json.loads(first.decode("utf-8"))
        self.assertEqual(scores["tool"], "authorbench")
        self.assertEqual(scores["version"], 1)
        self.assertEqual(scores["design"], "baseline")
        self.assertEqual(scores["smoke"], {"rc": 0, "samples": 2, "ok": True})
        self.assertEqual(len(scores["rows"]), 1)
        row = scores["rows"][0]
        self.assertEqual(list(row.keys()), ROW_KEYS)
        self.assertEqual(row["steps"], 2)
        self.assertEqual(row["tools"], 1)
        self.assertEqual(row["crpgc_calls"], 0)
        self.assertEqual(row["errors"], 0)
        self.assertEqual(row["commands"], {})
        self.assertFalse(row["excluded"])
        self.assertTrue(row["valid"])
        self.assertFalse(row["complete"])
        self.assertFalse(row["playable"])
        self.assertTrue(row["play"]["started"])
        self.assertEqual(row["play"]["combatants"], 2)
        self.assertEqual(len(scores["cells"]), 1)
        cell = scores["cells"][0]
        self.assertEqual(cell["n"], 1)
        self.assertEqual(cell["valid"], 1)
        self.assertEqual(cell["complete"], 0)
        self.assertEqual(cell["playable"], 0)
        self.assertEqual(cell["finished"], 1)
        self.assertEqual(cell["steps_mean"], 2.0)
        # two score calls leave byte-identical files (B-15)
        r2 = self.run_ab("score", str(self.scratch), "--out", "baseline")
        self.assertEqual(r2.returncode, 0)
        second = (self.scratch / "baseline" / "scores.json").read_bytes()
        self.assertEqual(first, second)
        # control: add a wolf (3rd creature) and fake_combatants 3 -> complete, playable
        run_dir = self.scratch / "baseline" / "runs" / "campaign-extend" / "m_a_x"
        camp = run_dir / "campaign" / "campaign.json"
        data = json.loads(camp.read_text(encoding="utf-8"))
        data["fake_combatants"] = 3
        camp.write_text(json.dumps(data), encoding="utf-8")
        (run_dir / "campaign" / "creatures" / "wolf.json").write_text(
            json.dumps({"schema": "crpg.creature/1", "slug": "wolf"}), encoding="utf-8")
        r3 = self.run_ab("score", str(self.scratch), "--out", "baseline")
        self.assertEqual(r3.returncode, 0)
        self.assertIn("1 playable", r3.stdout.splitlines()[0])
        scores2 = json.loads((self.scratch / "baseline" / "scores.json").read_text(encoding="utf-8"))
        row2 = scores2["rows"][0]
        self.assertTrue(row2["complete"])
        self.assertEqual(row2["play"]["combatants"], 3)
        self.assertTrue(row2["playable"])

    # CL7
    def test_cl7_score_excluded_and_unknown(self):
        self.run_ab("setup", str(self.scratch))
        self.run_ab("run", str(self.scratch), "--models", "m1",
                    "--briefs", "campaign-extend", "--design", "bad",
                    env=self.env(FAKE_OC_MODE="rc1"))
        r = self.run_ab("score", str(self.scratch), "--out", "bad")
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(r.stdout.splitlines()[0],
                         "SCORED bad: 1 runs, 0 valid, 0 playable, 1 excluded (smoke ok)")
        scores = json.loads((self.scratch / "bad" / "scores.json").read_text(encoding="utf-8"))
        row = scores["rows"][0]
        self.assertTrue(row["excluded"])
        self.assertFalse(row["campaign_present"])
        self.assertEqual(row["files"], 0)
        self.assertFalse(row["valid"])
        self.assertFalse(row["playable"])
        self.assertIsNone(row["play"]["rc"])
        cell = scores["cells"][0]
        self.assertEqual(cell["n"], 0)
        self.assertEqual(cell["excluded"], 1)
        self.assertIsNone(cell["steps_mean"])
        # a meta whose brief is unknown
        meta_path = self.scratch / "bad" / "logs" / "campaign-extend__m1.jsonl.meta.json"
        meta = json.loads(meta_path.read_text(encoding="utf-8"))
        meta["brief"] = "nope"
        meta_path.write_text(json.dumps(meta, indent=1), encoding="utf-8")
        r2 = self.run_ab("score", str(self.scratch), "--out", "bad")
        self.assertEqual(r2.returncode, 2)
        self.assertIn("score: unknown brief nope", r2.stderr)
        # an empty logs directory
        (self.scratch / "empty" / "logs").mkdir(parents=True)
        r3 = self.run_ab("score", str(self.scratch), "--out", "empty")
        self.assertEqual(r3.returncode, 2)
        self.assertIn("score: no runs in empty", r3.stderr)

    # CL8
    def test_cl8_run_retries_locked_database(self):
        self.run_ab("setup", str(self.scratch))
        state = self.tmp / "state"
        state.mkdir()
        oc_log = self.tmp / "oc.log"
        r = self.run_ab("run", str(self.scratch), "--models", "m1",
                        "--briefs", "campaign-extend", "--design", "baseline",
                        env=self.env(FAKE_OC_MODE="locked:2", FAKE_OC_STATE=str(state), FAKE_OC_LOG=str(oc_log)))
        self.assertEqual(r.returncode, 0, r.stderr)
        records = [json.loads(line) for line in oc_log.read_text(encoding="utf-8").splitlines()]
        self.assertEqual(len(records), 3)
        meta = json.loads((self.scratch / "baseline" / "logs" / "campaign-extend__m1.jsonl.meta.json").read_text(encoding="utf-8"))
        self.assertEqual(meta["rc"], 0)
        log_text = (self.scratch / "baseline" / "logs" / "campaign-extend__m1.jsonl").read_text(encoding="utf-8")
        self.assertEqual(log_text.count('"step_finish"'), 2)
        # always locked: three attempts, then rc 1 and exit 1
        state2 = self.tmp / "state2"
        state2.mkdir()
        oc_log2 = self.tmp / "oc2.log"
        r2 = self.run_ab("run", str(self.scratch), "--models", "m1",
                         "--briefs", "campaign-extend", "--design", "locked",
                         env=self.env(FAKE_OC_MODE="locked", FAKE_OC_STATE=str(state2), FAKE_OC_LOG=str(oc_log2)))
        self.assertEqual(r2.returncode, 1, r2.stderr)
        records2 = [json.loads(line) for line in oc_log2.read_text(encoding="utf-8").splitlines()]
        self.assertEqual(len(records2), 3)
        meta2 = json.loads((self.scratch / "locked" / "logs" / "campaign-extend__m1.jsonl.meta.json").read_text(encoding="utf-8"))
        self.assertEqual(meta2["rc"], 1)

    # CL9
    def test_cl9_no_absolute_paths(self):
        self.run_ab("setup", str(self.scratch))
        self.run_ab("run", str(self.scratch), "--models", "m/a#x",
                    "--briefs", "campaign-extend", "--design", "baseline", env=self.env())
        self.run_ab("score", str(self.scratch), "--out", "baseline")
        scores_text = (self.scratch / "baseline" / "scores.json").read_text(encoding="utf-8")
        meta_text = (self.scratch / "baseline" / "logs" / "campaign-extend__m_a_x.jsonl.meta.json").read_text(encoding="utf-8")
        roots = [str(self.scratch), str(REPO), str(Path.home())]
        for root in roots:
            variants = {root, root.replace("\\", "/"), root.replace("/", "\\")}
            for variant in variants:
                self.assertNotIn(variant, scores_text)
                self.assertNotIn(variant, meta_text)
        # control: an error event holding the scratch path is scrubbed to <path>
        scratch_fwd = str(self.scratch).replace("\\", "/")
        log_path = self.scratch / "baseline" / "logs" / "campaign-extend__m_a_x.jsonl"
        log_path.write_text(json.dumps({"type": "error", "error": {"message": f"failed in {scratch_fwd}/x"}}) + "\n",
                            encoding="utf-8")
        self.run_ab("score", str(self.scratch), "--out", "baseline")
        scores2 = json.loads((self.scratch / "baseline" / "scores.json").read_text(encoding="utf-8"))
        err = scores2["rows"][0]["err"]
        self.assertIn("<path>", err)
        self.assertNotIn(scratch_fwd, err)

    # CL10
    def test_cl10_shipped_briefs_and_data(self):
        for name in BRIEF_ORDER:
            brief = scoring.load_brief(AUTHORBENCH_DIR / "briefs" / name)
            self.assertEqual(brief["name"], name)
            self.assertEqual(brief["kind"], EXPECTED_BRIEFS[name]["kind"])
            self.assertEqual(brief["starter"], EXPECTED_BRIEFS[name]["starter"])
            self.assertEqual(brief["minimum"], EXPECTED_BRIEFS[name]["minimum"])
            self.assertEqual(brief["ruleset_slug"], EXPECTED_BRIEFS[name]["ruleset_slug"])
            self.assertEqual(brief["min_combatants"], EXPECTED_BRIEFS[name]["min_combatants"])
            self.assertEqual(brief["timeout_s"], EXPECTED_BRIEFS[name]["timeout_s"])
        for name in BRIEF_ORDER:
            prompt = (AUTHORBENCH_DIR / "briefs" / name / "prompt.txt").read_text(encoding="utf-8")
            self.assertEqual(prompt, HEAD + BUILDS[name] + TAIL)
            self.assertEqual(prompt.count("{{TOOLS}}"), 1)
        self.assertEqual((AUTHORBENCH_DIR / "tools-baseline.txt").read_text(encoding="utf-8"), TOOLS_TEXT)
        self.assertEqual((AUTHORBENCH_DIR / "models.txt").read_text(encoding="utf-8"), MODELS_TEXT)
        shipped = ([AUTHORBENCH_DIR / "tools-baseline.txt", AUTHORBENCH_DIR / "models.txt"]
                   + [AUTHORBENCH_DIR / "briefs" / name / "prompt.txt" for name in BRIEF_ORDER]
                   + [AUTHORBENCH_DIR / "briefs" / name / "brief.json" for name in BRIEF_ORDER])
        for path in shipped:
            text = path.read_text(encoding="utf-8")
            self.assertIsNone(_BANNED_RE.search(text), path)
        # control: the pin is live — a one-word change breaks the equality
        self.assertNotEqual(BUILDS["campaign-small"], BUILDS["campaign-small"].replace("goblin", "orc", 1))

    # CL11
    def test_cl11_compare_cli(self):
        b_dir = self.scratch / "b"
        c_dir = self.scratch / "c"
        b_dir.mkdir(parents=True)
        c_dir.mkdir(parents=True)
        base = {"tool": "authorbench", "version": 1, "design": "b",
                "smoke": {"rc": 0, "samples": 2, "ok": True}, "rows": [],
                "cells": [
                    {"brief": "campaign-small", "model": "m1", "n": 2, "excluded": 0,
                     "valid": 2, "complete": 2, "playable": 1, "finished": 0,
                     "steps_mean": 10.0, "errors_mean": 1.0, "secs_mean": 0.0, "cost_total": 0.0},
                    {"brief": "campaign-small", "model": "m2", "n": 2, "excluded": 0,
                     "valid": 2, "complete": 2, "playable": 1, "finished": 0,
                     "steps_mean": 10.0, "errors_mean": 1.0, "secs_mean": 0.0, "cost_total": 0.0},
                ]}
        cand = {"tool": "authorbench", "version": 1, "design": "c",
                "smoke": {"rc": 0, "samples": 2, "ok": True}, "rows": [],
                "cells": [
                    {"brief": "campaign-small", "model": "m1", "n": 2, "excluded": 0,
                     "valid": 2, "complete": 2, "playable": 2, "finished": 0,
                     "steps_mean": 12.5, "errors_mean": 0.0, "secs_mean": 0.0, "cost_total": 0.0},
                ]}
        (b_dir / "scores.json").write_text(json.dumps(base, indent=1) + "\n", encoding="utf-8")
        (c_dir / "scores.json").write_text(json.dumps(cand, indent=1) + "\n", encoding="utf-8")
        r = self.run_ab("compare", str(self.scratch), "b", "c")
        self.assertEqual(r.returncode, 0, r.stderr)
        lines = r.stdout.splitlines()
        self.assertEqual(lines[0], "COMPARE b -> c: 1 better, 0 same, 0 worse")
        self.assertIn("playable 1/2 -> 2/2", lines[1])
        self.assertIn("steps +2.5", lines[1])
        self.assertIn("errors -1.0", lines[1])
        self.assertIn("better", lines[1])
        rj = self.run_ab("compare", str(self.scratch), "b", "c", "--json")
        self.assertEqual(rj.returncode, 0, rj.stderr)
        parsed = json.loads(rj.stdout)
        self.assertEqual(len(parsed), 1)
        self.assertEqual(parsed[0]["verdict"], "better")
        r2 = self.run_ab("compare", str(self.scratch), "b", "missing")
        self.assertEqual(r2.returncode, 2)
        self.assertIn("compare: no scores in missing", r2.stderr)

    # CL12
    def test_cl12_all_and_usage(self):
        r = self.run_ab("all", str(self.scratch), "--models", "m1", "--briefs", "campaign-small")
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertTrue((self.scratch / "tpl").is_dir())
        self.assertTrue((self.scratch / "baseline" / "runs").is_dir())
        scores = json.loads((self.scratch / "baseline" / "scores.json").read_text(encoding="utf-8"))
        self.assertEqual(len(scores["rows"]), 1)
        self.assertEqual(scores["rows"][0]["brief"], "campaign-small")
        # a failed run still scores, and `all` returns the run's exit code
        r2 = self.run_ab("all", str(self.scratch), "--models", "m1", "--briefs", "campaign-small",
                         env=self.env(FAKE_OC_MODE="rc1"))
        self.assertEqual(r2.returncode, 1, r2.stderr)
        self.assertTrue((self.scratch / "baseline" / "scores.json").is_file())
        # no subcommand -> argparse usage, exit 2
        r3 = self.run_ab()
        self.assertEqual(r3.returncode, 2)


if __name__ == "__main__":
    unittest.main()

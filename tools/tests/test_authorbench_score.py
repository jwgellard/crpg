#!/usr/bin/env python3
"""Tests for tools/agents/authorbench/authorbench_scoring.py (T100a)."""

from __future__ import annotations

import copy
import importlib.util
import json
import os
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]
SCORING = REPO / "tools" / "agents" / "authorbench" / "authorbench_scoring.py"
FAKE = HERE / "fixtures" / "authorbench" / "fake_crpgc.py"
PREFIX = [sys.executable, str(FAKE)]


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


scoring = load("authorbench_scoring_under_test", SCORING)

SMALL = {
    "name": "campaign-small",
    "kind": "campaign",
    "starter": None,
    "minimum": {"creature": 2, "ability": 1, "outcome-table": 1, "ruleset": 1, "encounter": 1},
    "ruleset_slug": None,
    "min_combatants": 2,
    "timeout_s": 1800,
}

SMALL_DOCS = [
    ("creatures/a.json", "crpg.creature/1", {}),
    ("creatures/b.json", "crpg.creature/1", {}),
    ("abilities/s.json", "crpg.ability/1", {}),
    ("outcome_tables/t.json", "crpg.outcome-table/1", {}),
    ("rulesets/r.json", "crpg.ruleset/1", {"slug": "r"}),
    ("encounters/e.json", "crpg.encounter/1", {}),
]


def make_campaign(root, marker=None, combatants=None, docs=()):
    root = Path(root)
    root.mkdir(parents=True, exist_ok=True)
    data = {"schema": "crpg.campaign/1", "slug": "t"}
    if marker:
        data["note"] = marker
    if combatants is not None:
        data["fake_combatants"] = combatants
    (root / "campaign.json").write_text(json.dumps(data), encoding="utf-8")
    for relpath, schema, extra in docs:
        path = root / relpath
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps({"schema": schema, "slug": "d", **extra}), encoding="utf-8")
    return root


class AuthorbenchScoreCase(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="authorbench-score "))
        self.addCleanup(self._cleanup)

    def _cleanup(self):
        import shutil
        shutil.rmtree(self.tmp, ignore_errors=True)

    def make_brief_dir(self, tag, brief=None, prompt="a {{TOOLS}} b", name="campaign-small"):
        d = self.tmp / tag / name
        d.mkdir(parents=True, exist_ok=True)
        if brief is not None:
            if isinstance(brief, str):
                (d / "brief.json").write_text(brief, encoding="utf-8")
            else:
                (d / "brief.json").write_text(json.dumps(brief), encoding="utf-8")
        if prompt is not None:
            (d / "prompt.txt").write_text(prompt, encoding="utf-8")
        return d

    def check_refusal(self, tag, brief, text, prompt="a {{TOOLS}} b", name="campaign-small"):
        d = self.make_brief_dir(tag, brief=brief, prompt=prompt, name=name)
        with self.assertRaises(scoring.BriefError) as ctx:
            scoring.load_brief(d)
        self.assertEqual(str(ctx.exception), f"brief {name}: {text}")

    # SC1
    def test_sc1_load_brief(self):
        d = self.make_brief_dir("accept", brief=SMALL, prompt="a {{TOOLS}} b")
        self.assertEqual(scoring.load_brief(d), {**SMALL, "prompt": "a {{TOOLS}} b"})

        self.check_refusal("unknown", {**SMALL, "extra": 1}, "unknown key extra")
        missing_timeout = dict(SMALL)
        del missing_timeout["timeout_s"]
        self.check_refusal("missing", missing_timeout, "missing key timeout_s")
        self.check_refusal("name", {**SMALL, "name": "x"}, "name must equal the directory name")
        self.check_refusal("kind", {**SMALL, "kind": "big"}, "kind must be one of extend, campaign, ruleset")
        self.check_refusal("extend-null", {**SMALL, "kind": "extend", "starter": None},
                           "starter must be a repository-relative path")
        self.check_refusal("starter-campaign", {**SMALL, "starter": "campaigns/x"}, "starter must be null")
        self.check_refusal("starter-dotdot", {**SMALL, "kind": "extend", "starter": "../x"},
                           "starter must be a repository-relative path")
        self.check_refusal("min-empty", {**SMALL, "minimum": {}}, "minimum must map kinds to integers >= 1")
        self.check_refusal("min-zero", {**SMALL, "minimum": {"creature": 0}},
                           "minimum must map kinds to integers >= 1")
        self.check_refusal("min-bool", {**SMALL, "minimum": {"creature": True}},
                           "minimum must map kinds to integers >= 1")
        self.check_refusal("slug-null", {**SMALL, "kind": "ruleset", "ruleset_slug": None},
                           "ruleset_slug must be a string for kind ruleset, else null")
        self.check_refusal("combatants", {**SMALL, "min_combatants": 9},
                           "min_combatants must be an integer 0..8")
        self.check_refusal("timeout", {**SMALL, "timeout_s": 59}, "timeout_s must be an integer 60..7200")
        self.check_refusal("no-prompt", SMALL, "prompt.txt is missing", prompt=None)
        self.check_refusal("prompt-twice", SMALL, "prompt.txt must contain {{TOOLS}} exactly once",
                           prompt="a {{TOOLS}} b {{TOOLS}} c")
        self.check_refusal("prompt-none", SMALL, "prompt.txt must contain {{TOOLS}} exactly once",
                           prompt="a b c")
        # brief.json missing entirely
        self.check_refusal("brief-missing", None, "brief.json is not a JSON object")
        # brief.json not JSON
        self.check_refusal("brief-notjson", "not json", "brief.json is not a JSON object")
        # brief.json a JSON array
        self.check_refusal("brief-array", "[]", "brief.json is not a JSON object")

    # SC2
    def test_sc2_read_log(self):
        lines = [
            {"type": "step_start", "part": {}},
            {"type": "tool_use", "part": {"tool": "read", "state": {"status": "completed", "input": {"path": "brief.md"}}}},
            {"type": "tool_use", "part": {"tool": "bash", "state": {"status": "completed", "input": {"command": "crpgc schema creature"}, "metadata": {"exit": 0}}}},
            {"type": "step_finish", "part": {"tokens": {"input": 100, "output": 20, "reasoning": 5, "cache": {"read": 50}}, "cost": 0.01}},
            {"type": "tool_use", "part": {"tool": "bash", "state": {"status": "completed", "input": {"command": "crpgc validate campaign --json"}, "metadata": {"exit": 1}}}},
            {"type": "tool_use", "part": {"tool": "bash", "state": {"status": "completed", "input": {"command": "ls campaign"}, "metadata": {"exit": 0}}}},
            {"type": "step_finish", "part": {"tokens": {"input": 200, "output": 30, "reasoning": 0, "cache": {"read": 150}}, "cost": 0.02}},
            {"type": "tool_use", "part": {"tool": "bash", "state": {"status": "error", "input": {"command": "crpgc play --campaign campaign"}}}},
            {"type": "tool_use", "part": {"tool": "bash", "state": {"status": "completed", "input": {"command": "crpgc validate campaign --json && crpgc fmt campaign"}, "metadata": {"exit": 0}}}},
            {"type": "step_finish", "part": {"tokens": {"input": 10, "output": 1, "reasoning": 0, "cache": {"read": 0}}, "cost": 0.005}},
            {"type": "text", "part": {"text": "DONE"}},
        ]
        log = self.tmp / "log.jsonl"

        def write(text):
            log.write_text(text, encoding="utf-8")

        base_text = "\n".join(json.dumps(l) for l in lines) + "\n"
        write(base_text)
        result = scoring.read_log(log)
        self.assertEqual(result["steps"], 3)
        self.assertEqual(result["tools"], 6)
        self.assertEqual(result["crpgc_calls"], 4)
        self.assertEqual(result["crpgc_failures"], 2)
        self.assertEqual(result["commands"], {"fmt": 1, "play": 1, "schema": 1, "validate": 2})
        self.assertEqual(result["tok"], {"input": 310, "output": 51, "reasoning": 5, "cache_read": 200})
        self.assertAlmostEqual(result["cost"], 0.035, delta=1e-9)
        self.assertEqual(result["err"], "")

        # control: a non-bash tool whose input holds a crpgc-shaped command is
        # still a tool, but is never a crpgc call
        write(base_text + json.dumps(
            {"type": "tool_use", "part": {"tool": "read",
             "state": {"status": "completed", "input": {"command": "crpgc validate x"}}}}) + "\n")
        result = scoring.read_log(log)
        self.assertEqual(result["crpgc_calls"], 4)
        self.assertEqual(result["tools"], 7)
        self.assertEqual(result["commands"], {"fmt": 1, "play": 1, "schema": 1, "validate": 2})

        # control: append a JSON error event
        write(base_text + json.dumps({"type": "error", "error": {"message": "boom"}}) + "\n")
        result = scoring.read_log(log)
        self.assertEqual(result["err"], '{"message": "boom"}')
        self.assertEqual(result["steps"], 3)
        self.assertEqual(result["crpgc_failures"], 2)

        # control: append a non-JSON line containing "rror"
        write(base_text + "an error here\n")
        result = scoring.read_log(log)
        self.assertEqual(result["err"], "an error here")

        # control: both, JSON error line first
        write(base_text + json.dumps({"type": "error", "error": {"message": "boom"}}) + "\n" + "an error here\n")
        result = scoring.read_log(log)
        self.assertEqual(result["err"], '{"message": "boom"}')

        # control: metadata.exit true is not an integer failure
        lines2 = copy.deepcopy(lines)
        lines2[4]["part"]["state"]["metadata"]["exit"] = True
        write("\n".join(json.dumps(l) for l in lines2) + "\n")
        result = scoring.read_log(log)
        self.assertEqual(result["crpgc_failures"], 1)

        # control: missing file
        result = scoring.read_log(self.tmp / "nope.jsonl")
        self.assertEqual(result, {
            "steps": 0, "tools": 0, "crpgc_calls": 0, "crpgc_failures": 0,
            "commands": {}, "tok": {"input": 0, "output": 0, "reasoning": 0, "cache_read": 0},
            "cost": 0.0, "err": "",
        })

    # SC2 control: a line is one \n-terminated line; a raw U+2028 inside a JSON
    # string must not split one event into two non-JSON fragments.
    def test_sc2_line_splitting(self):
        log = self.tmp / "u2028.jsonl"
        line = json.dumps({
            "type": "tool_use",
            "part": {"tool": "bash",
                     "state": {"status": "completed",
                               "input": {"command": "crpgc validate \u2028x"},
                               "metadata": {"exit": 0}}},
        }, ensure_ascii=False)
        log.write_text(line + "\n", encoding="utf-8")
        self.assertIn("\u2028", log.read_text(encoding="utf-8"))
        result = scoring.read_log(log)
        self.assertEqual(result["tools"], 1)
        self.assertEqual(result["crpgc_calls"], 1)
        self.assertEqual(result["commands"], {"validate": 1})
        self.assertEqual(result["crpgc_failures"], 0)

    # SC2 (T100-E1): the shell tool, with the exit code one level deeper.
    def test_sc2_shell_tool_nested_exit(self):
        log = self.tmp / "shell.jsonl"

        def write(part):
            log.write_text(json.dumps({"type": "tool_use", "part": part}) + "\n",
                           encoding="utf-8")
            return scoring.read_log(log)

        def shell_part(nested_exit):
            return {"tool": "shell", "state": {
                "status": "completed",
                "input": {"command": "crpgc validate campaign --json"},
                "metadata": {"status": "completed", "truncated": False,
                             "metadata": {"exit": nested_exit}},
            }}

        result = write(shell_part(1))
        self.assertEqual(result["tools"], 1)
        self.assertEqual(result["crpgc_calls"], 1)
        self.assertEqual(result["crpgc_failures"], 1)
        self.assertEqual(result["commands"], {"validate": 1})

        # control: the same crpgc call with the nested exit 0 counts one call
        # and no failure
        result = write(shell_part(0))
        self.assertEqual(result["crpgc_calls"], 1)
        self.assertEqual(result["crpgc_failures"], 0)
        self.assertEqual(result["commands"], {"validate": 1})

        # control: a nested boolean exit is not an integer, so not a failure
        result = write(shell_part(True))
        self.assertEqual(result["crpgc_calls"], 1)
        self.assertEqual(result["crpgc_failures"], 0)

    # SC3
    def test_sc3_call_crpgc_and_prefix(self):
        self.assertEqual(scoring.crpgc_prefix("x.py"), [sys.executable, "x.py"])
        self.assertEqual(scoring.crpgc_prefix("/b/crpgc"), ["/b/crpgc"])

        d = make_campaign(self.tmp / "c")
        res = scoring.call_crpgc(PREFIX, ["validate", str(d), "--json"])
        self.assertEqual(res, {"rc": 0, "stdout": "[]\n", "stderr": "", "timeout": False})

        res2 = scoring.call_crpgc(PREFIX, ["play", "--campaign", str(d)], stdin_bytes=b"1\n" * 400)
        self.assertEqual(res2["rc"], 0)
        self.assertEqual(res2["stderr"], "stdin_lines=400\n")

        os.environ["FAKE_CRPGC_SLEEP"] = "5"
        try:
            res3 = scoring.call_crpgc(PREFIX, ["run"], timeout_s=1)
        finally:
            del os.environ["FAKE_CRPGC_SLEEP"]
        self.assertEqual(res3["rc"], None)
        self.assertTrue(res3["timeout"])
        self.assertEqual(res3["stdout"], "")

        res4 = scoring.call_crpgc(["/no/such/binary"], ["run"])
        self.assertEqual(res4["rc"], None)
        self.assertFalse(res4["timeout"])
        self.assertEqual(res4["stdout"], "")
        self.assertTrue(res4["stderr"])

    # SC4
    def test_sc4_scan_campaign(self):
        d = make_campaign(self.tmp / "c", docs=SMALL_DOCS)
        (d / "notes.txt").write_text("hello", encoding="utf-8")
        (d / "bad.json").write_text("not json", encoding="utf-8")
        (d / "list.json").write_text("[1]", encoding="utf-8")
        (d / "rulesets" / "z.json").write_text(json.dumps({"schema": "crpg.ruleset/2", "slug": "z"}), encoding="utf-8")
        (d / "x.json").write_text(json.dumps({"schema": "nope"}), encoding="utf-8")

        result = scoring.scan_campaign(d)
        self.assertEqual(result["files"], 12)
        self.assertEqual(result["json"], 11)
        self.assertEqual(result["unreadable"], 2)
        self.assertEqual(result["kinds"], {
            "ability": 1, "campaign": 1, "creature": 2, "encounter": 1,
            "outcome-table": 1, "ruleset": 2,
        })
        self.assertEqual(result["ruleset_slugs"], ["r", "z"])

        (d / "rulesets" / "z.json").unlink()
        result2 = scoring.scan_campaign(d)
        self.assertEqual(result2["ruleset_slugs"], ["r"])

    # SC4 control: a symlink inside campaign/ to an outside JSON file is not
    # counted and contributes no kind or slug.
    def test_sc4_scan_campaign_symlink(self):
        d = make_campaign(self.tmp / "c", docs=SMALL_DOCS)
        outside = self.tmp / "outside.json"
        outside.write_text(json.dumps({"schema": "crpg.ruleset/9", "slug": "ghost"}), encoding="utf-8")
        link = d / "creatures" / "link.json"
        try:
            link.symlink_to(outside)
        except (OSError, NotImplementedError):
            self.skipTest("cannot create symlinks on this platform")
        result = scoring.scan_campaign(d)
        self.assertEqual(result["files"], 7)
        self.assertEqual(result["json"], 7)
        self.assertEqual(result["kinds"]["ruleset"], 1)
        self.assertEqual(result["ruleset_slugs"], ["r"])

    # SC5
    def test_sc5_output_readers(self):
        self.assertEqual(scoring.parse_diagnostics("[]"), [])
        self.assertIsNone(scoring.parse_diagnostics("[1]"))
        self.assertIsNone(scoring.parse_diagnostics("not json"))
        self.assertIsNone(scoring.parse_diagnostics(""))
        self.assertIsNone(scoring.parse_diagnostics('{"a":1}'))
        self.assertEqual(scoring.parse_diagnostics(json.dumps([{"a": 1}, {"b": 2}])), [{"a": 1}, {"b": 2}])

        self.assertEqual(scoring.count_samples("50 " + "a" * 64 + "\n100 " + "b" * 64 + "\n"), 2)
        self.assertEqual(scoring.count_samples("50 " + "a" * 64 + "\n100 " + "b" * 63 + "\n"), 1)
        self.assertEqual(scoring.count_samples("50 " + "A" * 64 + "\n100 " + "b" * 64 + "\n"), 1)

        full = ("== Combat demo: Fake ==\n"
                "Encounter: Fake Fight\n"
                "Combatants:\n"
                "  Unit0 | side Unit0 | 10/10 hp\n"
                "  Unit1 | side Unit1 | 10/10 hp\n"
                "\n"
                "== Summary ==\n"
                "Rounds played: 3\n"
                "Final health:\n"
                "  Unit0 10/10 hp\n"
                "Survivors: Unit0\n"
                "Winner: Unit0\n")
        quits = ("== Combat demo: Fake ==\n"
                 "Encounter: Fake Fight\n"
                 "Combatants:\n"
                 "  Unit0 | side Unit0 | 10/10 hp\n"
                 "  Unit1 | side Unit1 | 10/10 hp\n"
                 "\n"
                 "Quit.\n")

        self.assertEqual(scoring.parse_play(full, 0),
                         {"started": True, "finished": True, "combatants": 2, "rounds": 3})
        self.assertEqual(scoring.parse_play(quits, 0),
                         {"started": True, "finished": False, "combatants": 2, "rounds": None})
        self.assertEqual(scoring.parse_play(full, 1),
                         {"started": False, "finished": False, "combatants": 0, "rounds": None})
        self.assertEqual(scoring.parse_play(full, None),
                         {"started": False, "finished": False, "combatants": 0, "rounds": None})
        self.assertEqual(scoring.parse_play("", 0),
                         {"started": False, "finished": False, "combatants": 0, "rounds": None})

        three = ("== Combat demo: Fake ==\n"
                 "Encounter: Fake Fight\n"
                 "Combatants:\n"
                 "  Unit0 | side Unit0 | 10/10 hp\n"
                 "  Unit1 | side Unit1 | 10/10 hp\n"
                 "  Unit2 | side Unit2 | 10/10 hp\n"
                 "\n"
                 "== Summary ==\n")
        self.assertEqual(scoring.parse_play(three, 0)["combatants"], 3)

        trailing = ("== Combat demo: Fake ==\n"
                    "Encounter: Fake Fight\n"
                    "Combatants:\n"
                    "  Unit0 | side Unit0 | 10/10 hp\n"
                    "\n"
                    "  Unit9 | side Unit9 | 10/10 hp\n")
        self.assertEqual(scoring.parse_play(trailing, 0)["combatants"], 1)

    # SC6
    def test_sc6_smoke_check(self):
        self.assertEqual(scoring.smoke_check(PREFIX), {"rc": 0, "samples": 2, "ok": True})
        os.environ["FAKE_CRPGC_SMOKE_RC"] = "3"
        try:
            self.assertEqual(scoring.smoke_check(PREFIX), {"rc": 3, "samples": 0, "ok": False})
        finally:
            del os.environ["FAKE_CRPGC_SMOKE_RC"]

    # SC7
    def test_sc7_score_run_healthy(self):
        run_dir = self.tmp / "run"
        make_campaign(run_dir / "campaign", docs=SMALL_DOCS)
        result = scoring.score_run(SMALL, run_dir, PREFIX)
        self.assertEqual(result, {
            "campaign_present": True,
            "files": 7,
            "kinds": {"ability": 1, "campaign": 1, "creature": 2, "encounter": 1,
                      "outcome-table": 1, "ruleset": 1},
            "valid": True,
            "validate": {"rc": 0, "errors": 0, "warnings": 0, "codes": [], "unparsable": False},
            "complete": True,
            "play": {"rc": 0, "started": True, "finished": True, "combatants": 2, "rounds": 3},
            "playable": True,
            "first_error": None,
        })

    # SC8
    def test_sc8_score_run_controls(self):
        not_run_play = {"rc": None, "started": False, "finished": False, "combatants": 0, "rounds": None}

        # (a) BROKEN
        d = make_campaign(self.tmp / "a" / "campaign", marker="BROKEN", docs=SMALL_DOCS)
        result = scoring.score_run(SMALL, self.tmp / "a", PREFIX)
        self.assertFalse(result["valid"])
        self.assertEqual(result["validate"],
                         {"rc": 1, "errors": 1, "warnings": 1, "codes": ["missing_field"], "unparsable": False})
        self.assertEqual(result["first_error"],
                         {"file": "campaign.json", "code": "missing_field",
                          "message": "campaign name is required"})
        self.assertEqual(result["play"], not_run_play)
        self.assertTrue(result["complete"])
        self.assertFalse(result["playable"])

        # (b) WARN
        make_campaign(self.tmp / "b" / "campaign", marker="WARN", docs=SMALL_DOCS)
        result = scoring.score_run(SMALL, self.tmp / "b", PREFIX)
        self.assertTrue(result["valid"])
        self.assertEqual(result["validate"]["warnings"], 1)
        self.assertIsNone(result["first_error"])
        self.assertTrue(result["playable"])

        # (c) BADOUT
        make_campaign(self.tmp / "c" / "campaign", marker="BADOUT", docs=SMALL_DOCS)
        result = scoring.score_run(SMALL, self.tmp / "c", PREFIX)
        self.assertFalse(result["valid"])
        self.assertTrue(result["validate"]["unparsable"])
        self.assertEqual(result["validate"]["rc"], 0)
        self.assertIsNone(result["play"]["rc"])

        # (d) no abilities/s.json
        d = make_campaign(self.tmp / "d" / "campaign", docs=SMALL_DOCS)
        (d / "abilities" / "s.json").unlink()
        result = scoring.score_run(SMALL, self.tmp / "d", PREFIX)
        self.assertTrue(result["valid"])
        self.assertFalse(result["complete"])
        self.assertTrue(result["play"]["started"])
        self.assertFalse(result["playable"])

        # (e) combatants = 1
        make_campaign(self.tmp / "e" / "campaign", combatants=1, docs=SMALL_DOCS)
        result = scoring.score_run(SMALL, self.tmp / "e", PREFIX)
        self.assertEqual(result["play"]["combatants"], 1)
        self.assertFalse(result["playable"])
        result = scoring.score_run({**SMALL, "min_combatants": 1}, self.tmp / "e", PREFIX)
        self.assertTrue(result["playable"])

        # (f) QUITS
        make_campaign(self.tmp / "f" / "campaign", marker="QUITS", docs=SMALL_DOCS)
        result = scoring.score_run(SMALL, self.tmp / "f", PREFIX)
        self.assertTrue(result["play"]["started"])
        self.assertFalse(result["play"]["finished"])
        self.assertTrue(result["playable"])

        # (g) NOENC
        make_campaign(self.tmp / "g" / "campaign", marker="NOENC", docs=SMALL_DOCS)
        result = scoring.score_run(SMALL, self.tmp / "g", PREFIX)
        self.assertEqual(result["play"]["rc"], 1)
        self.assertFalse(result["play"]["started"])
        self.assertFalse(result["playable"])

        # (h) no campaign/ directory
        run_dir = self.tmp / "h"
        run_dir.mkdir()
        result = scoring.score_run(SMALL, run_dir, PREFIX)
        self.assertEqual(result, scoring.empty_score())
        self.assertEqual(scoring.empty_score(), {
            "campaign_present": False,
            "files": 0,
            "kinds": {},
            "valid": False,
            "validate": {"rc": None, "errors": 0, "warnings": 0, "codes": [], "unparsable": False},
            "complete": False,
            "play": not_run_play,
            "playable": False,
            "first_error": None,
        })

        # (i) ruleset_slug tiny
        d = make_campaign(self.tmp / "i" / "campaign", docs=SMALL_DOCS)
        brief = {**SMALL, "kind": "ruleset", "ruleset_slug": "tiny"}
        result = scoring.score_run(brief, self.tmp / "i", PREFIX)
        self.assertFalse(result["complete"])
        (d / "rulesets" / "r.json").write_text(json.dumps({"schema": "crpg.ruleset/1", "slug": "tiny"}), encoding="utf-8")
        result = scoring.score_run(brief, self.tmp / "i", PREFIX)
        self.assertTrue(result["complete"])

    # SC9
    def test_sc9_aggregate_and_compare(self):
        rows = [
            {"brief": "campaign-small", "model": "m1", "rc": 0, "steps": 10, "errors": 2,
             "secs": 10.0, "cost": 0.1, "valid": True, "complete": True, "playable": True,
             "play": {"finished": True}},
            {"brief": "campaign-small", "model": "m1", "rc": 0, "steps": 20, "errors": 4,
             "secs": 20.0, "cost": 0.2, "valid": True, "complete": False, "playable": False,
             "play": {"finished": False}},
            {"brief": "campaign-small", "model": "m1", "rc": 1, "steps": 0, "cost": 0.3},
            {"brief": "campaign-small", "model": "m1", "rc": "TIMEOUT", "steps": 5, "errors": 1,
             "secs": 30.0, "cost": 0, "valid": False, "complete": False, "playable": False,
             "play": {"finished": False}},
        ]
        cells = scoring.aggregate(rows, ["campaign-small"])
        self.assertEqual(cells, [{
            "brief": "campaign-small", "model": "m1", "n": 3, "excluded": 1,
            "valid": 2, "complete": 1, "playable": 1, "finished": 1,
            "steps_mean": 11.7, "errors_mean": 2.3, "secs_mean": 20.0, "cost_total": 0.6,
        }])

        # a model with only an excluded row
        cells2 = scoring.aggregate([{"brief": "campaign-small", "model": "m1", "rc": 1, "steps": 0, "cost": 0.3}],
                                   ["campaign-small"])
        self.assertEqual(cells2[0]["n"], 0)
        self.assertIsNone(cells2[0]["steps_mean"])

        # ordering by brief_order then model: feed m2 first, m1 still sorts first
        rows3 = [
            {"brief": "campaign-large", "model": "m1", "rc": 0, "steps": 1, "errors": 0,
             "secs": 1.0, "cost": 0, "valid": True, "complete": True, "playable": True,
             "play": {"finished": True}},
            {"brief": "campaign-small", "model": "m2", "rc": 0, "steps": 1, "errors": 0,
             "secs": 1.0, "cost": 0, "valid": True, "complete": True, "playable": True,
             "play": {"finished": True}},
            {"brief": "campaign-small", "model": "m1", "rc": 0, "steps": 1, "errors": 0,
             "secs": 1.0, "cost": 0, "valid": True, "complete": True, "playable": True,
             "play": {"finished": True}},
        ]
        cells3 = scoring.aggregate(rows3, ["campaign-large", "campaign-small"])
        self.assertEqual([c["brief"] for c in cells3],
                         ["campaign-large", "campaign-small", "campaign-small"])
        self.assertEqual([c["model"] for c in cells3[1:]], ["m1", "m2"])

        base = [{"brief": "campaign-small", "model": "m1", "n": 2, "valid": 2, "complete": 2,
                 "playable": 1, "steps_mean": 10.0, "errors_mean": 1.0}]
        cand = [{"brief": "campaign-small", "model": "m1", "n": 4, "valid": 4, "complete": 4,
                 "playable": 3, "steps_mean": 10.0, "errors_mean": 1.0}]
        res = scoring.compare_cells(base, cand)
        self.assertEqual(res[0], {
            "brief": "campaign-small", "model": "m1",
            "base": {"n": 2, "playable": 1, "complete": 2, "valid": 2},
            "cand": {"n": 4, "playable": 3, "complete": 4, "valid": 4},
            "d_steps": 0.0, "d_errors": 0.0, "verdict": "better",
        })

        cand_worse = [{"brief": "campaign-small", "model": "m1", "n": 2, "valid": 2, "complete": 1,
                       "playable": 1, "steps_mean": 10.0, "errors_mean": 1.0}]
        self.assertEqual(scoring.compare_cells(base, cand_worse)[0]["verdict"], "worse")

        # control: rates are compared as fractions, not raw counts — cand has the
        # same playable count (1) but over n 4, so its 1/4 is below base's 1/2.
        base_den = [{"brief": "campaign-small", "model": "m1", "n": 2, "valid": 2, "complete": 2,
                     "playable": 1, "steps_mean": 10.0, "errors_mean": 1.0}]
        cand_den = [{"brief": "campaign-small", "model": "m1", "n": 4, "valid": 4, "complete": 4,
                     "playable": 1, "steps_mean": 10.0, "errors_mean": 1.0}]
        self.assertEqual(scoring.compare_cells(base_den, cand_den)[0]["verdict"], "worse")

        cand_same = [{"brief": "campaign-small", "model": "m1", "n": 2, "valid": 2, "complete": 2,
                      "playable": 1, "steps_mean": 99.0, "errors_mean": 1.0}]
        res_same = scoring.compare_cells(base, cand_same)
        self.assertEqual(res_same[0]["verdict"], "same")
        self.assertEqual(res_same[0]["d_steps"], 89.0)

        one_sided = scoring.compare_cells(base, [{"brief": "campaign-small", "model": "m2", "n": 2,
                                                  "valid": 2, "complete": 2, "playable": 1,
                                                  "steps_mean": 10.0, "errors_mean": 1.0}])
        self.assertEqual(one_sided, [])

        base_n0 = [{"brief": "campaign-small", "model": "m1", "n": 0, "valid": 0, "complete": 0,
                    "playable": 0, "steps_mean": None, "errors_mean": None}]
        self.assertEqual(scoring.compare_cells(base_n0, cand), [])

    # SC10
    def test_sc10_scrub_text_paths(self):
        self.assertEqual(
            scoring.scrub_text("a C:\\Users\\x\\s\\f and C:/Users/x/s/g and /home/y/z",
                               ["C:\\Users\\x\\s", "/home/y"]),
            "a <path>\\f and <path>/g and <path>/z",
        )
        self.assertEqual(scoring.scrub_text("unchanged", [""]), "unchanged")
        self.assertEqual(scoring.scrub_text("/a/b/c", ["/a", "/a/b"]), "<path>/c")


if __name__ == "__main__":
    unittest.main()

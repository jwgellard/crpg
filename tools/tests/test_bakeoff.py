#!/usr/bin/env python3
"""Tests for tools/agents/bakeoff/bakeoff.py (A0-7a)."""

from __future__ import annotations

import importlib.util
import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]
BAKEOFF = REPO / "tools" / "agents" / "bakeoff" / "bakeoff.py"
FIXTURES = REPO / "tools" / "agents" / "bakeoff" / "fixtures"
FAKE_OPENCODE = HERE / "fake_opencode.py"

spec = importlib.util.spec_from_file_location("bakeoff_under_test", BAKEOFF)
# bakeoff.py lives in a directory without __init__.py; load via runpy-style exec
# by importing the module file directly.
import importlib.machinery
loader = importlib.machinery.SourceFileLoader("bakeoff_under_test", str(BAKEOFF))
bakeoff = loader.load_module()


def sh(args, cwd=None, env=None):
    merged = dict(os.environ)
    if env:
        merged.update(env)
    return subprocess.run(args, cwd=cwd, capture_output=True, text=True, timeout=120, env=merged)


class BakeoffCase(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="bakeoff ")
        self.addCleanup(self.tmp.cleanup)
        self.scratch = Path(self.tmp.name) / "scratch"
        self.base_env = {
            "CRPG_OPENCODE": str(FAKE_OPENCODE),
            "CRPG_OC_START_GAP": "0",
            "CRPG_OC_LOCK_WAIT": "0.05",
            "FAKE_OC_MODE": "ok",
        }

    def run_cli(self, *args, env=None, timeout=120):
        merged = dict(os.environ)
        merged.update(self.base_env)
        if env:
            merged.update(env)
        return subprocess.run(
            [sys.executable, str(BAKEOFF), *args],
            capture_output=True, text=True, timeout=timeout, env=merged,
        )

    # T1
    def test_setup_builds_templates(self):
        r = self.run_cli("setup", str(self.scratch))
        self.assertEqual(r.returncode, 0, r.stderr)
        for name in ("scout-deps", "implement-fixed-point", "review-planted"):
            self.assertTrue((self.scratch / "tpl" / name).is_dir(), name)
        # no hidden file in template
        self.assertFalse((self.scratch / "tpl" / "implement-fixed-point" / "tests" / "zz_hidden.rs").exists())
        self.assertFalse((self.scratch / "tpl" / "implement-fixed-point" / "hidden.rs").exists())
        self.assertTrue((self.scratch / "ref" / "hidden.rs").is_file())
        # scout files come from 16f58c8
        import subprocess as sp
        proc = sp.run(["git", "show", "16f58c8:tools/lint/deps.py"], cwd=REPO,
                      capture_output=True)
        self.assertEqual(proc.returncode, 0)
        self.assertEqual((self.scratch / "tpl" / "scout-deps" / "tools" / "lint" / "deps.py").read_bytes(),
                         proc.stdout)

    # T2
    def test_scratch_inside_repo_refused(self):
        inside = REPO / "scratch-inside"
        r = self.run_cli("setup", str(inside))
        self.assertEqual(r.returncode, 2)
        self.assertIn("refused", r.stderr + r.stdout)

    # T3
    def test_opencode_json_denies(self):
        self.run_cli("setup", str(self.scratch))
        # run one fixture of each kind
        for fx in ("scout-deps", "implement-fixed-point", "review-planted"):
            self.run_cli("run", str(self.scratch), "--fixtures", fx,
                         "--models", "m", "--out", "bake", env={"FAKE_OC_MODE": "ok"})
        for fx in ("scout-deps", "implement-fixed-point", "review-planted"):
            cfg_path = self.scratch / "bake" / "runs" / fx / "m" / "opencode.json"
            self.assertTrue(cfg_path.is_file(), cfg_path)
            cfg = json.loads(cfg_path.read_text())
            self.assertEqual(cfg["permission"].get("execute"), "deny")
            kind = "implement" if fx == "implement-fixed-point" else ("scout" if fx == "scout-deps" else "review")
            if kind in ("scout", "review"):
                self.assertEqual(cfg["permission"].get("edit"), "deny")
                bash = cfg["permission"].get("bash", {})
                if isinstance(bash, dict):
                    self.assertEqual(bash.get("*>*"), "deny")
                else:
                    self.assertIn("*>*", cfg["permission"])

    # T4
    def test_scout_scorer(self):
        truth = bakeoff.TRUTH1
        L = {"all_text": json.dumps(truth)}
        sc = bakeoff.score_scout(self.scratch / "x", L)
        self.assertTrue(sc["pass"])
        # missing
        L2 = {"all_text": json.dumps(truth[:-1])}
        sc2 = bakeoff.score_scout(self.scratch / "x", L2)
        self.assertEqual(sc2["fn"], sorted(set(truth) - set(truth[:-1])))
        # extra
        L3 = {"all_text": json.dumps(truth + ["crpg-nope"])}
        sc3 = bakeoff.score_scout(self.scratch / "x", L3)
        self.assertEqual(sc3["fp"], ["crpg-nope"])

    # T5
    def test_review_scorer(self):
        fx = bakeoff.load_fixture("review-planted")
        # all five found
        ans = [{"line": 11, "spec": "S1", "summary": "x"},
               {"line": 16, "spec": "S2", "summary": "x"},
               {"line": 21, "spec": "S3", "summary": "overflow"},
               {"line": 22, "spec": "S3", "summary": "order"},
               {"line": 39, "spec": "S5", "summary": "cast"}]
        sc = bakeoff.score_review(self.scratch / "x", {"all_text": json.dumps(ans)}, fixture=fx)
        self.assertEqual(sc["found"], 5)
        # decoy hit
        sc2 = bakeoff.score_review(self.scratch / "x", {"all_text": json.dumps([{"line": 30, "spec": "S4", "summary": "x"}])}, fixture=fx)
        self.assertTrue(sc2["decoy_fp"])
        # wrong keyword
        sc3 = bakeoff.score_review(self.scratch / "x", {"all_text": json.dumps([{"line": 21, "spec": "S3", "summary": "nothing"}])}, fixture=fx)
        self.assertEqual(sc3["found"], 0)
        # no array
        sc4 = bakeoff.score_review(self.scratch / "x", {"all_text": "hello"}, fixture=fx)
        self.assertEqual(sc4.get("note"), "no array")

    # T6
    def test_implement_scorer(self):
        self.run_cli("setup", str(self.scratch))
        # reference solution
        run_dir = self.scratch / "bake" / "runs" / "implement-fixed-point" / "m"
        run_dir.mkdir(parents=True, exist_ok=True)
        shutil.copytree(self.scratch / "tpl" / "implement-fixed-point", run_dir, dirs_exist_ok=True)
        shutil.copyfile(
            FIXTURES / "implement-fixed-point" / "reference.rs.txt",
            run_dir / "src" / "lib.rs",
        )
        sc = bakeoff.score_implement(run_dir, {"all_text": ""}, scratch=self.scratch)
        self.assertTrue(sc["pass"])
        self.assertEqual(sc["hidden"], "21/21")
        # untouched template
        sc2 = bakeoff.score_implement(self.scratch / "tpl" / "implement-fixed-point",
                                     {"all_text": ""}, scratch=self.scratch)
        self.assertFalse(sc2["pass"])

    # T7
    def test_run_repeats_variant(self):
        self.run_cli("setup", str(self.scratch))
        r = self.run_cli("run", str(self.scratch), "--fixtures", "scout-deps",
                         "--models", "m", "--repeat", "3", "--variant", "high",
                         "--out", "bake", env={"FAKE_OC_MODE": "ok"})
        self.assertEqual(r.returncode, 0, r.stderr)
        metas = list((self.scratch / "bake" / "logs").glob("*.meta.json"))
        self.assertEqual(len(metas), 3)
        for mp in metas:
            m = json.loads(mp.read_text())
            self.assertEqual(m["variant"], "high")
        # logs exist
        logs = list((self.scratch / "bake" / "logs").glob("*.jsonl"))
        self.assertEqual(len(logs), 3)

    # T8
    def test_score_json_and_table(self):
        self.run_cli("setup", str(self.scratch))
        self.run_cli("run", str(self.scratch), "--fixtures", "scout-deps,review-planted",
                     "--models", "ma,mb", "--out", "bake", env={"FAKE_OC_MODE": "ok"})
        r = self.run_cli("score", str(self.scratch), "--out", "bake", "--json")
        self.assertEqual(r.returncode, 0, r.stderr)
        data = json.loads(r.stdout)
        self.assertGreaterEqual(len(data), 4)
        # table via non-json
        r2 = self.run_cli("score", str(self.scratch), "--out", "bake")
        self.assertEqual(r2.returncode, 0, r2.stderr)
        self.assertIn("| Model |", r2.stdout)
        self.assertIn("ma", r2.stdout)

    # T9
    def test_new_fixture_commit_source(self):
        fx_dir = FIXTURES / "throwaway-fixture"
        fx_dir.mkdir(parents=True, exist_ok=True)
        try:
            (fx_dir / "fixture.json").write_text(json.dumps({
                "kind": "scout",
                "prompt": "p",
                "timeout_s": 10,
                "edit": False,
                "files": [{"commit": "16f58c8", "paths": ["AGENTS.md"], "dest": "."}],
                "hidden": [],
                "score": {"truth": ["x"]},
            }))
            r = self.run_cli("setup", str(self.scratch))
            self.assertEqual(r.returncode, 0, r.stderr)
            self.assertTrue((self.scratch / "tpl" / "throwaway-fixture" / "AGENTS.md").is_file())
        finally:
            shutil.rmtree(fx_dir, ignore_errors=True)


if __name__ == "__main__":
    unittest.main()
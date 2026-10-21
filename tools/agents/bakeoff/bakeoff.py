#!/usr/bin/env python3
"""Bake-off harness: fixtures are data, one directory each.

Usage:
    python tools/agents/bakeoff/bakeoff.py setup <scratch>
    python tools/agents/bakeoff/bakeoff.py run <scratch> --fixtures <a,b|all> --models <m1,m2|@file> [--variant V] [--repeat N] [--par N] [--out NAME]
    python tools/agents/bakeoff/bakeoff.py score <scratch> [--out NAME] [--json]
    python tools/agents/bakeoff/bakeoff.py all <scratch>
"""
from __future__ import annotations

import argparse
import concurrent.futures as cf
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import threading
import time
from pathlib import Path

REPO = Path(__file__).resolve().parents[3]
HERE = Path(__file__).resolve().parent
FIXTURES_DIR = HERE / "fixtures"
BAKEOFF_DIR = HERE

# Reuse oc.py helpers rather than copying them.
_OC_DIR = REPO / "tools" / "agents"
if str(_OC_DIR) not in sys.path:
    sys.path.insert(0, str(_OC_DIR))
import oc  # noqa: E402

TRUTH1 = ["crpg-ai", "crpg-cli", "crpg-godot", "crpg-net", "crpg-persist",
          "crpg-script", "crpg-server", "crpg-testkit"]

BUGS_TEMPLATE = {
    "A": ((10, 12), None),
    "B": ((15, 17), None),
    "C": ((20, 23), r"overflow|saturat|panic|exceed|u32::MAX"),
    "D": ((19, 25), r"order|sort|HashMap|determin|iterat|random"),
    "E": ((38, 41), r"cast|as i32|negative|wrap|overflow|lower|reduc|truncat|exceed"),
}


def sha256_file(p):
    try:
        return hashlib.sha256(Path(p).read_bytes()).hexdigest()
    except OSError:
        return None


def is_inside_repo(scratch):
    scratch = Path(scratch).resolve()
    try:
        scratch.relative_to(REPO)
        return True
    except ValueError:
        return False


def parse_log(path):
    texts, steps, tools, err = [], 0, 0, ""
    tok = {"input": 0, "output": 0, "reasoning": 0, "cache_read": 0}
    cost = 0.0
    p = Path(path)
    if not p.is_file():
        return {"text": "", "all_text": "", "steps": 0, "tools": 0, "tok": tok,
                "cost": 0.0, "err": ""}
    for line in p.read_text(encoding="utf-8", errors="replace").splitlines():
        try:
            e = json.loads(line)
        except Exception:
            if "rror" in line:
                err = err or line.strip()[:160]
            continue
        t = e.get("type")
        part = e.get("part", {})
        if t == "text":
            texts.append(part.get("text", ""))
        elif t == "step_finish":
            steps += 1
            tk = part.get("tokens", {}) or {}
            tok["input"] += tk.get("input", 0) or 0
            tok["output"] += tk.get("output", 0) or 0
            tok["reasoning"] += tk.get("reasoning", 0) or 0
            tok["cache_read"] += (tk.get("cache") or {}).get("read", 0) or 0
            cost += part.get("cost", 0) or 0
        elif t == "tool_use":
            tools += 1
        elif t == "error":
            err = err or json.dumps(e.get("error"))[:200]
    return {"text": texts[-1] if texts else "", "all_text": "\n".join(texts),
            "steps": steps, "tools": tools, "tok": tok, "cost": cost, "err": err}


def last_json_array(text):
    if not isinstance(text, str):
        return None
    for m in reversed(list(re.finditer(r"\[[\s\S]*?\](?=[^\]]*$)|\[[\s\S]*\]", text))):
        try:
            return json.loads(m.group(0))
        except Exception:
            continue
    for m in re.finditer(r"\[[^\[\]]*(?:\{[^{}]*\}[^\[\]]*)*\]", text):
        try:
            return json.loads(m.group(0))
        except Exception:
            pass
    return None


def load_fixture(name):
    d = FIXTURES_DIR / name
    if not d.is_dir() or not (d / "fixture.json").is_file():
        return None
    data = json.loads((d / "fixture.json").read_text(encoding="utf-8"))
    data["_dir"] = d
    data["_name"] = name
    return data


def list_fixtures():
    out = []
    for d in sorted(FIXTURES_DIR.iterdir()):
        if d.is_dir() and (d / "fixture.json").is_file():
            out.append(d.name)
    return out


def hidden_src_name(h):
    if isinstance(h, str):
        return h
    return h.get("src") or h.get("from") or h.get("path") or h.get("file")


def hidden_ref_name(h):
    if isinstance(h, dict) and h.get("ref"):
        return h.get("ref")
    src = hidden_src_name(h)
    name = src
    if name.endswith(".txt"):
        name = name[:-4]
    return name


def hidden_dest_name(h):
    if isinstance(h, dict):
        return h.get("dest") or h.get("to") or h.get("target") or hidden_ref_name(h)
    return h


def rename_txt_tree(root):
    for p in sorted(root.rglob("*.txt"), key=lambda x: len(str(x)), reverse=True):
        if p.is_file():
            p.rename(p.with_name(p.name[:-4] + ".md"))


def copy_tree_filtered(src, dest, hidden_names):
    dest.mkdir(parents=True, exist_ok=True)
    for item in src.iterdir():
        if item.name == "fixture.json":
            continue
        if item.name in hidden_names:
            continue
        if item.name.endswith(".rs.txt"):
            continue
        target = dest / item.name
        if item.is_dir():
            shutil.copytree(item, target, dirs_exist_ok=True,
                            ignore=shutil.ignore_patterns("fixture.json", "*.rs.txt", *hidden_names))
        else:
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(item, target)


def apply_file_spec(spec, fx_dir, template, repo, hidden_names):
    if "tree" in spec:
        tree = spec["tree"]
        src_dir = fx_dir if tree in (".", "", None) else fx_dir / tree
        copy_tree_filtered(src_dir, template, hidden_names)
    elif "repo" in spec:
        src = repo / spec["repo"]
        dest_raw = spec.get("dest", ".")
        dest = template if dest_raw in (".", "", None) else template / dest_raw
        if src.is_dir():
            shutil.copytree(src, dest, dirs_exist_ok=True)
        else:
            dest.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(src, dest)
    elif "commit" in spec:
        sha = spec["commit"]
        dest_raw = spec.get("dest", ".")
        dest_dir = template if dest_raw in (".", "", None) else template / dest_raw
        dest_dir.mkdir(parents=True, exist_ok=True)
        for path in spec.get("paths", []):
            proc = subprocess.run(["git", "show", f"{sha}:{path}"],
                                  cwd=repo, capture_output=True)
            if proc.returncode != 0:
                raise RuntimeError(f"git show {sha}:{path} failed: {proc.stderr.decode(errors='replace')}")
            if dest_raw in (".", "", None):
                target = template / path
            else:
                target = dest_dir / Path(path).name
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(proc.stdout)
    elif "rename" in spec:
        for old, new in (spec["rename"] or {}).items():
            src_path = template / old
            dst_path = template / new
            dst_path.parent.mkdir(parents=True, exist_ok=True)
            if src_path.exists():
                if src_path.is_dir():
                    if dst_path.exists():
                        shutil.rmtree(dst_path)
                    shutil.move(str(src_path), str(dst_path))
                else:
                    shutil.move(str(src_path), str(dst_path))


def cmd_setup(scratch, args=None):
    scratch = Path(scratch).resolve()
    if is_inside_repo(scratch):
        print("setup: scratch directory is inside the repository; refused", file=sys.stderr)
        return 2
    tpl = scratch / "tpl"
    ref = scratch / "ref"
    if tpl.exists():
        shutil.rmtree(tpl)
    if ref.exists():
        shutil.rmtree(ref)
    tpl.mkdir(parents=True, exist_ok=True)
    ref.mkdir(parents=True, exist_ok=True)
    for fx_dir in sorted(FIXTURES_DIR.iterdir()):
        if not fx_dir.is_dir() or not (fx_dir / "fixture.json").is_file():
            continue
        fx = json.loads((fx_dir / "fixture.json").read_text(encoding="utf-8"))
        name = fx_dir.name
        template = tpl / name
        template.mkdir(parents=True, exist_ok=True)
        hidden_names = {hidden_src_name(h) for h in fx.get("hidden", [])}
        for spec in fx.get("files", []):
            apply_file_spec(spec, fx_dir, template, REPO, hidden_names)
        rename_txt_tree(template)
        for h in fx.get("hidden", []):
            src_name = hidden_src_name(h)
            src_path = fx_dir / src_name
            ref_name = hidden_ref_name(h)
            ref_path = ref / ref_name
            ref_path.parent.mkdir(parents=True, exist_ok=True)
            if src_path.is_file():
                shutil.copy2(src_path, ref_path)
    print(f"fixtures ready in {tpl}")
    return 0


def write_opencode_json(run_dir, edit):
    cfg = {
        "$schema": "https://opencode.ai/config.json",
        "autoupdate": False,
        "share": "disabled",
        "permission": {
            "execute": "deny",
            "webfetch": "deny",
            "websearch": "deny",
            "external_directory": "deny",
            "edit": "allow" if edit else "deny",
            "bash": "allow" if edit else {"*": "allow", "*>*": "deny"},
        },
    }
    if not edit:
        cfg["permission"]["*>*"] = "deny"
    (run_dir / "opencode.json").write_text(json.dumps(cfg, indent=2), encoding="utf-8")


def infer_scratch(path):
    p = Path(path).resolve()
    for cand in [p, *p.parents]:
        if (cand / "ref").is_dir() or (cand / "tpl").is_dir():
            return cand
    return p.parents[0] if p.parents else p


def infer_fixture_name(path, scratch=None):
    p = Path(path).resolve()
    # direct fixture dir under repo
    if (p / "fixture.json").is_file():
        return p.name
    # run dir: .../runs/<fixture>/<model>/...
    parts = p.parts
    if "runs" in parts:
        i = parts.index("runs")
        if i + 1 < len(parts):
            return parts[i + 1]
    # template dir: .../tpl/<fixture>
    if "tpl" in parts:
        i = parts.index("tpl")
        if i + 1 < len(parts):
            return parts[i + 1]
    return None


def load_fixture_by_name(name, scratch=None):
    if name is None:
        return None
    fx = load_fixture(name)
    return fx


def _as_log(log):
    if isinstance(log, (str, Path)):
        return parse_log(log)
    if isinstance(log, dict):
        if "all_text" in log or "text" in log:
            return log
        # maybe a meta dict
        if "log" in log:
            return parse_log(log["log"])
        return log
    return {"text": "", "all_text": "", "steps": 0, "tools": 0,
            "tok": {"input": 0, "output": 0, "reasoning": 0, "cache_read": 0},
            "cost": 0.0, "err": ""}


def template_dir_for(run_dir, scratch=None):
    if scratch is None:
        scratch = infer_scratch(run_dir)
    scratch = Path(scratch)
    name = infer_fixture_name(run_dir, scratch)
    # if run_dir itself is template
    if (Path(run_dir) / "fixture.json").is_file():
        return Path(run_dir)
    # try scratch/tpl/<name>
    if name:
        cand = scratch / "tpl" / name
        if cand.is_dir():
            return cand
    # walk up
    p = Path(run_dir)
    for cand in [p, *p.parents]:
        if (cand / "Cargo.toml").is_file() or (cand / "src" / "combat.rs").is_file():
            # could be template itself
            if (cand / "src").is_dir():
                return cand
    return None


def score_scout(run_dir, log, scratch=None, fixture=None, **kw):
    run_dir = Path(run_dir)
    L = _as_log(log)
    if scratch is None:
        scratch = infer_scratch(run_dir)
    scratch = Path(scratch)
    if fixture is None:
        name = infer_fixture_name(run_dir, scratch)
        if name is None and "scout" in str(run_dir).lower():
            name = "scout-deps"
        fixture = load_fixture_by_name(name)
    truth = None
    if fixture is not None:
        truth = (fixture.get("score") or {}).get("truth")
    if truth is None:
        truth = TRUTH1
    tpl = template_dir_for(run_dir, scratch)
    deps_path = run_dir / "tools" / "lint" / "deps.py"
    tpl_deps = None
    if tpl is not None:
        tpl_deps = tpl / "tools" / "lint" / "deps.py"
    unchanged = sha256_file(deps_path) == sha256_file(tpl_deps) if tpl_deps else (deps_path.is_file())
    arr = last_json_array(L.get("all_text", "") or L.get("text", ""))
    if not isinstance(arr, list):
        return {"pass": False, "note": "no array", "unchanged": unchanged}
    got = set(map(str, arr))
    truth_set = set(map(str, truth))
    return {"pass": got == truth_set, "tp": len(got & truth_set),
            "fp": sorted(got - truth_set), "fn": sorted(truth_set - got),
            "unchanged": unchanged}


def score_review(run_dir, log, scratch=None, fixture=None, **kw):
    run_dir = Path(run_dir)
    L = _as_log(log)
    if scratch is None:
        scratch = infer_scratch(run_dir)
    scratch = Path(scratch)
    if fixture is None:
        name = infer_fixture_name(run_dir, scratch)
        if name is None and "review" in str(run_dir).lower():
            name = "review-planted"
        fixture = load_fixture_by_name(name)
    tpl = template_dir_for(run_dir, scratch)
    combat_path = run_dir / "src" / "combat.rs"
    tpl_combat = tpl / "src" / "combat.rs" if tpl is not None else None
    unchanged = sha256_file(combat_path) == sha256_file(tpl_combat) if tpl_combat else True
    arr = last_json_array(L.get("all_text", "") or L.get("text", ""))
    if not isinstance(arr, list):
        return {"found": 0, "fp": None, "note": "no array", "unchanged": unchanged}
    # defects
    defects = []
    decoy_lines = []
    decoy_specs = []
    if fixture is not None:
        sc = fixture.get("score") or {}
        defects = sc.get("defects") or []
        decoy_lines = sc.get("decoy_lines") or []
        decoy_specs = sc.get("decoy_specs") or []
    else:
        # Fallback to the original planted defects for direct scorer tests.
        defects = [
            {"id": "A", "lines": [10, 12], "keywords": None},
            {"id": "B", "lines": [15, 17], "keywords": None},
            {"id": "C", "lines": [20, 23], "keywords": r"overflow|saturat|panic|exceed|u32::MAX"},
            {"id": "D", "lines": [19, 25], "keywords": r"order|sort|HashMap|determin|iterat|random"},
            {"id": "E", "lines": [38, 41], "keywords": r"cast|as i32|negative|wrap|overflow|lower|reduc|truncat|exceed"},
        ]
        decoy_lines = [[29, 34]]
        decoy_specs = ["S4"]
    found = set()
    fps = 0
    decoy = False
    for it in arr:
        if not isinstance(it, dict):
            fps += 1
            continue
        try:
            line = int(it.get("line", -1))
        except Exception:
            line = -1
        summ = str(it.get("summary", "")) + " " + str(it.get("spec", ""))
        hit = None
        for d in defects:
            did = d.get("id")
            if did in found:
                continue
            lo, hi = d.get("lines", [line, line])
            if not (isinstance(lo, int) and isinstance(hi, int)):
                lo, hi = line, line
            kwpat = d.get("keywords", None)
            if lo <= line <= hi and (kwpat is None or re.search(kwpat, summ, re.I)):
                hit = did
                break
        if hit:
            found.add(hit)
        else:
            fps += 1
            # decoy detection
            is_decoy = False
            for dl in decoy_lines:
                if isinstance(dl, (list, tuple)) and len(dl) == 2:
                    if dl[0] <= line <= dl[1]:
                        is_decoy = True
                elif isinstance(dl, int):
                    if line == dl:
                        is_decoy = True
            if any(spec in summ for spec in decoy_specs):
                is_decoy = True
            # old hardcoded fallback if no decoy config
            if not decoy_lines and not decoy_specs:
                if 29 <= line <= 34 or "S4" in summ:
                    is_decoy = True
            if is_decoy:
                decoy = True
    return {"found": len(found), "which": "".join(sorted(found)), "fp": fps,
            "decoy_fp": decoy, "unchanged": unchanged}


def score_implement(run_dir, log, scratch=None, fixture=None, **kw):
    run_dir = Path(run_dir)
    L = _as_log(log)
    if scratch is None:
        scratch = infer_scratch(run_dir)
    scratch = Path(scratch)
    if fixture is None:
        name = infer_fixture_name(run_dir, scratch)
        fixture = load_fixture_by_name(name)
    tpl = template_dir_for(run_dir, scratch)
    r = {}
    src_path = run_dir / "src" / "lib.rs"
    src = src_path.read_text(encoding="utf-8", errors="replace") if src_path.is_file() else ""
    r["todo_left"] = "todo!()" in src
    code_files = list(run_dir.glob("src/**/*.rs")) + list(run_dir.glob("tests/*.rs"))
    code = "\\n".join(p.read_text(encoding="utf-8", errors="replace") for p in code_files if p.is_file())
    r["float"] = bool(re.search(r"\\bf(32|64)\\b", code))
    r["unsafe"] = "unsafe" in src.replace("forbid(unsafe_code)", "")
    # rules from fixture
    rules = {}
    if fixture is not None:
        rules = (fixture.get("score") or {}).get("rules") or {}
    no_float_rule = rules.get("no_float", True) if "no_float" in rules else True
    no_unsafe_rule = rules.get("no_unsafe", True) if "no_unsafe" in rules else True
    unchanged_files = rules.get("files_unchanged", [])
    if not unchanged_files:
        unchanged_files = ["Cargo.toml", "tests/visible.rs"]
    unchanged_map = {}
    for fn in unchanged_files:
        a = sha256_file(run_dir / fn)
        b = sha256_file(tpl / fn) if tpl is not None else None
        unchanged_map[fn] = (a == b)
    r["files_unchanged"] = unchanged_map
    r["files_unchanged_ok"] = all(unchanged_map.values()) if unchanged_map else True
    r["cargo_toml_same"] = unchanged_map.get("Cargo.toml", True)
    r["visible_same"] = unchanged_map.get("tests/visible.rs", True)

    def run_cmd(cmd):
        try:
            p = subprocess.run(cmd, cwd=run_dir, capture_output=True, text=True,
                               timeout=600, stdin=subprocess.DEVNULL, encoding="utf-8", errors="replace")
            return p.returncode, p.stdout + p.stderr
        except subprocess.TimeoutExpired:
            return "TIMEOUT", ""

    # gates from fixture
    gates_spec = []
    if fixture is not None:
        gates_spec = (fixture.get("score") or {}).get("gates") or []
    gate_results = []
    gates_ok = True
    for g in gates_spec:
        cmd = list(g.get("cmd") or g.get("command") or [])
        expected = g.get("expected", 0)
        if expected is None:
            expected = 0
        rc, out = run_cmd(cmd)
        passed = (rc == expected)
        if not passed:
            gates_ok = False
        name = g.get("name")
        gr = {"name": name, "cmd": cmd, "expected": expected, "rc": rc, "passed": passed}
        gate_results.append(gr)
        if name:
            r[name] = passed
    r["gates"] = gate_results
    r["gates_ok"] = gates_ok if gates_spec else True
    for key in ("fmt", "clippy", "visible"):
        if key not in r:
            matches = [gr for gr in gate_results if gr.get("name") == key]
            if matches:
                r[key] = matches[0]["passed"]

    # hidden
    if fixture is None:
        name = infer_fixture_name(run_dir, scratch)
        if name is None:
            if "implement" in str(run_dir).lower():
                name = "implement-fixed-point"
        fixture = load_fixture_by_name(name)
    hidden_entries = []
    if fixture is not None:
        hidden_entries = fixture.get("hidden") or []
    copied = []
    for h in hidden_entries:
        ref_name = hidden_ref_name(h)
        dest_name = hidden_dest_name(h)
        src = scratch / "ref" / ref_name
        dst = run_dir / dest_name
        if src.is_file():
            dst.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(src, dst)
            copied.append((dst, dest_name))
    hidden_test = "zz_hidden"
    hidden_total = 21
    if fixture is not None:
        sc = fixture.get("score") or {}
        hidden_test = sc.get("hidden_test", hidden_test)
        hidden_total = sc.get("hidden_total", hidden_total)
    hidden_ran = bool(hidden_entries)
    if hidden_ran:
        rc, out = run_cmd(["cargo", "test", "-q", "--test", hidden_test])
        m = re.search(r"(\d+) passed; (\d+) failed", out)
        r["hidden"] = f"{m.group(1)}/{hidden_total}" if m else ("build-fail" if rc != 0 else "?")
        r["hidden_n"] = int(m.group(1)) if m else 0
    else:
        r["hidden"] = f"0/{hidden_total}" if hidden_total else "0/0"
        r["hidden_n"] = 0
    for dst, _ in copied:
        try:
            dst.unlink()
        except OSError:
            pass
    # rules_ok
    r["rules_ok"] = ((not r["float"]) if no_float_rule else True) and                     ((not r["unsafe"]) if no_unsafe_rule else True) and                     r["files_unchanged_ok"]
    r["pass"] = r["rules_ok"] and r["gates_ok"] and not r["todo_left"]
    return r


# aliases
score_t1 = score_scout
score_t2 = score_implement
score_t3 = score_review
score_scout_alias = score_scout
score_implement_alias = score_implement
score_review_alias = score_review


def score_by_kind(fixture, run_dir, log, scratch):
    kind = fixture.get("kind") if fixture else None
    if kind == "scout":
        return score_scout(run_dir, log, scratch=scratch, fixture=fixture)
    if kind == "implement":
        return score_implement(run_dir, log, scratch=scratch, fixture=fixture)
    if kind == "review":
        return score_review(run_dir, log, scratch=scratch, fixture=fixture)
    # fallback by name
    name = fixture.get("_name") if fixture else None
    if name and "scout" in name:
        return score_scout(run_dir, log, scratch=scratch, fixture=fixture)
    if name and "implement" in name:
        return score_implement(run_dir, log, scratch=scratch, fixture=fixture)
    if name and "review" in name:
        return score_review(run_dir, log, scratch=scratch, fixture=fixture)
    return {}


def cmd_run(scratch, args):
    scratch = Path(scratch).resolve()
    if is_inside_repo(scratch):
        print("run: scratch directory is inside the repository; refused", file=sys.stderr)
        return 2
    # fixtures
    if args.fixtures == "all":
        fixture_names = list_fixtures()
    else:
        fixture_names = [x.strip() for x in args.fixtures.split(",") if x.strip()]
    fixtures = {}
    for name in fixture_names:
        fx = load_fixture(name)
        if fx is None:
            print(f"run: unknown fixture {name}", file=sys.stderr)
            return 2
        fixtures[name] = fx
    # models
    if args.models.startswith("@"):
        p = Path(args.models[1:])
        if not p.is_absolute():
            p = Path.cwd() / p
        models = [ln.strip() for ln in p.read_text(encoding="utf-8").splitlines()
                  if ln.strip() and not ln.strip().startswith("#")]
    else:
        models = [x.strip() for x in args.models.split(",") if x.strip()]
    out = args.out or "bake"
    repeat = args.repeat or 1
    par = args.par or 6
    variant = args.variant
    runs_root = scratch / out / "runs"
    logs_root = scratch / out / "logs"
    runs_root.mkdir(parents=True, exist_ok=True)
    logs_root.mkdir(parents=True, exist_ok=True)

    # ensure templates exist
    for name in fixture_names:
        if not (scratch / "tpl" / name).is_dir():
            print(f"run: template missing for {name}; run setup first", file=sys.stderr)
            return 2

    def run_one(fx_name, model, rep):
        fx = fixtures[fx_name]
        model_safe = model.replace("/", "__")
        run_dir = runs_root / fx_name / model_safe
        if repeat > 1:
            run_dir = run_dir / f"r{rep}"
        if run_dir.exists():
            shutil.rmtree(run_dir)
        shutil.copytree(scratch / "tpl" / fx_name, run_dir)
        write_opencode_json(run_dir, bool(fx.get("edit", False)))
        log_name = f"{fx_name}__{model_safe}.jsonl"
        if repeat > 1:
            log_name = f"{fx_name}__{model_safe}__r{rep}.jsonl"
        log_path = logs_root / log_name
        if log_path.exists():
            log_path.unlink()
        meta_path = Path(str(log_path) + ".meta.json")
        timeout = fx.get("timeout_s", 300)
        prefix = oc.opencode_prefix()
        major = oc.opencode_major(prefix)
        cmd = list(prefix)
        if major >= 2:
            cmd.append("run")
            cmd.append("--standalone")
        else:
            cmd.append("run")
        cmd += oc.model_flag(model, variant, major)
        if major < 2:
            cmd += ["--dir", str(run_dir)]
        cmd += ["--format", "json", fx["prompt"]]
        kind = fx.get("kind", "")
        agent = "scout" if kind == "scout" else ("review" if kind == "review" else "implement")
        t0 = time.time()
        for attempt in range(3):
            oc.wait_start_gap(REPO)
            if log_path.exists():
                log_path.unlink()
            cause, rc = oc.run_worker(cmd, run_dir, log_path, timeout, 0, agent, model)
            dt = time.time() - t0
            txt = log_path.read_text(encoding="utf-8", errors="replace") if log_path.is_file() else ""
            if "database is locked" in txt and attempt < 2:
                time.sleep(5)
                continue
            break
        rc_out = "TIMEOUT" if cause == "timeout" else rc
        meta = {
            "fixture": fx_name,
            "task": fx_name,
            "model": model,
            "repeat": rep,
            "variant": variant,
            "rc": rc_out,
            "secs": round(dt, 1),
            "dir": str(run_dir),
            "log": str(log_path),
        }
        meta_path.write_text(json.dumps(meta, indent=1), encoding="utf-8")
        print(f"{fx_name} {model:46} rc={rc_out} {dt:6.1f}s", flush=True)
        failed = False
        if cause == "timeout":
            failed = True
        elif isinstance(rc, int) and rc != 0:
            failed = True
        return 1 if failed else 0

    jobs = [(fn, m, rep) for fn in fixture_names for m in models for rep in range(repeat)]
    rc_total = 0
    with cf.ThreadPoolExecutor(par) as ex:
        futs = [ex.submit(run_one, fn, m, rep) for (fn, m, rep) in jobs]
        for fut in futs:
            rc_total = rc_total or fut.result()
    return rc_total


def cell_text(fx, row):
    sc = row.get("score", {})
    kind = fx.get("kind")
    if kind == "scout":
        if sc.get("note"):
            s = sc["note"]
        else:
            s = "exact" if sc.get("pass") else f"wrong ({len(sc.get('fn') or [])} missed, {len(sc.get('fp') or [])} extra)"
        if row.get("rc") not in (0, "0"):
            s += f" rc={row.get('rc')}"
        return s
    if kind == "implement":
        hidden = sc.get("hidden", "?")
        gates = "ok" if (sc.get("fmt") and sc.get("clippy") and sc.get("visible")) else "/".join(
            k for k in ("fmt", "clippy", "visible") if not sc.get(k)) + " failed"
        rules = "ok" if sc.get("rules_ok") else ",".join(
            k for k, bad in (("float", sc.get("float")), ("unsafe", sc.get("unsafe")),
                             ("Cargo.toml", not sc.get("cargo_toml_same", True)),
                             ("edited tests", not sc.get("visible_same", True))) if bad)
        s = f"{hidden} · {gates} · {rules}"
        if sc.get("todo_left"):
            s += " · todo left"
        if row.get("rc") not in (0, "0"):
            s += f" · rc={row.get('rc')}"
        return s
    if kind == "review":
        if sc.get("note"):
            s = sc["note"]
        else:
            s = f"{sc.get('found', 0)}/5 [{sc.get('which', '')}] / {sc.get('fp', 0)}"
            if sc.get("decoy_fp"):
                s += " · decoy"
        if not sc.get("unchanged", True):
            s += " · EDITED FILE"
        if row.get("rc") not in (0, "0"):
            s += f" rc={row.get('rc')}"
        return s
    return str(sc)


def is_pass(fx, sc):
    kind = fx.get("kind")
    if kind == "scout":
        return bool(sc.get("pass"))
    if kind == "implement":
        return bool(sc.get("pass"))
    if kind == "review":
        return sc.get("found", 0) == 5 and (sc.get("fp") or 0) == 0
    return False


def cmd_score(scratch, args):
    scratch = Path(scratch).resolve()
    out = args.out or "bake"
    results_dir = scratch / out
    logs_dir = results_dir / "logs"
    meta_files = sorted(logs_dir.glob("*.meta.json")) if logs_dir.is_dir() else []
    rows = []
    for meta_path in meta_files:
        meta = json.loads(meta_path.read_text(encoding="utf-8"))
        fx_name = meta.get("fixture") or meta.get("task")
        fx = load_fixture(fx_name)
        if fx is None:
            print(f"score: unknown fixture {fx_name}", file=sys.stderr)
            return 2
        run_dir = Path(meta["dir"])
        log_path = Path(meta["log"])
        L = parse_log(log_path)
        sc = score_by_kind(fx, run_dir, L, scratch)
        row = dict(meta)
        row["steps"] = L["steps"]
        row["tools"] = L["tools"]
        row["tok"] = L["tok"]
        row["cost"] = L["cost"]
        row["err"] = L["err"]
        row["score"] = sc
        row["answer"] = (L["text"] or "")[-300:]
        rows.append(row)
    scores_path = results_dir / "scores.json"
    results_dir.mkdir(parents=True, exist_ok=True)
    scores_path.write_text(json.dumps(rows, indent=1), encoding="utf-8")
    if getattr(args, "json", False):
        print(json.dumps(rows, indent=1))
        return 0
    # print verdict-first summary + table
    print(f"scored {len(rows)} runs -> {scores_path}")
    # table
    fixtures = sorted({(r.get("fixture") or r.get("task")) for r in rows})
    models = sorted({r["model"] for r in rows})
    # aggregate repeat counts
    print("| Model | " + " | ".join(fixtures) + " |")
    print("|" + "---|" * (len(fixtures) + 1))
    for m in models:
        cells = []
        for fxn in fixtures:
            fx = load_fixture(fxn)
            fx_rows = [r for r in rows if r["model"] == m and (r.get("fixture") or r.get("task")) == fxn]
            if not fx_rows:
                cells.append("—")
                continue
            if len(fx_rows) == 1:
                cells.append(cell_text(fx, fx_rows[0]))
            else:
                passes = sum(1 for r in fx_rows if is_pass(fx, r.get("score", {})))
                cells.append(f"{passes}/{len(fx_rows)}")
        print(f"| `{m}` | " + " | ".join(cells) + " |")
    return 0


def parse_models_file():
    models = []
    p = BAKEOFF_DIR / "models.txt"
    for ln in p.read_text(encoding="utf-8").splitlines():
        ln = ln.strip()
        if not ln or ln.startswith("#"):
            continue
        models.append(ln)
    return models


def cmd_all(scratch, args):
    scratch = Path(scratch).resolve()
    if is_inside_repo(scratch):
        print("all: scratch directory is inside the repository; refused", file=sys.stderr)
        return 2
    rc = cmd_setup(scratch, None)
    if rc != 0:
        return rc
    models = parse_models_file()
    run_args = argparse.Namespace(
        fixtures="all",
        models=",".join(models),
        variant=getattr(args, "variant", None),
        repeat=getattr(args, "repeat", 1) or 1,
        par=getattr(args, "par", 6) or 6,
        out=getattr(args, "out", None) or "bake",
    )
    rc = cmd_run(scratch, run_args)
    score_args = argparse.Namespace(out=run_args.out, json=False)
    rc2 = cmd_score(scratch, score_args)
    return 0 if (rc == 0 and rc2 == 0) else (rc or rc2)


def main(argv=None):
    parser = argparse.ArgumentParser(prog="bakeoff.py")
    sub = parser.add_subparsers(dest="cmd", required=True)
    p_setup = sub.add_parser("setup")
    p_setup.add_argument("scratch")
    p_run = sub.add_parser("run")
    p_run.add_argument("scratch")
    p_run.add_argument("--fixtures", required=True)
    p_run.add_argument("--models", required=True)
    p_run.add_argument("--variant", default=None)
    p_run.add_argument("--repeat", type=int, default=1)
    p_run.add_argument("--par", type=int, default=6)
    p_run.add_argument("--out", default="bake")
    p_score = sub.add_parser("score")
    p_score.add_argument("scratch")
    p_score.add_argument("--out", default="bake")
    p_score.add_argument("--json", action="store_true")
    p_all = sub.add_parser("all")
    p_all.add_argument("scratch")
    p_all.add_argument("--variant", default=None)
    p_all.add_argument("--repeat", type=int, default=1)
    p_all.add_argument("--par", type=int, default=6)
    p_all.add_argument("--out", default="bake")
    args = parser.parse_args(argv)
    if args.cmd == "setup":
        return cmd_setup(args.scratch, args)
    if args.cmd == "run":
        return cmd_run(args.scratch, args)
    if args.cmd == "score":
        return cmd_score(args.scratch, args)
    if args.cmd == "all":
        return cmd_all(args.scratch, args)
    return 2


if __name__ == "__main__":
    sys.exit(main())
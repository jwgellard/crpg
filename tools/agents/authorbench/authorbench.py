#!/usr/bin/env python3
"""The command-line half of the authoring benchmark (T100b).

Runs the benchmark: `setup` builds the five brief templates, `run` starts the
OpenCode workers (through `oc.py`), `score` judges the runs (through
`authorbench_scoring.py`), `compare` reports one design against another, and
`all` chains the three. Standard library only; Python 3.11+. No network, no
real `crpgc`, no model call.
"""

from __future__ import annotations

import argparse
import concurrent.futures as cf
import hashlib
import json
import os
import re
import shutil
import sys
import time
from pathlib import Path

REPO = Path(__file__).resolve().parents[3]
HERE = Path(__file__).resolve().parent

# Reuse oc.py helpers and the scoring module rather than copying them.
for _head in (str(HERE), str(REPO / "tools" / "agents")):
    if _head not in sys.path:
        sys.path.insert(0, _head)
import oc  # noqa: E402
import authorbench_scoring as scoring  # noqa: E402

BRIEF_ORDER = ["campaign-extend", "campaign-small", "campaign-medium",
               "campaign-large", "ruleset-small"]
PROMPT = "Read brief.md in this directory and do exactly what it says."
DEFAULT_DESIGN = "baseline"

_SAFE_RE = re.compile(r"[^A-Za-z0-9._-]")


def is_inside_repo(path: Path) -> bool:
    path = Path(path).resolve()
    try:
        path.relative_to(REPO)
        return True
    except ValueError:
        return False


def parse_models(text: str) -> list[str]:
    """`"a,b"` or `"@file"`; raises `oc.Exit(2, "usage: ...")` when empty."""
    if not text:
        raise oc.Exit(2, "usage: --models is required (m1,m2 or @file)")
    if text.startswith("@"):
        path = Path(text[1:])
        if not path.is_absolute():
            path = Path.cwd() / path
        try:
            content = path.read_text(encoding="utf-8")
        except OSError:
            raise oc.Exit(2, f"usage: cannot read models file {text}")
        models = [ln.strip() for ln in content.splitlines()
                  if ln.strip() and not ln.strip().startswith("#")]
    else:
        models = [x.strip() for x in text.split(",") if x.strip()]
    if not models:
        raise oc.Exit(2, f"usage: no model ids in {text}")
    return models


def resolve_crpgc(arg: str | None) -> str | None:
    """Where the `crpgc` binary comes from (B-10); a non-`None` answer is absolute."""
    def existing(path: str) -> str | None:
        if path and Path(path).is_file():
            return str(Path(path).resolve())
        return None

    if arg is not None:
        return existing(arg)
    env = os.environ.get("CRPGC", "")
    if env:
        return existing(env)
    suffix = ".exe" if os.name == "nt" else ""
    for sub in ("release", "debug"):
        candidate = REPO / "target" / sub / f"crpgc{suffix}"
        if candidate.is_file():
            return str(candidate.resolve())
    found = shutil.which("crpgc")
    if found:
        return str(Path(found).resolve())
    return None


def write_opencode_json(run_dir: Path) -> None:
    cfg = {
        "$schema": "https://opencode.ai/config.json",
        "autoupdate": False,
        "share": "disabled",
        "permission": {
            "execute": "deny",
            "webfetch": "deny",
            "websearch": "deny",
            "external_directory": "deny",
            "edit": "allow",
            "bash": "allow",
        },
    }
    # Write bytes, not `write_text`: on Windows `write_text` turns ``\n`` into
    # ``\r\n``, and B-13 requires ``opencode.json`` to be exactly
    # `json.dumps(cfg, indent=2)` (LF only).
    (Path(run_dir) / "opencode.json").write_bytes(json.dumps(cfg, indent=2).encode("utf-8"))


def render_brief(prompt: str, tools_text: str) -> str:
    return prompt.replace(scoring.TOOLS_MARK, tools_text.rstrip("\n"))


def _safe_model(model: str) -> str:
    return _SAFE_RE.sub("_", oc.sanitize_model(model))


def cmd_setup(scratch: str) -> int:
    scratch = Path(scratch).resolve()
    if is_inside_repo(scratch):
        print("setup: scratch directory is inside the repository; refused", file=sys.stderr)
        return 2
    tpl = scratch / "tpl"
    if tpl.exists():
        shutil.rmtree(tpl)
    tpl.mkdir(parents=True, exist_ok=True)
    schemas_src = REPO / "schemas"
    for name in BRIEF_ORDER:
        brief_dir = HERE / "briefs" / name
        try:
            brief = scoring.load_brief(brief_dir)
        except scoring.BriefError as error:
            print(f"setup: {error}", file=sys.stderr)
            return 2
        template = tpl / name
        template.mkdir(parents=True, exist_ok=True)
        (template / "brief.md").write_bytes((brief_dir / "prompt.txt").read_bytes())
        schemas_dst = template / "schemas"
        schemas_dst.mkdir()
        for source in sorted(schemas_src.glob("*.json")):
            if source.is_file():
                shutil.copy2(source, schemas_dst / source.name)
        if brief["starter"] is not None:
            shutil.copytree(REPO / brief["starter"], template / "campaign")
    print(f"templates ready in {scratch}/tpl")
    return 0


def cmd_run(scratch: str, args: argparse.Namespace) -> int:
    scratch = Path(scratch).resolve()
    if is_inside_repo(scratch):
        print("run: scratch directory is inside the repository; refused", file=sys.stderr)
        return 2

    models = parse_models(args.models)

    if args.briefs == "all":
        brief_names = list(BRIEF_ORDER)
    else:
        brief_names = [x.strip() for x in args.briefs.split(",") if x.strip()]
    for name in brief_names:
        if name not in BRIEF_ORDER:
            print(f"run: unknown brief {name}", file=sys.stderr)
            return 2

    for name in brief_names:
        if not (scratch / "tpl" / name).is_dir():
            print(f"run: template missing for {name}; run setup first", file=sys.stderr)
            return 2

    if args.repeat < 1 or args.par < 1:
        print("run: --repeat and --par must be at least 1", file=sys.stderr)
        return 2

    if args.tools is None:
        tools_path = HERE / "tools-baseline.txt"
    else:
        tools_path = Path(args.tools)
        if not tools_path.is_absolute():
            tools_path = Path.cwd() / tools_path
    try:
        tools_text = tools_path.read_text(encoding="utf-8")
    except OSError:
        print(f"run: cannot read tools file {tools_path}", file=sys.stderr)
        return 2

    crpgc = resolve_crpgc(args.crpgc)
    if crpgc is None:
        print("authorbench: no-crpgc: build it or pass --crpgc", file=sys.stderr)
        return 2

    briefs = {}
    for name in brief_names:
        try:
            briefs[name] = scoring.load_brief(HERE / "briefs" / name)
        except scoring.BriefError as error:
            print(f"run: {error}", file=sys.stderr)
            return 2

    design = args.design or DEFAULT_DESIGN
    out_name = args.out or design
    repeat = args.repeat
    variant = args.variant
    suffix = ""
    if crpgc.endswith(".py"):
        suffix = ".py"
    elif crpgc.endswith(".exe"):
        suffix = ".exe"

    bin_dir = scratch / out_name / "bin"
    bin_dir.mkdir(parents=True, exist_ok=True)
    shutil.copy2(crpgc, bin_dir / f"crpgc{suffix}")

    runs_root = scratch / out_name / "runs"
    logs_root = scratch / out_name / "logs"
    runs_root.mkdir(parents=True, exist_ok=True)
    logs_root.mkdir(parents=True, exist_ok=True)

    tools_sha256 = hashlib.sha256(tools_text.encode("utf-8")).hexdigest()
    crpgc_sha256 = hashlib.sha256(Path(crpgc).read_bytes()).hexdigest()

    def run_one(name: str, model: str, rep: int) -> int:
        safe = _safe_model(model)
        run_dir = runs_root / name / safe
        if repeat > 1:
            run_dir = run_dir / f"r{rep}"
        if run_dir.exists():
            shutil.rmtree(run_dir)
        shutil.copytree(scratch / "tpl" / name, run_dir)
        (run_dir / "brief.md").write_bytes(
            render_brief(briefs[name]["prompt"], tools_text).encode("utf-8"))
        write_opencode_json(run_dir)

        log_name = f"{name}__{safe}.jsonl"
        if repeat > 1:
            log_name = f"{name}__{safe}__r{rep}.jsonl"
        log_path = logs_root / log_name
        meta_path = Path(str(log_path) + ".meta.json")

        timeout_s = briefs[name]["timeout_s"]
        prefix = oc.opencode_prefix()
        major = oc.opencode_major(prefix)
        cmd = list(prefix)
        cmd.append("run")
        if major >= 2:
            cmd.append("--standalone")
        cmd += oc.model_flag(model, variant, major)
        if major < 2:
            cmd += ["--dir", str(run_dir)]
        cmd += ["--format", "json", PROMPT]

        t0 = time.time()
        cause = None
        rc = None
        for attempt in range(3):
            oc.wait_start_gap(REPO)
            if log_path.exists():
                log_path.unlink()
            cause, rc = oc.run_worker(cmd, run_dir, log_path, timeout_s, 0, "implement", model)
            text = log_path.read_text(encoding="utf-8", errors="replace") if log_path.is_file() else ""
            if "database is locked" in text and attempt < 2:
                time.sleep(oc.env_float("CRPG_OC_LOCK_WAIT", 5.0))
                continue
            break

        rc_out = "TIMEOUT" if cause == "timeout" else rc
        secs = round(time.time() - t0, 1)
        dir_rel = f"runs/{name}/{safe}" + (f"/r{rep}" if repeat > 1 else "")
        meta = {
            "brief": name,
            "model": model,
            "repeat": rep,
            "variant": variant,
            "design": design,
            "rc": rc_out,
            "secs": secs,
            "dir": dir_rel,
            "log": f"logs/{log_name}",
            "tools_sha256": tools_sha256,
            "crpgc_sha256": crpgc_sha256,
        }
        meta_path.write_bytes(json.dumps(meta, indent=1).encode("utf-8"))
        print(f"{name} {model} rc={rc_out} {secs}s", flush=True)
        if cause == "timeout":
            return 1
        if isinstance(rc, int) and rc != 0:
            return 1
        return 0

    old_path = os.environ.get("PATH", "")
    os.environ["PATH"] = str(bin_dir) + os.pathsep + old_path
    try:
        jobs = [(name, model, rep) for name in brief_names for model in models for rep in range(repeat)]
        rc_total = 0
        with cf.ThreadPoolExecutor(args.par) as pool:
            futures = [pool.submit(run_one, name, model, rep) for (name, model, rep) in jobs]
            for future in futures:
                rc_total = rc_total or future.result()
    finally:
        os.environ["PATH"] = old_path
    return rc_total


def cmd_score(scratch: str, args: argparse.Namespace) -> int:
    scratch = Path(scratch).resolve()
    if is_inside_repo(scratch):
        print("score: scratch directory is inside the repository; refused", file=sys.stderr)
        return 2

    out = args.out or args.design or DEFAULT_DESIGN
    results_dir = scratch / out
    logs_dir = results_dir / "logs"
    meta_files = sorted(logs_dir.glob("*.meta.json")) if logs_dir.is_dir() else []
    if not meta_files:
        print(f"score: no runs in {out}", file=sys.stderr)
        return 2

    metas = []
    for meta_path in meta_files:
        meta = json.loads(meta_path.read_text(encoding="utf-8"))
        if meta.get("brief") not in BRIEF_ORDER:
            print(f"score: unknown brief {meta.get('brief')}", file=sys.stderr)
            return 2
        metas.append(meta)

    crpgc = None
    if args.crpgc:
        # An explicit --crpgc decides alone (B-10/B-14): it never falls back
        # to <out>/bin or resolve_crpgc(None) when it does not name a file.
        crpgc = resolve_crpgc(args.crpgc)
        if crpgc is None:
            print("authorbench: no-crpgc: build it or pass --crpgc", file=sys.stderr)
            return 2
    else:
        for suffix in (".exe", ".py", ""):
            candidate = results_dir / "bin" / f"crpgc{suffix}"
            if candidate.is_file():
                crpgc = str(candidate.resolve())
                break
        if crpgc is None:
            crpgc = resolve_crpgc(None)
        if crpgc is None:
            print("authorbench: no-crpgc: build it or pass --crpgc", file=sys.stderr)
            return 2

    prefix = scoring.crpgc_prefix(crpgc)
    smoke = scoring.smoke_check(prefix)
    briefs = {}
    rows = []
    roots = [str(scratch), str(Path.home())]
    for meta in metas:
        name = meta["brief"]
        if name not in briefs:
            briefs[name] = scoring.load_brief(HERE / "briefs" / name)
        log_path = results_dir / meta["log"]
        log = scoring.read_log(log_path)
        err = scoring.scrub_text(log["err"], roots)
        excluded = meta["rc"] != 0 and log["steps"] == 0
        row = {
            "brief": name,
            "model": meta["model"],
            "repeat": meta["repeat"],
            "variant": meta["variant"],
            "design": meta["design"],
            "rc": meta["rc"],
            "secs": meta["secs"],
            "excluded": excluded,
            "steps": log["steps"],
            "tools": log["tools"],
            "crpgc_calls": log["crpgc_calls"],
            "errors": log["crpgc_failures"],
            "commands": log["commands"],
            "tok": log["tok"],
            "cost": log["cost"],
            "err": err,
        }
        if excluded:
            score = scoring.empty_score()
        else:
            score = scoring.score_run(briefs[name], results_dir / meta["dir"], prefix)
        row.update(score)
        rows.append(row)

    rows.sort(key=lambda r: (BRIEF_ORDER.index(r["brief"]), r["model"], r["repeat"]))
    cells = scoring.aggregate(rows, BRIEF_ORDER)
    design = metas[0]["design"]
    scores = {
        "tool": "authorbench",
        "version": 1,
        "design": design,
        "smoke": smoke,
        "rows": rows,
        "cells": cells,
    }
    results_dir.mkdir(parents=True, exist_ok=True)
    content = json.dumps(scores, indent=1) + "\n"
    (results_dir / "scores.json").write_bytes(content.encode("utf-8"))

    if args.json:
        # Print the file's content only, byte-identical to scores.json (LF,
        # no newline translation): write the bytes straight to the binary
        # stdout, because `print` would turn `\n` into `\r\n` on Windows.
        sys.stdout.buffer.write(content.encode("utf-8"))
        sys.stdout.buffer.flush()
        return 0

    runs = len(rows)
    valid = sum(1 for r in rows if r["valid"])
    playable = sum(1 for r in rows if r["playable"])
    excluded = sum(1 for r in rows if r["excluded"])
    smoke_word = "ok" if smoke["ok"] else "FAILED"
    print(f"SCORED {design}: {runs} runs, {valid} valid, {playable} playable, "
          f"{excluded} excluded (smoke {smoke_word})")
    for cell in cells:
        steps = "-" if cell["steps_mean"] is None else cell["steps_mean"]
        errors = "-" if cell["errors_mean"] is None else cell["errors_mean"]
        print(f"| {cell['brief']} | {cell['model']} | n={cell['n']} | valid {cell['valid']} "
              f"| complete {cell['complete']} | playable {cell['playable']} "
              f"| finished {cell['finished']} | steps {steps} | errors {errors} |")
    return 0


def cmd_compare(scratch: str, args: argparse.Namespace) -> int:
    scratch = Path(scratch).resolve()
    if is_inside_repo(scratch):
        print("compare: scratch directory is inside the repository; refused", file=sys.stderr)
        return 2
    base_out = args.base_out
    cand_out = args.cand_out
    base_path = scratch / base_out / "scores.json"
    cand_path = scratch / cand_out / "scores.json"
    if not base_path.is_file():
        print(f"compare: no scores in {base_out}", file=sys.stderr)
        return 2
    if not cand_path.is_file():
        print(f"compare: no scores in {cand_out}", file=sys.stderr)
        return 2
    base = json.loads(base_path.read_text(encoding="utf-8"))
    cand = json.loads(cand_path.read_text(encoding="utf-8"))
    cells = scoring.compare_cells(base["cells"], cand["cells"])
    if args.json:
        print(json.dumps(cells, indent=1))
        return 0
    better = sum(1 for c in cells if c["verdict"] == "better")
    same = sum(1 for c in cells if c["verdict"] == "same")
    worse = sum(1 for c in cells if c["verdict"] == "worse")
    print(f"COMPARE {base_out} -> {cand_out}: {better} better, {same} same, {worse} worse")
    for cell in cells:
        b = cell["base"]
        c = cell["cand"]
        print(f"| {cell['brief']} | {cell['model']} | "
              f"playable {b['playable']}/{b['n']} -> {c['playable']}/{c['n']} | "
              f"complete {b['complete']}/{b['n']} -> {c['complete']}/{c['n']} | "
              f"valid {b['valid']}/{b['n']} -> {c['valid']}/{c['n']} | "
              f"steps {cell['d_steps']:+} | errors {cell['d_errors']:+} | {cell['verdict']} |")
    return 0


def cmd_all(scratch: str, args: argparse.Namespace) -> int:
    scratch = Path(scratch).resolve()
    if is_inside_repo(scratch):
        print("all: scratch directory is inside the repository; refused", file=sys.stderr)
        return 2
    rc = cmd_setup(scratch)
    if rc == 2:
        return 2
    models = args.models if args.models is not None else "@tools/agents/authorbench/models.txt"
    run_args = argparse.Namespace(
        models=models,
        briefs=args.briefs,
        design=args.design,
        out=args.out,
        variant=args.variant,
        repeat=args.repeat,
        par=args.par,
        crpgc=args.crpgc,
        tools=args.tools,
    )
    rc = cmd_run(scratch, run_args)
    if rc == 2:
        return 2
    score_args = argparse.Namespace(out=args.out, design=args.design, crpgc=args.crpgc, json=False)
    rc_score = cmd_score(scratch, score_args)
    if rc_score == 2:
        return 2
    return rc


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="authorbench.py")
    sub = parser.add_subparsers(dest="cmd", required=True)

    p_setup = sub.add_parser("setup")
    p_setup.add_argument("scratch")

    p_run = sub.add_parser("run")
    p_run.add_argument("scratch")
    p_run.add_argument("--models", default=None)
    p_run.add_argument("--briefs", default="all")
    p_run.add_argument("--design", default=DEFAULT_DESIGN)
    p_run.add_argument("--out", default=None)
    p_run.add_argument("--variant", default=None)
    p_run.add_argument("--repeat", type=int, default=1)
    p_run.add_argument("--par", type=int, default=3)
    p_run.add_argument("--crpgc", default=None)
    p_run.add_argument("--tools", default=None)

    p_score = sub.add_parser("score")
    p_score.add_argument("scratch")
    p_score.add_argument("--design", default=DEFAULT_DESIGN)
    p_score.add_argument("--out", default=None)
    p_score.add_argument("--crpgc", default=None)
    p_score.add_argument("--json", action="store_true")

    p_compare = sub.add_parser("compare")
    p_compare.add_argument("scratch")
    p_compare.add_argument("base_out")
    p_compare.add_argument("cand_out")
    p_compare.add_argument("--json", action="store_true")

    p_all = sub.add_parser("all")
    p_all.add_argument("scratch")
    p_all.add_argument("--models", default=None)
    p_all.add_argument("--briefs", default="all")
    p_all.add_argument("--design", default=DEFAULT_DESIGN)
    p_all.add_argument("--out", default=None)
    p_all.add_argument("--variant", default=None)
    p_all.add_argument("--repeat", type=int, default=1)
    p_all.add_argument("--par", type=int, default=3)
    p_all.add_argument("--crpgc", default=None)
    p_all.add_argument("--tools", default=None)

    args = parser.parse_args(argv)
    try:
        if args.cmd == "setup":
            return cmd_setup(args.scratch)
        if args.cmd == "run":
            return cmd_run(args.scratch, args)
        if args.cmd == "score":
            return cmd_score(args.scratch, args)
        if args.cmd == "compare":
            return cmd_compare(args.scratch, args)
        if args.cmd == "all":
            return cmd_all(args.scratch, args)
        return 2
    except oc.Exit as error:
        print(f"authorbench: {error.cause}", file=sys.stderr)
        return error.code


if __name__ == "__main__":
    sys.exit(main())

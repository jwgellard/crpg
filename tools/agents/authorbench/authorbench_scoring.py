#!/usr/bin/env python3
"""The scoring half of the authoring benchmark (T100a).

Judges one run of an agent authoring a campaign: brief loading and checking,
event-log reading, the `crpgc` calls, the scorer, aggregation and comparison.
Standard library only; Python 3.11+. It never reads the clock for a score and
never re-invokes a model.
"""

from __future__ import annotations

import json
import os
import re
import stat
import subprocess
import sys
from fractions import Fraction
from pathlib import Path

KEYS = ("name", "kind", "starter", "minimum", "ruleset_slug", "min_combatants", "timeout_s")
KINDS = ("extend", "campaign", "ruleset")
TOOLS_MARK = "{{TOOLS}}"
PLAY_INPUT_LINES = 400          # lines of "1\n" fed to `crpgc play`
CALL_TIMEOUT_S = 60             # per crpgc call made by the scorer
SMOKE_ARGS = ["run", "--ticks", "100", "--hash-every", "50", "--seed", "0"]
SMOKE_SAMPLES = 2               # ticks 50 and 100

_MIN_KIND_RE = re.compile(r"^[a-z][a-z0-9-]*$")
_SCHEMA_RE = re.compile(r"^crpg\.([a-z][a-z0-9-]*)/[0-9]+$")
_SAMPLE_RE = re.compile(r"^[0-9]+ [0-9a-f]{64}$")
_COMBATANT_RE = re.compile(r"^  .+ \| side .+ \| [0-9]+/[0-9]+ hp$")
_ROUNDS_RE = re.compile(r"^Rounds played: ([0-9]+)$")
_CRPGC_CALL_RE = re.compile(r"\bcrpgc(?:\.exe)?\s+([a-z][a-z-]*)")


class BriefError(ValueError):
    ...


def _is_int(value):
    return isinstance(value, int) and not isinstance(value, bool)


def _is_number(value):
    return isinstance(value, (int, float)) and not isinstance(value, bool)


def _decode(bts):
    return bts.decode("utf-8", errors="replace").replace("\r\n", "\n")


def _lines(text: str) -> list[str]:
    """Split text into ``\\n``-terminated lines only.

    A line is one ``\\n``-terminated line: unlike ``str.splitlines()`` this
    does not split on U+2028, U+2029, U+0085 or the U+001C..U+001E range, so a
    JSON event carrying a raw separator is not silently broken into fragments.
    ``\\r\\n`` is read as ``\\n`` first, and one trailing empty piece (from a
    final newline) is dropped.
    """
    text = text.replace("\r\n", "\n")
    parts = text.split("\n")
    if parts and parts[-1] == "":
        parts.pop()
    return parts


def load_brief(brief_dir: Path) -> dict:
    brief_dir = Path(brief_dir)
    name = brief_dir.name

    def fail(text):
        raise BriefError(f"brief {name}: {text}")

    brief_path = brief_dir / "brief.json"
    try:
        brief = json.loads(brief_path.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        brief = None
    if not isinstance(brief, dict):
        fail("brief.json is not a JSON object")

    unknown = sorted(k for k in brief if k not in KEYS)
    if unknown:
        fail(f"unknown key {unknown[0]}")
    for key in KEYS:
        if key not in brief:
            fail(f"missing key {key}")

    if brief["name"] != name:
        fail("name must equal the directory name")

    kind = brief["kind"]
    if kind not in KINDS:
        fail("kind must be one of extend, campaign, ruleset")

    starter = brief["starter"]
    if kind == "extend":
        ok = (
            isinstance(starter, str)
            and starter != ""
            and not starter.startswith("/")
            and "\\" not in starter
            and ".." not in starter.split("/")
        )
        if not ok:
            fail("starter must be a repository-relative path")
    elif starter is not None:
        fail("starter must be null")

    minimum = brief["minimum"]
    good_minimum = False
    if isinstance(minimum, dict) and minimum:
        good_minimum = all(
            isinstance(k, str) and _MIN_KIND_RE.match(k) and _is_int(v) and v >= 1
            for k, v in minimum.items()
        )
    if not good_minimum:
        fail("minimum must map kinds to integers >= 1")

    ruleset_slug = brief["ruleset_slug"]
    if kind == "ruleset":
        if not (isinstance(ruleset_slug, str) and ruleset_slug != ""):
            fail("ruleset_slug must be a string for kind ruleset, else null")
    elif ruleset_slug is not None:
        fail("ruleset_slug must be a string for kind ruleset, else null")

    if not (_is_int(brief["min_combatants"]) and 0 <= brief["min_combatants"] <= 8):
        fail("min_combatants must be an integer 0..8")

    if not (_is_int(brief["timeout_s"]) and 60 <= brief["timeout_s"] <= 7200):
        fail("timeout_s must be an integer 60..7200")

    prompt_path = brief_dir / "prompt.txt"
    try:
        prompt = prompt_path.read_text(encoding="utf-8")
    except OSError:
        fail("prompt.txt is missing")
    if prompt.count(TOOLS_MARK) != 1:
        fail("prompt.txt must contain {{TOOLS}} exactly once")

    result = dict(brief)
    result["prompt"] = prompt
    return result


def read_log(path: Path) -> dict:
    steps = 0
    tools = 0
    crpgc_calls = 0
    crpgc_failures = 0
    commands = {}
    tok = {"input": 0, "output": 0, "reasoning": 0, "cache_read": 0}
    cost = 0.0
    err = ""

    try:
        text = Path(path).read_text(encoding="utf-8", errors="replace")
    except OSError:
        text = ""

    for line in _lines(text):
        try:
            event = json.loads(line)
        except ValueError:
            if "rror" in line and not err:
                err = line.strip()[:160]
            continue
        if not isinstance(event, dict):
            continue

        etype = event.get("type")
        part = event.get("part")
        part = part if isinstance(part, dict) else {}
        if etype == "step_finish":
            steps += 1
            tokens = part.get("tokens")
            tokens = tokens if isinstance(tokens, dict) else {}
            cache = tokens.get("cache")
            cache = cache if isinstance(cache, dict) else {}
            tok["input"] += tokens.get("input") if _is_number(tokens.get("input")) else 0
            tok["output"] += tokens.get("output") if _is_number(tokens.get("output")) else 0
            tok["reasoning"] += tokens.get("reasoning") if _is_number(tokens.get("reasoning")) else 0
            tok["cache_read"] += cache.get("read") if _is_number(cache.get("read")) else 0
            c = part.get("cost")
            if _is_number(c):
                cost += c
        elif etype == "error":
            if not err:
                err = json.dumps(event.get("error"))[:200]
        elif etype == "tool_use":
            tools += 1
            state = part.get("state")
            if part.get("tool") in ("bash", "shell") and isinstance(state, dict):
                inp = state.get("input")
                command = inp.get("command") if isinstance(inp, dict) else None
                if isinstance(command, str):
                    names = _CRPGC_CALL_RE.findall(command)
                    if names:
                        crpgc_calls += 1
                        for n in names:
                            commands[n] = commands.get(n, 0) + 1
                        metadata = state.get("metadata")
                        metadata = metadata if isinstance(metadata, dict) else {}
                        exit_code = metadata.get("exit")
                        if not _is_int(exit_code):
                            nested = metadata.get("metadata")
                            nested = nested if isinstance(nested, dict) else {}
                            exit_code = nested.get("exit")
                        if state.get("status") == "error":
                            crpgc_failures += 1
                        elif _is_int(exit_code) and exit_code != 0:
                            crpgc_failures += 1

    return {
        "steps": steps,
        "tools": tools,
        "crpgc_calls": crpgc_calls,
        "crpgc_failures": crpgc_failures,
        "commands": dict(sorted(commands.items())),
        "tok": tok,
        "cost": cost,
        "err": err,
    }


def ends_in_service_fault(path: Path) -> bool:
    """True when the OpenCode service, not the model, ended the run.

    A run the service cut off is excluded like a run that never reached the
    model, rather than counted against the model. Two rules apply:

    * T100-E4: a Go quota error on ANY non-empty line (a JSON object with
      ``type == "error"`` whose ``error`` is an object with
      ``error["type"] == "provider.go-limit"``) excludes the run, because
      OpenCode retries a quota stop only after hours, so such a run can end
      only in the harness timeout.
    * T100-E2: otherwise only the LAST non-empty line is read, because a run
      that kept working after a transient error did not end in one. That line
      is a fault when it is an error event mentioning ``Failed to execute
      statement`` or carrying a 5xx ``statusCode`` (integer, not boolean),
      or a non-JSON line containing ``Failed to execute statement``.
    """
    try:
        text = Path(path).read_text(encoding="utf-8", errors="replace")
    except OSError:
        return False

    lines = [line for line in _lines(text) if line.strip()]
    if not lines:
        return False

    for line in lines:
        try:
            parsed = json.loads(line)
        except ValueError:
            continue
        if isinstance(parsed, dict) and parsed.get("type") == "error":
            err = parsed.get("error")
            if isinstance(err, dict) and err.get("type") == "provider.go-limit":
                return True

    last = lines[-1]
    try:
        event = json.loads(last)
    except ValueError:
        return "Failed to execute statement" in last
    if not isinstance(event, dict) or event.get("type") != "error":
        return False

    error = event.get("error")
    if "Failed to execute statement" in json.dumps(error):
        return True
    if not isinstance(error, dict):
        return False

    def is_5xx(value):
        return _is_int(value) and 500 <= value <= 599

    data = error.get("data")
    if isinstance(data, dict) and is_5xx(data.get("statusCode")):
        return True
    return is_5xx(error.get("statusCode"))


def crpgc_prefix(path: str) -> list[str]:
    if path.endswith(".py"):
        return [sys.executable, path]
    return [path]


def call_crpgc(prefix: list[str], args: list[str], stdin_bytes: bytes | None = None,
               timeout_s: float = CALL_TIMEOUT_S) -> dict:
    try:
        if stdin_bytes is None:
            proc = subprocess.run(prefix + args, stdin=subprocess.DEVNULL,
                                  capture_output=True, timeout=timeout_s, env=os.environ)
        else:
            proc = subprocess.run(prefix + args, input=stdin_bytes,
                                  capture_output=True, timeout=timeout_s, env=os.environ)
        return {
            "rc": proc.returncode,
            "stdout": _decode(proc.stdout),
            "stderr": _decode(proc.stderr),
            "timeout": False,
        }
    except subprocess.TimeoutExpired:
        return {"rc": None, "stdout": "", "stderr": "", "timeout": True}
    except OSError as exc:
        return {"rc": None, "stdout": "", "stderr": str(exc), "timeout": False}


def scan_campaign(campaign_dir: Path) -> dict:
    campaign_dir = Path(campaign_dir)
    files = 0
    json_count = 0
    kinds = {}
    ruleset_slugs = []
    unreadable = 0

    if not campaign_dir.is_dir():
        return {"files": 0, "json": 0, "kinds": {}, "ruleset_slugs": [], "unreadable": 0}

    for root, dirs, filenames in os.walk(campaign_dir, followlinks=False):
        dirs.sort()
        for filename in sorted(filenames):
            file_path = Path(root) / filename
            try:
                st = os.lstat(file_path)
            except OSError:
                continue
            if not stat.S_ISREG(st.st_mode):
                continue
            files += 1
            if filename.endswith(".json"):
                json_count += 1
                try:
                    doc = json.loads(file_path.read_text(encoding="utf-8"))
                except (OSError, ValueError):
                    unreadable += 1
                    continue
                if not isinstance(doc, dict):
                    unreadable += 1
                    continue
                schema = doc.get("schema")
                match = _SCHEMA_RE.match(schema) if isinstance(schema, str) else None
                if match:
                    kind = match.group(1)
                    kinds[kind] = kinds.get(kind, 0) + 1
                    if kind == "ruleset" and isinstance(doc.get("slug"), str):
                        ruleset_slugs.append(doc["slug"])

    return {
        "files": files,
        "json": json_count,
        "kinds": dict(sorted(kinds.items())),
        "ruleset_slugs": sorted(ruleset_slugs),
        "unreadable": unreadable,
    }


def parse_diagnostics(text: str) -> list | None:
    try:
        data = json.loads(text)
    except ValueError:
        return None
    if isinstance(data, list) and all(isinstance(item, dict) for item in data):
        return data
    return None


def parse_play(stdout: str, rc: int | None) -> dict:
    lines = _lines(stdout)
    started = rc == 0 and bool(lines) and lines[0].startswith("== Combat demo: ")
    finished = False
    combatants = 0
    rounds = None
    if started:
        finished = "== Summary ==" in lines
        in_header = False
        for line in lines:
            if line == "Combatants:":
                in_header = True
                continue
            if in_header:
                if line == "":
                    break
                if _COMBATANT_RE.match(line):
                    combatants += 1
        for line in lines:
            match = _ROUNDS_RE.match(line)
            if match:
                rounds = int(match.group(1))
                break
    return {"started": started, "finished": finished, "combatants": combatants, "rounds": rounds}


def count_samples(stdout: str) -> int:
    return sum(1 for line in _lines(stdout) if _SAMPLE_RE.match(line))


def smoke_check(prefix: list[str]) -> dict:
    result = call_crpgc(prefix, SMOKE_ARGS)
    samples = count_samples(result["stdout"])
    return {"rc": result["rc"], "samples": samples, "ok": result["rc"] == 0 and samples == SMOKE_SAMPLES}


def _not_run_validate() -> dict:
    return {"rc": None, "errors": 0, "warnings": 0, "codes": [], "unparsable": False}


def _not_run_play() -> dict:
    return {"rc": None, "started": False, "finished": False, "combatants": 0, "rounds": None}


def empty_score() -> dict:
    return {
        "campaign_present": False,
        "files": 0,
        "kinds": {},
        "valid": False,
        "validate": _not_run_validate(),
        "complete": False,
        "play": _not_run_play(),
        "playable": False,
        "first_error": None,
    }


def score_run(brief: dict, run_dir: Path, prefix: list[str]) -> dict:
    run_dir = Path(run_dir)
    campaign_dir = run_dir / "campaign"
    campaign_present = campaign_dir.is_dir()

    if campaign_present:
        scan = scan_campaign(campaign_dir)
    else:
        scan = {"files": 0, "kinds": {}, "ruleset_slugs": []}
    files = scan["files"]
    kinds = scan["kinds"]

    first_error = None
    if campaign_present:
        result = call_crpgc(prefix, ["validate", str(campaign_dir), "--json"])
        rc = result["rc"]
        diags = parse_diagnostics(result["stdout"])
        unparsable = rc is not None and diags is None
        errors = 0
        warnings = 0
        codes = []
        if isinstance(diags, list):
            for diag in diags:
                severity = diag.get("severity")
                if severity == "error":
                    errors += 1
                    code = diag.get("code")
                    if isinstance(code, str):
                        codes.append(code)
                    if first_error is None:
                        first_error = {
                            "file": diag.get("file"),
                            "code": diag.get("code"),
                            "message": str(diag.get("message", ""))[:160],
                        }
                elif severity == "warning":
                    warnings += 1
        validate = {
            "rc": rc,
            "errors": errors,
            "warnings": warnings,
            "codes": sorted(set(codes))[:10],
            "unparsable": unparsable,
        }
        valid = rc == 0 and diags is not None
    else:
        validate = _not_run_validate()
        valid = False

    minimum = brief.get("minimum", {})
    complete = all(kinds.get(kind, 0) >= need for kind, need in minimum.items())
    ruleset_slug = brief.get("ruleset_slug")
    if complete and ruleset_slug is not None and ruleset_slug not in scan["ruleset_slugs"]:
        complete = False

    if valid:
        play_result = call_crpgc(
            prefix,
            ["play", "--campaign", str(campaign_dir), "--seed", "0"],
            stdin_bytes=b"1\n" * PLAY_INPUT_LINES,
        )
        play = {"rc": play_result["rc"]}
        play.update(parse_play(play_result["stdout"], play_result["rc"]))
    else:
        play = _not_run_play()

    playable = valid and complete and play["started"] and play["combatants"] >= brief["min_combatants"]

    return {
        "campaign_present": campaign_present,
        "files": files,
        "kinds": kinds,
        "valid": valid,
        "validate": validate,
        "complete": complete,
        "play": play,
        "playable": playable,
        "first_error": first_error,
    }


def aggregate(rows: list[dict], brief_order: list[str]) -> list[dict]:
    groups = {}
    order = {}
    for row in rows:
        brief = row.get("brief")
        model = row.get("model")
        key = (brief, model)
        if key not in groups:
            groups[key] = []
            order[key] = brief_order.index(brief) if brief in brief_order else len(brief_order)
        groups[key].append(row)

    cells = []
    for key in sorted(groups, key=lambda k: (order[k], k[1])):
        brief, model = key
        group = groups[key]
        excluded = 0
        non_excluded = []
        for row in group:
            # B-8 (T100-E2): a run that never reached the model, or one cut
            # off by the service, is excluded and left out of every count.
            if (row.get("rc") != 0 and row.get("steps", 0) == 0) or row.get("excluded") is True:
                excluded += 1
            else:
                non_excluded.append(row)
        n = len(non_excluded)
        valid = sum(1 for r in non_excluded if r.get("valid"))
        complete = sum(1 for r in non_excluded if r.get("complete"))
        playable = sum(1 for r in non_excluded if r.get("playable"))
        finished = sum(1 for r in non_excluded if (r.get("play") or {}).get("finished"))
        if n:
            steps_mean = round(sum(r.get("steps", 0) for r in non_excluded) / n, 1)
            errors_mean = round(sum(r.get("errors", 0) for r in non_excluded) / n, 1)
            secs_mean = round(sum(r.get("secs", 0) for r in non_excluded) / n, 1)
        else:
            steps_mean = errors_mean = secs_mean = None
        cost_total = round(sum(r.get("cost", 0) for r in group), 4)
        cells.append({
            "brief": brief,
            "model": model,
            "n": n,
            "excluded": excluded,
            "valid": valid,
            "complete": complete,
            "playable": playable,
            "finished": finished,
            "steps_mean": steps_mean,
            "errors_mean": errors_mean,
            "secs_mean": secs_mean,
            "cost_total": cost_total,
        })
    return cells


def compare_cells(base: list[dict], cand: list[dict]) -> list[dict]:
    cand_map = {(c["brief"], c["model"]): c for c in cand}
    result = []
    for b in base:
        key = (b["brief"], b["model"])
        c = cand_map.get(key)
        if c is None or b["n"] <= 0 or c["n"] <= 0:
            continue
        base_tuple = (Fraction(b["playable"], b["n"]), Fraction(b["complete"], b["n"]), Fraction(b["valid"], b["n"]))
        cand_tuple = (Fraction(c["playable"], c["n"]), Fraction(c["complete"], c["n"]), Fraction(c["valid"], c["n"]))
        if cand_tuple > base_tuple:
            verdict = "better"
        elif cand_tuple < base_tuple:
            verdict = "worse"
        else:
            verdict = "same"
        result.append({
            "brief": b["brief"],
            "model": b["model"],
            "base": {"n": b["n"], "playable": b["playable"], "complete": b["complete"], "valid": b["valid"]},
            "cand": {"n": c["n"], "playable": c["playable"], "complete": c["complete"], "valid": c["valid"]},
            "d_steps": round(c["steps_mean"] - b["steps_mean"], 1),
            "d_errors": round(c["errors_mean"] - b["errors_mean"], 1),
            "verdict": verdict,
        })
    return result


def scrub_text(text: str, roots: list[str]) -> str:
    variants = []
    for root in roots:
        if not root:
            continue
        variants.append(root)
        if "\\" in root:
            variants.append(root.replace("\\", "/"))
        if "/" in root:
            variants.append(root.replace("/", "\\"))
    for variant in sorted(set(variants), key=len, reverse=True):
        text = text.replace(variant, "<path>")
    return text

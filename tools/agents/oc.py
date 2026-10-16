#!/usr/bin/env python3
"""OpenCode dispatcher: start workers safely, verify, print a verdict first.

Usage:
    python tools/agents/oc.py run --task ID --role ROLE --model MODEL [...]
    python tools/agents/oc.py wave SPEC [SPEC ...] [--timeout SEC] ...
    python tools/agents/oc.py ask --role scout --model MODEL QUESTION
    python tools/agents/oc.py record --task ID --role ROLE --model MODEL ...
    python tools/agents/oc.py stats [--role ROLE] [--since DATE] [--json]

Exit codes: 0 verified (or, for review/scout/ask, the worker left a text
message); 1 a check failed; 2 usage or infrastructure error. See
tools/agents/README.md and docs/AGENT_TEAM_PLAN.md section 5.1.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import signal
import statistics
import subprocess
import sys
import threading
import time
from datetime import datetime, timezone
from pathlib import Path

RESULT_VERSION = 1
ROLES = ("implement", "tests", "review", "scout", "grunt")
VERIFY_ROLES = ("implement", "tests", "grunt")
REVIEW_OUTCOMES = ("review-ready", "review-fix", "review-reject", "merged", "reverted")
PASS_OUTCOMES = {"review-ready", "merged"}

PROMPT = "Read BRIEF.md in this directory and do what it says."
STALL_SENTENCE = (
    "Write the code in small edits; do not plan the whole solution in one step."
)
STALL_VARIANT = "low"
DEFAULT_TIMEOUT = 1800
DEFAULT_IDLE = 600
START_GAP_DEFAULT = 4.0
LOCK_RETRIES = 2
MAX_SUMMARY_LINES = 40
WORKER_TAIL_LINES = 15
CHECK_TAIL_LINES = 12

# Thresholds from docs/agents/MODELS.md section 4.1 (demote and promote rule).
DEMOTE_WINDOW = 4
DEMOTE_FAILURES = 2
PROMOTE_MIN_RUNS = 5

# Infra causes that are not the model's fault: they never count against it.
NON_FAULT_CAUSES = frozenset({"quota", "402", "locked"})
# Infra causes that are the model's fault: they count against it.
MODEL_FAULT_CAUSES = frozenset({"no-edit", "timeout", "hang"})


class Exit(Exception):
    """Raised to stop a command with an exit code and a cause."""

    def __init__(self, code: int, cause: str | None = None) -> None:
        super().__init__(cause or "")
        self.code = code
        self.cause = cause


def eprint(*parts: str) -> None:
    print(*parts, file=sys.stderr)


def utcnow() -> str:
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def env_float(name: str, default: float) -> float:
    try:
        return float(os.environ.get(name, default))
    except (TypeError, ValueError):
        return default


def sanitize_model(model: str) -> str:
    return model.replace("/", "_")


# -- repository and slot paths -------------------------------------------


def repo_root() -> Path:
    """Main worktree of this repository, wherever the caller runs from."""
    override = os.environ.get("CRPG_REPO")
    if override:
        return Path(override).resolve()
    try:
        proc = subprocess.run(
            ["git", "worktree", "list", "--porcelain"],
            capture_output=True,
            text=True,
            timeout=30,
        )
    except (OSError, subprocess.SubprocessError) as error:
        raise Exit(2, f"no-repo: cannot list worktrees: {error}")
    if proc.returncode != 0:
        raise Exit(2, "no-repo: not inside a git repository")
    for line in proc.stdout.splitlines():
        if line.startswith("worktree "):
            return Path(line[len("worktree "):].strip()).resolve()
    raise Exit(2, "no-repo: empty worktree list")


def slots_dir(repo: Path) -> Path:
    override = os.environ.get("CRPG_SLOTS")
    if override:
        return Path(override).resolve()
    return (repo.parent / "crpg-slots").resolve()


def list_slots(slots: Path) -> list[tuple[int, Path]]:
    """Numbered slot-K directories, lowest number first."""
    found: list[tuple[int, Path]] = []
    if slots.is_dir():
        for child in slots.iterdir():
            match = re.fullmatch(r"slot-(\d+)", child.name)
            if match and child.is_dir():
                found.append((int(match.group(1)), child.resolve()))
    found.sort()
    return found


def runs_dir(repo: Path) -> Path:
    return repo / ".agents" / "runs"


def slot_lock_dir(repo: Path) -> Path:
    return repo / ".agents" / "slots"


# -- executable ----------------------------------------------------------


def opencode_prefix() -> list[str]:
    """Argv prefix for the real OpenCode executable, never the .cmd shim."""
    override = os.environ.get("CRPG_OPENCODE")
    if override:
        if override.endswith(".py"):
            return [sys.executable, override]
        return [override]
    if os.name == "nt":
        appdata = os.environ.get("APPDATA", "")
        exe = Path(appdata) / "npm" / "node_modules" / "opencode-ai" / "bin" / "opencode.exe"
        if exe.is_file():
            return [str(exe)]
    found = shutil.which("opencode")
    if found and not found.lower().endswith(".cmd"):
        return [found]
    if found:
        # A .cmd shim would parse < > | & inside the prompt through cmd.exe.
        sibling = Path(found).with_suffix(".exe")
        alt = Path(found).parent / "node_modules" / "opencode-ai" / "bin" / "opencode.exe"
        for candidate in (sibling, alt):
            if candidate.is_file():
                return [str(candidate)]
        raise Exit(2, "no-exe: only an opencode.cmd shim is on PATH")
    raise Exit(2, "no-exe: opencode not found")


# -- slot locks ------------------------------------------------------------


def _pid_alive(pid: int) -> bool:
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    except OSError:
        if os.name == "nt":
            try:
                proc = subprocess.run(
                    ["taskkill", "/FI", f"PID eq {pid}", "/NH"],
                    capture_output=True,
                    text=True,
                    timeout=15,
                )
                return str(pid) in proc.stdout
            except (OSError, subprocess.SubprocessError):
                return True
        return True
    return True


def _read_lock_pid(path: Path) -> int | None:
    try:
        return int(path.read_text(encoding="utf-8").split()[0])
    except (OSError, ValueError, IndexError):
        return None


# Slots held by this process (slot number -> count), so a wave thread that
# already holds a slot can re-enter it for its member run without deadlocking
# against its own lock file. Cross-process exclusion still uses the files.
_HELD: dict[int, int] = {}
_HELD_LOCK = threading.Lock()


def acquire_slot(
    repo: Path, slots: list[tuple[int, Path]], preferred: int | None, wait: bool
) -> tuple[int, Path]:
    """Lock one slot; returns its number and path. Raises Exit(2) when busy."""
    if not slots:
        raise Exit(2, "no-slot: no slot worktrees found")
    by_number = {number: path for number, path in slots}
    if preferred is not None:
        if preferred not in by_number:
            raise Exit(2, f"no-slot: slot-{preferred} does not exist")
        with _HELD_LOCK:
            if _HELD.get(preferred, 0) > 0:
                return preferred, by_number[preferred]
        candidates = [(preferred, by_number[preferred])]
    else:
        with _HELD_LOCK:
            candidates = [(n, p) for n, p in slots if n not in _HELD]
        if not candidates:
            if not wait:
                raise Exit(2, "slot-busy: every candidate slot is locked")
    lock_dir = slot_lock_dir(repo)
    deadline = time.time() + (3600.0 if wait else 0.0)
    while True:
        with _HELD_LOCK:
            free = [(n, p) for n, p in candidates if n not in _HELD] if preferred is None else candidates
        for number, path in free:
            lock_dir.mkdir(parents=True, exist_ok=True)
            lock = lock_dir / f"slot-{number}.lock"
            try:
                fd = os.open(str(lock), os.O_CREAT | os.O_EXCL | os.O_WRONLY)
            except FileExistsError:
                pid = _read_lock_pid(lock)
                if pid is not None and not _pid_alive(pid):
                    try:
                        lock.unlink()
                    except OSError:
                        pass
                    continue
                continue
            except OSError:
                continue
            with os.fdopen(fd, "w", encoding="utf-8") as handle:
                handle.write(f"{os.getpid()}\n")
            with _HELD_LOCK:
                _HELD[number] = _HELD.get(number, 0) + 1
            return number, path
        if not wait or time.time() >= deadline:
            raise Exit(2, "slot-busy: every candidate slot is locked")
        time.sleep(0.5)


def release_slot(repo: Path, number: int) -> None:
    with _HELD_LOCK:
        left = _HELD.get(number, 0) - 1
        if left <= 0:
            _HELD.pop(number, None)
        else:
            _HELD[number] = left
            return
    try:
        (slot_lock_dir(repo) / f"slot-{number}.lock").unlink()
    except OSError:
        pass


# -- start staggering -------------------------------------------------------


_START_THREAD_LOCK = threading.Lock()


def _read_start(path: Path) -> float | None:
    try:
        return float(path.read_text(encoding="utf-8").split()[0])
    except (OSError, ValueError, IndexError):
        return None


def wait_start_gap(repo: Path) -> None:
    """Keep worker starts at least the start gap apart, across processes."""
    gap = env_float("CRPG_OC_START_GAP", START_GAP_DEFAULT)
    if gap <= 0:
        return
    stamp_dir = repo / ".agents"
    stamp_dir.mkdir(parents=True, exist_ok=True)
    stamp = stamp_dir / "last_start"
    with _START_THREAD_LOCK:
        while True:
            last = _read_start(stamp)
            now = time.time()
            if last is not None and now < last + gap:
                time.sleep(last + gap - now)
                continue
            token = f"{time.time()} {os.getpid()}"
            try:
                stamp.write_text(token + "\n", encoding="utf-8")
            except OSError:
                return
            time.sleep(0.05)
            if stamp.read_text(encoding="utf-8").strip() == token:
                return


def lock_wait() -> float:
    return env_float("CRPG_OC_LOCK_WAIT", 5.0)


# -- git helpers -------------------------------------------------------------


def git(args: list[str], cwd: Path, timeout: int = 120) -> subprocess.CompletedProcess:
    return subprocess.run(
        ["git", *args],
        cwd=cwd,
        capture_output=True,
        text=True,
        timeout=timeout,
    )


def current_branch(slot: Path) -> str | None:
    proc = git(["rev-parse", "--abbrev-ref", "HEAD"], slot)
    if proc.returncode != 0:
        return None
    return proc.stdout.strip()


def slot_changed(slot: Path) -> bool:
    """True when the slot holds edits other than BRIEF.md."""
    proc = git(["status", "--porcelain"], slot)
    if proc.returncode != 0:
        return True
    for line in proc.stdout.splitlines():
        path = line[3:] if len(line) > 3 else line
        path = path.strip().strip('"')
        if path == "BRIEF.md" or path.endswith("/BRIEF.md"):
            continue
        if path:
            return True
    return False


def slot_committed(slot: Path, base: str) -> bool:
    proc = git(["rev-list", "--count", f"{base}..HEAD"], slot)
    if proc.returncode != 0:
        return False
    try:
        return int(proc.stdout.strip()) > 0
    except ValueError:
        return False


def slot_edited(slot: Path, base: str) -> bool:
    return slot_changed(slot) or slot_committed(slot, base)


def fetch_base(slot: Path, base: str) -> None:
    """Fetch the base's remote; a failed fetch is exit 2."""
    if "/" not in base:
        return
    remote, _, branch = base.partition("/")
    if not remote or not branch:
        raise Exit(2, f"fetch: malformed base {base!r}")
    try:
        proc = git(["fetch", remote, branch], slot, timeout=300)
    except subprocess.TimeoutExpired:
        raise Exit(2, "fetch: git fetch timed out")
    if proc.returncode != 0:
        raise Exit(2, f"fetch: {proc.stderr.strip() or proc.stdout.strip()}")


# -- brief -------------------------------------------------------------------


def prepare_brief(repo: Path, slot: Path, task: str, attempt: int,
                  brief: str | None, notes: str | None) -> None:
    target = slot / "BRIEF.md"
    if brief is not None:
        source = Path(brief)
        if not source.is_file():
            raise Exit(2, f"no-brief: {brief} does not exist")
        target.write_bytes(source.read_bytes())
    else:
        generator = repo / "tools" / "agents" / "brief.py"
        if not generator.is_file():
            raise Exit(2, "no-brief: no --brief given and tools/agents/brief.py is missing")
        proc = subprocess.run(
            [sys.executable, str(generator), task, "--out", str(target)],
            cwd=repo,
            capture_output=True,
            text=True,
            timeout=120,
        )
        if proc.returncode != 0 or not target.is_file():
            raise Exit(2, f"no-brief: brief.py failed: {proc.stderr.strip()}")
    if notes is not None:
        text = Path(notes).read_text(encoding="utf-8") if Path(notes).is_file() else notes
        with target.open("a", encoding="utf-8") as handle:
            handle.write(f"\n## Attempt {attempt} notes\n\n{text}")
            if not text.endswith("\n"):
                handle.write("\n")


# -- worker execution ----------------------------------------------------------


def opencode_log_path() -> Path:
    override = os.environ.get("CRPG_OPENCODE_LOG")
    if override:
        return Path(override)
    home = Path(os.environ.get("USERPROFILE") or os.path.expanduser("~"))
    return home / ".local" / "share" / "opencode" / "log" / "opencode.log"


def classify_infra(text: str) -> str | None:
    """Map provider/transport failure text to an infra cause, or None."""
    lowered = text.lower()
    if "402" in lowered or ("insufficient" in lowered and "funds" in lowered):
        return "402"
    for marker in ("rate limit", "quota", "usage limit", "too many requests", " 429"):
        if marker in lowered:
            return "quota"
    if "headers timed out" in lowered:
        return "timeout"
    if "model not found" in lowered or "unknown model" in lowered:
        return "model"
    return None


def _kill_tree(proc: subprocess.Popen) -> None:
    if proc.poll() is not None:
        return
    if os.name == "nt":
        subprocess.run(
            ["taskkill", "/T", "/F", "/PID", str(proc.pid)],
            capture_output=True,
            timeout=30,
        )
    else:
        try:
            os.killpg(os.getpgid(proc.pid), signal.SIGKILL)
        except (ProcessLookupError, PermissionError, OSError):
            try:
                proc.kill()
            except OSError:
                pass


def _watch_opencode_log(since_size: int, agent: str, model: str) -> str:
    """Classify new OpenCode-log errors for this run; 'hang' when none match."""
    suffix = model.split("/")[-1]
    try:
        with opencode_log_path().open("rb") as handle:
            handle.seek(since_size)
            blob = handle.read().decode("utf-8", errors="replace")
    except OSError:
        return "hang"
    for line in blob.splitlines():
        if "level=ERROR" not in line:
            continue
        if "agent=title" in line:
            continue
        if f"agent={agent}" not in line:
            continue
        if model not in line and f"modelID={suffix}" not in line:
            continue
        cause = classify_infra(line)
        if cause:
            return cause
    return "hang"


def run_worker(
    cmd: list[str],
    cwd: Path,
    log_path: Path,
    timeout_s: int,
    idle_s: int,
    agent: str,
    model: str,
) -> tuple[str | None, int]:
    """Run the worker; returns (idle/timeout cause or None, process rc).

    Never uses shell=True; stdin is always DEVNULL.
    """
    assert "--auto" not in cmd
    log_path.parent.mkdir(parents=True, exist_ok=True)
    try:
        log_size = opencode_log_path().stat().st_size
    except OSError:
        log_size = 0
    proc_args: dict = {
        "cwd": cwd,
        "env": {**os.environ, "PWD": str(cwd)},
        "stdin": subprocess.DEVNULL,
    }
    if os.name != "nt":
        proc_args["start_new_session"] = True
    start = time.time()
    deadline = start + timeout_s
    with log_path.open("a", encoding="utf-8") as log_file:
        try:
            proc = subprocess.Popen(cmd, stdout=log_file, stderr=subprocess.STDOUT, **proc_args)
        except OSError as error:
            raise Exit(2, f"no-exe: cannot start opencode: {error}")
        last_new = time.time()
        last_size = log_path.stat().st_size if log_path.exists() else 0
        rc = None
        idle_cause: str | None = None
        while True:
            try:
                rc = proc.wait(timeout=0.25)
                break
            except subprocess.TimeoutExpired:
                pass
            now = time.time()
            try:
                size = log_path.stat().st_size
            except OSError:
                size = last_size
            if size != last_size:
                last_size = size
                last_new = now
            if now >= deadline:
                _kill_tree(proc)
                try:
                    rc = proc.wait(timeout=30)
                except subprocess.TimeoutExpired:
                    rc = 124
                return "timeout", rc
            if idle_s > 0 and now - last_new >= idle_s:
                idle_cause = _watch_opencode_log(log_size, agent, model)
                _kill_tree(proc)
                try:
                    rc = proc.wait(timeout=30)
                except subprocess.TimeoutExpired:
                    rc = 124
                return idle_cause, rc
        return None, (rc if rc is not None else 1)


# -- event log parsing -----------------------------------------------------------


def parse_events(log_path: Path) -> dict:
    steps = 0
    in_tokens = 0
    cached = 0
    out_tokens = 0
    cost = 0.0
    last_text = ""
    last_reason: str | None = None
    saw_step = False
    raw_chunks: list[str] = []
    try:
        blob = log_path.read_text(encoding="utf-8", errors="replace")
    except OSError:
        blob = ""
    for line in blob.splitlines():
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        if not isinstance(event, dict):
            continue
        kind = event.get("type")
        part = event.get("part") or {}
        if kind == "step_start":
            saw_step = True
        elif kind == "step_finish":
            steps += 1
            last_reason = part.get("reason")
            tokens = part.get("tokens") or {}
            cache = tokens.get("cache") or {}
            try:
                in_tokens += int(tokens.get("input", 0))
                cached += int(cache.get("read", 0))
                out_tokens += int(tokens.get("output", 0)) + int(tokens.get("reasoning", 0))
                cost += float(part.get("cost", 0.0))
            except (TypeError, ValueError):
                pass
        elif kind == "text":
            text = part.get("text")
            if isinstance(text, str) and text.strip():
                last_text = text
        if kind == "tool_use":
            state = part.get("state") or {}
            for key in ("error", "output"):
                value = state.get(key)
                if isinstance(value, str) and value.strip():
                    raw_chunks.append(value)
    return {
        "steps": steps,
        "input": in_tokens,
        "cached": cached,
        "output": out_tokens,
        "cost": cost,
        "last_text": last_text,
        "last_reason": last_reason,
        "saw_step": saw_step,
        "raw": blob + "\n" + "\n".join(raw_chunks),
    }


def next_attempt_number(task_dir: Path) -> int:
    existing = 0
    if task_dir.is_dir():
        existing = len([p for p in task_dir.glob("*.jsonl") if p.is_file()])
    return existing + 1


# -- verify ----------------------------------------------------------------------


def verify_prefix(repo: Path) -> list[str]:
    override = os.environ.get("CRPG_VERIFY")
    if override:
        return [sys.executable, override]
    candidate = repo / "tools" / "agents" / "verify_task.py"
    if not candidate.is_file():
        raise Exit(2, "no-verify: tools/agents/verify_task.py is missing")
    return [sys.executable, str(candidate)]


def run_verify(repo: Path, task: str, slot: Path, role: str) -> dict:
    cmd = verify_prefix(repo) + [task, "--slot", str(slot), "--json"]
    if role in ("tests", "grunt"):
        cmd += ["--skip", "gates"]
    try:
        proc = subprocess.run(cmd, cwd=repo, capture_output=True, text=True, timeout=900)
    except (OSError, subprocess.SubprocessError) as error:
        raise Exit(2, f"verify-error: cannot run verifier: {error}")
    try:
        payload = json.loads(proc.stdout.strip())
    except (json.JSONDecodeError, ValueError):
        payload = {"verdict": "ERROR", "error": proc.stdout.strip() or f"exit {proc.returncode}",
                   "checks": [], "files": []}
    if not isinstance(payload, dict):
        payload = {"verdict": "ERROR", "error": "verifier printed no object",
                   "checks": [], "files": []}
    payload["_rc"] = proc.returncode
    return payload


# -- formatting --------------------------------------------------------------------


def human_count(value: int | float) -> str:
    number = float(value)
    if number >= 1_000_000:
        return f"{number / 1_000_000:.1f}".rstrip("0").rstrip(".") + "M"
    if number >= 1_000:
        return f"{number / 1_000:.0f}k"
    if isinstance(value, float) and not value.is_integer():
        return f"{value:.2f}"
    return str(int(number))


def human_elapsed(seconds: float) -> str:
    total = max(0, int(seconds))
    if total < 60:
        return f"{total}s"
    minutes, secs = divmod(total, 60)
    if minutes < 60:
        return f"{minutes}m{secs:02d}s"
    hours, minutes = divmod(minutes, 60)
    return f"{hours}h{minutes:02d}m"


def verdict_line(info: dict) -> str:
    tokens = info["tokens"]
    return (
        f"{info['verdict']} {info['task']} {info['role']} \u00b7 {info['model']} "
        f"\u00b7 attempt {info['attempt']} \u00b7 {info['steps']} steps "
        f"\u00b7 {human_count(tokens['input'])} in "
        f"({human_count(tokens['cached'])} cached) / {human_count(tokens['output'])} out "
        f"\u00b7 ${info['cost']:.2f} \u00b7 {human_elapsed(info['elapsed_s'])}"
    )


def next_line(task: str, verdict: str, cause: str | None, attempt: int) -> str:
    if verdict == "PASS":
        return "next: open the PR"
    if verdict == "FAIL":
        if attempt < 2:
            return f"next: retry with this output (attempt {attempt + 1} of 2 on this rung)"
        return "next: escalate to the next rung (AGENT_TEAM_PLAN section 4.4)"
    if cause in ("quota", "402", "locked"):
        return "next: retry on the next model in the same tier (not an attempt)"
    if cause in ("timeout", "hang", "no-edit", "model"):
        return "next: retry on the next model in the same tier"
    return "next: fix the setup and rerun"


def build_summary(info: dict, verify: dict | None, full: bool) -> list[str]:
    lines = [verdict_line(info)]
    if info.get("cause"):
        lines.append(f"cause: {info['cause']}")
    files = (verify or {}).get("files") or []
    if files:
        bits = [f"{f.get('path')} +{f.get('added', 0)} -{f.get('deleted', 0)}" for f in files]
        lines.append("files: " + ", ".join(bits))
    checks = (verify or {}).get("checks") or []
    if checks:
        bits = []
        for check in checks:
            status = check.get("status", "?")
            piece = f"{check.get('name')}: {status}"
            if status != "ok" and check.get("detail"):
                piece += f" ({check['detail']})"
            bits.append(piece)
        # One scope line carrying every check, as in AGENT_TEAM_PLAN 5.1.
        lines.append("scope: " + " \u00b7 ".join(bits))
    if info["verdict"] == "FAIL" and (verify or {}).get("error"):
        lines.append(f"error: {verify['error']}")
    for check in checks:
        if check.get("status") == "failed":
            lines.append(f"--- {check.get('name')} (last {CHECK_TAIL_LINES} lines) ---")
            output = check.get("output") or []
            lines.extend(output[-CHECK_TAIL_LINES:])
    if info.get("last_text"):
        lines.append(f"--- worker report (first {WORKER_TAIL_LINES} lines) ---")
        lines.extend(info["last_text"].splitlines()[:WORKER_TAIL_LINES])
    lines.append(next_line(info["task"], info["verdict"], info.get("cause"), info["attempt"]))
    if not full and len(lines) > MAX_SUMMARY_LINES:
        trimmed = lines[: MAX_SUMMARY_LINES - 1]
        trimmed.append(f"... truncated to {MAX_SUMMARY_LINES} lines (--full for all {len(lines)})")
        return trimmed
    return lines


# -- result records ------------------------------------------------------------------


def record_path(task_dir: Path, attempt: int, model: str) -> Path:
    return task_dir / f"{attempt}-{sanitize_model(model)}.result.json"


def write_record(path: Path, info: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    payload = {
        "version": RESULT_VERSION,
        "task": info["task"],
        "role": info["role"],
        "model": info["model"],
        "attempt": info["attempt"],
        "started_utc": info["started_utc"],
        "elapsed_s": round(info["elapsed_s"], 1),
        "verdict": info["verdict"],
        "cause": info.get("cause"),
        "failed_checks": info.get("failed_checks", []),
        "steps": info["steps"],
        "tokens": info["tokens"],
        "cost": round(info["cost"], 4),
        "retries": info.get("retries", []),
        "outcomes": info.get("outcomes", []),
    }
    path.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")


def load_records(repo: Path) -> list[dict]:
    records: list[dict] = []
    root = runs_dir(repo)
    if not root.is_dir():
        return records
    for path in sorted(root.glob("*/*.result.json")):
        try:
            records.append(json.loads(path.read_text(encoding="utf-8")))
        except (OSError, json.JSONDecodeError, ValueError):
            continue
    return records


def is_failure(record: dict) -> bool:
    """A run the model is blamed for: verify FAIL, review reject, or a fault infra exit."""
    if record.get("verdict") == "FAIL":
        return True
    for outcome in record.get("outcomes") or []:
        if outcome.get("outcome") == "review-reject":
            return True
    return record.get("cause") in MODEL_FAULT_CAUSES


def first_attempt_eligible(record: dict) -> bool:
    return record.get("attempt") == 1 and record.get("cause") not in NON_FAULT_CAUSES


def first_attempt_pass(record: dict) -> bool:
    if not first_attempt_eligible(record) or record.get("verdict") != "PASS":
        return False
    for outcome in record.get("outcomes") or []:
        if outcome.get("outcome") in ("review-fix", "review-reject"):
            return False
    return True


# -- run -----------------------------------------------------------------------------


def do_run(args: argparse.Namespace) -> int:
    repo = repo_root()
    slots = list_slots(slots_dir(repo))
    dry = bool(args.dry_run)

    slot_number: int | None = None
    slot: Path | None = None
    if not dry:
        slot_number, slot = acquire_slot(repo, slots, args.slot, wait=False)
    else:
        if args.slot is not None:
            wanted = [entry for entry in slots if entry[0] == args.slot]
            if not wanted:
                raise Exit(2, f"no-slot: slot-{args.slot} does not exist")
            slot_number, slot = wanted[0]
        elif slots:
            slot_number, slot = slots[0]
        else:
            raise Exit(2, "no-slot: no slot worktrees found")

    task_dir = runs_dir(repo) / args.task
    attempt = next_attempt_number(task_dir)
    log_path = task_dir / f"{attempt}-{sanitize_model(args.model)}.jsonl"
    branch = f"task/{args.task}-{args.slug}" if args.slug else f"task/{args.task}"
    base = args.base or "origin/master"
    started = utcnow()
    wall_start = time.time()
    timeout_s = args.timeout if args.timeout is not None else DEFAULT_TIMEOUT
    idle_s = args.idle if args.idle is not None else DEFAULT_IDLE

    info: dict = {
        "task": args.task,
        "role": args.role,
        "model": args.model,
        "attempt": attempt,
        "started_utc": started,
        "elapsed_s": 0.0,
        "verdict": "INFRA",
        "cause": None,
        "failed_checks": [],
        "steps": 0,
        "tokens": {"input": 0, "cached": 0, "output": 0},
        "cost": 0.0,
        "retries": [],
        "outcomes": [],
        "last_text": "",
    }

    def finish(code: int, verify: dict | None) -> int:
        info["elapsed_s"] = time.time() - wall_start
        lines = build_summary(info, verify, bool(args.full))
        payload = {
            "tool": "oc",
            "version": RESULT_VERSION,
            "verdict": info["verdict"],
            "task": info["task"],
            "role": info["role"],
            "model": info["model"],
            "attempt": info["attempt"],
            "cause": info["cause"],
            "failed_checks": info["failed_checks"],
            "steps": info["steps"],
            "tokens": info["tokens"],
            "cost": info["cost"],
            "elapsed_s": round(info["elapsed_s"], 1),
            "files": (verify or {}).get("files") or [],
            "checks": (verify or {}).get("checks") or [],
            "summary": "\n".join(lines),
        }
        if slot_number is not None and not dry:
            release_slot(repo, slot_number)
        if not dry:
            write_record(record_path(task_dir, attempt, args.model), info)
        capture = getattr(args, "_capture", None)
        if capture is not None:
            # A wave member: hand the payload to the owner thread instead
            # of printing, since redirecting sys.stdout is process-global
            # and races with the other members.
            capture.append(payload)
        elif args.json:
            print(json.dumps(payload, indent=2))
        else:
            for line in lines:
                print(line)
        return code

    prefix = opencode_prefix()
    variant = args.variant

    def worker_cmd(prompt: str, use_variant: str | None) -> list[str]:
        cmd = prefix + [
            "run",
            "--agent", f"crpg-{args.role}",
            "-m", args.model,
            "--dir", str(slot),
            "--format", "json",
        ]
        if use_variant:
            cmd += ["--variant", use_variant]
        return cmd + [prompt]

    prompt = PROMPT
    skeleton = [
        verdict_line({**info, "steps": 0, "elapsed_s": 0.0}),
        "files: ...",
        "checks: ...",
        "next: ...",
    ]
    if dry:
        print(f"DRY-RUN {args.task} {args.role} \u00b7 {args.model} \u00b7 slot-{slot_number}")
        print("command: " + " ".join(worker_cmd(prompt, variant)))
        for line in skeleton:
            print(line)
        return 0

    assert slot is not None and slot_number is not None
    try:
        # -- prepare the slot worktree -------------------------------------
        fetch_base(slot, base)
        if attempt == 1 or args.fresh:
            proc = git(["checkout", "-B", branch, base], slot)
            if proc.returncode != 0:
                raise Exit(2, f"branch: cannot create {branch} from {base}: {proc.stderr.strip()}")
            git(["clean", "-ffdx", "-e", "target/"], slot)
        else:
            if current_branch(slot) != branch:
                raise Exit(2, f"branch: slot is not on {branch}")
        prepare_brief(repo, slot, args.task, attempt, args.brief, args.notes)

        # -- start the worker ----------------------------------------------
        wait_start_gap(repo)
        events = {"steps": 0, "input": 0, "cached": 0, "output": 0, "cost": 0.0,
                  "last_text": "", "last_reason": None, "saw_step": False, "raw": ""}
        retries = 0
        while True:
            idle_or_timeout, rc = run_worker(
                worker_cmd(prompt, variant), slot, log_path, timeout_s, idle_s,
                f"crpg-{args.role}", args.model,
            )
            events = parse_events(log_path)
            if idle_or_timeout in ("timeout", "hang", "quota", "402", "model"):
                info.update(steps=events["steps"],
                            tokens={"input": events["input"], "cached": events["cached"],
                                    "output": events["output"]},
                            cost=events["cost"], last_text=events["last_text"])
                info["cause"] = idle_or_timeout
                if idle_or_timeout in MODEL_FAULT_CAUSES:
                    info["failed_checks"] = [idle_or_timeout]
                return finish(2, None)
            if "database is locked" in events["raw"].lower() and not events["saw_step"]:
                if retries < LOCK_RETRIES:
                    retries += 1
                    info["retries"].append({"cause": "locked", "variant": variant})
                    time.sleep(lock_wait())
                    continue
                info.update(steps=events["steps"],
                            tokens={"input": events["input"], "cached": events["cached"],
                                    "output": events["output"]},
                            cost=events["cost"], last_text=events["last_text"])
                info["cause"] = "locked"
                return finish(2, None)
            cause = classify_infra(events["raw"])
            if cause:
                info.update(steps=events["steps"],
                            tokens={"input": events["input"], "cached": events["cached"],
                                    "output": events["output"]},
                            cost=events["cost"], last_text=events["last_text"])
                info["cause"] = cause
                if cause in MODEL_FAULT_CAUSES:
                    info["failed_checks"] = [cause]
                return finish(2, None)
            if events.get("last_reason") == "length" and not slot_edited(slot, base):
                if not any(r.get("cause") == "stall" for r in info["retries"]):
                    info["retries"].append({"cause": "stall", "variant": STALL_VARIANT})
                    prompt = f"{PROMPT} {STALL_SENTENCE}"
                    variant = STALL_VARIANT
                    wait_start_gap(repo)
                    continue
                info.update(steps=events["steps"],
                            tokens={"input": events["input"], "cached": events["cached"],
                                    "output": events["output"]},
                            cost=events["cost"], last_text=events["last_text"])
                info["cause"] = "no-edit"
                info["failed_checks"] = ["no-edit"]
                return finish(2, None)
            if rc != 0 and not events["last_text"]:
                info.update(steps=events["steps"],
                            tokens={"input": events["input"], "cached": events["cached"],
                                    "output": events["output"]},
                            cost=events["cost"], last_text=events["last_text"])
                info["cause"] = "error"
                return finish(2, None)
            break

        info.update(steps=events["steps"],
                    tokens={"input": events["input"], "cached": events["cached"],
                            "output": events["output"]},
                    cost=events["cost"], last_text=events["last_text"])

        # -- verify ---------------------------------------------------------
        if args.role in VERIFY_ROLES:
            verify = run_verify(repo, args.task, slot, args.role)
            verdict = verify.get("verdict")
            if verdict == "PASS":
                info["verdict"] = "PASS"
                return finish(0, verify)
            if verdict == "FAIL":
                info["verdict"] = "FAIL"
                info["failed_checks"] = [
                    c.get("name") for c in verify.get("checks", [])
                    if c.get("status") == "failed"
                ] or ["verify"]
                return finish(1, verify)
            info["verdict"] = "INFRA"
            info["cause"] = "verify-error"
            info["failed_checks"] = ["verify"]
            return finish(2, verify)
        if events["last_text"]:
            info["verdict"] = "PASS"
            return finish(0, None)
        info["verdict"] = "FAIL"
        info["cause"] = "no-text"
        info["failed_checks"] = ["worker text"]
        return finish(1, None)
    except Exit as stop:
        info["cause"] = stop.cause or info.get("cause")
        if stop.cause:
            eprint(f"oc: {stop.cause}")
        if info["cause"] in MODEL_FAULT_CAUSES and not info["failed_checks"]:
            info["failed_checks"] = [info["cause"]]
        code = stop.code
        if code == 1 and info["verdict"] != "FAIL":
            info["verdict"] = "FAIL"
        return finish(code, None)
    except Exception as error:  # noqa: BLE001 - never leave a slot locked
        if slot_number is not None:
            release_slot(repo, slot_number)
        eprint(f"oc: internal error: {error}")
        return 2


# -- ask -----------------------------------------------------------------------------


def do_ask(args: argparse.Namespace) -> int:
    repo = repo_root()
    task = "ask"
    task_dir = runs_dir(repo) / task
    attempt = next_attempt_number(task_dir)
    log_path = task_dir / f"{attempt}-{sanitize_model(args.model)}.jsonl"
    started = utcnow()
    wall_start = time.time()
    timeout_s = args.timeout if args.timeout is not None else DEFAULT_TIMEOUT
    idle_s = args.idle if args.idle is not None else DEFAULT_IDLE
    agent = f"crpg-{args.role}"
    question = " ".join(args.question)

    info: dict = {
        "task": task,
        "role": args.role,
        "model": args.model,
        "attempt": attempt,
        "started_utc": started,
        "elapsed_s": 0.0,
        "verdict": "INFRA",
        "cause": None,
        "failed_checks": [],
        "steps": 0,
        "tokens": {"input": 0, "cached": 0, "output": 0},
        "cost": 0.0,
        "retries": [],
        "outcomes": [],
        "last_text": "",
    }

    def finish(code: int) -> int:
        info["elapsed_s"] = time.time() - wall_start
        lines = build_summary(info, None, bool(args.full))
        payload = {
            "tool": "oc",
            "version": RESULT_VERSION,
            "verdict": info["verdict"],
            "task": task,
            "role": args.role,
            "model": args.model,
            "attempt": attempt,
            "cause": info["cause"],
            "failed_checks": info["failed_checks"],
            "steps": info["steps"],
            "tokens": info["tokens"],
            "cost": info["cost"],
            "elapsed_s": round(info["elapsed_s"], 1),
            "summary": "\n".join(lines),
        }
        write_record(record_path(task_dir, attempt, args.model), info)
        if args.json:
            print(json.dumps(payload, indent=2))
        else:
            for line in lines:
                print(line)
        return code

    prefix = opencode_prefix()
    cmd = prefix + [
        "run",
        "--agent", agent,
        "-m", args.model,
        "--dir", str(repo),
        "--format", "json",
        question,
    ]
    try:
        wait_start_gap(repo)
        idle_or_timeout, rc = run_worker(cmd, repo, log_path, timeout_s, idle_s, agent, args.model)
        events = parse_events(log_path)
        info.update(steps=events["steps"],
                    tokens={"input": events["input"], "cached": events["cached"],
                            "output": events["output"]},
                    cost=events["cost"], last_text=events["last_text"])
        if idle_or_timeout is not None:
            info["cause"] = idle_or_timeout
            if idle_or_timeout in MODEL_FAULT_CAUSES:
                info["failed_checks"] = [idle_or_timeout]
            return finish(2)
        cause = classify_infra(events["raw"])
        if cause:
            info["cause"] = cause
            if cause in MODEL_FAULT_CAUSES:
                info["failed_checks"] = [cause]
            return finish(2)
        if events["last_text"]:
            info["verdict"] = "PASS"
            return finish(0)
        info["verdict"] = "FAIL"
        info["cause"] = "no-text"
        info["failed_checks"] = ["worker text"]
        return finish(1)
    except Exit as stop:
        info["cause"] = stop.cause or info.get("cause")
        return finish(stop.code)


# -- wave --------------------------------------------------------------------------------


def parse_spec(text: str) -> tuple[str, str, str]:
    parts = text.split(":")
    if len(parts) != 3 or not all(parts):
        raise Exit(2, f"usage: wave SPEC must be ID:ROLE:MODEL, got {text!r}")
    task, role, model = parts
    if role not in ROLES:
        raise Exit(2, f"usage: unknown role {role!r}")
    return task, role, model


def do_wave(args: argparse.Namespace) -> int:
    repo = repo_root()
    members = [parse_spec(spec) for spec in args.specs]
    slots = list_slots(slots_dir(repo))
    if not slots:
        raise Exit(2, "no-slot: no slot worktrees found")
    timeout_s = args.timeout if args.timeout is not None else DEFAULT_TIMEOUT

    results: list[dict] = [{} for _ in members]
    held: list[tuple[int, int]] = []
    held_lock = threading.Lock()

    def member_job(index: int, task: str, role: str, model: str) -> None:
        number, _slot = acquire_slot(repo, slots, None, wait=True)
        with held_lock:
            held.append((index, number))
        try:
            sub = argparse.Namespace(
                task=task,
                role=role,
                model=model,
                slot=number,
                timeout=timeout_s,
                idle=DEFAULT_IDLE,
                variant=None,
                brief=None,
                notes=None,
                slug=None,
                base=args.base,
                fresh=False,
                dry_run=False,
                json=True,
                full=False,
                _capture=[],
            )
            code = do_run(sub)
            if sub._capture:
                payload = sub._capture[0]
            else:
                payload = {"verdict": "INFRA", "cause": "error", "attempt": 0,
                           "failed_checks": [], "summary": ""}
            payload["code"] = code
            results[index] = payload
        finally:
            release_slot(repo, number)

    workers = min(len(slots), len(members))
    threads = [
        threading.Thread(target=member_job, args=(i, *members[i]), daemon=True)
        for i in range(len(members))
    ]
    running: list[threading.Thread] = []
    cursor = 0
    while cursor < len(threads) or any(t.is_alive() for t in running):
        while cursor < len(threads) and len([t for t in running if t.is_alive()]) < workers:
            threads[cursor].start()
            running.append(threads[cursor])
            cursor += 1
        time.sleep(0.1)
    for thread in threads:
        thread.join()

    worst = 0
    lines: list[str] = []
    for (task, role, model), payload in zip(members, results):
        code = int(payload.get("code", 2))
        worst = max(worst, code)
        verdict = payload.get("verdict", "INFRA")
        attempt = payload.get("attempt", 0)
        cause = payload.get("cause")
        failing = ", ".join(payload.get("failed_checks") or [])
        tail = cause or failing or "-"
        lines.append(f"{verdict} {task} {role} \u00b7 {model} \u00b7 attempt {attempt} \u00b7 {tail}")
    for (task, _role, _model), payload in zip(members, results):
        attempt = payload.get("attempt", 0)
        lines.append(next_line(task, payload.get("verdict", "INFRA"),
                              payload.get("cause"), attempt) + f" ({task})")
    if args.json:
        print(json.dumps({"tool": "oc", "version": RESULT_VERSION,
                          "members": results, "exit": worst}, indent=2))
        return worst
    if len(lines) > MAX_SUMMARY_LINES:
        lines = lines[: MAX_SUMMARY_LINES - 1] + ["... truncated to 40 lines"]
    for line in lines:
        print(line)
    return worst


# -- record --------------------------------------------------------------------------------


def find_record(task_dir: Path, model: str, attempt: int | None) -> Path | None:
    name = sanitize_model(model)
    if attempt is not None:
        candidate = task_dir / f"{attempt}-{name}.result.json"
        return candidate if candidate.is_file() else None
    matches = sorted(task_dir.glob(f"*-{name}.result.json")) if task_dir.is_dir() else []
    return matches[-1] if matches else None


def do_record(args: argparse.Namespace) -> int:
    repo = repo_root()
    task_dir = runs_dir(repo) / args.task
    target = find_record(task_dir, args.model, args.attempt)
    entry = {"outcome": args.outcome, "note": args.note or "", "utc": utcnow()}
    if target is not None:
        try:
            payload = json.loads(target.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError, ValueError) as error:
            raise Exit(2, f"record: cannot read {target}: {error}")
        outcomes = payload.get("outcomes") or []
        outcomes.append(entry)
        payload["outcomes"] = outcomes
        target.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")
        print(f"recorded {args.outcome} on {target.name}")
        return 0
    attempt = args.attempt or 1
    verdict = "PASS" if args.outcome in PASS_OUTCOMES else "FAIL"
    payload = {
        "version": RESULT_VERSION,
        "task": args.task,
        "role": args.role,
        "model": args.model,
        "attempt": attempt,
        "started_utc": utcnow(),
        "elapsed_s": 0.0,
        "verdict": verdict,
        "cause": None,
        "failed_checks": [],
        "steps": 0,
        "tokens": {"input": 0, "cached": 0, "output": 0},
        "cost": 0.0,
        "retries": [],
        "outcomes": [entry],
    }
    task_dir.mkdir(parents=True, exist_ok=True)
    fresh = record_path(task_dir, attempt, args.model)
    fresh.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")
    print(f"recorded {args.outcome} on {fresh.name} (new minimal record, verdict {verdict})")
    return 0


# -- stats ----------------------------------------------------------------------------------


def stats_table(repo: Path, role: str | None, since: str | None) -> list[dict]:
    records = load_records(repo)
    if role is not None:
        records = [r for r in records if r.get("role") == role]
    if since is not None:
        records = [r for r in records if str(r.get("started_utc", "")) >= since]
    groups: dict[tuple[str, str], list[dict]] = {}
    for record in records:
        key = (str(record.get("model")), str(record.get("role")))
        groups.setdefault(key, []).append(record)
    rows: list[dict] = []
    for (model, group_role), items in groups.items():
        items.sort(key=lambda r: str(r.get("started_utc", "")))
        firsts = [r for r in items if r.get("attempt") == 1]
        eligible = [r for r in firsts if first_attempt_eligible(r)]
        passes = sum(1 for r in eligible if first_attempt_pass(r))
        rate = (passes / len(eligible)) if eligible else 0.0
        recent = items[-DEMOTE_WINDOW:]
        failures = sum(1 for r in recent if is_failure(r))
        infra: dict[str, int] = {}
        for item in items:
            if item.get("verdict") == "INFRA":
                cause = str(item.get("cause") or "unknown")
                infra[cause] = infra.get(cause, 0) + 1
        steps = [int(item.get("steps", 0)) for item in items]
        elapsed = [float(item.get("elapsed_s", 0.0)) for item in items]
        rows.append({
            "model": model,
            "role": group_role,
            "runs": len(items),
            "first_attempt_pass_rate": round(rate, 3),
            "first_attempt_passes": passes,
            "first_attempt_eligible": len(eligible),
            "verify_failures": sum(1 for r in items if r.get("verdict") == "FAIL"),
            "review_rejects": sum(
                1 for r in items for o in (r.get("outcomes") or [])
                if o.get("outcome") == "review-reject"
            ),
            "infra": infra,
            "median_steps": statistics.median(steps) if steps else 0,
            "median_elapsed_s": statistics.median(elapsed) if elapsed else 0,
            "total_cost": round(sum(float(r.get("cost", 0.0)) for r in items), 4),
            "demote": failures >= DEMOTE_FAILURES,
            "promote": False,
        })
    rows.sort(key=lambda r: (-r["first_attempt_pass_rate"], -r["runs"], r["model"]))
    contenders = [r for r in rows if r["runs"] >= PROMOTE_MIN_RUNS]
    if contenders:
        best = max(r["first_attempt_pass_rate"] for r in contenders)
        for row in contenders:
            if row["first_attempt_pass_rate"] == best:
                row["promote"] = True
    return rows


def stats_lines(rows: list[dict]) -> list[str]:
    lines = ["model x role | runs | 1st-pass | fail | reject | infra | steps~ elapsed~ | cost"]
    for row in rows:
        infra = ",".join(f"{k}:{v}" for k, v in sorted(row["infra"].items())) or "-"
        mark = ""
        if row["demote"]:
            mark += " DEMOTE?"
        if row["promote"]:
            mark += " PROMOTE?"
        lines.append(
            f"{row['model']} x {row['role']} | runs {row['runs']} | "
            f"1st-pass {row['first_attempt_pass_rate']:.0%} "
            f"({row['first_attempt_passes']}/{row['first_attempt_eligible']}) | "
            f"fail {row['verify_failures']} | reject {row['review_rejects']} | "
            f"infra {infra} | steps~{row['median_steps']} "
            f"{human_elapsed(float(row['median_elapsed_s']))} | "
            f"${row['total_cost']:.2f}{mark}"
        )
    if len(lines) > MAX_SUMMARY_LINES:
        lines = lines[: MAX_SUMMARY_LINES - 1] + ["... truncated to 40 lines"]
    return lines


def do_stats(args: argparse.Namespace) -> int:
    repo = repo_root()
    if args.since is not None:
        try:
            datetime.strptime(args.since, "%Y-%m-%d")
        except ValueError:
            raise Exit(2, f"usage: --since must be YYYY-MM-DD, got {args.since!r}")
    rows = stats_table(repo, args.role, args.since)
    if args.json:
        print(json.dumps({"tool": "oc", "version": RESULT_VERSION, "rows": rows}, indent=2))
    else:
        for line in stats_lines(rows):
            print(line)
    return 0


# -- main -------------------------------------------------------------------------------------


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(prog="oc.py", description="OpenCode dispatcher")
    sub = parser.add_subparsers(dest="command", required=True)

    run = sub.add_parser("run", help="run one worker")
    run.add_argument("--task", required=True)
    run.add_argument("--role", required=True, choices=ROLES)
    run.add_argument("--model", required=True)
    run.add_argument("--slot", type=int, default=None)
    run.add_argument("--timeout", type=int, default=None)
    run.add_argument("--idle", type=int, default=None)
    run.add_argument("--variant", default=None)
    run.add_argument("--brief", default=None)
    run.add_argument("--notes", default=None)
    run.add_argument("--slug", default=None)
    run.add_argument("--base", default=None)
    run.add_argument("--fresh", action="store_true")
    run.add_argument("--dry-run", action="store_true")
    run.add_argument("--json", action="store_true")
    run.add_argument("--full", action="store_true")

    wave = sub.add_parser("wave", help="run several workers in parallel")
    wave.add_argument("specs", nargs="+", help="ID:ROLE:MODEL")
    wave.add_argument("--timeout", type=int, default=None)
    wave.add_argument("--base", default=None)
    wave.add_argument("--json", action="store_true")

    ask = sub.add_parser("ask", help="ask a question, no slot, no verify")
    ask.add_argument("--role", required=True, choices=ROLES)
    ask.add_argument("--model", required=True)
    ask.add_argument("--timeout", type=int, default=None)
    ask.add_argument("--idle", type=int, default=None)
    ask.add_argument("--json", action="store_true")
    ask.add_argument("--full", action="store_true")
    ask.add_argument("question", nargs="+")

    record = sub.add_parser("record", help="append a review/merge outcome")
    record.add_argument("--task", required=True)
    record.add_argument("--role", required=True, choices=ROLES)
    record.add_argument("--model", required=True)
    record.add_argument("--outcome", required=True, choices=REVIEW_OUTCOMES)
    record.add_argument("--attempt", type=int, default=None)
    record.add_argument("--note", default=None)

    stats = sub.add_parser("stats", help="rank models per role")
    stats.add_argument("--role", default=None, choices=ROLES)
    stats.add_argument("--since", default=None)
    stats.add_argument("--json", action="store_true")
    return parser


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    try:
        if args.command == "run":
            return do_run(args)
        if args.command == "wave":
            return do_wave(args)
        if args.command == "ask":
            return do_ask(args)
        if args.command == "record":
            return do_record(args)
        if args.command == "stats":
            return do_stats(args)
    except Exit as stop:
        if stop.cause:
            eprint(f"oc: {stop.cause}")
        return stop.code
    raise Exit(2, "usage: unknown command")


if __name__ == "__main__":
    sys.exit(main())

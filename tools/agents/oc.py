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
WORKER_TAIL_LINES = 14
CHECK_TAIL_LINES = 12

# Thresholds from docs/agents/MODELS.md section 4.1 (demote and promote rule).
DEMOTE_WINDOW = 4
DEMOTE_FAILURES = 2
PROMOTE_MIN_RUNS = 5

# Infra causes that are not the model's fault: they never count against it.
NON_FAULT_CAUSES = frozenset({"quota", "402", "locked"})
# Infra causes that are the model's fault: they count against it.
MODEL_FAULT_CAUSES = frozenset({"no-edit", "timeout", "hang"})

# docs/agents/MODELS.md section 4: the Zen LongCat ID now lives on the Go
# subscription, so the Zen form is refused with a pointer at the Go form.
# Cat Jam came back after the OpenCode 2.x upgrade as Beam Free (the user,
# 2026-10-16), so both Cat Jam IDs and Beam's Zen ID name the Go Beam ID.
RENAMED_MODELS = {
    "opencode/longcat-2.5-preview-free": "opencode-go/longcat-2.5-preview-free",
    "opencode/beam-free": "opencode-go/beam-free",
    "opencode/cat-jam-free": "opencode-go/beam-free",
    "opencode-go/cat-jam-free": "opencode-go/beam-free",
}

# Wave concurrency caps from docs/agents/MODELS.md sections 1 and 4
# (manager routing rules): free Zen models serialize per model and run at
# most three at once; any one model runs at most three at once.
FREE_ZEN_PREFIX = "opencode/"
FREE_ZEN_PER_MODEL = 1
FREE_ZEN_TOTAL = 3
PER_MODEL_MAX = 3

# A synthesized result record (from `record` with no matching run) carries
# this cause so stats can tell "the manager wrote this down" apart from a
# real verifier or infra failure (docs/agents/MODELS.md section 4.1).
RECORDED_CAUSE = "recorded"

# A brief that was never filled in still carries the manager template's
# placeholder marker; running a worker on it is refused as no-brief.
BRIEF_PLACEHOLDER = "<MANAGER:"

# A slot prepared from a base that predates A0-2 has no agent declaration,
# so OpenCode cannot start `crpg-<role>` at all (it fails in seconds with
# `Agent not found`). oc.py copies the declaration and `opencode.json` from
# the repository root into such a slot, and lists the copies in the
# worktree's `info/exclude` so the verifier's scope check never counts them
# as the worker's changes.
OPENCODE_JSON = Path("opencode.json")
AGENTS_DIR = Path(".opencode") / "agents"


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
        exe2 = Path(appdata) / "npm" / "node_modules" / "@opencode" / "cli" / "bin" / "opencode.exe"
        if exe2.is_file():
            return [str(exe2)]
        exe1 = Path(appdata) / "npm" / "node_modules" / "opencode-ai" / "bin" / "opencode.exe"
        if exe1.is_file():
            return [str(exe1)]
    found = shutil.which("opencode")
    if found and not found.lower().endswith(".cmd"):
        return [found]
    if found:
        # A .cmd shim would parse < > | & inside the prompt through cmd.exe.
        sibling = Path(found).with_suffix(".exe")
        alts = (
            sibling,
            Path(found).parent / "node_modules" / "@opencode" / "cli" / "bin" / "opencode.exe",
            Path(found).parent / "node_modules" / "opencode-ai" / "bin" / "opencode.exe",
        )
        for candidate in alts:
            if candidate.is_file():
                return [str(candidate)]
        raise Exit(2, "no-exe: only an opencode.cmd shim is on PATH")
    raise Exit(2, "no-exe: opencode not found")


_VERSION_CACHE: dict[tuple[str, ...], int] = {}


def opencode_major(prefix: list[str]) -> int:
    """Major version of the OpenCode executable (1 or 2), cached per process.

    Probes once with `<exe> --version` (e.g. `opencode v2.0.31`); a `.py`
    fake answers it too. Unparseable output or a failed probe means 2.x,
    the version installed on this machine since 2026-10-16.
    """
    key = tuple(prefix)
    if key in _VERSION_CACHE:
        return _VERSION_CACHE[key]
    major = 2
    try:
        proc = subprocess.run(
            [*prefix, "--version"],
            capture_output=True,
            text=True,
            timeout=15,
        )
        blob = (proc.stdout or "") + "\n" + (proc.stderr or "")
        match = re.search(r"(\d+)\.", blob)
        if match:
            major = int(match.group(1))
    except (OSError, subprocess.SubprocessError, ValueError):
        major = 2
    _VERSION_CACHE[key] = major
    return major


def model_flag(model: str, variant: str | None, major: int) -> list[str]:
    """The `-m` flag for this executable generation: 2.x carries a variant
    as `model#variant`; 1.x keeps a separate `--variant` flag."""
    if major >= 2:
        return ["-m", f"{model}#{variant}" if variant else model]
    if variant:
        return ["-m", model, "--variant", variant]
    return ["-m", model]


# -- slot locks ------------------------------------------------------------


def _win_kernel32():
    """kernel32 with declared prototypes, so 64-bit handles are not cut down.

    HANDLE is c_void_p and a DWORD is c_uint32: without these declarations
    ctypes guesses c_int and truncates a 64-bit handle on the way out.
    """
    import ctypes
    from ctypes import wintypes

    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel32.OpenProcess.restype = wintypes.HANDLE
    kernel32.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
    kernel32.GetExitCodeProcess.restype = wintypes.BOOL
    kernel32.GetExitCodeProcess.argtypes = [
        wintypes.HANDLE, ctypes.POINTER(wintypes.DWORD)
    ]
    kernel32.GetLastError.restype = wintypes.DWORD
    kernel32.CloseHandle.restype = wintypes.BOOL
    kernel32.CloseHandle.argtypes = [wintypes.HANDLE]
    return kernel32


def _win_pid_alive(pid: int, kernel32) -> bool:
    """Windows liveness from a kernel32-shaped object; alive when in doubt.

    A NULL OpenProcess means dead only for ERROR_INVALID_PARAMETER (87, no
    such process). ERROR_ACCESS_DENIED (5) is a live process we may not
    open, and any other code is uncertain, so both count as alive. A failed
    GetExitCodeProcess is uncertain too. Never signals or kills.
    """
    import ctypes

    SYNCHRONIZE = 0x00100000
    PROCESS_QUERY_LIMITED_INFORMATION = 0x1000
    STILL_ACTIVE = 259
    ERROR_INVALID_PARAMETER = 87

    handle = kernel32.OpenProcess(
        SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION, False, pid
    )
    if not handle:
        try:
            error = int(kernel32.GetLastError())
        except Exception:  # noqa: BLE001 - no error code at all: assume alive
            return True
        return error != ERROR_INVALID_PARAMETER
    try:
        code = ctypes.c_uint32()
        if not kernel32.GetExitCodeProcess(handle, ctypes.byref(code)):
            return True
        return code.value == STILL_ACTIVE
    finally:
        try:
            kernel32.CloseHandle(handle)
        except Exception:  # noqa: BLE001 - a handle we cannot close still probed
            pass


def _pid_alive(pid: int) -> bool:
    """True when a process with this PID exists. Never signals or kills.

    os.kill(pid, 0) is a probe on POSIX but not on Windows, where signal 0
    is CTRL_C_EVENT and other values terminate, so Windows uses a read-only
    OpenProcess + GetExitCodeProcess check (stdlib ctypes) with a parsing
    tasklist as fallback. When in doubt the PID counts as alive, so a lock
    whose owner cannot be proven dead is never treated as stale.
    """
    if pid <= 0:
        return False
    if os.name == "nt":
        try:
            return _win_pid_alive(pid, _win_kernel32())
        except Exception:  # noqa: BLE001 - fall back to tasklist below
            pass
        try:
            proc = subprocess.run(
                ["tasklist", "/FI", f"PID eq {pid}", "/NH"],
                capture_output=True,
                text=True,
                timeout=15,
            )
            return re.search(rf"\b{pid}\b", proc.stdout) is not None
        except (OSError, subprocess.SubprocessError):
            return True
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    except (PermissionError, OSError):
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
            fd = None
            for _ in range(2):
                # One stale retry: a reclaimed lock is taken here, on this
                # slot, not skipped for the next candidate.
                try:
                    fd = os.open(str(lock), os.O_CREAT | os.O_EXCL | os.O_WRONLY)
                    break
                except FileExistsError:
                    pid = _read_lock_pid(lock)
                    # A dead owner, or a lock with no parsable PID at all,
                    # is stale: reclaim it and retry this same slot.
                    if pid is not None and _pid_alive(pid):
                        break
                    try:
                        lock.unlink()
                    except OSError:
                        break
                    continue
                except OSError:
                    break
            if fd is None:
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


def brief_generator(repo: Path) -> Path:
    generator = repo / "tools" / "agents" / "brief.py"
    if not generator.is_file():
        raise Exit(2, "no-brief: no --brief given and tools/agents/brief.py is missing")
    return generator


def check_brief_text(text: str, origin: str) -> None:
    """Refuse a brief that was never filled in (exit 2 no-brief)."""
    if BRIEF_PLACEHOLDER in text:
        raise Exit(2, f"no-brief: {origin} has an unfilled placeholder ({BRIEF_PLACEHOLDER}...)")


def resolve_brief_source(repo: Path, task: str,
                         brief: str | None) -> tuple[str, Path | None]:
    """Where this run's brief comes from: --brief, else the filed brief for
    the task, else the brief.py generator. Returns ("file", path) or
    ("generator", None). File sources are placeholder-checked here; the
    generator's output is checked after it runs. Raises Exit(2) no-brief
    when nothing provides a brief, so --dry-run validates like a real run.
    """
    if brief is not None:
        source = Path(brief)
        if not source.is_file():
            raise Exit(2, f"no-brief: {brief} does not exist")
        check_brief_text(source.read_text(encoding="utf-8", errors="replace"), str(source))
        return ("file", source)
    filed = repo / ".agents" / "briefs" / f"{task}.md"
    if filed.is_file():
        check_brief_text(filed.read_text(encoding="utf-8", errors="replace"), str(filed))
        return ("file", filed)
    brief_generator(repo)
    return ("generator", None)


def run_brief_generator(repo: Path, task: str, dest: Path) -> None:
    generator = brief_generator(repo)
    proc = subprocess.run(
        [sys.executable, str(generator), task, "--out", str(dest)],
        cwd=repo,
        capture_output=True,
        text=True,
        timeout=120,
    )
    if proc.returncode != 0 or not dest.is_file():
        raise Exit(2, f"no-brief: brief.py failed: {proc.stderr.strip()}")
    check_brief_text(dest.read_text(encoding="utf-8", errors="replace"), str(dest))


def prepare_brief(repo: Path, slot: Path, task: str, attempt: int,
                  brief: str | None, notes: str | None) -> None:
    target = slot / "BRIEF.md"
    kind, source = resolve_brief_source(repo, task, brief)
    if kind == "file":
        assert source is not None
        target.write_bytes(source.read_bytes())
        check_brief_text(target.read_text(encoding="utf-8", errors="replace"), str(target))
    else:
        run_brief_generator(repo, task, target)
    if notes is not None:
        text = Path(notes).read_text(encoding="utf-8") if Path(notes).is_file() else notes
        with target.open("a", encoding="utf-8") as handle:
            handle.write(f"\n## Attempt {attempt} notes\n\n{text}")
            if not text.endswith("\n"):
                handle.write("\n")


def brief_owned_crates(text: str) -> set[str]:
    """Crate names owned by the brief: `crates/<name>/...` paths listed as
    bullets under its `## Files you own` section."""
    crates: set[str] = set()
    in_section = False
    for line in text.splitlines():
        stripped = line.strip()
        if stripped.startswith("##"):
            title = stripped.strip("#").strip().lower()
            in_section = title == "files you own"
            continue
        if not in_section:
            continue
        match = re.match(r"\s*[-*]\s+`?([^`\s]+)`?", line)
        if match:
            parts = match.group(1).strip().split("/")
            if len(parts) >= 2 and parts[0] == "crates":
                crates.add(parts[1])
    return crates


def check_wave_crates(members: list[tuple[str, str, str]],
                      owned: list[set[str]]) -> None:
    """Refuse two wave members owning paths under the same crates/<name>/:
    each crate takes one task at a time (AGENT_TEAM_PLAN section 5.1)."""
    seen: dict[str, str] = {}
    for (task, _role, _model), crates in zip(members, owned):
        for crate in sorted(crates):
            if crate in seen:
                raise Exit(
                    2,
                    f"usage: wave members {seen[crate]} and {task} both own "
                    f"paths under crates/{crate}/",
                )
            seen[crate] = task


def agent_declaration(role: str) -> Path:
    """The OpenCode agent file a `crpg-<role>` worker needs, repo-relative."""
    return AGENTS_DIR / f"crpg-{role}.md"


def exclude_path(slot: Path) -> Path:
    """The slot worktree's `info/exclude` file, resolved by git itself."""
    proc = git(["rev-parse", "--git-path", "info/exclude"], slot)
    if proc.returncode != 0:
        raise Exit(2, f"no-agent: cannot locate info/exclude: {proc.stderr.strip()}")
    path = Path(proc.stdout.strip())
    return path if path.is_absolute() else (slot / path)


def add_to_exclude(slot: Path, paths: list[str]) -> None:
    """List `paths` (slot-relative) in the worktree's info/exclude, once each."""
    if not paths:
        return
    target = exclude_path(slot)
    lines: list[str] = []
    if target.is_file():
        lines = target.read_text(encoding="utf-8").splitlines()
    added = False
    for path in paths:
        if path not in lines:
            lines.append(path)
            added = True
    if not added:
        return
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text("\n".join(lines) + "\n", encoding="utf-8")


def ensure_agent_files(repo: Path, slot: Path, role: str) -> list[str]:
    """Copy the role's agent declaration into a slot that lacks it.

    OpenCode reads `opencode.json` and `.opencode/agents/` from the slot, so
    a slot prepared from a base that predates A0-2 cannot start a worker at
    all. Copies come from `repo` (the repository root) and never overwrite a
    file the slot already has. Returns the slot-relative paths copied, for
    the exclude file. Exit 2 `no-agent` when the slot lacks the declaration
    and the repository root cannot supply it either: that is a setup
    failure, and it must be reported before OpenCode starts, not after.
    """
    declaration = agent_declaration(role)
    if (slot / declaration).is_file():
        return []
    if not (repo / declaration).is_file():
        raise Exit(
            2,
            f"no-agent: {declaration.as_posix()} is missing from both the slot "
            f"and {repo}",
        )
    sources = [OPENCODE_JSON, declaration]
    source_agents = repo / AGENTS_DIR
    if source_agents.is_dir():
        for child in sorted(source_agents.rglob("*")):
            if child.is_file():
                relative = AGENTS_DIR / child.relative_to(source_agents)
                if relative not in sources:
                    sources.append(relative)
    copied: list[str] = []
    for relative in sources:
        source = repo / relative
        target = slot / relative
        if not source.is_file() or target.exists():
            continue
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(source.read_bytes())
        copied.append(relative.as_posix())
    return copied


def check_model_id(model: str) -> None:
    """Refuse moved or removed model IDs (exit 2 usage).

    Zen IDs that moved to the Go subscription name the Go form, and Cat
    Jam IDs name Beam, its new name (docs/agents/MODELS.md section 4).
    """
    if model in RENAMED_MODELS:
        raise Exit(
            2,
            f"usage: model {model} moved to {RENAMED_MODELS[model]}; "
            f"use {RENAMED_MODELS[model]} instead",
        )


def wave_fits(model: str, running: list[str]) -> bool:
    """True when one more run of this model may start alongside `running`."""
    if model.startswith(FREE_ZEN_PREFIX):
        if sum(1 for active in running if active == model) >= FREE_ZEN_PER_MODEL:
            return False
        return sum(1 for active in running if active.startswith(FREE_ZEN_PREFIX)) < FREE_ZEN_TOTAL
    return sum(1 for active in running if active == model) < PER_MODEL_MAX


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
    if "agent not found" in lowered:
        return "no-agent"
    if (
        "model not found" in lowered
        or "unknown model" in lowered
        or "provider.no-route" in lowered
        or "model unavailable" in lowered
    ):
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
    infra_chunks: list[str] = []
    error_types: list[str] = []
    try:
        blob = log_path.read_text(encoding="utf-8", errors="replace")
    except OSError:
        blob = ""
    for line in blob.splitlines():
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            # Not an event: raw process output (stdout/stderr), which is
            # where transport errors such as "database is locked" appear.
            if line.strip():
                infra_chunks.append(line)
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
        elif kind == "error":
            infra_chunks.append(line)
            detail = event.get("error")
            if isinstance(detail, dict) and isinstance(detail.get("type"), str):
                error_types.append(detail["type"])
        if kind == "tool_use":
            # A tool error echoes command output, so it is never a
            # provider/process failure and must never be classified as
            # infra (attempt-5 finding 1): only top-level `error` events,
            # the process's own output and the OpenCode log lines carry
            # those. The output is still scanned for the lock marker via
            # `raw`; the error is read by nobody.
            output = (part.get("state") or {}).get("output")
            if isinstance(output, str) and output.strip():
                raw_chunks.append(output)
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
        # Provider/process errors only: error events and raw process output.
        # Never tool outputs or tool errors, and never the worker's text.
        "infra_text": "\n".join(infra_chunks),
        # The `type` of every top-level error event, in order.
        "error_types": error_types,
    }


def infra_cause(events: dict) -> str | None:
    """Classify a finished run's infra cause, or None.

    A run that ended with a normal `stop` finish is never an infra exit,
    whatever words its tool outputs or report contain.
    """
    if events.get("last_reason") == "stop":
        return None
    return classify_infra(events.get("infra_text", ""))


def empty_run_cause(events: dict) -> str:
    """Why a run that changed nothing ended: an infra cause or `no-edit`.

    An `error` event in the log is the provider's failure, not the model's:
    the infra rules classify it first (a rate limit stays `quota`), and any
    other `provider.*` error is `model`. Only a log with no such error is
    the model's own `no-edit`.
    """
    cause = infra_cause(events)
    if cause:
        return cause
    if any(t.startswith("provider.") for t in events.get("error_types") or []):
        return "model"
    return "no-edit"


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


def run_verify(repo: Path, task: str, slot: Path, role: str, base: str) -> dict:
    # The changed-set base is the run's base: a slot prepared from
    # `--base origin/setup/A0-config` must not report that base's own
    # commits as out-of-scope changes. Without --base the verifier falls
    # back to `merge-base HEAD origin/master` (verify_task.py:215).
    cmd = verify_prefix(repo) + [task, "--slot", str(slot), "--base", base, "--json"]
    if role == "grunt":
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


def next_line(task: str, verdict: str, cause: str | None, attempt: int,
              role: str | None = None) -> str:
    if verdict == "PASS":
        # Ask, scout and review runs answer a question or give a verdict;
        # there is no PR step for them.
        if task == "ask" or role == "scout":
            return "next: read the answer"
        if role == "review":
            return "next: act on the verdict"
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
    if info.get("report"):
        lines.append(f"report: {info['report']}")
    if info.get("last_text"):
        lines.append(f"--- worker report (first {WORKER_TAIL_LINES} lines) ---")
        lines.extend(info["last_text"].splitlines()[:WORKER_TAIL_LINES])
    lines.append(next_line(info["task"], info["verdict"], info.get("cause"),
                          info["attempt"], info.get("role")))
    if not full and len(lines) > MAX_SUMMARY_LINES:
        trimmed = lines[: MAX_SUMMARY_LINES - 1]
        trimmed.append(f"... truncated to {MAX_SUMMARY_LINES} lines (--full for all {len(lines)})")
        return trimmed
    return lines


# -- result records ------------------------------------------------------------------


def record_path(task_dir: Path, attempt: int, model: str) -> Path:
    return task_dir / f"{attempt}-{sanitize_model(model)}.result.json"


def report_path(task_dir: Path, attempt: int, model: str) -> Path:
    return task_dir / f"{attempt}-{sanitize_model(model)}.report.md"


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
    """A run the model is blamed for, exactly per MODELS.md section 4.1.

    A failure is a verify FAIL, a review reject, or an infra exit the model
    caused (no-edit, timeout, hang). Anything else is not: a synthesized
    record from `record` (cause "recorded") counts only through an explicit
    review-reject outcome, and a review/scout run that left no text
    (cause "no-text") never counts.
    """
    for outcome in record.get("outcomes") or []:
        if outcome.get("outcome") == "review-reject":
            return True
    if record.get("cause") in MODEL_FAULT_CAUSES:
        return True
    return (
        record.get("verdict") == "FAIL"
        and record.get("cause") not in (RECORDED_CAUSE, "no-text")
    )


def first_attempt_eligible(record: dict) -> bool:
    return record.get("attempt") == 1 and record.get("cause") not in NON_FAULT_CAUSES


def first_attempt_pass(record: dict) -> bool:
    if not first_attempt_eligible(record) or record.get("verdict") != "PASS":
        return False
    for outcome in record.get("outcomes") or []:
        if outcome.get("outcome") in ("review-fix", "review-reject"):
            return False
    return True


# -- shared result-record helpers -------------------------------------------------


def new_info(task: str, role: str, model: str, attempt: int, started: str) -> dict:
    """The blank result record every run fills in (the model scorecard row)."""
    return {
        "task": task, "role": role, "model": model, "attempt": attempt,
        "started_utc": started, "elapsed_s": 0.0, "verdict": "INFRA",
        "cause": None, "failed_checks": [], "steps": 0,
        "tokens": {"input": 0, "cached": 0, "output": 0}, "cost": 0.0,
        "retries": [], "outcomes": [], "last_text": "",
    }


def absorb_events(info: dict, events: dict) -> None:
    """Copy a parsed event log's counters into the result record."""
    info.update(steps=events["steps"],
                tokens={"input": events["input"], "cached": events["cached"],
                        "output": events["output"]},
                cost=events["cost"], last_text=events["last_text"])


def result_payload(info: dict, verify: dict | None, summary: str) -> dict:
    """The --json / wave-capture object for a finished run."""
    return {
        "tool": "oc", "version": RESULT_VERSION, "verdict": info["verdict"],
        "task": info["task"], "role": info["role"], "model": info["model"],
        "attempt": info["attempt"], "cause": info["cause"],
        "failed_checks": info["failed_checks"], "steps": info["steps"],
        "tokens": info["tokens"], "cost": info["cost"],
        "elapsed_s": round(info["elapsed_s"], 1),
        "files": (verify or {}).get("files") or [],
        "checks": (verify or {}).get("checks") or [],
        "summary": summary,
    }


# -- run -----------------------------------------------------------------------------


def do_run(args: argparse.Namespace) -> int:
    repo = repo_root()
    check_model_id(args.model)
    slots = list_slots(slots_dir(repo))
    dry = bool(args.dry_run)

    task_dir = runs_dir(repo) / args.task
    attempt = next_attempt_number(task_dir)
    log_path = task_dir / f"{attempt}-{sanitize_model(args.model)}.jsonl"
    branch = f"task/{args.task}-{args.slug}" if args.slug else f"task/{args.task}"
    base = args.base or "origin/master"
    started = utcnow()
    wall_start = time.time()
    timeout_s = args.timeout if args.timeout is not None else DEFAULT_TIMEOUT
    idle_s = args.idle if args.idle is not None else DEFAULT_IDLE

    info = new_info(args.task, args.role, args.model, attempt, started)

    def finish(code: int, verify: dict | None) -> int:
        info["elapsed_s"] = time.time() - wall_start
        if info.get("last_text") and not dry:
            # The summary shows the head of the report; the file keeps it all.
            report = report_path(task_dir, attempt, args.model)
            report.parent.mkdir(parents=True, exist_ok=True)
            report.write_text(info["last_text"], encoding="utf-8")
            info["report"] = str(report)
        lines = build_summary(info, verify, bool(args.full))
        payload = result_payload(info, verify, "\n".join(lines))
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

    slot_number: int | None = None
    slot: Path | None = None
    if not dry:
        try:
            slot_number, slot = acquire_slot(repo, slots, args.slot, wait=False)
        except Exit as stop:
            # A run that never got a slot still leaves its result record,
            # so the model scorecard sees the infra exit.
            if stop.cause and stop.cause.startswith("slot-busy"):
                info["cause"] = stop.cause
                eprint(f"oc: {stop.cause}")
                return finish(2, None)
            raise
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

    variant = args.variant

    def worker_cmd(prefix: list[str], prompt: str, use_variant: str | None,
                   major: int | None = None) -> list[str]:
        if major is None:
            major = opencode_major(prefix)
        if major >= 2:
            # 2.x has no --dir and no --variant: the workspace is the
            # process cwd plus PWD, a variant is part of the model, and
            # each run gets its own server via --standalone.
            return prefix + [
                "run",
                "--standalone",
                "--agent", f"crpg-{args.role}",
                *model_flag(args.model, use_variant, major),
                "--format", "json",
                prompt,
            ]
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
        # A dry run validates like a real run (brief included) but changes
        # nothing: no lock, no branch move, no log, no result record.
        resolve_brief_source(repo, args.task, args.brief)
        prefix = opencode_prefix()
        major = opencode_major(prefix)
        print(f"DRY-RUN {args.task} {args.role} \u00b7 {args.model} \u00b7 slot-{slot_number}")
        print("command: " + " ".join(worker_cmd(prefix, prompt, variant, major)))
        for line in skeleton:
            print(line)
        return 0

    assert slot is not None and slot_number is not None
    try:
        prefix = opencode_prefix()
        major = opencode_major(prefix)
        # -- prepare the slot worktree -------------------------------------
        fetch_base(slot, base)
        if attempt == 1 or args.fresh:
            proc = git(["checkout", "-B", branch, base], slot)
            if proc.returncode != 0:
                raise Exit(2, f"branch: cannot create {branch} from {base}: {proc.stderr.strip()}")
            proc = git(["clean", "-ffdx", "-e", "target/"], slot)
            if proc.returncode != 0:
                raise Exit(2, f"branch: cannot clean the slot: {proc.stderr.strip()}")
        else:
            if current_branch(slot) != branch:
                raise Exit(2, f"branch: slot is not on {branch}")
        prepare_brief(repo, slot, args.task, attempt, args.brief, args.notes)
        # The slot must be able to start `crpg-<role>` before OpenCode does;
        # a no-agent setup failure is reported here, not as a fake worker
        # error four seconds in. The copies are excluded so the verifier's
        # scope check never counts them as the worker's changes.
        add_to_exclude(slot, ensure_agent_files(repo, slot, args.role))

        # -- start the worker ----------------------------------------------
        wait_start_gap(repo)
        events = {"steps": 0, "input": 0, "cached": 0, "output": 0, "cost": 0.0,
                  "last_text": "", "last_reason": None, "saw_step": False, "raw": ""}
        retries = 0
        while True:
            idle_or_timeout, rc = run_worker(
                worker_cmd(prefix, prompt, variant, major), slot, log_path, timeout_s,
                idle_s, f"crpg-{args.role}", args.model,
            )
            events = parse_events(log_path)
            if idle_or_timeout in ("timeout", "hang", "quota", "402", "model", "no-agent"):
                absorb_events(info, events)
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
                absorb_events(info, events)
                info["cause"] = "locked"
                return finish(2, None)
            cause = infra_cause(events)
            if cause:
                absorb_events(info, events)
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
                absorb_events(info, events)
                info["cause"] = "no-edit"
                info["failed_checks"] = ["no-edit"]
                return finish(2, None)
            if rc != 0 and not events["last_text"]:
                absorb_events(info, events)
                info["cause"] = "error"
                return finish(2, None)
            break

        absorb_events(info, events)

        # -- verify ---------------------------------------------------------
        if args.role in VERIFY_ROLES:
            verify = run_verify(repo, args.task, slot, args.role, base)
            verdict = verify.get("verdict")
            if (verdict == "PASS" and args.role in ("implement", "tests")
                    and not verify.get("files")):
                # Every check passes on an empty change set, so an empty
                # `files` list means the worker changed nothing (or the
                # provider failed under it): not a PASS.
                cause = empty_run_cause(events)
                info["verdict"] = "INFRA"
                info["cause"] = cause
                if cause in MODEL_FAULT_CAUSES:
                    info["failed_checks"] = [cause]
                return finish(2, verify)
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
    check_model_id(args.model)
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

    info = new_info(task, args.role, args.model, attempt, started)

    def finish(code: int) -> int:
        info["elapsed_s"] = time.time() - wall_start
        lines = build_summary(info, None, bool(args.full))
        payload = result_payload(info, None, "\n".join(lines))
        write_record(record_path(task_dir, attempt, args.model), info)
        if args.json:
            print(json.dumps(payload, indent=2))
        else:
            for line in lines:
                print(line)
        return code

    try:
        # Resolution and version probing happen inside the try: a missing
        # executable (no-exe) must still write its result record, exactly
        # as `run` does (attempt-5 finding 3).
        prefix = opencode_prefix()
        major = opencode_major(prefix)
        if major >= 2:
            # 2.x: no --dir; the repo root is the process cwd plus PWD.
            cmd = prefix + [
                "run",
                "--standalone",
                "--agent", agent,
                "-m", args.model,
                "--format", "json",
                question,
            ]
        else:
            cmd = prefix + [
                "run",
                "--agent", agent,
                "-m", args.model,
                "--dir", str(repo),
                "--format", "json",
                question,
            ]
        wait_start_gap(repo)
        idle_or_timeout, rc = run_worker(cmd, repo, log_path, timeout_s, idle_s, agent, args.model)
        events = parse_events(log_path)
        absorb_events(info, events)
        if idle_or_timeout is not None:
            info["cause"] = idle_or_timeout
            if idle_or_timeout in MODEL_FAULT_CAUSES:
                info["failed_checks"] = [idle_or_timeout]
            return finish(2)
        cause = infra_cause(events)
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
        if stop.cause:
            eprint(f"oc: {stop.cause}")
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
    for _task, _role, model in members:
        check_model_id(model)
    slots = list_slots(slots_dir(repo))
    if not slots:
        raise Exit(2, "no-slot: no slot worktrees found")
    timeout_s = args.timeout if args.timeout is not None else DEFAULT_TIMEOUT

    # Each member's brief, resolved before anything starts: a filed brief or
    # the generator, placeholder-checked. Two members owning the same crate
    # are refused here, not after a worker has run.
    scratch = runs_dir(repo) / "_wave_briefs"
    brief_files: list[str] = []
    owned: list[set[str]] = []
    for task, _role, _model in members:
        kind, source = resolve_brief_source(repo, task, None)
        if kind == "generator":
            scratch.mkdir(parents=True, exist_ok=True)
            source = scratch / f"{task}.md"
            run_brief_generator(repo, task, source)
        assert source is not None
        brief_files.append(str(source))
        owned.append(brief_owned_crates(
            source.read_text(encoding="utf-8", errors="replace")))
    check_wave_crates(members, owned)

    results: list[dict] = [{} for _ in members]
    gate_lock = threading.Lock()
    running_models: dict[int, str] = {}

    def member_job(index: int, task: str, role: str, model: str) -> None:
        number, _slot = acquire_slot(repo, slots, None, wait=True)
        try:
            sub = argparse.Namespace(
                task=task,
                role=role,
                model=model,
                slot=number,
                timeout=timeout_s,
                idle=DEFAULT_IDLE,
                variant=None,
                brief=brief_files[index],
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
            with gate_lock:
                running_models.pop(index, None)
            release_slot(repo, number)

    workers = min(len(slots), len(members))
    threads = [
        threading.Thread(target=member_job, args=(i, *members[i]), daemon=True)
        for i in range(len(members))
    ]
    started: list[threading.Thread] = []
    cursor = 0
    # Members start in spec order. A member over a model cap waits for a
    # free place instead of being refused; nothing behind it starts first.
    # The model's place is reserved here, in the scheduling thread, before
    # the worker starts: reserving it in the worker would race the next
    # fits check and let two capped members run together.
    while cursor < len(threads) or any(t.is_alive() for t in started):
        while cursor < len(threads):
            alive = len([t for t in started if t.is_alive()])
            if alive >= workers:
                break
            with gate_lock:
                fits = wave_fits(members[cursor][2], list(running_models.values()))
                if fits:
                    running_models[cursor] = members[cursor][2]
            if not fits:
                break
            try:
                threads[cursor].start()
            except Exception:
                with gate_lock:
                    running_models.pop(cursor, None)
                raise
            started.append(threads[cursor])
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
    for (task, role, _model), payload in zip(members, results):
        attempt = payload.get("attempt", 0)
        lines.append(next_line(task, payload.get("verdict", "INFRA"),
                               payload.get("cause"), attempt, role) + f" ({task})")
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
    payload = new_info(args.task, args.role, args.model, attempt, utcnow())
    payload.update(version=RESULT_VERSION, verdict=verdict,
                   cause=RECORDED_CAUSE, outcomes=[entry])
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
    # The summary uses U+00B7 MIDDLE DOT; on Windows a piped stdout defaults
    # to the ANSI code page and the dot comes out as garbage. Force UTF-8
    # (with replacement, never a crash) before anything prints.
    for stream in (sys.stdout, sys.stderr):
        try:
            stream.reconfigure(encoding="utf-8", errors="replace")
        except (AttributeError, ValueError, OSError):
            pass
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

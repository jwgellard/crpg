#!/usr/bin/env python3
"""Fake `opencode` executable for tools/tests/test_oc.py.

Reads its behaviour from environment variables so each test can stage one
failure mode without the network or the real OpenCode:

  FAKE_OC_VERSION  text printed for `--version` (default 2.x,
                     `opencode v2.0.31`); set to a 1.x string such as
                     `opencode v1.18.35` to emulate OpenCode 1.x. In 2.x
                     mode the fake fails like the real 2.x on `--dir` or
                     `--variant` (usage text, exit 1) and expects
                     `--standalone`.
  FAKE_OC_LOG      JSON-lines file recording every invocation (argv, cwd, PWD).
  FAKE_OC_MODE     comma-separated behaviours, applied in order and matched
                     as whole tokens (never substrings, so `error-quota`
                     cannot fall into the `quota` branch):
                     ok            normal run: step_start, step_finish
                                   (tokens/cost), text, step_finish stop.
                     no-text       normal run with no text event (review/scout
                                   get exit 1 no-text).
                     locked[:N]    print "database is locked" and exit 1 until
                                   the Nth call (counter in FAKE_OC_STATE),
                                   then behave as ok. Default N is large: with
                                   no N the fake stays locked for ever.
                      stall         every step_finish has reason "length".
                      stall-once    first call stalls, later calls behave as ok
                                    (counter in FAKE_OC_STATE).
                      stall-tool-error
                                    first call stalls (reason "length") with a
                                    tool error reading "rate limit", later
                                    calls behave as ok: the tool error must not
                                    classify, so the stall retry still happens.
                      silent        write nothing, sleep until killed.
                      hang-forever  alias for silent.
                      slow          sleep FAKE_OC_SLOW seconds, then behave as ok
                                    (for wave concurrency-cap checks).
                      markers       clean run ending in stop whose tool output
                                    and report text contain every infra-table
                                    marker; must keep its normal verdict.
                      tool-error-quota
                                    a tool_use whose state.error carries a
                                    quota marker, last step_finish reason
                                    "tool-calls", text, exit 1: a tool error
                                    echoes command output and must never
                                    classify as infra.
                     fail402       print "ERROR 402 ... insufficient funds".
                     quota         print "AI_APICallError: Go usage limit exceeded".
                     sig-model     print "model not found: <model>".
                     headers       print "headers timed out".
                     no-route      emit a top-level error event
                                   (provider.no-route, "Model unavailable")
                                   with exit 1 and no steps, like 2.x.
                     no-agent      emit a top-level error event reading
                                   `Agent not found: "crpg-implement"`,
                                   exit 1, no steps: the cause table must
                                   map it to no-agent, not error.
                     error-quota   emit a top-level error event whose message
                                   says "Rate limit exceeded", exit 1, no
                                   steps: the message goes through the cause
                                   table (quota), not the plain-quota branch.
                     rc1           exit 1 with no output at all.
                     spawn-child   start a heartbeating child, then sleep.
                     error-log     append a matching level=ERROR line to
                                   $CRPG_OPENCODE_LOG (for the idle watchdog).
                     prompt-file   write the received prompt to
                                   $FAKE_OC_PROMPT_FILE for exact-match checks.
   FAKE_OC_STARTS   file to append time.time() at startup (start-gap checks).
   FAKE_OC_STDIN    "check": report whether stdin hit EOF at once (closed /
                    DEVNULL) or blocked (open pipe); used to prove oc.py passes
                    stdin=DEVNULL.
   FAKE_OC_SLOW     seconds the "slow" mode sleeps before its normal run.
   FAKE_OC_CONC     file logging "start <ts>" / "end <ts>" per invocation,
                    for wave concurrency-cap checks.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import threading
import time

STEP_COST = 0.0


def mode_tokens(raw: str) -> list[str]:
    """Split FAKE_OC_MODE into whole tokens on commas."""
    return [chunk.strip() for chunk in raw.split(",") if chunk.strip()]


def has_token(tokens: list[str], name: str) -> bool:
    """Exact token match, with an optional `:param` suffix (`locked:2`).

    Substring matching made `error-quota` fall into the `quota` branch and
    left the error-event branch dead, so every mode is matched whole.
    """
    return any(token == name or token.startswith(name + ":") for token in tokens)


def emit(obj: dict) -> None:
    sys.stdout.write(json.dumps(obj) + "\n")
    sys.stdout.flush()


def emit_ok(prompt: str, tokens: list[str]) -> None:
    reason = "length" if has_token(tokens, "stall") else "stop"
    emit({"type": "step_start", "timestamp": 1,
          "part": {"type": "step-start", "sessionID": "ses_fake"}})
    emit({"type": "tool_use", "timestamp": 2,
          "part": {"type": "tool", "tool": "read",
                   "state": {"status": "completed", "output": "ok"}}})
    emit({"type": "step_finish", "timestamp": 3,
          "part": {"type": "step-finish", "reason": "tool-calls",
                   "tokens": {"input": 1200, "output": 300, "reasoning": 50,
                              "cache": {"read": 900, "write": 10}},
                   "cost": 0.01}})
    if not has_token(tokens, "no-text"):
        emit({"type": "text", "timestamp": 4,
              "part": {"type": "text", "text": "fake worker report line 1\nline 2"}})
    emit({"type": "step_finish", "timestamp": 5,
          "part": {"type": "step-finish", "reason": reason,
                   "tokens": {"input": 400, "output": 100, "reasoning": 10,
                              "cache": {"read": 300, "write": 0}},
                   "cost": 0.02}})
    _ = prompt


def emit_stalled_with_tool_error() -> None:
    """A stalled run whose tool error reads "rate limit".

    The tool error must not classify as infra, so the stall retry still
    happens on the next (ok) call.
    """
    emit({"type": "step_start", "timestamp": 1,
          "part": {"type": "step-start", "sessionID": "ses_fake"}})
    emit({"type": "tool_use", "timestamp": 2,
          "part": {"type": "tool", "tool": "bash",
                   "state": {"status": "error",
                             "error": "429 rate limit exceeded"}}})
    emit({"type": "step_finish", "timestamp": 3,
          "part": {"type": "step-finish", "reason": "tool-calls",
                   "tokens": {"input": 1200, "output": 300, "reasoning": 50,
                              "cache": {"read": 900, "write": 10}},
                   "cost": 0.01}})
    emit({"type": "step_finish", "timestamp": 4,
          "part": {"type": "step-finish", "reason": "length",
                   "tokens": {"input": 400, "output": 100, "reasoning": 10,
                              "cache": {"read": 300, "write": 0}},
                   "cost": 0.02}})


def emit_tool_error_run() -> None:
    """A run whose tool_use carries a quota-looking state.error.

    Last step_finish reason is "tool-calls" and the process exits 1; a tool
    error echoes command output, so this must keep its normal verdict
    instead of exiting 2 quota.
    """
    emit({"type": "step_start", "timestamp": 1,
          "part": {"type": "step-start", "sessionID": "ses_fake"}})
    emit({"type": "tool_use", "timestamp": 2,
          "part": {"type": "tool", "tool": "bash",
                   "state": {"status": "error",
                             "error": "AI_APICallError: Too Many Requests"}}})
    emit({"type": "step_finish", "timestamp": 3,
          "part": {"type": "step-finish", "reason": "tool-calls",
                   "tokens": {"input": 1200, "output": 300, "reasoning": 50,
                              "cache": {"read": 900, "write": 10}},
                   "cost": 0.01}})
    emit({"type": "text", "timestamp": 4,
          "part": {"type": "text", "text": "fake worker report\n"}})
    emit({"type": "step_finish", "timestamp": 5,
          "part": {"type": "step-finish", "reason": "tool-calls",
                   "tokens": {"input": 400, "output": 100, "reasoning": 10,
                              "cache": {"read": 300, "write": 0}},
                   "cost": 0.02}})


def counter_was_zero(name: str) -> bool:
    """Increment $FAKE_OC_STATE/<name>.count; True when it was 0."""
    state = os.environ.get("FAKE_OC_STATE", "")
    path = os.path.join(state, name) if state else ""
    seen = 0
    if path and os.path.exists(path):
        try:
            seen = int(open(path, encoding="utf-8").read().strip())
        except ValueError:
            seen = 0
    if path:
        with open(path, "w", encoding="utf-8") as handle:
            handle.write(str(seen + 1))
    return seen == 0


# Every marker in oc.py's infra table, as plain words a worker could read
# from a file or quote in its report. A run carrying these must still get
# its normal verdict: only provider/process errors classify as infra.
MARKERS = (
    "402 insufficient funds rate limit quota usage limit too many requests "
    " 429 headers timed out model not found unknown model database is locked"
)


def emit_markers() -> None:
    emit({"type": "step_start", "timestamp": 1,
          "part": {"type": "step-start", "sessionID": "ses_fake"}})
    emit({"type": "tool_use", "timestamp": 2,
          "part": {"type": "tool", "tool": "read",
                   "state": {"status": "completed", "output": MARKERS}}})
    emit({"type": "step_finish", "timestamp": 3,
          "part": {"type": "step-finish", "reason": "tool-calls",
                   "tokens": {"input": 1200, "output": 300, "reasoning": 50,
                              "cache": {"read": 900, "write": 10}},
                   "cost": 0.01}})
    emit({"type": "text", "timestamp": 4,
          "part": {"type": "text", "text": "fake worker report\n" + MARKERS}})
    emit({"type": "step_finish", "timestamp": 5,
          "part": {"type": "step-finish", "reason": "stop",
                   "tokens": {"input": 400, "output": 100, "reasoning": 10,
                              "cache": {"read": 300, "write": 0}},
                   "cost": 0.02}})


def check_stdin() -> int:
    """Exit 0 with STDiN-CLOSED when stdin is at EOF at once, else report open."""
    result: list[str] = []

    def reader() -> None:
        try:
            data = sys.stdin.read()
        except Exception:  # noqa: BLE001
            data = ""
        result.append(data)

    thread = threading.Thread(target=reader, daemon=True)
    thread.start()
    thread.join(3.0)
    if thread.is_alive() or not result:
        sys.stdout.write("STDIN-OPEN\n")
        sys.stdout.flush()
        time.sleep(30)
        return 3
    sys.stdout.write("STDIN-CLOSED\n")
    sys.stdout.flush()
    return 0


def spawn_child() -> None:
    beat = os.environ.get("FAKE_OC_HEARTBEAT", "")
    child = subprocess.Popen(
        [sys.executable, "-c",
         "import time,sys\n"
         "beat=sys.argv[1]\n"
         "i=0\n"
         "while True:\n"
         "    open(beat,'a').write(f'{i} {time.time()}\\n')\n"
         "    i+=1\n"
         "    time.sleep(0.2)\n",
         beat],
        stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    statedir = os.environ.get("FAKE_OC_STATE", "")
    if statedir:
        with open(os.path.join(statedir, "child.pid"), "w", encoding="utf-8") as handle:
            handle.write(str(child.pid))
    time.sleep(60)


def fake_major() -> int:
    """Major version this fake emulates, from $FAKE_OC_VERSION (default 2)."""
    import re

    text = os.environ.get("FAKE_OC_VERSION", "opencode v2.0.31")
    match = re.search(r"(\d+)\.", text)
    try:
        return int(match.group(1)) if match else 2
    except ValueError:
        return 2


def main() -> int:
    argv = sys.argv[1:]

    # Answer the version probe before any logging: version checks must not
    # appear in FAKE_OC_LOG, FAKE_OC_STARTS or FAKE_OC_CONC.
    if "--version" in argv:
        sys.stdout.write(os.environ.get("FAKE_OC_VERSION", "opencode v2.0.31") + "\n")
        sys.stdout.flush()
        return 0

    # OpenCode 2.x has no --dir and no --variant: fail like the real one so
    # a dispatcher still building the 1.x argv cannot pass unnoticed.
    if fake_major() >= 2 and ("--dir" in argv or "--variant" in argv):
        sys.stdout.write(
            "error: unknown flag --dir\n"
            "Usage: opencode run [--standalone] --agent <agent> "
            "-m <model[#variant]> --format json <prompt>\n"
        )
        sys.stdout.flush()
        # Still record the bad call for debugging, then fail.
        log = os.environ.get("FAKE_OC_LOG", "")
        if log:
            with open(log, "a", encoding="utf-8") as handle:
                handle.write(json.dumps({
                    "argv": argv,
                    "cwd": os.getcwd(),
                    "pwd": os.environ.get("PWD", ""),
                }) + "\n")
        return 1

    starts = os.environ.get("FAKE_OC_STARTS", "")
    if starts:
        with open(starts, "a", encoding="utf-8") as handle:
            handle.write(f"{time.time()}\n")

    log = os.environ.get("FAKE_OC_LOG", "")
    if log:
        with open(log, "a", encoding="utf-8") as handle:
            handle.write(json.dumps({
                "argv": argv,
                "cwd": os.getcwd(),
                "pwd": os.environ.get("PWD", ""),
            }) + "\n")

    mode = os.environ.get("FAKE_OC_MODE", "ok")
    tokens = mode_tokens(mode)
    prompt = argv[-1] if argv else ""
    prompt_file = os.environ.get("FAKE_OC_PROMPT_FILE", "")
    if prompt_file and (has_token(tokens, "prompt-file") or has_token(tokens, "special")):
        with open(prompt_file, "w", encoding="utf-8") as handle:
            handle.write(prompt)

    if os.environ.get("FAKE_OC_STDIN", "") == "check":
        return check_stdin()

    if has_token(tokens, "spawn-child"):
        spawn_child()
        return 0

    if has_token(tokens, "locked"):
        wanted = 10 ** 9
        for token in tokens:
            if token.startswith("locked:"):
                try:
                    wanted = int(token.partition(":")[2])
                except ValueError:
                    wanted = 10 ** 9
        state = os.environ.get("FAKE_OC_STATE", "")
        counter = os.path.join(state, "locked.count") if state else ""
        seen = 0
        if counter and os.path.exists(counter):
            try:
                seen = int(open(counter, encoding="utf-8").read().strip())
            except ValueError:
                seen = 0
        if seen < wanted:
            if counter:
                with open(counter, "w", encoding="utf-8") as handle:
                    handle.write(str(seen + 1))
            sys.stdout.write("Error: database is locked\n")
            sys.stdout.flush()
            return 1
        emit_ok(prompt, ["ok"])
        return 0

    if has_token(tokens, "error-log"):
        logpath = os.environ.get("CRPG_OPENCODE_LOG", "")
        agent = "crpg-implement"
        model = "muse-spark"
        for i, part in enumerate(argv):
            if part == "--agent" and i + 1 < len(argv):
                agent = argv[i + 1]
            if part == "-m" and i + 1 < len(argv):
                model = argv[i + 1]
        suffix = model.split("/")[-1]
        line = (
            'timestamp=2026-10-16T10:27:27.101Z level=ERROR message="stream error" '
            f'providerID=opencode-go modelID={suffix} session.id=ses_fake agent={agent} '
            'error.error="AI_APICallError: Go usage limit exceeded"\n'
        )
        if logpath:
            with open(logpath, "a", encoding="utf-8") as handle:
                handle.write(line)
        time.sleep(60)
        return 0

    if has_token(tokens, "silent") or has_token(tokens, "hang-forever"):
        time.sleep(60)
        return 0

    if has_token(tokens, "stall-tool-error"):
        if counter_was_zero("stall-tool-error"):
            emit_stalled_with_tool_error()
        else:
            emit_ok(prompt, ["ok"])
        return 0

    if has_token(tokens, "stall-once"):
        if counter_was_zero("stall"):
            emit_ok(prompt, ["stall"])
        else:
            emit_ok(prompt, ["ok"])
        return 0

    if has_token(tokens, "tool-error-quota"):
        emit_tool_error_run()
        return 1

    if has_token(tokens, "fail402"):
        sys.stdout.write("ERROR 402 Payment Required: insufficient funds\n")
        sys.stdout.flush()
        return 1

    if has_token(tokens, "quota"):
        sys.stdout.write("AI_RetryError: Failed after 3 attempts. "
                         "Last error: Go usage limit exceeded\n")
        sys.stdout.flush()
        return 1

    if has_token(tokens, "sig-model"):
        sys.stdout.write("model not found: no-such-model\n")
        sys.stdout.flush()
        return 1

    if has_token(tokens, "headers"):
        sys.stdout.write("stream error: headers timed out\n")
        sys.stdout.flush()
        return 1

    if has_token(tokens, "no-route"):
        emit({"type": "error", "timestamp": 1,
              "error": {"type": "provider.no-route",
                        "message": "Model unavailable: opencode-go/cat-jam-free"}})
        return 1

    if has_token(tokens, "no-agent"):
        # The agent the run asked for is not defined: exit 1 with a top-level
        # error event, no steps, like 2.x.
        emit({"type": "error", "timestamp": 1,
              "error": {"type": "provider.agentNotFound",
                        "message": 'Agent not found: "crpg-implement"'}})
        return 1

    if has_token(tokens, "error-quota"):
        # A top-level error event whose message carries the quota marker:
        # the message goes through the cause table, so the cause is quota.
        emit({"type": "error", "timestamp": 1,
              "error": {"type": "provider.apiError",
                        "message": "Rate limit exceeded"}})
        return 1

    if has_token(tokens, "rc1"):
        return 1

    if has_token(tokens, "markers"):
        emit_markers()
        return 0

    if has_token(tokens, "slow"):
        try:
            delay = float(os.environ.get("FAKE_OC_SLOW", "1.0"))
        except ValueError:
            delay = 1.0
        time.sleep(delay)

    emit_ok(prompt, tokens)
    return 0


if __name__ == "__main__":
    # The --version probe is not a worker run: it must not appear in the
    # concurrency log either.
    conc = "" if "--version" in sys.argv else os.environ.get("FAKE_OC_CONC", "")
    if conc:
        with open(conc, "a", encoding="utf-8") as handle:
            handle.write(f"start {time.time()}\n")
    code = main()
    if conc:
        with open(conc, "a", encoding="utf-8") as handle:
            handle.write(f"end {time.time()}\n")
    sys.exit(code)

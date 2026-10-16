#!/usr/bin/env python3
"""Fake `opencode` executable for tools/tests/test_oc.py.

Reads its behaviour from environment variables so each test can stage one
failure mode without the network or the real OpenCode:

  FAKE_OC_LOG      JSON-lines file recording every invocation (argv, cwd, PWD).
  FAKE_OC_MODE     comma-separated behaviours, applied in order:
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
                     silent        write nothing, sleep until killed.
                     hang-forever  alias for silent.
                     fail402       print "ERROR 402 ... insufficient funds".
                     quota         print "AI_APICallError: Go usage limit exceeded".
                     sig-model     print "model not found: <model>".
                     headers       print "headers timed out".
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
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import threading
import time

STEP_COST = 0.0


def emit(obj: dict) -> None:
    sys.stdout.write(json.dumps(obj) + "\n")
    sys.stdout.flush()


def emit_ok(prompt: str, mode: str) -> None:
    reason = "length" if "stall" in mode else "stop"
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
    if "no-text" not in mode:
        emit({"type": "text", "timestamp": 4,
              "part": {"type": "text", "text": "fake worker report line 1\nline 2"}})
    emit({"type": "step_finish", "timestamp": 5,
          "part": {"type": "step-finish", "reason": reason,
                   "tokens": {"input": 400, "output": 100, "reasoning": 10,
                              "cache": {"read": 300, "write": 0}},
                   "cost": 0.02}})
    _ = prompt


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


def main() -> int:
    starts = os.environ.get("FAKE_OC_STARTS", "")
    if starts:
        with open(starts, "a", encoding="utf-8") as handle:
            handle.write(f"{time.time()}\n")

    argv = sys.argv[1:]
    log = os.environ.get("FAKE_OC_LOG", "")
    if log:
        with open(log, "a", encoding="utf-8") as handle:
            handle.write(json.dumps({
                "argv": argv,
                "cwd": os.getcwd(),
                "pwd": os.environ.get("PWD", ""),
            }) + "\n")

    mode = os.environ.get("FAKE_OC_MODE", "ok")
    prompt = argv[-1] if argv else ""
    prompt_file = os.environ.get("FAKE_OC_PROMPT_FILE", "")
    if prompt_file and ("prompt-file" in mode or "special" in mode):
        with open(prompt_file, "w", encoding="utf-8") as handle:
            handle.write(prompt)

    if os.environ.get("FAKE_OC_STDIN", "") == "check":
        return check_stdin()

    if "spawn-child" in mode:
        spawn_child()
        return 0

    if "locked" in mode:
        wanted = 0
        for chunk in mode.split(","):
            if chunk.startswith("locked"):
                _, _, count = chunk.partition(":")
                wanted = int(count) if count else 10 ** 9
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
        emit_ok(prompt, "ok")
        return 0

    if "error-log" in mode:
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

    if "silent" in mode or "hang-forever" in mode:
        time.sleep(60)
        return 0

    if "stall-once" in mode:
        state = os.environ.get("FAKE_OC_STATE", "")
        counter = os.path.join(state, "stall.count") if state else ""
        seen = 0
        if counter and os.path.exists(counter):
            try:
                seen = int(open(counter, encoding="utf-8").read().strip())
            except ValueError:
                seen = 0
        if counter:
            with open(counter, "w", encoding="utf-8") as handle:
                handle.write(str(seen + 1))
        if seen == 0:
            emit_ok(prompt, "stall")
        else:
            emit_ok(prompt, "ok")
        return 0

    if "fail402" in mode:
        sys.stdout.write("ERROR 402 Payment Required: insufficient funds\n")
        sys.stdout.flush()
        return 1

    if "quota" in mode:
        sys.stdout.write("AI_RetryError: Failed after 3 attempts. "
                         "Last error: Go usage limit exceeded\n")
        sys.stdout.flush()
        return 1

    if "sig-model" in mode:
        sys.stdout.write("model not found: no-such-model\n")
        sys.stdout.flush()
        return 1

    if "headers" in mode:
        sys.stdout.write("stream error: headers timed out\n")
        sys.stdout.flush()
        return 1

    if "rc1" in mode:
        return 1

    emit_ok(prompt, mode)
    return 0


if __name__ == "__main__":
    sys.exit(main())

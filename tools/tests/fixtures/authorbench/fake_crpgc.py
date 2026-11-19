#!/usr/bin/env python3
"""Fake crpgc for tools/tests/test_authorbench*.py.

Behaviour is read from the text of <campaign>/campaign.json (a marker word
anywhere in it) and from environment variables; it never touches the network.
"""
import json
import os
import sys
import time

DIAG_ERR = {"file": "campaign.json", "pointer": "/name", "severity": "error",
            "code": "missing_field", "message": "campaign name is required",
            "suggested_fix": "add a name"}
DIAG_WARN = {"file": "campaign.json", "pointer": "", "severity": "warning",
             "code": "unused_asset", "message": "asset is unused",
             "suggested_fix": None}
DIAG_IO = {"file": None, "pointer": "", "severity": "error", "code": "io",
           "message": "cannot read campaign.json: not_found", "suggested_fix": None}


def campaign_text(root):
    path = os.path.join(root, "campaign.json")
    if not os.path.isfile(path):
        return None
    with open(path, encoding="utf-8") as handle:
        return handle.read()


def validate(args):
    text = campaign_text(args[0])
    if text is None:
        print(json.dumps([DIAG_IO]))
        return 1
    if "BADOUT" in text:
        print("not json")
        return 0
    if "BROKEN" in text:
        print(json.dumps([DIAG_ERR, DIAG_WARN]))
        return 1
    if "WARN" in text:
        print(json.dumps([DIAG_WARN]))
        return 0
    print("[]")
    return 0


def play(args):
    root = args[args.index("--campaign") + 1]
    lines = len(sys.stdin.buffer.read().splitlines())
    print(f"stdin_lines={lines}", file=sys.stderr)
    text = campaign_text(root)
    if text is None:
        print("crpgc play: cannot read campaign", file=sys.stderr)
        return 1
    if "NOENC" in text:
        print("crpgc play: campaign has no encounter", file=sys.stderr)
        return 1
    try:
        count = int(json.loads(text).get("fake_combatants", 2))
    except (ValueError, AttributeError):
        count = 2
    print("== Combat demo: Fake ==")
    print("Encounter: Fake Fight")
    print("Combatants:")
    for i in range(count):
        print(f"  Unit{i} | side Unit{i} | 10/10 hp")
    print()
    if "QUITS" in text:
        print("Quit.")
        return 0
    print("== Summary ==")
    print("Rounds played: 3")
    print("Final health:")
    print("  Unit0 10/10 hp")
    print("Survivors: Unit0")
    print("Winner: Unit0")
    return 0


def run(args):
    forced = os.environ.get("FAKE_CRPGC_SMOKE_RC")
    if forced is not None:
        return int(forced)
    ticks = int(args[args.index("--ticks") + 1])
    every = int(args[args.index("--hash-every") + 1])
    for tick in range(every, ticks + 1, every):
        print(f"{tick} " + "ab" * 32)
    return 0


def main():
    argv = sys.argv[1:]
    delay = float(os.environ.get("FAKE_CRPGC_SLEEP", "0"))
    if delay:
        time.sleep(delay)
    if not argv:
        print("crpgc: missing subcommand", file=sys.stderr)
        return 2
    table = {"validate": validate, "play": play, "run": run}
    if argv[0] not in table:
        print(f"crpgc: unknown subcommand '{argv[0]}'", file=sys.stderr)
        return 2
    return table[argv[0]](argv[1:])


if __name__ == "__main__":
    sys.exit(main())

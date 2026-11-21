#!/usr/bin/env python3
"""Mutant driver for T155d (the new lines of crates/crpg-server/src/main.rs).

Applies one mutant at a time by exact text replacement in the source file the
mutant table cites, runs
`cargo test -p crpg-server --locked -j 2 --no-fail-fast --lib
 --test server_binary_movement --test server_binary`
(T155C_FULL=1 runs the whole crate's tests instead; use it for survivors) with
CARGO_INCREMENTAL=0, records one JSON line per mutant in results.jsonl, and
restores the original bytes in a `finally`.

usage: run_mutants.py [--check] [ids...]
       --check applies and restores every anchor without cargo (no results).
       no ids: all rows, in table order.
"""
import json
import os
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / "reviews/T155d/results.jsonl"
TIMEOUT = 900
DUMP = os.environ.get("MUTANT_DUMP")

# id -> (description, contract row, [(file, old, new), ...])
M = {}


def m(mid, desc, item, edits):
    M[mid] = (desc, item, edits)


F = "crates/crpg-server/src/main.rs"

m("M0", "unmutated run (must pass)", "-", [])

AREA = "    if movement && config.area.is_none() {\n"
m("M1", "main.rs serve (~625, B-M1): the `movement needs area` check is removed"
  " (`if false && ...`)", "B-M1",
  [(F, AREA, "    if false && movement && config.area.is_none() {\n")])
MOV = "    let movement = config.movement == Some(true);\n"
m("M2", "main.rs serve (~624, B-M1): `Some(false)` counts as movement"
  " (`config.movement.is_some()`)", "B-M1",
  [(F, MOV, "    let movement = config.movement.is_some();\n")])
m("M2b", "main.rs serve (~624, B-M1): an absent `movement` counts as movement"
  " (`unwrap_or(true)`)", "B-M1",
  [(F, MOV, "    let movement = config.movement.unwrap_or(true);\n")])
m("M2c", "main.rs serve (~625, B-M1): the area check also fires for a combat"
  " world (`movement &&` dropped)", "B-M1",
  [(F, AREA, "    if config.area.is_none() {\n")])

V4 = "(true, Some(facts), Some(rights)) => Entry::Movement(V4Invitation {"
RT = "                rights: rights.into(),\n"
FT = "                facts: facts.into(),\n"
m("M3", "main.rs serve (~682, B-M2): movement accepts a missing `rights`"
  " (default rights)", "B-M2",
  [(F, V4, "(true, Some(facts), rights) => Entry::Movement(V4Invitation {"),
   (F, RT, "                rights: rights.map(PauseRights::from).unwrap_or_default(),\n")])
m("M4", "main.rs serve (~682, B-M2): movement accepts a missing `facts`"
  " (default facts)", "B-M2",
  [(F, V4, "(true, facts, Some(rights)) => Entry::Movement(V4Invitation {"),
   (F, FT, "                facts: facts.map(FactGrants::from).unwrap_or_default(),\n")])
COMBAT = "(false, None, None) => Entry::Combat("
m("M5", "main.rs serve (~694, B-M2): the `facts and rights need movement`"
  " check is removed (combat arm takes any facts/rights)", "B-M2",
  [(F, COMBAT, "(false, _, _) => Entry::Combat(")])
m("M5b", "main.rs serve (~694, B-M2): a combat entry with `rights` only is"
  " accepted", "B-M2", [(F, COMBAT, "(false, None, _) => Entry::Combat(")])
m("M5c", "main.rs serve (~694, B-M2): a combat entry with `facts` only is"
  " accepted", "B-M2", [(F, COMBAT, "(false, _, None) => Entry::Combat(")])
E1 = '"entry {i}: movement needs facts and rights"'
E2 = '"entry {i}: facts and rights need movement"'
m("M6", "main.rs serve (~691, B-M2): `movement needs facts and rights` reports"
  " entry 0 instead of `i`", "B-M2",
  [(F, E1, '"entry 0: movement needs facts and rights"')])
m("M6b", "main.rs serve (~701, B-M2): `facts and rights need movement` reports"
  " entry 0 instead of `i`", "B-M2",
  [(F, E2, '"entry 0: facts and rights need movement"')])
m("M6c", "main.rs serve (~691/701, B-M2): the two error texts are swapped",
  "B-M2", [(F, E1, "@@E2@@"), (F, E2, E1), (F, "@@E2@@", E2)])

BIND = ("    let server = if movement {\n"
        "        QuicServer::bind_movement_resumable(server_config)\n")
m("M7", "main.rs serve (~752, B-M3): movement binds `bind_resumable`", "B-M3",
  [(F, BIND, BIND.replace("if movement", "if false"))])
m("M8", "main.rs serve (~752, B-M3): a combat world binds"
  " `bind_movement_resumable`", "B-M3",
  [(F, BIND, BIND.replace("if movement", "if true"))])

REG = ("            Entry::Movement(invitation) =>"
       " quic.register_v4_invitation(invitation).map(|_| ()),\n")
REGC = ("            Entry::Combat(invitation) =>"
        " quic.register_invitation(invitation).map(|_| ()),\n")
m("M9", "main.rs serve (~768, B-M4): a movement entry registers with"
  " `register_invitation`", "B-M4",
  [(F, REG, "            Entry::Movement(invitation) => quic\n"
    "                .register_invitation(Invitation {\n"
    "                    credential: invitation.credential,\n"
    "                    control: invitation.control,\n"
    "                    grants: invitation.grants,\n"
    "                })\n"
    "                .map(|_| ()),\n")])
m("M9b", "main.rs serve (~767, B-M4): a combat entry registers with"
  " `register_v4_invitation` (empty facts, default rights)", "B-M4",
  [(F, REGC, "            Entry::Combat(invitation) => quic\n"
    "                .register_v4_invitation(V4Invitation {\n"
    "                    credential: invitation.credential,\n"
    "                    control: invitation.control,\n"
    "                    grants: invitation.grants,\n"
    "                    facts: FactGrants::default(),\n"
    "                    rights: PauseRights::default(),\n"
    "                })\n"
    "                .map(|_| ()),\n")])
m("M9c", "main.rs serve (~770, B-M4): a register refusal reports entry 0"
  " instead of `i`", "B-M4",
  [(F, 'fail("register")(format!("entry {i}: {e}"))',
    'fail("register")(format!("entry 0: {e}"))')])
V4H = "                credential,\n                control,\n                grants,\n" + FT
m("M9d", "main.rs serve (~686, B-M4): the movement invitation's `control` is"
  " empty", "B-M4",
  [(F, V4H, V4H.replace("                control,\n",
                        "                control: ControlGrant { actors: Vec::new() },\n"))])
m("M9e", "main.rs serve (~687, B-M4): the movement invitation's `grants` is"
  " empty", "B-M4",
  [(F, V4H, V4H.replace("                grants,\n",
                        "                grants: DisclosureGrants::default(),\n"))])
m("M9f", "main.rs serve (~689, B-M4): the movement invitation's `rights` is"
  " the default (deny all)", "B-M4",
  [(F, RT, "                rights: {\n                    let _ = rights;\n"
    "                    PauseRights::default()\n                },\n")])
m("M9g", "main.rs serve (~688, B-M4): the movement invitation's `facts` is"
  " empty", "B-M4",
  [(F, FT, "                facts: {\n                    let _ = facts;\n"
    "                    FactGrants::default()\n                },\n")])

m("M10", "main.rs From<FactsEntry> (~366, B-M4): `pools` <- `effects`", "B-M4",
  [(F, "                    pools: e.pools,\n", "                    pools: e.effects,\n")])
m("M10b", "main.rs From<FactsEntry> (~367, B-M4): `effects` <- `pools`", "B-M4",
  [(F, "                    effects: e.effects,\n", "                    effects: e.pools,\n")])
m("M10c", "main.rs From<FactsEntry> (~368, B-M4): `stats` <- `tags`", "B-M4",
  [(F, "                    stats: e.stats,\n", "                    stats: e.tags,\n")])
m("M10d", "main.rs From<FactsEntry> (~369, B-M4): `tags` <- `stats`", "B-M4",
  [(F, "                    tags: e.tags,\n", "                    tags: e.stats,\n")])
m("M10e", "main.rs From<FactsEntry> (~365, B-M4): `entity` replaced by a"
  " fixed id (index 0 generation 1, via serde_json)", "B-M4",
  [(F, "                    entity: e.entity,\n",
    "                    entity: {\n                        let _ = e.entity;\n"
    "                        serde_json::from_str::<EntityId>(\n"
    "                            r#\"{\"index\":0,\"generation\":1}\"#,\n"
    "                        )\n"
    "                        .unwrap()\n                    },\n")])
m("M10f", "main.rs From<FactsEntry> (~362, B-M4): the entity list is dropped",
  "B-M4",
  [(F, "                .into_iter()\n                .map(|e| EntityFactGrant {",
    "                .into_iter()\n                .take(0)\n                .map(|e| EntityFactGrant {")])
m("M10g", "main.rs From<FactsEntry> (~375, B-M4): encounter `mode` <-"
  " `pending_mode`", "B-M4",
  [(F, "                mode: f.encounter.mode,\n", "                mode: f.encounter.pending_mode,\n")])
m("M10h", "main.rs From<FactsEntry> (~376, B-M4): encounter `pending_mode` <-"
  " `mode`", "B-M4",
  [(F, "                pending_mode: f.encounter.pending_mode,\n", "                pending_mode: f.encounter.mode,\n")])
m("M10i", "main.rs From<FactsEntry> (~377, B-M4): encounter `clock` <-"
  " `timeline`", "B-M4",
  [(F, "                clock: f.encounter.clock,\n", "                clock: f.encounter.timeline,\n")])
m("M10j", "main.rs From<FactsEntry> (~378, B-M4): encounter `timeline` <-"
  " `clock`", "B-M4",
  [(F, "                timeline: f.encounter.timeline,\n", "                timeline: f.encounter.clock,\n")])
m("M10k", "main.rs From<FactsEntry> (~381, B-M4): host `pause_reason` <-"
  " `turn_timer`", "B-M4",
  [(F, "                pause_reason: f.host.pause_reason,\n", "                pause_reason: f.host.turn_timer,\n")])
m("M10l", "main.rs From<FactsEntry> (~382, B-M4): host `auto_pause` <-"
  " `pause_reason`", "B-M4",
  [(F, "                auto_pause: f.host.auto_pause,\n", "                auto_pause: f.host.pause_reason,\n")])
m("M10m", "main.rs From<FactsEntry> (~383, B-M4): host `turn_timer` <-"
  " `auto_pause`", "B-M4",
  [(F, "                turn_timer: f.host.turn_timer,\n", "                turn_timer: f.host.auto_pause,\n")])

m("M11", "main.rs From<RightsEntry> (~392, B-M4): `pause` and `unpause`"
  " swapped", "B-M4",
  [(F, "            pause: r.pause,\n            unpause: r.unpause,\n",
    "            pause: r.unpause,\n            unpause: r.pause,\n")])
m("M11b", "main.rs From<RightsEntry> (~392, B-M4): `pause` <- `unpause`",
  "B-M4", [(F, "            pause: r.pause,\n", "            pause: r.unpause,\n")])
m("M11c", "main.rs From<RightsEntry> (~393, B-M4): `unpause` <- `pause`",
  "B-M4", [(F, "            unpause: r.unpause,\n", "            unpause: r.pause,\n")])

for tag, ty in (("a", "FactsEntry"), ("b", "EntityFactEntry"),
                ("c", "EncounterEntry"), ("d", "HostGrantsEntry"),
                ("e", "RightsEntry")):
    m("M12" + tag, f"main.rs `{ty}` (new type, T155d interface): its"
      " `#[serde(deny_unknown_fields)]` is removed", "interface",
      [(F, f"#[serde(deny_unknown_fields)]\nstruct {ty} {{\n",
        f"struct {ty} {{\n")])

START = '        if movement { " movement=on" } else { "" },\n'
m("M13", "main.rs serve (~823, B-M5): the ` movement=on` suffix is omitted",
  "B-M5", [(F, START, '        if movement { "" } else { "" },\n')])
m("M13b", "main.rs serve (~823, B-M5): the suffix loses its leading space",
  "B-M5", [(F, START, '        if movement { "movement=on" } else { "" },\n')])
m("M13c", "main.rs serve (~823, B-M5): the suffix reads `movement=off`",
  "B-M5", [(F, START, '        if movement { " movement=off" } else { "" },\n')])
m("M14", "main.rs serve (~823, B-M5): the suffix is added on the combat path"
  " too", "B-M5", [(F, START, '        if true { " movement=on" } else { "" },\n')])
m("M14b", "main.rs serve (~823, B-M5): the suffix is added only on the combat"
  " path", "B-M5", [(F, START, '        if !movement { " movement=on" } else { "" },\n')])
m("M14c", "main.rs serve (~823, B-M5): the combat line gains a trailing"
  " space", "B-M5",
  [(F, START, '        if movement { " movement=on" } else { " " },\n')])

m("M15", "main.rs serve (~626, B-M1): the `movement needs area` message is"
  " altered", "B-M1",
  [(F, '"movement needs area"', '"movement requires area"')])
m("M16", "main.rs serve (~688, B-M2): the `(true, _, _)` error arm builds a"
  " Combat entry instead of failing", "B-M2",
  [(F, "            (true, _, _) => {\n"
       "                return Err(fail(\"invitations\")(format!(\n"
       "                    " + E1 + "\n                )));\n            }\n",
    "            (true, _, _) => Entry::Combat(Invitation {\n"
    "                credential,\n                control,\n"
    "                grants,\n            }),\n")])

FILES = sorted({name for _, _, edits in M.values() for name, _, _ in edits})


def read_originals():
    return {name: (ROOT / name).read_bytes() for name in FILES}


def apply(originals, edits):
    """Returns {file: mutated bytes} for the files this mutant edits."""
    out = {}
    for name, old, new in edits:
        src = out.get(name, originals[name])
        crlf = b"\r\n" in src
        text = src.decode("utf-8")
        if crlf:
            old = old.replace("\n", "\r\n")
            new = new.replace("\n", "\r\n")
        count = text.count(old)
        if count != 1:
            raise ValueError(
                f"{name}: anchor matched {count} times: {old[:70]!r}"
            )
        out[name] = text.replace(old, new, 1).encode("utf-8")
    return out


def run_cargo():
    env = dict(os.environ, CARGO_INCREMENTAL="0")
    cmd = ["cargo", "test", "-p", "crpg-server", "--locked", "-j", "2",
           "--no-fail-fast"]
    if os.environ.get("T155C_FULL") != "1":
        cmd += ["--lib", "--test", "server_binary_movement", "--test",
                "server_binary"]
    p = subprocess.Popen(cmd, cwd=ROOT, env=env, stdout=subprocess.PIPE,
                         stderr=subprocess.STDOUT, text=True, encoding="utf-8",
                         errors="replace")
    try:
        out, _ = p.communicate(timeout=TIMEOUT)
        return p.returncode, out, False
    except subprocess.TimeoutExpired:
        subprocess.run(["taskkill", "/F", "/T", "/PID", str(p.pid)],
                       capture_output=True)
        try:
            out, _ = p.communicate(timeout=30)
        except Exception:
            out = ""
        return -1, out, True


def classify(rc, out, timed_out):
    if timed_out:
        return "killed", ["<timeout>"]
    if "test result:" not in out or "could not compile" in out:
        return "compile-error", []
    failed = [ln.split()[1] for ln in out.splitlines()
              if ln.startswith("test ") and ln.rstrip().endswith("FAILED")]
    if rc == 0 and not failed:
        return "survived", []
    return "killed", failed


def main():
    args = sys.argv[1:]
    check = "--check" in args
    ids = [a for a in args if a != "--check"] or list(M)
    originals = read_originals()
    lines = {}
    if not check:
        OUT.parent.mkdir(parents=True, exist_ok=True)
        if OUT.exists():
            for line in OUT.read_text(encoding="utf-8").splitlines():
                if line.strip():
                    old = json.loads(line)
                    lines[old["id"]] = line
        for mid in ids:
            lines.pop(mid, None)

    def flush():
        order = list(M)
        OUT.write_text(
            "".join(lines[mid] + "\n" for mid in order if mid in lines),
            encoding="utf-8")

    try:
        for mid in ids:
            desc, item, edits = M[mid]
            rec = {"id": mid, "desc": desc, "contract": item,
                   "edits": [{"file": f, "old": o, "new": n}
                             for f, o, n in edits]}
            t0 = time.time()
            try:
                mutated = apply(originals, edits)
            except ValueError as e:
                rec.update(status="anchor-error", detail=str(e))
                print(mid, "anchor-error", e, flush=True)
                if not check:
                    lines[mid] = json.dumps(rec)
                    flush()
                continue
            if check:
                print(mid, "anchor-ok", desc, flush=True)
                continue
            rc, out, to = (0, "", False)
            try:
                for name, data in mutated.items():
                    (ROOT / name).write_bytes(data)
                rc, out, to = run_cargo()
            finally:
                for name in mutated:
                    (ROOT / name).write_bytes(originals[name])
            if DUMP:
                Path(DUMP).mkdir(parents=True, exist_ok=True)
                (Path(DUMP) / f"{mid}.cargo.log").write_text(
                    out, encoding="utf-8")
            status, failed = classify(rc, out, to)
            rec.update(status=status, failed_tests=failed,
                       suite="full" if os.environ.get("T155C_FULL") == "1" else "subset",
                       seconds=round(time.time() - t0, 1))
            if mid == "M0" and status == "survived":
                status = "pass"
                rec["status"] = "pass"
            lines_out = out.splitlines()
            for n, ln in enumerate(lines_out):
                if "panicked at" in ln:
                    rec["panic"] = " | ".join(
                        x.strip() for x in lines_out[n:n + 4])[:600]
                    break
            if status == "compile-error":
                rec["detail"] = [ln for ln in out.splitlines()
                                 if ln.startswith("error")][:3]
            if status == "survived":
                rec["class"] = "UNCLASSIFIED"
            lines[mid] = json.dumps(rec)
            flush()
            print(mid, status, failed, flush=True)
    finally:
        for name, data in originals.items():
            (ROOT / name).write_bytes(data)


if __name__ == "__main__":
    main()

"""Merge scores: t1/t3 from bake/scores.json, t2 from bake2/scores.json. Prints a markdown table."""
import json, os, sys
S = sys.argv[1]
rows = {}
def load(p, tasks):
    for r in json.load(open(p)):
        if r["task"] in tasks: rows.setdefault(r["model"], {})[r["task"]] = r
load(os.path.join(S, "bake", "scores.json"), {"t1"})
load(os.path.join(S, "bake3", "scores.json"), {"t3"})
load(os.path.join(S, "bake2", "scores.json"), {"t2"})
def tok(r): t = r["tok"]; return (t["input"] + t["cache_read"] + t["output"]) // 1000
print("| Model | Scout | Implement: hidden tests, gates, rules | Review: found / false pos. | Impl. steps · time · tokens (k) |")
print("|---|---|---|---|---|")
for m in sorted(rows):
    d = rows[m]
    t1 = d.get("t1"); t2 = d.get("t2"); t3 = d.get("t3")
    s1 = "—" if not t1 else ("exact" if t1["score"].get("pass") else f"wrong ({len(t1['score'].get('fn') or [])} missed, {len(t1['score'].get('fp') or [])} extra)" if t1["score"].get("note") is None else t1["score"]["note"])
    if t1 and t1["rc"] != 0: s1 += f" rc={t1['rc']}"
    if t2:
        sc = t2["score"]
        gates = "ok" if (sc["fmt"] and sc["clippy"] and sc["visible"]) else "/".join(k for k in ("fmt", "clippy", "visible") if not sc[k]) + " failed"
        rules = "ok" if sc["rules_ok"] else ",".join(k for k, bad in (("float", sc["float"]), ("unsafe", sc["unsafe"]), ("Cargo.toml", not sc["cargo_toml_same"]), ("edited tests", not sc["visible_same"])) if bad)
        s2 = f"{sc['hidden']} · {gates} · {rules}" + (" · todo left" if sc["todo_left"] else "") + (f" · rc={t2['rc']}" if t2["rc"] != 0 else "")
        perf = f"{t2['steps']} · {t2['secs']:.0f}s · {tok(t2)}"
    else: s2, perf = "—", "—"
    if t3:
        sc = t3["score"]
        s3 = sc.get("note") or f"{sc['found']}/5 [{sc['which']}] / {sc['fp']}" + (" · decoy" if sc.get("decoy_fp") else "")
        if not sc.get("unchanged", True): s3 += " · EDITED FILE"
        if t3["rc"] != 0: s3 += f" rc={t3['rc']}"
    else: s3 = "—"
    print(f"| `{m}` | {s1} | {s2} | {s3} | {perf} |")

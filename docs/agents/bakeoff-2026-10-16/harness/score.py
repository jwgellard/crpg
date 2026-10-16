"""Score bake-off runs. Usage: score.py <scratch>"""
import glob, hashlib, json, os, re, shutil, subprocess, sys
S = sys.argv[1]
TPL = os.path.join(S, "tpl")
def sha(p):
    try: return hashlib.sha256(open(p, "rb").read()).hexdigest()
    except OSError: return None
def parse_log(path):
    texts, steps, tools, err = [], 0, 0, ""
    tok = {"input": 0, "output": 0, "reasoning": 0, "cache_read": 0}; cost = 0.0
    for line in open(path, encoding="utf-8", errors="replace"):
        try: e = json.loads(line)
        except Exception:
            if "rror" in line: err = err or line.strip()[:160]
            continue
        t = e.get("type"); part = e.get("part", {})
        if t == "text": texts.append(part.get("text", ""))
        elif t == "step_finish":
            steps += 1; tk = part.get("tokens", {}) or {}
            tok["input"] += tk.get("input", 0) or 0; tok["output"] += tk.get("output", 0) or 0
            tok["reasoning"] += tk.get("reasoning", 0) or 0; tok["cache_read"] += (tk.get("cache") or {}).get("read", 0) or 0
            cost += part.get("cost", 0) or 0
        elif t == "tool_use": tools += 1
        elif t == "error": err = err or json.dumps(e.get("error"))[:200]
    return {"text": texts[-1] if texts else "", "all_text": "\n".join(texts), "steps": steps, "tools": tools, "tok": tok, "cost": cost, "err": err}
def last_json_array(text):
    for m in reversed(list(re.finditer(r"\[[\s\S]*?\](?=[^\]]*$)|\[[\s\S]*\]", text))):
        try: return json.loads(m.group(0))
        except Exception: continue
    # fallback: find any array
    for m in re.finditer(r"\[[^\[\]]*(?:\{[^{}]*\}[^\[\]]*)*\]", text):
        try: return json.loads(m.group(0))
        except Exception: pass
    return None
TRUTH1 = sorted(["crpg-ai","crpg-cli","crpg-godot","crpg-net","crpg-persist","crpg-script","crpg-server","crpg-testkit"])
def score_t1(d, L):
    arr = last_json_array(L["all_text"]); unchanged = sha(os.path.join(d,"tools/lint/deps.py")) == sha(os.path.join(TPL,"t1/tools/lint/deps.py"))
    if not isinstance(arr, list): return {"pass": False, "note": "no array", "unchanged": unchanged}
    got = set(map(str, arr)); truth = set(TRUTH1)
    return {"pass": got == truth, "tp": len(got & truth), "fp": sorted(got - truth), "fn": sorted(truth - got), "unchanged": unchanged}
def run(cmd, d, t=600):
    try:
        p = subprocess.run(cmd, cwd=d, capture_output=True, text=True, timeout=t, stdin=subprocess.DEVNULL, encoding="utf-8", errors="replace")
        return p.returncode, p.stdout + p.stderr
    except subprocess.TimeoutExpired: return "TIMEOUT", ""
def score_t2(d, L):
    r = {}
    src = open(os.path.join(d, "src/lib.rs"), encoding="utf-8", errors="replace").read() if os.path.exists(os.path.join(d,"src/lib.rs")) else ""
    r["todo_left"] = "todo!()" in src
    code = "\n".join(open(p, encoding="utf-8", errors="replace").read() for p in glob.glob(os.path.join(d, "src/**/*.rs"), recursive=True) + glob.glob(os.path.join(d, "tests/*.rs")))
    r["float"] = bool(re.search(r"\bf(32|64)\b", code))
    r["unsafe"] = "unsafe" in src.replace("forbid(unsafe_code)", "")
    r["cargo_toml_same"] = sha(os.path.join(d,"Cargo.toml")) == sha(os.path.join(TPL,"t2/Cargo.toml"))
    r["visible_same"] = sha(os.path.join(d,"tests/visible.rs")) == sha(os.path.join(TPL,"t2/tests/visible.rs"))
    rc, out = run(["cargo", "fmt", "--check"], d); r["fmt"] = rc == 0
    rc, out = run(["cargo", "clippy", "-q", "--all-targets", "--", "-D", "warnings"], d); r["clippy"] = rc == 0
    rc, out = run(["cargo", "test", "-q", "--test", "visible"], d); r["visible"] = rc == 0
    shutil.copy(os.path.join(S, "ref", "hidden.rs"), os.path.join(d, "tests", "zz_hidden.rs"))
    rc, out = run(["cargo", "test", "-q", "--test", "zz_hidden"], d)
    m = re.search(r"(\d+) passed; (\d+) failed", out)
    r["hidden"] = f"{m.group(1)}/21" if m else ("build-fail" if rc != 0 else "?")
    r["hidden_n"] = int(m.group(1)) if m else 0
    os.remove(os.path.join(d, "tests", "zz_hidden.rs"))
    r["rules_ok"] = not r["float"] and not r["unsafe"] and r["cargo_toml_same"] and r["visible_same"]
    r["pass"] = r["rules_ok"] and r["fmt"] and r["clippy"] and r["visible"] and not r["todo_left"]
    return r
BUGS = {"A": ((10,12), ("S1",), None), "B": ((15,17), ("S2",), None),
        "C": ((20,23), ("S3",), r"overflow|saturat|panic|exceed|u32::MAX"),
        "D": ((19,25), ("S3",), r"order|sort|HashMap|determin|iterat|random"),
        "E": ((38,41), ("S5",), r"cast|as i32|negative|wrap|overflow|lower|reduc|truncat|exceed")}
def score_t3(d, L):
    unchanged = sha(os.path.join(d,"src/combat.rs")) == sha(os.path.join(TPL,"t3/src/combat.rs"))
    arr = last_json_array(L["all_text"])
    if not isinstance(arr, list): return {"found": 0, "fp": None, "note": "no array", "unchanged": unchanged}
    found, fps, decoy = set(), 0, False
    for it in arr:
        if not isinstance(it, dict): fps += 1; continue
        try: line = int(it.get("line", -1))
        except Exception: line = -1
        summ = str(it.get("summary", "")) + " " + str(it.get("spec", ""))
        hit = None
        for k, ((lo, hi), specs, kw) in BUGS.items():
            if k in found: continue
            if lo <= line <= hi and (kw is None or re.search(kw, summ, re.I)): hit = k; break
        if hit: found.add(hit)
        else:
            fps += 1
            if 29 <= line <= 34 or "S4" in summ: decoy = True
    return {"found": len(found), "which": "".join(sorted(found)), "fp": fps, "decoy_fp": decoy, "unchanged": unchanged}
rows = []
for meta_p in sorted(glob.glob(os.path.join(S, os.environ.get("BAKE_OUT","bake"), "logs", "*.meta.json"))):
    meta = json.load(open(meta_p)); L = parse_log(meta["log"]); t = meta["task"]
    sc = {"t1": score_t1, "t2": score_t2, "t3": score_t3}[t](meta["dir"], L)
    rows.append({**meta, **{k: L[k] for k in ("steps", "tools", "tok", "cost", "err")}, "score": sc, "answer": L["text"][-300:]})
json.dump(rows, open(os.path.join(S, os.environ.get("BAKE_OUT","bake"), "scores.json"), "w"), indent=1)
for r in rows:
    sc = r["score"]; tk = r["tok"]
    if r["task"] == "t1": s = f"pass={sc.get('pass')} fp={sc.get('fp')} fn={sc.get('fn')} unch={sc.get('unchanged')}"
    elif r["task"] == "t2": s = f"pass={sc['pass']} hidden={sc['hidden']} vis={sc['visible']} clippy={sc['clippy']} fmt={sc['fmt']} rules={sc['rules_ok']} float={sc['float']} todo={sc['todo_left']}"
    else: s = f"found={sc.get('found')}/5 [{sc.get('which','')}] fp={sc.get('fp')} decoy={sc.get('decoy_fp')} unch={sc.get('unchanged')}"
    print(f"{r['task']} {r['model']:44} rc={str(r['rc']):7} {r['secs']:6.0f}s steps={r['steps']:3} in={tk['input']//1000}k+{tk['cache_read']//1000}kc out={tk['output']//1000}k ${r['cost']:.3f} | {s} {('ERR '+r['err'][:80]) if r['err'] else ''}")

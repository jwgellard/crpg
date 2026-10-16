"""Run OpenCode models on sandboxed bake-off tasks. Usage: bake.py <scratch> <task,...> <model> [model...]"""
import json, os, shutil, subprocess, sys, time, threading, concurrent.futures as cf
S = sys.argv[1]; TASKS = sys.argv[2].split(","); MODELS = sys.argv[3:]
OC = shutil.which("opencode")
# Windows: the npm shim is a .cmd, and cmd.exe would parse <, >, | and & inside the prompt.
_exe = os.path.join(os.path.dirname(OC or ""), "node_modules", "opencode-ai", "bin", "opencode.exe")
if os.path.exists(_exe): OC = _exe
PROMPTS = {
 "t1": "Answer from the files in this directory only. In tools/lint/deps.py, the ALLOWED table says which workspace crates each crate may depend on. Which crates are allowed to depend directly on crpg-sim? Reply with one line only: a JSON array of the crate names, sorted alphabetically. Do not modify any file.",
 "t2": "Read AGENTS.md and src/lib.rs. Implement the three todo!() functions in src/lib.rs exactly as their doc comments specify. Follow AGENTS.md. Finish when `cargo test`, `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` all pass. Then reply with a two-line report: status (done or blocked) and a one-sentence summary.",
 "t3": "Review src/combat.rs against SPEC.md. Do not modify any file. Report only real defects where the code violates SPEC.md: no style issues, no suggestions. Reply with a JSON array only, one object per defect: {\"line\": <line number in src/combat.rs>, \"spec\": \"S<n>\", \"summary\": \"<one sentence>\"}. If there are none, reply [].",
}
TIMEOUT = {"t1": 240, "t2": 900, "t3": 300}
CFG = {"$schema": "https://opencode.ai/config.json", "autoupdate": False, "share": "disabled",
       "permission": {"edit": "allow", "bash": "allow", "webfetch": "deny", "websearch": "deny", "external_directory": "deny"}}
lock = threading.Lock(); last_start = [0.0]
def job(task, model):
    name = model.replace("/", "__")
    d = os.path.join(S, os.environ.get("BAKE_OUT","bake"), "runs", task, name)
    if os.path.exists(d): shutil.rmtree(d, ignore_errors=True)
    shutil.copytree(os.path.join(S, "tpl", task), d)
    if task == "t2":
        src = open(os.path.join(d, "src", "lib.rs"), encoding="utf-8").read()
        assert src.count("todo!()") == 3 and sorted(os.listdir(os.path.join(d, "tests"))) == ["visible.rs"], "dirty t2 template"
    cfg = json.loads(json.dumps(CFG))
    if task != "t2": cfg["permission"]["edit"] = "deny"
    json.dump(cfg, open(os.path.join(d, "opencode.json"), "w"))
    log = os.path.join(S, os.environ.get("BAKE_OUT","bake"), "logs", f"{task}__{name}.jsonl")
    for attempt in range(3):
        with lock:
            wait = last_start[0] + 4 - time.time()
            if wait > 0: time.sleep(wait)
            last_start[0] = time.time()
        t0 = time.time()
        with open(log, "w", encoding="utf-8") as f:
            env = dict(os.environ, PWD=d)
            p = subprocess.Popen([OC, "run", "-m", model, "--dir", d, "--format", "json", PROMPTS[task]], cwd=d, env=env,
                                 stdin=subprocess.DEVNULL, stdout=f, stderr=subprocess.STDOUT)
            try: rc = p.wait(timeout=TIMEOUT[task])
            except subprocess.TimeoutExpired:
                subprocess.run(["taskkill", "/T", "/F", "/PID", str(p.pid)], capture_output=True); rc = "TIMEOUT"
        dt = time.time() - t0
        txt = open(log, encoding="utf-8", errors="replace").read()
        if "database is locked" in txt and attempt < 2: time.sleep(5); continue
        break
    meta = {"task": task, "model": model, "rc": rc, "secs": round(dt, 1), "dir": d, "log": log}
    json.dump(meta, open(log + ".meta.json", "w"))
    print(f"{task} {model:46} rc={rc} {dt:6.1f}s", flush=True)
os.makedirs(os.path.join(S, os.environ.get("BAKE_OUT","bake"), "logs"), exist_ok=True)
jobs = [(t, m) for t in TASKS for m in MODELS]
with cf.ThreadPoolExecutor(int(os.environ.get("BAKE_PAR", "6"))) as ex:
    for fut in [ex.submit(job, t, m) for t, m in jobs]: fut.result()

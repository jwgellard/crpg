#!/usr/bin/env python3
"""Mutant driver for T040b (crates/crpg-server/src/main.rs, the whole file).

Applies one mutant at a time by exact text replacement (CRLF-aware, the anchor
must occur exactly once), runs

  cargo test -p crpg-server --locked -j 2 --bin crpg-server --test server_binary

with CARGO_INCREMENTAL=0 (timeout 900 s), records one JSON line per mutant in
results.jsonl, and restores the original bytes in a `finally`.

usage: run_mutants.py [--check] [--full] [ids...]
       --check  applies and restores every anchor without cargo (no results).
       --full   runs the full `cargo test -p crpg-server --locked -j 2` instead
                and writes results-full.jsonl (for survivors of the first run).
       no ids: all rows, in table order.
"""
import json
import os
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SRC = ROOT / "crates/crpg-server/src"
OUT = ROOT / "reviews/T040b/results.jsonl"
OUT_FULL = ROOT / "reviews/T040b/results-full.jsonl"
TIMEOUT = 900
TIMEOUT_FULL = 2400
DUMP = os.environ.get("MUTANT_DUMP")

F = "main.rs"
FILES = [F]

M = {}


def m(mid, desc, item, edits):
    M[mid] = (desc, item, edits)


def e(old, new):
    return [(F, old, new)]


# ---- B-10: command line ------------------------------------------------------
m("M01", "main.rs parse_args: `--help` honoured only as the first argument (`args.iter().take(1).any`)",
  "B-10 help anywhere (S1 control `--bogus --help`)",
  e('if args.iter().any(|a| a == "--help") {', 'if args.iter().take(1).any(|a| a == "--help") {'))

m("M02", "main.rs parse_args: `--version` tested before `--help` (the two blocks swapped)",
  "B-10 help wins over version (S1)",
  e('    if args.iter().any(|a| a == "--help") {\n'
    '        return Ok(Command::Help);\n'
    '    }\n'
    '    if args.iter().any(|a| a == "--version") {\n'
    '        return Ok(Command::Version);\n'
    '    }\n',
    '    if args.iter().any(|a| a == "--version") {\n'
    '        return Ok(Command::Version);\n'
    '    }\n'
    '    if args.iter().any(|a| a == "--help") {\n'
    '        return Ok(Command::Help);\n'
    '    }\n'))

m("M03", "main.rs USAGE: second usage line loses ` | --version`",
  "B-10 usage text (S1)",
  e('       crpg-server --help | --version\\n";', '       crpg-server --help\\n";'))

m("M04", "main.rs parse_args: extra argument reported as the path (`unknown(&args[1])`)",
  "B-10 unknown argument names the extra (S2)",
  e('[_, _, extra, ..] => Err(unknown(extra)),', '[_, _, _extra, ..] => Err(unknown(&args[1])),'))

m("M05", "main.rs main: usage failure exits 1 instead of 2",
  "B-10 exit 2 (S2)",
  e('eprintln!("crpg-server: usage: {text}");\n            std::process::exit(2);',
    'eprintln!("crpg-server: usage: {text}");\n            std::process::exit(1);'))

# ---- B-11 start-up ----------------------------------------------------------
m("M06", "main.rs serve stage 1: area range upper bound exclusive (`-32768..32767`)",
  "B-11.1 area range boundary (S3 uses 40000 only)",
  e('let range = -32768..=32767;', 'let range = -32768..32767;'))

m("M07", "main.rs serve stage 1: only `area.min` is range-checked (`.chain(&area.max)` dropped)",
  "B-11.1 area max checked (S3 checks min only)",
  e('area.min.iter().chain(&area.max).any(|v| !range.contains(v))', 'area.min.iter().any(|v| !range.contains(v))'))

m("M08", "main.rs serve stage 2: log file opened truncating (`.write(true).truncate(true)` for `.append(true)`)",
  "B-11.2 append (S5 prefilled `old\\n`)",
  e('            .append(true)\n', '            .write(true)\n            .truncate(true)\n'))

m("M09", "main.rs serve stage 2: log file opened without create (`.create(false)`)",
  "B-11.2 create (S5 prefilled)",
  e('            .create(true)\n', '            .create(false)\n'))

m("M10", "main.rs serve stage 2: log open failure reported at stage `config`",
  "B-11.2 stage word `log`",
  e('.map_err(fail("log"))?;', '.map_err(fail("config"))?;'))

m("M11", "main.rs serve stage 3: file-read failure detail drops the `{what} file: ` prefix",
  "B-11.3 read-error detail (S3 missing config only? S11 PEM only)",
  e('        fs::read(resolve(base, name)).map_err(|e| Failure {\n'
    '            stage: "certificate",\n'
    '            detail: format!("{what} file: {e}"),\n',
    '        fs::read(resolve(base, name)).map_err(|e| Failure {\n'
    '            stage: "certificate",\n'
    '            detail: format!("{e}{}", &what[..0]),\n'))

m("M12", "main.rs serve stage 3: PEM decode failure always labelled `certificate file` (key errors too)",
  "B-11.3 `private_key file:` label (S11 m2)",
  e('        move |e: rustls_pki_types::pem::Error| Failure {\n'
    '            stage: "certificate",\n'
    '            detail: format!("{what} file: {e}"),\n',
    '        move |e: rustls_pki_types::pem::Error| Failure {\n'
    '            stage: "certificate",\n'
    '            detail: format!("certificate file: {e}{}", &what[..0]),\n'))

m("M13", "main.rs serve stage 4: empty invitations array no longer refused (`&& false`)",
  "B-11.4 no invitations (S3)",
  e('if entries.is_empty() {', 'if entries.is_empty() && false {'))

m("M14", "main.rs serve stage 4: MAX_INVITATIONS entries refused (`>` -> `>=`)",
  "B-11.4 too many boundary",
  e('if entries.len() > MAX_INVITATIONS {', 'if entries.len() >= MAX_INVITATIONS {'))

m("M15", "main.rs parse_credential: uppercase hex digits accepted (`b'0'..=b'9' | b'A'..=b'F'`)",
  "B-11.4 lowercase only",
  e("b'0'..=b'9' => Some(b - b'0'),", "b'0'..=b'9' | b'A'..=b'F' => Some(b.wrapping_sub(b'0')),"))

m("M16", "main.rs serve stage 4: credential error always names entry 0",
  "B-11.4 entry index (S3 uses entry 0 only)",
  e('fail("invitations")(format!("entry {i}: credential is not 64 lowercase hex"))',
    'fail("invitations")(format!("entry {}: credential is not 64 lowercase hex", i * 0))'))

m("M17", "main.rs serve: stage 5 (ai) block moved to after stage 6 (incarnation spent first)",
  "B-11 stages 1-5 never touch the sidecar (S3/S8)",
  [(F,
    '    // 5. ai.\n'
    '    let ai = match &config.ai {\n'
    '        Some(name) => {\n'
    '            let file: AiFile = read_json(&resolve(base, name)).map_err(fail("ai"))?;\n'
    '            let sides = Sides::new(\n'
    '                file.sides\n'
    '                    .into_iter()\n'
    '                    .map(|s| (s.entity, SideId(s.side)))\n'
    '                    .collect(),\n'
    '            )\n'
    '            .map_err(fail("ai"))?;\n'
    '            Some((sides, file.controlled))\n'
    '        }\n'
    '        None => None,\n'
    '    };\n'
    '\n'
    '    // 6. incarnation.\n'
    '    let incarnation = spend_incarnation(&save)?;\n',
    '    // 6. incarnation.\n'
    '    let incarnation = spend_incarnation(&save)?;\n'
    '\n'
    '    // 5. ai.\n'
    '    let ai = match &config.ai {\n'
    '        Some(name) => {\n'
    '            let file: AiFile = read_json(&resolve(base, name)).map_err(fail("ai"))?;\n'
    '            let sides = Sides::new(\n'
    '                file.sides\n'
    '                    .into_iter()\n'
    '                    .map(|s| (s.entity, SideId(s.side)))\n'
    '                    .collect(),\n'
    '            )\n'
    '            .map_err(fail("ai"))?;\n'
    '            Some((sides, file.controlled))\n'
    '        }\n'
    '        None => None,\n'
    '    };\n')])

m("M18", "main.rs serve stage 6: sidecar written after the load succeeds, not before it",
  "B-11.6 sidecar written before the load (S4)",
  [(F, '    write().map_err(fail("incarnation"))?;\n    Ok(next)\n',
    '    let _ = &write;\n    Ok(next)\n'),
   (F, '        .map_err(fail("load"))?\n        .host;\n',
    '        .map_err(fail("load"))?\n        .host;\n'
    '    fs::write(sidecar_path(&save), format!("{incarnation}\\n")).map_err(fail("incarnation"))?;\n')])

m("M19", "main.rs spend_incarnation: an absent sidecar counts as 1 (`NotFound => 1`)",
  "B-11.6 absent sidecar spends 1 (S4)",
  e('Err(error) if error.kind() == io::ErrorKind::NotFound => 0,', 'Err(error) if error.kind() == io::ErrorKind::NotFound => 1,'))

m("M20", "main.rs spend_incarnation: u64::MAX no longer exhausted (`wrapping_add` in a `Some`)",
  "B-11.6 exhausted",
  e('    let next = last\n        .checked_add(1)\n        .ok_or_else(|| fail("incarnation")("exhausted"))?;\n',
    '    let next = Some(last.wrapping_add(1))\n        .ok_or_else(|| fail("incarnation")("exhausted"))?;\n'))

m("M21", "main.rs resolve: relative paths no longer joined to the config directory",
  "B-11 relative paths (every S-test runs with another cwd)",
  e('    base.join(path)\n', '    let _ = base;\n    path.to_path_buf()\n'))

m("M22", "main.rs serve stage 8: area corners swapped (`corner(area.max), corner(area.min)`)",
  "B-11.8 area min/max order (S7)",
  e('host.bind_area_bounds(corner(area.min), corner(area.max))', 'host.bind_area_bounds(corner(area.max), corner(area.min))'))

m("M23", "main.rs serve stage 9: the unpause call removed",
  "B-11.9 unpause",
  e('    host.unpause(START_MS).map_err(fail("unpause"))?;\n', '    let _ = &host;\n'))

m("M24", "main.rs serve stage 16: start line `invitations=` constant 1",
  "B-11.16 invitations count (S5 has one entry)",
  e('    let count = invitations.len();\n', '    let count = 1usize.min(invitations.len().max(1));\n'))

m("M25", "main.rs serve stage 13: AI hook never installed (`drop(hook)` for `set_ai`)",
  "B-14 hook installed (S8 start-up only)",
  e('        hosting.set_ai(Some(hook));\n', '        drop(hook);\n'))

# ---- B-13 observer -----------------------------------------------------------
m("M26", "main.rs observer: only the first rejection of a report is logged (`.take(1)`)",
  "B-13 every element (S5 has one per report)",
  e('for rejection in report.rejections() {', 'for rejection in report.rejections().into_iter().take(1) {'))

m("M27", "main.rs observer: `t=` is the constant 0, not now_ms",
  "B-13 t={now_ms} (S5, T040-E2 reading)",
  e('observer_log.line(&format!("crpg-server: reject t={now_ms} {rejection}"));',
    'observer_log.line(&format!("crpg-server: reject t={} {rejection}", now_ms * 0));'))

m("M28", "main.rs observer: a line is logged even for an empty rejection list",
  "B-13 nothing logged for an empty list (S5 control)",
  e('        for rejection in report.rejections() {\n',
    '        if report.rejections().is_empty() {\n'
    '            observer_log.line(&format!("crpg-server: reject t={now_ms} none"));\n'
    '        }\n'
    '        for rejection in report.rejections() {\n'))

# ---- B-15 console ------------------------------------------------------------
m("M29", "main.rs console: blank line is not ignored (`else if !command.is_empty()` -> `else`)",
  "B-15 empty line ignored",
  e('        } else if !command.is_empty() {\n', '        } else {\n'))

m("M30", "main.rs console: only trailing whitespace trimmed (`trim_end_matches`)",
  "B-15 padded input",
  e('text.trim_matches(|c: char| c.is_ascii_whitespace())', 'text.trim_end_matches(|c: char| c.is_ascii_whitespace())'))

m("M31", "main.rs console: `quit` matched as a prefix (`starts_with(\"quit\")`)",
  "B-15 exact quit (S5)",
  e('if command == "quit" {', 'if command.starts_with("quit") {'))

m("M32", "main.rs console: unknown-command line goes to stderr only (`eprintln!`), not the log file",
  "B-15 unknown command via Log (S5 log-file equality)",
  e('log.line(&format!("crpg-server: console: unknown command {command}"));',
    'eprintln!("crpg-server: console: unknown command {command}");'))

m("M33", "main.rs console: finished-loop check disabled (`if false`)",
  "B-15 die on a finished loop",
  e('Some(running) if running.handle.is_finished() => die(guard),', 'Some(running) if false && running.handle.is_finished() => die(guard),'))

# ---- B-17 end of input -------------------------------------------------------
m("M34", "main.rs console: end of input quits (`Ok(0) | Err(_) => finish(shared, Reason::Quit)`)",
  "B-17 end of input keeps serving (S6)",
  e('Ok(0) | Err(_) => break,', 'Ok(0) | Err(_) => finish(shared, Reason::Quit),'))

# ---- B-16 finish -------------------------------------------------------------
m("M35", "main.rs finish: `shutdown(true)` for `shutdown(false)`",
  "B-16.3 shutdown(false)",
  e('.into_quic().shutdown(false);', '.into_quic().shutdown(true);'))

m("M36", "main.rs finish: the save is skipped (`Ok::<(), String>(())` for `save_host_file`)",
  "B-16.3 finish saves (S5)",
  e('match save_host_file(&save, &shutdown.host, &identity) {',
    'match { let _ = (&save, &shutdown.host, &identity); Ok::<(), String>(()) } {'))

m("M37", "main.rs finish: the stop line always says `reason=quit`",
  "B-16.3 reason word (S10 signal; S5 is quit)",
  e('                reason.word(),\n', '                { let _ = reason.word(); "quit" },\n'))

m("M38", "main.rs finish: a failed save exits 0",
  "B-16.3 save error exit 1",
  e('log.line(&format!("crpg-server: error save: {error}"));\n            std::process::exit(1);',
    'log.line(&format!("crpg-server: error save: {error}"));\n            std::process::exit(0);'))

m("M39", "main.rs finish: guard dropped right after the take, before the stop",
  "B-16 exit while holding the lock",
  e('    let Some(running) = guard.take() else {\n        std::process::exit(1);\n    };\n    let Running {\n',
    '    let Some(running) = guard.take() else {\n        std::process::exit(1);\n    };\n    drop(guard);\n    let Running {\n'))

m("M40", "main.rs finish: a loop error no longer exits (falls through to the save)",
  "B-16.2 loop error exit 1, no save",
  e('        log.line(&format!("crpg-server: error loop: {error}"));\n        std::process::exit(1);\n    }\n    let shutdown',
    '        log.line(&format!("crpg-server: error loop: {error}"));\n    }\n    let shutdown'))

m("M41", "main.rs serve stage 15: handler stops with Reason::Quit instead of Reason::Signal",
  "B-11.15 handler reason (S10 only)",
  e('finish(&for_handler, Reason::Signal)', 'finish(&for_handler, Reason::Quit)'))

# ---- B-19 PEM ----------------------------------------------------------------
m("M42", "main.rs pem_block: leading whitespace not dropped (`position(|_| true)`)",
  "B-19 whitespace drop (S11 `\\r\\n  ` prefix)",
  e('.position(|b| !b.is_ascii_whitespace())', '.position(|_| true)'))

m("M43", "main.rs pem_block: the whole input (with its whitespace) is handed to the decoder",
  "B-19 decode the block (S11 prefix)",
  e('block.starts_with(b"-----BEGIN").then_some(block)', 'block.starts_with(b"-----BEGIN").then_some(bytes)'))

# ---- Log tee -----------------------------------------------------------------
m("M44", "main.rs Log::line: the log file gets the text without its newline",
  "tee same lines as stderr (S5 log equality)",
  e('let _ = file.write_all(&bytes);', 'let _ = file.write_all(text.as_bytes());'))

m("M45", "main.rs main: start-up failure line goes to stderr only (`eprintln!`), not the log file",
  "B-11 failure line through Log (log file once open)",
  e('                    log.line(&format!(\n'
    '                        "crpg-server: error {}: {}",\n'
    '                        failure.stage, failure.detail\n'
    '                    ));\n',
    '                    eprintln!("crpg-server: error {}: {}", failure.stage, failure.detail);\n'))




def read_originals():
    return {name: (SRC / name).read_bytes() for name in FILES}


def apply(originals, edits):
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
            raise ValueError(f"{name}: anchor matched {count} times: {old[:70]!r}")
        out[name] = text.replace(old, new, 1).encode("utf-8")
    return out


UNION = ["--bin", "crpg-server", "--test", "server_binary"]


def run_cargo(full):
    env = dict(os.environ, CARGO_INCREMENTAL="0")
    cmd = ["cargo", "test", "-p", "crpg-server", "--locked", "-j", "2"]
    if not full:
        cmd += UNION
    timeout = TIMEOUT_FULL if full else TIMEOUT
    p = subprocess.Popen(cmd, cwd=ROOT, env=env, stdout=subprocess.PIPE,
                         stderr=subprocess.STDOUT, text=True, encoding="utf-8",
                         errors="replace")
    try:
        out, _ = p.communicate(timeout=timeout)
        return p.returncode, out, False
    except subprocess.TimeoutExpired:
        subprocess.run(["taskkill", "/F", "/T", "/PID", str(p.pid)], capture_output=True)
        try:
            out, _ = p.communicate(timeout=30)
        except Exception:
            out = ""
        return -1, out, True


def classify(rc, out, timed_out):
    if timed_out:
        return "killed", ["<timeout>"]
    if "could not compile" in out or any(ln.startswith("error[") for ln in out.splitlines()):
        return "compile-error", []
    if "test result:" not in out:
        return "compile-error", []
    failed = [ln.split()[1] for ln in out.splitlines()
              if ln.startswith("test ") and ln.rstrip().endswith("FAILED")]
    if rc == 0 and not failed:
        return "survived", []
    return "killed", failed


def main():
    args = sys.argv[1:]
    check = "--check" in args
    full = "--full" in args
    ids = [a for a in args if not a.startswith("--")] or list(M)
    out_path = OUT_FULL if full else OUT
    originals = read_originals()
    lines = {}
    if not check:
        out_path.parent.mkdir(parents=True, exist_ok=True)
        if out_path.exists():
            for line in out_path.read_text(encoding="utf-8").splitlines():
                if line.strip():
                    lines[json.loads(line)["id"]] = line
        for mid in ids:
            lines.pop(mid, None)

    def flush():
        out_path.write_text(
            "".join(lines[mid] + "\n" for mid in M if mid in lines), encoding="utf-8")

    try:
        for mid in ids:
            desc, item, edits = M[mid]
            rec = {"id": mid, "desc": desc, "contract": item,
                   "edits": [{"file": f, "old": o, "new": n} for f, o, n in edits]}
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
                    (SRC / name).write_bytes(data)
                rc, out, to = run_cargo(full)
            finally:
                for name in mutated:
                    (SRC / name).write_bytes(originals[name])
            if DUMP:
                Path(DUMP).mkdir(parents=True, exist_ok=True)
                (Path(DUMP) / f"{mid}.cargo.log").write_text(out, encoding="utf-8")
            status, failed = classify(rc, out, to)
            rec.update(status=status, failed_tests=failed,
                       seconds=round(time.time() - t0, 1))
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
            (SRC / name).write_bytes(data)


if __name__ == "__main__":
    main()

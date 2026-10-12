#!/usr/bin/env python3
"""Run the WHOLE suite and print a STATUS MATRIX: every test file x every environment.

Cells: GREEN (exit 0), RED (exit 1), BLOCKED (exit 3), CRASH (other), or "n/a" if not run here.
The matrix is STATE ONLY. dev+mock and prod are ROWS of the same table, not alternatives.
A footer records: a case GREEN in dev AND GREEN in prod is a T0 issue.

  python tests/matrix.py dev        # the files that run in dev (no external dep)
  python tests/matrix.py prod       # the files that need a real environment (registry/provider)
  python tests/matrix.py            # both columns, one table
"""
import os, re, subprocess, sys

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(HERE)
G = "\033[32m"; R = "\033[31m"; Y = "\033[33m"; D = "\033[2m"; B = "\033[1m"; X = "\033[0m"

def files():
    return [l.strip() for l in open(os.path.join(HERE, "all-files.txt"), encoding="utf-8") if l.strip()]

def last_error(out):
    lines = [l.strip() for l in out.splitlines() if l.strip()]
    for l in reversed(lines):
        if l.startswith("FAIL") or "error" in l.lower() or "Traceback" in l or "blocked" in l.lower():
            return l[:100]
    return lines[-1][:100] if lines else ""

def classify(code):
    return {0: "GREEN", 1: "RED", 3: "BLOCKED"}.get(code, "CRASH")

def run(f, profile):
    env = dict(os.environ)
    if profile == "prod":
        env["AGENT_HUB_PROFILE"] = "release"   # the release binary: official address only
    else:
        env.pop("AGENT_HUB_PROFILE", None)     # dev binary
    try:
        p = subprocess.run([sys.executable, os.path.join(HERE, f)], cwd=REPO,
                           env=env, capture_output=True, text=True, timeout=900)
        code = p.returncode; out = (p.stdout or "") + (p.stderr or "")
    except subprocess.TimeoutExpired:
        code = -9; out = "TIMEOUT"
    return code, classify(code), out

def color(state):
    return {"GREEN": G, "RED": R, "BLOCKED": Y, "CRASH": R, "n/a": D}.get(state, "") + state + X

def main():
    mode = sys.argv[1] if len(sys.argv) > 1 else "both"
    cols = ["dev", "prod"] if mode == "both" else [mode]
    fs = files()
    rows = []
    for f in fs:
        cells = []
        errs = {}
        for c in cols:
            code, state, out = run(f, c)
            cells.append(state)
            errs[c] = last_error(out)
        rows.append((f, cells, errs))
        print(f"{f:70} " + " ".join(f"{c}=" + color(s) for c, s in zip(cols, cells)))
    # the table
    print(f"\n{B}=== STATUS MATRIX ==={X}   columns: {cols}")
    w = max(len(r[0]) for r in rows)
    print(f"{B}{'test'.ljust(w)} | " + " | ".join(c.ljust(8) for c in cols) + X)
    print("-" * (w + 12 * len(cols)))
    for f, cells, errs in rows:
        errtxt = " ; ".join(f"{c}:{errs[c]}" for c in cols if errs[c])
        print(f"{f.ljust(w)} | " + " | ".join(f"{color(s):20}" for s in cells) + (f"  {D}{errtxt}{X}" if errtxt else ""))
    green = sum(1 for _, cells, _e in rows for s in cells if s == "GREEN")
    red = sum(1 for _, cells, _e in rows for s in cells if s == "RED")
    blk = sum(1 for _, cells, _e in rows for s in cells if s == "BLOCKED")
    print(f"{B}summary: {G}{green} GREEN{X}, {R}{red} RED{X}, {Y}{blk} BLOCKED{X} across {len(rows)} files x {len(cols)} env")
    print(f"{D}all-green is NOT correctness. A case GREEN in dev AND GREEN in prod is a T0 issue "
          f"(the environments are not actually distinguished).{X}")

if __name__ == "__main__":
    main()

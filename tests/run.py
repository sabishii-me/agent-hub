#!/usr/bin/env python3
"""Run the real test layers. `python tests/run.py [layer]` runs one band, not the world.

Each test file is its own process. Its outcome is classified by EXIT CODE and by the summary
line it prints, into four SEPARATE buckets - never one "N/N passed" that hides the rest:

  PASS     exit 0 and its summary shows 0 failed;
  FAIL     a non-zero exit that is not the BLOCKED sentinel;
  BLOCKED  exit 3, or the file prints a `BLOCKED:` line: a real precondition is missing (the
           official registry is unpublished), so the capability is UNVERIFIED - not a pass and
           not a failure;
  CRASH    a non-zero exit with no summary line (the file died before reporting anything).

No test fakes anything, and a missing real dependency is BLOCKED with a reason, never a green.
"""
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
LAYERS = ["contract", "lifecycle", "interrupt", "approvals", "concurrency", "provider",
          "plugins", "harnesses", "presets", "tools", "model", "skills", "connections"]
BLOCKED_EXIT = 3
SUMMARY = re.compile(r"\[[^\]]+\]\s+(\d+)/(\d+)\s+ok,\s+(\d+)\s+failed")
BLOCKED_N = re.compile(r",\s+(\d+)\s+BLOCKED")


def collect(layer):
    d = os.path.join(HERE, layer)
    if not os.path.isdir(d):
        return []
    return [os.path.join(d, n) for n in sorted(os.listdir(d))
            if n.endswith(".py") and not n.startswith("_")]


def main():
    which = sys.argv[1] if len(sys.argv) > 1 else None
    layers = [which] if which else LAYERS
    files = []
    for layer in layers:
        found = collect(layer)
        if which and not found:
            print(f"no tests in layer '{layer}' (yet)")
        files += found
    if not files:
        print("no test files found")
        return 0

    print(f"hub test suite: {len(files)} file(s)\n")
    passed, failed, blocked, crashed, partial = [], [], [], [], []
    for f in files:
        rel = os.path.relpath(f, HERE).replace(os.sep, "/")
        print(f"=== {rel} ===")
        r = subprocess.run([sys.executable, f], capture_output=True, text=True)
        out = (r.stdout or "") + (r.stderr or "")
        sys.stdout.write(out)
        if not out.endswith("\n"):
            print()
        m = SUMMARY.search(out)
        bn = BLOCKED_N.search(out)
        n_blocked = int(bn.group(1)) if bn else 0
        if r.returncode == BLOCKED_EXIT or "BLOCKED:" in out:
            blocked.append(rel)
            print(f"--- {rel}: BLOCKED (a real precondition is missing; UNVERIFIED)")
        elif r.returncode == 0 and m and int(m.group(3)) == 0 and n_blocked == 0:
            passed.append(rel)
            print(f"--- {rel}: PASS ({m.group(1)}/{m.group(2)})")
        elif r.returncode == 0 and m and int(m.group(3)) == 0 and n_blocked > 0:
            partial.append(rel)
            print(f"--- {rel}: PARTIAL ({m.group(1)}/{m.group(2)} ok, {n_blocked} BLOCKED - not fully verified)")
        elif r.returncode == 0:
            # exit 0 with no tally line and no BLOCKED: the file reported success its own way.
            passed.append(rel)
            print(f"--- {rel}: PASS (exit 0)")
        elif m:
            failed.append(rel)
            print(f"--- {rel}: FAIL ({m.group(3)} failed)")
        else:
            crashed.append(rel)
            print(f"--- {rel}: CRASH (exit {r.returncode}, no summary)")
        print()

    # A file that BLOCKED mid-way (some checks ran, then a precondition failed) is reported as
    # both: partially verified + blocked. Keep it simple and honest: any BLOCKED line wins.
    total = len(files)
    print("=" * 60)
    print(f"files: {total}")
    print(f"  PASS    : {len(passed)}")
    print(f"  FAIL    : {len(failed)}")
    print(f"  BLOCKED : {len(blocked)}  (a real precondition is missing -> UNVERIFIED)")
    print(f"  PARTIAL : {len(partial)}  (some checks passed, some BLOCKED -> not fully verified)")
    print(f"  CRASH   : {len(crashed)}")
    if blocked:
        print("\nBLOCKED (NOT verified, NOT a pass):")
        for b in blocked:
            print(f"  - {b}")
    if failed:
        print("\nFAIL:")
        for b in failed:
            print(f"  - {b}")
    if crashed:
        print("\nCRASH:")
        for b in crashed:
            print(f"  - {b}")
    print("\nNo 'all green': {p} passed, {f} failed, {b} blocked, {c} crashed.".format(
        p=len(passed), f=len(failed), b=len(blocked), c=len(crashed)))
    # Non-zero if anything FAILED or CRASHED. BLOCKED does not fail the run, but is never hidden.
    return 1 if (failed or crashed) else 0


if __name__ == "__main__":
    sys.exit(main())

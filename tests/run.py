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
        print("no test files found - nothing was verified; exit non-zero")
        return 2

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
        n_failed = int(m.group(3)) if m else None
        saw_blocked = (r.returncode == BLOCKED_EXIT or "BLOCKED:" in out)
        # PRIORITY: a FAILURE or a CRASH can never be hidden by a BLOCKED line, and a non-zero
        # exit that is not the BLOCKED sentinel is never a PASS.
        if r.returncode not in (0, BLOCKED_EXIT):
            # A hard error (uncaught traceback, assertion, timeout) OUTSIDE the BLOCKED sentinel.
            if n_failed and n_failed > 0:
                failed.append(rel)
                print(f"--- {rel}: FAIL ({n_failed} failed)")
            else:
                crashed.append(rel)
                print(f"--- {rel}: CRASH (exit {r.returncode})")
        elif n_failed and n_failed > 0:
            # exit 0 but the file's OWN summary says checks FAILED -> FAIL, regardless of exit code
            # or any BLOCKED line. This closes 'failing assertion exits 0'.
            failed.append(rel)
            print(f"--- {rel}: FAIL ({n_failed} failed, reported in its summary)")
        elif saw_blocked:
            # The file printed a BLOCKED line: the whole file is blocked (it stopped at a missing
            # precondition). Its earlier ok checks are NOT a pass for the blocked capability.
            if n_blocked == 0:
                # a bare 'BLOCKED:' with no summary count - still blocked.
                blocked.append(rel)
            else:
                blocked.append(rel)
            print(f"--- {rel}: BLOCKED (a real precondition is missing; UNVERIFIED)")
        elif n_blocked > 0:
            # Some checks passed AND some are blocked (the file finished with ok + a BLOCKED count).
            # NOT a clean pass: PARTIAL.
            partial.append(rel)
            print(f"--- {rel}: PARTIAL ({m.group(1) if m else '?'}/{m.group(2) if m else '?'} ok, {n_blocked} BLOCKED - not fully verified)")
        elif m and int(m.group(1)) == 0 and int(m.group(2)) == 0:
            # 0/0 ok is NOT a pass: nothing was asserted, so nothing was proven.
            crashed.append(rel)
            print(f"--- {rel}: EMPTY (0/0 checks - nothing was proven)")
        elif m and int(m.group(2)) > 0:
            passed.append(rel)
            print(f"--- {rel}: PASS ({m.group(1)}/{m.group(2)})")
        elif r.returncode == 0 and not m and not saw_blocked:
            # exit 0, no summary, no BLOCKED: a file that reported success its own way, but with
            # no tally we cannot see a count - treat as UNVERIFIED, not PASS.
            crashed.append(rel)
            print(f"--- {rel}: NO-SUMMARY (exit 0, no tally - UNVERIFIED)")
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
    print("\nNo 'all green': {p} passed, {f} failed, {b} blocked, {c} crashed/unverified.".format(
        p=len(passed), f=len(failed), b=len(blocked), c=len(crashed)))
    # Non-zero if anything FAILED, CRASHED, or if NOTHING was verified (an all-BLOCKED run must
    # NOT look like success to a CI that reads only the exit code). A run with zero PASS and zero
    # FAIL proved nothing, so it exits non-zero too.
    if failed or crashed:
        return 1
    if len(passed) == 0:
        print("no test VERIFIED anything (0 passed) - exit non-zero so this is never read as success")
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())

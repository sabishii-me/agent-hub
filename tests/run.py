#!/usr/bin/env python3
"""The CURRENT entry: local boundaries only - NOT hub-capability acceptance.

The official plugin registry is UNPUBLISHED, so the plugin-driven user chain cannot be established
and MUST NOT be a default acceptance entry. This runner therefore runs ONLY the files that verify a
LOCAL boundary with NO registry and NO plugin chain, and it says so.

  python tests/run.py                 # the local-boundary entry (no registry, no plugin chain)
  python tests/run.py --materials     # the UNVERIFIED materials (need the published registry):
                                      #   run for information only; their result is NOT a capability
  python tests/run.py --materials <f> # one material file by name

Nothing here is 'hub capabilities passed'. A file's green is a statement about its own narrow check.
No test fakes anything; a missing real precondition is BLOCKED with a reason, never a green.
"""
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))

# The ONLY files admitted to the current entry. Each verifies a LOCAL boundary that needs neither
# the registry nor a plugin chain. Their scope is named for each; none is a plugin-driver capability.
LOCAL_BOUNDARY = [
    "contract/status-surface-openapi-and-models-answer.py",   # the served metadata shapes (route set, models array)
    "contract/the-served-surface-equals-the-contract.py",     # the served ROUTE SET equals the contract
    "concurrency/one-hundred-connections-do-not-time-out.py", # connection count + SSE open, measured
    "plugins/a-caller-cannot-install-from-an-arbitrary-url.py",  # no registry -> unlisted source refused
    "plugins/install-reconciles-against-the-registry.py",     # no registry -> refusal (authorized path BLOCKED)
    "plugins/install-refusals-have-honest-codes.py",          # refusal codes (no registry)
    "plugins/registry-refresh-updates-the-local-registry.py", # refresh ERROR surface (not registry capability)
    "plugins/the-release-hub-error-surface.py",               # release-build error codes (refresh BLOCKED)
    "plugins/the-release-hub-ignores-the-registry-override.py",  # a release binary ignores the env override
    "harnesses/an-uninstalled-harness-cannot-be-used.py",     # empty hub: no harness, session 404
    "skills/skills-crud-over-the-real-surface.py",            # skills CRUD (authoritative read-back)
    "connections/connections-are-crud-and-delete-is-real.py", # connections CRUD (authoritative read-back)
]

BLOCKED_EXIT = 3
SUMMARY = re.compile(r"\[[^\]]+\]\s+(\d+)/(\d+)\s+ok,\s+(\d+)\s+failed")
BLOCKED_N = re.compile(r",\s+(\d+)\s+BLOCKED")


def _all_files():
    out = []
    for root, _, files in os.walk(HERE):
        if "__pycache__" in root or os.path.relpath(root, HERE) == "lib":
            continue
        for n in sorted(files):
            if n.endswith(".py") and not n.startswith("_"):
                out.append(os.path.relpath(os.path.join(root, n), HERE).replace(os.sep, "/"))
    return out


def classify(f):
    rel = f
    r = subprocess.run([sys.executable, os.path.join(HERE, f)], capture_output=True, text=True)
    out = (r.stdout or "") + (r.stderr or "")
    sys.stdout.write(out)
    if not out.endswith("\n"):
        print()
    m = SUMMARY.search(out)
    bn = BLOCKED_N.search(out)
    n_blocked = int(bn.group(1)) if bn else 0
    n_failed = int(m.group(3)) if m else None
    saw_blocked = (r.returncode == BLOCKED_EXIT or "BLOCKED:" in out)
    if r.returncode not in (0, BLOCKED_EXIT):
        if n_failed and n_failed > 0:
            return "FAIL", rel
        return "CRASH", rel
    if n_failed and n_failed > 0:
        return "FAIL", rel
    if saw_blocked:
        return "BLOCKED", rel
    if n_blocked > 0:
        return "PARTIAL", rel
    if m and int(m.group(1)) == 0 and int(m.group(2)) == 0:
        return "EMPTY", rel
    if m and int(m.group(2)) > 0:
        return "PASS", rel
    return "NO-SUMMARY", rel


def main():
    argv = sys.argv[1:]
    materials = "--materials" in argv
    argv = [a for a in argv if a != "--materials"]
    only = argv[0] if argv else None

    if materials:
        files = _all_files()
        if only:
            files = [f for f in files if only in f]
    else:
        files = LOCAL_BOUNDARY
        if only:
            files = [f for f in files if only in f]
        print("ENTRY: local boundaries only (NO registry, NO plugin chain).")
        print("This is NOT hub-capability acceptance; the plugin chain is UNVERIFIED.\n")
    if not files:
        print("no files selected; exit non-zero")
        return 2

    buckets = {}
    for f in files:
        print(f"=== {f} ===")
        kind, r = classify(f)
        buckets.setdefault(kind, []).append(r)
        print(f"--- {r}: {kind}")
        print()

    print("=" * 60)
    for k in ("PASS", "PARTIAL", "BLOCKED", "FAIL", "CRASH", "EMPTY", "NO-SUMMARY"):
        if k in buckets:
            print(f"  {k:10}: {len(buckets[k])}")
    if materials:
        print("\n--materials ran the UNVERIFIED set. Its result is NOT a hub-capability result;")
        print("BLOCKED means the real precondition (published registry) is missing.")
    else:
        print("\nThese are LOCAL, NARROW observations only - NOT 'hub capabilities passed'.")
        print("The plugin-driven user chain (install by id -> runtime -> session -> turn) is UNVERIFIED.")
    return 1 if (buckets.get("FAIL") or buckets.get("CRASH")) else 0


if __name__ == "__main__":
    sys.exit(main())

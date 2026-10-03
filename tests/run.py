#!/usr/bin/env python3
"""Run the real test layers. `python tests/run.py [layer]` runs one band, not the world.

A layer is a directory under tests/. With no argument it runs every layer in order. Each
test file is a process; a non-zero exit is a failure. No test fakes anything: a missing real
dependency makes the file SKIP with a reason.
"""
import os
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
LAYERS = ["contract", "lifecycle", "interrupt", "approvals", "concurrency", "provider", "tools", "model", "skills", "connections"]


def collect(layer):
    d = os.path.join(HERE, layer)
    if not os.path.isdir(d):
        return []
    out = []
    for name in sorted(os.listdir(d)):
        if name.endswith(".py") and not name.startswith("_"):
            out.append(os.path.join(d, name))
    return out


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
    failed = 0
    for f in files:
        rel = os.path.relpath(f, HERE).replace(os.sep, "/")
        print(f"=== {rel} ===")
        r = subprocess.run([sys.executable, f])
        if r.returncode != 0:
            failed += 1
        print()
    print(f"{len(files) - failed}/{len(files)} file(s) passed")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())

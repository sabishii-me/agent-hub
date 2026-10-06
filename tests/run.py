#!/usr/bin/env python3
"""The CURRENT entry: a fixed set of PURE-LOCAL files. NOT hub-capability acceptance.

The official plugin registry is UNPUBLISHED and no plugin environment exists. This entry runs
ONLY files that verify a LOCAL boundary with NO registry override, NO local registry server, and
NO external registry request. The plugin-driven user chain is UNVERIFIED.

  python tests/run.py             # pure-local entry (spawns ONLY the files in LOCAL_ONLY)
  python tests/run.py --materials # LISTS the unverified files (path + why) and runs NOTHING
"""
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))

# PURE-LOCAL only: no override, no local registry server, no external registry request.
LOCAL_ONLY = [
    "contract/status-surface-openapi-and-models-answer.py",
    "contract/the-served-surface-equals-the-contract.py",
    "concurrency/one-hundred-connections-do-not-time-out.py",
    "plugins/a-caller-cannot-install-from-an-arbitrary-url.py",
    "plugins/install-reconciles-against-the-registry.py",
    "plugins/install-refusals-have-honest-codes.py",
    "harnesses/an-uninstalled-harness-cannot-be-used.py",
    "skills/skills-crud-over-the-real-surface.py",
    "connections/connections-are-crud-and-delete-is-real.py",
]

# UNVERIFIED materials: need a published registry / a plugin env / a real provider.
# NOT run by this entry; --materials only LISTS them with the reason.
MATERIALS = {
    "contract/the-events-stream-delivers-plugin-changes.py": "needs an installed plugin",
    "lifecycle/a-session-closes-reopens-and-forks-without-touching-its-source.py": "needs an installed plugin",
    "lifecycle/the-session-crud-and-read-through-are-real.py": "needs an installed plugin",
    "lifecycle/the-whole-chain-plugin-to-session-to-tools-over-v1.py": "needs the published registry + a real provider",
    "interrupt/a-cancelled-turn-releases-the-session.py": "needs an installed plugin",
    "interrupt/a-killed-hub-recovers-its-sessions.py": "needs an installed plugin",
    "interrupt/a-provider-that-cannot-be-reached-is-refused-not-faked.py": "needs an installed plugin + a real provider",
    "interrupt/a-provider-that-hangs-does-not-hang-the-turn.py": "needs an installed plugin + a real provider",
    "interrupt/cancel-is-idempotent-and-bounded.py": "needs an installed plugin",
    "interrupt/killing-the-adapter-during-a-cancel-settles-the-turn.py": "needs an installed plugin + a real provider",
    "interrupt/killing-the-adapter-leaves-an-honest-state.py": "needs an installed plugin + a real provider",
    "interrupt/killing-the-adapter-mid-turn-does-not-hang-the-turn.py": "needs an installed plugin + a real provider",
    "interrupt/killing-the-runtime-process-settles-the-turn.py": "needs an installed plugin + a real provider",
    "interrupt/killing-the-whole-tree-then-restarting-recovers.py": "needs an installed plugin",
    "interrupt/send-immediately-after-cancel.py": "needs an installed plugin + a real provider",
    "interrupt/the-network-drops-mid-turn-and-the-turn-settles.py": "needs an installed plugin + a real provider",
    "approvals/an-approval-preset-asks-before-every-tool.py": "needs an installed plugin + a real provider",
    "approvals/denying-an-approval-blocks-the-side-effect.py": "needs an installed plugin + a real provider",
    "approvals/review-remains-switchable-after-a-preset.py": "needs an installed plugin + a real provider",
    "concurrency/a-second-turn-while-one-runs-is-refused.py": "needs an installed plugin + a real provider",
    "concurrency/cancelling-one-session-does-not-disturb-another.py": "needs an installed plugin + a real provider",
    "concurrency/many-sessions-cancel-at-once.py": "needs an installed plugin + a real provider",
    "concurrency/many-sessions-of-one-harness.py": "needs an installed plugin",
    "provider/model-provider-crud-and-types-are-real.py": "needs an installed plugin",
    "plugins/a-hub-killed-mid-remove-finishes-the-removal.py": "needs an installed plugin",
    "plugins/a-plugin-is-installed-listed-enabled-and-removed.py": "needs an installed plugin",
    "plugins/concurrent-install-and-remove.py": "needs an installed plugin",
    "plugins/concurrent-mixed-install-remove.py": "needs an installed plugin",
    "plugins/registry-refresh-updates-the-local-registry.py": "uses a registry override + a local registry server",
    "plugins/the-release-hub-ignores-the-registry-override.py": "uses a registry override + a local canary server",
    "plugins/the-release-hub-error-surface.py": "sends a registry refresh to the official address + runs a RELEASE binary",
    "harnesses/harness-discovery-and-its-capability-gated-routes.py": "needs an installed plugin",
    "presets/a-preset-is-listed-and-applied.py": "needs an installed plugin",
    "presets/plan-mode-is-applied-and-reports.py": "needs an installed plugin",
    "tools/the-model-really-runs-a-tool.py": "needs an installed plugin + a real provider",
    "model/a-real-model-answers-with-a-confirmed-identity.py": "needs an installed plugin + a real provider",
}

# PRODUCT GAPS (not available operations): listed, not tested, not filled in by the suite.
PRODUCT_GAPS = [
    "install by id is NOT implemented: POST /v1/plugins takes a caller url/artifact, not id-only",
    "the official registry release is UNPUBLISHED, so no authoritative plugin source exists",
]

BLOCKED_EXIT = 3
SUMMARY = re.compile(r"\[[^\]]+\]\s+(\d+)/(\d+)\s+ok,\s+(\d+)\s+failed")
BLOCKED_N = re.compile(r",\s+(\d+)\s+BLOCKED")


def classify(f):
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
        return ("FAIL" if (n_failed and n_failed > 0) else "CRASH"), f
    if n_failed and n_failed > 0:
        return "FAIL", f
    if saw_blocked:
        return "BLOCKED", f
    if n_blocked > 0:
        return "PARTIAL", f
    if m and int(m.group(1)) == 0 and int(m.group(2)) == 0:
        return "EMPTY", f
    if m and int(m.group(2)) > 0:
        return "PASS", f
    return "NO-SUMMARY", f


def list_materials():
    print("UNVERIFIED MATERIALS (NOT run by this entry - listed only):")
    print()
    for f, why in sorted(MATERIALS.items()):
        print("  - " + f)
        print("      why not run: " + why)
    print()
    print("PRODUCT GAPS (not operations; not filled in by the suite):")
    for g in PRODUCT_GAPS:
        print("  - " + g)
    print()
    print("Nothing above was executed. The registry is unpublished; the plugin chain is UNVERIFIED.")
    return 0


def main():
    argv = sys.argv[1:]
    if "--materials" in argv:
        return list_materials()
    only = argv[0] if argv else None
    files = LOCAL_ONLY
    if only:
        files = [f for f in files if only in f]
    print("ENTRY: PURE-LOCAL files only (no registry override, no local server, no external refresh).")
    print("This is NOT hub-capability acceptance; the plugin chain is UNVERIFIED.")
    print()
    if not files:
        print("no files selected; exit non-zero")
        return 2
    buckets = {}
    for f in files:
        print("=== " + f + " ===")
        kind, r = classify(f)
        buckets.setdefault(kind, []).append(r)
        print("--- " + r + ": " + kind)
        print()
    print("=" * 60)
    for k in ("PASS", "PARTIAL", "BLOCKED", "FAIL", "CRASH", "EMPTY", "NO-SUMMARY"):
        if k in buckets:
            print("  %-10s %d" % (k, len(buckets[k])))
    print()
    print("These are LOCAL, NARROW observations only - NOT hub capabilities passed.")
    print("The plugin-driven user chain (install by id -> runtime -> session -> turn) is UNVERIFIED.")
    if buckets.get("FAIL") or buckets.get("CRASH"):
        return 1
    if not buckets.get("PASS"):
        print("NOTHING was verified (0 PASS) - exit non-zero")
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())

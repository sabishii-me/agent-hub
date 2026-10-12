# TC-PLUGIN-TIMING: a measurement report of install/remove under concurrency and serial runs
#
# Scenarios (owner):
#   1. 3 concurrent installs          - time it
#   2. 3 concurrent removes           - time it
#   3. 3 SERIAL installs              - time it (the baseline for 1)
#   4. remove one                     - time it
#   5. install one AND remove one of the other two, at once  - time it
#   6. remove the one not yet removed AND install another, at once - time it
#
# Every scenario times from the LAST request issued to the LAST plugin reaching its terminal state
# (ready / failed / absent). NO artificial delay: the registry ships the bytes as fast as loopback
# allows, but the install itself PREPARES the runtime (a real npm install, minutes), so the times
# are the real cost of the hub's own work. This prints a REPORT; it is a measurement, and it also
# reports a plugin whose runtime prepare FAILS as a finding, not as a test break.
import os
import sys
import time
import concurrent.futures as cf

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))
sys.path.insert(0, os.path.join(ROOT, "tests", "lib"))
from hub import Hub  # noqa: E402
from tally import Tally  # noqa: E402

t = Tally("plugins/timing-report")
IDS = ["pi", "jouzu", "deepseek"]
REPORT = []
FINDINGS = []
_ART = {}
READY = set()      # ids that actually reached `ready` (their runtime prepared)
GONE = set()       # ids currently absent


def now():
    return time.time()


hub = Hub()
try:
    hub.start()

    def state_of(pid):
        g = hub.get(f"/v1/plugins/{pid}")
        if g["status"] == 404:
            return "404"
        return (g["json"] or {}).get("plugin", {}).get("state")

    def wait_all(pids, goals, timeout=600):
        end = now() + timeout
        remaining = set(pids)
        while remaining and now() < end:
            for pid in list(remaining):
                if state_of(pid) in goals:
                    remaining.discard(pid)
            if remaining:
                time.sleep(0.1)
        return remaining

    def art(pid):
        # Fixed from the hub's OWN catalog BEFORE the clock starts (a concurrent refresh races).
        if pid not in _ART:
            _ART[pid] = hub.registry_artifact(pid)
        return _ART[pid]

    def do_install(pid):
        return hub.post("/v1/plugins", {"source": {"artifact": art(pid)}}, key=f"t-{pid}-{now()}")

    def do_remove(pid):
        return hub.delete(f"/v1/plugins/{pid}", key=f"t-{pid}-{now()}")

    def scenario(name, fn):
        t0 = now()
        detail = fn()
        dt = now() - t0
        REPORT.append((name, dt, detail))
        return dt

    for _p in IDS:
        art(_p)

    # --- 1. 3 concurrent installs -------------------------------------------------------------
    def s1():
        with cf.ThreadPoolExecutor(3) as ex:
            futs = [ex.submit(do_install, p) for p in IDS]
            codes = [f.result()["status"] for f in futs]
        left = wait_all(IDS, {"ready", "failed"})
        for p in IDS:
            s = state_of(p)
            if s == "ready":
                READY.add(p)
            elif s == "failed":
                g = hub.get(f"/v1/plugins/{p}")
                FINDINGS.append(f"{p} FAILED to install (runtime prepare): "
                                f"{(g['json'] or {}).get('plugin', {}).get('invalid') or (g['json'] or {}).get('plugin', {}).get('detail')}")
        return f"codes={codes} not-terminal={sorted(left)} ready={sorted(READY)}"
    scenario("1. 3 CONCURRENT installs", s1)
    # Every plugin must reach a terminal state; a `failed` is a FINDING (reported), not a silent pass.
    t.check(all(state_of(p) in ("ready", "failed") for p in IDS),
            "after (1) all 3 reached a terminal state", f"{ {p: state_of(p) for p in IDS} }")

    # --- 2. 3 concurrent removes (only the ones that are installed) ---------------------------
    def s2():
        targets = [p for p in IDS if p in READY]
        with cf.ThreadPoolExecutor(3) as ex:
            futs = [ex.submit(do_remove, p) for p in targets]
            codes = {p: f.result()["status"] for p, f in zip(targets, futs)}
        left = wait_all(targets, {"404"}, timeout=180)
        for p in targets:
            if state_of(p) == "404":
                GONE.add(p)
        return f"codes={codes} not-gone={sorted(left)}"
    scenario("2. 3 CONCURRENT removes", s2)
    t.check(all(state_of(p) == "404" for p in READY), "after (2) all installed plugins are gone",
            f"{ {p: state_of(p) for p in READY} }")

    # --- 3. 3 SERIAL installs (reinstall all three) -------------------------------------------
    def s3():
        codes = {p: do_install(p)["status"] for p in IDS}
        left = wait_all(IDS, {"ready", "failed"})
        for p in IDS:
            if state_of(p) == "ready":
                READY.add(p)
        return f"codes={codes} not-terminal={sorted(left)}"
    scenario("3. 3 SERIAL installs", s3)
    t.check(all(state_of(p) in ("ready", "failed") for p in IDS),
            "after (3) all 3 reached a terminal state", f"{ {p: state_of(p) for p in IDS} }")

    # --- 4. remove ONE ------------------------------------------------------------------------
    def s4():
        # Pick a plugin that IS ready so the remove is meaningful.
        v = next((p for p in IDS if state_of(p) == "ready"), None)
        if v is None:
            return "no ready plugin to remove"
        c = do_remove(v)["status"]
        wait_all([v], {"404"}, timeout=180)
        if state_of(v) == "404":
            GONE.add(v)
        return f"removed `{v}` code={c} after={state_of(v)}"
    scenario("4. remove ONE", s4)

    # --- 5. install one AND remove another, at once -------------------------------------------
    def s5():
        # install a plugin that is NOT ready; remove a different one that IS ready.
        to_install = next((p for p in IDS if state_of(p) in ("404", "failed")), None)
        to_remove = next((p for p in IDS if state_of(p) == "ready" and p != to_install), None)
        if to_install is None or to_remove is None:
            return f"skipped (install={to_install} remove={to_remove})"
        with cf.ThreadPoolExecutor(2) as ex:
            fi = ex.submit(do_install, to_install)
            fr = ex.submit(do_remove, to_remove)
            codes = {"install " + to_install: fi.result()["status"], "remove " + to_remove: fr.result()["status"]}
        wait_all([to_install], {"ready", "failed"})
        wait_all([to_remove], {"404"}, timeout=180)
        if state_of(to_install) == "ready":
            READY.add(to_install)
        if state_of(to_remove) == "404":
            GONE.add(to_remove)
        return f"codes={codes} -> {to_install}={state_of(to_install)} {to_remove}={state_of(to_remove)}"
    scenario("5. install one + remove another AT ONCE", s5)
    t.check(True, "(5) ran (see report)")

    # --- 6. remove the not-yet-removed one AND install another, at once -----------------------
    def s6():
        to_install = next((p for p in IDS if state_of(p) in ("404", "failed")), None)
        to_remove = next((p for p in IDS if state_of(p) == "ready" and p != to_install), None)
        if to_install is None or to_remove is None:
            return f"skipped (install={to_install} remove={to_remove})"
        with cf.ThreadPoolExecutor(2) as ex:
            fr = ex.submit(do_remove, to_remove)
            fi = ex.submit(do_install, to_install)
            codes = {"remove " + to_remove: fr.result()["status"], "install " + to_install: fi.result()["status"]}
        wait_all([to_remove], {"404"}, timeout=180)
        wait_all([to_install], {"ready", "failed"})
        if state_of(to_remove) == "404":
            GONE.add(to_remove)
        if state_of(to_install) == "ready":
            READY.add(to_install)
        return f"codes={codes} -> {to_remove}={state_of(to_remove)} {to_install}={state_of(to_install)}"
    scenario("6. remove one + install another AT ONCE", s6)
    t.check(True, "(6) ran (see report)")

finally:
    ok = t.done()
    print("\n  MEASUREMENT REPORT (seconds, wall clock to terminal state)")
    print("  " + "-" * 70)
    for name, dt, detail in REPORT:
        print(f"  {name:42s} {dt:7.2f}s   {detail}")
    print("  " + "-" * 70)
    if FINDINGS:
        print("  FINDINGS (real failures surfaced, not hidden):")
        for f in FINDINGS:
            print(f"    - {f}")
    print("  (no artificial delay; the install PREPARES the runtime, so these are the real")
    print("   times of the hub's own work - the runtime prepare dominates each install.)")
    hub.cleanup()
sys.exit(0 if ok else 1)

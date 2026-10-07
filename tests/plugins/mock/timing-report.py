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
# (ready / absent), with NO artificial delay: the registry ships the bytes as fast as loopback
# allows, so the numbers are the REAL cost of the hub's own work (download, verify, unpack, land,
# SQLite commit, announce). This prints a REPORT; it is a measurement, not a pass/fail of numbers.
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

    def wait_all(pids, goals, timeout=300):
        end = now() + timeout
        remaining = set(pids)
        while remaining and now() < end:
            for pid in list(remaining):
                if state_of(pid) in goals:
                    remaining.discard(pid)
            if remaining:
                time.sleep(0.05)
        return remaining

    def art(pid):
        return hub.registry_artifact(pid)

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

    # --- 1. 3 concurrent installs -------------------------------------------------------------
    def s1():
        with cf.ThreadPoolExecutor(3) as ex:
            futs = [ex.submit(do_install, p) for p in IDS]
            codes = [f.result()["status"] for f in futs]
        left = wait_all(IDS, {"ready", "failed"})
        return f"codes={codes} not-ready={sorted(left)}"
    scenario("1. 3 CONCURRENT installs", s1)
    t.check(all(state_of(p) == "ready" for p in IDS), "after (1) all 3 are ready",
            f"{ {p: state_of(p) for p in IDS} }")

    # --- 2. 3 concurrent removes --------------------------------------------------------------
    def s2():
        with cf.ThreadPoolExecutor(3) as ex:
            futs = [ex.submit(do_remove, p) for p in IDS]
            codes = [f.result()["status"] for f in futs]
        left = wait_all(IDS, {"404"}, timeout=120)
        return f"codes={codes} not-gone={sorted(left)}"
    scenario("2. 3 CONCURRENT removes", s2)
    t.check(all(state_of(p) == "404" for p in IDS), "after (2) all 3 are gone",
            f"{ {p: state_of(p) for p in IDS} }")

    # --- 3. 3 SERIAL installs -----------------------------------------------------------------
    def s3():
        codes = [do_install(p)["status"] for p in IDS]
        left = wait_all(IDS, {"ready", "failed"})
        return f"codes={codes} not-ready={sorted(left)}"
    scenario("3. 3 SERIAL installs", s3)
    t.check(all(state_of(p) == "ready" for p in IDS), "after (3) all 3 are ready",
            f"{ {p: state_of(p) for p in IDS} }")

    # --- 4. remove ONE ------------------------------------------------------------------------
    scenario("4. remove ONE (deepseek)", lambda: f"code={do_remove('deepseek')['status']} after={wait_all(['deepseek'], {'404'}, 120) or 'gone'}")

    # --- 5. install one AND remove one of the other two, at once ------------------------------
    #     currently: pi, jouzu ready; deepseek gone. Install deepseek; remove pi. at once.
    def s5():
        with cf.ThreadPoolExecutor(2) as ex:
            fi = ex.submit(do_install, "deepseek")
            fr = ex.submit(do_remove, "pi")
            codes = {"install deepseek": fi.result()["status"], "remove pi": fr.result()["status"]}
        wait_all(["deepseek"], {"ready", "failed"})
        wait_all(["pi"], {"404"}, 120)
        return f"codes={codes}"
    scenario("5. install one + remove another AT ONCE", s5)
    t.check(state_of("deepseek") == "ready" and state_of("pi") == "404",
            "after (5) deepseek is ready and pi is gone",
            f"deepseek={state_of('deepseek')} pi={state_of('pi')}")

    # --- 6. remove the not-yet-removed one AND install another, at once -----------------------
    #     currently: jouzu, deepseek ready; pi gone. Remove jouzu; install pi. at once.
    def s6():
        with cf.ThreadPoolExecutor(2) as ex:
            fr = ex.submit(do_remove, "jouzu")
            fi = ex.submit(do_install, "pi")
            codes = {"remove jouzu": fr.result()["status"], "install pi": fi.result()["status"]}
        wait_all(["jouzu"], {"404"}, 120)
        wait_all(["pi"], {"ready", "failed"})
        return f"codes={codes}"
    scenario("6. remove one + install another AT ONCE", s6)
    t.check(state_of("jouzu") == "404" and state_of("pi") == "ready",
            "after (6) jouzu is gone and pi is ready",
            f"jouzu={state_of('jouzu')} pi={state_of('pi')}")

finally:
    ok = t.done()
    print("\n  MEASUREMENT REPORT (seconds, wall clock to terminal state)")
    print("  " + "-" * 66)
    for name, dt, detail in REPORT:
        print(f"  {name:44s} {dt:6.2f}s   {detail}")
    print("  " + "-" * 66)
    print(f"  (no artificial delay: loopback serves the bytes; these are the REAL times of")
    print(f"   the hub's own per-install work, so the concurrency of the hub is what shows.)")
    hub.cleanup()
sys.exit(0 if ok else 1)

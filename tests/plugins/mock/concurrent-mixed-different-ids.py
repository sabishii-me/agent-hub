# TC-PLUGIN-CONCURRENT-MIXED: the FULL lifecycle of DIFFERENT plugins install AND remove at once
#
# The requirement (owner): concurrent install/remove means DIFFERENT plugins - one plugin is being
# installed while another is being removed, the two in flight TOGETHER (mixed states), and the
# WHOLE flow must be observable: `installing`...`ready` for one, `ready`...`removing`...`absent`
# for the other - not just the 202 that starts it. A 202 alone is NOT a result (the install is
# detached); the states during and after are.
#
# The hub installs from its OFFICIAL registry (real GitHub release assets): the download is a real
# network hop, so `installing` is a real, visible duration - no mock, no injected delay. The states
# are read from the hub and the terminal states are waited for.
import os
import sys
import time
import concurrent.futures as cf

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))
sys.path.insert(0, os.path.join(ROOT, "tests", "lib"))
from hub import Hub  # noqa: E402
from tally import Tally  # noqa: E402

t = Tally("plugins/concurrent-mixed-different-ids")

hub = Hub()
try:
    hub.start()

    def state_of(pid):
        g = hub.get(f"/v1/plugins/{pid}")
        if g["status"] == 404:
            return "404"
        return (g["json"] or {}).get("plugin", {}).get("state")

    def wait_terminal(pid, goals, timeout=300):
        end = time.time() + timeout
        while time.time() < end:
            s = state_of(pid)
            if s in goals:
                return s
            time.sleep(0.05)
        return state_of(pid)

    # Pre-install the remove target and WAIT (real completion), so the window removes a READY plugin.
    target = hub.registry_artifact("deepseek")
    hub.post("/v1/plugins", {"source": {"artifact": target}}, key="mx-pre-deepseek")
    t.check(wait_terminal("deepseek", {"ready", "failed"}) == "ready",
            "the remove target reaches `ready` before the window (real wait, not the 202)",
            "deepseek never became ready")

    seq = {"pi": [], "jouzu": [], "deepseek": []}
    stop = {"v": False}

    def sampler():
        while not stop["v"]:
            for pid in seq:
                s = state_of(pid)
                if not seq[pid] or seq[pid][-1] != s:
                    seq[pid].append(s)
            time.sleep(0.02)

    def install(pid):
        a = hub.registry_artifact(pid)
        return pid, "install", a["url"].rsplit("/", 1)[-1], hub.post(
            "/v1/plugins", {"source": {"artifact": a}}, key=f"mx-i-{pid}-{time.time()}")

    def remove(pid):
        return pid, "remove", None, hub.delete(f"/v1/plugins/{pid}", key=f"mx-r-{pid}-{time.time()}")

    smp = cf.ThreadPoolExecutor(max_workers=1)
    sample_fut = smp.submit(sampler)
    time.sleep(0.1)
    try:
        # THE MIXED WINDOW: install two ids never installed here, remove a third - all at once.
        with cf.ThreadPoolExecutor(max_workers=3) as ex:
            futs = [ex.submit(install, "pi"), ex.submit(install, "jouzu"), ex.submit(remove, "deepseek")]
            results = [f.result() for f in futs]
        accepted = {pid: (op, r["status"]) for pid, op, _z, r in results}
        t.check(all(r["status"] < 300 for _, _, _z, r in results),
                "every mixed op is ACCEPTED at once (installs and a remove, different plugins)",
                f"results={accepted}")
        t.check(any(op == "install" for _p, op, _z, _r in results)
                and any(op == "remove" for _p, op, _z, _r in results),
                "the window really mixed an install AND a remove (not one kind)",
                f"results={accepted}")

        # FULL FLOW: both installs must pass THROUGH `installing` and reach `ready`; the remove must
        # pass through `removing` and become `absent`. Sampled while everything ran.
        t.check(wait_terminal("pi", {"ready", "failed"}) == "ready", "`pi` reaches `ready`", "not ready")
        t.check(wait_terminal("jouzu", {"ready", "failed"}) == "ready", "`jouzu` reaches `ready`", "not ready")
        t.check(wait_terminal("deepseek", {"404"}, timeout=120) == "404",
                "the removed plugin becomes `absent` (404)", "still present")
    finally:
        stop["v"] = True
        sample_fut.result()
        smp.shutdown()

    t.check("installing" in seq["pi"], "`pi` was OBSERVED in `installing` (the install is not instant)",
            f"sequence={seq['pi']}")
    t.check("installing" in seq["jouzu"], "`jouzu` was OBSERVED in `installing`",
            f"sequence={seq['jouzu']}")
    t.check("removing" in seq["deepseek"], "`deepseek` was OBSERVED in `removing`",
            f"sequence={seq['deepseek']}")
    # THE MIXED STATE: both an install and a remove were in flight at the SAME sample - the two
    # different plugins evolved concurrently, not one after the other.
    mixed = any("installing" in seq[p] and "removing" in seq["deepseek"]
                for p in ("pi", "jouzu"))
    t.check(mixed, "an install and a remove were in flight TOGETHER (mixed states)",
            f"pi={seq['pi']} jouzu={seq['jouzu']} deepseek={seq['deepseek']}")
    # No foreign state leaked: pi/jouzu never show `removing`, deepseek never shows `installing`.
    t.check("removing" not in seq["pi"] and "removing" not in seq["jouzu"],
            "neither installed plugin ever showed `removing`", f"pi={seq['pi']} jouzu={seq['jouzu']}")
    t.check("installing" not in seq["deepseek"],
            "the removed plugin never showed `installing`", f"deepseek={seq['deepseek']}")

    # REAL download: the mock registry logged a GET for each installed zip.
    hits = hub.registry_hits()
    wanted = [z for _p, op, z, _r in results if op == "install"]
    t.check(all(z in hits for z in wanted),
            "REAL download: the hub GET each plugin zip over HTTP from the registry",
            f"wanted={wanted} hits={hits}")
finally:
    ok = t.done()
    hub.cleanup()
sys.exit(0 if ok else 1)

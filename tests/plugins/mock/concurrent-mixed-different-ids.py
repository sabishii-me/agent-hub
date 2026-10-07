# TC-PLUGIN-CONCURRENT-MIXED: DIFFERENT plugins install AND remove at the same time
#
# The requirement (owner): concurrent install/remove means DIFFERENT plugins - plugin A installed
# while plugin B is removed, the two in flight TOGETHER (mixed states). It is NOT "remove a plugin
# while IT installs" (that is the same-id conflict rule, a different matter).
#
# This test pre-installs one plugin so a remove has a real target, then, in ONE window, issues an
# install of ONE plugin and a remove of ANOTHER at once. All operations must be accepted and reach a
# terminal state; the hub must NOT serialize them (a single global op lock would refuse one).
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

    def wait_ready(pid, timeout=300):
        end = time.time() + timeout
        while time.time() < end:
            g = hub.get(f"/v1/plugins/{pid}")
            st = (g["json"] or {}).get("plugin", {}).get("state")
            if st == "ready":
                return True
            if st == "failed":
                return False
            time.sleep(0.5)
        return False

    # Pre-install ONE plugin so the later remove has a REAL, ready target. Do NOT pre-install a
    # plugin that the window will install again - that would be a same-id conflict, a different rule.
    target = hub.registry_artifact("deepseek")
    installed = hub.post("/v1/plugins", {"source": {"artifact": target}}, key="mx-pre-deepseek")
    t.check(installed["status"] == 202, "the remove target installs first (202)",
            f"status={installed['status']} {installed['text'][:120]}")
    t.check(wait_ready("deepseek"), "the remove target reaches `ready` before the mixed window",
            "deepseek never became ready")

    # THE MIXED WINDOW: install `pi` and `jouzu` (two ids never installed here) while removing
    # `deepseek` (a third id) - all issued at once with nothing sequential.
    def install(pid):
        a = hub.registry_artifact(pid)
        return pid, "install", a["url"].rsplit("/", 1)[-1], hub.post("/v1/plugins", {"source": {"artifact": a}}, key=f"mx-i-{pid}-{time.time()}")

    def remove(pid):
        return pid, "remove", None, hub.delete(f"/v1/plugins/{pid}", key=f"mx-r-{pid}-{time.time()}")

    with cf.ThreadPoolExecutor(max_workers=3) as ex:
        futs = [ex.submit(install, "pi"), ex.submit(install, "jouzu"), ex.submit(remove, "deepseek")]
        results = [f.result() for f in futs]

    accepted = {pid: (op, r["status"]) for pid, op, _z, r in results}
    t.check(all(r["status"] < 300 for _, _, _z, r in results),
            "every mixed op is ACCEPTED at once (an install and a remove, different plugins)",
            f"results={accepted}")
    t.check(any(op == "install" and r["status"] < 300 for _, op, _z, r in results)
            and any(op == "remove" and r["status"] < 300 for _, op, _z, r in results),
            "the window really mixed an install AND a remove (not one kind)",
            f"results={accepted}")

    # REAL download: the hub FETCHED the two zips over HTTP from the registry - the mock registry
    # logged a GET for each. A local copy would leave no hit.
    hits = hub.registry_hits()
    wanted = [z for _, op, z, _r in results if op == "install"]
    t.check(all(z in hits for z in wanted),
            "REAL download: the hub GET each plugin zip over HTTP from the registry",
            f"wanted={wanted} hits={hits}")

    # Both directions must ACTUALLY happen: BOTH installed plugins reach ready, the removed one
    # becomes absent. If the hub serialized them, one would be refused and this fails. We wait for
    # the REAL terminal state (install is detached: a 202 means `installing`, not done).
    t.check(wait_ready("pi"), "the concurrently-installed plugin `pi` reaches `ready`", "pi not ready")
    t.check(wait_ready("jouzu"), "the concurrently-installed plugin `jouzu` reaches `ready`", "jouzu not ready")
    end = time.time() + 120
    gone = False
    while time.time() < end:
        g = hub.get("/v1/plugins/deepseek")
        if g["status"] == 404:
            gone = True
            break
        time.sleep(0.5)
    t.check(gone, "the concurrently-removed plugin is GONE (404)",
            f"GET deepseek still {g['status']}")
finally:
    ok = t.done()
    hub.cleanup()
sys.exit(0 if ok else 1)

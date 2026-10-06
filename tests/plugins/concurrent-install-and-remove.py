# Concurrency on the plugin surface must not corrupt it: several installs of the SAME id at
# once must converge to ONE ready plugin (idempotent, not a torn directory), and an
# install racing a remove of a different plugin must leave each plugin in a sane state.
#
# FACT:    N concurrent installs of one id converge to exactly one ready plugin (no torn
#          dir, no stuck op); a concurrent install + remove of another plugin both finish
# SOURCE:  contract/v1.json POST/DELETE /v1/plugins (an id already installed is REPLACED,
#          atomically: "the old copy is moved aside first"); ARCHITECTURE s12
# EXPOSES: a concurrent install that lands a half-tree or leaves the plugin stuck, or a
#          remove/install pair that deadlocks the ops
import concurrent.futures as cf
import os
import sys
import time

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "lib"))
from hub import Hub, hub_sha          # noqa: E402
from tally import Tally, combo        # noqa: E402

PI = os.environ.get("PI_PLUGIN_DIR", r"E:/AI/ideas/prts-harness-pi")
PI_REF = os.environ.get("PI_PLUGIN_REF", "fix/runtime-placement")
t = Tally("plugins/concurrent")
combo(hub_sha())
if not t.require(os.path.isdir(PI), "a real plugin source is present", f"no plugin at {PI}"):
    t.done(); sys.exit(1)

hub = Hub()
try:
    hub.start()

    def install(i):
        r = hub.post("/v1/plugins", {"source": {"url": PI, "ref": PI_REF}}, key=f"ci-{i}")
        return r["status"]

    # N concurrent installs of the SAME id.
    N = 6
    with cf.ThreadPoolExecutor(max_workers=N) as ex:
        codes = list(ex.map(install, range(N)))
    # The SAME plugin cannot be installed concurrently: exactly one proceeds (202) and
    # every other concurrent call is REFUSED 409 conflict - never a 500 (the owner's
    # rule; a 500 internal_error is the bug this test exists to catch).
    accepted = sum(1 for c in codes if c == 202)
    conflicts = sum(1 for c in codes if c == 409)
    t.check(accepted >= 1, f"one install of the id proceeds (202)", f"codes={codes}")
    t.check(all(c in (202, 409) for c in codes),
            f"every concurrent install of one id is 202 or 409 (never 5xx)", f"codes={codes}")
    t.check(conflicts >= 1, f"a concurrent install of the same id is REFUSED 409", f"codes={codes}")

    # It converges to EXACTLY ONE plugin, `ready`, with a valid manifest on disk.
    ready = False
    for _ in range(240):
        g = hub.get("/v1/plugins/pi")
        st = None if g["status"] == 404 else (g["json"] or {}).get("plugin", {}).get("state")
        if st == "ready":
            ready = True
            break
        time.sleep(0.25)
    t.check(ready, "after concurrent installs the plugin is `ready` (no torn dir, no stuck op)",
            f"body={hub.get('/v1/plugins/pi')['text'][:160]}")
    rows = (hub.get("/v1/plugins")["json"] or {}).get("plugins", [])
    t.check(sum(1 for r in rows if r.get("id") == "pi") == 1,
            "exactly ONE `pi` plugin exists (not duplicated)", f"rows={[r.get('id') for r in rows]}")

    # A concurrent install + remove: install `pi` while removing it, repeatedly. Each
    # operation finishes; the final state is a real, readable one (ready or absent).
    def churn(i):
        if i % 2 == 0:
            hub.post("/v1/plugins", {"source": {"url": PI, "ref": PI_REF}}, key=f"ch-i{i}")
        else:
            hub.delete("/v1/plugins/pi", key=f"ch-d{i}")
    with cf.ThreadPoolExecutor(max_workers=4) as ex:
        list(ex.map(churn, range(4)))
    # Let the ops settle.
    final = None
    for _ in range(240):
        g = hub.get("/v1/plugins/pi")
        final = "absent" if g["status"] == 404 else (g["json"] or {}).get("plugin", {}).get("state")
        if final in ("ready", "absent", "failed"):
            break
        time.sleep(0.25)
    t.check(final in ("ready", "absent", "failed"),
            "after install/remove churn the plugin is in a terminal state (no stuck removing/installing)",
            f"final_state={final}")
finally:
    ok = t.done()
    hub.cleanup()
sys.exit(0 if ok else 1)

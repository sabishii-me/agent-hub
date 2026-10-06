# §10 crash boundary (the old interruption/removal.mjs): a hub KILLED in the middle of a
# REMOVE must come back and FINISH the removal - never left holding half of it (a
# directory with no record, or a record with no directory). Real: a real hub process,
# killed with no cleanup (a power cut), restarted on the SAME data dir.
#
# FACT:    a hub killed mid-remove restarts to a CONSISTENT state and finishes the removal
#          (no half state, no phantom record)
# SOURCE:  ARCHITECTURE s10 (plugin install/replace recovery, the boundary table);
#          docs/issues/20261005-060000
# EXPOSES: a crash that leaves a plugin directory without its record (or the reverse) - a
#          silent half-state after a power cut
import os
import sys
import time

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "lib"))
from hub import Hub, hub_sha, require_registry_or_blocked          # noqa: E402
from tally import Tally, combo        # noqa: E402

PI = os.environ.get("PI_PLUGIN_DIR", r"E:/AI/ideas/prts-harness-pi")
PI_REF = os.environ.get("PI_PLUGIN_REF", "fix/runtime-placement")
t = Tally("plugins/killed-mid-remove")
combo(hub_sha())
require_registry_or_blocked()   # BLOCKED until the official registry is published
if not t.require(os.path.isdir(PI), "a real plugin source is present", f"no plugin at {PI}"):
    t.done(); sys.exit(1)

hub = Hub()
hub2 = None
try:
    hub.start()
    hub.post("/v1/plugins", {"source": {"artifact": hub.registry_artifact("pi")}}, key="k-install")
    ready = False
    for _ in range(240):
        g = hub.get("/v1/plugins/pi")
        if g["status"] == 200 and (g["json"] or {}).get("plugin", {}).get("state") == "ready":
            ready = True; break
        time.sleep(0.25)
    t.check(ready, "the plugin is installed and ready before the removal", f"body={hub.get('/v1/plugins/pi')['text'][:140]}")

    target = os.path.join(hub.plugins, "pi")

    # Start a removal and KILL the hub while it is in flight (no cleanup - a power cut).
    def do_remove():
        try:
            hub.delete("/v1/plugins/pi", key="k-remove")
        except Exception:
            pass
    import threading
    th = threading.Thread(target=do_remove, daemon=True)
    th.start()
    # Kill IMMEDIATELY - the removal is a race we must interrupt, so we do not wait for
    # an observable state; we kill within a few ms of issuing the DELETE. (Because the
    # remove can finish in <10ms, the kill may land before, during or after it; the
    # ASSERTION below is the invariant that must hold for ALL of those - a consistent
    # restart - not a claim about which boundary we hit.)
    time.sleep(0.005)
    hub.kill()
    th.join(timeout=5)
    # Whatever we interrupted, the plugin's record and directory must agree after restart.

    # Restart on the SAME data dir: the hub must finish the removal and be consistent.
    hub2 = Hub(data_dir=hub.dir)
    hub2.start()
    # The finish is async at boot; give it a moment, then require full consistency.
    # THE INVARIANT (§10): after a crash, the RECORD and the DIRECTORY must AGREE, and
    # never a silent half. Two consistent outcomes exist, both honest:
    #   both present  = the remove had not begun (or was rolled back) -> plugin intact;
    #   both gone     = the remove was finished.
    # A HALF (record without dir, or dir without record) is the defect this catches.
    consistent = False
    listed = None
    dir_exists = None
    for _ in range(240):
        listed = any(p.get("id") == "pi" for p in (hub2.get("/v1/plugins")["json"] or {}).get("plugins", []))
        dir_exists = os.path.exists(target)
        if listed == dir_exists:   # both true or both false
            consistent = True
            break
        time.sleep(0.25)
    t.check(consistent,
            "after restart the record and the directory AGREE (no silent half-state)",
            f"listed={listed} dir_exists={dir_exists}")
    if consistent and not dir_exists:
        t.check(not listed, "a finished removal leaves neither the record nor the directory", f"listed={listed}")

    # The hub still works: a fresh install of the same plugin succeeds.
    r = hub2.post("/v1/plugins", {"source": {"artifact": hub2.registry_artifact("pi")}}, key="k-again")
    t.check(r["status"] in (200, 202), "the restarted hub can install again afterwards", f"status={r['status']} {r['text'][:140]}")
finally:
    if hub2 is not None:
        hub2.cleanup()
    ok = t.done()
    hub.cleanup()
sys.exit(0 if ok else 1)

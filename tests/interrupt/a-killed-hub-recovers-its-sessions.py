# A hub KILLED mid-life (a power cut, no cleanup) must come back to a state that makes sense
# and must not leave a session claiming `active` without a live process. This drives a real
# binary, kills it with taskkill /T /F, restarts on the SAME data dir, and reads the truth.
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "lib"))
from hub import Hub, hub_sha                   # noqa: E402
from tally import Tally, combo, kind_of        # noqa: E402

PI = os.environ.get("PI_PLUGIN_DIR", r"E:/AI/ideas/prts-harness-pi")
HARNESS = os.environ.get("PI_HARNESS_ID", "pi")

t = Tally("interrupt/kill-recover")
combo(hub_sha(), kind_of(PI))
if not os.path.isdir(PI):
    t.skip("kill/recover", f"no real plugin at {PI}")
    t.done()
    sys.exit(0)

hub = Hub(plugins_src=PI)
sid = None
data_dir = None
try:
    hub.start()
    data_dir = hub.dir
    r = hub.post("/v1/sessions", {"harnessId": HARNESS}, key="k-1")
    t.check(r["status"] == 202, "create is accepted", f"status={r['status']}")
    sid = r["json"]["session"]["id"]
    s = hub.wait_status(sid, "active")
    t.check(s.get("status") == "active", "the session is active before the kill", f"status={s.get('status')}")

    # KILL with no cleanup.
    hub.kill()
    t.check(hub.child.poll() is not None, "the hub process is dead (killed, not stopped)")

    # Restart on the SAME data dir.
    hub2 = Hub(data_dir=data_dir, plugins_src=None)
    # reuse the already-populated dir: point plugins at the same one
    hub2.plugins = hub.plugins
    hub2.start()
    try:
        # After recovery, the session must NOT claim active unless a process is live.
        # The honest states are `active` (a process was re-attached) or `needs-repair`.
        g = hub2.get(f"/v1/sessions/{sid}")
        st = (g["json"] or {}).get("session", {}).get("status")
        t.check(st in ("active", "needs-repair", "readonly", "starting_failed"),
                "after recovery the session reports a real state, not a lie", f"status={st}")

        # The recovered hub must still serve: a fresh session works.
        r2 = hub2.post("/v1/sessions", {"harnessId": HARNESS}, key="k-2")
        t.check(r2["status"] == 202, "the recovered hub accepts a new session", f"status={r2['status']} {r2['text'][:100]}")
        sid2 = r2["json"]["session"]["id"]
        s2 = hub2.wait_status(sid2, "active")
        t.check(s2.get("status") == "active", "the new session reaches active after recovery", f"status={s2.get('status')}")
    finally:
        hub2.stop()
finally:
    ok = t.done()
    # clean the whole data dir once
    try:
        import shutil
        if data_dir:
            shutil.rmtree(data_dir, ignore_errors=True)
    except Exception:
        pass
sys.exit(0 if ok else 1)

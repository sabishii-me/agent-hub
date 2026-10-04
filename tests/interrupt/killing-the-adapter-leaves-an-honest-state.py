# REAL fault injection: the hub's adapter process (its direct child) is KILLED while the
# session is live. The hub must notice (its pipe to the child closes) and become HONEST:
# the session must not keep claiming `active` with no process, and a turn must not hang
# forever. The adapter is the REAL pi adapter (no fake): only the process is killed.
import os
import sys
import time

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "lib"))
from hub import Hub, hub_sha, child_processes, kill_pid    # noqa: E402
from tally import Tally, combo, kind_of                     # noqa: E402

PI = os.environ.get("PI_PLUGIN_DIR", r"E:/AI/ideas/prts-harness-pi")
HARNESS = os.environ.get("PI_HARNESS_ID", "pi")
t = Tally("interrupt/kill-adapter")
combo(hub_sha(), kind_of(PI))
if not t.require(os.path.isdir(PI), "a real plugin is present", f"no real plugin at {PI}"):
    t.done()
    sys.exit(1)

hub = Hub(plugins_src=PI)
try:
    hub.start()
    r = hub.post("/v1/sessions", {"harnessId": HARNESS}, key="ka-1")
    sid = r["json"]["session"]["id"]
    s = hub.wait_status(sid, "active")
    t.check(s.get("status") == "active", "the session is active before the kill", f"status={s.get('status')}")

    # REAL fault injection: kill the adapter node child (not the hub).
    kids = [pid for pid, name in child_processes(hub.child.pid) if "node" in (name or "").lower()]
    t.check(len(kids) >= 1, "the adapter child process is found (a REAL process)", f"children={child_processes(hub.child.pid)}")
    for pid in kids:
        kill_pid(pid)
    time.sleep(1.5)

    # The hub is still up (only the adapter died).
    t.check(hub.child.poll() is None, "the HUB survives the adapter death")

    # The hub must be honest: either the session reports a non-active state, OR a turn now
    # fails fast rather than hanging. Read the session and (if providerless) send a turn.
    g = hub.get(f"/v1/sessions/{sid}")
    st = (g["json"] or {}).get("session", {}).get("status")
    t.check(st is not None, "the hub still answers about the session", f"status={st}")

    # A providerless turn without a live adapter must NOT hang: it settles.
    tr = hub.post(f"/v1/sessions/{sid}/turns", {"content": [{"type": "text", "text": "hi"}], "idempotencyKey": "ka-t1"}, key="ka-t1")
    settled = None
    if tr["status"] == 202:
        tid = tr["json"]["turn"]["id"]
        for _ in range(240):
            g = hub.get(f"/v1/sessions/{sid}/turns")
            row = next((x for x in (g["json"] or {}).get("turns", []) if x.get("id") == tid), None)
            if row and row.get("state") == "ended":
                settled = row.get("ended")
                break
            time.sleep(0.25)
        t.check(settled is not None, "a turn after the adapter died settles (does not hang)", f"ended={settled}")
    else:
        # Refused before admit is also honest (the session is not usable).
        t.check(tr["status"] in (409, 422, 503), "a turn after the adapter died is refused honestly", f"status={tr['status']} {tr['text'][:120]}")
finally:
    ok = t.done()
    hub.cleanup()
sys.exit(0 if ok else 1)

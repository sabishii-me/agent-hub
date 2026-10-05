# REAL fault injection: the hub's adapter process (its direct child) is KILLED while the
# session is live. The hub must notice (its pipe to the child closes) and become HONEST:
# the session must not keep claiming `active` with no process, and a turn must not hang
# forever. The adapter is the REAL pi adapter (no fake): only the process is killed.
#
# FACT:    killing the adapter CHILD does not leave the session claiming active; a later turn is refused, not hung
# SOURCE:  ARCHITECTURE s21 N2 (no process behind the row); docs/issues/20261005-020000
# EXPOSES: a session that keeps active with no process (20261005-020000)
# (A test that would pass whatever happens is not a test: this block names the fact it
#  proves and where that fact comes from; the assertions below are that exact fact.)
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

    # The hub must be honest: it must NOT keep reporting `active` for a session whose adapter
    # process is gone (the docstring's claim). This is the SPECIFIC property; `is not None`
    # accepted `active` and hid a real defect (docs/issues/20261005-020000).
    g = hub.get(f"/v1/sessions/{sid}")
    st = (g["json"] or {}).get("session", {}).get("status")
    t.check(st != "active", "the session no longer claims `active` after its adapter died",
            f"status={st} (a processless session reporting `active` is the defect)")

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
        # A turn must not HANG: it must reach a terminal. Any terminal value is honest here
        # (the contract does not fix which); the property is "it settled, not `running` forever".
        t.check(settled is not None, "a turn after the adapter died settles (does not hang)", f"ended={settled}")
        t.check(settled != "completed",
                "a turn whose adapter is dead is NOT reported as a clean `completed`",
                f"ended={settled} (a dead adapter cannot have completed a turn cleanly)")
    else:
        # The session is now needs-repair (not active); a turn must be REFUSED, never
        # 2xx. The contract does not pin WHICH 4xx for a non-active session, so assert
        # the real property (a 4xx refusal that names the state), not an invented set.
        body = (tr["json"] or {})
        refused = 400 <= tr["status"] < 500
        names_state = "needs-repair" in tr["text"] or "active" in tr["text"] or body.get("error") in ("validation_failed", "session_busy", "conflict")
        t.check(refused and names_state,
                "a turn after the adapter died is refused (4xx) and says why",
                f"status={tr['status']} body={tr['text'][:140]}")
finally:
    ok = t.done()
    hub.cleanup()
sys.exit(0 if ok else 1)

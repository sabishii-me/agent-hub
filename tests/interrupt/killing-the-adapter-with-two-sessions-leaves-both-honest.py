# TC-INTERRUPT-016
# CONCURRENCY C6: TWO sessions run on the same harness, and the adapter processes are
# killed. BOTH sessions must become honest (off `active`), none may wedge, and neither may
# stay `active` with no process behind it. Real: real adapters, real process kills.
#
# FACT:    killing the adapters under two concurrent sessions leaves BOTH honest (none stays active, none wedges)
# SOURCE:  ARCHITECTURE s21 N2 (no live process behind a broken row); docs/issues/20261005-020000
# EXPOSES: one session's death hiding another's, or a multi-session harness that leaves a session wedged
# (A test that would pass whatever happens is not a test: this block names the fact it
#  proves and where that fact comes from; the assertions below are that exact fact.)
import os
import sys
import time

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "lib"))
from hub import Hub, hub_sha, child_processes, kill_pid              # noqa: E402
from tally import Tally, combo, kind_of                              # noqa: E402

PI = os.environ.get("PI_PLUGIN_DIR", r"E:/AI/ideas/prts-harness-pi")
HARNESS = os.environ.get("PI_HARNESS_ID", "pi")

t = Tally("interrupt/kill-adapter-two-sessions")
combo(hub_sha(), kind_of(PI))
if not t.require(os.path.isdir(PI), "a real plugin is present", f"no real plugin at {PI}"):
    t.done(); sys.exit(1)

hub = Hub(plugins_src=PI)
try:
    hub.start()
    # Two sessions on the SAME harness.
    r1 = hub.post("/v1/sessions", {"harnessId": HARNESS}, key="c6-s1")
    r2 = hub.post("/v1/sessions", {"harnessId": HARNESS}, key="c6-s2")
    s1 = r1["json"]["session"]["id"]; s2 = r2["json"]["session"]["id"]
    t.check(hub.wait_status(s1, "active").get("status") == "active", "session 1 is active")
    t.check(hub.wait_status(s2, "active").get("status") == "active", "session 2 is active")
    t.check(s1 != s2, "the two sessions are distinct")

    # Kill every adapter child of the hub (each session runs its own adapter process).
    kids = [pid for pid, name in child_processes(hub.child.pid) if "node" in (name or "").lower()]
    t.check(len(kids) >= 1, "a real adapter child is found", f"children={child_processes(hub.child.pid)}")
    for pid in kids:
        kill_pid(pid)

    # BOTH must leave `active` (poll, since the death is noticed asynchronously).
    def leaves_active(sid, tries=160):
        st = "active"
        for _ in range(tries):
            st = (hub.get(f"/v1/sessions/{sid}")["json"] or {}).get("session", {}).get("status")
            if st != "active":
                break
            time.sleep(0.25)
        return st

    st1 = leaves_active(s1); st2 = leaves_active(s2)
    t.check(st1 != "active", "session 1 no longer claims active after its adapter died", f"status={st1}")
    t.check(st2 != "active", "session 2 no longer claims active after its adapter died", f"status={st2}")

    # Neither is wedged by the other's death: each accepts a defined (non-5xx) turn outcome
    # or is cleanly refused, and both still read.
    for sid in (s1, s2):
        t.check(hub.get(f"/v1/sessions/{sid}")["status"] == 200, f"session {sid} still reads")
        tr = hub.post(f"/v1/sessions/{sid}/turns",
                      {"content": [{"type": "text", "text": "hi"}], "idempotencyKey": f"c6-t-{sid}"}, key=f"c6-t-{sid}")
        t.check(tr["status"] < 500, f"session {sid} is not wedged (a turn is not a 5xx)",
                f"status={tr['status']} {tr['text'][:120]}")

    t.check(hub.child.poll() is None, "the HUB survives both adapter deaths")
finally:
    ok = t.done()
    hub.cleanup()
sys.exit(0 if ok else 1)

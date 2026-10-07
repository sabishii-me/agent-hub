# PASSIVE P2/P4: a REAL turn is running (real provider); the adapter child is KILLED
# mid-turn. The turn must settle honestly (not hang forever, not stay `running` with no
# process), and the session must not keep claiming `active`. The adapter is the real pi
# adapter - only its process is killed (real fault injection, the hub is never bypassed).
#
# FACT:    an adapter killed MID-TURN settles the turn (failed), and the session does not keep claiming active
# SOURCE:  ARCHITECTURE s21 N2; docs/issues/20261005-020000
# EXPOSES: a turn left running, or a processless session stuck active
# (A test that would pass whatever happens is not a test: this block names the fact it
#  proves and where that fact comes from; the assertions below are that exact fact.)
import os
import sys
import time

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "lib"))
from hub import Hub, hub_sha, register_provider, child_processes, kill_pid, wait_turn  # noqa: E402
from tally import Tally, combo, kind_of                          # noqa: E402

PI = os.environ.get("PI_PLUGIN_DIR", r"E:/AI/ideas/prts-harness-pi")
HARNESS = os.environ.get("PI_HARNESS_ID", "pi")
MODEL = os.environ.get("PI_MODEL", "deepseek-flash")

t = Tally("interrupt/kill-adapter-mid-turn")
combo(hub_sha(), kind_of(PI))
if not t.require(os.path.isdir(PI), "a real plugin is present", f"no real plugin at {PI}"):
    t.done(); sys.exit(1)

hub = Hub(plugins_src=PI)
try:
    hub.start()
    t.require(register_provider(hub) is not None, "a real provider is registered", "no real provider")
    r = hub.post("/v1/sessions", {"harnessId": HARNESS, "modelProviderId": "p", "modelId": MODEL}, key="km-s")
    sid = r["json"]["session"]["id"]
    t.check(hub.wait_status(sid, "active").get("status") == "active", "the session is active")

    tr = hub.post(f"/v1/sessions/{sid}/turns", {"content": [{"type": "text", "text": "Count slowly from 1 to 400, one number per line."}], "idempotencyKey": "km-t"}, key="km-t")
    t.check(tr["status"] == 202, "the turn is accepted", f"status={tr['status']}")
    tid = tr["json"]["turn"]["id"]
    row = wait_turn(hub, sid, tid, {"running", "awaiting_approval", "awaiting_question"}, tries=160)
    t.check(row.get("state") in ("running", "awaiting_approval", "awaiting_question"),
            "the turn is running (a real model call is in flight)", f"state={row.get('state')}")

    # REAL fault injection: kill the adapter's node child.
    kids = [pid for pid, name in child_processes(hub.child.pid) if "node" in (name or "").lower()]
    t.check(len(kids) >= 1, "the real adapter process is found to kill", f"children={child_processes(hub.child.pid)}")
    for pid in kids:
        kill_pid(pid)

    # The turn MUST settle (bounded). A killed adapter must not leave a turn running.
    end = wait_turn(hub, sid, tid, {"ended"}, tries=240)
    t.check(end.get("state") == "ended", "the turn settles after the adapter died (no hang)",
            f"state={end.get('state')} ended={end.get('ended')}")
    # Killed mid-turn, no cancel: the turn died on its own -> `failed` (ARCHITECTURE
    # §21 N2). The old `in (failed,interrupted,cancelled)` accepted terminals the
    # situation cannot produce.
    t.check(end.get("ended") == "failed",
            "a turn whose adapter died mid-turn ends `failed`",
            f"ended={end.get('ended')} (required: failed)")

    # The hub is still alive; the session is honest (not silently `active`-and-dead).
    t.check(hub.child.poll() is None, "the HUB survives the adapter death")
    g = hub.get(f"/v1/sessions/{sid}")
    st = (g["json"] or {}).get("session", {}).get("status")
    # The file's own docstring: "the session must not keep claiming `active`". The old
    # assertion accepted `active` and hid the defect (docs/issues/20261005-020000).
    t.check(st != "active",
            "the session does NOT keep claiming `active` with no process behind it",
            f"status={st} (a processless session reporting `active` is the defect)")
finally:
    ok = t.done()
    hub.cleanup()
sys.exit(0 if ok else 1)

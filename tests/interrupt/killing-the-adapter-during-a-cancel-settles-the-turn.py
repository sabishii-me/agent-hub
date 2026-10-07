# PASSIVE M3: the adapter is KILLED while a cancel is IN FLIGHT (the abort was delivered,
# the process dies before it could confirm). The turn must still settle honestly and the
# session must not be left wedged. Real fault injection: cancel a real turn, kill the real
# adapter child in the same instant.
#
# FACT:    an adapter killed while a cancel is in flight still settles the turn (interrupted) and does not wedge the session
# SOURCE:  ARCHITECTURE s21 N2 (a turn open while cancelling -> interrupted)
# EXPOSES: a turn left running with no process (a hang)
# (A test that would pass whatever happens is not a test: this block names the fact it
#  proves and where that fact comes from; the assertions below are that exact fact.)
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "lib"))
from hub import Hub, hub_sha, register_provider, wait_turn, child_processes, kill_pid  # noqa: E402
from tally import Tally, combo, kind_of                          # noqa: E402

PI = os.environ.get("PI_PLUGIN_DIR", r"E:/AI/ideas/prts-harness-pi")
HARNESS = os.environ.get("PI_HARNESS_ID", "pi")
MODEL = os.environ.get("PI_MODEL", "deepseek-flash")

t = Tally("interrupt/kill-adapter-during-cancel")
combo(hub_sha(), kind_of(PI))
if not t.require(os.path.isdir(PI), "a real plugin is present", f"no real plugin at {PI}"):
    t.done(); sys.exit(1)

hub = Hub(plugins_src=PI)
try:
    hub.start()
    t.require(register_provider(hub) is not None, "a real provider is registered", "no real provider")
    r = hub.post("/v1/sessions", {"harnessId": HARNESS, "modelProviderId": "p", "modelId": MODEL}, key="kc-s")
    sid = r["json"]["session"]["id"]
    t.check(hub.wait_status(sid, "active").get("status") == "active", "the session is active")
    tr = hub.post(f"/v1/sessions/{sid}/turns", {"content": [{"type": "text", "text": "Count slowly from 1 to 500."}], "idempotencyKey": "kc-t"}, key="kc-t")
    tid = tr["json"]["turn"]["id"]
    row = wait_turn(hub, sid, tid, {"running", "awaiting_approval", "awaiting_question"}, tries=160)
    t.check(row.get("state") in ("running", "awaiting_approval", "awaiting_question"), "a turn is running", f"state={row.get('state')}")

    cn = hub.post(f"/v1/sessions/{sid}/cancel")
    t.check(cn["status"] < 300, "cancel is accepted", f"status={cn['status']}")
    kids = [pid for pid, nm in child_processes(hub.child.pid) if "node" in (nm or "").lower()]
    t.check(len(kids) >= 1, "the real adapter child is found", f"children={child_processes(hub.child.pid)}")
    for pid in kids:
        kill_pid(pid)

    end = wait_turn(hub, sid, tid, {"ended"}, tries=240)
    t.check(end.get("state") == "ended", "the turn settles after the adapter died mid-cancel (no hang)",
            f"state={end.get('state')}")
    # This race has TWO honest outcomes, and neither is a fake: if the adapter
    # CONFIRMED the abort before it died, the turn is `cancelled`; if it died with the
    # cancel unconfirmed, §21 N2 makes it `interrupted`. What must NEVER happen: a
    # `failed` (the adapter's death is not the model failing) or a `completed`.
    t.check(end.get("ended") in ("cancelled", "interrupted"),
            "a turn cancelling when the adapter died ends cancelled (confirmed) or interrupted (unconfirmed)",
            f"ended={end.get('ended')} (a `failed`/`completed` here would be wrong)")
    t.check(end.get("ended") != "failed",
            "the adapter's death is not reported as a model `failed`", f"ended={end.get('ended')}")
    t.check(hub.child.poll() is None, "the HUB survives")
    t.check(hub.get(f"/v1/sessions/{sid}")["status"] == 200, "the session still reads")
finally:
    ok = t.done()
    hub.cleanup()
sys.exit(0 if ok else 1)

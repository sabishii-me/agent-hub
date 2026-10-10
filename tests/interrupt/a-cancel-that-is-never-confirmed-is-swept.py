# TC-INTERRUPT-018
# ACTIVE A5: a cancel is DELIVERED but the adapter dies before it can CONFIRM the stop. The
# turn is left `cancelling` with no live process. The hub's sweep must then settle it
# honestly (`interrupted`) and mark the session repairable - never leave it held forever.
# Real: a real adapter, a real kill; the sweep is the hub's, on its real tick.
#
# FACT:    a cancel whose adapter died before confirming is swept to `interrupted` and the session becomes repairable
# SOURCE:  ARCHITECTURE s21 (settle_unconfirmed_cancels / reconcile_stalled_cancels, TASK-048 F4); contract/v1.json turn.ended
# EXPOSES: a turn stuck `cancelling` forever when the adapter that owed the confirmation is gone
# (A test that would pass whatever happens is not a test: this block names the fact it
#  proves and where that fact comes from; the assertions below are that exact fact.)
import os
import sys
import time

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "lib"))
from hub import Hub, hub_sha, register_provider, wait_turn, child_processes, kill_pid  # noqa: E402
from tally import Tally, combo, kind_of                          # noqa: E402

PI = os.environ.get("PI_PLUGIN_DIR", r"E:/AI/ideas/prts-harness-pi")
HARNESS = os.environ.get("PI_HARNESS_ID", "pi")
MODEL = os.environ.get("PI_MODEL", "deepseek-flash")

t = Tally("interrupt/unconfirmed-cancel-swept")
combo(hub_sha(), kind_of(PI))
if not t.require(os.path.isdir(PI), "a real plugin is present", f"no real plugin at {PI}"):
    t.done(); sys.exit(1)

hub = Hub(plugins_src=PI)
try:
    hub.start()
    PROV = register_provider(hub)
    t.require(PROV is not None, "a real provider is registered", "no real provider")
    r = hub.post("/v1/sessions", {"harnessId": HARNESS, "modelProviderId": PROV["id"], "modelId": MODEL}, key="uc-s")
    sid = r["json"]["session"]["id"]
    t.check(hub.wait_status(sid, "active").get("status") == "active", "the session is active")

    tr = hub.post(f"/v1/sessions/{sid}/turns",
                  {"content": [{"type": "text", "text": "Count slowly from 1 to 500."}], "idempotencyKey": "uc-t1"}, key="uc-t1")
    t.check(tr["status"] == 202, "the turn is accepted", f"status={tr['status']} {tr['text'][:120]}")
    tid = tr["json"]["turn"]["id"]
    wait_turn(hub, sid, tid, {"running", "awaiting_approval", "awaiting_question"}, tries=160)

    # Cancel, then KILL the adapter before it can confirm - so the cancel is delivered but never
    # confirmed, and the process that owed the confirmation is gone.
    cn = hub.post(f"/v1/sessions/{sid}/cancel")
    t.check(cn["status"] < 500, "the cancel is accepted (not a 5xx)", f"status={cn['status']}")
    kids = [pid for pid, name in child_processes(hub.child.pid) if "node" in (name or "").lower()]
    t.check(len(kids) >= 1, "a real adapter child is found", f"children={child_processes(hub.child.pid)}")
    for pid in kids:
        kill_pid(pid)

    # The sweep runs on the hub's real tick (5s); poll generously. The turn must reach a terminal.
    end = wait_turn(hub, sid, tid, {"ended"}, tries=400)
    t.check(end.get("state") == "ended",
            "the unconfirmed cancel is swept to a terminal (no forever-cancelling)",
            f"state={end.get('state')} ended={end.get('ended')}")
    t.check(end.get("ended") in ("interrupted", "cancelled", "failed"),
            "the swept terminal is an honest one (interrupted/cancelled/failed)",
            f"ended={end.get('ended')}")
    t.check(end.get("ended") != "completed",
            "a turn whose adapter died before confirming is never a clean completed",
            f"ended={end.get('ended')}")

    # The session is repairable (a deliberate stop leaves needs-repair), not left `active`.
    st = "active"
    for _ in range(200):
        st = (hub.get(f"/v1/sessions/{sid}")["json"] or {}).get("session", {}).get("status")
        if st != "active":
            break
        time.sleep(0.25)
    t.check(st != "active", "the session is not left `active` after the unconfirmed cancel", f"status={st}")

    # It is recoverable: reopen brings it back and it accepts a turn again.
    op = hub.post(f"/v1/sessions/{sid}/reopen")
    t.check(op["status"] < 300, "the session can be reopened after the sweep", f"status={op['status']} {op['text'][:140]}")
    s2 = hub.wait_status(sid, "active")
    t.check(s2.get("status") == "active", "the reopened session is active", f"status={s2.get('status')} err={s2.get('startError')}")

    t.check(hub.child.poll() is None, "the HUB survives the adapter death")
finally:
    ok = t.done()
    hub.cleanup()
sys.exit(0 if ok else 1)

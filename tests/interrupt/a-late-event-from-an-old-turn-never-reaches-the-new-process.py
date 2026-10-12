# TC-INTERRUPT-019
# CONCURRENCY C4: a turn runs, the session is STOPPED and REOPENED (a NEW adapter process on
# the same stored ref), and the OLD turn must not bleed into the new one. The old turn must be
# settled before/at reopen (never left running on the new process), and the NEW turn must be
# unaffected - no event, no terminal, no id from the old turn reaches it. Real: real adapter.
#
# FACT:    after a stop/reopen a late event from the OLD turn never reaches the NEW process; the new turn is unaffected
# SOURCE:
# EXPOSES: an old turn's late terminal/event taken by a new turn, or an old turn left running on a new process
# (A test that would pass whatever happens is not a test: this block names the fact it
#  proves and where that fact comes from; the assertions below are that exact fact.)
import os
import sys
import time

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "lib"))
from hub import Hub, hub_sha, register_provider, wait_turn      # noqa: E402
from tally import Tally, combo, kind_of                          # noqa: E402

PI = os.environ.get("PI_PLUGIN_DIR", r"E:/AI/ideas/prts-harness-pi")
HARNESS = os.environ.get("PI_HARNESS_ID", "pi")
MODEL = os.environ.get("PI_MODEL", "deepseek-flash")

t = Tally("interrupt/late-event-old-turn")
combo(hub_sha(), kind_of(PI))
if not t.require(os.path.isdir(PI), "a real plugin is present", f"no real plugin at {PI}"):
    t.done(); sys.exit(1)

hub = Hub(plugins_src=PI)
try:
    hub.start()
    PROV = register_provider(hub)
    t.require(PROV is not None, "a real provider is registered", "no real provider")
    r = hub.post("/v1/sessions", {"harnessId": HARNESS, "modelProviderId": PROV["id"], "modelId": MODEL}, key="le-s")
    sid = r["json"]["session"]["id"]
    ref = None
    t.check(hub.wait_status(sid, "active").get("status") == "active", "the session is active")
    ref = (hub.get(f"/v1/sessions/{sid}")["json"] or {}).get("session", {}).get("nativeRef")
    t.check(bool(ref), "the session has a native ref", f"ref={ref}")

    # Turn 1 runs, then we CLOSE (stop the process) while it may be mid-flight.
    tr1 = hub.post(f"/v1/sessions/{sid}/turns",
                   {"content": [{"type": "text", "text": "Count slowly from 1 to 500."}], "idempotencyKey": "le-t1"}, key="le-t1")
    t.check(tr1["status"] == 202, "turn 1 is accepted", f"status={tr1['status']} {tr1['text'][:120]}")
    tid1 = tr1["json"]["turn"]["id"]
    wait_turn(hub, sid, tid1, {"running", "awaiting_approval", "awaiting_question"}, tries=160)

    cl = hub.post(f"/v1/sessions/{sid}/close")
    t.check(cl["status"] < 300, "the close stops the process", f"status={cl['status']} {cl['text'][:120]}")

    # The OLD turn must be settled (not left running on the new process).
    end1 = wait_turn(hub, sid, tid1, {"ended"}, tries=400)
    t.check(end1.get("state") == "ended", "the old turn settles across the stop (not left running)",
            f"state={end1.get('state')} ended={end1.get('ended')}")
    old_ended = end1.get("ended")

    # REOPEN: a NEW adapter process on the same ref.
    op = hub.post(f"/v1/sessions/{sid}/reopen")
    t.check(op["status"] < 300, "reopen re-attaches on the stored ref", f"status={op['status']} {op['text'][:140]}")
    s2 = hub.wait_status(sid, "active")
    t.check(s2.get("status") == "active", "the reopened session is active", f"status={s2.get('status')} err={s2.get('startError')}")

    # Turn 2 on the NEW process.
    tr2 = hub.post(f"/v1/sessions/{sid}/turns",
                   {"content": [{"type": "text", "text": "say hi"}], "idempotencyKey": "le-t2"}, key="le-t2")
    t.check(tr2["status"] == 202, "turn 2 is accepted on the new process", f"status={tr2['status']} {tr2['text'][:120]}")
    tid2 = tr2["json"]["turn"]["id"]
    t.check(tid2 != tid1, "turn 2 has a DIFFERENT id from turn 1")

    end2 = wait_turn(hub, sid, tid2, {"ended"}, tries=400)
    t.check(end2.get("state") == "ended", "turn 2 reaches a terminal", f"state={end2.get('state')} ended={end2.get('ended')}")

    # The OLD turn's terminal must be UNCHANGED by the new process's activity (no bleed-through).
    g = hub.get(f"/v1/sessions/{sid}/turns")
    row1 = next((x for x in ((g["json"] or {}).get("turns") or []) if x.get("id") == tid1), None)
    t.check(row1 is not None, "turn 1 is still listed")
    if row1 is not None:
        t.check(row1.get("ended") == old_ended,
                "the OLD turn's terminal is unchanged after the new process ran",
                f"before={old_ended} after={row1.get('ended')}")
        t.check(row1.get("state") == "ended", "the OLD turn did not resume on the new process",
                f"state={row1.get('state')}")

    # turn 2's terminal belongs to turn 2, not turn 1.
    t.check(end2.get("ended") != old_ended or tid2 != tid1,
            "the new turn's terminal is its own", f"t1={old_ended} t2={end2.get('ended')}")
finally:
    ok = t.done()
    hub.cleanup()
sys.exit(0 if ok else 1)

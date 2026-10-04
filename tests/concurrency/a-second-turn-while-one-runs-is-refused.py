# C3: while a turn RUNS on a session, a second turn must be REFUSED (409
# `session_busy`), never queued and never dispatched. This is the one-at-a-time rule
# the contract states; a queued second turn would run two prompts on one process and
# the abort/cancel binding would be ambiguous. No fake: a real provider drives a real
# turn on the real pi adapter.
import os
import sys
import time

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "lib"))
from hub import Hub, hub_sha, register_provider, wait_turn      # noqa: E402
from tally import Tally, combo, kind_of                          # noqa: E402

PI = os.environ.get("PI_PLUGIN_DIR", r"E:/AI/ideas/prts-harness-pi")
HARNESS = os.environ.get("PI_HARNESS_ID", "pi")
MODEL = os.environ.get("PI_MODEL", "deepseek-flash")

t = Tally("concurrency/busy")
combo(hub_sha(), kind_of(PI))
if not t.require(os.path.isdir(PI), "a real plugin is present", f"no real plugin at {PI}"):
    t.done(); sys.exit(1)

hub = Hub(plugins_src=PI)
try:
    hub.start()
    t.require(register_provider(hub) is not None, "a real provider is registered", "no real provider in ~/.pi/agent/models.json")
    r = hub.post("/v1/sessions", {"harnessId": HARNESS, "modelProviderId": "p", "modelId": MODEL}, key="c3-s")
    sid = r["json"]["session"]["id"]
    s = hub.wait_status(sid, "active")
    t.check(s.get("status") == "active", "the session is active", f"status={s.get('status')} err={s.get('startError')}")

    # A LONG prompt so the turn is still running when the second arrives.
    tr = hub.post(f"/v1/sessions/{sid}/turns", {"content": [{"type": "text", "text": "Count slowly from 1 to 400, one number per line."}], "idempotencyKey": "c3-t1"}, key="c3-t1")
    t.check(tr["status"] == 202, "the first turn is accepted", f"status={tr['status']}")
    tid = tr["json"]["turn"]["id"]

    row = wait_turn(hub, sid, tid, {"running", "awaiting_approval", "awaiting_question", "ended"}, tries=160)
    running = row.get("state") in ("running", "awaiting_approval", "awaiting_question")

    if not running:
        # The model answered before we could observe it; the busy rule cannot be
        # exercised THIS run. Say so as a FAILURE of the exercise, not a fake pass.
        t.check(False, "the first turn is observed running (needed to test the busy rule)",
                f"state={row.get('state')} (the model finished too fast to observe)")
    else:
        # The second turn, DIFFERENT idempotencyKey (a new intent): must be refused.
        tr2 = hub.post(f"/v1/sessions/{sid}/turns", {"content": [{"type": "text", "text": "say hi"}], "idempotencyKey": "c3-t2"}, key="c3-t2")
        t.check(tr2["status"] == 409, "a second turn while one runs is refused 409", f"status={tr2['status']} {tr2['text'][:140]}")
        t.check((tr2["json"] or {}).get("error") == "session_busy", "the refusal is `session_busy`", f"body={tr2['text'][:140]}")
        # It was NOT queued: no second turn row exists.
        lst = hub.get(f"/v1/sessions/{sid}/turns")
        ids = [x.get("id") for x in (lst["json"] or {}).get("turns", [])]
        t.check(ids.count(tid) == 1 and len(ids) == 1, "the refused turn was NOT queued (only the first exists)", f"turnIds={ids}")
        # Cancel the running turn so the test ends clean.
        hub.post(f"/v1/sessions/{sid}/cancel")
        wait_turn(hub, sid, tid, {"ended"}, tries=240)
finally:
    ok = t.done()
    hub.cleanup()
sys.exit(0 if ok else 1)

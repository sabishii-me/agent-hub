# A cancel is in flight (the turn is `cancelling`, the adapter may not have stopped). A NEW
# turn is sent AT ONCE, in the same instant. The hub must NOT dispatch a new prompt onto a
# process it is still aborting, and must not wedge: either the new turn is admitted AFTER
# the cancel settles (and runs on a clean process), or it is refused while the old turn is
# still held. What it must NEVER do: run two prompts at once, or leave the session stuck.
# Real provider, real adapter, real abort.
#
# FACT:    a turn sent the instant a cancel is in flight has a DEFINED outcome (admitted or 4xx), never a 5xx/hang
# SOURCE:  contract/v1.json POST /v1/sessions/{id}/turns
# EXPOSES: a 5xx or a hang from the immediate follow-up
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

t = Tally("interrupt/send-after-cancel")
combo(hub_sha(), kind_of(PI))
if not t.require(os.path.isdir(PI), "a real plugin is present", f"no real plugin at {PI}"):
    t.done(); sys.exit(1)

hub = Hub(plugins_src=PI)
try:
    hub.start()
    t.require(register_provider(hub) is not None, "a real provider is registered", "no real provider")
    r = hub.post("/v1/sessions", {"harnessId": HARNESS, "modelProviderId": "p", "modelId": MODEL}, key="sc-s")
    sid = r["json"]["session"]["id"]
    t.check(hub.wait_status(sid, "active").get("status") == "active", "the session is active")

    tr = hub.post(f"/v1/sessions/{sid}/turns", {"content": [{"type": "text", "text": "Count slowly from 1 to 500, one number per line."}], "idempotencyKey": "sc-t1"}, key="sc-t1")
    t.check(tr["status"] == 202, "the first turn is accepted", f"status={tr['status']}")
    tid1 = tr["json"]["turn"]["id"]
    row = wait_turn(hub, sid, tid1, {"running", "awaiting_approval", "awaiting_question"}, tries=160)
    running = row.get("state") in ("running", "awaiting_approval", "awaiting_question")
    t.check(running, "the first turn is running before we cancel", f"state={row.get('state')}")

    if not running:
        t.done(); sys.exit(1)

    # CANCEL, then IMMEDIATELY send a new turn (no wait).
    cn = hub.post(f"/v1/sessions/{sid}/cancel")
    t.check(cn["status"] < 300, "cancel is accepted", f"status={cn['status']}")
    tr2 = hub.post(f"/v1/sessions/{sid}/turns", {"content": [{"type": "text", "text": "say hi"}], "idempotencyKey": "sc-t2"}, key="sc-t2")
    # EITHER refused (while the cancel is unconfirmed) OR admitted - never a 500/hang.
    # The real property: a DEFINED outcome (2xx admitted, or any 4xx refusal). The old
    # (202,409,422,503) set was a fake: it silently excluded a legitimate 400 and would
    # fail on it, while calling itself "never a 5xx".
    t.check(tr2["status"] < 500, "the immediate second turn is refused or accepted, never a 5xx",
            f"status={tr2['status']} {tr2['text'][:160]}")
    t.check(tr2["status"] == 202 or 400 <= tr2["status"] < 500,
            "the immediate second turn has a DEFINED outcome (admitted or 4xx-refused)",
            f"status={tr2['status']} {tr2['text'][:160]}")

    # The first turn settles.
    e1 = wait_turn(hub, sid, tid1, {"ended"}, tries=300)
    t.check(e1.get("state") == "ended", "the cancelled turn settles", f"state={e1.get('state')} ended={e1.get('ended')}")

    # The session is usable: a turn NOW must be accepted (occupancy released).
    tr3 = hub.post(f"/v1/sessions/{sid}/turns", {"content": [{"type": "text", "text": "say hi again"}], "idempotencyKey": "sc-t3"}, key="sc-t3")
    t.check(tr3["status"] == 202, "a turn AFTER the cancel settled is accepted (session not wedged)",
            f"status={tr3['status']} {tr3['text'][:160]}")
    if tr3["status"] == 202:
        tid3 = tr3["json"]["turn"]["id"]
        e3 = wait_turn(hub, sid, tid3, {"ended"}, tries=300)
        t.check(e3.get("state") == "ended" and e3.get("ended") == "completed",
                "the post-cancel turn runs cleanly to `completed`", f"ended={e3.get('ended')}")

    # And if the immediate turn WAS admitted, it must also reach a terminal (no orphan).
    if tr2["status"] == 202:
        tid2 = tr2["json"]["turn"]["id"]
        e2 = wait_turn(hub, sid, tid2, {"ended"}, tries=300)
        t.check(e2.get("state") == "ended", "the immediately-sent turn also reaches a terminal (no orphan)",
                f"state={e2.get('state')} ended={e2.get('ended')}")
finally:
    ok = t.done()
    hub.cleanup()
sys.exit(0 if ok else 1)

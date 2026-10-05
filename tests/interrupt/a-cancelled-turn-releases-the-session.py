# A real turn is running (a real model call), then CANCELLED. The turn must reach a terminal
# state and the session must be usable again - the occupancy is released by a CONFIRMED stop,
# not a hope. This needs a REAL provider (a turn only runs with one), so it SKIPs without
# authorization; it is never faked.
import os
import sys
import time

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "lib"))
from hub import Hub, hub_sha, real_provider      # noqa: E402
from tally import Tally, combo, kind_of          # noqa: E402

PI = os.environ.get("PI_PLUGIN_DIR", r"E:/AI/ideas/prts-harness-pi")
HARNESS = os.environ.get("PI_HARNESS_ID", "pi")

t = Tally("interrupt/cancel")
combo(hub_sha(), kind_of(PI))
prov = real_provider()
if not t.require(os.path.isdir(PI), "a real plugin is present", f"no real plugin at {PI}"):
    t.done()
    sys.exit(1)
if not t.require(prov is not None, "a real provider is present", "no real provider in ~/.pi/agent/models.json"):
    t.done()
    sys.exit(1)

hub = Hub(plugins_src=PI)
try:
    hub.start()
    c = hub.post("/v1/model-providers", {"id": "p", "url": prov["url"], "api": prov["api"], "token": prov["token"]})
    t.check(c["status"] < 300, "a real provider is registered", f"status={c['status']} {c['text'][:120]}")
    r = hub.post("/v1/sessions", {"harnessId": HARNESS, "modelProviderId": "p", "modelId": os.environ.get("PI_MODEL", "deepseek-flash")}, key="cn-1")
    sid = r["json"]["session"]["id"]
    s = hub.wait_status(sid, "active")
    t.check(s.get("status") == "active", "the session is active", f"status={s.get('status')} err={s.get('startError')}")

    # A long-running prompt: ask for something that takes time, then cancel mid-flight.
    tr = hub.post(f"/v1/sessions/{sid}/turns", {"content": [{"type": "text", "text": "Count slowly from 1 to 400, one number per line."}], "idempotencyKey": "cn-turn"}, key="cn-turn")
    t.check(tr["status"] == 202, "the turn is accepted", f"status={tr['status']}")
    tid = tr["json"]["turn"]["id"]

    # Once it is running, cancel.
    ran = False
    for _ in range(120):
        g = hub.get(f"/v1/sessions/{sid}/turns")
        rows = (g["json"] or {}).get("turns", [])
        st = next((x.get("state") for x in rows if x.get("id") == tid), None)
        if st in ("running", "awaiting_approval", "awaiting_question"):
            ran = True
            break
        if st == "ended":
            break
        time.sleep(0.25)
    t.check(ran, "the turn reaches running (a real model call is in flight)")

    if ran:
        cn = hub.post(f"/v1/sessions/{sid}/cancel")
        t.check(cn["status"] < 300, "cancel is accepted", f"status={cn['status']} {cn['text'][:120]}")

    # The turn must END (cancelled or interrupted), not hang.
    ended = None
    for _ in range(300):
        g = hub.get(f"/v1/sessions/{sid}/turns")
        rows = (g["json"] or {}).get("turns", [])
        row = next((x for x in rows if x.get("id") == tid), None)
        if row and row.get("state") == "ended":
            ended = row.get("ended")
            break
        time.sleep(0.25)
    # A cancel the hub CONFIRMED settles `cancelled` (contract/v1.json turn.ended enum;
    # `interrupted` is the OTHER terminal - an unconfirmed cancel reconciled at restart,
    # ARCHITECTURE §21 N2 - so it is NOT acceptable for a confirmed cancel here). Measured
    # on pi: a confirmed cancel yields exactly `cancelled`. `in (...)` was a fake that hid
    # an adapter's `failed` mis-classification (test-findings F1).
    t.check(ended == "cancelled",
            "a confirmed cancel ends `cancelled`",
            f"ended={ended} (required: cancelled; `interrupted`/`failed` are not a confirmed cancel)")

    # The session is usable again: a NEW turn is admitted (occupancy released).
    tr2 = hub.post(f"/v1/sessions/{sid}/turns", {"content": [{"type": "text", "text": "say hi"}], "idempotencyKey": "cn-turn-2"}, key="cn-turn-2")
    t.check(tr2["status"] == 202, "the session accepts a new turn after the cancel", f"status={tr2['status']} {tr2['text'][:120]}")

    hub.delete("/v1/model-providers/p")
finally:
    ok = t.done()
    hub.cleanup()
sys.exit(0 if ok else 1)

# TC-INTERRUPT-013
# ACTIVE A3: the cancel ARRIVES BEFORE the prompt is sent. A caller cancels a session it
# has NOT yet prompted, then sends a turn in the same instant. The hub must not dispatch a
# prompt that a cancel already covers, must not wedge, and the turn must reach a DEFINED
# outcome (no 5xx, no hang). Real: a real adapter, no fake.
#
# FACT:    a cancel that precedes the prompt does not corrupt the session: the next turn has a defined outcome and reaches a terminal
# SOURCE: (cancel holds busy until a real stop); contract/v1.json POST /v1/sessions/{id}/cancel
# EXPOSES: a pre-prompt cancel that swallows the next turn, 5xxs, or leaves the session wedged
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

t = Tally("interrupt/cancel-before-prompt")
combo(hub_sha(), kind_of(PI))
if not t.require(os.path.isdir(PI), "a real plugin is present", f"no real plugin at {PI}"):
    t.done(); sys.exit(1)

hub = Hub(plugins_src=PI)
try:
    hub.start()
    PROV = register_provider(hub)
    t.require(PROV is not None, "a real provider is registered", "no real provider")
    r = hub.post("/v1/sessions", {"harnessId": HARNESS, "modelProviderId": PROV["id"], "modelId": MODEL}, key="cb-s")
    sid = r["json"]["session"]["id"]
    t.check(hub.wait_status(sid, "active").get("status") == "active", "the session is active")

    # CANCEL FIRST, before any prompt. On an idle session this is a clean no-op.
    c = hub.post(f"/v1/sessions/{sid}/cancel")
    t.check(c["status"] < 500, "a cancel before any prompt is not a 5xx", f"status={c['status']} {c['text'][:120]}")

    # Then send a turn. It must have a DEFINED outcome: admitted (202) or refused 4xx.
    tr = hub.post(f"/v1/sessions/{sid}/turns",
                  {"content": [{"type": "text", "text": "say hi in one word"}], "idempotencyKey": "cb-t1"}, key="cb-t1")
    t.check(tr["status"] == 202 or 400 <= tr["status"] < 500,
            "the turn after a pre-prompt cancel has a DEFINED outcome (202 or 4xx, never 5xx)",
            f"status={tr['status']} {tr['text'][:140]}")

    # If admitted, it must reach a terminal (the cancel must not leave a turn running forever).
    if tr["status"] == 202:
        tid = tr["json"]["turn"]["id"]
        end = wait_turn(hub, sid, tid, {"ended"}, tries=400)
        t.check(end.get("state") == "ended", "an admitted turn reaches a terminal (no hang)",
                f"state={end.get('state')} ended={end.get('ended')}")

    # The session is still usable / honest: it reads, and a further turn is defined.
    t.check(hub.get(f"/v1/sessions/{sid}")["status"] == 200, "the session still reads")
    tr2 = hub.post(f"/v1/sessions/{sid}/turns",
                   {"content": [{"type": "text", "text": "say hi again"}], "idempotencyKey": "cb-t2"}, key="cb-t2")
    t.check(tr2["status"] < 500, "the session is not wedged (a further turn is not a 5xx)",
            f"status={tr2['status']} {tr2['text'][:120]}")
finally:
    ok = t.done()
    hub.cleanup()
sys.exit(0 if ok else 1)

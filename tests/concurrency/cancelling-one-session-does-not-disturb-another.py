# C2: two sessions run turns at once; cancelling ONE must not disturb the other. The
# abort is bound to the cancelled turn's own process generation, so the other session's
# turn must keep running and settle on its own. No fake: a real provider drives two real
# turns on two real sessions.
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "lib"))
from hub import Hub, hub_sha, register_provider, wait_turn      # noqa: E402
from tally import Tally, combo, kind_of                          # noqa: E402

PI = os.environ.get("PI_PLUGIN_DIR", r"E:/AI/ideas/prts-harness-pi")
HARNESS = os.environ.get("PI_HARNESS_ID", "pi")
MODEL = os.environ.get("PI_MODEL", "deepseek-flash")

t = Tally("concurrency/cross-talk")
combo(hub_sha(), kind_of(PI))
if not t.require(os.path.isdir(PI), "a real plugin is present", f"no real plugin at {PI}"):
    t.done(); sys.exit(1)

hub = Hub(plugins_src=PI)
try:
    hub.start()
    t.require(register_provider(hub) is not None, "a real provider is registered", "no real provider")
    a = hub.post("/v1/sessions", {"harnessId": HARNESS, "modelProviderId": "p", "modelId": MODEL}, key="cc-a")
    b = hub.post("/v1/sessions", {"harnessId": HARNESS, "modelProviderId": "p", "modelId": MODEL}, key="cc-b")
    sid_a = a["json"]["session"]["id"]
    sid_b = b["json"]["session"]["id"]
    t.check(hub.wait_status(sid_a, "active").get("status") == "active", "session A is active")
    t.check(hub.wait_status(sid_b, "active").get("status") == "active", "session B is active")

    ta = hub.post(f"/v1/sessions/{sid_a}/turns", {"content": [{"type": "text", "text": "Count slowly from 1 to 400."}], "idempotencyKey": "cc-ta"}, key="cc-ta")
    tb = hub.post(f"/v1/sessions/{sid_b}/turns", {"content": [{"type": "text", "text": "Count slowly from 1 to 400."}], "idempotencyKey": "cc-tb"}, key="cc-tb")
    t.check(ta["status"] == 202 and tb["status"] == 202, "both turns are accepted")
    tid_a = ta["json"]["turn"]["id"]
    tid_b = tb["json"]["turn"]["id"]

    rowa = wait_turn(hub, sid_a, tid_a, {"running", "awaiting_approval", "awaiting_question", "ended"}, tries=160)
    rowb = wait_turn(hub, sid_b, tid_b, {"running", "awaiting_approval", "awaiting_question", "ended"}, tries=160)
    both = rowa.get("state") in ("running", "awaiting_approval", "awaiting_question") and \
           rowb.get("state") in ("running", "awaiting_approval", "awaiting_question")
    t.check(both, "both turns run at once", f"a={rowa.get('state')} b={rowb.get('state')}")

    if both:
        # Cancel A ONLY.
        cn = hub.post(f"/v1/sessions/{sid_a}/cancel")
        t.check(cn["status"] < 300, "cancel A is accepted", f"status={cn['status']}")
        # A settles.
        enda = wait_turn(hub, sid_a, tid_a, {"ended"}, tries=240)
        t.check(enda.get("state") == "ended", "A's turn settles", f"state={enda.get('state')} ended={enda.get('ended')}")
        # B MUST still be running (no cross-talk) - read it immediately after A settled.
        gb = hub.get(f"/v1/sessions/{sid_b}/turns")
        rowb2 = next((x for x in (gb["json"] or {}).get("turns", []) if x.get("id") == tid_b), None)
        t.check(rowb2 and rowb2.get("state") != "ended",
                "B is NOT disturbed by A's cancel (still running)",
                f"b_state={rowb2.get('state') if rowb2 else None}")
        # Let B finish on its own (cancel it to end the test cleanly, then wait).
        hub.post(f"/v1/sessions/{sid_b}/cancel")
        wait_turn(hub, sid_b, tid_b, {"ended"}, tries=240)
        # Both sessions remain usable.
        t.check(hub.get(f"/v1/sessions/{sid_a}")["status"] == 200, "A still reads")
        t.check(hub.get(f"/v1/sessions/{sid_b}")["status"] == 200, "B still reads")
finally:
    ok = t.done()
    hub.cleanup()
sys.exit(0 if ok else 1)

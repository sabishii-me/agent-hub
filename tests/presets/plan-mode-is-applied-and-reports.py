# `plan` is a session policy knob (PATCH /v1/sessions/{id} {plan:true|false}). The contract
# distinguishes TWO fields: `plan` (the REQUESTED state) and `appliedPlan` (what the ADAPTER
# actually applied - "proof"). This file tests both, separately, and does NOT claim runtime
# behaviour:
#  (A)  the hub RECORDS the requested plan        -> `plan`
#  (B)  the adapter's APPLIED plan is reported    -> `appliedPlan`
#  (B2) switch OFF: both fields read false
#  (C)  plan actually CONSTRAINS runtime behaviour -> UNVERIFIED (BLOCKED; no contract-pinned
#       observable, and it is not inferred from the fields above)
#
# FACT:    PATCH plan sets `plan` (request) and the adapter reports `appliedPlan` (applied);
#          runtime behavioural enforcement is NOT claimed
# SOURCE:  ARCHITECTURE s23; contract/v1.json session `plan` vs `appliedPlan`
# EXPOSES: a plan knob whose request is stored but never applied (A passes, B fails)
# (A test that would pass whatever happens is not a test: this block names the fact it
#  proves and where that fact comes from; the assertions below are that exact fact.)
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "lib"))
from hub import Hub, hub_sha, register_provider, wait_turn      # noqa: E402
from tally import Tally, combo, kind_of              # noqa: E402

PI = os.environ.get("PI_PLUGIN_DIR", r"E:/AI/ideas/prts-harness-pi")
HARNESS = os.environ.get("PI_HARNESS_ID", "pi")
MODEL = os.environ.get("PI_MODEL", "deepseek-flash")

t = Tally("presets/plan")
combo(hub_sha(), kind_of(PI))
if not t.require(os.path.isdir(PI), "a real plugin is present", f"no real plugin at {PI}"):
    t.done(); sys.exit(1)

hub = Hub(plugins_src=PI)
try:
    hub.start()
    t.require(register_provider(hub) is not None, "a real provider is registered", "no real provider")
    r = hub.post("/v1/sessions", {"harnessId": HARNESS, "modelProviderId": "p", "modelId": MODEL}, key="pl-s")
    sid = r["json"]["session"]["id"]
    s = hub.wait_status(sid, "active")
    t.check(s.get("status") == "active", "the session is active", f"status={s.get('status')} err={s.get('startError')}")

    # (A) CONFIG READ/WRITE: the hub records the REQUESTED plan (`plan`). This proves storage, not
    # application.
    p = hub.patch(f"/v1/sessions/{sid}", {"plan": True})
    t.check(p["status"] < 300, "PATCH plan:true is accepted", f"status={p['status']} {p['text'][:160]}")
    sess = (p["json"] or {}).get("session", (p["json"] or {}))
    t.check(sess.get("plan") is True,
            "(A) the hub RECORDS the requested plan:true (`plan` - a stored request, not application)",
            f"plan={sess.get('plan')}")

    # (B) APPLIED REPORT: the contract's field for what the ADAPTER applied is `appliedPlan` ("proof"),
    # distinct from `plan` ("the request"). Assert the APPLIED field, not the request field.
    sess2 = (hub.get(f"/v1/sessions/{sid}")["json"] or {}).get("session", {})
    applied = sess2.get("appliedPlan")
    t.check(applied is True,
            "(B) the adapter's APPLIED plan is reported true (`appliedPlan` - the contract's proof field)",
            f"appliedPlan={applied} plan={sess2.get('plan')} err={sess2.get('startError')}")

    # (C) ACTUAL RUNTIME BEHAVIOUR under plan (e.g. a write is refused/hidden). This needs a
    # behaviour the contract makes observable; we do NOT infer it from `plan`/`appliedPlan` and do
    # NOT fabricate it. UNVERIFIED here.
    t.blocked_check(
        "(C) plan ACTUALLY constrains runtime behaviour (a write under plan is refused/hidden)",
        "no contract-pinned observable for it; not inferred from the plan/appliedPlan FIELDS")

    # (B2) Switch OFF and confirm the adapter reports appliedPlan false.
    p2 = hub.patch(f"/v1/sessions/{sid}", {"plan": False}, key="plan-off")
    t.check(p2["status"] < 300, "PATCH plan:false is accepted", f"status={p2['status']} {p2['text'][:160]}")
    sess3 = (hub.get(f"/v1/sessions/{sid}")["json"] or {}).get("session", {})
    t.check(sess3.get("plan") is False and sess3.get("appliedPlan") is False,
            "(B2) plan:false is recorded AND the adapter reports appliedPlan false",
            f"plan={sess3.get('plan')} appliedPlan={sess3.get('appliedPlan')}")
finally:
    ok = t.done()
    hub.cleanup()
sys.exit(0 if ok else 1)

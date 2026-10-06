# `plan` is a session policy knob (PATCH /v1/sessions/{id} {plan:true|false}); the hub
# forwards it to the adapter's plan extension and follows the harness's own plan state
# (the contract's plan.changed). A PATCH to true must be APPLIED (the session reports
# plan true, or the change is refused with a reason) - never silently ignored. Exposes the
# case where the plan knob does nothing. Real adapter; no model call needed to toggle.
#
# FACT:    PATCH plan:true is APPLIED (the session reports plan true), and plan:false is accepted
# SOURCE:  ARCHITECTURE s23; contract/v1.json PATCH session policy plan
# EXPOSES: a plan knob that silently does nothing
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

    # Turn plan ON.
    p = hub.patch(f"/v1/sessions/{sid}", {"plan": True})
    t.check(p["status"] < 300, "PATCH plan:true is accepted", f"status={p['status']} {p['text'][:160]}")
    body = p["json"] or {}
    sess = body.get("session", body)
    # pi SHIPS the plan extension, so plan:true must be genuinely APPLIED: the session
    # reports `plan == True`. The contract's warning path is for a harness that CANNOT
    # apply it; accepting a warning here was a fake that passed even when plan did
    # nothing (the very failure this file exists to expose).
    reported = sess.get("plan")
    # The bool is the contract's APPLIED read-back; it is a CLAIM. The behavioural proof that plan
    # is really in effect (below) is what makes it non-fake: under plan a write tool must NOT run.
    # The contract's rule for a policy field is 'applied fields must confirm ACTUAL state'. The
    # confirmation for plan is the session's read-back of the plan state the HARNESS holds - not a
    # side-effect guess. A 'write did not happen' is NOT the product's contract (a provider error or
    # a non-executing model would also produce it) and was therefore retracted; a turn that writes
    # under plan would be a harness bug we would see as plan_off/plan_changed arriving instead.
    t.check(reported is True,
            "plan:true is APPLIED and CONFIRMED by the session's read-back (the contract's confirmation)",
            f"plan={reported} warning={body.get('warning') or sess.get('warning')}")

    # Plan is switchable: turn it OFF and confirm the harness now holds false.
    p2 = hub.patch(f"/v1/sessions/{sid}", {"plan": False}, key="plan-off")
    t.check(p2["status"] < 300, "PATCH plan:false is accepted", f"status={p2['status']} {p2['text'][:160]}")
    sess2 = (hub.get(f"/v1/sessions/{sid}")["json"] or {}).get("session", {})
    t.check(sess2.get("plan") is False, "plan:false is CONFIRMED by the read-back (it really switched)",
            f"plan={sess2.get('plan')}")
finally:
    ok = t.done()
    hub.cleanup()
sys.exit(0 if ok else 1)

# `plan` is a session policy knob (PATCH /v1/sessions/{id} {plan:true|false}); the hub
# forwards it to the adapter's plan extension and follows the harness's own plan state
# (the contract's plan.changed). A PATCH to true must be APPLIED (the session reports
# plan true, or the change is refused with a reason) - never silently ignored. Exposes the
# case where the plan knob does nothing. Real adapter; no model call needed to toggle.
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "lib"))
from hub import Hub, hub_sha, register_provider      # noqa: E402
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
    # The hub follows the harness. Either the response reports plan true, or a warning says
    # it was not confirmed - a silent no-op is the failure we are exposing.
    reported = sess.get("plan")
    warning = body.get("warning") or sess.get("warning")
    t.check(reported is True or warning,
            "plan:true is either APPLIED (plan=true) or honestly warned (never a silent no-op)",
            f"plan={reported} warning={warning}")

    # Turn plan OFF again (a preset that does not ask must be switchable away).
    p2 = hub.patch(f"/v1/sessions/{sid}", {"plan": False})
    t.check(p2["status"] < 300, "PATCH plan:false is accepted", f"status={p2['status']} {p2['text'][:160]}")
finally:
    ok = t.done()
    hub.cleanup()
sys.exit(0 if ok else 1)

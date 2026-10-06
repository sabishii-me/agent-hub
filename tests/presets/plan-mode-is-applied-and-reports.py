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
    t.check(reported is True,
            "plan:true is APPLIED (the session reports plan true)",
            f"plan={reported} warning={body.get('warning') or sess.get('warning')}")

    # BEHAVIOURAL proof that plan is in effect: under plan, a turn that asks for a WRITE must not
    # produce the file (plan mode is read-only). A field alone is not execution.
    cwd = sess.get("cwd")
    if cwd:
        plan_target = os.path.join(cwd, "plan-must-not-write.txt")
        tr = hub.post(f"/v1/sessions/{sid}/turns", {
            "content": [{"type": "text", "text": f"Use the write tool to create {plan_target} with the word PLAN."}],
            "idempotencyKey": "plan-write"}, key="plan-write")
        if (tr["json"] or {}).get("turn"):
            tid = tr["json"]["turn"]["id"]
            row = wait_turn(hub, sid, tid, {"ended"}, tries=240)
            # The turn must have REACHED a terminal - a provider error or a hang must not pass.
            t.check(row.get("state") == "ended", "the plan-mode write turn reached a terminal",
                    f"state={row.get('state')}")
            t.check(not os.path.exists(plan_target),
                    "under plan, a requested WRITE did not happen",
                    f"target={plan_target} exists={os.path.exists(plan_target)}")
        else:
            t.check(False, "the plan-mode write turn was accepted", f"status={tr['status']} {tr['text'][:120]}")

        # CAUSALITY: with plan OFF, the SAME write on a fresh target MUST now happen. Without this,
        # 'no file' could just mean the tools are broken - not that plan blocked it.
        p_off = hub.patch(f"/v1/sessions/{sid}", {"plan": False}, key="plan-off")
        t.check(p_off["status"] < 300, "plan:false is accepted (to test restoration)", f"status={p_off['status']}")
        off_target = os.path.join(cwd, "plan-off-should-write.txt")
        tr2 = hub.post(f"/v1/sessions/{sid}/turns", {
            "content": [{"type": "text", "text": f"Use the write tool to create {off_target} with the word OFF."}],
            "idempotencyKey": "plan-off-write"}, key="plan-off-write")
        if (tr2["json"] or {}).get("turn"):
            row2 = wait_turn(hub, sid, tr2["json"]["turn"]["id"], {"ended"}, tries=240)
            t.check(row2.get("state") == "ended", "the plan-OFF write turn reached a terminal", f"state={row2.get('state')}")
            t.check(os.path.exists(off_target),
                    "with plan OFF, the SAME write DOES happen (plan, not a broken tool, was the difference)",
                    f"target={off_target} exists={os.path.exists(off_target)}")
        else:
            t.check(False, "the plan-OFF write turn was accepted", f"status={tr2['status']} {tr2['text'][:120]}")
        p2 = p_off
    else:
        t.check(False, "the session reports a cwd (to test plan's read-only effect)", "no cwd")
        p2 = hub.patch(f"/v1/sessions/{sid}", {"plan": False})
    t.check(p2["status"] < 300, "PATCH plan:false is accepted", f"status={p2['status']} {p2['text'][:160]}")
finally:
    ok = t.done()
    hub.cleanup()
sys.exit(0 if ok else 1)

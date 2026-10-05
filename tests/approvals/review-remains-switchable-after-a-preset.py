# The preset's `approve` is the session's STARTING review state, not a nail. After the fix for
# 20261004-050000 (a preset reported APPLIED but not enforced), review must still be switchable
# within the session: `/review off` (PATCH review:false) lets tools run without asking, and
# `/review on` (PATCH review:true) asks again. This test goes RED if the fix froze review on, or
# if a switched state does not survive (a later spawn must not silently re-enable the preset).
#
# Real provider, real preset, real approval round-trip. No mock, no skip.
#
# FACT:    review is an in-session switch: a preset may start it on, and PATCH review:false/true toggles it for the NEXT tool
# SOURCE:  ARCHITECTURE s23; contract/v1.json PATCH session policy review
# EXPOSES: review frozen by a preset, or a switched state lost on a later spawn (20261004-070000)
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

t = Tally("approvals/review-toggle")
combo(hub_sha(), kind_of(PI))
if not t.require(os.path.isdir(PI), "a real plugin is present", f"no real plugin at {PI}"):
    t.done(); sys.exit(1)


def run_read_turn(hub, sid, tag):
    """Drive a turn that reads a file; return (approval, turn_id). An approval
    appears only while review is on."""
    tr = hub.post(f"/v1/sessions/{sid}/turns", {
        "content": [{"type": "text", "text": "Use the read tool to read E:/AI/ideas/prts-hub/Cargo.toml, then reply with its first line."}],
        "idempotencyKey": tag,
    }, key=tag)
    tid = tr["json"]["turn"]["id"]
    approval = None
    for _ in range(80):
        rows = (hub.get(f"/v1/sessions/{sid}/approvals")["json"] or {}).get("approvals", [])
        if rows:
            approval = rows[0]
            break
        if wait_turn(hub, sid, tid, {"ended"}, tries=1).get("state") == "ended":
            break
        time.sleep(0.25)
    return approval, tid


hub = Hub(plugins_src=PI)
try:
    hub.start()
    t.require(register_provider(hub) is not None, "a real provider is registered", "no real provider")
    r = hub.post("/v1/sessions", {"harnessId": HARNESS, "modelProviderId": "p", "modelId": MODEL, "presetId": "heavy-review"}, key="rt-s")
    sid = r["json"]["session"]["id"]
    s = hub.wait_status(sid, "active")
    t.check(s.get("appliedPreset") == "heavy-review", "the approve:true preset is applied", f"appliedPreset={s.get('appliedPreset')}")

    # 1. With the preset, a tool raises an approval (the preset gates).
    ap1, t1 = run_read_turn(hub, sid, "rt-t1")
    if not t.check(ap1 is not None, "with the preset, a tool raises an approval", "no approval; the preset did not gate"):
        wait_turn(hub, sid, t1, {"ended"}, tries=120)
    else:
        hub.post(f"/v1/sessions/{sid}/approvals/{ap1['id']}", {"decision": "allow"})
        t.check(wait_turn(hub, sid, t1, {"ended"}, tries=240).get("state") == "ended", "the allowed turn settles", "turn did not settle")

    # 2. Switch review OFF; a tool must now run WITHOUT asking.
    p = hub.patch(f"/v1/sessions/{sid}", {"review": False}, key="rt-off")
    t.check(p["status"] < 300, "review:false is accepted", f"status={p['status']} {p['text'][:120]}")
    ap2, t2 = run_read_turn(hub, sid, "rt-t2")
    if not t.check(ap2 is None, "review switched OFF: a tool runs without asking", f"an approval was raised anyway: {ap2}"):
        wait_turn(hub, sid, t2, {"ended"}, tries=120)
        hub.post(f"/v1/sessions/{sid}/approvals/{ap2['id']}", {"decision": "allow"})
    else:
        t.check(wait_turn(hub, sid, t2, {"ended"}, tries=240).get("state") == "ended", "the ungated turn settles", "turn did not settle")

    # 3. Switch review back ON; a tool must ask again (the fix did not freeze it off).
    p = hub.patch(f"/v1/sessions/{sid}", {"review": True}, key="rt-on")
    t.check(p["status"] < 300, "review:true is accepted", f"status={p['status']} {p['text'][:120]}")
    ap3, t3 = run_read_turn(hub, sid, "rt-t3")
    if not t.check(ap3 is not None, "review switched ON again: a tool asks again", "no approval; review did not come back on"):
        wait_turn(hub, sid, t3, {"ended"}, tries=120)
    else:
        hub.post(f"/v1/sessions/{sid}/approvals/{ap3['id']}", {"decision": "allow"})
        t.check(wait_turn(hub, sid, t3, {"ended"}, tries=240).get("state") == "ended", "the re-gated turn settles", "turn did not settle")
finally:
    ok = t.done()
    hub.cleanup()
sys.exit(0 if ok else 1)

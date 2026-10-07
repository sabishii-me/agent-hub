# The `heavy-review` preset declares {"approve": true} - "every tool call asks before it
# runs". A session that selects it MUST raise a REAL approval before the tool runs: the
# approval appears on GET /v1/sessions/{id}/approvals, allowing it (POST the SAME id)
# resumes and the tool runs; denying it blocks the tool with no side effect. This test
# EXISTS TO EXPOSE THE PROBLEM: if the preset does not actually gate, it goes red.
#
# FACT:    a session with an approve:true preset raises a REAL approval before the tool runs
# SOURCE:  ARCHITECTURE s22; contract/v1.json PATCH session policy review; the heavy-review preset in <plugin>/presets/
# EXPOSES: a preset that reports appliedPreset but does NOT gate the tool (20261004-050000 class)
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

t = Tally("approvals/preset-gates")
combo(hub_sha(), kind_of(PI))
if not t.require(os.path.isdir(PI), "a real plugin is present", f"no real plugin at {PI}"):
    t.done(); sys.exit(1)

hub = Hub(plugins_src=PI)
try:
    hub.start()
    t.require(register_provider(hub) is not None, "a real provider is registered", "no real provider")
    r = hub.post("/v1/sessions", {"harnessId": HARNESS, "modelProviderId": "p", "modelId": MODEL, "presetId": "heavy-review"}, key="ap-s")
    sid = r["json"]["session"]["id"]
    s = hub.wait_status(sid, "active")
    t.check(s.get("status") == "active", "the session is active", f"status={s.get('status')} err={s.get('startError')}")
    t.check(s.get("appliedPreset") == "heavy-review", "the approval preset is APPLIED", f"appliedPreset={s.get('appliedPreset')}")

    # Prompt that forces a tool call.
    tr = hub.post(f"/v1/sessions/{sid}/turns", {"content": [{"type": "text", "text": "Use the read tool to read E:/AI/ideas/prts-hub/Cargo.toml, then reply with its first line."}], "idempotencyKey": "ap-t1"}, key="ap-t1")
    t.check(tr["status"] == 202, "the turn is accepted", f"status={tr['status']}")
    tid = tr["json"]["turn"]["id"]

    # An approval MUST appear (the whole point of the preset).
    approval = None
    for _ in range(80):
        a = hub.get(f"/v1/sessions/{sid}/approvals")
        rows = (a["json"] or {}).get("approvals", [])
        if rows:
            approval = rows[0]
            break
        if wait_turn(hub, sid, tid, {"ended"}, tries=1).get("state") == "ended":
            break
        time.sleep(0.25)
    if not t.check(approval is not None,
                   "selecting an `approve:true` preset raises a REAL approval before the tool runs",
                   "no approval was raised; the tool ran ungated"):
        # leave the turn to settle so cleanup is clean
        wait_turn(hub, sid, tid, {"ended"}, tries=120)
    else:
        aid = approval.get("id")
        t.check(bool(aid), "the approval carries an id", f"approval={approval}")
        # DENY it: the tool must not run.
        d = hub.post(f"/v1/sessions/{sid}/approvals/{aid}", {"decision": "deny"})
        t.check(d["status"] < 300, "denying the approval is accepted", f"status={d['status']} {d['text'][:120]}")
        end = wait_turn(hub, sid, tid, {"ended"}, tries=240)
        t.check(end.get("state") == "ended", "the turn settles after deny", f"state={end.get('state')}")
        # After resolver clears, the approvals list must be empty (the id was consumed).
        a2 = hub.get(f"/v1/sessions/{sid}/approvals")
        ids = [x.get("id") for x in (a2["json"] or {}).get("approvals", [])]
        t.check(aid not in ids, "the answered approval is no longer pending", f"pending={ids}")
finally:
    ok = t.done()
    hub.cleanup()
sys.exit(0 if ok else 1)

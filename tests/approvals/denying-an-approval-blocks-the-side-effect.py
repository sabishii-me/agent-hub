# Denying an approval must PREVENT the tool's side effect: the model asks to write a file,
# the caller denies, and the file must NOT exist afterwards. Real provider, real preset,
# real approval round-trip. Exposes the case where a preset's gate does not really gate.
#
# FACT:    a DENIED approval prevents the tool's side effect
# SOURCE:  ARCHITECTURE s22; contract/v1.json POST /v1/sessions/{id}/approvals/{aid}
# EXPOSES: a gate that records the denial but lets the write happen anyway
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

t = Tally("approvals/deny-blocks")
combo(hub_sha(), kind_of(PI))
if not t.require(os.path.isdir(PI), "a real plugin is present", f"no real plugin at {PI}"):
    t.done(); sys.exit(1)

hub = Hub(plugins_src=PI)
try:
    hub.start()
    t.require(register_provider(hub) is not None, "a real provider is registered", "no real provider")
    r = hub.post("/v1/sessions", {"harnessId": HARNESS, "modelProviderId": "p", "modelId": MODEL, "presetId": "heavy-review"}, key="dn-s")
    sid = r["json"]["session"]["id"]
    s = hub.wait_status(sid, "active")
    cwd = s.get("cwd")
    t.require(cwd is not None, "the session reports a cwd (so the write target is a real path)",
              "no cwd on the session; cannot test a side effect without a target path")
    target = os.path.join(cwd, "must-not-exist.txt")
    if os.path.exists(target):
        t.check(False, "the target file does not pre-exist", f"target={target} already exists")

    tr = hub.post(f"/v1/sessions/{sid}/turns", {"content": [{"type": "text", "text": f"Use the write tool to create the file {target} containing the single word DENIED, then reply done."}], "idempotencyKey": "dn-t1"}, key="dn-t1")
    tid = tr["json"]["turn"]["id"]

    approval = None
    for _ in range(80):
        a = hub.get(f"/v1/sessions/{sid}/approvals")
        rows = (a["json"] or {}).get("approvals", [])
        if rows:
            approval = rows[0]; break
        if wait_turn(hub, sid, tid, {"ended"}, tries=1).get("state") == "ended":
            break
        time.sleep(0.25)
    t.check(approval is not None, "an approval is raised for the write", "no approval raised (the write preset did not gate)")
    if approval:
        aid = approval.get("id")
        dn = hub.post(f"/v1/sessions/{sid}/approvals/{aid}", {"decision": "deny"})
        # The deny must be ACCEPTED: an unchecked deny could fail and the file still not exist for
        # an unrelated reason.
        t.check(dn["status"] < 300, "the DENY decision is accepted", f"status={dn['status']} {dn['text'][:120]}")
        row = wait_turn(hub, sid, tid, {"ended"}, tries=240)
        # The turn must actually REACH a terminal; a turn stuck awaiting the approval must not pass.
        t.check(row.get("state") == "ended", "the turn reaches a terminal after the deny",
                f"state={row.get('state')}")
        t.check(not os.path.exists(target),
                "the DENIED write did NOT happen (no side effect)", f"target={target} exists={os.path.exists(target)}")
    else:
        row = wait_turn(hub, sid, tid, {"ended"}, tries=120)
        t.check(row.get("state") == "ended", "the turn settles even with no approval", f"state={row.get('state')}")
finally:
    ok = t.done()
    hub.cleanup()
sys.exit(0 if ok else 1)

# TC-INTERRUPT-017
# CONCURRENCY C7: an APPROVAL is pending (a real gated tool asked), and the adapter DIES
# before it is answered. The pending wait must NOT hang forever: the turn settles honestly
# and the session does not keep claiming active. Real: real adapter, real approval, real kill.
#
# FACT:    an approval pending when the adapter dies does not hang the turn; it settles and the session is honest
# SOURCE:  ARCHITECTURE s22 + s21 N2; contract/v1.json GET /v1/sessions/{id}/approvals
# EXPOSES: a turn stuck awaiting an approval whose adapter is gone (a hang)
# (A test that would pass whatever happens is not a test: this block names the fact it
#  proves and where that fact comes from; the assertions below are that exact fact.)
import os
import sys
import time

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "lib"))
from hub import Hub, hub_sha, register_provider, wait_turn, child_processes, kill_pid  # noqa: E402
from tally import Tally, combo, kind_of                          # noqa: E402

PI = os.environ.get("PI_PLUGIN_DIR", r"E:/AI/ideas/prts-harness-pi")
HARNESS = os.environ.get("PI_HARNESS_ID", "pi")
MODEL = os.environ.get("PI_MODEL", "deepseek-flash")
# The preset that gates EVERY tool, so a write raises an approval. Harness-specific: pi/jouzu ship
# `heavy-review`; a harness without an approval-gating preset cannot drive this case (it would fail
# honestly below, not skip).
APPROVAL_PRESET = os.environ.get("PI_APPROVAL_PRESET", "heavy-review")

t = Tally("interrupt/approval-pending-adapter-dies")
combo(hub_sha(), kind_of(PI))
if not t.require(os.path.isdir(PI), "a real plugin is present", f"no real plugin at {PI}"):
    t.done(); sys.exit(1)

hub = Hub(plugins_src=PI)
try:
    hub.start()
    PROV = register_provider(hub)
    t.require(PROV is not None, "a real provider is registered", "no real provider")
    # heavy-review gates every tool, so a write raises an approval.
    r = hub.post("/v1/sessions", {"harnessId": HARNESS, "modelProviderId": PROV["id"], "modelId": MODEL, "presetId": APPROVAL_PRESET}, key="c7-s")
    sid = r["json"]["session"]["id"]
    s = hub.wait_status(sid, "active")
    cwd = s.get("cwd")
    t.check(s.get("status") == "active", "the session is active", f"status={s.get('status')} err={s.get('startError')}")
    t.require(cwd is not None, "the session reports a cwd", "no cwd")

    tr = hub.post(f"/v1/sessions/{sid}/turns",
                  {"content": [{"type": "text", "text": f"Use the write tool to create the file {os.path.join(cwd,'c7.txt')} containing OK, then reply done."}], "idempotencyKey": "c7-t1"}, key="c7-t1")
    t.check(tr["status"] == 202, "the turn is accepted", f"status={tr['status']} {tr['text'][:120]}")
    tid = tr["json"]["turn"]["id"]

    # Wait for a REAL pending approval.
    approval = None
    for _ in range(120):
        rows = (hub.get(f"/v1/sessions/{sid}/approvals")["json"] or {}).get("approvals", [])
        if rows:
            approval = rows[0]; break
        if wait_turn(hub, sid, tid, {"ended"}, tries=1).get("state") == "ended":
            break
        time.sleep(0.25)
    if not t.require(approval is not None, f"a real approval is pending (the `{APPROVAL_PRESET}` preset gated the write)",
                     f"no approval raised by `{APPROVAL_PRESET}` (this harness has no approval-gating preset)"):
        t.done(); sys.exit(1)
    t.check(approval.get("id") is not None, "the approval has an id", f"approval={approval}")

    # KILL the adapter while the approval is pending, and do NOT answer it.
    kids = [pid for pid, name in child_processes(hub.child.pid) if "node" in (name or "").lower()]
    t.check(len(kids) >= 1, "a real adapter child is found", f"children={child_processes(hub.child.pid)}")
    for pid in kids:
        kill_pid(pid)

    # The turn must NOT hang: it reaches a terminal (bounded).
    row = wait_turn(hub, sid, tid, {"ended"}, tries=400)
    t.check(row.get("state") == "ended",
            "the turn settles after the adapter died with an approval pending (no hang)",
            f"state={row.get('state')} ended={row.get('ended')}")
    t.check(row.get("ended") != "completed",
            "a turn whose adapter died mid-approval is NOT a clean completed",
            f"ended={row.get('ended')}")

    # The session is honest: no active claim with no process.
    st = "active"
    for _ in range(160):
        st = (hub.get(f"/v1/sessions/{sid}")["json"] or {}).get("session", {}).get("status")
        if st != "active":
            break
        time.sleep(0.25)
    t.check(st != "active", "the session no longer claims active after its adapter died", f"status={st}")
    t.check(hub.child.poll() is None, "the HUB survives the adapter death")
finally:
    ok = t.done()
    hub.cleanup()
sys.exit(0 if ok else 1)

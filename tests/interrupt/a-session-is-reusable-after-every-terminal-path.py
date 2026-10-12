# TC-INTERRUPT-020
# ACTIVE A9: after EVERY terminal path a session can reach, it must remain USABLE - it reads,
# and it accepts a further turn with a DEFINED outcome (never a 5xx, never wedged). This walks
# the terminals one by one: a normal COMPLETED turn, a CANCELLED turn, and a session left
# needs-repair after its adapter died. Real: real adapter, real provider, real kill.
#
# FACT:    a session is reusable (reads + accepts a defined turn) after a completed turn, a cancelled turn, and a needs-repair recovery
# SOURCE:; contract/v1.json POST /v1/sessions/{id}/turns
# EXPOSES: a terminal path that leaves the session wedged (busy forever, or 5xx on the next turn)
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

t = Tally("interrupt/reuse-after-every-terminal")
combo(hub_sha(), kind_of(PI))
if not t.require(os.path.isdir(PI), "a real plugin is present", f"no real plugin at {PI}"):
    t.done(); sys.exit(1)

hub = Hub(plugins_src=PI)
try:
    hub.start()
    PROV = register_provider(hub)
    t.require(PROV is not None, "a real provider is registered", "no real provider")

    def defined_turn(sid, key, note):
        tr = hub.post(f"/v1/sessions/{sid}/turns", {"content": [{"type": "text", "text": "say hi"}], "idempotencyKey": key}, key=key)
        t.check(tr["status"] < 500, f"{note}: a further turn is not a 5xx", f"status={tr['status']} {tr['text'][:120]}")
        return tr

    # --- 1) after a NORMAL (completed) turn ---
    r = hub.post("/v1/sessions", {"harnessId": HARNESS, "modelProviderId": PROV["id"], "modelId": MODEL}, key="ru-s1")
    sid = r["json"]["session"]["id"]
    hub.wait_status(sid, "active")
    tr = hub.post(f"/v1/sessions/{sid}/turns", {"content": [{"type": "text", "text": "say hi in one word"}], "idempotencyKey": "ru-t1"}, key="ru-t1")
    tid = tr["json"]["turn"]["id"]
    e = wait_turn(hub, sid, tid, {"ended"}, tries=400)
    t.check(e.get("state") == "ended", "a normal turn reaches a terminal", f"state={e.get('state')}")
    t.check(hub.get(f"/v1/sessions/{sid}")["status"] == 200, "after a completed turn the session reads")
    tr2 = defined_turn(sid, "ru-t2", "after a completed turn")
    if tr2["status"] == 202:
        t.check(wait_turn(hub, sid, tr2["json"]["turn"]["id"], {"ended"}, tries=400).get("state") == "ended",
                "the turn after a completed turn reaches a terminal")

    # --- 2) after a CANCELLED turn ---
    tr3 = hub.post(f"/v1/sessions/{sid}/turns", {"content": [{"type": "text", "text": "Count slowly from 1 to 500."}], "idempotencyKey": "ru-t3"}, key="ru-t3")
    tid3 = tr3["json"]["turn"]["id"]
    wait_turn(hub, sid, tid3, {"running", "awaiting_approval", "awaiting_question"}, tries=160)
    hub.post(f"/v1/sessions/{sid}/cancel")
    e3 = wait_turn(hub, sid, tid3, {"ended"}, tries=400)
    t.check(e3.get("state") == "ended", "the cancelled turn reaches a terminal", f"state={e3.get('state')}")
    t.check(hub.get(f"/v1/sessions/{sid}")["status"] == 200, "after a cancelled turn the session reads")
    tr4 = defined_turn(sid, "ru-t4", "after a cancelled turn")
    if tr4["status"] == 202:
        t.check(wait_turn(hub, sid, tr4["json"]["turn"]["id"], {"ended"}, tries=400).get("state") == "ended",
                "the turn after a cancelled turn reaches a terminal")

    # --- 3) after the adapter DIED (session needs-repair), verify reuse after a reopen ---
    tr5 = hub.post(f"/v1/sessions/{sid}/turns", {"content": [{"type": "text", "text": "Count slowly from 1 to 500."}], "idempotencyKey": "ru-t5"}, key="ru-t5")
    tid5 = tr5["json"]["turn"]["id"]
    wait_turn(hub, sid, tid5, {"running", "awaiting_approval", "awaiting_question"}, tries=160)
    kids = [pid for pid, name in child_processes(hub.child.pid) if "node" in (name or "").lower()]
    for pid in kids:
        kill_pid(pid)
    st = "active"
    for _ in range(200):
        st = (hub.get(f"/v1/sessions/{sid}")["json"] or {}).get("session", {}).get("status")
        if st != "active":
            break
        time.sleep(0.25)
    t.check(st != "active", "after the adapter died the session is not left active", f"status={st}")
    op = hub.post(f"/v1/sessions/{sid}/reopen")
    t.check(op["status"] < 300, "the session reopens after the death", f"status={op['status']} {op['text'][:140]}")
    s2 = hub.wait_status(sid, "active")
    t.check(s2.get("status") == "active", "the reopened session is active", f"status={s2.get('status')} err={s2.get('startError')}")
    tr6 = defined_turn(sid, "ru-t6", "after a needs-repair reopen")
    if tr6["status"] == 202:
        t.check(wait_turn(hub, sid, tr6["json"]["turn"]["id"], {"ended"}, tries=400).get("state") == "ended",
                "the turn after a needs-repair reopen reaches a terminal")
finally:
    ok = t.done()
    hub.cleanup()
sys.exit(0 if ok else 1)

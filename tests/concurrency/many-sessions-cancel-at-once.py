# C1: several sessions run turns at once, then ALL are cancelled together. Every turn
# must reach a terminal state; none may wedge another; every session must stay usable.
# The stop of one adapter must not block the settle of another. No fake: N real turns on
# N real sessions, N real aborts.
import os
import sys
import time

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "lib"))
from hub import Hub, hub_sha, register_provider, wait_turn      # noqa: E402
from tally import Tally, combo, kind_of                          # noqa: E402

PI = os.environ.get("PI_PLUGIN_DIR", r"E:/AI/ideas/prts-harness-pi")
HARNESS = os.environ.get("PI_HARNESS_ID", "pi")
MODEL = os.environ.get("PI_MODEL", "deepseek-flash")
N = int(os.environ.get("CONCURRENT_SESSIONS", "4"))

t = Tally("concurrency/many-cancel")
combo(hub_sha(), kind_of(PI))
if not t.require(os.path.isdir(PI), "a real plugin is present", f"no real plugin at {PI}"):
    t.done(); sys.exit(1)

hub = Hub(plugins_src=PI)
try:
    hub.start()
    t.require(register_provider(hub) is not None, "a real provider is registered", "no real provider")

    sids, tids = [], []
    for i in range(N):
        r = hub.post("/v1/sessions", {"harnessId": HARNESS, "modelProviderId": "p", "modelId": MODEL}, key=f"c1-s{i}")
        sid = r["json"]["session"]["id"]; sids.append(sid)
        t.check(hub.wait_status(sid, "active").get("status") == "active", f"session {i} is active")
        tr = hub.post(f"/v1/sessions/{sid}/turns", {"content": [{"type": "text", "text": "Count slowly from 1 to 400."}], "idempotencyKey": f"c1-t{i}"}, key=f"c1-t{i}")
        t.check(tr["status"] == 202, f"turn {i} is accepted", f"status={tr['status']}")
        tids.append(tr["json"]["turn"]["id"])

    # Wait until they are running (bounded; require at least one observed running).
    started = 0
    for sid, tid in zip(sids, tids):
        row = wait_turn(hub, sid, tid, {"running", "awaiting_approval", "awaiting_question", "ended"}, tries=160)
        if row.get("state") in ("running", "awaiting_approval", "awaiting_question"):
            started += 1
    t.check(started >= 1, f"at least one of {N} turns observed running", f"started={started}")

    # Cancel ALL at once (no waiting between).
    for sid in sids:
        hub.post(f"/v1/sessions/{sid}/cancel")

    # EVERY turn settles to a terminal. Assert per turn, with a bound.
    settled = 0
    for i, (sid, tid) in enumerate(zip(sids, tids)):
        row = wait_turn(hub, sid, tid, {"ended"}, tries=320)
        ok = row.get("state") == "ended"
        if ok:
            settled += 1
        t.check(ok, f"turn {i} settled after the concurrent cancels", f"state={row.get('state')} ended={row.get('ended')}")
    t.check(settled == N, f"all {N} turns settled (none wedged)", f"settled={settled}/{N}")

    # Every session is still readable (none left in a broken state).
    readable = sum(1 for sid in sids if hub.get(f"/v1/sessions/{sid}")["status"] == 200)
    t.check(readable == N, f"all {N} sessions still read", f"readable={readable}/{N}")
finally:
    ok = t.done()
    hub.cleanup()
sys.exit(0 if ok else 1)

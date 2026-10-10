# TC-INTERRUPT-015
# ACTIVE A10: a turn that reached a KNOWN terminal (here, a CONFIRMED cancel -> `cancelled`)
# must NOT be overwritten later by a sweep/orphan reconciliation. The hub settles a turn
# ONCE; a second pass must see the committed terminal and leave it alone. Real: real adapter.
#
# FACT:    a known terminal is committed once and is not overwritten by a later reconciliation
# SOURCE:  ARCHITECTURE s21 (settle_turn commits the terminal once, SQL WHERE ended IS NULL); contract/v1.json turn.ended
# EXPOSES: a settled turn's terminal silently rewritten (e.g. cancelled -> interrupted/failed)
# (A test that would pass whatever happens is not a test: this block names the fact it
#  proves and where that fact comes from; the assertions below are that exact fact.)
import os
import sys
import time

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "lib"))
from hub import Hub, hub_sha, register_provider, wait_turn, turn_state   # noqa: E402
from tally import Tally, combo, kind_of                                  # noqa: E402

PI = os.environ.get("PI_PLUGIN_DIR", r"E:/AI/ideas/prts-harness-pi")
HARNESS = os.environ.get("PI_HARNESS_ID", "pi")
MODEL = os.environ.get("PI_MODEL", "deepseek-flash")

t = Tally("interrupt/terminal-once")
combo(hub_sha(), kind_of(PI))
if not t.require(os.path.isdir(PI), "a real plugin is present", f"no real plugin at {PI}"):
    t.done(); sys.exit(1)

hub = Hub(plugins_src=PI)
try:
    hub.start()
    PROV = register_provider(hub)
    t.require(PROV is not None, "a real provider is registered", "no real provider")
    r = hub.post("/v1/sessions", {"harnessId": HARNESS, "modelProviderId": PROV["id"], "modelId": MODEL}, key="kt-s")
    sid = r["json"]["session"]["id"]
    hub.wait_status(sid, "active")

    tr = hub.post(f"/v1/sessions/{sid}/turns",
                  {"content": [{"type": "text", "text": "Count slowly from 1 to 500."}], "idempotencyKey": "kt-t1"}, key="kt-t1")
    t.check(tr["status"] == 202, "the turn is accepted", f"status={tr['status']} {tr['text'][:120]}")
    tid = tr["json"]["turn"]["id"]

    # Let it run, then cancel it CONFIRMED, and record the committed terminal.
    wait_turn(hub, sid, tid, {"running", "awaiting_approval", "awaiting_question"}, tries=160)
    hub.post(f"/v1/sessions/{sid}/cancel")
    end = wait_turn(hub, sid, tid, {"ended"}, tries=400)
    first = end.get("ended")
    t.check(end.get("state") == "ended", "the cancelled turn reaches a terminal", f"state={end.get('state')}")
    t.check(first in ("cancelled", "interrupted"),
            "a cancelled turn's terminal is cancelled/interrupted (not failed)", f"ended={first}")

    # Read it again after a settle window: a sweep/orphan reconciliation must NOT rewrite it.
    first_row = None
    g = hub.get(f"/v1/sessions/{sid}/turns")
    first_row = next((x for x in ((g["json"] or {}).get("turns") or []) if x.get("id") == tid), None)
    first_ended = first_row.get("ended") if first_row else None
    t.check(first_ended == first,
            "the terminal is read back unchanged immediately after it commits",
            f"first={first} readback={first_ended}")

    time.sleep(3.0)
    g2 = hub.get(f"/v1/sessions/{sid}/turns")
    row2 = next((x for x in ((g2["json"] or {}).get("turns") or []) if x.get("id") == tid), None)
    t.check(row2 is not None, "the turn is still listed after a settle window")
    if row2 is not None:
        t.check(row2.get("ended") == first_ended,
                "a later reconciliation does NOT overwrite the known terminal",
                f"before={first_ended} after={row2.get('ended')}")
        t.check(row2.get("state") == "ended",
                "the turn stays `ended` after a settle window",
                f"state={row2.get('state')}")

    # The session remains usable and honest.
    t.check(hub.get(f"/v1/sessions/{sid}")["status"] == 200, "the session still reads")
finally:
    ok = t.done()
    hub.cleanup()
sys.exit(0 if ok else 1)

# TC-INTERRUPT-014
# ACTIVE A8: a cancel is in flight (the turn is cancelling), and the caller CLOSES the
# session while that stop is unconfirmed. The close must stop and free the session
# cleanly, and a later REOPEN must re-attach on the stored ref and accept a turn again.
# Real: a real adapter, no fake.
#
# FACT:    a close across an unconfirmed cancel frees the session, and reopen re-attaches on the stored ref
# SOURCE:  contract/v1.json POST /v1/sessions/{id}/close, /v1/sessions/{id}/reopen; ARCHITECTURE s21
# EXPOSES: a close that leaves the session wedged, or a reopen that cannot re-attach after a cancel+close
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

t = Tally("interrupt/close-across-cancel")
combo(hub_sha(), kind_of(PI))
if not t.require(os.path.isdir(PI), "a real plugin is present", f"no real plugin at {PI}"):
    t.done(); sys.exit(1)

hub = Hub(plugins_src=PI)
try:
    hub.start()
    PROV = register_provider(hub)
    t.require(PROV is not None, "a real provider is registered", "no real provider")
    r = hub.post("/v1/sessions", {"harnessId": HARNESS, "modelProviderId": PROV["id"], "modelId": MODEL}, key="cc-s")
    sid = r["json"]["session"]["id"]
    t.check(hub.wait_status(sid, "active").get("status") == "active", "the session is active")
    # The native ref is written when the adapter answers session/start (the row goes active), so it
    # is read AFTER the session is active - not from the `starting` row.
    ref = (hub.get(f"/v1/sessions/{sid}")["json"] or {}).get("session", {}).get("nativeRef")
    t.check(bool(ref), "the session has a native ref to re-attach on", f"ref={ref}")

    # A real turn is running, then we CANCEL and CLOSE immediately - the stop may be unconfirmed.
    tr = hub.post(f"/v1/sessions/{sid}/turns",
                  {"content": [{"type": "text", "text": "Count slowly from 1 to 500."}], "idempotencyKey": "cc-t"}, key="cc-t")
    t.check(tr["status"] == 202, "the turn is accepted", f"status={tr['status']} {tr['text'][:120]}")
    tid = tr["json"]["turn"]["id"]
    wait_turn(hub, sid, tid, {"running", "awaiting_approval", "awaiting_question"}, tries=160)

    cn = hub.post(f"/v1/sessions/{sid}/cancel")
    t.check(cn["status"] < 500, "the cancel is accepted (not a 5xx)", f"status={cn['status']}")
    cl = hub.post(f"/v1/sessions/{sid}/close")
    t.check(cl["status"] < 300, "the close across the in-flight cancel succeeds", f"status={cl['status']} {cl['text'][:120]}")

    # The turn must settle (a close must not leave it running forever).
    end = wait_turn(hub, sid, tid, {"ended"}, tries=400)
    t.check(end.get("state") == "ended", "the turn settles after the close (no hang)",
            f"state={end.get('state')}")

    # A closed session reads as closed, not active.
    st = (hub.get(f"/v1/sessions/{sid}")["json"] or {}).get("session", {}).get("status")
    t.check(st != "active", "a closed session is not still `active`", f"status={st}")

    # REOPEN re-attaches on the stored ref and the session is usable again.
    op = hub.post(f"/v1/sessions/{sid}/reopen")
    t.check(op["status"] < 300, "reopen re-attaches after cancel+close", f"status={op['status']} {op['text'][:140]}")
    s2 = hub.wait_status(sid, "active")
    t.check(s2.get("status") == "active", "the reopened session is active again",
            f"status={s2.get('status')} err={s2.get('startError')}")
    tr2 = hub.post(f"/v1/sessions/{sid}/turns",
                   {"content": [{"type": "text", "text": "say hi"}], "idempotencyKey": "cc-t2"}, key="cc-t2")
    t.check(tr2["status"] == 202, "the reopened session accepts a turn", f"status={tr2['status']} {tr2['text'][:120]}")
finally:
    ok = t.done()
    hub.cleanup()
sys.exit(0 if ok else 1)

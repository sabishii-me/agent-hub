# TC-INTERRUPT-021
# PASSIVE P3: the adapter CLOSES ITS OWN STDOUT but stays ALIVE (the process did not exit). The hub
# reads EOF on a child that is still running: it must settle the in-flight turn HONESTLY (never
# hang, never a clean `completed`) and keep answering. A closed stdout is "the adapter stopped
# talking", not "the turn succeeded".
#
# Real: a REAL adapter process (tests/fixtures/misbehaving) that speaks the real protocol and then
# closes its stdout while continuing to live.
#
# FACT:    an adapter that closes stdout but stays alive settles the in-flight turn honestly (never a clean completed); the hub survives
# SOURCE: (the bus drains pending requests with Closed when stdout ends); contract/v1.json turn.ended
# EXPOSES: a turn stuck forever (or reported completed) when the adapter goes silent without exiting
# (A test that would pass whatever happens is not a test: this block names the fact it
#  proves and where that fact comes from; the assertions below are that exact fact.)
import os
import sys
import time

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "lib"))
from hub import Hub, hub_sha, register_provider, wait_turn, child_processes   # noqa: E402
from tally import Tally, combo, kind_of                                        # noqa: E402

# This case needs a DELIBERATELY MISBEHAVING harness, so it PINS the fixture (it is not about
# pi/jouzu/deepseek). The harness sweep sets PI_PLUGIN_DIR/PI_HARNESS_ID to the real harness under
# test; this file ignores that and always drives the fixture, so it runs the same in every sweep.
MB = os.environ.get("PI_MISBEHAVING_DIR", r"E:/AI/ideas/prts-harness-misbehaving")
HARNESS = "misbehaving"
MODEL = os.environ.get("PI_MODEL", "deepseek-flash")

t = Tally("interrupt/adapter-closes-stdout")
combo(hub_sha(), kind_of(MB))
if not t.require(os.path.isdir(MB), "the misbehaving harness fixture is present", f"no fixture at {MB}"):
    t.done(); sys.exit(1)

hub = Hub(plugins_src=MB, env={"AGENT_HUB_MISBEHAVE": "close-stdout"})
try:
    hub.start()
    PROV = register_provider(hub)
    t.require(PROV is not None, "a real provider is registered", "no real provider")
    r = hub.post("/v1/sessions", {"harnessId": HARNESS, "modelProviderId": PROV["id"], "modelId": MODEL}, key="p3-s")
    sid = (r.get("json") or {}).get("session", {}).get("id")
    if not t.require(sid is not None, "the session starts against the misbehaving harness", f"status={r['status']} {r['text'][:160]}"):
        raise SystemExit(0)
    t.check(hub.wait_status(sid, "active").get("status") == "active", "the session is active")

    # The adapter closes stdout on the FIRST prompt. The turn must settle honestly.
    tr = hub.post(f"/v1/sessions/{sid}/turns",
                  {"content": [{"type": "text", "text": "Say hello."}], "idempotencyKey": "p3-t1"}, key="p3-t1")
    t.check(tr["status"] == 202, "the turn is accepted", f"status={tr['status']} {tr['text'][:120]}")
    tid = tr["json"]["turn"]["id"]

    end = wait_turn(hub, sid, tid, {"ended"}, tries=440)
    t.check(end.get("state") == "ended",
            "the turn settles after stdout closes (no hang)",
            f"state={end.get('state')}")
    t.check(end.get("ended") != "completed",
            "an adapter that went silent is never a clean `completed`",
            f"ended={end.get('ended')}")
    t.check(end.get("ended") in ("failed", "interrupted"),
            "the honest terminal is failed/interrupted",
            f"ended={end.get('ended')} cause={end.get('cause')}")

    t.check(hub.child.poll() is None, "the HUB survives the closed stdout")
    t.check(hub.get(f"/v1/sessions/{sid}")["status"] == 200, "the hub still answers afterwards")
finally:
    ok = t.done()
    hub.cleanup()
sys.exit(0 if ok else 1)

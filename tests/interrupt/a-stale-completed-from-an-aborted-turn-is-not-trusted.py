# TC-INTERRUPT-020
# ACTIVE A4: a cancel arrives WHILE the prompt is running, and the harness answers with a STALE
# terminal - a `turn_end` claiming `state: ok` (completed) FOR THE TURN THAT WAS JUST ABORTED. The
# hub must not trust a lie from a harness it asked to stop: the turn must not be reported as a clean
# `completed`, and the hub must stay honest and alive.
#
# Real: a REAL adapter process (tests/fixtures/misbehaving) that speaks the real protocol but
# deliberately reports an aborted turn as `ok`.
#
# FACT:    an aborted turn's stale `completed` is not trusted; the turn is not a clean completed
# SOURCE: (a cancel's terminal is the harness's to prove, but a completed after an abort is not a proof of success); contract/adapter-v1.json (turn_end.state)
# EXPOSES: a stale completed snapshot from a harness being believed after a cancel
# (A test that would pass whatever happens is not a test: this block names the fact it
#  proves and where that fact comes from; the assertions below are that exact fact.)
import os
import sys
import time

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "lib"))
from hub import Hub, hub_sha, register_provider, wait_turn            # noqa: E402
from tally import Tally, combo, kind_of                                # noqa: E402

# This case needs a DELIBERATELY MISBEHAVING harness, so it PINS the fixture (it is not about
# pi/jouzu/deepseek). The harness sweep sets PI_PLUGIN_DIR/PI_HARNESS_ID to the real harness under
# test; this file ignores that and always drives the fixture, so it runs the same in every sweep.
MB = os.environ.get("PI_MISBEHAVING_DIR", r"E:/AI/ideas/prts-harness-misbehaving")
HARNESS = "misbehaving"
MODEL = os.environ.get("PI_MODEL", "deepseek-flash")

t = Tally("interrupt/stale-completed-after-abort")
combo(hub_sha(), kind_of(MB))
if not t.require(os.path.isdir(MB), "the misbehaving harness fixture is present", f"no fixture at {MB}"):
    t.done(); sys.exit(1)

hub = Hub(plugins_src=MB, env={"AGENT_HUB_MISBEHAVE": "abort-stale-completed"})
try:
    hub.start()
    PROV = register_provider(hub)
    t.require(PROV is not None, "a real provider is registered", "no real provider")
    r = hub.post("/v1/sessions", {"harnessId": HARNESS, "modelProviderId": PROV["id"], "modelId": MODEL}, key="a4-s")
    sid = (r.get("json") or {}).get("session", {}).get("id")
    if not t.require(sid is not None, "the session starts against the misbehaving harness", f"status={r['status']} {r['text'][:160]}"):
        raise SystemExit(0)
    t.check(hub.wait_status(sid, "active").get("status") == "active", "the session is active")

    tr = hub.post(f"/v1/sessions/{sid}/turns",
                  {"content": [{"type": "text", "text": "Say hello."}], "idempotencyKey": "a4-t1"}, key="a4-t1")
    t.check(tr["status"] == 202, "the turn is accepted", f"status={tr['status']} {tr['text'][:120]}")
    tid = tr["json"]["turn"]["id"]
    run = wait_turn(hub, sid, tid, {"running", "awaiting_approval", "awaiting_question"}, tries=160)
    t.check(run.get("state") in ("running", "awaiting_approval", "awaiting_question"),
            "the turn is running before the abort", f"state={run.get('state')}")

    # Abort while the prompt runs. The harness will answer with a STALE `turn_end` claiming ok.
    cn = hub.post(f"/v1/sessions/{sid}/cancel")
    t.check(cn["status"] < 500, "the cancel is accepted (not a 5xx)", f"status={cn['status']}")

    # The turn must reach a terminal (it does - the harness sends one), and that terminal must NOT
    # be a clean `completed` for a turn the hub just asked to abort.
    end = wait_turn(hub, sid, tid, {"ended"}, tries=240)
    t.check(end.get("state") == "ended", "the turn reaches a terminal", f"state={end.get('state')}")
    t.check(end.get("ended") != "completed",
            "an aborted turn's stale `completed` is NOT trusted (the turn is not a clean completed)",
            f"ended={end.get('ended')} cause={end.get('cause')}")

    t.check(hub.child.poll() is None, "the HUB survives the stale terminal")
    t.check(hub.get(f"/v1/sessions/{sid}")["status"] == 200, "the hub still answers afterwards")
finally:
    ok = t.done()
    hub.cleanup()
sys.exit(0 if ok else 1)

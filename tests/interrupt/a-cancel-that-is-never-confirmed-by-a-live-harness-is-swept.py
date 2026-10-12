# TC-INTERRUPT-019
# ACTIVE A2: a cancel is DELIVERED to a LIVE harness that ACKNOWLEDGES it but NEVER CONFIRMS the
# stop - a "half-dead" harness (the process is alive; it just refuses to emit the turn's terminal).
# The hub's cancel timeout must settle the turn honestly (`interrupted`) even though a process is
# still there, and it must never report a clean `completed`.
#
# Real: a REAL adapter process (tests/fixtures/misbehaving) that speaks the real protocol but
# deliberately never confirms an abort; the sweep is the hub's own, on its real tick.
#
# FACT:    a cancel acknowledged but never confirmed by a LIVE harness is swept to `interrupted`, never `completed`
# SOURCE:  ARCHITECTURE s21 (settle_unconfirmed_cancels, TASK-048 F4); contract/adapter-v1.json invariants (the abort ack is not a stop proof)
# EXPOSES: a turn stuck `cancelling` forever when the harness answers the abort but never stops
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

t = Tally("interrupt/unconfirmed-cancel-live-harness")
combo(hub_sha(), kind_of(MB))
if not t.require(os.path.isdir(MB), "the misbehaving harness fixture is present", f"no fixture at {MB}"):
    t.done(); sys.exit(1)

# The violation under test: the adapter acknowledges the abort and never sends the turn's terminal.
hub = Hub(plugins_src=MB, env={"AGENT_HUB_MISBEHAVE": "abort-silent"})
try:
    hub.start()
    PROV = register_provider(hub)
    t.require(PROV is not None, "a real provider is registered", "no real provider")
    r = hub.post("/v1/sessions", {"harnessId": HARNESS, "modelProviderId": PROV["id"], "modelId": MODEL}, key="a2-s")
    sid = (r.get("json") or {}).get("session", {}).get("id")
    if not t.require(sid is not None, "the session starts against the misbehaving harness", f"status={r['status']} {r['text'][:160]}"):
        raise SystemExit(0)
    t.check(hub.wait_status(sid, "active").get("status") == "active", "the session is active")

    tr = hub.post(f"/v1/sessions/{sid}/turns",
                  {"content": [{"type": "text", "text": "Say hello."}], "idempotencyKey": "a2-t1"}, key="a2-t1")
    t.check(tr["status"] == 202, "the turn is accepted", f"status={tr['status']} {tr['text'][:120]}")
    tid = tr["json"]["turn"]["id"]
    run = wait_turn(hub, sid, tid, {"running", "awaiting_approval", "awaiting_question"}, tries=160)
    t.check(run.get("state") in ("running", "awaiting_approval", "awaiting_question"),
            "the turn is running against the misbehaving harness", f"state={run.get('state')}")

    # Cancel: the adapter ACKs the abort but NEVER confirms. The process stays alive.
    cn = hub.post(f"/v1/sessions/{sid}/cancel")
    t.check(cn["status"] < 500, "the cancel is accepted (not a 5xx)", f"status={cn['status']}")

    # The harness is still ALIVE - this is the whole point (unlike A5, the process did not die).
    kids = [pid for pid, name in child_processes(hub.child.pid) if "node" in (name or "").lower()]
    t.check(len(kids) >= 1, "the misbehaving adapter is still a live child (a half-dead harness)",
            f"children={child_processes(hub.child.pid)}")

    # The hub's cancel timeout (5s tick) must settle it, even though nothing died.
    end = wait_turn(hub, sid, tid, {"ended"}, tries=440)
    t.check(end.get("state") == "ended",
            "the unconfirmed cancel on a LIVE harness is swept to a terminal (no forever-cancelling)",
            f"state={end.get('state')}")
    t.check(end.get("ended") == "interrupted",
            "a cancel the harness never confirmed settles `interrupted`",
            f"ended={end.get('ended')} cause={end.get('cause')} (required: interrupted)")
    t.check(end.get("ended") != "completed",
            "a turn whose harness never confirmed the stop is never a clean `completed`",
            f"ended={end.get('ended')}")

    # The hub is alive and the session is not left holding the turn.
    t.check(hub.child.poll() is None, "the HUB survives the unconfirmed cancel")
    t.check(hub.get(f"/v1/sessions/{sid}")["status"] == 200, "the hub still answers afterwards")
finally:
    ok = t.done()
    hub.cleanup()
sys.exit(0 if ok else 1)

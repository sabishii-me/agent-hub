# TC-INTERRUPT-009
# PASSIVE M1: the pi RUNTIME process (a DESCENDANT of the hub, under the adapter) is killed
# while a real turn runs, but the ADAPTER process stays alive. The turn must settle
# honestly (the model call can no longer complete), the session must stay usable, and the
# hub must not be left thinking a turn is running. Real fault injection: only a real
# process is killed; nothing is faked.
#
# FACT:    killing the RUNTIME process (adapter alive) settles the turn and never reports a clean completed
# SOURCE:  ARCHITECTURE s21 N2; contract/v1.json turn.ended
# EXPOSES: a killed runtime reported as a clean completion
# (A test that would pass whatever happens is not a test: this block names the fact it
#  proves and where that fact comes from; the assertions below are that exact fact.)
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "lib"))
from hub import Hub, hub_sha, register_provider, wait_turn, descendants_with_command, kill_pid  # noqa: E402
from tally import Tally, combo, kind_of                          # noqa: E402

PI = os.environ.get("PI_PLUGIN_DIR", r"E:/AI/ideas/prts-harness-pi")
HARNESS = os.environ.get("PI_HARNESS_ID", "pi")
MODEL = os.environ.get("PI_MODEL", "deepseek-flash")

t = Tally("interrupt/kill-runtime")
combo(hub_sha(), kind_of(PI))
if not t.require(os.path.isdir(PI), "a real plugin is present", f"no real plugin at {PI}"):
    t.done(); sys.exit(1)

hub = Hub(plugins_src=PI)
try:
    hub.start()
    PROV = register_provider(hub)
    t.require(PROV is not None, "a real provider is registered", "no real provider")
    r = hub.post("/v1/sessions", {"harnessId": HARNESS, "modelProviderId": PROV["id"], "modelId": MODEL}, key="kr-s")
    sid = r["json"]["session"]["id"]
    t.check(hub.wait_status(sid, "active").get("status") == "active", "the session is active")

    # The runtime process already exists once the session is active. Capture it NOW, before the turn:
    # enumerating descendants costs ~1.5s (PowerShell CIM), longer than a fast harness's whole turn,
    # so doing it after the prompt would let the turn finish before the kill landed (measured on dsh).
    # The runtime is the node descendant that is NOT the adapter: pi runs its runtime as a node child,
    # dsh runs its server (`bin.js web`) as a node child.
    desc = descendants_with_command(hub.child.pid)
    nodes = [(pid, cmd) for pid, nm, cmd in desc if "node" in (nm or "").lower()]
    runtimes = [(pid, cmd) for pid, cmd in nodes if "adapter.cjs" not in (cmd or "")]
    t.check(len(nodes) >= 1, "a real descendant node process is found", f"descendants={desc}")
    t.check(len(runtimes) >= 1, "the harness RUNTIME (a node descendant that is not the adapter) is found",
            f"nodes={nodes}")

    tr = hub.post(f"/v1/sessions/{sid}/turns", {"content": [{"type": "text", "text": "Count slowly from 1 to 500, one number per line, and take your time."}], "idempotencyKey": "kr-t"}, key="kr-t")
    t.check(tr["status"] == 202, "the turn is accepted", f"status={tr['status']}")
    tid = tr["json"]["turn"]["id"]
    row = wait_turn(hub, sid, tid, {"running", "awaiting_approval", "awaiting_question"}, tries=160)
    t.check(row.get("state") in ("running", "awaiting_approval", "awaiting_question"),
            "the turn is running", f"state={row.get('state')}")

    # Kill the RUNTIME process - the harness - the instant the turn is running, leaving the ADAPTER
    # alive (it is the process the hub owns and must survive).
    if runtimes:
        kill_pid(runtimes[-1][0])
        kill_pid(runtimes[-1][0])

    end = wait_turn(hub, sid, tid, {"ended"}, tries=240)
    t.check(end.get("state") == "ended", "the turn settles after the runtime died (no hang)",
            f"state={end.get('state')}")
    t.check(end.get("ended") != "completed", "a killed runtime is never a clean `completed`",
            f"ended={end.get('ended')}")
    t.check(hub.child.poll() is None, "the HUB survives the runtime death")
    t.check(hub.get(f"/v1/sessions/{sid}")["status"] == 200, "the session still reads")
finally:
    ok = t.done()
    hub.cleanup()
sys.exit(0 if ok else 1)

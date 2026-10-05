# PASSIVE P7: the NETWORK drops MID-TURN. The provider is reachable through a REAL TCP
# forwarder (a real network hop, not a fake provider); the turn runs; then the forwarder
# is KILLED, so the in-flight connection dies - a REAL network failure. The turn must
# settle HONESTLY (not hang, never `completed`), and the hub must keep answering.
#
# FACT:    a network drop mid-turn settles the turn (failed), never a clean completed; the hub survives
# SOURCE:  ARCHITECTURE s21 N2; contract/v1.json turn.ended
# EXPOSES: a dropped network reported as a completed turn, or a frozen hub
# (A test that would pass whatever happens is not a test: this block names the fact it
#  proves and where that fact comes from; the assertions below are that exact fact.)
import os
import sys
import time

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "lib"))
from hub import Hub, hub_sha, real_provider, wait_turn           # noqa: E402
from tcpfwd import Forwarder                                      # noqa: E402
from tally import Tally, combo, kind_of                          # noqa: E402

PI = os.environ.get("PI_PLUGIN_DIR", r"E:/AI/ideas/prts-harness-pi")
HARNESS = os.environ.get("PI_HARNESS_ID", "pi")
MODEL = os.environ.get("PI_MODEL", "deepseek-flash")

t = Tally("interrupt/network-drop")
combo(hub_sha(), kind_of(PI))
if not t.require(os.path.isdir(PI), "a real plugin is present", f"no real plugin at {PI}"):
    t.done(); sys.exit(1)

prov = real_provider()
if not t.require(prov is not None, "a real provider is configured", "no real provider in ~/.pi/agent/models.json"):
    t.done(); sys.exit(1)

# Split the real provider URL into host:port for the forwarder.
from urllib.parse import urlparse
u = urlparse(prov["url"])
host = u.hostname
port = u.port or (443 if u.scheme == "https" else 80)
t.require(u.scheme == "http", "the real provider is plain HTTP (the forwarder is a TCP hop)",
          f"scheme={u.scheme} (HTTPS cannot be forwarded by a plain TCP pipe here)")

fwd = Forwarder(host, port)
hub = Hub(plugins_src=PI)
try:
    hub.start()
    hub.post("/v1/model-providers", {"id": "p", "url": fwd.url, "api": prov["api"], "token": prov["token"]})
    r = hub.post("/v1/sessions", {"harnessId": HARNESS, "modelProviderId": "p", "modelId": MODEL}, key="nd-s")
    sid = r["json"]["session"]["id"]
    s = hub.wait_status(sid, "active")
    t.check(s.get("status") == "active", "the session starts with the provider reached THROUGH the forwarder",
            f"status={s.get('status')} err={s.get('startError')}")

    tr = hub.post(f"/v1/sessions/{sid}/turns", {"content": [{"type": "text", "text": "Count slowly from 1 to 500, one number per line."}], "idempotencyKey": "nd-t"}, key="nd-t")
    t.check(tr["status"] == 202, "the turn is accepted", f"status={tr['status']}")
    tid = tr["json"]["turn"]["id"]
    row = wait_turn(hub, sid, tid, {"running", "awaiting_approval", "awaiting_question", "ended"}, tries=160)
    running = row.get("state") in ("running", "awaiting_approval", "awaiting_question")
    t.check(running, "the turn is running through the forwarder (a real model call in flight)",
            f"state={row.get('state')}")

    if running:
        # CUT THE NETWORK for real: the forwarder dies, the in-flight connection resets.
        fwd.die()
        # The turn must settle honestly within a bound.
        end = wait_turn(hub, sid, tid, {"ended"}, tries=300)
        t.check(end.get("state") == "ended", "the turn settles after the network drop (no hang)",
                f"state={end.get('state')}")
        t.check(end.get("ended") != "completed", "a dropped network is never a clean `completed`",
                f"ended={end.get('ended')}")
        # No cancel and no restart: the turn died on its own, so the terminal is
        # `failed` (ARCHITECTURE §21 N2 reserves `interrupted` for a turn that was
        # `cancelling`; `cancelled` needs a confirmed cancel - neither happened here).
        t.check(end.get("ended") == "failed",
                "a turn killed by a network drop ends `failed`",
                f"ended={end.get('ended')} cause={end.get('cause')} (required: failed)")
        # The hub is still alive and answers.
        t.check(hub.child.poll() is None, "the hub survives the network drop")
        t.check(hub.get(f"/v1/sessions/{sid}")["status"] == 200, "the hub still answers after the drop")
finally:
    try:
        fwd.die()
    except Exception:
        pass
    ok = t.done()
    hub.cleanup()
sys.exit(0 if ok else 1)

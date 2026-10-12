# TC-INTERRUPT-013
# PASSIVE P5 (MID-TURN): the PROVIDER fails MID-TURN - an API error AFTER the first token.
# The real provider is reached through a REAL HTTP proxy (a real hop, not a fake provider);
# the session starts, the turn runs, the model streams its FIRST tokens, and THEN the proxy
# makes the upstream connection die mid-stream (a real mid-flight failure). The turn must
# settle HONESTLY (not hang, never `completed`), and the hub must keep answering.
#
# FACT:    a provider that fails after the first token settles the turn (failed), never a clean completed; the hub survives
# SOURCE:; contract/v1.json turn.ended
# EXPOSES: a mid-turn provider error reported as a completed turn, or a frozen hub
# (A test that would pass whatever happens is not a test: this block names the fact it
#  proves and where that fact comes from; the assertions below are that exact fact.)
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "lib"))
from hub import Hub, hub_sha, real_provider, wait_turn           # noqa: E402
from httpfault import HttpFault                                   # noqa: E402
from tally import Tally, combo, kind_of                           # noqa: E402

PI = os.environ.get("PI_PLUGIN_DIR", r"E:/AI/ideas/prts-harness-pi")
HARNESS = os.environ.get("PI_HARNESS_ID", "pi")
MODEL = os.environ.get("PI_MODEL", "gpt-6-astra")

t = Tally("interrupt/provider-errors-mid-turn")
combo(hub_sha(), kind_of(PI))
if not t.require(os.path.isdir(PI), "a real plugin is present", f"no real plugin at {PI}"):
    t.done(); sys.exit(1)

prov = real_provider()
if not t.require(prov is not None, "a real provider is configured", "no real provider in ~/.pi/agent/models.json"):
    t.done(); sys.exit(1)

from urllib.parse import urlparse
u = urlparse(prov["url"])
t.require(u.scheme in ("http", "https"), "the real provider has an HTTP(S) URL", f"scheme={u.scheme}")

# Cut AFTER the stream has delivered the first tokens: 2 KiB of SSE body is more than the
# first `content_block_delta`, so the model has provably started answering before the cut.
CUT_AFTER = int(os.environ.get("PI_P5_CUT_AFTER", "2048"))
fault = HttpFault(u.hostname, u.port or (443 if u.scheme == "https" else 80), cut_after_bytes=CUT_AFTER)
hub = Hub(plugins_src=PI)
try:
    hub.start()
    r = hub.post("/v1/model-providers", {"id": "HOME-JP-prod", "url": fault.url, "api": prov["api"], "token": prov["token"]})
    PROV = ((r.get("json") or {}).get("provider") or {}).get("id")
    t.check(r["status"] < 300, "a provider reached through the faulting proxy is registered", f"status={r['status']} {r['text'][:120]}")

    r = hub.post("/v1/sessions", {"harnessId": HARNESS, "modelProviderId": PROV, "modelId": MODEL}, key="pm-s")
    sid = r["json"]["session"]["id"]
    s = hub.wait_status(sid, "active")
    t.check(s.get("status") == "active", "the session starts with the provider reached THROUGH the proxy",
            f"status={s.get('status')} err={s.get('startError')}")

    tr = hub.post(f"/v1/sessions/{sid}/turns",
                  {"content": [{"type": "text", "text": "Count slowly from 1 to 500, one number per line."}],
                   "idempotencyKey": "pm-t"}, key="pm-t")
    t.check(tr["status"] == 202, "the turn is accepted", f"status={tr['status']}")
    tid = tr["json"]["turn"]["id"]

    # The turn runs: the model streams its first tokens through the proxy, THEN the proxy
    # kills the upstream mid-stream. The turn must settle honestly within a bound.
    end = wait_turn(hub, sid, tid, {"ended"}, tries=400)
    t.check(end.get("state") == "ended", "the turn settles after the mid-turn provider failure (no hang)",
            f"state={end.get('state')}")
    t.check(fault.cut_done.is_set(), "the proxy actually cut a streaming response mid-flight (first tokens had been sent)",
            f"cut_done={fault.cut_done.is_set()}")
    t.check(end.get("ended") != "completed", "a mid-turn provider error is never a clean `completed`",
            f"ended={end.get('ended')}")
    t.check(end.get("ended") == "failed", "a turn killed by a mid-turn provider error ends `failed`",
            f"ended={end.get('ended')} cause={end.get('cause')} (required: failed)")
    t.check(hub.child.poll() is None, "the hub survives the mid-turn provider failure")
    t.check(hub.get(f"/v1/sessions/{sid}")["status"] == 200, "the hub still answers after the failure")
finally:
    try:
        fault.die()
    except Exception:
        pass
    ok = t.done()
    hub.cleanup()
sys.exit(0 if ok else 1)

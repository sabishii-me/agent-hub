# PASSIVE P5 (start-time): a provider whose endpoint is UNREACHABLE (a real dead route ->
# a real ECONNREFUSED, not a fake server) must fail the session START honestly, naming
# the real reason - never a session that looks `active` and then silently produces no
# model answer. The failure is real (a real connection attempt); the hub is driven
# through /v1.
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "lib"))
from hub import Hub, hub_sha                                    # noqa: E402
from tally import Tally, combo, kind_of                          # noqa: E402

PI = os.environ.get("PI_PLUGIN_DIR", r"E:/AI/ideas/prts-harness-pi")
HARNESS = os.environ.get("PI_HARNESS_ID", "pi")
DEAD = os.environ.get("DEAD_PROVIDER_URL", "http://127.0.0.1:9")

t = Tally("interrupt/provider-unreachable")
combo(hub_sha(), kind_of(PI))
if not t.require(os.path.isdir(PI), "a real plugin is present", f"no real plugin at {PI}"):
    t.done(); sys.exit(1)

hub = Hub(plugins_src=PI)
try:
    hub.start()
    c = hub.post("/v1/model-providers", {"id": "dead", "url": DEAD, "api": "anthropic-messages", "token": "x-not-a-key"})
    t.check(c["status"] < 300, "a provider with a dead endpoint is registered", f"status={c['status']} {c['text'][:120]}")

    r = hub.post("/v1/sessions", {"harnessId": HARNESS, "modelProviderId": "dead", "modelId": "deepseek-flash"}, key="pu-s")
    sid = r["json"]["session"]["id"]
    s = hub.wait_status(sid, "active")
    # HONEST: the session must NOT be `active` (an unreachable provider cannot serve).
    t.check(s.get("status") != "active", "a session with an unreachable provider does NOT become active",
            f"status={s.get('status')}")
    t.check(s.get("status") == "starting_failed", "it is `starting_failed`", f"status={s.get('status')}")
    err = (s.get("startError") or "")
    t.check("ECONNREFUSED" in err or "refus" in err.lower() or "connect" in err.lower(),
            "the reason names the real connection failure (not a generic message)", f"startError={err[:160]}")

    # A turn cannot be sent on a failed session (not silently accepted into a void).
    tr = hub.post(f"/v1/sessions/{sid}/turns", {"content": [{"type": "text", "text": "hi"}], "idempotencyKey": "pu-t"}, key="pu-t")
    t.check(tr["status"] >= 400, "a turn on a `starting_failed` session is refused", f"status={tr['status']} {tr['text'][:120]}")
finally:
    ok = t.done()
    hub.cleanup()
sys.exit(0 if ok else 1)

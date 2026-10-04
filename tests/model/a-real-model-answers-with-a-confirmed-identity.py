# A real model turn must produce a real assistant message AND confirm the identity: the model
# that answered is the one that was requested, and the hub reports it. A second turn continues
# the SAME conversation. This needs a REAL provider, so it SKIPs without authorization.
import os
import sys
import time

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "lib"))
from hub import Hub, hub_sha, real_provider      # noqa: E402
from tally import Tally, combo, kind_of          # noqa: E402

PI = os.environ.get("PI_PLUGIN_DIR", r"E:/AI/ideas/prts-harness-pi")
HARNESS = os.environ.get("PI_HARNESS_ID", "pi")
MODEL = os.environ.get("PI_MODEL", "deepseek-flash")

t = Tally("model/identity")
combo(hub_sha(), kind_of(PI))
prov = real_provider()
if not t.require(os.path.isdir(PI) and prov is not None, "a real plugin and a real provider are present", ("no real provider in ~/.pi/agent/models.json" if not prov else f"no plugin at {PI}")):
    t.done()
    sys.exit(1)

def assistant_messages(hub, sid):
    g = hub.get(f"/v1/sessions/{sid}/messages")
    msgs = (g["json"] or {}).get("messages", [])
    return [m for m in msgs if m.get("role") == "assistant"]

hub = Hub(plugins_src=PI)
try:
    hub.start()
    hub.post("/v1/model-providers", {"id": "p", "url": prov["url"], "api": prov["api"], "token": prov["token"]})
    r = hub.post("/v1/sessions", {"harnessId": HARNESS, "modelProviderId": "p", "modelId": MODEL}, key="m-1")
    sid = r["json"]["session"]["id"]
    s = hub.wait_status(sid, "active")
    t.check(s.get("status") == "active", "the session is active", f"status={s.get('status')} err={s.get('startError')}")
    t.check(s.get("appliedProvider") == "p", "the hub confirms the applied provider", f"appliedProvider={s.get('appliedProvider')}")
    t.check(s.get("appliedModel") == MODEL, "the hub confirms the applied model", f"appliedModel={s.get('appliedModel')}")

    # turn 1
    hub.post(f"/v1/sessions/{sid}/turns", {"content": [{"type": "text", "text": "Remember the number 4242. Reply with just: ok"}], "idempotencyKey": "m-t1"}, key="m-t1")
    a1 = None
    for _ in range(600):
        am = assistant_messages(hub, sid)
        if am:
            a1 = am[-1]
            break
        time.sleep(0.5)
    t.check(a1 is not None, "a real assistant message arrives", "")
    if a1:
        # The contract's defs.message carries NO model field: identity is SESSION-level
        # (appliedProvider/appliedModel, checked above). Assert the CONTRACT shape, not a
        # field I expected.
        t.check("model" not in a1 and "modelId" not in a1,
                "the message shape matches the contract (no per-message model field)", f"keys={sorted(a1.keys())}")
        t.check("id" in a1 and "role" in a1 and "content" in a1, "the message has the required contract fields")

    # turn 2 continues the SAME conversation: ask for the remembered number.
    before = len(assistant_messages(hub, sid))
    hub.post(f"/v1/sessions/{sid}/turns", {"content": [{"type": "text", "text": "What number did I ask you to remember? Reply with just the number."}], "idempotencyKey": "m-t2"}, key="m-t2")
    a2 = None
    for _ in range(900):
        am = assistant_messages(hub, sid)
        if len(am) > before:
            a2 = am[-1]
            break
        time.sleep(0.5)
    t.check(a2 is not None, "a second turn produces a new assistant message", "")
    if a2:
        text = "".join(c.get("text", "") for c in (a2.get("content") or []) if c.get("type") == "text")
        t.check("4242" in text, "the second turn recalls turn 1 (the conversation CONTINUES)", f"reply={text[:120]!r}")

    hub.delete("/v1/model-providers/p")
finally:
    ok = t.done()
    hub.cleanup()
sys.exit(0 if ok else 1)

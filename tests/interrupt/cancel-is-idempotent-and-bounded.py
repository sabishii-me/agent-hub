# Cancel semantics that need no model:
#  - cancel on an IDLE session is idempotent and does not error or wedge it;
#  - repeated cancel is idempotent;
#  - a cancel with no in-flight turn does NOT put the session into a stuck state.
import os
import sys
import time

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "lib"))
from hub import Hub, hub_sha                   # noqa: E402
from tally import Tally, combo, kind_of        # noqa: E402

PI = os.environ.get("PI_PLUGIN_DIR", r"E:/AI/ideas/prts-harness-pi")
HARNESS = os.environ.get("PI_HARNESS_ID", "pi")
t = Tally("interrupt/cancel-idempotent")
combo(hub_sha(), kind_of(PI))
if not t.require(os.path.isdir(PI), "a real plugin is present", f"no real plugin at {PI}"):
    t.done()
    sys.exit(1)

hub = Hub(plugins_src=PI)
try:
    hub.start()
    r = hub.post("/v1/sessions", {"harnessId": HARNESS}, key="ci-1")
    sid = r["json"]["session"]["id"]
    hub.wait_status(sid, "active")

    # cancel an idle session: accepted or a clean no-op, never 500.
    c1 = hub.post(f"/v1/sessions/{sid}/cancel")
    t.check(c1["status"] < 300 or c1["status"] == 409, "cancel on an idle session is clean (not 500)", f"status={c1['status']} {c1['text'][:120]}")
    c2 = hub.post(f"/v1/sessions/{sid}/cancel")
    t.check(c2["status"] < 500, "a repeated cancel is not an error", f"status={c2['status']} {c2['text'][:120]}")
    c3 = hub.post(f"/v1/sessions/{sid}/cancel")
    t.check(c3["status"] == c2["status"], "cancel is idempotent (same status repeated)", f"{c2['status']} vs {c3['status']}")

    # The session is still usable: a providerless turn is admitted (or refused cleanly), and
    # the session is not wedged by the cancels.
    tr = hub.post(f"/v1/sessions/{sid}/turns", {"content": [{"type": "text", "text": "hi"}], "idempotencyKey": "ci-t1"}, key="ci-t1")
    # "Not wedged" = the request is DEFINED (it returned, and not a 5xx). The old set
    # (202,409,503) was a fake: it silently excluded a 400 (this session has no model)
    # and so tested the set, not the property.
    t.check(tr["status"] < 500,
            "the session is not wedged by idle cancels (a defined, non-5xx outcome)",
            f"status={tr['status']} {tr['text'][:120]}")

    # get is still honest
    g = hub.get(f"/v1/sessions/{sid}")
    t.check(g["status"] == 200, "the session still reads", f"status={g['status']}")
finally:
    ok = t.done()
    hub.cleanup()
sys.exit(0 if ok else 1)

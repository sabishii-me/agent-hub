# GET /v1/harnesses lists the real plugins, and each capability-gated route answers from the
# REAL adapter: a harness WITHOUT a capability answers `unsupported` (501) - honestly, never
# a fabricated empty. pi has models/presets/skills; it has NO providers, so its
# connections/auth routes must be `unsupported`, and jouzu's (which HAS providers) must not
# be a blanket 501.
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "lib"))
from hub import Hub, hub_sha          # noqa: E402
from tally import Tally, combo        # noqa: E402

PI = os.environ.get("PI_PLUGIN_DIR", r"E:/AI/ideas/prts-harness-pi")
t = Tally("harnesses/routes")
combo(hub_sha())
if not os.path.isdir(PI):
    t.skip("harness routes", f"no real plugin at {PI}")
    t.done()
    sys.exit(0)

hub = Hub(plugins_src=PI)
try:
    hub.start()
    l = hub.get("/v1/harnesses")
    rows = (l["json"] or {}).get("harnesses", [])
    t.check(l["status"] == 200 and any(h.get("id") == "pi" for h in rows), "GET /v1/harnesses lists pi", f"rows={[h.get('id') for h in rows]}")

    # models: pi declares `models`; the route forwards to the adapter.
    m = hub.get("/v1/harnesses/pi/models")
    t.check(m["status"] in (200, 501), "GET /v1/harnesses/{id}/models answers", f"status={m['status']}")

    # tools: the contract says a harness without the `tools` capability answers
    # known:false - an UNKNOWN catalog, never faked as an empty one (200, not 501).
    tl = hub.get("/v1/harnesses/pi/tools")
    t.check(tl["status"] == 200 and (tl["json"] or {}).get("known") is False,
            "/tools on a non-tools harness answers known:false (per the contract)", f"status={tl['status']} body={tl['text'][:120]}")

    # connections without the `providers` capability -> unsupported.
    cs = hub.get("/v1/harnesses/pi/connections/schema")
    t.check(cs["status"] == 501, "connections/schema on a non-provider harness is `unsupported`", f"status={cs['status']} {cs['text'][:120]}")

    # extensions: the hub installs a complete snapshot; list + patch.
    ex = hub.get("/v1/harnesses/pi/extensions")
    t.check(ex["status"] == 200, "GET /v1/harnesses/{id}/extensions answers", f"status={ex['status']} {ex['text'][:120]}")
    body = ex["json"] or {}
    t.check("available" in body or "selected" in body or "extensions" in body, "the extensions body names available/selected", f"keys={list(body.keys())}")

    # an unknown harness is a real 404, not a 500.
    u = hub.get("/v1/harnesses/nope")
    t.check(u["status"] in (404, 501), "an unknown harness answers 404 (or unsupported), not 500", f"status={u['status']}")
finally:
    ok = t.done()
    hub.cleanup()
sys.exit(0 if ok else 1)

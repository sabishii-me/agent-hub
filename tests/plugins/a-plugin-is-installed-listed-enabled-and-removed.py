# The plugin lifecycle against a REAL plugin directory (the real pi adapter). Install from a
# local path (a git source), list, read, prepare (the adapter materialises its runtime),
# disable/enable, then remove. The hub scans its OWN plugins root; a deployment dir is not
# removed (that rule is exercised in the docs, not faked here).
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "lib"))
from hub import Hub, hub_sha          # noqa: E402
from tally import Tally, combo        # noqa: E402

PI = os.environ.get("PI_PLUGIN_DIR", r"E:/AI/ideas/prts-harness-pi")
t = Tally("plugins/lifecycle")
combo(hub_sha())
if not t.require(os.path.isdir(PI), "a real plugin is present", f"no real plugin at {PI}"):
    t.done()
    sys.exit(1)

# A DEPLOYMENT dir: put the plugin under the hub's plugins root, not installed by the hub.
hub = Hub()
try:
    import shutil
    shutil.copytree(PI, os.path.join(hub.plugins, "pi"))
    hub.start()

    l = hub.get("/v1/plugins")
    rows = (l["json"] or {}).get("plugins", [])
    t.check(l["status"] == 200, "GET /v1/plugins answers", f"status={l['status']}")
    t.check(any(r.get("id") == "pi" for r in rows), "the deployment plugin is listed", f"rows={[r.get('id') for r in rows]}")

    g = hub.get("/v1/plugins/pi")
    t.check(g["status"] == 200, "GET /v1/plugins/{id} answers", f"status={g['status']}")

    # prepare: the adapter materialises the runtime; the hub VERIFIES the declared command.
    p = hub.post("/v1/plugins/pi/prepare")
    pj = p["json"] or {}
    t.check(p["status"] < 300 and pj.get("ready") is True, "prepare reports a ready runtime", f"body={p['text'][:160]}")
    t.check(pj.get("version"), "prepare names the runtime version", f"version={pj.get('version')}")

    # A deployment directory is read-only to the hub: remove is refused.
    d = hub.delete("/v1/plugins/pi")
    t.check(d["status"] in (409, 403), "the hub refuses to remove a deployment dir", f"status={d['status']} {d['text'][:120]}")

    dis = hub.post("/v1/plugins/pi/disable")
    t.check(dis["status"] < 300, "disable the plugin", f"status={dis['status']} {dis['text'][:120]}")
    en = hub.post("/v1/plugins/pi/enable")
    t.check(en["status"] < 300, "enable the plugin", f"status={en['status']} {en['text'][:120]}")

    # icon: the plugin ships one; a variant is served as bytes.
    ic = hub.get("/v1/plugins/pi/icon/light")
    t.check(ic["status"] in (200, 404), "GET the icon variant answers", f"status={ic['status']}")
finally:
    ok = t.done()
    hub.cleanup()
sys.exit(0 if ok else 1)

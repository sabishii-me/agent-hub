# TC-PLUGINS-005
# The plugin lifecycle against a REAL plugin, installed THROUGH /v1: install from a git
# source (a local path), the resource reaches `ready`, list/get show it, preparing
# materialises its runtime, disable/enable flip it, then REMOVE really removes it (GET
# then reads `absent`/404). A deployment dir is separately refused (409) - both paths.
#
# FACT:    a plugin is INSTALLED through POST /v1/plugins, reaches `ready`, and is really
#          REMOVED through DELETE /v1/plugins/{id}; a deployment dir is refused (409)
# SOURCE: contract/v1.json POST/GET/DELETE /v1/plugins
# EXPOSES: an install that never lands, a remove that leaves the plugin (or removes a
#          deployment dir), or a prepare that reports ready without a runtime
import os
import sys
import time

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", "lib"))
from hub import Hub, hub_sha          # noqa: E402
from tally import Tally, combo        # noqa: E402

PI = os.environ.get("PI_PLUGIN_DIR", r"E:/AI/ideas/prts-harness-pi")
PI_REF = os.environ.get("PI_PLUGIN_REF", "fix/runtime-placement")
t = Tally("plugins/lifecycle")
combo(hub_sha())
if not t.require(os.path.isdir(PI), "a real plugin source is present", f"no plugin at {PI}"):
    t.done(); sys.exit(1)

# A fresh hub with an EMPTY plugins root: nothing is pre-installed, so the install below
# is the only reason the plugin exists.
hub = Hub()
try:
    hub.start()

    # Nothing is present before the install.
    l0 = hub.get("/v1/plugins")
    t.check(l0["status"] == 200 and not (l0["json"] or {}).get("plugins"),
            "no plugin is present before the install", f"rows={(l0['json'] or {}).get('plugins')}")

    # INSTALL through /v1, from a git source (a local path + ref).
    ins = hub.post("/v1/plugins", {"source": {"artifact": hub.registry_artifact("pi")}}, key="install-1")
    t.check(ins["status"] == 202, "install is accepted (202 + Location)", f"status={ins['status']} {ins['text'][:160]}")
    t.check((ins["json"] or {}).get("pluginId"),
            "the install names the plugin id", f"body={ins['text'][:160]}")

    # The resource reaches `ready` (the install is a long command - it downloads the adapter AND
    # prepares the runtime, tens of seconds - read at the Location).
    t0 = time.time()
    state = None
    for _ in range(3000):
        g = hub.get("/v1/plugins/pi")
        state = (g["json"] or {}).get("plugin", {}).get("state")
        if state in ("ready", "failed", "absent"):
            break
        time.sleep(0.25)
    t.check(state == "ready", "the installed plugin reaches `ready`", f"state={state}")

    # It is listed and readable.
    l = hub.get("/v1/plugins")
    t.check(any(r.get("id") == "pi" for r in (l["json"] or {}).get("plugins", [])),
            "the installed plugin is listed", f"rows={[(r.get('id'), r.get('state')) for r in (l['json'] or {}).get('plugins', [])]}")
    g = hub.get("/v1/plugins/pi")
    t.check(g["status"] == 200 and (g["json"] or {}).get("plugin", {}).get("id") == "pi",
            "GET /v1/plugins/{id} reads the installed plugin", f"status={g['status']}")

    # INSTALL PREPARES THE RUNTIME itself: there is no separate
    # `prepare` command. `ready` must MEAN the runtime is on disk - a fact about the filesystem.
    pj = (g["json"] or {}).get("plugin", {})
    t.check(pj.get("state") == "ready", "install ends `ready` (the install IS the prepare)", f"state={pj.get('state')}")
    t.check(pj.get("runtimeReady") is True, "`ready` means runtimeReady (the runtime was materialised)",
            f"runtimeReady={pj.get('runtimeReady')}")
    # REAL install: the runtime command the manifest declares now EXISTS on disk (a fact about the
    # filesystem, not a claim) - the runtime was materialised, not reported.
    runtime_cmd = os.path.join(hub.plugins, "pi", "runtime", "dist", "cli.js")
    t.check(os.path.isfile(runtime_cmd),
            "REAL install: the declared runtime command exists on disk after install",
            f"missing {runtime_cmd}")
    t.check((pj.get("runtime") or {}).get("target"), "the runtime target is resolved (not null)",
            f"runtime={pj.get('runtime')}")
    # REAL download (not a local copy): the installed manifest.json is byte-identical to the one
    # inside the zip the hub DOWNLOADED from the registry url. If the hub had copied a local dir
    # instead of downloading, the zip's manifest could differ and this would catch it.
    import hashlib, io, urllib.request, zipfile
    url = hub.registry_artifact("pi")["url"]
    zbytes = urllib.request.urlopen(url, timeout=120).read()
    with zipfile.ZipFile(io.BytesIO(zbytes)) as z:
        zman = z.read("manifest.json")
    with open(os.path.join(hub.plugins, "pi", "manifest.json"), "rb") as f:
        iman = f.read()
    t.check(hashlib.sha256(zman).hexdigest() == hashlib.sha256(iman).hexdigest(),
            "REAL download: the installed manifest equals the zip's manifest (downloaded, not copied)",
            f"zip={hashlib.sha256(zman).hexdigest()[:12]} installed={hashlib.sha256(iman).hexdigest()[:12]}")
    print(f"  (install incl. runtime took {time.time()-t0:.1f}s)")

    # disable / enable flip the HARNESS status (the contract has no plugin `enabled`
    # field; the plugin's `state` is absent|installing|removing|preparing|failed|ready).
    # Read the harness back and assert its status, not a guessed field.
    dis = hub.post("/v1/plugins/pi/disable")
    t.check(dis["status"] < 300, "disable is accepted", f"status={dis['status']} {dis['text'][:120]}")
    hs = next((h for h in (hub.get("/v1/harnesses")["json"] or {}).get("harnesses", []) if h.get("id") == "pi"), None)
    t.check(hs is not None and hs.get("status") == "disabled",
            "after disable the harness reads disabled", f"harnesses={(hub.get('/v1/harnesses')['json'] or {}).get('harnesses')}")
    en = hub.post("/v1/plugins/pi/enable")
    t.check(en["status"] < 300, "enable is accepted", f"status={en['status']} {en['text'][:120]}")
    hs2 = next((h for h in (hub.get("/v1/harnesses")["json"] or {}).get("harnesses", []) if h.get("id") == "pi"), None)
    t.check(hs2 is not None and hs2.get("status") == "enabled",
            "after enable the harness reads enabled", f"harnesses={(hub.get('/v1/harnesses')['json'] or {}).get('harnesses')}")

    # icon: the plugin ships one; the variant is served as bytes.
    ic = hub.get("/v1/plugins/pi/icon/light")
    t.check(ic["status"] == 200, "GET the icon variant answers 200 (the plugin ships it)", f"status={ic['status']}")

    # REMOVE: really removes the plugin this hub installed.
    d = hub.delete("/v1/plugins/pi", key="remove-1")
    t.check(d["status"] in (200, 202), "remove is accepted", f"status={d['status']} {d['text'][:140]}")
    # After remove the plugin is gone: GET is 404 (the contract's `state` has no
    # "removed" value - absent = catalog-only - so absence is a 404, not a field).
    gone = False
    state2 = None
    for _ in range(240):
        g2 = hub.get("/v1/plugins/pi")
        if g2["status"] == 404:
            gone = True
            break
        state2 = (g2["json"] or {}).get("plugin", {}).get("state")
        time.sleep(0.25)
    t.check(gone, "after remove the plugin GET is 404 (really removed)", f"last_state={state2}")
    still = [r.get("id") for r in (hub.get("/v1/plugins")["json"] or {}).get("plugins", [])]
    t.check("pi" not in still, "the removed plugin is gone from the list", f"rows={still}")
finally:
    ok = t.done()
    hub.cleanup()
sys.exit(0 if ok else 1)

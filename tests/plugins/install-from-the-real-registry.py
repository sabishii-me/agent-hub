# TC-PLUGIN-REAL-REGISTRY: install from the OFFICIAL registry over the network (no mock at all)
#
# This is the test that proves the hub USES its real registry. It runs against the RELEASE binary,
# whose registry address is COMPILED IN (crates/plugins/src/registry.rs OFFICIAL_REGISTRY_URL =
# https://github.com/sabishii-me/agent-hub/releases/download/registry/registry.json) - there is no
# environment override in a release build, so NOTHING here can be a mock: the hub fetches the
# registry from GitHub and installs the release artifact from GitHub.
#
# It must run in the PROD configuration (AGENT_HUB_ENV=prod, AGENT_HUB_PROFILE=release). In dev the
# harness injects a loopback mock registry, which is exactly what this test must NOT use - so it
# FAILS in dev with a clear reason rather than passing against a mock.
import json
import os
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.join(ROOT, "tests", "lib"))
from hub import Hub, environment  # noqa: E402
from tally import Tally  # noqa: E402

t = Tally("plugins/real-registry")
MUST = os.environ.get("AGENT_HUB_REAL_PLUGIN", "pi")

# A RELEASE build has no registry override; a dev build does. If we are NOT in prod, the hub would
# be handed a mock - so this test cannot be honest. Say so, do not pretend.
if environment() != "prod":
    t.check(False, "the real-registry test must run with AGENT_HUB_ENV=prod (no mock registry)",
            f"environment={environment()} would inject a loopback mock; run: AGENT_HUB_ENV=prod python {os.path.basename(__file__)}")
    ok = t.done()
    sys.exit(0 if ok else 1)

# Hub started with NOTHING injected: no AGENT_HUB_REGISTRY_URL, no plugins_src. The hub MUST use its
# compiled-in official address.
hub = Hub()
try:
    hub.start()

    # 1. The hub refreshes from the OFFICIAL url (this is the real network call).
    rr = hub.post("/v1/plugins/registry/refresh", timeout=60)
    t.check(rr["status"] < 300, "the hub refreshed its OFFICIAL registry (real GitHub fetch)",
            f"status={rr['status']} body={rr['text'][:200]}")

    # 2. The catalog restates it.
    cat = hub.get("/v1/plugins/catalog")["json"] or {}
    ids = [p.get("id") for p in cat.get("plugins", [])]
    t.check(MUST in ids, f"the official catalog lists `{MUST}` (and the hub did not fabricate it)",
            f"ids={ids}")

    # 3. The hub's OWN catalog names a github.com release url (proof it is the published artifact).
    entry = next((p for p in cat.get("plugins", []) if p.get("id") == MUST), None)
    v = (entry or {}).get("versions", [{}])[0]
    t.check(v.get("url", "").startswith("https://github.com/sabishii-me/"),
            "the artifact url is a github.com release asset (the real published artifact)",
            f"url={v.get('url')}")

    # 4. Install it. The hub downloads the REGISTRY (done) and then the ARTIFACT and the RUNTIME,
    #    all over the network.
    t0 = time.time()
    r = hub.post("/v1/plugins", {"source": {"artifact": {
        "url": v["url"], "sha256": v["sha256"], "id": MUST,
        "pluginType": entry.get("pluginType"), "version": v["version"], "size": v.get("size")}}},
        key="real-install")
    t.check(r["status"] == 202, "the install is accepted (202)", f"status={r['status']} {r['text'][:160]}")

    state = None
    while time.time() - t0 < 1800:
        g = hub.get(f"/v1/plugins/{MUST}")
        state = None if g["status"] == 404 else (g["json"] or {}).get("plugin", {}).get("state")
        if state in ("ready", "failed"):
            break
        time.sleep(0.5)
    t.check(state == "ready", f"the released `{MUST}` installs to `ready` (runtime materialised)",
            f"state={state} body={(g['json'] or {}) if state=='failed' else ''}")
    g = hub.get(f"/v1/plugins/{MUST}")
    pj = (g["json"] or {}).get("plugin", {})
    target = (pj.get("runtime") or {}).get("target")
    t.check(pj.get("runtimeReady") is True and target and os.path.exists(target),
            "the declared runtime command exists on disk after install",
            f"runtimeReady={pj.get('runtimeReady')} target={target}")
    print(f"  ({MUST} {v.get('version')} installed from the OFFICIAL registry in {time.time()-t0:.0f}s)")
finally:
    ok = t.done()
    hub.cleanup()
sys.exit(0 if ok else 1)

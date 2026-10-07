# REGISTRY-ONLINE: the hub downloads ITS registry from the configured address, lists it, and
# INSTALLS a release the registry itself names - end to end. This is the launch rehearsal: the ONLY
# difference from production is the address (a real HTTP server on loopback here; the hub's compiled
# address there).
#
# The test supplies the registry ADDRESS only. It never supplies a url/sha256 to install: the hub's
# own refresh + catalog produce the release, and the hub reconciles the install against that same
# registry. The registry bytes served ARE the repository's real registry.json (real releases, real
# digests) - not a self-invented catalog.
#
# FACT:    with AGENT_HUB_REGISTRY_URL at a real HTTP registry (the repo's registry.json), the hub
#          refreshes, its catalog lists the real ids, and POST /v1/plugins with the artifact THE
#          CATALOG NAMES installs the release (fresh root ~ ready).
# SOURCE:  crates/plugins/src/service.rs (refresh_registry, catalog, begin_install -> authorized);
#          crates/plugins/src/routes.rs; contract/errors.json; the repo's registry.json.
# EXPOSES: refresh that does not populate the catalog; an install path that ignores the registry; a
#          catalog that does not reflect the served bytes.
#
# Requires: network to fetch the real release zip (the digest is checked by the hub).
import http.server
import json
import os
import sys
import threading
import time

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "lib"))
from hub import Hub, hub_sha          # noqa: E402
from tally import Tally, combo        # noqa: E402

t = Tally("plugins/registry-online")
combo(hub_sha())

REPO = os.path.abspath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", ".."))
REGISTRY = os.path.join(REPO, "registry.json")
t.require(os.path.isfile(REGISTRY), "the repository registry.json is present", f"no {REGISTRY}")
with open(REGISTRY, encoding="utf-8") as f:
    REG_BYTES = f.read().encode("utf-8")
REAL_IDS = {p["id"] for p in json.loads(REG_BYTES).get("plugins", [])}


class _Reg(http.server.BaseHTTPRequestHandler):
    """Serves the repository's registry.json verbatim - a REAL registry, not a canary."""
    def do_GET(self):
        self.send_response(200)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(REG_BYTES)))
        self.end_headers()
        self.wfile.write(REG_BYTES)

    def log_message(self, *a):
        pass


srv = http.server.HTTPServer(("127.0.0.1", 0), _Reg)
threading.Thread(target=srv.serve_forever, daemon=True).start()
URL = f"http://127.0.0.1:{srv.server_address[1]}/registry.json"

hub = Hub(env={"AGENT_HUB_REGISTRY_URL": URL})
try:
    hub.start()
    # 1. The hub fetches ITS registry from the configured address.
    rr = hub.post("/v1/plugins/registry/refresh", timeout=60)
    t.check(rr["status"] < 300, "the hub refreshes its registry from the configured address",
            f"status={rr['status']} {rr['text'][:160]}")

    # 2. The catalog reflects the served registry (the REAL ids).
    cat = hub.get("/v1/plugins/catalog")
    got = {p.get("id") for p in ((cat["json"] or {}).get("plugins") or [])}
    t.check(REAL_IDS and REAL_IDS <= got, "the catalog lists the registry's real ids",
            f"expected~{sorted(REAL_IDS)} got={sorted(got)}")

    # 3. Install a LISTED harness id, taking the release the HUB'S CATALOG names (the test supplies
    #    no url/sha256 of its own).
    entry = next((p for p in ((cat["json"] or {}).get("plugins") or [])
                  if p.get("pluginType") == "harness-adapter"), None)
    if not t.check(entry is not None, "the registry lists a harness-adapter to install",
                   f"entries={sorted(got)}"):
        raise SystemExit(1)
    pid = entry["id"]
    v = (entry.get("versions") or [None])[0]
    if not t.check(bool(v), f"the catalog names a release for `{pid}`", f"entry={entry}"):
        raise SystemExit(1)
    before = hub.get("/v1/plugins").get("json", {}).get("plugins", [])
    t.check(not any(p.get("id") == pid for p in before), "the id is NOT installed before the install", "")

    art = {"url": v["url"], "sha256": v["sha256"], "id": pid,
           "pluginType": entry.get("pluginType"), "version": v["version"], "size": v.get("size")}
    r = hub.post("/v1/plugins", {"source": {"artifact": art}}, key="online-" + pid, timeout=60)
    t.check(r["status"] == 202, f"installing the catalog's release for `{pid}` is accepted",
            f"status={r['status']} {r['text'][:160]}")

    state = None
    for _ in range(1920):
        g = hub.get(f"/v1/plugins/{pid}")
        state = (g["json"] or {}).get("plugin", {}).get("state")
        if state in ("ready", "failed", "absent"):
            break
        time.sleep(0.25)
    t.check(state == "ready", f"`{pid}` reaches `ready` from the registry's release", f"state={state}")
finally:
    hub.cleanup()
    srv.shutdown()

_ok = t.done()
sys.exit(0 if _ok else 1)

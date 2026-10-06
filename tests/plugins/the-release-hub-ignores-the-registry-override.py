# Does a RELEASE build honour AGENT_HUB_REGISTRY_URL? It MUST NOT: the registry address is
# compiled in, so neither an API caller nor the environment can point the hub at a foreign
# registry (the injection hole, docs/issues/20261005-130000).
#
# The discriminator is a fact about the SERVER, not about a catalog list: the test stands up a
# canary registry on loopback and counts whether the hub CONTACTS it.
#   - DEBUG build  -> the canary server MUST be hit (the override is how tests point the hub);
#   - RELEASE build-> the canary server MUST NOT be hit (the override does not exist), and the
#                     refresh must have gone to the built-in address instead.
# This cannot pass on an 'empty list' the way the previous version could: an empty catalog is not
# evidence, the server-hit count is.
#
# FACT:    with AGENT_HUB_REGISTRY_URL at a canary, a DEBUG hub CONTACTS the canary; a RELEASE hub
#          does NOT (its override is compiled out). Measured by the server's own hit count.
# SOURCE:  crates/plugins/src/registry.rs (env override #[cfg(debug_assertions)] only);
#          docs/issues/20261005-130000.
# EXPOSES: a release binary that reads the override -> anyone can redirect the hub's registry.
import http.server
import json
import os
import subprocess
import sys
import threading

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "lib"))
from tally import Tally, combo        # noqa: E402
from hub import hub_sha               # noqa: E402

t = Tally("plugins/release-ignores-override")
combo(hub_sha())

CANARY = "OVERRIDE-WAS-USED"
REG = {"schema": 1, "note": "canary", "plugins": [
    {"id": CANARY, "pluginType": "harness-adapter", "name": "Canary", "versions": []}]}

HITS = {"n": 0}


class _H(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        HITS["n"] += 1
        body = json.dumps(REG).encode()
        self.send_response(200)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *a):
        pass


def run(profile):
    """Run a hub of `profile` in its own process with AGENT_HUB_REGISTRY_URL at a canary
    server. Return (server_hits, refresh_status, catalog_ids)."""
    HITS["n"] = 0
    srv = http.server.HTTPServer(("127.0.0.1", 0), _H)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    url = f"http://127.0.0.1:{srv.server_address[1]}/registry.json"
    code = f'''
import os, sys
os.environ["AGENT_HUB_PROFILE"] = {profile!r}
sys.path.insert(0, os.path.join(os.getcwd(), "tests", "lib"))
from hub import Hub
h = Hub(env={{"AGENT_HUB_REGISTRY_URL": {url!r}}})
try:
    h.start()
    r = h.post("/v1/plugins/registry/refresh", timeout=40)
    c = h.get("/v1/plugins/catalog")
    ids = [p.get("id") for p in ((c["json"] or {{}}).get("plugins") or [])]
    print("REFRESH", r["status"])
    print("IDS", ",".join(ids))
finally:
    h.cleanup()
'''
    out = subprocess.run([sys.executable, "-c", code], capture_output=True, text=True, timeout=120)
    srv.shutdown()
    status, ids = None, []
    for line in out.stdout.splitlines():
        if line.startswith("REFRESH "):
            status = int(line[8:])
        if line.startswith("IDS "):
            ids = [x for x in line[4:].split(",") if x]
    return HITS["n"], status, ids


dbg_hits, dbg_status, dbg_ids = run("debug")
rel_hits, rel_status, rel_ids = run("release")
print(f"debug  : canary hits={dbg_hits} refresh={dbg_status} ids={dbg_ids}")
print(f"release: canary hits={rel_hits} refresh={rel_status} ids={rel_ids}")

# The DEBUG build must actually reach the canary: this proves the canary server works AND that the
# override is live in a dev build. Without this, "release did not hit it" proves nothing.
t.check(dbg_hits > 0 and CANARY in dbg_ids,
        "the DEBUG build uses AGENT_HUB_REGISTRY_URL and CONTACTED the canary (server was hit)",
        f"hits={dbg_hits} ids={dbg_ids}")

# The RELEASE build must NOT contact the canary at all - a fact about the server, not the catalog.
t.check(rel_hits == 0,
        "the RELEASE build did NOT contact the canary (the override is compiled out)",
        f"hits={rel_hits} ids={rel_ids}")

# And its refresh went to the BUILT-IN address (so the canary id is absent). Whether that address
# answers 2xx (published) or fails (unpublished) is a separate fact, reported separately.
t.check(CANARY not in rel_ids,
        "the RELEASE build's catalog has NO canary id (it did not use the override)",
        f"ids={rel_ids}")

_ok = t.done()
sys.exit(0 if _ok else 1)

# Does a RELEASE build honour AGENT_HUB_REGISTRY_URL? It MUST NOT: the registry address is
# compiled in, and an API caller / environment must not be able to point the hub at a foreign
# registry (that is the injection hole, docs/issues/20261005-130000).
#
# The test is a DECISIVE discriminator, not "it happened to work": it stands up a loopback
# registry that is DISTINGUISHABLE (plugin id `OVERRIDE-WAS-USED`) and sets
# AGENT_HUB_REGISTRY_URL at it. If the release hub used the override, its catalog would contain
# `OVERRIDE-WAS-USED`. It must instead contain only the FIRST-PARTY set from the build-time
# address and NOT the canary.
#
# FACT:    with AGENT_HUB_REGISTRY_URL pointing at a canary registry, a RELEASE hub's catalog
#          does NOT contain the canary id (the override is compiled out); a DEBUG hub's catalog
#          DOES (the override is how tests point it at loopback).
# SOURCE:  crates/plugins/src/registry.rs (env override is #[cfg(debug_assertions)] only);
#          docs/issues/20261005-130000.
# EXPOSES: a release binary that reads the override -> anyone can redirect the hub's registry.
import http.server
import json
import os
import subprocess
import sys
import threading

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "lib"))

CANARY = "OVERRIDE-WAS-USED"
REG = {"schema": 1, "note": "canary", "plugins": [
    {"id": CANARY, "pluginType": "harness-adapter", "name": "Canary", "versions": []}]}


class _H(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        body = json.dumps(REG).encode()
        self.send_response(200)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *a):
        pass


def run(profile):
    """Run one hub of `profile` with AGENT_HUB_REGISTRY_URL at a canary; return the catalog
    ids it fetched. Uses a fresh subprocess so the profile module state is clean."""
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
    ids = []
    for line in out.stdout.splitlines():
        if line.startswith("IDS "):
            ids = [x for x in line[4:].split(",") if x]
    if not ids and out.returncode != 0:
        print("  (subprocess failed)", out.stderr[-400:])
    return ids


rel = run("release")
dbg = run("debug")

# The debug build MUST use the override (proves the canary server works at all).
print(f"debug catalog  : {dbg}")
print(f"release catalog: {rel}")
ok = True


def check(cond, what, detail=""):
    global ok
    print(("  ok   " if cond else "  FAIL ") + what + ("" if cond else f" - {detail}"))
    ok = ok and cond


check(CANARY in dbg, "the DEBUG build DOES use AGENT_HUB_REGISTRY_URL (the canary)",
      f"debug ids={dbg}")
check(CANARY not in rel, "the RELEASE build does NOT use AGENT_HUB_REGISTRY_URL (compiled out)",
      f"release ids={rel}")
print(f"  [plugins/release-ignores-override] {'PASS' if ok else 'FAIL'}")
sys.exit(0 if ok else 1)

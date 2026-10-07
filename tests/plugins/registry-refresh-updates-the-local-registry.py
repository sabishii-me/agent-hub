# The registry ADDRESS is compiled into the hub. A DEV build lets AGENT_HUB_REGISTRY_URL override
# it, which is how these tests point refresh at a bad source to exercise its ERROR handling. This
# file tests ONLY the error surface. It does NOT fabricate a WORKING registry and treat a
# self-invented id appearing in the catalog as success - the authorized/success path needs the
# OFFICIAL registry and is BLOCKED (owner direction: no mock registry).
#
# FACT:    POST /v1/plugins/registry/refresh with no URL, an unreachable address, a non-2xx
#          answer, a non-JSON body, or JSON without a plugins array fails 502 with code
#          `registry_unavailable` and a detail naming WHICH case; with a correct registry it
#          succeeds (2xx) and GET /v1/plugins/catalog restates exactly that registry.
# SOURCE:  crates/plugins/src/service.rs:144-172 (refresh_registry, all six cases),
#          99-143 (catalog); crates/plugins/src/routes.rs:89-101; transport/src/error.rs
#          render (body {"error":CODE,"detail":...}); contract/errors.json;
#          ARCHITECTURE.md:691-692; owner direction 2026-10-05.
# EXPOSES: any registry failure that returns a 500; a code that is not the registry's own
#          (today: plugin_install_failed, false + retryable — docs/issues/20261005-110000);
#          or a refresh wired to a file the catalog does not read.
import http.server
import json
import os
import socket
import sys
import threading

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "lib"))
from hub import Hub, hub_sha          # noqa: E402
from tally import Tally, combo        # noqa: E402

t = Tally("plugins/registry-refresh")
combo(hub_sha())

CODE = "registry_unavailable"        # the code the owner requires (issue 20261005-110000)
GOOD = {"schema": 1, "plugins": [{"id": "src-only", "pluginType": "harness-adapter",
                                  "name": "Src Only", "versions": []}]}


class _Handler(http.server.BaseHTTPRequestHandler):
    """One configurable registry source. `mode` decides what it answers."""
    mode = "ok"

    def do_GET(self):
        if self.mode == "http500":
            self.send_response(500); self.end_headers(); self.wfile.write(b"boom"); return
        if self.mode == "nonjson":
            body = b"<html>not json</html>"
        elif self.mode == "noarray":
            body = json.dumps({"schema": 1, "note": "no plugins here"}).encode()
        else:
            body = json.dumps(GOOD).encode()
        self.send_response(200)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *a):
        pass


def serve(mode):
    h = type("H", (_Handler,), {"mode": mode})
    srv = http.server.HTTPServer(("127.0.0.1", 0), h)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    return srv, f"http://127.0.0.1:{srv.server_address[1]}/registry.json"


def refresh_case(env, name, want_detail_contains):
    hub = Hub(env=env)
    try:
        hub.start()
        r = hub.post("/v1/plugins/registry/refresh")
        code = (r["json"] or {}).get("error")
        detail = ((r["json"] or {}).get("detail") or "").lower()
        t.check(r["status"] == 502, f"{name}: is 502", f"status={r['status']} body={r['text'][:180]}")
        t.check(code == CODE, f"{name}: code is {CODE}", f"code={code} body={r['text'][:180]}")
        t.check(want_detail_contains.lower() in detail,
                f"{name}: detail names the case ({want_detail_contains!r})", f"detail={detail!r}")
    finally:
        hub.stop()


# --- Case 1 (changed): with NO override the hub uses the COMPILED-IN official address, so
#     "no env" is NOT an error. The error case for "cannot reach the registry" is an
#     unreachable override (Case 2). Here we only assert the hub does not report a missing
#     URL when an override is absent - it has an official address to use.
hub0 = Hub()
try:
    hub0.start()
    # No override: refresh must NOT fail with "not set"; it must contact the official address
    # (its outcome is 2xx if reachable, or a fetch failure - never a "not configured" error).
    r0 = hub0.post("/v1/plugins/registry/refresh", timeout=40)
    detail0 = ((r0["json"] or {}).get("detail") or "").lower()
    # With no override the hub contacts the COMPILED-IN address. Whether that answers 2xx or fails,
    # the failure must NOT be "no URL configured": the hub always has an address.
    t.check("not set" not in detail0 and "no registry url" not in detail0,
            "no override: the hub uses its compiled-in address, never a 'not configured' error",
            f"status={r0['status']} body={r0['text'][:180]}")
finally:
    hub0.cleanup()

# --- Case 2: an address that does not answer (closed port).
s = socket.socket(); s.bind(("127.0.0.1", 0)); dead = s.getsockname()[1]; s.close()
refresh_case({"AGENT_HUB_REGISTRY_URL": f"http://127.0.0.1:{dead}/registry.json"},
             "unreachable address", "fetch failed")

# --- Case 3: a server that answers 500.
srv3, url3 = serve("http500")
try:
    refresh_case({"AGENT_HUB_REGISTRY_URL": url3}, "non-2xx answer", "500")
finally:
    srv3.shutdown()

# --- Case 4: a server that answers a non-JSON body.
srv4, url4 = serve("nonjson")
try:
    refresh_case({"AGENT_HUB_REGISTRY_URL": url4}, "non-JSON body", "not json")
finally:
    srv4.shutdown()

# --- Case 5: a server that answers JSON without a plugins array.
srv5, url5 = serve("noarray")
try:
    refresh_case({"AGENT_HUB_REGISTRY_URL": url5}, "JSON without a plugins array", "plugins")
finally:
    srv5.shutdown()

# --- Case 6: a CORRECT, REACHABLE registry whose content becomes the catalog. This needs a real
# registry to be meaningful; a loopback server here would be the test fabricating the registry and
# proving only that the hub can read bytes it was handed. BLOCKED until the official one is
# published (owner direction: no mock registry).
t.blocked_check(
    "a reachable, valid registry: refresh succeeds and the catalog restates it",
    "needs the official registry (UNPUBLISHED); not tested by fabricating one")

_ok = t.done()
sys.exit(0 if _ok else 1)

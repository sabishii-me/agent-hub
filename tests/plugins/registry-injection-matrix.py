# TC-PLUGINS-008
# ENV: dev + release   MOCK: yes (a local HTTP registry / a local file registry)
#
# The registry-injection matrix, printed as a STATUS TABLE. Each row is one case with its
# EXPECTED result; the table shows the OBSERVED result and GREEN/RED. A GREEN row means only
# "this case met its expectation in THIS environment" - it is NOT a statement that the system
# is correct. Correctness is judged by the difference ACROSS environments (dev/staging/prod).
#
# FACT:    (1) official 404, no injection -> install refused 403 plugin_not_in_registry;
#          (2) dev + listed registry -> install accepted, reaches ready;
#          (3) release + the same injection -> refused (release reads no override);
#          (4) dev + unparseable registry -> 502 registry_unavailable (NOT 403);
#          (5) release cannot inject -> refused, same as (1);
#          (6) dev + registry URL requiring auth (401) -> 502 registry_unavailable;
#          (7) dev + 200 but not a registry -> 502 registry_unavailable.
# SOURCE:  crates/plugins/src/service.rs (load_registry_from_file, RegistryUrlMissing,
#          registry_file #[cfg(debug_assertions)]); crates/plugins/src/registry.rs; contract/errors.json.
# EXPOSES: a registry failure reported as plugin_not_in_registry; a release build honouring the
#          override; an install accepted with no registry.
import http.server, json, os, subprocess, sys, tempfile, threading, time, urllib.request, urllib.error

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "lib"))
from status_table import table                      # noqa: E402

REPO = os.path.abspath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", ".."))
DBG = os.path.join(REPO, "target", "debug", "agent-hub.exe")
REL = os.path.join(REPO, "target", "release", "agent-hub.exe")
reg = json.load(open(os.path.join(REPO, "registry.json"), encoding="utf-8"))
e0 = next(p for p in reg["plugins"] if p["id"] == "pi"); v0 = e0["versions"][0]
ART = {"url": v0["url"], "sha256": v0["sha256"], "id": "pi",
       "pluginType": "harness-adapter", "version": v0["version"], "size": v0["size"]}


def _call(base, tok, method, path, body=None, timeout=120):
    d = json.dumps(body).encode() if body is not None else None
    r = urllib.request.Request(base + path, data=d, method=method,
        headers={"authorization": "Bearer " + tok, "content-type": "application/json"})
    try:
        x = urllib.request.urlopen(r, timeout=timeout); return x.status, x.read().decode()
    except urllib.error.HTTPError as h:
        return h.code, h.read().decode()


def run(exe, env_extra, do_refresh=False, wait=False):
    dd = tempfile.mkdtemp(prefix="matrix-")
    env = {k: v for k, v in os.environ.items() if k not in ("AGENT_HUB_REGISTRY_FILE", "AGENT_HUB_REGISTRY_URL")}
    env.update({"AGENT_HUB_DATA_DIR": dd, "AGENT_HUB_ADDR": "127.0.0.1:0"})
    env.update(env_extra)
    p = subprocess.Popen([exe], env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    def ep():
        f = os.path.join(dd, "endpoint.json")
        for _ in range(300):
            if os.path.exists(f): return json.load(open(f))
            time.sleep(0.1)
        raise RuntimeError("no endpoint.json")
    try:
        e = ep(); base, tok = e["url"], e["token"]
        if do_refresh:
            _call(base, tok, "POST", "/v1/plugins/registry/refresh", timeout=60)
        st, body = _call(base, tok, "POST", "/v1/plugins", {"source": {"artifact": ART}})
        code = None
        try: code = json.loads(body).get("error")
        except Exception: pass
        state = None
        if wait and st < 400:
            for _ in range(1200):
                s, b = _call(base, tok, "GET", "/v1/plugins/pi")
                if s == 200:
                    state = json.loads(b).get("plugin", {}).get("state")
                    if state in ("ready", "failed"): break
                time.sleep(0.25)
        return st, code, state
    finally:
        p.terminate()
        try: p.wait(5)
        except Exception: p.kill()


# a local HTTP server that answers 200 with HTML (not a registry)
class _Html(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        b = b"<!doctype html><html>not a registry</html>"
        self.send_response(200); self.send_header("content-type", "text/html")
        self.send_header("content-length", str(len(b))); self.end_headers(); self.wfile.write(b)
    def log_message(self, *a): pass

class _Auth(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(401); self.end_headers(); self.wfile.write(b"unauthorized")
    def log_message(self, *a): pass


def srv(handler):
    s = http.server.HTTPServer(("127.0.0.1", 0), handler)
    threading.Thread(target=s.serve_forever, daemon=True).start()
    return s, f"http://127.0.0.1:{s.server_address[1]}/registry.json"

_regfile = os.path.join(REPO, "registry.json")
_badreg = os.path.join(tempfile.mkdtemp(prefix="badreg-"), "registry.json"); open(_badreg, "w").write("{ not json")
_h, _html = srv(_Html); _a, _auth = srv(_Auth)

results = []
def case(name, env, mock, expected, observed, ok):
    results.append((env, mock, name, expected, observed, ok))

# (1) official 404, no injection
st, code, _ = run(DBG, {})
case("404, no injection", "dev", False, "403 plugin_not_in_registry", f"{st} {code}", st == 403 and code == "plugin_not_in_registry")
# (2) dev + injected listed registry
st, code, state = run(DBG, {"AGENT_HUB_REGISTRY_FILE": _regfile}, wait=True)
case("injected listed registry", "dev", True, "202 -> ready", f"{st} {code or ''} -> {state}", st == 202 and state == "ready")
# (3) release + injection must be ignored
st, code, _ = run(REL, {"AGENT_HUB_REGISTRY_FILE": _regfile})
case("release + injection", "dev", True, "403 plugin_not_in_registry", f"{st} {code}", st == 403 and code == "plugin_not_in_registry")
# (4) dev + unparseable registry
st, code, _ = run(DBG, {"AGENT_HUB_REGISTRY_FILE": _badreg})
case("unparseable registry", "dev", True, "502 registry_unavailable", f"{st} {code}", st == 502 and code == "registry_unavailable")
# (5) release + no injection (= official 404)
st, code, _ = run(REL, {})
case("404, release no inject", "dev", False, "403 plugin_not_in_registry", f"{st} {code}", st == 403 and code == "plugin_not_in_registry")
# (6) dev + auth-required registry
st, code, _ = run(DBG, {"AGENT_HUB_REGISTRY_URL": _auth}, do_refresh=True)
case("registry needs auth (401)", "dev", True, "502 registry_unavailable", f"{st} {code}", st == 502 and code == "registry_unavailable")
# (7) dev + 200 but not a registry
st, code, _ = run(DBG, {"AGENT_HUB_REGISTRY_URL": _html}, do_refresh=True)
case("200 but not a registry", "dev", True, "502 registry_unavailable", f"{st} {code}", st == 502 and code == "registry_unavailable")

_h.shutdown(); _a.shutdown()

green, red = table("registry-injection matrix", "dev(+release binary)", True,
                   [(c, e, o, ok) for (_, _, c, e, o, ok) in results])
sys.exit(0 if red == 0 else 1)

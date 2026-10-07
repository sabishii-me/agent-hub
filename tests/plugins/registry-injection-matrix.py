import json, os, subprocess, tempfile, time, urllib.request, urllib.error
REPO=r"E:\AI\ideas\prts-hub"; DBG=os.path.join(REPO,"target","debug","agent-hub.exe"); REL=os.path.join(REPO,"target","release","agent-hub.exe")
reg=json.load(open(os.path.join(REPO,"registry.json")))
e0=next(p for p in reg["plugins"] if p["id"]=="pi"); v0=e0["versions"][0]
art0={"url":v0["url"],"sha256":v0["sha256"],"id":"pi","pluginType":"harness-adapter","version":v0["version"],"size":v0["size"]}

def run(exe, env_extra, art, wait=False):
    dd=tempfile.mkdtemp(prefix="spec-")
    env={k:val for k,val in os.environ.items() if k not in ("AGENT_HUB_REGISTRY_FILE","AGENT_HUB_REGISTRY_URL")}
    env.update({"AGENT_HUB_DATA_DIR":dd,"AGENT_HUB_ADDR":"127.0.0.1:0","AGENT_HUB_CONTRACT_DIR":os.path.join(REPO,"contract")})
    env.update(env_extra)
    p=subprocess.Popen([exe],env=env,stdout=subprocess.PIPE,stderr=subprocess.STDOUT)
    def ep():
        f=os.path.join(dd,"endpoint.json")
        for _ in range(300):
            if os.path.exists(f):return json.load(open(f))
            time.sleep(0.1)
        raise RuntimeError("no endpoint")
    try:
        e=ep(); base=e["url"]; tok=e["token"]
        def call(m,path,body=None):
            d=json.dumps(body).encode() if body is not None else None
            r=urllib.request.Request(base+path,data=d,method=m,headers={"authorization":"Bearer "+tok,"content-type":"application/json"})
            try:
                x=urllib.request.urlopen(r,timeout=120);return x.status,x.read().decode()
            except urllib.error.HTTPError as h:return h.code,h.read().decode()
        cat=call("GET","/v1/plugins/catalog")
        st,b=call("POST","/v1/plugins",{"source":{"artifact":art}})
        state=None
        if wait and st<400:
            for _ in range(1200):
                s,bb=call("GET","/v1/plugins/pi")
                if s==200:
                    state=json.loads(bb).get("plugin",{}).get("state")
                    if state in ("ready","failed"):break
                time.sleep(0.25)
        return cat,st,b[:220],state
    finally:
        p.terminate()
        try:p.wait(5)
        except Exception:p.kill()
    return

print("=== 1. official 404, NO injection (debug) -> any install must FAIL ===")
print(run(DBG, {}, art0))
print()
print("=== 2. DEV + inject registry (AGENT_HUB_REGISTRY_FILE) -> the listed plugin INSTALLS ===")
print(run(DBG, {"AGENT_HUB_REGISTRY_FILE": os.path.join(REPO,"registry.json")}, art0, wait=True))
print()
print("=== 3. RELEASE + inject registry -> must NOT install ===")
print(run(REL, {"AGENT_HUB_REGISTRY_FILE": os.path.join(REPO,"registry.json")}, art0))
print()
print("=== 4. DEV + inject an UNPARSEABLE registry -> a correct error code ===")
bad=os.path.join(tempfile.mkdtemp(prefix="badreg-"),"registry.json"); open(bad,"w").write("{ this is not json")
print(run(DBG, {"AGENT_HUB_REGISTRY_FILE": bad}, art0))
print()
print("=== 5. RELEASE cannot inject -> same state as when there is no registry ===")
print(run(REL, {"AGENT_HUB_REGISTRY_URL":"http://127.0.0.1:1/nope", "AGENT_HUB_REGISTRY_FILE": os.path.join(REPO,"registry.json")}, art0))

# === 6. DEV + a registry URL that REQUIRES AUTH (401) -> refresh fails; install must answer
#        registry_unavailable (502), the SAME code as an unparseable registry - the REGISTRY is
#        the problem, not the source. ===
import http.server, threading
class _Auth(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(401); self.end_headers(); self.wfile.write(b"unauthorized")
    def log_message(self,*a): pass
_srv = http.server.HTTPServer(("127.0.0.1",0), _Auth)
threading.Thread(target=_srv.serve_forever, daemon=True).start()
_authurl = f"http://127.0.0.1:{_srv.server_address[1]}/registry.json"

def run_refresh_then_install(exe, url, art):
    dd=tempfile.mkdtemp(prefix="auth-")
    env={k:val for k,val in os.environ.items() if k not in ("AGENT_HUB_REGISTRY_FILE","AGENT_HUB_REGISTRY_URL")}
    env.update({"AGENT_HUB_DATA_DIR":dd,"AGENT_HUB_ADDR":"127.0.0.1:0","AGENT_HUB_CONTRACT_DIR":os.path.join(REPO,"contract"),
                "AGENT_HUB_REGISTRY_URL":url})
    p=subprocess.Popen([exe],env=env,stdout=subprocess.PIPE,stderr=subprocess.STDOUT)
    def ep():
        f=os.path.join(dd,"endpoint.json")
        for _ in range(300):
            if os.path.exists(f):return json.load(open(f))
            time.sleep(0.1)
        raise RuntimeError("no endpoint")
    try:
        e=ep(); base=e["url"]; tok=e["token"]
        def call(m,path,body=None):
            d=json.dumps(body).encode() if body is not None else None
            r=urllib.request.Request(base+path,data=d,method=m,headers={"authorization":"Bearer "+tok,"content-type":"application/json"})
            try:
                x=urllib.request.urlopen(r,timeout=60);return x.status,x.read().decode()
            except urllib.error.HTTPError as h:return h.code,h.read().decode()
        ref=call("POST","/v1/plugins/registry/refresh")
        inst=call("POST","/v1/plugins",{"source":{"artifact":art}})
        return ref, inst
    finally:
        p.terminate()
        try:p.wait(5)
        except Exception:p.kill()

print()
print("=== 6. DEV + registry URL requiring auth (401) -> install must be registry_unavailable ===")
ref, inst = run_refresh_then_install(DBG, _authurl, art0)
print("refresh ->", ref)
print("install ->", inst)
_srv.shutdown()

# === 7. DEV + a registry URL that answers 200 but is NOT a registry (e.g. an HTML page) ->
#        refresh fails to parse; install must be registry_unavailable (502), NOT
#        plugin_not_in_registry. A reachable address is not a registry. ===
class _Html(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        body=b"<!doctype html><html><body>not a registry</body></html>"
        self.send_response(200); self.send_header("content-type","text/html")
        self.send_header("content-length",str(len(body))); self.end_headers(); self.wfile.write(body)
    def log_message(self,*a): pass
_srv2 = http.server.HTTPServer(("127.0.0.1",0), _Html)
threading.Thread(target=_srv2.serve_forever, daemon=True).start()
_htmlurl = f"http://127.0.0.1:{_srv2.server_address[1]}/registry.json"

print()
print("=== 7. DEV + registry URL answering 200 but NOT a registry -> install must be registry_unavailable ===")
ref, inst = run_refresh_then_install(DBG, _htmlurl, art0)
print("refresh ->", ref)
print("install ->", inst)
_srv2.shutdown()

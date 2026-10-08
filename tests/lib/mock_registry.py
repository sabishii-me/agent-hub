"""MOCK registry server (test-only). NOT the product registry.

Purpose: a test needs a registry the hub can fetch, without publishing the OFFICIAL one
(which is a release asset and does not exist yet). This MOCK serves a REAL registry - real
plugin zips, real sha256 - over loopback HTTP, so a DEV hub (AGENT_HUB_REGISTRY_URL /
AGENT_HUB_REGISTRY_FILE) can refresh and install for real. The hub, the download, the digest
check and the landing are the product's; ONLY the registry ADDRESS is a mock, and every name
here says MOCK so it can never be read as the official registry.

Two modes:
  - no-auth: anyone GETs /registry.json.
  - token:  GET requires `Authorization: Bearer <token>`; without it -> 401. (The hub's
            registry fetch does NOT send a token today - see
            docs/needReview/test-gap-registry-and-install.md.)
"""
import hashlib
import http.server
import json
import os
import threading
import time


def _zip_bytes(path):
    with open(path, "rb") as f:
        return f.read()


class MockRegistry:
    """A loopback MOCK registry built from real plugin directories.

    Add a plugin with `add(plugin_dir)`; it packs the dir (excludes runtime/.git/node_modules),
    hashes the zip, and records a release entry. Call `url()` for the address to inject.
    """

    EXCLUDED = {".git", ".github", "node_modules", "runtime", ".venv", "__pycache__", ".cache", ".DS_Store"}

    def __init__(self, token=None, delay=0.0):
        self.token = token
        self.delay = delay          # seconds to stall each response - makes `installing` observable
        self.entries = {}          # id -> entry
        self._zips = {}            # filename -> bytes
        self.hits = []             # every path GETed, in order (proves a real HTTP fetch)
        self._lock = threading.Lock()
        self._server = None
        self._thread = None

    def _collect(self, root, prefix=""):
        out = []
        for name in sorted(os.listdir(root)):
            rel = f"{prefix}{name}"
            if rel.split("/")[0] in self.EXCLUDED:
                continue
            full = os.path.join(root, name)
            if os.path.isdir(full):
                out.extend(self._collect(full, rel + "/"))
            elif os.path.isfile(full):
                out.append((rel, full))
        return out

    def add(self, plugin_dir):
        """Pack `plugin_dir` into a zip and record it as a MOCK registry release."""
        manifest = json.load(open(os.path.join(plugin_dir, "manifest.json"), encoding="utf-8"))
        pid, ptype = manifest["id"], manifest["pluginType"]
        version = manifest.get("version")
        if not version:
            pkg = os.path.join(plugin_dir, "package.json")
            version = json.load(open(pkg, encoding="utf-8"))["version"] if os.path.exists(pkg) else None
        assert version, f"{plugin_dir} has no version"
        fname = f"{ptype}-{pid}-{version}.zip"
        import io, zipfile
        zbuf_io = io.BytesIO()
        with zipfile.ZipFile(zbuf_io, "w", zipfile.ZIP_DEFLATED) as z:
            for rel, full in self._collect(plugin_dir):
                z.write(full, rel)
        zbuf = zbuf_io.getvalue()
        self._zips[fname] = zbuf
        sha = hashlib.sha256(zbuf).hexdigest()
        self.entries[pid] = {
            "id": pid, "pluginType": ptype, "name": manifest.get("name", pid),
            "versions": [{"version": version, "sha256": sha, "size": len(zbuf), "_file": fname}],
        }
        return self.entries[pid]

    def registry_json(self):
        plugins = []
        for e in self.entries.values():
            e2 = dict(e)
            e2["versions"] = [{k: v for k, v in vv.items() if k != "_file"} |
                              {"url": f"http://127.0.0.1:{self._server.server_address[1]}/{vv['_file']}"}
                              for vv in e["versions"]]
            plugins.append(e2)
        return {"schema": 1, "note": "MOCK registry (test-only)", "plugins": plugins}

    def start(self):
        outer = self

        class H(http.server.BaseHTTPRequestHandler):
            def _auth_ok(self):
                if outer.token is None:
                    return True
                return self.headers.get("Authorization") == f"Bearer {outer.token}"

            def do_GET(self):
                if not self._auth_ok():
                    self.send_response(401); self.end_headers(); self.wfile.write(b"unauthorized"); return
                name = self.path.lstrip("/")
                # Record the hit so a test can prove the hub REALLY fetched over HTTP (not a copy).
                with outer._lock:
                    outer.hits.append(name)
                if name == "registry.json":
                    body = json.dumps(outer.registry_json()).encode()
                    ctype = "application/json"
                elif name in outer._zips:
                    body = outer._zips[name]; ctype = "application/zip"
                else:
                    self.send_response(404); self.end_headers(); return
                self.send_response(200)
                self.send_header("content-type", ctype)
                self.send_header("content-length", str(len(body)))
                self.end_headers()
                if outer.delay:
                    time.sleep(outer.delay)
                self.wfile.write(body)

            def log_message(self, *a):
                pass

        self._server = http.server.HTTPServer(("127.0.0.1", 0), H)
        self._thread = threading.Thread(target=self._server.serve_forever, daemon=True)
        self._thread.start()
        return self

    def url(self):
        return f"http://127.0.0.1:{self._server.server_address[1]}/registry.json"

    def stop(self):
        if self._server:
            self._server.shutdown()

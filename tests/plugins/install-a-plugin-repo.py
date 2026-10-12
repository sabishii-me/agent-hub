# TC-PLUGIN-REPO-INSTALL: install THIS plugin repo's own (local, unpublished) build with a real hub
#
# The purpose (owner): prove a plugin's OWN version is INSTALLABLE, from inside its OWN repository,
# by driving a REAL hub the way a USER does - refresh the registry, then POST /v1/plugins - and
# requiring the install to complete (the runtime materialised, state `ready`). NOTHING here is
# faked: the artifact is this repo's real build (a zip of the repo, real sha256), served by a
# loopback registry, downloaded by the hub over HTTP.
#
# Scope (owner, this round): INSTALL ONLY. Running the harness (a session, a turn) is out of scope
# and comes later. Do not widen this.
#
# Run from a plugin repo (or point PLUGIN_REPO at one):
#     PLUGIN_REPO=<plugin repo> python <this file>
# The hub's data dir lives INSIDE the repo (`.hub-isolated/`), so the run is isolated to that repo
# and leaves nothing behind. The registry the hub is given is this repo's LOCAL build, clearly a
# stand-in for the CI/formal release artifact (which does not exist yet - the registry is treated
# as unpublished this round).
import json
import os
import shutil
import subprocess
import sys
import time
import urllib.request

HUB_REPO = os.path.abspath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", ".."))
sys.path.insert(0, os.path.join(HUB_REPO, "tests", "lib"))
from mock_registry import MockRegistry  # noqa: E402
from tally import Tally  # noqa: E402

PLUGIN_REPO = os.environ.get("PLUGIN_REPO")
if not PLUGIN_REPO:
    # default: the repo this file is checked out beside, if the caller did not name one
    PLUGIN_REPO = os.path.abspath(os.path.join(HUB_REPO, "..", "prts-harness-pi"))
PLUGIN_REPO = os.path.abspath(PLUGIN_REPO)

_SUFFIX = ".exe" if os.name == "nt" else ""
PROFILE = os.environ.get("AGENT_HUB_PROFILE", "debug").strip() or "debug"
EXE = os.path.join(HUB_REPO, "target", PROFILE, "agent-hub" + _SUFFIX)

t = Tally("plugin-repo/install")


def endpoint_of(data_dir, timeout=30):
    p = os.path.join(data_dir, "endpoint.json")
    end = time.time() + timeout
    while time.time() < end:
        if os.path.exists(p):
            try:
                with open(p, encoding="utf-8") as f:
                    return json.load(f)
            except Exception:
                pass
        time.sleep(0.1)
    raise RuntimeError("the hub wrote no endpoint.json")


class RepoHub:
    """A real hub whose DATA DIR is inside the plugin repo - the isolated run the owner asked for."""

    def __init__(self, repo):
        self.repo = repo
        self.dir = os.path.join(repo, ".hub-isolated")
        shutil.rmtree(self.dir, ignore_errors=True)
        os.makedirs(self.dir, exist_ok=True)
        self.plugins = os.path.join(self.dir, "plugins")
        os.makedirs(self.plugins, exist_ok=True)
        self.mock = None
        self.child = None
        self._log = None

    def start(self):
        if not os.path.exists(EXE):
            raise RuntimeError(f"the hub binary is missing: {EXE} (cargo build --workspace)")
        # The registry the hub is given: THIS repo's local build (real zip + real sha256). Marked
        # as a stand-in for the CI/formal release artifact.
        self.mock = MockRegistry()
        self.mock.add(self.repo)
        self.mock.start()
        env = dict(os.environ)
        env.update({
            "AGENT_HUB_DATA_DIR": self.dir,
            "AGENT_HUB_PLUGINS_DIR": self.plugins,
            "AGENT_HUB_ADDR": "127.0.0.1:0",
            "AGENT_HUB_CONTRACT_DIR": os.path.join(HUB_REPO, "contract"),
            "AGENT_HUB_REGISTRY_URL": self.mock.url(),
        })
        self._log = open(os.path.join(self.dir, "hub-stdout.log"), "w", encoding="utf-8")
        self.child = subprocess.Popen(
            [EXE], env=env, stdout=self._log, stderr=self._log,
            creationflags=(0x08000000 if os.name == "nt" else 0))
        ep = endpoint_of(self.dir)
        self.base = ep["url"] if isinstance(ep, dict) else ep
        self.token = ep["token"] if isinstance(ep, dict) else None
        return self

    def req(self, method, path, body=None):
        url = self.base + path
        data = json.dumps(body).encode() if body is not None else None
        r = urllib.request.Request(url, data=data, method=method)
        r.add_header("content-type", "application/json")
        if self.token:
            r.add_header("authorization", "Bearer " + self.token)
        try:
            with urllib.request.urlopen(r, timeout=120) as resp:
                return resp.status, json.loads(resp.read().decode() or "{}")
        except urllib.error.HTTPError as e:
            raw = e.read().decode()
            try:
                return e.code, json.loads(raw or "{}")
            except Exception:
                return e.code, {"raw": raw}

    def get(self, p):
        return self.req("GET", p)

    def post(self, p, body=None):
        return self.req("POST", p, body)

    def cleanup(self):
        if self.child and self.child.poll() is None:
            self.child.terminate()
            try:
                self.child.wait(timeout=15)
            except Exception:
                subprocess.run(["taskkill", "/PID", str(self.child.pid), "/T", "/F"],
                               stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        if self._log:
            self._log.close()
        if self.mock:
            try:
                self.mock.stop()
            except Exception:
                pass
        shutil.rmtree(self.dir, ignore_errors=True)

    def log(self):
        try:
            with open(os.path.join(self.dir, "hub-stdout.log"), encoding="utf-8", errors="replace") as f:
                return f.read()
        except Exception:
            return "(no log)"


# --- the manifest's declared id / version (the plugin's OWN identity) --------------------------
with open(os.path.join(PLUGIN_REPO, "manifest.json"), encoding="utf-8") as f:
    MANIFEST = json.load(f)
PID = MANIFEST["id"]
VERSION = MANIFEST.get("version")

hub = RepoHub(PLUGIN_REPO)
try:
    hub.start()
    t.check(True, f"a real hub started with its data dir INSIDE {os.path.basename(PLUGIN_REPO)}",
            f"data_dir={hub.dir}")

    # The hub reads the registry we gave it (the repo's LOCAL build).
    s, cat = hub.post("/v1/plugins/registry/refresh")
    t.check(s < 300, "the hub refreshed its registry (this repo's local build)",
            f"status={s} body={json.dumps(cat)[:160]}")

    s, cat = hub.get("/v1/plugins/catalog")
    entry = next((p for p in (cat.get("plugins") or []) if p.get("id") == PID), None)
    t.check(entry is not None, f"the catalog lists `{PID}`", f"catalog={cat}")
    v = (entry or {}).get("versions", [{}])[0] if entry else {}
    t.check(v.get("version") == VERSION, f"the catalog lists version {VERSION} (this repo's build)",
            f"listed={v.get('version')}")

    # THE USER OPERATION: install. Install PREPARES the runtime (one command).
    artifact = {"url": v["url"], "sha256": v["sha256"], "id": PID,
                "pluginType": entry.get("pluginType"), "version": v["version"], "size": v.get("size")}
    t0 = time.time()
    s, body = hub.post("/v1/plugins", {"source": {"artifact": artifact}})
    t.check(s == 202, f"POST /v1/plugins accepts the install (202)", f"status={s} body={json.dumps(body)[:160]}")

    # Wait for the terminal state; `failed` is a FAILURE, reported with the reason (never skipped).
    state = None
    while time.time() - t0 < 1800:
        s, body = hub.get(f"/v1/plugins/{PID}")
        state = None if s == 404 else (body.get("plugin") or {}).get("state")
        if state in ("ready", "failed"):
            break
        time.sleep(0.25)
    took = time.time() - t0
    t.check(state != "failed", f"`{PID}` install did not FAIL", f"state={state} body={json.dumps(body)[:240]}")
    t.check(state == "ready", f"`{PID}` reaches `ready` (installed + runtime prepared)", f"state={state}")

    s, body = hub.get(f"/v1/plugins/{PID}")
    pj = (body.get("plugin") or {})
    t.check(pj.get("runtimeReady") is True, "the runtime is on disk (runtimeReady true)",
            f"runtimeReady={pj.get('runtimeReady')} runtime={json.dumps(pj.get('runtime'))}")
    rt = (pj.get("runtime") or {})
    target = rt.get("target")
    t.check(bool(target) and os.path.exists(target),
            "the declared runtime command EXISTS on disk", f"target={target}")
    t.check(all(z in hub.mock.hits for z in [v["url"].rsplit("/", 1)[-1]]),
            "REAL download: the hub GET the artifact over HTTP from the registry",
            f"hits={hub.mock.hits}")

    print(f"  ({PID} {VERSION}: installed in {took:.1f}s, runtime at {target})")
finally:
    ok = t.done()
    hub.cleanup()
sys.exit(0 if ok else 1)

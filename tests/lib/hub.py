"""Drive a REAL agent-hub process over its REAL surfaces. Nothing is faked.

It spawns the built binary, waits for `endpoint.json`, speaks HTTP with the bearer, and
kills with a process-tree taskkill (no cleanup, like a power cut). It imports NOTHING of the
hub and reads NONE of its database: the only inputs are the process, its stdout, its
endpoint.json, and the HTTP surface - the same things a user has.
"""
import json
import os
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.abspath(os.path.join(HERE, "..", ".."))
EXE = os.path.join(REPO, "target", "debug", "agent-hub.exe" if os.name == "nt" else "agent-hub")


def hub_sha():
    try:
        return subprocess.check_output(["git", "-C", REPO, "rev-parse", "--short", "HEAD"], text=True).strip()
    except Exception:
        return "?"


class Hub:
    """One real hub process on its own isolated data dir."""

    def __init__(self, plugins_src=None, env=None, data_dir=None):
        self.dir = data_dir or tempfile.mkdtemp(prefix="hub-test-")
        self.plugins = os.path.join(self.dir, "plugins")
        os.makedirs(self.plugins, exist_ok=True)
        # NO backdoor: a plugin source is NOT copied into the plugins root. It is
        # INSTALLED through the real /v1 surface after start() (see install_plugins),
        # so every test that uses a plugin proves the INSTALL path. Copying the dir in
        # would make the hub's own plugins root pre-populated and test nothing.
        if plugins_src is None:
            self._plugin_sources = []
        elif isinstance(plugins_src, (list, tuple)):
            self._plugin_sources = list(plugins_src)
        else:
            self._plugin_sources = [plugins_src]
        self._installed = set()
        self.extra_env = env or {}
        self.child = None
        self.ep = None
        # Resources this test registered through /v1. cleanup() deletes them FIRST
        # (the same path a real deployment uses) so the OS keychain entries they own
        # are removed, not leaked. A leaked credential is a real side effect on the
        # host, not test scratch.
        self._providers = set()
        self._connections = set()
        self._log = open(os.path.join(self.dir, "hub-stdout.log"), "a", encoding="utf-8")

    @staticmethod
    def _plugin_id(d):
        try:
            with open(os.path.join(d, "manifest.json"), encoding="utf-8") as f:
                return json.load(f).get("id") or os.path.basename(d.rstrip("/\\"))
        except Exception:
            return os.path.basename(d.rstrip("/\\"))

    def start(self):
        if not os.path.exists(EXE):
            raise RuntimeError(f"the hub binary is missing: {EXE} (cargo build --workspace)")
        env = dict(os.environ)
        env.update({
            "AGENT_HUB_DATA_DIR": self.dir,
            "AGENT_HUB_PLUGINS_DIR": self.plugins,
            "AGENT_HUB_ADDR": "127.0.0.1:0",
            "AGENT_HUB_CONTRACT_DIR": os.path.join(REPO, "contract"),
        })
        env.update(self.extra_env)
        self.child = subprocess.Popen([EXE], env=env, stdout=self._log, stderr=self._log, creationflags=(0x08000000 if os.name == "nt" else 0))
        self.ep = self._await_endpoint()
        self.install_plugins()
        return self

    def install_plugins(self):
        """Install every requested plugin THROUGH /v1 as a REAL ARTIFACT: the published
        release zip named in the repository's `registry.json` (url + sha256 + size), and
        then `prepare` so its runtime is materialised the way the hub really does it.

        There is NO copy-the-dir shortcut any more. A `plugins_src` value is a plugin ID
        (e.g. "pi"); the artifact comes from registry.json, NOT from a local path. This is
        the only install path a real deployment has."""
        if not self._plugin_sources:
            return
        reg = self._registry()
        for spec in self._plugin_sources:
            # A source may be a plugin ID ("pi") or a path to the plugin repo (legacy
            # call sites pass PI_PLUGIN_DIR). Either way the ID is what the registry is
            # keyed by; a directory is read for its manifest id, NEVER copied.
            if isinstance(spec, str) and os.path.isdir(spec):
                pid = self._plugin_id(spec)
            else:
                pid = str(spec)
            if pid in self._installed:
                continue
            entry = next((p for p in reg.get("plugins", []) if p.get("id") == pid), None)
            if entry is None:
                raise RuntimeError(f"no registry entry for plugin `{pid}`")
            v = (entry.get("versions") or [None])[0]
            if not v:
                raise RuntimeError(f"registry entry `{pid}` names no versions")
            artifact = {"url": v["url"], "sha256": v["sha256"], "id": pid,
                        "pluginType": entry.get("pluginType"), "version": v["version"],
                        "size": v.get("size")}
            r = self.post("/v1/plugins", {"source": {"artifact": artifact}}, key="install-" + pid)
            if isinstance(r.get("json"), dict) and r["json"].get("error"):
                raise RuntimeError(f"install {pid} failed: {r['status']} {r['text'][:200]}")
            self._await_plugin(pid, "ready")
            # Materialise the runtime (the zip is adapter-only; the runtime is fetched by
            # the adapter). A real install ends with a runnable runtime.
            pr = self.post(f"/v1/plugins/{pid}/prepare")
            if isinstance(pr.get("json"), dict) and pr["json"].get("error"):
                raise RuntimeError(f"prepare {pid} failed: {pr['status']} {pr['text'][:200]}")
            self._await_runtime(pid)
            self._installed.add(pid)

    def _registry(self):
        path = os.environ.get("AGENT_HUB_REGISTRY_FILE", os.path.join(REPO, "registry.json"))
        with open(path, encoding="utf-8") as f:
            return json.load(f)

    def _await_plugin(self, pid, want):
        for _ in range(1920):
            g = self.get(f"/v1/plugins/{pid}")
            st = None if g["status"] == 404 else (g["json"] or {}).get("plugin", {}).get("state")
            if st == want:
                return st
            if st == "failed":
                raise RuntimeError(f"plugin {pid} FAILED: {g['text'][:220]}")
            time.sleep(0.25)
        raise RuntimeError(f"plugin {pid} did not reach {want}: {self.get(f'/v1/plugins/{pid}')['text'][:220]}")

    def _await_runtime(self, pid):
        # `prepare` answers ready; confirm the manifest's command is on disk (the hub's
        # own `runtimeReady` is a filesystem fact).
        for _ in range(1920):
            g = self.get(f"/v1/plugins/{pid}")
            pj = (g["json"] or {}).get("plugin", {})
            if pj.get("runtimeReady"):
                return True
            if pj.get("state") == "failed":
                raise RuntimeError(f"runtime for {pid} FAILED: {g['text'][:220]}")
            time.sleep(0.25)
        raise RuntimeError(f"runtime for {pid} never became ready: {self.get(f'/v1/plugins/{pid}')['text'][:220]}")

    def _await_endpoint(self, timeout=30.0):
        epfile = os.path.join(self.dir, "endpoint.json")
        end = time.time() + timeout
        while time.time() < end:
            try:
                with open(epfile, encoding="utf-8") as f:
                    ep = json.load(f)
                if ep.get("pid") == self.child.pid:
                    return ep
            except Exception:
                pass
            time.sleep(0.1)
        raise RuntimeError(f"no endpoint.json for pid {self.child.pid}; log:\n{self.log()}")

    @property
    def base(self):
        return self.ep["url"]

    @property
    def token(self):
        return self.ep["token"]

    # --- HTTP -------------------------------------------------------------
    def req(self, method, path, body=None, key=None, timeout=60):
        data = None if body is None else json.dumps(body).encode()
        r = urllib.request.Request(self.base + path, data=data, method=method)
        r.add_header("authorization", "Bearer " + self.token)
        r.add_header("content-type", "application/json")
        if key:
            r.add_header("idempotency-key", key)
        try:
            with urllib.request.urlopen(r, timeout=timeout) as resp:
                raw = resp.read().decode()
                status = resp.status
                headers = dict(resp.headers)
        except urllib.error.HTTPError as e:
            raw = e.read().decode()
            status = e.code
            headers = dict(e.headers)
        try:
            parsed = json.loads(raw)
        except Exception:
            parsed = None
        return {"status": status, "json": parsed, "text": raw, "headers": headers}

    def get(self, p, **k):
        return self.req("GET", p, **k)

    def post(self, p, body=None, **k):
        r = self.req("POST", p, body, **k)
        self._track(p, body, r)
        return r

    def _track(self, path, body, r):
        # Remember what this run created so cleanup() can delete it via /v1 and the
        # hub can release the keychain entry it owns.
        if r.get("status") not in (200, 201, 202) or not isinstance(body, dict):
            return
        rid = body.get("id")
        if not isinstance(rid, str):
            return
        if path == "/v1/model-providers":
            self._providers.add(rid)
        elif path == "/v1/connections":
            self._connections.add(rid)

    def patch(self, p, body=None, **k):
        return self.req("PATCH", p, body, **k)

    def put(self, p, body=None, **k):
        return self.req("PUT", p, body, **k)

    def delete(self, p, **k):
        return self.req("DELETE", p, **k)

    def sse_open(self, last_event_id=None, timeout=30):
        """Open GET /v1/events as a REAL SSE stream and return a reader with
        `.events` (parsed so far) and `.wait(n, timeout)` to block until n frames
        arrived. A background thread reads the socket. Frames are parsed into
        {id, event, data} - the hub's real framing, not a fake."""
        import threading
        url = self.base + "/v1/events" + (f"?lastEventId={last_event_id}" if last_event_id is not None else "")
        req = urllib.request.Request(url)
        req.add_header("authorization", "Bearer " + self.token)
        if last_event_id is not None:
            req.add_header("last-event-id", str(last_event_id))
        reader = _SSEReader(req, timeout)
        reader.start()
        return reader

    def log(self):
        try:
            with open(os.path.join(self.dir, "hub-stdout.log"), encoding="utf-8", errors="replace") as f:
                return f.read()
        except Exception:
            return "(no log)"

    # --- lifecycle --------------------------------------------------------
    def kill(self):
        """Kill the process tree with no cleanup - a power cut, not a shutdown."""
        if self.child and self.child.poll() is None:
            if os.name == "nt":
                subprocess.run(["taskkill", "/PID", str(self.child.pid), "/T", "/F"],
                               stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            else:
                self.child.send_signal(signal.SIGKILL)
            try:
                self.child.wait(timeout=10)
            except Exception:
                pass
        time.sleep(0.3)

    def stop(self):
        if self.child and self.child.poll() is None:
            self.child.terminate()
            try:
                self.child.wait(timeout=10)
            except Exception:
                self.kill()
        time.sleep(0.2)

    def cleanup(self):
        # Delete every credential-bearing resource THIS run created, through the real
        # /v1 API, BEFORE the data dir goes away: that is how a provider/connection's
        # keychain entry is released (the store is keyed by the instance id in the
        # data dir). Without this, each run leaks an OS credential.
        for pid in list(self._providers):
            try: self.delete(f"/v1/model-providers/{pid}")
            except Exception: pass
        for cid in list(self._connections):
            try: self.delete(f"/v1/connections/{cid}")
            except Exception: pass
        self.stop()
        try:
            self._log.close()
        except Exception:
            pass
        shutil.rmtree(self.dir, ignore_errors=True)

    # --- convenience ------------------------------------------------------
    def create_session(self, harness, **extra):
        body = {"harnessId": harness}
        body.update(extra)
        key = extra.pop("idempotencyKey", None)
        r = self.post("/v1/sessions", body, key=key)
        return r

    def wait_status(self, sid, want, tries=480, interval=0.25):
        last = ""
        for _ in range(tries):
            g = self.get(f"/v1/sessions/{sid}")
            last = (g["json"] or {}).get("session", {}).get("status", "")
            if last == want:
                return g["json"]["session"]
            if last in ("failed", "starting_failed", "needs-repair") and want != last:
                return g["json"]["session"]
            time.sleep(interval)
        return {"status": last}


def turn_state(hub, sid, tid):
    """The state string of one turn, read through the REAL surface, or None."""
    g = hub.get(f"/v1/sessions/{sid}/turns")
    rows = (g["json"] or {}).get("turns", [])
    row = next((x for x in rows if x.get("id") == tid), None)
    return row.get("state") if row else None


def wait_turn(hub, sid, tid, states, tries=240, interval=0.25):
    """Poll ONE turn until its state is in `states`. Bounded; returns the turn row
    (or the last seen row) so a caller asserts on the OBSERVED state, never a wait."""
    last = None
    for _ in range(tries):
        g = hub.get(f"/v1/sessions/{sid}/turns")
        rows = (g["json"] or {}).get("turns", [])
        last = next((x for x in rows if x.get("id") == tid), None)
        if last and last.get("state") in states:
            return last
        time.sleep(interval)
    return last or {}


def register_provider(hub, pid="p"):
    """Register the REAL provider from ~/.pi/agent/models.json. Returns its identity
    dict, or None when the host has none (a missing real dependency FAILS, never
    skips)."""
    prov = real_provider()
    if not prov:
        return None
    hub.post("/v1/model-providers", {"id": pid, "url": prov["url"], "api": prov["api"], "token": prov["token"]})
    return {"id": pid, **prov}


def real_provider(env_var="HOME_JP_PROD_JSON"):
    """The REAL provider from ~/.pi/agent/models.json (HOME-JP-prod). None when absent."""
    import pathlib
    f = pathlib.Path(os.path.expanduser("~/.pi/agent/models.json"))
    if not f.exists():
        return None
    try:
        cfg = json.loads(f.read_text(encoding="utf-8"))
    except Exception:
        return None
    p = (cfg.get("providers") or {}).get("HOME-JP-prod")
    if not p or not p.get("baseUrl") or not p.get("apiKey"):
        return None
    return {"url": p["baseUrl"], "api": p.get("api"), "token": p["apiKey"]}


def child_processes(parent_pid):
    """The PIDs of the hub's direct children (its adapter processes), by parent pid.

    Real fault injection without faking the adapter: kill the REAL adapter child and watch
    the hub. Uses PowerShell CIM (wmic is deprecated)."""
    out = []
    try:
        ps = ("Get-CimInstance Win32_Process | "
              "Where-Object { $_.ParentProcessId -eq %d } | "
              "Select-Object -Property ProcessId,Name | "
              "ForEach-Object { \"$($_.ProcessId) $($_.Name)\" }") % parent_pid
        txt = subprocess.check_output(["powershell", "-NoProfile", "-Command", ps],
                                      text=True, stderr=subprocess.DEVNULL, timeout=30)
        for line in txt.splitlines():
            line = line.strip()
            if not line:
                continue
            pid_s, _, name = line.partition(" ")
            if pid_s.isdigit():
                out.append((int(pid_s), name))
    except Exception:
        pass
    return out


def all_descendants(root_pid):
    """Every descendant (pid, name) under root_pid, by walking the process tree. Real
    fault injection: reach the pi runtime node under the adapter, not just the hub's
    direct child."""
    ps = ("Get-CimInstance Win32_Process | "
          "Select-Object -Property ProcessId,ParentProcessId,Name | "
          "ForEach-Object { \"$($_.ProcessId) $($_.ParentProcessId) $($_.Name)\" }")
    try:
        txt = subprocess.check_output(["powershell", "-NoProfile", "-Command", ps],
                                      text=True, stderr=subprocess.DEVNULL, timeout=30)
    except Exception:
        return []
    rows = []
    for line in txt.splitlines():
        parts = line.strip().split()
        if len(parts) == 3 and parts[0].isdigit() and parts[1].isdigit():
            rows.append((int(parts[0]), int(parts[1]), parts[2]))
    # walk from root
    want = {root_pid}
    out = []
    changed = True
    while changed:
        changed = False
        for pid, ppid, name in rows:
            if ppid in want and pid not in want:
                want.add(pid)
                out.append((pid, name))
                changed = True
    return out


def kill_pid(pid):
    if os.name == "nt":
        subprocess.run(["taskkill", "/PID", str(pid), "/T", "/F"],
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    else:
        try:
            os.kill(pid, signal.SIGKILL)
        except Exception:
            pass


class _SSEReader:
    """A real SSE reader over urllib: a thread reads frames into a list; the test
    waits for N of them. Parses the hub's own framing (`id:`, `event:`, `data:`)."""
    def __init__(self, req, timeout):
        import threading
        self._req = req
        self._timeout = timeout
        self._resp = None
        self._err = None
        self._lock = threading.Lock()
        self._event = threading.Event()
        self.events = []
        self._t = threading.Thread(target=self._run, daemon=True)

    def start(self):
        self._t.start()
        # Wait until the response is open (or errored) so a caller sees a failure.
        deadline = time.time() + self._timeout
        while self._resp is None and self._err is None and time.time() < deadline:
            time.sleep(0.02)
        if self._resp is None and self._err is None:
            raise RuntimeError('SSE stream did not open')

    def _run(self):
        try:
            self._resp = urllib.request.urlopen(self._req, timeout=self._timeout)
            cur = {}
            for raw in self._resp:
                line = raw.decode('utf-8').rstrip(chr(13)+chr(10))
                if line == '':
                    if cur:
                        with self._lock:
                            self.events.append(cur)
                        self._event.set()
                        cur = {}
                    continue
                if line.startswith(':'):
                    continue
                field, _, val = line.partition(':')
                val = val[1:] if val.startswith(' ') else val
                cur[field] = val
        except Exception as e:
            self._err = e
        finally:
            self._event.set()

    def wait(self, n, timeout=20):
        end = time.time() + timeout
        while time.time() < end:
            with self._lock:
                if len(self.events) >= n:
                    return True
            time.sleep(0.05)
        return False

    def close(self):
        try:
            if self._resp is not None:
                self._resp.close()
        except Exception:
            pass

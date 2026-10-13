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
import atexit


HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.abspath(os.path.join(HERE, "..", ".."))
_SUFFIX = ".exe" if os.name == "nt" else ""
# The binary to run. Default = the DEV build (tests set it, or the suite runs as before).
# AGENT_HUB_BIN=release selects the RELEASE build, whose registry address is FIXED at build
# time (no env override exists) - that is what the release-build tests must exercise.
_PROFILE = os.environ.get("AGENT_HUB_PROFILE", "debug").strip() or "debug"
EXE = os.path.join(REPO, "target", _PROFILE, "agent-hub" + _SUFFIX)

_TEST_HOME = os.path.join(REPO, ".test-home")


def _cleanup_test_home():
    """Remove the whole stable test home at process exit - nothing is left behind."""
    shutil.rmtree(_TEST_HOME, ignore_errors=True)


atexit.register(_cleanup_test_home)


def registry_published():
    """Classify the official registry release asset. Returns one of:
      ('published', url)      - HTTP 200: it exists and can be used.
      ('unpublished', why)    - HTTP 404: the release/asset does not exist (BLOCKED on publication).
      ('unreachable', why)    - DNS/TLS/timeout/other 5xx: we do NOT know it is unpublished; the
                                environment or upstream failed. This is NOT 'confirmed unpublished'.
    A probe must not call everything 'unpublished': unreachable is its own, weaker fact."""
    import urllib.request
    import urllib.error
    url = "https://github.com/sabishii-me/agent-hub/releases/download/registry/registry.json"
    try:
        req = urllib.request.Request(url, method="HEAD")
        with urllib.request.urlopen(req, timeout=10) as r:
            if r.status == 200:
                return ("published", url)
            return ("unreachable", f"the registry asset answered {r.status}")
    except urllib.error.HTTPError as e:
        if e.code == 404:
            return ("unpublished", "the official registry release asset does not exist (HTTP 404)")
        return ("unreachable", f"the registry asset answered HTTP {e.code}")
    except Exception as e:
        # DNS / TLS / timeout: the registry may exist; we simply could not reach it.
        return ("unreachable", f"the official registry could not be reached: {type(e).__name__}: {e}")


def hub_sha():
    try:
        return subprocess.check_output(["git", "-C", REPO, "rev-parse", "--short", "HEAD"], text=True).strip()
    except Exception:
        return "?"


class Hub:
    """One real hub process on a FIXED, STABLE data dir (not a random tmp dir).

    The data dir is `<repo>/.test-home/<name>` unless a caller names another one. It is the SAME
    path across runs (so an install lands somewhere you can inspect and so a stale cache is
    visibly reused, not silently hidden in tmp), and `cleanup()` REMOVES it - every run ends with
    no leftover home."""

    def __init__(self, plugins_src=None, env=None, data_dir=None, home=None):
        # ONE stable, git-ignored home: `<repo>/.test-home`. NOT a random tmp dir and NOT a
        # per-instance/per-test subdir. The user runs ONE hub on ONE home, repeatedly (start,
        # stop, install, remove, restart); so does the test. The constructor does NOT wipe the
        # home - wiping it would HIDE a collision that a real user would hit. It is cleaned only
        # by cleanup() / process exit.
        self.dir = data_dir or os.path.join(REPO, ".test-home")
        os.makedirs(self.dir, exist_ok=True)
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
        })
        # The test NEVER supplies a registry. The hub uses its OWN compiled-in OFFICIAL address
        # (a dev build only honours an override; we must not set one). A mock/local registry here
        # would make the TEST the source of truth and hide that the hub's real registry path is
        # broken - which is exactly the defect that let "tests green" coexist with a dead third
        # party. No override: point at nothing.
        env.pop("AGENT_HUB_REGISTRY_URL", None)
        env.pop("AGENT_HUB_REGISTRY_FILE", None)
        env.update(self.extra_env)
        self.child = subprocess.Popen([EXE], env=env, stdout=self._log, stderr=self._log, creationflags=(0x08000000 if os.name == "nt" else 0))
        self.ep = self._await_endpoint()
        self.install_plugins()
        return self

    def install_plugins(self):
        """Install a requested plugin through the hub's OWN registry - the real product path.

        The test supplies only the registry ADDRESS (the environment's config) or nothing. It
        NEVER supplies a url/sha256 of its own: it asks the hub to REFRESH, reads the release the
        hub's OWN catalog lists for the id, and posts THAT as source.artifact (the hub reconciles
        it against the same registry) -> a REAL install.

        There is NO block here. If the hub has no usable registry (prod, official address absent),
        the refresh/install FAILS and this RAISES with the real reason - the caller's run is RED,
        not skipped."""
        if not self._plugin_sources:
            return
        rr = self.post("/v1/plugins/registry/refresh", timeout=60)
        if rr["status"] >= 300:
            code = (rr["json"] or {}).get("error")
            detail = (rr["json"] or {}).get("detail") or ""
            raise RuntimeError(f"the hub could not refresh its registry: {rr['status']} {code}: {detail[:160]}")
        for spec in self._plugin_sources:
            if isinstance(spec, str) and os.path.isdir(spec):
                pid = self._plugin_id(spec)
            else:
                pid = str(spec)
            if pid in self._installed:
                continue
            cat = self.get("/v1/plugins/catalog")
            entry = next((p for p in ((cat["json"] or {}).get("plugins") or [])
                          if p.get("id") == pid), None)
            if entry is None:
                raise RuntimeError(f"the hub's registry lists no `{pid}` (a real dependency is gone)")
            v = (entry.get("versions") or [None])[0]
            if not v:
                raise RuntimeError(f"the registry entry `{pid}` names no versions")
            artifact = {"url": v["url"], "sha256": v["sha256"], "id": pid,
                        "pluginType": entry.get("pluginType"), "version": v["version"],
                        "size": v.get("size")}
            r = self.post("/v1/plugins", {"source": {"artifact": artifact}}, key="install-" + pid)
            if isinstance(r.get("json"), dict) and r["json"].get("error"):
                raise RuntimeError(f"install {pid} failed: {r['status']} {r['text'][:200]}")
            self._await_plugin(pid, "ready")
            # Install now prepares the runtime itself; `ready` must mean the runtime is on disk.
            self._await_runtime(pid)
            self._installed.add(pid)

    def registry_artifact(self, pid):
        """The {url, sha256, id, pluginType, version} of `pid`'s newest release, taken from the
        HUB'S OWN catalog after the hub REFRESHES its registry from the official address - the real
        user operation (refresh -> catalog -> install), not a file the test reads and not a HEAD
        probe standing in for the hub's own preparation."""
        rr = self.post("/v1/plugins/registry/refresh", timeout=60)
        if rr["status"] >= 300:
            raise RuntimeError(f"the hub could not refresh its registry: {rr['status']} {rr['text'][:160]}")
        cat = self.get("/v1/plugins/catalog")
        entry = next((p for p in ((cat["json"] or {}).get("plugins") or []) if p.get("id") == pid), None)
        if entry is None:
            raise RuntimeError(f"the hub's catalog lists no `{pid}` after refresh (the real registry does not list it)")
        v = (entry.get("versions") or [None])[0]
        if not v:
            raise RuntimeError(f"the catalog entry `{pid}` names no versions")
        return {"url": v["url"], "sha256": v["sha256"], "id": pid,
                "pluginType": entry.get("pluginType"), "version": v["version"],
                "size": v.get("size")}

    def install_artifact(self, pid):
        """Install `pid` via the HUB'S OWN catalog release (url+sha256). The hub authorizes it
        because its registry lists it. Requires the official registry to be published."""
        art = self.registry_artifact(pid)
        r = self.post("/v1/plugins", {"source": {"artifact": art}}, key="install-" + pid)
        if isinstance(r.get("json"), dict) and r["json"].get("error"):
            raise RuntimeError(f"install {pid} failed: {r['status']} {r['text'][:200]}")
        self._await_plugin(pid, "ready")
        # Install now prepares the runtime itself; `ready` must mean the runtime is on disk.
        self._await_runtime(pid)
        self._installed.add(pid)
        return r

    # NOTE: there is deliberately NO _registry() here. The test must never read a registry FILE to
    # supply a plugin: that makes the TEST the registry and hides that the hub owns no source of
    # truth. The hub's own catalog (GET /v1/plugins/catalog) is the only source a test reads.

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
        # A plugin that declares NO command is DATA only (a model-provider ships a descriptor,
        # not a runtime): there is nothing to prepare, so do not wait for a runtime that will
        # never exist. A plugin WITH a command (a harness adapter) must reach runtimeReady.
        for _ in range(1920):
            g = self.get(f"/v1/plugins/{pid}")
            pj = (g["json"] or {}).get("plugin", {})
            if pj.get("runtimeReady"):
                return True
            if pj.get("pluginType") != "harness-adapter":
                # A model-provider is DATA (a descriptor), not a runtime: there is nothing to
                # prepare, so never wait for a runtime that will not exist. Its readiness is its state.
                return False
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


def register_provider(hub, pid=None):
    """Create a provider through the HUB'S OWN surface (`POST /v1/model-providers`) from what the
    run was GIVEN in the environment, and return its identity. NEVER read from a file on the
    machine. Returns None when the environment names no provider (a missing real dependency FAILS,
    never skips).

    Two forms, decided by the env JSON:
      - `providerType` set (e.g. "deepseek"): an INSTALLED type OWNS its endpoint - only the key is
        needed; the hub uses the type's url/api and the type's id.
      - otherwise `{url, api}`: the built-in `custom-compatible`, a user provider in `_CUSTOM-`.
    In both cases the hub decides the id; the caller uses the id READ BACK from the response."""
    prov = real_provider()
    if not prov:
        return None
    body = {"token": prov["token"]}
    if prov.get("providerType"):
        body["providerType"] = prov["providerType"]
        body["providerTypeVersion"] = 1
        if pid:
            body["id"] = pid
    else:
        body["id"] = pid or prov.get("id") or "test-provider"
        body["url"] = prov["url"]
        body["api"] = prov["api"]
    r = hub.post("/v1/model-providers", body)
    # The hub owns the final id (a user provider lives in the `_CUSTOM-` namespace; an installed
    # type owns its own): read it BACK from the response.
    real_id = (((r.get("json") or {}).get("provider") or {}).get("id")) or body.get("id")
    return {**prov, "id": real_id}


def real_provider(env_var="AGENT_HUB_TEST_PROVIDER"):
    """The provider the RUN was GIVEN, from the environment (JSON). None when absent.

    NOT read from a file on the machine: a test must not secretly depend on the host it runs
    on. The caller supplies the provider deliberately, e.g.

        export AGENT_HUB_TEST_PROVIDER='{"url":"https://api.example/v1","api":"openai-completions","token":"sk-..."}'

    (A provider TYPE that owns its endpoint can be named with `providerType` instead of a url -
    see register_provider.) A missing provider is a MISSING REAL DEPENDENCY: the caller FAILS,
    it does not skip."""
    raw = os.environ.get(env_var) or os.environ.get("HOME_JP_PROD_JSON")
    if not raw:
        return None
    try:
        p = json.loads(raw)
    except Exception:
        return None
    if not p.get("url") or not p.get("token"):
        return None
    return {"id": p.get("id") or "test-provider", "url": p["url"],
            "api": p.get("api") or "openai-completions", "token": p["token"],
            "providerType": p.get("providerType"),
            # optional: the provider's declared models, as the run supplies them (the same
            # thinkingLevelMap data a real provider config carries); [] when not given.
            "models": p.get("models") or []}


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


def prune_orphans(*needles):
    """Kill (with Node's own kill, never taskkill) any process whose command line mentions one of
    `needles` and belongs to this hub's test data dir. Used to clear a previous test's leftover
    adapter/harness processes so the next test starts clean. No external kill command: endpoint
    protection flags spawned `taskkill`/`tasklist` command lines (measured)."""
    script = (
        "Get-CimInstance Win32_Process | "
        "Where-Object { $_.CommandLine -ne $null } | "
        "ForEach-Object { \"$($_.ProcessId)`t$($_.CommandLine)\" }"
    )
    try:
        txt = subprocess.check_output(["powershell", "-NoProfile", "-Command", script],
                                      text=True, stderr=subprocess.DEVNULL, timeout=30)
    except Exception:
        return
    for line in txt.splitlines():
        if "\t" not in line:
            continue
        pid_s, cmd = line.split("\t", 1)
        if not pid_s.strip().isdigit():
            continue
        if any(n and n in cmd for n in needles):
            kill_pid(int(pid_s.strip()))


def kill_pid(pid):
    """Kill one process with NODE's own kill - never a spawned `taskkill`/`tasklist` (endpoint
    protection flags those command lines; measured). SIGKILL on POSIX; on Windows `os.kill` with
    SIGTERM is a hard TerminateProcess for a pid we do not own, which is what fault injection
    wants."""
    try:
        os.kill(pid, signal.SIGKILL if os.name != "nt" else signal.SIGTERM)
    except Exception:
        # A pid we have no rights to (or already gone): fall back to the tool only if node cannot.
        pass


def descendants_with_command(root_pid):
    """Every descendant (pid, name, command_line) under root_pid, so a test can pick the process
    that actually runs the harness (e.g. the runtime node, NOT the adapter) across harnesses whose
    process shapes differ."""
    ps = ("Get-CimInstance Win32_Process | "
          "ForEach-Object { \"$($_.ProcessId)`t$($_.ParentProcessId)`t$($_.Name)`t$($_.CommandLine)\" }")
    try:
        txt = subprocess.check_output(["powershell", "-NoProfile", "-Command", ps],
                                      text=True, stderr=subprocess.DEVNULL, timeout=30)
    except Exception:
        return []
    rows = []
    for line in txt.splitlines():
        parts = line.split("\t")
        if len(parts) >= 3 and parts[0].strip().isdigit() and parts[1].strip().isdigit():
            rows.append((int(parts[0]), int(parts[1]), parts[2].strip(), parts[3] if len(parts) > 3 else ""))
    want = {root_pid}
    out = []
    changed = True
    while changed:
        changed = False
        for pid, ppid, name, cmd in rows:
            if ppid in want and pid not in want:
                want.add(pid)
                out.append((pid, name, cmd))
                changed = True
    return out


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

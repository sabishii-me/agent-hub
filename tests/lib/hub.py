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
        if plugins_src:
            # Copy the REAL plugin directory in (never a stub). The hub scans for a
            # manifest.json, and the harness id is the manifest's id, so name the dir
            # by that id (a dir named otherwise would not be found).
            for name in (plugins_src if isinstance(plugins_src, (list, tuple)) else [plugins_src]):
                pid = self._plugin_id(name)
                dst = os.path.join(self.plugins, pid)
                if os.path.exists(dst):
                    shutil.rmtree(dst)
                shutil.copytree(name, dst)
        self.extra_env = env or {}
        self.child = None
        self.ep = None
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
        return self

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
        return self.req("POST", p, body, **k)

    def patch(self, p, body=None, **k):
        return self.req("PATCH", p, body, **k)

    def put(self, p, body=None, **k):
        return self.req("PUT", p, body, **k)

    def delete(self, p, **k):
        return self.req("DELETE", p, **k)

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


def kill_pid(pid):
    if os.name == "nt":
        subprocess.run(["taskkill", "/PID", str(pid), "/T", "/F"],
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    else:
        try:
            os.kill(pid, signal.SIGKILL)
        except Exception:
            pass

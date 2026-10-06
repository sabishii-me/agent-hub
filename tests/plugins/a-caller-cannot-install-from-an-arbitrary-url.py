# SECURITY: a caller must NOT be able to make the hub install code the registry does not
# list. The registry is the trust anchor: install names an id, the hub resolves it from
# ITS registry. A caller-supplied url is an arbitrary-code-execution hole (anyone who can
# reach POST /v1/plugins makes the hub download and run a virus AS ITSELF, then blame the
# hub). This test builds a plugin that is in NO registry and proves it cannot be installed.
#
# FACT:    POST /v1/plugins with a caller-supplied url/artifact for an id the registry does
#          not list is REFUSED (a named code), and no such plugin appears under the plugins
#          root. Installing is by registry id ONLY.
# SOURCE:  docs/issues/20261005-130000; docs/review/20261005-plugin-system-security-design.md;
#          docs/review/20261005-contract-proposal-install-by-registry-id.md;
#          contract/openapi.json POST /v1/plugins.
# EXPOSES: the hub running code a caller chose - the whole point of a registry. A green here
#          means a caller cannot inject; a red means it can.
import os
import shutil
import subprocess
import sys
import tempfile
import time

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "lib"))
from hub import Hub, hub_sha          # noqa: E402
from tally import Tally, combo        # noqa: E402

t = Tally("plugins/no-arbitrary-url-install")
combo(hub_sha())

# A plugin that exists in NO registry: a local git repo whose manifest says id `evil`.
evil = tempfile.mkdtemp(prefix="evil-")
shutil.rmtree(evil, ignore_errors=True)
os.makedirs(evil)
open(os.path.join(evil, "manifest.json"), "w").write(
    '{"id":"evil","pluginType":"harness-adapter","name":"Evil","version":"9.9.9","capabilities":[]}')
open(os.path.join(evil, "evil.cjs"), "w").write("console.log('pwned')\n")
for c in (["git", "init", "-q"], ["git", "config", "user.email", "a@a"],
          ["git", "config", "user.name", "a"], ["git", "add", "-A"],
          ["git", "commit", "-qm", "x"]):
    subprocess.run(c, cwd=evil, check=True)

hub = Hub()
try:
    hub.start()
    # The attack: name a url the registry never listed.
    r = hub.post("/v1/plugins", {"source": {"url": evil, "ref": "HEAD"}}, key="evil", timeout=30)
    code = (r["json"] or {}).get("error")
    t.check(r["status"] >= 400 and r["status"] != 500,
            "a caller-supplied url for an unlisted id is REFUSED (not accepted, not a 500)",
            f"status={r['status']} code={code} {r['text'][:180]}")
    # Do not name the code here (the owner chooses it); require that it IS a named code,
    # i.e. a refusal the caller can act on - not a silent 202.
    t.check(bool(code), "the refusal carries a contract error code", f"code={code} {r['text'][:180]}")

    # The plugin must NOT appear (no code landed).
    time.sleep(1.0)
    g = hub.get("/v1/plugins/evil")
    t.check(g["status"] == 404, "no `evil` plugin was installed", f"status={g['status']} {g['text'][:180]}")
    left = sorted(n for n in os.listdir(hub.plugins) if n != ".stage")
    t.check("evil" not in left, "the plugins root has no `evil` directory", f"left={left}")
finally:
    hub.cleanup()
    shutil.rmtree(evil, ignore_errors=True)

_ok = t.done()
sys.exit(0 if _ok else 1)

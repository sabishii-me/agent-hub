# POST /v1/plugins REFUSALS: every rejection names itself with its contract code, and a
# refused install leaves NO half tree behind. A success-only suite cannot see a wrong code
# or a leaked directory.
#
# FACT:    (B6) a source that names neither url nor artifact -> 400 validation_failed;
#          (B7) a git tree with no manifest.json -> plugin_archive_invalid naming it, and
#               the plugins root gains NO plugin directory (only the hub's own .stage);
#          (B8) an artifact whose sha256 is wrong -> artifact_digest_mismatch, naming both
#               digests, and nothing is unpacked;
#          (B1) an id that lives in a deployment directory -> 409 conflict, naming it.
# SOURCE:  crates/plugins/src/routes.rs:232-241; service.rs:49-56,17-18,336-430,484-500;
#          source.rs:58-72; contract/errors.json; contract/openapi.json POST /v1/plugins;
#          ARCHITECTURE s21 N3.
# EXPOSES: a refusal that returns 500; a code that is not the contract's; a message that does
#          not name what was wrong; a half-written tree after a refusal; or a HANG.
import os
import shutil
import subprocess
import sys
import tempfile

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "lib"))
from hub import Hub, hub_sha          # noqa: E402
from tally import Tally, combo        # noqa: E402

PI = os.environ.get("PI_PLUGIN_DIR", r"E:/AI/ideas/prts-harness-pi")
PI_REF = os.environ.get("PI_PLUGIN_REF", "fix/runtime-placement")
t = Tally("plugins/install-refusals")
combo(hub_sha())
if not t.require(os.path.isdir(PI), "a real plugin source is present", f"no plugin at {PI}"):
    t.done(); sys.exit(1)


def code_and_detail(r):
    j = r["json"] or {}
    return j.get("error"), (j.get("detail") or "")


def plugin_dirs(root):
    """Directories under the plugins root EXCLUDING the hub's own .stage - that is
    infrastructure, not a landed plugin."""
    return sorted(n for n in os.listdir(root) if n != ".stage")


# --- B6: a source that names neither url nor artifact.
hub = Hub()
try:
    hub.start()
    r = hub.post("/v1/plugins", {"source": {}})
    code, detail = code_and_detail(r)
    t.check(r["status"] == 400, "B6: a source with no url/artifact is 400", f"status={r['status']} {r['text'][:160]}")
    t.check(code == "validation_failed", "B6: code is validation_failed", f"code={code} {r['text'][:160]}")
    t.check("url" in detail.lower() and "artifact" in detail.lower(),
            "B6: the message names the two source shapes", f"detail={detail!r}")
finally:
    hub.cleanup()

# --- B7: a git repo whose tree has no manifest.json -> refused, no half tree.
repo = tempfile.mkdtemp(prefix="nomanifest-")
shutil.rmtree(repo, ignore_errors=True)
os.makedirs(repo)
open(os.path.join(repo, "package.json"), "w").write("{}\n")
for c in (["git", "init", "-q"], ["git", "config", "user.email", "t@t"],
          ["git", "config", "user.name", "t"], ["git", "add", "-A"],
          ["git", "commit", "-qm", "x"]):
    subprocess.run(c, cwd=repo, check=True)
hub = Hub()
try:
    hub.start()
    r = hub.post("/v1/plugins", {"source": {"url": repo, "ref": "HEAD"}}, key="b7", timeout=30)
    code, detail = code_and_detail(r)
    t.check(r["status"] == 502, "B7: a repo without manifest.json is 502", f"status={r['status']} {r['text'][:160]}")
    t.check(code == "plugin_archive_invalid", "B7: code is plugin_archive_invalid", f"code={code} {r['text'][:160]}")
    t.check("manifest" in detail.lower(), "B7: the message names the manifest", f"detail={detail!r}")
    left = plugin_dirs(hub.plugins)
    t.check(left == [], "B7: a refused install leaves NO plugin directory (only .stage)", f"left={left}")
finally:
    hub.cleanup()
    shutil.rmtree(repo, ignore_errors=True)

# --- B8: an artifact whose sha256 is wrong -> refused BEFORE unpacking; no half tree.
hub = Hub()
try:
    hub.start()
    bad = "1" * 64
    r = hub.post("/v1/plugins", {"source": {"artifact": {
        "url": "https://github.com/sabishii-me/agent-hub-harness-adapter-pi/releases/download/v0.1.8/harness-adapter-pi-0.1.8.zip",
        "sha256": bad, "id": "pi", "pluginType": "harness-adapter", "version": "0.1.8"}}}, key="b8", timeout=40)
    code, detail = code_and_detail(r)
    t.check(r["status"] == 502, "B8: a wrong sha256 is 502", f"status={r['status']} code={code} {r['text'][:160]}")
    t.check(code == "artifact_digest_mismatch", "B8: code is artifact_digest_mismatch", f"code={code} {r['text'][:160]}")
    t.check(bad in detail and "sha256" in detail.lower(),
            "B8: the message carries the expected digest and names sha256", f"detail={detail!r}")
    t.check("pi" not in plugin_dirs(hub.plugins), "B8: a refused artifact left no `pi` directory",
            f"left={plugin_dirs(hub.plugins)}")
finally:
    hub.cleanup()

# --- B1: an id that lives in a DEPLOYMENT directory -> 409, naming it. (See
# 20261005-120000: this HANGS today - the bounded timeout makes the RED visible, not a stall.)
dep = tempfile.mkdtemp(prefix="deploy-")
os.makedirs(os.path.join(dep, "pi"))
shutil.copy(os.path.join(PI, "manifest.json"), os.path.join(dep, "pi", "manifest.json"))
hub = Hub(env={"AGENT_HUB_PLUGINS_DIR": dep})
try:
    hub.start()
    try:
        r = hub.post("/v1/plugins", {"source": {"url": PI, "ref": PI_REF}}, key="b1", timeout=20)
        code, detail = code_and_detail(r)
        t.check(r["status"] == 409, "B1: installing an id owned by a deployment dir is 409",
                f"status={r['status']} code={code} {r['text'][:160]}")
        t.check(code == "conflict", "B1: code is conflict", f"code={code} {r['text'][:160]}")
        t.check(dep in detail, "B1: the message names the deployment directory", f"detail={detail!r} dep={dep}")
    except Exception as e:
        t.check(False, "B1: the request RETURNS (409 conflict) instead of hanging",
                f"HUNG: {type(e).__name__} (docs/issues/20261005-120000)")
finally:
    hub.cleanup()
    shutil.rmtree(dep, ignore_errors=True)

t.done()

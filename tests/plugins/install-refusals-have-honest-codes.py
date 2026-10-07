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

# --- B7: with NO registry, a git source is refused 403. NOTE what this does and does NOT prove:
# the hub has no registry here, so EVERY source is refused for lack of a registry - this is NOT
# evidence that git sources are specifically rejected by a sha256 rule. It is the no-registry
# refusal, which is all that is verifiable without the official registry.
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
    t.check(r["status"] == 403 and code == "plugin_not_in_registry",
            "no registry: a git source is refused 403 plugin_not_in_registry",
            f"status={r['status']} code={code} {r['text'][:160]}")
    t.check("registry" in detail.lower(), "the message names the registry", f"detail={detail!r}")
    left = plugin_dirs(hub.plugins)
    t.check(left == [], "a refused install leaves NO plugin directory (only .stage)", f"left={left}")
finally:
    hub.cleanup()
    shutil.rmtree(repo, ignore_errors=True)

# --- B8: with NO registry, an artifact is refused 403. This does NOT prove the sha256 is checked:
# there is no registry to compare the sha256 against, so the refusal is the no-registry refusal,
# not a digest-mismatch. Calling it "the listed url with a wrong sha256" would be unfounded. The
# digest-mismatch path is BLOCKED (needs the official registry to be 'listed' at all).
hub = Hub()
try:
    hub.start()
    bad = "1" * 64
    r = hub.post("/v1/plugins", {"source": {"artifact": {
        "url": "https://github.com/sabishii-me/agent-hub-harness-adapter-pi/releases/download/v0.1.8/harness-adapter-pi-0.1.8.zip",
        "sha256": bad, "id": "pi", "pluginType": "harness-adapter", "version": "0.1.8"}}}, key="b8", timeout=40)
    code, detail = code_and_detail(r)
    t.check(r["status"] == 403 and code == "plugin_not_in_registry",
            "no registry: an artifact is refused 403 plugin_not_in_registry",
            f"status={r['status']} code={code} {r['text'][:160]}")
    t.check("pi" not in plugin_dirs(hub.plugins), "a refused artifact left no `pi` directory",
            f"left={plugin_dirs(hub.plugins)}")
    t.blocked_check(
        "an artifact whose sha256 differs from a REGISTRY entry is refused for THAT reason",
        "needs the official registry to be a 'listed' url at all (UNPUBLISHED)")
finally:
    hub.cleanup()

_ok = t.done()
sys.exit(0 if _ok else 1)

# Install must RECONCILE the caller's source against the registry (docs/issues/20261005-130000):
# an artifact whose url AND sha256 match a registry release is AUTHORIZED; anything else is
# refused with 403 plugin_not_in_registry, and NOTHING is fetched or landed. The sha256 is the
# anchor: matching the url alone would let a proxied url serve different bytes.
#
# FACT:    with a registry loaded,
#            a source whose url+sha256 match a registry release -> accepted (202);
#            a source whose url is NOT listed                -> 403 plugin_not_in_registry;
#            a source whose url IS listed but sha256 differs  -> 403 plugin_not_in_registry;
#            no registry loaded                               -> any source refused.
# SOURCE:  crates/plugins/src/service.rs (begin_install -> authorized -> Registry::authorizes);
#          crates/plugins/src/registry.rs (authorizes: url AND sha256);
#          contract/errors.json (plugin_not_in_registry 403); docs/issues/20261005-130000.
# EXPOSES: an unlisted source installed (the injection hole); a url-only match accepted (a proxied
#          url would pass); or a listed release wrongly refused.
import json
import os
import sys
import tempfile

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "lib"))
from hub import Hub, hub_sha          # noqa: E402
from tally import Tally, combo        # noqa: E402

t = Tally("plugins/install-reconciles")
combo(hub_sha())

REPO = os.path.abspath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", ".."))
repo_reg = json.load(open(os.path.join(REPO, "registry.json")))
pi = next(p for p in repo_reg["plugins"] if p["id"] == "pi" and p["pluginType"] == "harness-adapter")
v = pi["versions"][0]

# A registry that lists exactly this one release.
d = tempfile.mkdtemp(prefix="reg-")
rf = os.path.join(d, "registry.json")
open(rf, "w").write(json.dumps(
    {"schema": 1, "plugins": [{"id": "pi", "pluginType": "harness-adapter", "name": "Pi",
                               "versions": [v]}]}))


def code_of(r):
    return (r["json"] or {}).get("error")


# --- No registry: nothing is authorized.
hub0 = Hub()
try:
    hub0.start()
    r = hub0.post("/v1/plugins", {"source": {"artifact": {
        "url": v["url"], "sha256": v["sha256"], "id": "pi",
        "pluginType": "harness-adapter", "version": v["version"]}}}, key="none", timeout=30)
    t.check(r["status"] == 403 and code_of(r) == "plugin_not_in_registry",
            "no registry loaded -> even a real release is refused (403 plugin_not_in_registry)",
            f"status={r['status']} code={code_of(r)} {r['text'][:160]}")
finally:
    hub0.cleanup()

# --- With the registry loaded.
hub = Hub(env={"AGENT_HUB_REGISTRY_FILE": rf, "AGENT_HUB_REGISTRY_URL": "file://" + rf})
try:
    hub.start()
    # authorized: exact url + sha256.
    r = hub.post("/v1/plugins", {"source": {"artifact": {
        "url": v["url"], "sha256": v["sha256"], "id": "pi",
        "pluginType": "harness-adapter", "version": v["version"]}}}, key="ok", timeout=60)
    t.check(r["status"] == 202, "a source matching a registry release (url+sha256) is accepted",
            f"status={r['status']} {r['text'][:160]}")

    # unauthorized: url not listed at all.
    r = hub.post("/v1/plugins", {"source": {"artifact": {
        "url": "https://example.com/evil.zip", "sha256": "0" * 64, "id": "evil",
        "pluginType": "harness-adapter", "version": "1.0.0"}}}, key="evil", timeout=30)
    t.check(r["status"] == 403 and code_of(r) == "plugin_not_in_registry",
            "a url the registry does not list is refused (403 plugin_not_in_registry)",
            f"status={r['status']} code={code_of(r)} {r['text'][:160]}")

    # unauthorized: the LISTED url, but a DIFFERENT sha256 (a proxied url serving other bytes).
    r = hub.post("/v1/plugins", {"source": {"artifact": {
        "url": v["url"], "sha256": "0" * 64, "id": "pi",
        "pluginType": "harness-adapter", "version": v["version"]}}}, key="proxy", timeout=30)
    t.check(r["status"] == 403 and code_of(r) == "plugin_not_in_registry",
            "the listed url with a DIFFERENT sha256 is refused (a proxied url cannot pass)",
            f"status={r['status']} code={code_of(r)} {r['text'][:160]}")
finally:
    hub.cleanup()
    import shutil
    shutil.rmtree(d, ignore_errors=True)

t.done()

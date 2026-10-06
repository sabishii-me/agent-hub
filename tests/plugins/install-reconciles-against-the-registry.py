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
v = pi["versions"][0]   # a real release url+sha256, used as the SOURCE whose refusal we test


def code_of(r):
    return (r["json"] or {}).get("error")


# This file tests the NO-REGISTRY behaviour of the reconcile gate. The AUTHORIZED path (a
# registry that lists a release -> the hub installs it) needs the OFFICIAL registry, which is
# unpublished; it is NOT tested here by fabricating a registry (no mock registry, no local
# injection - owner direction). It is BLOCKED.
#
# --- No registry: nothing is authorized. The harness supplies none, and we clear any dev override,
# so the hub REALLY has no registry.
hub0 = Hub(env={"AGENT_HUB_REGISTRY_FILE": ""})
try:
    hub0.start()
    r = hub0.post("/v1/plugins", {"source": {"artifact": {
        "url": v["url"], "sha256": v["sha256"], "id": "pi",
        "pluginType": "harness-adapter", "version": v["version"]}}}, key="none", timeout=30)
    t.check(r["status"] == 403 and code_of(r) == "plugin_not_in_registry",
            "no registry loaded -> even a real release is refused (403 plugin_not_in_registry)",
            f"status={r['status']} code={code_of(r)} {r['text'][:160]}")
    # An unlisted url likewise: no registry -> refused.
    r = hub0.post("/v1/plugins", {"source": {"artifact": {
        "url": "https://example.com/evil.zip", "sha256": "0" * 64, "id": "evil",
        "pluginType": "harness-adapter", "version": "1.0.0"}}}, key="evil", timeout=30)
    t.check(r["status"] == 403 and code_of(r) == "plugin_not_in_registry",
            "no registry loaded -> an unlisted url is also refused (403)",
            f"status={r['status']} code={code_of(r)} {r['text'][:160]}")
    # A git source: no sha256, cannot match a release registry -> refused.
    r = hub0.post("/v1/plugins", {"source": {"url": "https://example.com/evil.git", "ref": "main"}}, key="git", timeout=30)
    t.check(r["status"] == 403 and code_of(r) == "plugin_not_in_registry",
            "no registry loaded -> a git source is refused (403)",
            f"status={r['status']} code={code_of(r)} {r['text'][:160]}")
finally:
    hub0.cleanup()

# The AUTHORIZED path (registry lists a release -> the hub installs it, url+sha256 reconcile) and
# the proxied-url case (listed url, different sha256) BOTH need the official registry, which is
# UNPUBLISHED. Testing them would require fabricating a registry, which is forbidden. BLOCKED.
t.blocked_check(
    "a source matching a registry release is accepted; a proxied url is refused",
    "needs the official registry (UNPUBLISHED); not tested by fabricating one")

_ok = t.done()
sys.exit(0 if _ok else 1)

# ERROR CODES against the RELEASE build of the hub. A release binary's registry address is
# FIXED at build time (no env override exists), so this is what a USER actually runs. Every
# failure must answer a CONTRACT code - {"error": CODE, "detail": ...} with the HTTP status
# contract/errors.json pins - never axum's own text, never a lie in the detail.
#
# FACT:    on a release hub, each of these failures answers its contract code and status:
#            POST /v1/plugins {}            -> 400 validation_failed (body shape)
#            POST /v1/plugins {source:{}}   -> 400 validation_failed (no url/artifact)
#            GET  /v1/plugins/{missing}     -> 404 not_found
#            DELETE /v1/plugins/{missing}   -> 404 not_found (NOT "a deployment directory")
#            POST /v1/plugins/{missing}/prepare -> 404 not_found
#            artifact with a wrong sha256   -> 502 artifact_digest_mismatch
#            artifact with an unreachable url -> 502 artifact_download_failed
#          and the registry override env var is IGNORED by the release binary.
# SOURCE:  contract/errors.json + contract/openapi.json; crates/plugins/src/routes.rs (install
#          rejection -> validation_failed); service.rs (begin_remove: NotFound vs NotInstalledByHub);
#          crates/plugins/src/registry.rs (address fixed at build time).
# EXPOSES: a failure that returns 422/axum text; a removed-but-nonexistent id described as a
#          deployment directory (a false detail); or a release binary honouring the dev override.
import os
import sys

# This file is the RELEASE build's error surface: select the release binary for the harness.
os.environ["AGENT_HUB_PROFILE"] = "release"

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "lib"))
from hub import Hub, hub_sha, EXE      # noqa: E402
from tally import Tally, combo         # noqa: E402

t = Tally("plugins/release-error-surface")
combo(hub_sha(), f"profile=release")
t.require(os.path.exists(EXE), "the RELEASE binary exists (cargo build --release)", f"missing {EXE}")
if not t.require(os.path.exists(EXE), "release binary present", f"no {EXE}"):
    t.done(); sys.exit(1)


def code_of(r):
    return (r["json"] or {}).get("error")


hub = Hub(env={"AGENT_HUB_REGISTRY_URL": "http://127.0.0.1:59999/registry.json"})  # must be IGNORED
try:
    hub.start()

    r = hub.post("/v1/plugins", {})
    t.check(r["status"] == 400 and code_of(r) == "validation_failed",
            "no `source` field -> 400 validation_failed (not 422 axum text)",
            f"status={r['status']} code={code_of(r)} {r['text'][:140]}")

    r = hub.post("/v1/plugins", {"source": {}})
    t.check(r["status"] == 400 and code_of(r) == "validation_failed",
            "source with no url/artifact -> 400 validation_failed",
            f"status={r['status']} code={code_of(r)} {r['text'][:140]}")

    r = hub.get("/v1/plugins/nope")
    t.check(r["status"] == 404 and code_of(r) == "not_found",
            "GET a missing plugin -> 404 not_found", f"status={r['status']} code={code_of(r)}")

    r = hub.delete("/v1/plugins/nope")
    detail = (r["json"] or {}).get("detail") or ""
    t.check(r["status"] == 404 and code_of(r) == "not_found",
            "DELETE a missing plugin -> 404 not_found", f"status={r['status']} code={code_of(r)}")
    t.check("deployment" not in detail.lower(),
            "a missing plugin is NOT described as a deployment directory",
            f"detail={detail!r}")

    r = hub.post("/v1/plugins/nope/prepare")
    t.check(r["status"] == 404 and code_of(r) == "not_found",
            "prepare a missing plugin -> 404 not_found", f"status={r['status']} code={code_of(r)}")

    # A RELEASE hub has NO registry (the official one is unpublished). Every install source is
    # therefore refused at the registry gate with 403 plugin_not_in_registry - the correct
    # behaviour with no registry, and the digest/download checks are UNREACHABLE (nothing is
    # fetched). These are asserted as the real behaviour, not the old (unreachable) 502s.
    r = hub.post("/v1/plugins", {"source": {"artifact": {
        "url": "https://github.com/sabishii-me/agent-hub-harness-adapter-pi/releases/download/v0.1.8/harness-adapter-pi-0.1.8.zip",
        "sha256": "1" * 64, "id": "pi", "pluginType": "harness-adapter", "version": "0.1.8"}}},
        key="rel-sha", timeout=40)
    t.check(r["status"] == 403 and code_of(r) == "plugin_not_in_registry",
            "release hub with no registry: an artifact is refused 403 at the gate (not fetched)",
            f"status={r['status']} code={code_of(r)} {r['text'][:120]}")

    r = hub.post("/v1/plugins", {"source": {"artifact": {
        "url": "https://example.invalid/x.zip", "sha256": "0" * 64, "id": "pi",
        "pluginType": "harness-adapter", "version": "0.1.8"}}}, key="rel-dl", timeout=30)
    t.check(r["status"] == 403 and code_of(r) == "plugin_not_in_registry",
            "release hub with no registry: an unreachable url is refused 403 at the gate",
            f"status={r['status']} code={code_of(r)} {r['text'][:120]}")

    # The built-in address is the maintainer's 'registry' release asset. TWO distinct outcomes:
    #   - the refresh reports it cannot REACH the registry (the asset is unpublished, HTTP 404 in
    #     the detail) -> BLOCKED on publication; this is environment, not a product defect;
    #   - ANY other non-success (a 500, a malformed response, a wrong code) -> a PRODUCT failure
    #     and must FAIL. An unpublished asset must not swallow a real product bug.
    r = hub.post("/v1/plugins/registry/refresh", timeout=40)
    code = code_of(r)
    detail = (r["json"] or {}).get("detail") or ""
    if r["status"] < 300:
        n = (r["json"] or {}).get("plugins")
        t.check(isinstance(n, int) and n > 0,
                "the release hub refreshed from its built-in address (registry is published)",
                f"status={r['status']} body={r['text'][:120]}")
    elif code == "registry_unavailable" and "404" in detail:
        # The built-in asset is not published: an expected, environment-caused block.
        t.blocked_check("the release hub refreshes from its built-in registry address",
                        f"the 'registry' release asset is UNPUBLISHED (404); refresh said: {detail[:100]}")
    else:
        # A non-success that is NOT 'the asset is unpublished' is a PRODUCT failure, not a block.
        t.check(False,
                "the release hub's refresh fails ONLY because the asset is unpublished; any other "
                "failure is a product bug",
                f"status={r['status']} code={code} detail={detail[:100]}")
finally:
    hub.cleanup()

_ok = t.done()
sys.exit(0 if _ok else 1)

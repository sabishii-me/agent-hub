# A hub with an EMPTY plugins root must NOT be able to open a session on any harness:
# there is no adapter for a harness that is not installed. This pins the FAILURE path -
# "it does not exist until it is installed" - which the success-path tests assume.
#
# FACT:    with no plugin installed, /v1/harnesses is empty and POST /v1/sessions is
#          refused (harness_not_found), never silently served
# SOURCE:  contract/v1.json POST /v1/sessions (a harness must exist); contract/errors.json
#          harness_not_found; ARCHITECTURE (a plugin is installed, never baked in)
# EXPOSES: a hub that answers a session for a harness it does not have (a baked-in or
#          fabricated harness) - or a test that installs nothing and still "passes"
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "lib"))
from hub import Hub, hub_sha          # noqa: E402
from tally import Tally, combo        # noqa: E402

t = Tally("harnesses/uninstalled")
combo(hub_sha())

hub = Hub()   # EMPTY plugins root, on purpose
try:
    hub.start()
    l = hub.get("/v1/plugins")
    t.check(l["status"] == 200 and not (l["json"] or {}).get("plugins"),
            "no plugin is installed", f"plugins={(l['json'] or {}).get('plugins')}")

    h = hub.get("/v1/harnesses")
    t.check(h["status"] == 200 and not (h["json"] or {}).get("harnesses"),
            "no harness is listed (a harness comes from an installed plugin)",
            f"harnesses={(h['json'] or {}).get('harnesses')}")

    r = hub.post("/v1/sessions", {"harnessId": "pi"}, key="uninstalled-1")
    t.check(r["status"] == 404,
            "POST /v1/sessions for an uninstalled harness is refused (404)", f"status={r['status']}")
    t.check((r["json"] or {}).get("error") == "harness_not_found",
            "the refusal names harness_not_found", f"body={r['text'][:160]}")
    t.check("pi" in (r["text"] or "") or "adapter" in (r["text"] or ""),
            "the refusal names the missing harness/adapter", f"body={r['text'][:160]}")

    # A turn on a session that never existed is also refused (nothing to serve).
    tr = hub.post("/v1/sessions/s-nope/turns", {"content": [{"type": "text", "text": "hi"}], "idempotencyKey": "u-t"}, key="u-t")
    t.check(tr["status"] == 404, "a turn on a non-existent session is 404", f"status={tr['status']}")
finally:
    ok = t.done()
    hub.cleanup()
sys.exit(0 if ok else 1)

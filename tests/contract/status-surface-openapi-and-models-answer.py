# Metadata routes must answer REAL, not 501: the hub's status, its surface, its OpenAPI
# projection, and the merged model list. Each is read and its SHAPE checked against the
# contract (a red here is the product's fault).
import json
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "lib"))
from hub import Hub, hub_sha, REPO          # noqa: E402
from tally import Tally, combo              # noqa: E402

t = Tally("contract/metadata")
combo(hub_sha())
hub = Hub()
try:
    hub.start()

    st = hub.get("/v1/status")
    t.check(st["status"] == 200, "GET /v1/status answers 200", f"status={st['status']}")
    t.check(isinstance(st["json"], dict) and st["json"], "status is a non-empty object", f"body={st['text'][:120]}")

    sf = hub.get("/v1/surface")
    t.check(sf["status"] == 200 and sf["json"], "GET /v1/surface answers a body", f"status={sf['status']}")

    oa = hub.get("/v1/openapi.json")
    t.check(oa["status"] == 200, "GET /v1/openapi.json answers 200", f"status={oa['status']}")
    t.check(isinstance(oa["json"], dict) and ("openapi" in oa["json"] or "paths" in oa["json"]),
            "the served OpenAPI has a valid shape", f"keys={list((oa['json'] or {}).keys())[:6]}")

    # The committed openapi.json equals what the hub serves (one authority, no drift).
    disk = json.load(open(os.path.join(REPO, "contract", "openapi.json"), encoding="utf-8"))
    t.check(oa["json"] == disk, "the SERVED openapi equals the committed contract/openapi.json",
            "the served projection drifts from the artifact")

    m = hub.get("/v1/models")
    t.check(m["status"] == 200, "GET /v1/models answers 200", f"status={m['status']}")
    t.check("models" in (m["json"] or {}), "GET /v1/models returns a models array", f"keys={list((m['json'] or {}).keys())}")
finally:
    ok = t.done()
    hub.cleanup()
sys.exit(0 if ok else 1)

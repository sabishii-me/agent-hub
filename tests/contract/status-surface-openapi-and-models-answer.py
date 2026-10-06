# Metadata routes must answer REAL, not 501: the hub's status, its surface, its OpenAPI
# projection, and the merged model list. Each is read and its SHAPE checked against the
# contract (a red here is the product's fault).
#
# FACT:    status/surface/openapi/models answer REAL (200), and the served openapi equals the committed contract
# SOURCE:  contract/v1.json GET /v1/status,/v1/surface,/v1/openapi.json,/v1/models
# EXPOSES: a stub/501 metadata route, or a served contract drifting from the file
# (A test that would pass whatever happens is not a test: this block names the fact it
#  proves and where that fact comes from; the assertions below are that exact fact.)
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
    # The value must be an ARRAY of model objects, not merely the presence of the key ({"models":
    # "not-an-array"} must fail). Each entry must carry an id/providerId per the contract.
    arr = (m["json"] or {}).get("models")
    t.check(isinstance(arr, list), "GET /v1/models returns a models ARRAY (not a bare key)",
            f"type={type(arr).__name__} body={m['text'][:160]}")
    if isinstance(arr, list) and arr:
        sample = arr[0]
        t.check(isinstance(sample, dict) and ("id" in sample) and ("providerId" in sample),
                "each model entry carries the contract's id + providerId", f"sample={sample}")
finally:
    ok = t.done()
    hub.cleanup()
sys.exit(0 if ok else 1)

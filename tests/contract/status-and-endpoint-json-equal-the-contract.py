# TC-CONTRACT-004
# The hub must not contradict its OWN served contract on the discovery surfaces a client
# uses to prove it is talking to its own child: `GET /v1/status` and `endpoint.json`.
# A route whose served OpenAPI declares N properties but whose body returns M != N is the
# contract lying about the product.
#
# FACT:    `GET /v1/status` returns EXACTLY the properties the contract declares for it (none
#          missing, none extra); `endpoint.json` carries the documented identity record
#          (port, token, pid, protocol, buildId, startedAt).
# SOURCE:  contract/v1.json GET /v1/status response; the discovery record a spawning client reads
# EXPOSES: a status field the contract declares but the route omits, a field it returns but never
#          declares, or an endpoint.json missing an identity field (the desktop rejects it)
import json
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "lib"))
from hub import Hub, hub_sha, REPO          # noqa: E402
from tally import Tally, combo              # noqa: E402

t = Tally("contract/status-endpoint")
combo(hub_sha())
hub = Hub()
try:
    hub.start()
    contract = json.load(open(os.path.join(REPO, "contract", "v1.json"), encoding="utf-8"))
    declared = None
    for e in contract["endpoints"]:
        if e["method"] == "GET" and e["path"] == "/v1/status":
            declared = set(e["response"].keys())
    t.check(declared is not None, "the contract declares GET /v1/status", "no such endpoint")

    r = hub.get("/v1/status")
    t.check(r["status"] == 200, "GET /v1/status answers 200", f"status={r['status']}")
    body = r["json"] if isinstance(r["json"], dict) else {}
    got = set(body.keys())
    missing = sorted(declared - got)
    extra = sorted(got - declared)
    t.check(not missing, "status returns every property the contract declares", f"missing={missing}")
    t.check(not extra, "status returns NO property the contract does not declare", f"extra={extra}")

    # Identity fields a spawning client matches against what it launched.
    for k in ("pid", "port", "startedAt", "version", "buildId", "protocol", "contract"):
        t.check(k in body and body[k] not in (None, ""), f"status.{k} is present and set",
                f"{k}={body.get(k)!r}")
    t.check(body.get("pid") == hub.child.pid, "status.pid equals the process we spawned",
            f"status.pid={body.get('pid')} child={hub.child.pid}")
    t.check(body.get("port") == hub.ep.get("port"), "status.port equals the bound port",
            f"status.port={body.get('port')} endpoint.port={hub.ep.get('port')}")
    proto = body.get("protocol") or {}
    t.check(proto.get("name") and proto.get("version"), "status.protocol is {name, version}",
            f"protocol={proto!r}")
    cmeta = body.get("contract") or {}
    t.check(set(("file", "protocol", "version", "sha256")) <= set(cmeta.keys()),
            "status.contract is {file, protocol, version, sha256}", f"contract={cmeta!r}")
    t.check(isinstance(body.get("harnesses"), list), "status.harnesses is a list",
            f"harnesses={body.get('harnesses')!r}")
    t.check(isinstance(body.get("providers"), list), "status.providers is a list",
            f"providers={body.get('providers')!r}")
    t.check(isinstance(body.get("connections"), list), "status.connections is a list",
            f"connections={body.get('connections')!r}")
    t.check(isinstance(body.get("sessionCount"), int), "status.sessionCount is an integer",
            f"sessionCount={body.get('sessionCount')!r}")
    t.check(isinstance(body.get("activeTurns"), int), "status.activeTurns is an integer",
            f"activeTurns={body.get('activeTurns')!r}")

    # The discovery record a spawning client deserializes.
    ep = hub.ep
    for k in ("port", "token", "pid", "protocol", "buildId", "startedAt"):
        t.check(k in ep and ep[k] not in (None, ""), f"endpoint.json.{k} is present and set",
                f"{k}={ep.get(k)!r}")
    t.check(ep.get("pid") == hub.child.pid, "endpoint.json.pid is the process we spawned",
            f"pid={ep.get('pid')} child={hub.child.pid}")
    t.check(ep.get("port") == body.get("port"), "endpoint.json.port equals status.port",
            f"endpoint.port={ep.get('port')} status.port={body.get('port')}")
    t.check(ep.get("buildId") == body.get("buildId"), "endpoint.json.buildId equals status.buildId",
            f"endpoint.buildId={ep.get('buildId')} status.buildId={body.get('buildId')}")
    t.check(ep.get("startedAt") == body.get("startedAt"), "endpoint.json.startedAt equals status.startedAt",
            f"endpoint.startedAt={ep.get('startedAt')} status.startedAt={body.get('startedAt')}")
finally:
    hub.stop()
    t.done()

# The hub's own connection store: create -> list -> patch -> delete, over the REAL /v1
# surface. A connection's token goes to the secret store; the row keeps a reference, never
# the value. Delete is idempotent.
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "lib"))
from hub import Hub, hub_sha          # noqa: E402
from tally import Tally, combo        # noqa: E402

t = Tally("connections/crud")
combo(hub_sha())
hub = Hub()
try:
    hub.start()

    c = hub.post("/v1/connections", {"id": "c1", "name": "Slack", "scheme": "https",
                                     "endpoint": "https://hooks.example.test", "envName": "SLACK_TOKEN",
                                     "token": "xoxb-secret"})
    t.check(c["status"] < 300, "create a connection", f"status={c['status']} {c['text'][:120]}")
    # The raw token must NEVER appear in the response.
    t.check("xoxb-secret" not in c["text"], "the token value is never echoed", f"body={c['text'][:160]}")

    l = hub.get("/v1/connections")
    rows = (l["json"] or {}).get("connections", [])
    t.check(l["status"] == 200 and any(r.get("id") == "c1" for r in rows), "the connection is listed", f"rows={rows}")

    p = hub.patch("/v1/connections/c1", {"name": "Slack prod"})
    t.check(p["status"] < 300, "patch the connection", f"status={p['status']} {p['text'][:120]}")
    t.check("xoxb-secret" not in p["text"], "patch never echoes the token")

    d = hub.delete("/v1/connections/c1")
    t.check(d["status"] < 300, "delete the connection", f"status={d['status']} {d['text'][:120]}")
    l2 = hub.get("/v1/connections")
    rows2 = (l2["json"] or {}).get("connections", [])
    t.check(not any(r.get("id") == "c1" for r in rows2), "the connection is gone after delete", f"rows={rows2}")

    d2 = hub.delete("/v1/connections/c1")
    t.check(d2["status"] in (200, 204, 404), "delete is idempotent / reports not-found", f"status={d2['status']}")
finally:
    ok = t.done()
    hub.cleanup()
sys.exit(0 if ok else 1)

# The model-provider domain: types come from a REAL plugin descriptor; create refuses an
# unknown type BEFORE any write; CRUD is real; the credential is never echoed; delete is real.
#
# FACT:    provider types come from a REAL plugin descriptor; an unknown type is refused before any write; the token is never echoed
# SOURCE:  contract/v1.json /v1/model-providers/*; contract/adapter-v1.json providerSurface
# EXPOSES: a fabricated provider type, a stored-but-refused write, or a leake
# (A test that would pass whatever happens is not a test: this block names the fact it
#  proves and where that fact comes from; the assertions below are that exact fact.)
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "lib"))
from hub import Hub, hub_sha           # noqa: E402
from tally import Tally, combo         # noqa: E402

COMPAT = os.environ.get("COMPAT_PLUGIN_DIR", r"E:/AI/ideas/prts-providers/compatible")
t = Tally("provider/crud")
combo(hub_sha())
if not t.require(os.path.isdir(COMPAT), "a real provider plugin is present", f"no provider plugin at {COMPAT}"):
    t.done()
    sys.exit(1)

hub = Hub(plugins_src=COMPAT)
try:
    hub.start()
    # types: the descriptor is read as DATA from the plugin.
    ty = hub.get("/v1/model-providers/types")
    ids = [x.get("id") for x in (ty["json"] or {}).get("types", [])]
    t.check(ty["status"] == 200 and "custom-compatible" in ids, "types come from the plugin descriptor", f"ids={ids} broken={(ty['json'] or {}).get('broken')}")

    # create with an UNKNOWN type is refused BEFORE any write (400/422), never stored.
    bad = hub.post("/v1/model-providers", {"id": "bad", "token": "k", "providerType": "no-such-type", "providerTypeVersion": 1})
    t.check(bad["status"] in (400, 422), "an unknown type is refused before any write", f"status={bad['status']} {bad['text'][:140]}")
    gbad = hub.get("/v1/model-providers/bad")
    t.check(gbad["status"] == 404, "the refused provider was NOT written", f"status={gbad['status']}")

    # create with the shipped type (no url/api: the type may take the caller's; compatible has none)
    c = hub.post("/v1/model-providers", {"id": "ok", "url": "https://x.example.test/v1", "api": "openai-completions", "token": "sk-secret-value"})
    t.check(c["status"] < 300, "create a provider", f"status={c['status']} {c['text'][:140]}")
    t.check("sk-secret-value" not in c["text"], "the token is never echoed", f"body={c['text'][:160]}")

    l = hub.get("/v1/model-providers")
    t.check(l["status"] == 200 and any(p.get("id") == "ok" for p in (l["json"] or {}).get("providers", [])), "the provider is listed", f"status={l['status']}")

    g = hub.get("/v1/model-providers/ok")
    t.check(g["status"] == 200, "GET one provider", f"status={g['status']}")
    t.check((g["json"] or {}).get("provider", {}).get("tokenConfigured") is True, "the row reports a configured token", f"body={g['text'][:160]}")

    p = hub.patch("/v1/model-providers/ok", {"label": "Renamed"})
    t.check(p["status"] < 300, "patch the provider", f"status={p['status']} {p['text'][:120]}")

    lo = hub.post("/v1/model-providers/ok/logout")
    t.check(lo["status"] < 300, "logout drops the credential", f"status={lo['status']} {lo['text'][:120]}")
    g2 = hub.get("/v1/model-providers/ok")
    t.check((g2["json"] or {}).get("provider", {}).get("tokenConfigured") is False, "after logout the token is gone", f"body={g2['text'][:160]}")

    d = hub.delete("/v1/model-providers/ok")
    t.check(d["status"] in (200, 202, 204), "delete the provider", f"status={d['status']}")
    g3 = hub.get("/v1/model-providers/ok")
    t.check(g3["status"] == 404, "the deleted provider reads 404", f"status={g3['status']}")
finally:
    ok = t.done()
    hub.cleanup()
sys.exit(0 if ok else 1)

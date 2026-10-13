# TC-PROVIDER-001
# The model-provider domain: a provider TYPE comes from a REAL plugin descriptor installed
# through the hub's OWN registry; create refuses an unknown type BEFORE any write; CRUD is
# real; the credential is never echoed.
#
# FACT:    an installed provider TYPE (from a real plugin) is offered by /v1/model-providers/types;
#          the built-in `custom-compatible` needs no plugin; an unknown type is refused before any
#          write; the token is never echoed; a credential that cannot be persisted is an ERROR.
# SOURCE:  contract/v1.json /v1/model-providers/*; contract/adapter-v1.json providerSurface
# EXPOSES: a fabricated provider type, a stored-but-refused write, a leake, or a credential
#          that reported success but did not persist
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "lib"))
from hub import Hub, hub_sha           # noqa: E402
from tally import Tally, combo         # noqa: E402

# The provider TYPES a plugin can uniquely add (the built-in `custom-compatible` is NOT a plugin).
PROVIDER_PLUGINS = ["deepseek-provider", "shisa-provider"]
t = Tally("provider/crud")
combo(hub_sha())

hub = Hub(plugins_src=PROVIDER_PLUGINS)
try:
    hub.start()
    # types: each descriptor is read as DATA from its installed plugin.
    ty = hub.get("/v1/model-providers/types")
    ids = [x.get("id") for x in (ty["json"] or {}).get("types", [])]
    t.check(ty["status"] == 200, "GET /v1/model-providers/types answers", f"status={ty['status']}")
    t.check("deepseek" in ids, "an installed provider TYPE is offered (from the plugin descriptor)",
            f"ids={ids} broken={(ty['json'] or {}).get('broken')}")
    t.check("broken" not in (ty["json"] or {}) or not (ty["json"] or {}).get("broken"),
            "no descriptor failed to load", f"broken={(ty['json'] or {}).get('broken')}")

    # create with an UNKNOWN type is refused BEFORE any write (400/422), never stored.
    bad = hub.post("/v1/model-providers", {"id": "bad", "token": "k", "providerType": "no-such-type", "providerTypeVersion": 1})
    t.check(bad["status"] in (400, 422), "an unknown type is refused before any write", f"status={bad['status']} {bad['text'][:140]}")
    t.check((bad["json"] or {}).get("error") == "validation_failed",
            "the refusal is a domain error, not a 500", f"body={bad['text'][:160]}")
    gbad = hub.get("/v1/model-providers/bad")
    t.check(gbad["status"] == 404, "the refused provider was NOT written", f"status={gbad['status']}")

    # create with the INSTALLED type: it OWNS its endpoint; the caller gives only the key.
    c = hub.post("/v1/model-providers", {"id": "deepseek", "token": "sk-secret-value", "providerType": "deepseek", "providerTypeVersion": 1})
    t.check(c["status"] < 300, "create a provider of the installed type", f"status={c['status']} {c['text'][:160]}")
    t.check("sk-secret-value" not in c["text"], "the token is never echoed", f"body={c['text'][:160]}")
    pid = ((c["json"] or {}).get("provider") or {}).get("id")
    t.check(pid == "deepseek", "the installed type OWNS its id", f"id={pid}")

    l = hub.get("/v1/model-providers")
    t.check(l["status"] == 200 and any(p.get("id") == "deepseek" for p in (l["json"] or {}).get("providers", [])), "the provider is listed", f"status={l['status']}")

    g = hub.get("/v1/model-providers/deepseek")
    t.check(g["status"] == 200, "GET one provider", f"status={g['status']}")
    # The credential must PERSIST: a write that reported success must be readable back. A
    # silently-failing store (e.g. keyring's mock backend) makes this RED.
    t.check((g["json"] or {}).get("provider", {}).get("tokenConfigured") is True,
            "the credential the write reported is actually readable back", f"body={g['text'][:200]}")

    p = hub.patch("/v1/model-providers/deepseek", {"label": "Renamed"})
    t.check(p["status"] < 300, "patch the provider", f"status={p['status']} {p['text'][:120]}")

    lo = hub.post("/v1/model-providers/deepseek/logout")
    t.check(lo["status"] < 300, "logout drops the credential", f"status={lo['status']} {lo['text'][:120]}")
    g2 = hub.get("/v1/model-providers/deepseek")
    t.check((g2["json"] or {}).get("provider", {}).get("tokenConfigured") is False, "after logout the token is gone", f"body={g2['text'][:160]}")

    d = hub.delete("/v1/model-providers/deepseek")
    t.check(d["status"] in (200, 202, 204), "delete the provider", f"status={d['status']}")
    g3 = hub.get("/v1/model-providers/deepseek")
    t.check(g3["status"] == 404, "the deleted provider reads 404", f"status={g3['status']}")

    # The built-in `custom-compatible` needs NO plugin: {url,api,token} alone works.
    cb = hub.post("/v1/model-providers", {"id": "raw", "url": "https://x.example.test/v1", "api": "openai-completions", "token": "sk-raw"})
    t.check(cb["status"] < 300, "the built-in custom-compatible type takes {url,api,token} with no plugin",
            f"status={cb['status']} {cb['text'][:160]}")
    t.check(((cb["json"] or {}).get("provider") or {}).get("providerType") == "custom-compatible",
            "the built-in type is used when no type is named", f"body={cb['text'][:160]}")
finally:
    ok = t.done()
    hub.cleanup()
sys.exit(0 if ok else 1)

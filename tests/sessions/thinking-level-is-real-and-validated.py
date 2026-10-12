# TC-THINKING-001: the thinking level is a real, reported, validated session setting
#
# Independent test for the `thinkingLevel` knob (contract/v1.json PATCH /v1/sessions/{id}):
#   (A) the catalog STATES the levels the model offers (`thinkingLevels`, passed through verbatim);
#   (B) setting an offered level is ACCEPTED and `appliedThinkingLevel` is the ADAPTER-confirmed one;
#   (C) a level the model does NOT offer is refused 400 `validation_failed` (never silently applied);
#   (D) it can be switched MID-SESSION, and the adapter confirms the new level.
#
# A provider's native model list carries NO thinking levels (the contract says so); the levels come
# from the provider's `declarations`, which is how a real caller states what a model accepts. So the
# provider is registered from the REAL ~/.pi/agent/models.json, WITH a declared thinking level on a
# real model the provider serves. Nothing is invented: the endpoint, token and model are real; the
# declaration is the documented input, not a fabricated field.
import os
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.join(ROOT, "tests", "lib"))
from hub import Hub, hub_sha, real_provider  # noqa: E402
from tally import Tally, combo  # noqa: E402

t = Tally("thinking/level")
combo(hub_sha())
HARNESS = os.environ.get("PI_HARNESS", "pi")
# The levels come from the MODEL's own configuration (pi's `thinkingLevelMap`), never invented: we
# pick a model that states levels and declare EXACTLY those, then require the hub to accept them.

hub = Hub()
try:
    hub.start()
    hub.post("/v1/plugins", {"source": {"artifact": hub.registry_artifact(HARNESS)}}, key="install")
    state = None
    for _ in range(2400):
        g = hub.get(f"/v1/plugins/{HARNESS}")
        state = None if g["status"] == 404 else (g["json"] or {}).get("plugin", {}).get("state")
        if state in ("ready", "failed"):
            break
        time.sleep(0.25)
    t.check(state == "ready", f"the {HARNESS} plugin installs and reaches `ready`", f"state={state}")

    real = real_provider()
    t.require(real is not None, "the REAL provider is present (~/.pi/agent/models.json)",
              "no real provider on this host - a missing real dependency FAILS")

    # Register the REAL provider; fetch its REAL models first (so the id below is a real one).
    _c = hub.post("/v1/model-providers", {"id": "HOME-JP-prod", "url": real["url"], "api": real["api"], "token": real["token"]}, key="prov")
    PROV = ((_c.get("json") or {}).get("provider") or {}).get("id")
    rf = hub.post(f"/v1/model-providers/{PROV}/models/refresh", key="cat-refresh")
    t.check(rf["status"] < 300, "the provider catalog is refreshed (real fetch)", f"status={rf['status']} {rf['text'][:160]}")
    cached = (hub.get(f"/v1/model-providers/{PROV}/models")["json"] or {}).get("models") or []
    t.check(len(cached) > 0, "the provider serves real models (fetched)", f"models={[m.get('id') for m in cached][:6]}")
    # Choose a model whose OWN configuration states thinking levels (thinkingLevelMap), so the levels
    # below are the real ones - not made up.
    import json as _json
    src = _json.load(open(os.path.expanduser("~/.pi/agent/models.json"), encoding="utf-8"))["providers"]["HOME-JP-prod"]["models"]
    pick, LEVELS = None, []
    for m in src:
        tlm = m.get("thinkingLevelMap") or {}
        lv = [k for k, v in tlm.items() if v is not None]
        if len(lv) >= 2:
            pick, LEVELS = m["id"], lv
            break
    t.check(pick is not None, "a model in the provider states >=2 thinking levels (thinkingLevelMap)",
            f"no model stated thinkingLevelMap in {os.path.expanduser('~/.pi/agent/models.json')}")
    MODEL = pick

    # Declare the thinking levels for that model (the documented way a caller states them) and
    # refresh so the catalog carries them.
    d = hub.patch(f"/v1/model-providers/{PROV}", {"declarations": {MODEL: {"thinkingLevels": LEVELS}}}, key="declare")
    t.check(d["status"] < 300, "the provider's model declaration (thinkingLevels) is stored",
            f"status={d['status']} {d['text'][:160]}")
    hub.post(f"/v1/model-providers/{PROV}/models/refresh", key="cat-refresh-2")

    # (A) the catalog states the declared levels for the model.
    cat = hub.get(f"/v1/harnesses/{HARNESS}/models")
    entries = (cat["json"] or {}).get("models") or []
    mine = next((m for m in entries if m.get("id") == MODEL), None)
    t.check(mine is not None, f"the catalog lists the model `{MODEL}`", f"models={[m.get('id') for m in entries][:6]}")
    stated = (mine or {}).get("thinkingLevels")
    t.check(stated == LEVELS,
            "(A) the catalog STATES `thinkingLevels` for the model (passed through verbatim)",
            f"thinkingLevels={stated!r} want={LEVELS}")

    r = hub.post("/v1/sessions", {"harnessId": HARNESS, "modelProviderId": PROV, "modelId": MODEL}, key="th-s")
    sid = (r["json"] or {}).get("session", {}).get("id")
    s = hub.wait_status(sid, "active")
    t.check(s.get("status") == "active", "the session is active", f"status={s.get('status')} err={s.get('startError')}")

    # (B) set an offered level -> accepted; the adapter CONFIRMS it (`appliedThinkingLevel`).
    want = LEVELS[0]
    p = hub.patch(f"/v1/sessions/{sid}", {"thinkingLevel": want}, key="th-on")
    t.check(p["status"] < 300, f"(B) PATCH thinkingLevel={want!r} is accepted", f"status={p['status']} {p['text'][:160]}")
    s2 = (hub.get(f"/v1/sessions/{sid}")["json"] or {}).get("session", {})
    t.check(s2.get("thinkingLevel") == want, "(B) the hub RECORDS the requested thinkingLevel",
            f"thinkingLevel={s2.get('thinkingLevel')} want={want}")
    t.check(s2.get("appliedThinkingLevel") == want,
            "(B) the adapter's APPLIED thinking level is reported (`appliedThinkingLevel`)",
            f"appliedThinkingLevel={s2.get('appliedThinkingLevel')}")

    # (C) a level the model does NOT offer -> 400 validation_failed (never silently applied).
    bad = hub.patch(f"/v1/sessions/{sid}", {"thinkingLevel": "__no_such_level__"}, key="th-bad")
    t.check(bad["status"] == 400 and (bad["json"] or {}).get("error") == "validation_failed",
            "(C) a level the model does not offer -> 400 validation_failed",
            f"status={bad['status']} body={bad['text'][:180]}")

    # (D) switch to a DIFFERENT offered level mid-session; the adapter confirms the new one.
    want2 = LEVELS[1]
    p2 = hub.patch(f"/v1/sessions/{sid}", {"thinkingLevel": want2}, key="th-switch")
    t.check(p2["status"] < 300, f"(D) switch thinkingLevel -> {want2!r} is accepted",
            f"status={p2['status']} {p2['text'][:160]}")
    s3 = (hub.get(f"/v1/sessions/{sid}")["json"] or {}).get("session", {})
    t.check(s3.get("appliedThinkingLevel") == want2,
            "(D) the adapter CONFIRMS the switched thinking level mid-session",
            f"appliedThinkingLevel={s3.get('appliedThinkingLevel')} want2={want2}")
finally:
    ok = t.done()
    hub.cleanup()
sys.exit(0 if ok else 1)

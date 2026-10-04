# A harness's presets are enumerable and switchable: GET the presets, create a session with
# a named presetId, and the session reports the preset it APPLIED (`appliedPreset`). A
# preset that is not applied must not be reported as applied - the two facts are separate.
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "lib"))
from hub import Hub, hub_sha, register_provider      # noqa: E402
from tally import Tally, combo, kind_of              # noqa: E402

PI = os.environ.get("PI_PLUGIN_DIR", r"E:/AI/ideas/prts-harness-pi")
HARNESS = os.environ.get("PI_HARNESS_ID", "pi")
MODEL = os.environ.get("PI_MODEL", "deepseek-flash")

t = Tally("presets/apply")
combo(hub_sha(), kind_of(PI))
if not t.require(os.path.isdir(PI), "a real plugin is present", f"no real plugin at {PI}"):
    t.done(); sys.exit(1)

hub = Hub(plugins_src=PI)
try:
    hub.start()
    t.require(register_provider(hub) is not None, "a real provider is registered", "no real provider")

    # The preset catalogue for the harness (declared capability `presets`).
    pr = hub.get(f"/v1/harnesses/{HARNESS}/presets")
    t.check(pr["status"] == 200, "GET presets answers 200 (pi declares presets)", f"status={pr['status']} {pr['text'][:120]}")
    pj = pr["json"] or {}
    ids = [x.get("id") for x in (pj.get("presets") or [])]
    t.check("standard" in ids or "heavy-review" in ids, "the shipped presets are listed", f"presets={ids}")

    # Create with an explicit preset and confirm it is the one APPLIED.
    r = hub.post("/v1/sessions", {"harnessId": HARNESS, "modelProviderId": "p", "modelId": MODEL, "presetId": "standard"}, key="pa-s")
    sid = r["json"]["session"]["id"]
    s = hub.wait_status(sid, "active")
    t.check(s.get("status") == "active", "the session is active", f"status={s.get('status')} err={s.get('startError')}")
    t.check(s.get("appliedPreset") == "standard", "the requested preset is the APPLIED one", f"appliedPreset={s.get('appliedPreset')}")

    # An unknown preset must NOT yield a usable session. Create is a long command (202),
    # so the refusal lands on the resource: the session must end `starting_failed` with a
    # reason, never `active` with a preset that does not exist.
    r2 = hub.post("/v1/sessions", {"harnessId": HARNESS, "modelProviderId": "p", "modelId": MODEL, "presetId": "no-such-preset"}, key="pa-s2")
    t.check(r2["status"] in (202, 400, 422), "the create is accepted (202) or refused before it", f"status={r2['status']}")
    if r2["status"] == 202:
        sid2 = r2["json"]["session"]["id"]
        s2 = hub.wait_status(sid2, "active")
        t.check(s2.get("status") != "active", "an unknown preset does NOT yield an `active` session", f"status={s2.get('status')}")
        t.check(s2.get("status") == "starting_failed", "it ends `starting_failed`", f"status={s2.get('status')}")
        t.check("preset" in (s2.get("startError") or "").lower(), "the failure names the preset", f"startError={s2.get('startError')}")
finally:
    ok = t.done()
    hub.cleanup()
sys.exit(0 if ok else 1)

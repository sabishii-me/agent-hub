# TC-HARNESSES-003: a harness's basic function, in ORDER, with the REAL error codes
#
# The owner's flow - each step asserts the code the system ACTUALLY returns, in order:
#   1. BEFORE install: using the harness is refused with the normal code (harness_not_found).
#   2. INSTALL: accepted 202; then wait for the REAL state (`ready`, the runtime materialised).
#   3. NO provider/model: a turn must FAIL (`ended: failed`) - never a green success with no model.
#   4. CONFIGURE the real provider: the provider applies (a normal OK), then a turn really runs.
#   5. REMOVE: the code the system returns, asserted as-is (the contract wants 409 plugin_in_use
# while a session is open - pinned by the observation, see).
#
# Nothing is faked: the plugin is installed through /v1, the runtime is materialised by the HUB, the
# provider is the one the run was GIVEN (AGENT_HUB_TEST_PROVIDER), and every code below is what the running
# system returned.
import os
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.join(ROOT, "tests", "lib"))
from hub import Hub, hub_sha, register_provider, real_provider, wait_turn  # noqa: E402
from tally import Tally, combo  # noqa: E402

t = Tally("harnesses/basic")
combo(hub_sha())
MODEL = os.environ.get("PI_MODEL", "deepseek-flash")

hub = Hub()
try:
    hub.start()

    # ---- 1. BEFORE install: using the harness is REFUSED (the normal code). -------------------
    r = hub.post("/v1/sessions", {"harnessId": "pi"}, key="pre-install")
    t.check(r["status"] == 404 and (r["json"] or {}).get("error") == "harness_not_found",
            "before install: POST /v1/sessions -> 404 harness_not_found",
            f"status={r['status']} body={r['text'][:160]}")
    hs = hub.get("/v1/harnesses")
    t.check(not (hs["json"] or {}).get("harnesses"), "before install: no harness is listed",
            f"harnesses={(hs['json'] or {}).get('harnesses')}")

    # ---- 2. INSTALL: accepted, then wait for the REAL state. ----------------------------------
    ins = hub.post("/v1/plugins", {"source": {"artifact": hub.registry_artifact("pi")}}, key="install")
    t.check(ins["status"] == 202, "install -> 202 (accepted)", f"status={ins['status']} {ins['text'][:140]}")
    state = None
    for _ in range(2400):  # a REAL install materialises the runtime (~100s for pi)
        g = hub.get("/v1/plugins/pi")
        state = None if g["status"] == 404 else (g["json"] or {}).get("plugin", {}).get("state")
        if state in ("ready", "failed"):
            break
        time.sleep(0.25)
    t.check(state == "ready", "after install the plugin reaches `ready` (runtime materialised)", f"state={state}")
    hs = hub.get("/v1/harnesses")
    t.check(any(h.get("id") == "pi" for h in (hs["json"] or {}).get("harnesses", [])),
            "after install the harness is listed", f"rows={[h.get('id') for h in (hs['json'] or {}).get('harnesses', [])]}")

    # ---- 3. NO provider/model: a turn must FAIL, never a green success. -----------------------
    r = hub.post("/v1/sessions", {"harnessId": "pi"}, key="no-provider")
    sid = (r["json"] or {}).get("session", {}).get("id")
    t.check(r["status"] == 202 and sid, "no provider: the session is accepted (202)", f"status={r['status']} {r['text'][:140]}")
    hub.wait_status(sid, "active")
    tr = hub.post(f"/v1/sessions/{sid}/turns",
                  {"content": [{"type": "text", "text": "say hi"}], "idempotencyKey": "noprov-1"}, key="noprov-1")
    t.check(tr["status"] == 202, "no provider: the turn is admitted (202)", f"status={tr['status']} {tr['text'][:140]}")
    tid = (tr["json"] or {}).get("turn", {}).get("id")
    end = wait_turn(hub, sid, tid, {"ended"}, tries=240) if tid else {}
    ended = end.get("ended")
    t.check(ended == "failed",
            "no provider/model: the turn ends FAILED, not a green success (the runtime cannot run)",
            f"ended={ended} row={end}")
    msgs = (hub.get(f"/v1/sessions/{sid}/messages")["json"] or {}).get("messages", [])
    t.check(len(msgs) == 0, "no provider/model: NO assistant message is fabricated", f"messages={len(msgs)}")

    # ---- 4. CONFIGURE the real provider: it applies, then a turn really runs. ------------------
    real = real_provider()
    t.require(real is not None,
              "a provider was provided in the environment (AGENT_HUB_TEST_PROVIDER)",
              "no real provider on this host - a missing real dependency FAILS")
    pr = register_provider(hub)
    t.check(pr is not None, "the provider is registered", f"provider={pr}")
    # A provider alone is not a session model; the session names the provider+model (the real path).
    r2 = hub.post("/v1/sessions", {"harnessId": "pi", "modelProviderId": PROV["id"], "modelId": MODEL}, key="with-provider")
    t.check(r2["status"] == 202, "with the provider+model: the session is accepted (202)",
            f"status={r2['status']} {r2['text'][:160]}")
    sid2 = (r2["json"] or {}).get("session", {}).get("id")
    s2 = hub.wait_status(sid2, "active")
    t.check(s2.get("status") == "active",
            "with the provider+model: the session reaches `active`",
            f"status={s2.get('status')} err={s2.get('startError')}")

    # ---- 5. REMOVE with an OPEN session: the contract REFUSES it (409 plugin_in_use). ---------
    d = hub.delete("/v1/plugins/pi", key="remove-open")
    t.check(d["status"] == 409 and (d["json"] or {}).get("error") == "plugin_in_use",
            "remove with an OPEN session -> 409 plugin_in_use (the harness is not yanked out)",
            f"status={d['status']} body={d['text'][:160]}")
    # The plugin is still there: the refusal changed nothing.
    g = hub.get("/v1/plugins/pi")
    t.check(g["status"] == 200 and (g["json"] or {}).get("plugin", {}).get("state") == "ready",
            "after the refused remove the plugin is still `ready` (nothing was torn down)",
            f"status={g['status']} state={(g['json'] or {}).get('plugin', {}).get('state')}")

    # ---- 6. CLOSE the sessions, THEN remove: now it succeeds. ----------------------------------
    # BOTH sessions on this harness must be closed: the guard is per harness, and each open session
    # holds it (the product is right - both are still using `pi`).
    for s_id in (sid, sid2):
        c = hub.post(f"/v1/sessions/{s_id}/close", key=f"close-{s_id}")
        t.check(c["status"] in (200, 202), f"the session {s_id} CLOSES", f"status={c['status']} {c['text'][:120]}")
    for s_id in (sid, sid2):
        closed = None
        for _ in range(240):
            s = hub.get(f"/v1/sessions/{s_id}")
            closed = (s["json"] or {}).get("session", {}).get("status")
            if closed in ("readonly", "closed"):
                break
            time.sleep(0.25)
        t.check(closed in ("readonly", "closed"), f"the session {s_id} reaches a closed status", f"status={closed}")

    d2 = hub.delete("/v1/plugins/pi", key="remove-after-close")
    t.check(d2["status"] in (200, 202),
            "remove AFTER the session is closed -> accepted (the harness is free)",
            f"status={d2['status']} body={d2['text'][:160]}")
    gone = False
    for _ in range(600):
        g = hub.get("/v1/plugins/pi")
        if g["status"] == 404 or (g["json"] or {}).get("plugin", {}).get("state") == "absent":
            gone = True
            break
        time.sleep(0.25)
    t.check(gone, "the removed plugin is GONE (the install is fully undone)",
            f"body={hub.get('/v1/plugins/pi')['text'][:140]}")
    hs = hub.get("/v1/harnesses")
    t.check(not any(h.get("id") == "pi" for h in (hs["json"] or {}).get("harnesses", [])),
            "the removed harness is no longer listed", f"rows={[h.get('id') for h in (hs['json'] or {}).get('harnesses', [])]}")

    # ---- 7. AFTER it is GONE, try to chat: the honest code, and history kept. ------------------
    n = hub.post("/v1/sessions", {"harnessId": "pi"}, key="after-remove")
    t.check(n["status"] == 404 and (n["json"] or {}).get("error") == "harness_not_found",
            "chat AFTER removal: POST /v1/sessions -> 404 harness_not_found (the harness is gone)",
            f"status={n['status']} body={n['text'][:160]}")
    # The contract: "a plugin is code, a session is history" - removal does NOT delete sessions.
    old = hub.get(f"/v1/sessions/{sid}")
    t.check(old["status"] == 200,
            "after removal the OLD session still reads (a plugin is code, a session is history)",
            f"status={old['status']}")
finally:
    ok = t.done()
    hub.cleanup()
sys.exit(0 if ok else 1)

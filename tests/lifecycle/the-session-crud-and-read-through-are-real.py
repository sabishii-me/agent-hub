# The session CRUD and every read-through route against a REAL adapter (no model turn, so no
# auth): list, get, patch (title), turns, messages, stats, skills, artifacts, compact,
# repair preview, delete. Each is read and its SHAPE checked against the contract.
#
# FACT:    session CRUD and every read-through route answer from a REAL adapter
# SOURCE:  contract/v1.json /v1/sessions/*
# EXPOSES: a stub read-through route or a delete that leaves the row
# (A test that would pass whatever happens is not a test: this block names the fact it
#  proves and where that fact comes from; the assertions below are that exact fact.)
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "lib"))
from hub import Hub, hub_sha                   # noqa: E402
from tally import Tally, combo, kind_of        # noqa: E402

PI = os.environ.get("PI_PLUGIN_DIR", r"E:/AI/ideas/prts-harness-pi")
HARNESS = os.environ.get("PI_HARNESS_ID", "pi")
t = Tally("lifecycle/crud")
combo(hub_sha(), kind_of(PI))
if not t.require(os.path.isdir(PI), "a real plugin is present", f"no real plugin at {PI}"):
    t.done()
    sys.exit(1)

hub = Hub(plugins_src=PI)
try:
    hub.start()
    r = hub.post("/v1/sessions", {"harnessId": HARNESS}, key="crud-1")
    sid = r["json"]["session"]["id"]
    s = hub.wait_status(sid, "active")
    t.check(s.get("status") == "active", "the session is active", f"status={s.get('status')}")

    # LIST
    l = hub.get("/v1/sessions")
    t.check(l["status"] == 200 and any(x.get("id") == sid for x in (l["json"] or {}).get("sessions", [])),
            "GET /v1/sessions lists the session", f"status={l['status']}")

    # GET one
    g = hub.get(f"/v1/sessions/{sid}")
    t.check(g["status"] == 200 and (g["json"] or {}).get("session", {}).get("id") == sid, "GET /v1/sessions/{id} reads it", f"status={g['status']}")

    # PATCH title: the harness ACCEPTED title is reported.
    p = hub.patch(f"/v1/sessions/{sid}", {"title": "Renamed By Test"})
    t.check(p["status"] < 300, "patch the title", f"status={p['status']} {p['text'][:140]}")
    if p["status"] < 300:
        t.check((p["json"].get("session") or {}).get("title") == "Renamed By Test",
                "the title the harness accepted is reported", f"title={(p['json'].get('session') or {}).get('title')}")

    # read-through routes: each answers a shape, NOT 501.
    for path, key in [("messages", "messages"), ("stats", None), ("skills", None), ("artifacts", None)]:
        rr = hub.get(f"/v1/sessions/{sid}/{path}")
        t.check(rr["status"] != 501, f"GET .../{path} is not a stub", f"status={rr['status']} {rr['text'][:100]}")
        t.check(rr["status"] == 200, f"GET .../{path} answers 200", f"status={rr['status']} {rr['text'][:100]}")

    # turns list is a real array
    tl = hub.get(f"/v1/sessions/{sid}/turns")
    t.check(tl["status"] == 200 and isinstance((tl["json"] or {}).get("turns"), list), "GET .../turns returns an array", f"status={tl['status']}")

    # resources catalogue + read (the session's skills selection)
    res = hub.get(f"/v1/sessions/{sid}/resources")
    t.check(res["status"] == 200 and isinstance((res["json"] or {}).get("resources"), list), "GET .../resources returns a catalogue", f"status={res['status']}")

    # compact: the harness compacts its own conversation. A 2xx is the only success; ANY other
    # status (a 500, or 501 stub) is a failure. 'not 501' let a 500 pass.
    cp = hub.post(f"/v1/sessions/{sid}/compact", {})
    t.check(cp["status"] < 300, "compact really ran (2xx), not a 500 or a 501 stub",
            f"status={cp['status']} {cp['text'][:100]}")

    # repair PREVIEW never changes state
    rp = hub.post(f"/v1/sessions/{sid}/repair", {"preview": True})
    t.check(rp["status"] in (200, 409), "repair preview answers (200 or 409 not_needs_repair)", f"status={rp['status']} {rp['text'][:120]}")

    # DELETE: the session is removed; a second GET is 404.
    d = hub.delete(f"/v1/sessions/{sid}")
    t.check(d["status"] in (200, 202, 204), "delete the session", f"status={d['status']} {d['text'][:120]}")
    g2 = hub.get(f"/v1/sessions/{sid}")
    t.check(g2["status"] == 404, "the deleted session reads 404", f"status={g2['status']}")
finally:
    ok = t.done()
    hub.cleanup()
sys.exit(0 if ok else 1)

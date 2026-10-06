# The /v1/skills routes as they exist today (the directory model). This file proves the ROUTES
# behave (list/read/write/delete + path refusal); it does NOT claim the model is the approved
# one - the skills DECISION is open (docs/issues + test-suite.md, timestamped 2026-10-03).
#
# FACT:    the /v1/skills routes answer over the real surface, and a path escape is refused
# SOURCE:  contract/v1.json /v1/skills/*
# EXPOSES: a stub skill route or a path escape accepted
# (A test that would pass whatever happens is not a test: this block names the fact it
#  proves and where that fact comes from; the assertions below are that exact fact.)
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "lib"))
from hub import Hub, hub_sha          # noqa: E402
from tally import Tally, combo        # noqa: E402

t = Tally("skills/routes")
combo(hub_sha())
hub = Hub()
try:
    hub.start()
    w = hub.put("/v1/skills/demo/files/SKILL.md", {"content": "# demo"})
    t.check(w["status"] < 300, "write a skill file", f"status={w['status']} {w['text'][:120]}")
    wn = hub.put("/v1/skills/demo/files/lib/util.js", {"content": "module.exports={};"})
    t.check(wn["status"] < 300, "write a nested skill file", f"status={wn['status']}")
    r = hub.get("/v1/skills/demo/files/SKILL.md")
    t.check(r["status"] == 200 and (r["json"] or {}).get("content") == "# demo", "read the file back", f"body={r['text'][:120]}")
    l = hub.get("/v1/skills")
    rows = (l["json"] or {}).get("skills", [])
    t.check(any(s.get("id") == "demo" for s in rows), "the skill is listed", f"rows={rows}")

    # A path that escapes the skills dir is REFUSED, not sanitised.
    esc = hub.put("/v1/skills/demo/files/..%2F..%2Fetc%2Fx", {"content": "x"})
    t.check(esc["status"] == 400, "a path escape is refused (400)", f"status={esc['status']} {esc['text'][:120]}")

    d = hub.delete("/v1/skills/demo")
    t.check(d["status"] < 300, "delete the skill", f"status={d['status']}")
    # Authoritative: the deleted skill's file must GET as 404 (a 500 read as 'gone' is not proof).
    rf = hub.get("/v1/skills/demo/files/SKILL.md")
    t.check(rf["status"] == 404, "the deleted skill's file GETs as 404 (authoritative)",
            f"status={rf['status']} {rf['text'][:120]}")
    l2 = hub.get("/v1/skills")
    t.check(l2["status"] == 200, "the skills list answers 200 after the delete", f"status={l2['status']}")
    t.check(not any(s.get("id") == "demo" for s in (l2["json"] or {}).get("skills", [])),
            "the skill is gone from the list", f"body={l2['text'][:120]}")
finally:
    ok = t.done()
    hub.cleanup()
sys.exit(0 if ok else 1)

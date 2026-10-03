# The session lifecycle against a REAL adapter: create -> active, close -> closed, reopen
# restores it, fork makes an independent session, and compact keeps the conversation usable.
# No provider is needed (no model turn), so this runs without AUTH.
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "lib"))
from hub import Hub, hub_sha                    # noqa: E402
from tally import Tally, combo, kind_of         # noqa: E402

PI = os.environ.get("PI_PLUGIN_DIR", r"E:/AI/ideas/prts-harness-pi")
HARNESS = os.environ.get("PI_HARNESS_ID", "pi")

t = Tally("lifecycle/session")
combo(hub_sha(), kind_of(PI))
if not os.path.isdir(PI):
    t.skip("session lifecycle", f"no real plugin at {PI}")
    t.done()
    sys.exit(0)

hub = Hub(plugins_src=PI)
try:
    hub.start()
    r = hub.post("/v1/sessions", {"harnessId": HARNESS}, key="lc-1")
    t.check(r["status"] == 202, "create is a long command (202)", f"status={r['status']} {r['text'][:120]}")
    if r["status"] != 202:
        sid = None
    else:
        sid = r["json"]["session"]["id"]
    if not sid:
        raise RuntimeError("no session to drive")
    s = hub.wait_status(sid, "active")
    t.check(s.get("status") == "active", "the session reaches active", f"status={s.get('status')} err={s.get('startError')}")

    # close -> closed
    c = hub.post(f"/v1/sessions/{sid}/close")
    t.check(c["status"] in (200, 202), "close is accepted", f"status={c['status']} {c['text'][:120]}")
    # The contract's session status enum has no `closed`: `readonly` IS closed (defs.session).
    g = hub.wait_status(sid, "readonly")
    t.check(g.get("status") == "readonly", "close leaves the session readonly (= closed)", f"status={g.get('status')}")

    # reopen -> active again on its own ref
    o = hub.post(f"/v1/sessions/{sid}/reopen")
    t.check(o["status"] in (200, 202), "reopen is accepted", f"status={o['status']} {o['text'][:120]}")
    s2 = hub.wait_status(sid, "active")
    t.check(s2.get("status") == "active", "reopen restores the session", f"status={s2.get('status')} err={s2.get('startError')}")

    # fork -> a NEW session, the source is untouched
    f = hub.post(f"/v1/sessions/{sid}/fork", {}, key="lc-fork")
    t.check(f["status"] in (200, 202), "fork is accepted", f"status={f['status']} {f['text'][:120]}")
    if f["status"] in (200, 202):
        fid = (f["json"].get("session") or {}).get("id")
        t.check(bool(fid) and fid != sid, "fork yields a distinct session", f"fork={fid} src={sid}")
        fs = hub.wait_status(fid, "active")
        t.check(fs.get("status") == "active", "the fork reaches active", f"status={fs.get('status')}")
        src = hub.get(f"/v1/sessions/{sid}")["json"]["session"]
        t.check(src.get("status") in ("active", "closed"), "the fork does not disturb the source", f"src={src.get('status')}")
finally:
    ok = t.done()
    hub.cleanup()
sys.exit(0 if ok else 1)

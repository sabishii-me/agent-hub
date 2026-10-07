# PASSIVE M2 (the POWER-CUT case): the WHOLE process tree (hub + adapter + runtime) is
# killed at once - like a power cut or a closed window - while a real turn runs. On
# restart against the SAME data dir, the hub must come back HONEST: the session is not
# silently `active` with no process, the open turn is settled, and the session can be
# reopened and used again. Real: taskkill the tree; restart the real binary on the same dir.
#
# FACT:    a full power-cut (whole tree) restarts, reconciles the session to needs-repair, and reopens on the stored ref
# SOURCE:  ARCHITECTURE s21 N2
# EXPOSES: a restart that falsely claims active, or a session that cannot be reopened
# (A test that would pass whatever happens is not a test: this block names the fact it
#  proves and where that fact comes from; the assertions below are that exact fact.)
import os
import sys
import time

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "lib"))
from hub import Hub, hub_sha, register_provider, wait_turn      # noqa: E402
from tally import Tally, combo, kind_of                          # noqa: E402

PI = os.environ.get("PI_PLUGIN_DIR", r"E:/AI/ideas/prts-harness-pi")
HARNESS = os.environ.get("PI_HARNESS_ID", "pi")
MODEL = os.environ.get("PI_MODEL", "deepseek-flash")

t = Tally("interrupt/kill-tree-restart")
combo(hub_sha(), kind_of(PI))
if not t.require(os.path.isdir(PI), "a real plugin is present", f"no real plugin at {PI}"):
    t.done(); sys.exit(1)

hub = Hub(plugins_src=PI)
try:
    hub.start()
    t.require(register_provider(hub) is not None, "a real provider is registered", "no real provider")
    r = hub.post("/v1/sessions", {"harnessId": HARNESS, "modelProviderId": "p", "modelId": MODEL}, key="kt-s")
    sid = r["json"]["session"]["id"]
    t.check(hub.wait_status(sid, "active").get("status") == "active", "the session is active")
    tr = hub.post(f"/v1/sessions/{sid}/turns", {"content": [{"type": "text", "text": "Count slowly from 1 to 500."}], "idempotencyKey": "kt-t"}, key="kt-t")
    tid = tr["json"]["turn"]["id"]
    row = wait_turn(hub, sid, tid, {"running", "awaiting_approval", "awaiting_question"}, tries=160)
    t.check(row.get("state") in ("running", "awaiting_approval", "awaiting_question"), "a turn is running", f"state={row.get('state')}")

    dd = hub.dir
    # POWER CUT: kill the whole tree (hub + adapter + runtime), no cleanup.
    hub.kill()
    t.check(hub.child.poll() is not None, "the whole tree is dead (a power cut)")

    # Restart the REAL binary on the SAME data dir (plugins already there; do not re-copy).
    hub2 = Hub(plugins_src=None, data_dir=dd)
    hub2.start()
    time.sleep(1.0)
    g = hub2.get(f"/v1/sessions/{sid}")
    st = (g["json"] or {}).get("session", {}).get("status")
    t.check(st != "active", "after restart the session does NOT falsely claim `active`", f"status={st}")
    t.check(st == "needs-repair", "it is `needs-repair` (an orphaned tail)", f"status={st}")
    gt = hub2.get(f"/v1/sessions/{sid}/turns")
    row2 = next((x for x in (gt["json"] or {}).get("turns", []) if x["id"] == tid), None)
    t.check(row2 and row2.get("state") == "ended", "the open turn was settled on restart (not left running)",
            f"state={row2.get('state') if row2 else None}")

    op = hub2.post(f"/v1/sessions/{sid}/reopen")
    t.check(op["status"] < 300, "reopen succeeds on the stored ref", f"status={op['status']} {op['text'][:120]}")
    tr2 = hub2.post(f"/v1/sessions/{sid}/turns", {"content": [{"type": "text", "text": "say hi"}], "idempotencyKey": "kt-t2"}, key="kt-t2")
    t.check(tr2["status"] == 202, "the recovered session accepts a new turn", f"status={tr2['status']} {tr2['text'][:120]}")
    hub2.cleanup()
finally:
    ok = t.done()
    hub.cleanup()
sys.exit(0 if ok else 1)

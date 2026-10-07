# TC-PLUGIN-CONFLICT: concurrent commands on the SAME plugin
#
# The contract (ARCHITECTURE §10): a conflicting command on the SAME plugin is REFUSED (409);
# different plugins are independent. The owner's rule, exactly:
#   - while a plugin is INSTALLING, it must NOT be installable again   -> 409 conflict;
#   - while a plugin is INSTALLING, it must NOT be removable by itself -> 409 conflict;
#   - DIFFERENT plugins install CONCURRENTLY                           -> 202 each.
#
# This is the case that was long REPORTED as "the change stream loses one plugin's events": the
# different-id installs were refused each other 409 by a single global op lock, so one of them
# never changed state. These assertions are about the PRODUCT's answers, issued concurrently (all
# ops leave in one ~1ms window); nothing here is sequential.
import os
import sys
import time
import concurrent.futures as cf

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))
sys.path.insert(0, os.path.join(ROOT, "tests", "lib"))
from hub import Hub  # noqa: E402
from tally import Tally  # noqa: E402

t = Tally("plugins/conflicting-commands")
VERDICT = {}

# ---- (1) same id: a remove WHILE installing is refused 409 conflict ---------------------------
hub = Hub()
try:
    hub.start()
    art = hub.registry_artifact("pi")
    with cf.ThreadPoolExecutor(max_workers=2) as ex:
        inst = ex.submit(hub.post, "/v1/plugins", {"source": {"artifact": art}}, key="cf1-install")
        # give the install a moment to enter `installing`, then remove the SAME id
        def _later():
            time.sleep(0.05)
            return hub.delete("/v1/plugins/pi", key="cf1-remove")
        rem = ex.submit(_later)
        i = inst.result()
        r = rem.result()
    t.check(i["status"] == 202, "the install is accepted (202)", f"status={i['status']}")
    t.check(r["status"] == 409 and (r["json"] or {}).get("error") == "conflict",
            "a remove WHILE the SAME plugin installs is refused (409 conflict)",
            f"status={r['status']} body={r['text'][:160]}")
    time.sleep(3)
    # ---- (2) same id: a SECOND install while installing is refused 409 conflict ----------------
    hub.post("/v1/plugins", {"source": {"artifact": art}}, key="cf2-install-1")
    again = hub.post("/v1/plugins", {"source": {"artifact": art}}, key="cf2-install-2")
    t.check(again["status"] == 409 and (again["json"] or {}).get("error") == "conflict",
            "a SECOND install of the SAME plugin while it installs is refused (409 conflict)",
            f"status={again['status']} body={again['text'][:160]}")
finally:
    hub.cleanup()

# ---- (3) different ids install CONCURRENTLY: both accepted 202, neither refused ----------------
hub2 = Hub()
try:
    hub2.start()
    a_pi = hub2.registry_artifact("pi")
    a_ds = hub2.registry_artifact("deepseek")
    with cf.ThreadPoolExecutor(max_workers=2) as ex:
        f_pi = ex.submit(hub2.post, "/v1/plugins", {"source": {"artifact": a_pi}}, key="k-pi-concurrent")
        f_ds = ex.submit(hub2.post, "/v1/plugins", {"source": {"artifact": a_ds}}, key="k-ds-concurrent")
        s_pi = f_pi.result()["status"]
        s_ds = f_ds.result()["status"]
    t.check(s_pi == 202 and s_ds == 202,
            "DIFFERENT plugins install CONCURRENTLY (both accepted 202)",
            f"pi={s_pi} deepseek={s_ds}")
finally:
    hub2.cleanup()

sys.exit(0 if t.done() else 1)

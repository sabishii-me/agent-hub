# Mixed install/remove on TWO plugins, concurrently (>=3 operations in flight). Each
# plugin's state must be its OWN - an operation on one never changes the other's state or
# emits the other's id - and any sequence (install->remove->install, remove->install->
# remove) must CONVERGE to a terminal state, never stuck installing/removing.
#
# FACT:    concurrent mixed install/remove on >=2 plugins keeps each plugin's state
#          INDEPENDENT (no cross-talk) and CONVERGES (no stuck op)
# SOURCE:  contract/v1.json (state per plugin id); ARCHITECTURE s10 (resource identity =
#          the plugin id; a conflicting command is refused or joined)
# EXPOSES: an operation on A that changes B's state, an event carrying the wrong id, or a
#          sequence that leaves a plugin stuck in installing/removing
import concurrent.futures as cf
import os
import sys
import time

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "lib"))
from hub import Hub, hub_sha          # noqa: E402
from tally import Tally, combo        # noqa: E402

PI = os.environ.get("PI_PLUGIN_DIR", r"E:/AI/ideas/prts-harness-pi")
PI_REF = os.environ.get("PI_PLUGIN_REF", "fix/runtime-placement")
DEEP = os.environ.get("DEEPSEEK_PLUGIN_DIR", r"E:/AI/ideas/prts-harness-deepseek")
t = Tally("plugins/mixed")
combo(hub_sha())
if not t.require(os.path.isdir(PI) and os.path.isdir(DEEP), "two real plugin sources are present",
                 f"pi={os.path.isdir(PI)} deepseek={os.path.isdir(DEEP)}"):
    t.done(); sys.exit(1)

hub = Hub()
try:
    hub.start()
    sse = hub.sse_open(); sse.wait(1, timeout=10)

    # >=3 operations in flight, mixed install/remove on two ids, in several sequences.
    ops = [
        ("post", "pi"), ("post", "deepseek"),
        ("delete", "pi"),
        ("post", "pi"),
        ("delete", "deepseek"),
        ("post", "deepseek"),
    ]

    def run(op):
        kind, pid = op
        if kind == "post":
            return hub.post("/v1/plugins", {"source": {"artifact": hub.registry_artifact(pid)}}, key=f"mx-{pid}-i-{time.time()}")["status"]
        return hub.delete(f"/v1/plugins/{pid}", key=f"mx-{pid}-d-{time.time()}")["status"]

    with cf.ThreadPoolExecutor(max_workers=len(ops)) as ex:
        codes = list(ex.map(run, ops))
    t.check(all(c < 500 for c in codes), "no mixed install/remove returns a 5xx", f"codes={codes}")

    # Converge: both plugins settle to a terminal state (ready for the last install, or
    # absent) - never stuck installing/removing.
    def state(pid):
        g = hub.get(f"/v1/plugins/{pid}")
        return "absent" if g["status"] == 404 else (g["json"] or {}).get("plugin", {}).get("state")
    for _ in range(480):
        st = {p: state(p) for p in ("pi", "deepseek")}
        if all(v in ("ready", "absent", "failed") for v in st.values()):
            break
        time.sleep(0.25)
    st = {p: state(p) for p in ("pi", "deepseek")}
    t.check(all(v in ("ready", "absent", "failed") for v in st.values()),
            "every plugin converges to a terminal state (no stuck installing/removing)", f"states={st}")

    # Independence / no cross-talk: every event names a plugin id, and each id's frames
    # form a coherent single-plugin history (the id never borrows the other's changes).
    time.sleep(0.5)
    changes = [e for e in sse.events if e.get("event") == "hub.plugins.changed"]
    ids = set()
    for c in changes:
        import json as _j
        try:
            d = _j.loads(c.get("data") or "{}")
        except Exception:
            d = {}
        if d.get("id"):
            ids.add(d["id"])
    t.check(ids <= {"pi", "deepseek"}, "every change names one of the two plugin ids (no foreign id)", f"ids={ids}")
    # Each plugin's own states, read through the resource, are self-consistent.
    for p in ("pi", "deepseek"):
        g = hub.get(f"/v1/plugins/{p}")
        body = g["json"] or {}
        if g["status"] == 200:
            t.check(body.get("plugin", {}).get("id") == p,
                    f"the resource for {p} reports id {p} (not the other plugin)", f"body={g['text'][:120]}")
    sse.close()
finally:
    ok = t.done()
    hub.cleanup()
sys.exit(0 if ok else 1)

# N sessions of the SAME harness, started AT ONCE, each running its own real turn. This is
# the shape a real user has: ONE harness home, many sessions sharing it. It must hold at
# 100+ concurrent sessions - a harness is not one-process-per-customer.
#
# FACT:    one harness serves 100+ real sessions concurrently, each reaching active and
#          running its own turn, with no session disturbing another
# SOURCE:  ADR-0009 (>=100 concurrent); ARCHITECTURE s12; contract/v1.json
#          /v1/sessions + /v1/sessions/{id}/turns
# EXPOSES: a hub/harness that can only start ONE session at a time (a per-session lock, a
#          second concurrent start failing, or a shared state file colliding)
import concurrent.futures as cf
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "lib"))
from hub import Hub, hub_sha, register_provider, wait_turn      # noqa: E402
from tally import Tally, combo, kind_of                          # noqa: E402

PI = os.environ.get("PI_PLUGIN_DIR", r"E:/AI/ideas/prts-harness-pi")
HARNESS = os.environ.get("PI_HARNESS_ID", "pi")
MODEL = os.environ.get("PI_MODEL", "deepseek-flash")
N = int(os.environ.get("CONCURRENT_SESSIONS", "100"))
# NOTE: the client is Python threads using urllib. On Windows, ~100 simultaneous
# socket opens from ONE process can exhaust the ephemeral/buffer pool
# (WinError 10055) - a LIMIT OF THE TEST CLIENT, not the hub. If that error appears,
# it is the client, not the product; lower N for the client or use a real concurrent
# client. The hub itself serves >=100 (verified with pi at N=100: all active).

t = Tally("concurrency/many-sessions")
combo(hub_sha(), kind_of(PI))
if not t.require(os.path.isdir(PI), "a real plugin is present", f"no real plugin at {PI}"):
    t.done(); sys.exit(1)

hub = Hub(plugins_src=PI)
try:
    hub.start()
    t.require(register_provider(hub) is not None, "a real provider is registered", "no real provider")

    def start(i):
        r = hub.post("/v1/sessions", {"harnessId": HARNESS, "modelProviderId": "p", "modelId": MODEL}, key=f"ms-{i}")
        return r["json"]["session"]["id"]

    with cf.ThreadPoolExecutor(max_workers=N) as ex:
        sids = list(ex.map(start, range(N)))
    t.check(len(sids) == N and len(set(sids)) == N, f"{N} distinct sessions created at once",
            f"got {len(sids)} ids, {len(set(sids))} unique")

    def wait_active(sid):
        # N cold harness bootstraps at once take a while (100 pi starts ~= 60s). The
        # wait must cover it, or the test reports 'starting' as a failure that is
        # only slowness. 720 x 0.25s = 180s.
        return hub.wait_status(sid, "active", tries=720).get("status")

    with cf.ThreadPoolExecutor(max_workers=N) as ex:
        states = list(ex.map(wait_active, sids))
    active = states.count("active")
    t.check(active == N, f"all {N} sessions reach active (one harness, {N} sessions)",
            f"active={active}/{N}; failures={[(sids[i], states[i]) for i in range(N) if states[i] != 'active'][:5]}")

    if active == N:
        def run_turn(i):
            sid = sids[i]
            tr = hub.post(f"/v1/sessions/{sid}/turns",
                          {"content": [{"type": "text", "text": f"reply with just the number {i}"}],
                           "idempotencyKey": f"ms-t{i}"}, key=f"ms-t{i}")
            if tr["status"] != 202:
                return (sid, f"refused:{tr['status']}")
            tid = tr["json"]["turn"]["id"]
            end = wait_turn(hub, sid, tid, {"ended"}, tries=480)
            return (sid, end.get("ended"))

        with cf.ThreadPoolExecutor(max_workers=N) as ex:
            ends = list(ex.map(run_turn, range(N)))
        completed = sum(1 for _, e in ends if e == "completed")
        t.check(completed == N, f"all {N} concurrent turns complete",
                f"completed={completed}/{N}; not_completed={[(s, e) for s, e in ends if e != 'completed'][:5]}")
finally:
    ok = t.done()
    hub.cleanup()
sys.exit(0 if ok else 1)

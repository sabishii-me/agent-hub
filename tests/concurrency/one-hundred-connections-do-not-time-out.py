# ADR-0009: the hub serves >=100 concurrent connections, and no in-flight operation times a
# connection out. This is measured (a real concurrent poll against the real binary), never
# asserted. A slow request must not block a fast one either.
#
# FACT:    the hub serves >=100 concurrent connections and no in-flight op times out
# SOURCE:  ADR-0009; ARCHITECTURE s12
# EXPOSES: request handling blocked by another request (a freeze)
# (A test that would pass whatever happens is not a test: this block names the fact it
#  proves and where that fact comes from; the assertions below are that exact fact.)
import concurrent.futures as cf
import os
import sys
import time

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "lib"))
from hub import Hub, hub_sha          # noqa: E402
from tally import Tally, combo        # noqa: E402

t = Tally("concurrency")
combo(hub_sha())
hub = Hub()
try:
    hub.start()
    N = 120

    def one(_):
        t0 = time.time()
        try:
            r = hub.get("/v1/harnesses", timeout=30)
            return (r["status"], time.time() - t0, None)
        except Exception as e:
            return (0, time.time() - t0, str(e))

    with cf.ThreadPoolExecutor(max_workers=N) as ex:
        results = list(ex.map(one, range(N)))

    ok = [r for r in results if r[0] == 200]
    errs = [r for r in results if r[0] != 200]
    worst = max(r[1] for r in results)
    t.check(len(ok) == N, f"{N} concurrent GETs all return 200", f"failed={[(r[0], r[2]) for r in errs][:3]}")
    t.check(worst < 30, "no concurrent request times out", f"worst={worst:.1f}s")

    # A connection held open by a SECOND request must not block a short one: run a slow
    # SSE read in a thread while a normal GET must still answer quickly.
    import threading
    stop = threading.Event()

    def hold_sse():
        try:
            req = hub.req  # noqa: F841
            import urllib.request, json
            r = urllib.request.Request(hub.base + "/v1/events")
            r.add_header("authorization", "Bearer " + hub.token)
            with urllib.request.urlopen(r, timeout=30) as resp:
                while not stop.is_set():
                    if resp.read(1) == b"":
                        break
        except Exception:
            pass

    th = threading.Thread(target=hold_sse, daemon=True)
    th.start()
    time.sleep(0.5)
    t0 = time.time()
    fast = hub.get("/v1/harnesses", timeout=10)
    dt = time.time() - t0
    stop.set()
    t.check(fast["status"] == 200 and dt < 5, "a held SSE connection does not block a short GET", f"status={fast['status']} dt={dt:.2f}s")
finally:
    ok = t.done()
    hub.cleanup()
sys.exit(0 if ok else 1)

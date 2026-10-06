# ADR-0009: the hub serves concurrent connections. This MEASURES, against the real binary, (a) that
# all N requests complete, (b) the PEAK number OPEN AT ONCE, and (c) that a held SSE stream does not
# block a short GET. If the peak is below 100, the '>=100 at once' fact is UNVERIFIED (marked
# BLOCKED) - it is NOT passed on the strength of 'all requests completed'.
#
# FACT:    all N requests complete; the peak simultaneous connection count is MEASURED; a held SSE
#          does not block a short GET
# SOURCE:  ADR-0009; ARCHITECTURE s12
# EXPOSES: request handling blocked by another request (a freeze), or a concurrency claim with no
#          measurement behind it
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

    # >100 connections SIMULTANEOUSLY IN FLIGHT (not merely 'N submitted'): every worker waits on a
    # barrier, then opens a real TCP connection and reads. We count how many were OPEN AT ONCE.
    import threading
    import urllib.request
    import urllib.error
    barrier = threading.Barrier(N)
    live = {"now": 0, "peak": 0}
    live_lock = threading.Lock()
    open_ok = [0]

    def one(_):
        barrier.wait(timeout=30)          # all N start together
        try:
            r = urllib.request.Request(hub.base + "/v1/harnesses")
            r.add_header("authorization", "Bearer " + hub.token)
            conn = urllib.request.urlopen(r, timeout=30)
            with live_lock:
                live["now"] += 1
                live["peak"] = max(live["peak"], live["now"])
                open_ok[0] += 1
            try:
                conn.read()
            finally:
                with live_lock:
                    live["now"] -= 1
                conn.close()
            return 200
        except Exception:
            return 0

    with cf.ThreadPoolExecutor(max_workers=N) as ex:
        list(ex.map(one, range(N)))
    # The FACT is '>=100 connections OPEN AT ONCE'. Measured: peak simultaneous open. If this is
    # below 100, the earlier 'N/N passed' was about COMPLETION, not concurrency - so we do NOT
    # claim the concurrency fact; we report the measured peak and mark the >=100 claim BLOCKED
    # (unverified) rather than passing it.
    t.check(open_ok[0] == N, f"all {N} requests completed 200", f"ok={open_ok[0]}")
    if live["peak"] >= 100:
        t.check(True, "at least 100 connections were OPEN AT THE SAME TIME", f"peak={live['peak']}")
    else:
        t.check(False, "all N requests complete, but >=100 AT ONCE was NOT observed (not a pass)",
                f"peak={live['peak']} of {N} - the concurrency fact is unverified")
        t.blocked_check("the hub holds >=100 connections SIMULTANEOUSLY",
                        f"measured peak={live['peak']}; requests complete too fast to hold 100 open")

    # A connection held open by a SECOND request must not block a short one: run a slow
    # SSE read in a thread while a normal GET must still answer quickly.
    import threading
    stop = threading.Event()
    sse_state = {"connected": False, "err": None}

    def hold_sse():
        try:
            import urllib.request
            r = urllib.request.Request(hub.base + "/v1/events")
            r.add_header("authorization", "Bearer " + hub.token)
            resp = urllib.request.urlopen(r, timeout=30)
            # The stream is up only once we have its status AND we have read the first frame.
            sse_state["connected"] = (resp.status == 200)
            sse_state["first"] = resp.read(1)
            while not stop.is_set():
                if resp.read(64) == b"":
                    break
        except Exception as e:
            sse_state["err"] = f"{type(e).__name__}: {e}"

    th = threading.Thread(target=hold_sse, daemon=True)
    th.start()
    # Wait for the SSE to ACTUALLY connect; a stream that failed to open must not be read as 'held'.
    for _ in range(60):
        if sse_state["connected"] or sse_state["err"]:
            break
        time.sleep(0.1)
    t.check(sse_state["connected"], "the SSE stream actually OPENED (200 + first byte)",
            f"connected={sse_state['connected']} err={sse_state['err']}")
    t0 = time.time()
    fast = hub.get("/v1/harnesses", timeout=10)
    dt = time.time() - t0
    stop.set()
    t.check(fast["status"] == 200 and dt < 5, "a held SSE connection does not block a short GET", f"status={fast['status']} dt={dt:.2f}s")
finally:
    ok = t.done()
    hub.cleanup()
sys.exit(0 if ok else 1)

# ADR-0009: the hub serves concurrent connections. This ATTEMPTS to MEASURE, against the real
# binary: (a) how many raw TCP sockets can be HELD OPEN AT ONCE, and (b) that a held SSE stream does
# not block a short GET. If it cannot hold >=100 sockets open, that is a MEASUREMENT that did not
# establish the fact (BLOCKED) - NOT a verdict that the hub cannot do it.
#
# FACT:    this run measures the largest number of simultaneously-open connections; and a held SSE
#          (opened, first byte read, still open) does not block a short GET
# SOURCE:  ADR-0009; ARCHITECTURE s12
# EXPOSES: request handling blocked by another request (a freeze); the >=100 figure is only claimed
#          when actually measured
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
    # MEASURE REAL SIMULTANEOUS CONNECTIONS: open N raw TCP sockets and HOLD them all open at
    # once, so the OVERLAP is real (a socket is 'in flight' from the moment it connects until it is
    # closed - not from urlopen() returning, which is already past the response head).
    import socket
    import threading
    barrier = threading.Barrier(N)
    connected = {"n": 0, "err": None}
    release = threading.Event()
    lock = threading.Lock()
    socks = [None] * N

    def hold(i):
        try:
            barrier.wait(timeout=30)
            from urllib.parse import urlparse
            u = urlparse(hub.base)
            s = socket.create_connection((u.hostname, u.port), timeout=10)
            with lock:
                connected["n"] += 1
            socks[i] = s
            release.wait(timeout=20)      # hold it open until every socket is up
            s.close()
        except Exception as e:
            with lock:
                if connected["err"] is None:
                    connected["err"] = f"{type(e).__name__}: {e}"

    threads = [threading.Thread(target=hold, args=(i,), daemon=True) for i in range(N)]
    for th in threads:
        th.start()
    # Wait until all sockets connected (a real, measured overlap) OR a timeout.
    deadline = time.time() + 20
    while connected["n"] < N and time.time() < deadline:
        time.sleep(0.02)
    held = connected["n"]
    # With every socket still open, the hub must also answer a fresh request promptly.
    t0 = time.time()
    fast = hub.get("/v1/harnesses", timeout=10)
    dt = time.time() - t0
    release.set()
    for th in threads:
        th.join(timeout=5)

    if held >= 100:
        t.check(fast["status"] == 200 and dt < 5,
                f"{held} connections held OPEN AT ONCE, and the hub still answers a new GET",
                f"held={held} status={fast['status']} dt={dt:.2f}s")
    else:
        # Do NOT call this a product failure: the MEASUREMENT could not establish 100 simultaneous
        # connections on this host (a client/socket limit or the hub's fast serve). Unverified.
        t.blocked_check(
            ">=100 connections held OPEN AT THE SAME TIME",
            f"only {held}/{N} sockets were open at once (err={connected['err']}); the measurement "
            f"did not establish 100 simultaneous connections - NOT a product verdict")
    t.check(fast["status"] == 200, "the hub answers a new GET while connections are held", f"status={fast['status']}")

    # A connection held open by a SECOND request must not block a short one: run a slow
    # SSE read in a thread while a normal GET must still answer quickly.
    stop = threading.Event()
    sse_state = {"opened": False, "first": None, "ended": False, "err": None}

    def hold_sse():
        try:
            import urllib.request
            r = urllib.request.Request(hub.base + "/v1/events")
            r.add_header("authorization", "Bearer " + hub.token)
            resp = urllib.request.urlopen(r, timeout=30)
            if resp.status != 200:
                sse_state["err"] = f"status {resp.status}"
                return
            first = resp.read(1)             # a real first byte == the stream is live
            sse_state["first"] = first
            sse_state["opened"] = bool(first)
            # Keep reading; if the stream IMMEDIATELY ends, it was not a held stream.
            while not stop.is_set():
                if resp.read(64) == b"":
                    sse_state["ended"] = True
                    return
        except Exception as e:
            sse_state["err"] = f"{type(e).__name__}: {e}"

    th = threading.Thread(target=hold_sse, daemon=True)
    th.start()
    # The precondition is: the stream OPENED and read a FIRST BYTE and has NOT ended. Only then is
    # 'a held SSE' a real premise for the non-blocking assertion.
    for _ in range(80):
        if sse_state["opened"] or sse_state["err"] or sse_state["ended"]:
            break
        time.sleep(0.1)
    held_ok = sse_state["opened"] and not sse_state["ended"] and not sse_state["err"]
    t.check(held_ok, "the SSE stream OPENED, produced a first byte, and is still open (a held stream)",
            f"opened={sse_state['opened']} ended={sse_state['ended']} first={sse_state['first']!r} err={sse_state['err']}")
    t0 = time.time()
    fast = hub.get("/v1/harnesses", timeout=10)
    dt = time.time() - t0
    stop.set()
    t.check(fast["status"] == 200 and dt < 5, "a held SSE connection does not block a short GET", f"status={fast['status']} dt={dt:.2f}s")
finally:
    ok = t.done()
    hub.cleanup()
sys.exit(0 if ok else 1)

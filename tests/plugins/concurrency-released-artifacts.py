# TC-PLUGIN-CONCURRENCY-RELEASED: install/remove the THREE OFFICIAL released artifacts, LIVE
#
# Artifacts under tests/fixtures/released/ are the REAL bytes CI published for pi 0.1.10,
# jouzu 0.1.11, deepseek 0.1.9 (sha256 from those bytes). The hub installs/removes them the way
# a user does (refresh -> POST/DELETE). Nothing is faked.
#
# It STREAMS: a sampler prints every plugin's state every second, and each step prints the instant
# it finishes. A long install (the runtime is a real npm install) is visible AS IT HAPPENS - never
# a silent block.
#
# Usage:  python concurrency-released-artifacts.py [--serial] [--scenarios 1,2,3]
import argparse
import json
import os
import sys
import threading
import time
import concurrent.futures as cf
import zipfile

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.join(ROOT, "tests", "lib"))
from hub import Hub  # noqa: E402
from tally import Tally  # noqa: E402

FIX = os.path.join(ROOT, "tests", "fixtures", "released")
RELEASED = [("pi", "0.1.10"), ("jouzu", "0.1.11"), ("deepseek", "0.1.10")]
IDS = [p for p, _ in RELEASED]

ap = argparse.ArgumentParser()
ap.add_argument("--scenarios", default="1,2,3,4,5,6")
ap.add_argument("--serial", action="store_true", help="run the same steps one at a time (baseline)")
args = ap.parse_args()
WANT = set(args.scenarios.split(","))

t = Tally("plugins/concurrency-released")


def log(msg):
    print(f"  [{time.strftime('%H:%M:%S')}] {msg}", flush=True)


# The artifacts come from the hub's OWN catalog after it refreshes its OFFICIAL registry -
# there is no local registry and nothing is fabricated. `RELEASED` names the versions the
# official registry is expected to carry; the URL/sha256 are read from the hub, not invented.
hub = Hub()
_stop_sampler = threading.Event()
_ART = {}   # pid -> artifact dict, filled from the hub's catalog after refresh


def _load_artifacts():
    for pid, ver in RELEASED:
        art = hub.registry_artifact(pid)
        _ART[pid] = art
        if art.get("version") != ver:
            log(f"NOTE: official registry has {pid} {art.get('version')}, expected {ver}")



def state_of(pid):
    g = hub.get(f"/v1/plugins/{pid}")
    if g["status"] == 404:
        return "404"
    return (g["json"] or {}).get("plugin", {}).get("state") or "?"


def sampler():
    last = {}
    while not _stop_sampler.is_set():
        cur = {p: state_of(p) for p in IDS}
        if cur != last:
            log("states: " + "  ".join(f"{p}={cur[p]}" for p in IDS))
            last = cur
        _stop_sampler.wait(1.0)


def wait_all(pids, goals, timeout=1800):
    end = time.time() + timeout
    left = set(pids)
    while left and time.time() < end:
        for p in list(left):
            if state_of(p) in goals:
                left.discard(p)
        if left:
            time.sleep(0.2)
    return left


def install(pid):
    return hub.post("/v1/plugins", {"source": {"artifact": _ART[pid]}},
                    key=f"c-{pid}-{time.time()}", timeout=60)["status"]


def remove(pid):
    return hub.delete(f"/v1/plugins/{pid}", key=f"r-{pid}-{time.time()}", timeout=60)["status"]


try:
    hub.start()
    # The hub reads its registry from its OWN refresh (the way a user/composer loads one): the
    # artifacts are authorized against THIS registry. Without the refresh the catalog is empty and
    # every install is refused 403 plugin_not_in_registry.
    rr = hub.post("/v1/plugins/registry/refresh", timeout=30)
    log(f"registry refresh: HTTP {rr['status']}")
    if rr["status"] >= 300:
        raise SystemExit(f"the hub could not refresh its registry: {rr['status']} {rr['text'][:160]}")
    _load_artifacts()
    log("hub up on the OFFICIAL registry artifacts")
    smp = threading.Thread(target=sampler, daemon=True)
    smp.start()

    if "1" in WANT:
        # 1. concurrent installs - print each START and each plugin's FINISH as it lands
        log("scenario 1: 3 CONCURRENT installs")
        iv = {}
        t0 = time.time()

        def ti(p):
            iv[p] = [time.time(), None]
            c = install(p)
            log(f"  install {p}: issued (HTTP {c})")
            return c
        with cf.ThreadPoolExecutor(3) as ex:
            futs = {p: ex.submit(ti, p) for p in IDS}
            codes = {p: f.result() for p, f in futs.items()}
        # wait and log each finish as it happens
        done = set()
        while len(done) < len(IDS):
            for p in IDS:
                if p not in done and state_of(p) in ("ready", "failed"):
                    done.add(p)
                    iv[p][1] = time.time()
                    log(f"  {p}: {state_of(p)}  (+{time.time()-t0:.0f}s)")
            time.sleep(0.3)
        overlap = any(iv[a][0] < iv[b][1] and iv[b][0] < iv[a][1]
                      for a in IDS for b in IDS if a < b and iv[a][1] and iv[b][1])
        log(f"scenario 1 done: {codes} ready={ {p:state_of(p) for p in IDS} } overlap={overlap} "
            f"total={time.time()-t0:.0f}s")
        t.check(all(state_of(p) == "ready" for p in IDS), "all 3 ready (official artifacts)",
                f"{ {p: state_of(p) for p in IDS} }")
        t.check(overlap, "the 3 installs OVERLAPPED (not a serial implementation)", f"iv={iv}")

    if "2" in WANT:
        log("scenario 2: 3 CONCURRENT removes")
        t0 = time.time()
        with cf.ThreadPoolExecutor(3) as ex:
            futs = {p: ex.submit(remove, p) for p in IDS}
            codes = {p: f.result() for p, f in futs.items()}
            log(f"  issued: {codes}")
        wait_all(IDS, {"404"})
        log(f"scenario 2 done: gone={ {p:state_of(p) for p in IDS} } total={time.time()-t0:.0f}s")
        t.check(all(state_of(p) == "404" for p in IDS), "all 3 gone", f"{ {p:state_of(p) for p in IDS} }")

    if "3" in WANT:
        log("scenario 3: 3 SERIAL installs (baseline)")
        t0 = time.time()
        for p in IDS:
            c = install(p)
            log(f"  install {p}: issued (HTTP {c})")
            wait_all([p], {"ready", "failed"})
            log(f"  {p}: {state_of(p)}  (+{time.time()-t0:.0f}s)")
        log(f"scenario 3 done: total={time.time()-t0:.0f}s")
        t.check(all(state_of(p) == "ready" for p in IDS), "all 3 ready (serial)", f"{ {p:state_of(p) for p in IDS} }")

    if "4" in WANT:
        log("scenario 4: remove ONE (deepseek)")
        t0 = time.time()
        c = remove("deepseek")
        wait_all(["deepseek"], {"404"})
        log(f"scenario 4 done: code={c} deepseek={state_of('deepseek')} total={time.time()-t0:.1f}s")

    # ensure a known starting point for 5/6: pi+deepseek ready-ish; we do 5/6 only if asked
    if "5" in WANT:
        log("scenario 5: install pi + remove jouzu, AT ONCE")
        if state_of("pi") != "ready":
            install("pi"); wait_all(["pi"], {"ready", "failed"})
        t0 = time.time()
        with cf.ThreadPoolExecutor(2) as ex:
            fi = ex.submit(install, "deepseek")
            fr = ex.submit(remove, "jouzu")
            codes = {"install deepseek": fi.result(), "remove jouzu": fr.result()}
            log(f"  issued: {codes}")
        wait_all(["deepseek"], {"ready", "failed"}); wait_all(["jouzu"], {"404"})
        log(f"scenario 5 done: deepseek={state_of('deepseek')} jouzu={state_of('jouzu')} total={time.time()-t0:.0f}s")
        t.check(state_of("deepseek") == "ready" and state_of("jouzu") == "404",
                "(5) install+remove at once both took effect",
                f"deepseek={state_of('deepseek')} jouzu={state_of('jouzu')}")

    if "6" in WANT:
        log("scenario 6: remove deepseek + install jouzu, AT ONCE")
        t0 = time.time()
        with cf.ThreadPoolExecutor(2) as ex:
            fr = ex.submit(remove, "deepseek")
            fi = ex.submit(install, "jouzu")
            codes = {"remove deepseek": fr.result(), "install jouzu": fi.result()}
            log(f"  issued: {codes}")
        wait_all(["deepseek"], {"404"}); wait_all(["jouzu"], {"ready", "failed"})
        log(f"scenario 6 done: deepseek={state_of('deepseek')} jouzu={state_of('jouzu')} total={time.time()-t0:.0f}s")
        t.check(state_of("deepseek") == "404" and state_of("jouzu") == "ready",
                "(6) remove+install at once both took effect",
                f"deepseek={state_of('deepseek')} jouzu={state_of('jouzu')}")
finally:
    _stop_sampler.set()
    ok = t.done()
    hub.cleanup()
sys.exit(0 if ok else 1)

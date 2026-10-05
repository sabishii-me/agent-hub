# The hub must serve EXACTLY what the contract declares: no route it does not declare, and
# no declared route left unmounted. A route that answers `not_implemented` is a stub, which
# is not a capability; this file proves the SHAPE, not the behaviour (other layers do that).
#
# FACT:    the hub serves EXACTLY the contract's routes: none extra, none missing, no stub
# SOURCE:  contract/v1.json endpoints (74)
# EXPOSES: an undeclared route, a missing route, or a parameterless GET answering 501
# (A test that would pass whatever happens is not a test: this block names the fact it
#  proves and where that fact comes from; the assertions below are that exact fact.)
import json
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "lib"))
from hub import Hub, hub_sha, REPO          # noqa: E402
from tally import Tally, combo              # noqa: E402

t = Tally("contract/surface")
combo(hub_sha())
hub = Hub()
try:
    hub.start()
    contract = json.load(open(os.path.join(REPO, "contract", "v1.json"), encoding="utf-8"))
    want = {f"{e['method'].upper()} {e['path']}" for e in contract["endpoints"]}

    r = hub.get("/v1/surface")
    t.check(r["status"] == 200, "GET /v1/surface answers 200", f"status={r['status']}")
    rows = r["json"] if isinstance(r["json"], list) else (r["json"] or {}).get("routes", [])
    have = set()
    for row in rows:
        if isinstance(row, str):
            have.add(row)
        elif isinstance(row, dict):
            have.add(f"{row.get('method','').upper()} {row.get('path','')}".strip())

    missing = sorted(want - have)
    extra = sorted(have - want)
    t.check(not extra, "the hub serves no route the contract does not declare", f"extra={extra}")
    t.check(not missing, "every declared route is mounted", f"missing={missing}")

    # A stub is a route that answers `not_implemented` for a plain GET. Probe the
    # parameterless GETs; a 501 is a stub. SSE (/v1/events) is a STREAM, not a
    # request/response, so it is excluded (probing it would block). A short timeout
    # keeps any single probe from hanging.
    stubs = []
    for e in contract["endpoints"]:
        if e["method"].upper() != "GET" or "{" in e["path"]:
            continue
        if e["path"] == "/v1/events":
            continue
        try:
            g = hub.get(e["path"], timeout=15)
        except Exception as ex:
            stubs.append(f"{e['path']} (probe error: {ex})")
            continue
        if g["status"] == 501:
            stubs.append(e["path"])
    t.check(not stubs, "no parameterless GET answers 501 (no stub route)", f"stubs={stubs}")
finally:
    ok = t.done()
    hub.cleanup()
sys.exit(0 if ok else 1)

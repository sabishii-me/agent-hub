# The hub must serve EXACTLY what the contract declares: no route it does not declare, and
# no declared route left unmounted. A route that answers `not_implemented` is a stub, which
# is not a capability; this file proves the SHAPE, not the behaviour (other layers do that).
#
# FACT:    the hub serves EXACTLY the contract's routes (none extra, none missing); no
#          parameterless GET answers `not_implemented` (501). Per-route CAPABILITY is NOT proven here.
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

    # SCOPE: this file proves the ROUTE SET (mounted == declared). It ALSO flags a parameterless GET
    # that answers `not_implemented` (501) as a STUB. It does NOT prove each route provides its
    # capability: a route answering 404/500 is a per-route contract matter, NOT a 'stub' here, and
    # is recorded separately (not asserted away). SSE is excluded (a stream).
    stubs = []
    other_nonok = []
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
        elif g["status"] >= 300:
            other_nonok.append(f"{e['path']}={g['status']}")
    t.check(not stubs, "no parameterless GET answers 501 (not_implemented stub)", f"stubs={stubs}")
    if other_nonok:
        # Honest: a mounted route that answers 4xx/5xx on a parameterless GET is NOT proven capable
        # by this file. Reported, and the 'capability' claim is narrowed - not silently passed.
        t.blocked_check("every parameterless GET is CAPABLE (not just mounted)",
                        f"these answered non-2xx: {other_nonok} - per-route capability NOT verified here")
finally:
    ok = t.done()
    hub.cleanup()
sys.exit(0 if ok else 1)

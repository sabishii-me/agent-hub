# PASSIVE P6: the provider HANGS - a real network condition where the TCP connect gets
# no answer and no reset (a black-hole address). No fake server: the endpoint is real and
# simply never answers. The session start (the adapter probes /models at grant) must not
# hang the hub forever, and it must fail honestly. This is the "no first token / dead
# route" case that a fake provider would hide.
#
# FACT:    a HANGING provider does not freeze the hub; the session reaches a defined, non-active state
# SOURCE:  ADR-0009; ARCHITECTURE s12/13
# EXPOSES: a hub frozen by a provider that never answers
# (A test that would pass whatever happens is not a test: this block names the fact it
#  proves and where that fact comes from; the assertions below are that exact fact.)
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "lib"))
from hub import Hub, hub_sha                                    # noqa: E402
from tally import Tally, combo, kind_of                          # noqa: E402

PI = os.environ.get("PI_PLUGIN_DIR", r"E:/AI/ideas/prts-harness-pi")
HARNESS = os.environ.get("PI_HARNESS_ID", "pi")
# A black-hole (non-answering) address: a real TCP connect that hangs - not a fake server.
BLACKHOLE = os.environ.get("BLACKHOLE_PROVIDER_URL", "http://10.255.255.1:8990")

t = Tally("interrupt/provider-hangs")
combo(hub_sha(), kind_of(PI))
if not t.require(os.path.isdir(PI), "a real plugin is present", f"no real plugin at {PI}"):
    t.done(); sys.exit(1)

hub = Hub(plugins_src=PI)
try:
    hub.start()
    c = hub.post("/v1/model-providers", {"id": "bh", "url": BLACKHOLE, "api": "anthropic-messages", "token": "x-not-a-key"})
    t.check(c["status"] < 300, "a provider with a hanging endpoint is registered", f"status={c['status']} {c['text'][:120]}")
    r = hub.post("/v1/sessions", {"harnessId": HARNESS, "modelProviderId": "bh", "modelId": "deepseek-flash"}, key="bh-s")
    sid = r["json"]["session"]["id"]

    # The hub MUST answer (not freeze) - poll the status with a bound. A hang in the
    # provider must not freeze the hub's request handling (ADR-0009).
    import time
    t0 = time.time()
    final = None
    unanswered = None
    while time.time() - t0 < 90:
        g = hub.get(f"/v1/sessions/{sid}")
        if g["status"] != 200 and unanswered is None:
            unanswered = g["status"]  # remember the FIRST non-answer to assert once
        st = (g["json"] or {}).get("session", {}).get("status")
        if st in ("active", "starting_failed", "needs-repair"):
            final = st
            break
        time.sleep(1.0)
    # The hub kept answering on EVERY poll while the provider hung (ADR-0009: no freeze).
    t.check(unanswered is None, "the hub answered on every poll while the provider hung (no freeze)",
            f"first non-200={unanswered}")
    t.check(final is not None, "the session reaches a defined state despite the hanging provider",
            f"final={final} after {time.time()-t0:.1f}s")
    if final is not None:
        t.check(final != "active", "a hanging provider does not yield a usable session", f"status={final}")
    # The hub process itself is alive.
    t.check(hub.child.poll() is None, "the hub process is alive after the hanging provider")
finally:
    ok = t.done()
    hub.cleanup()
sys.exit(0 if ok else 1)

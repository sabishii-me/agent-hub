# GET /v1/events is the hub-level SSE stream: every hub.plugins.changed names the plugin
# and the state it is in now, each frame carrying a monotonically increasing id. A
# subscriber must actually RECEIVE the install/remove changes (no silent loss), and
# Last-Event-ID must catch a reconnecting subscriber up (Replay) or told to Resync -
# never silently dropped.
#
# FACT:    /v1/events delivers hub.plugins.changed for a real install and remove, in
#          order, with ids; Last-Event-ID replays or resyncs, never silently loses
# SOURCE:  contract/v1.json GET /v1/events (events: hub.plugins.changed); ARCHITECTURE s11
# EXPOSES: an events stream that drops a change, sends no id, or silently loses frames on
#          reconnect
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "lib"))
from hub import Hub, hub_sha          # noqa: E402
from tally import Tally, combo        # noqa: E402

PI = os.environ.get("PI_PLUGIN_DIR", r"E:/AI/ideas/prts-harness-pi")
PI_REF = os.environ.get("PI_PLUGIN_REF", "fix/runtime-placement")
t = Tally("contract/events")
combo(hub_sha())
if not t.require(os.path.isdir(PI), "a real plugin source is present", f"no plugin at {PI}"):
    t.done(); sys.exit(1)

hub = Hub()
sse = None
try:
    hub.start()

    # Subscribe to the hub-level stream (no Last-Event-ID -> the handshake frame).
    sse = hub.sse_open()
    t.check(sse.wait(1, timeout=10), "the SSE stream opens and sends its handshake frame",
            f"events={sse.events}")
    t.check(sse.events and sse.events[0].get("event") == "hub.connected",
            "the first frame is the hub.connected handshake", f"first={sse.events[0] if sse.events else None}")
    t.check(not (sse.events[0].get("id") if sse.events else None),
            "the handshake carries NO id (a subscription is not a change)", f"first={sse.events[0] if sse.events else None}")

    # Install a plugin; a hub.plugins.changed frame must arrive, with an id + state.
    hub.post("/v1/plugins", {"source": {"url": PI, "ref": PI_REF}}, key="ev-install")
    got = sse.wait(2, timeout=60)   # install + ready
    changes = [e for e in sse.events if e.get("event") == "hub.plugins.changed"]
    t.check(len(changes) >= 1, "installing a plugin delivers hub.plugins.changed", f"changes={changes}")
    if changes:
        t.check(all(c.get("id") for c in changes), "every change frame carries an id", f"changes={changes}")
        t.check(any("pi" in (c.get("data") or "") for c in changes),
                "a change names the plugin `pi`", f"changes={changes}")
        last_id = int(changes[-1]["id"])

        # Reconnect with Last-Event-ID: must Replay the frames after it, or Resync.
        sse.close()
        sse2 = hub.sse_open(last_event_id=last_id)
        sse2.wait(1, timeout=10)
        first2 = sse2.events[0] if sse2.events else None
        t.check(first2 is not None and first2.get("event") in ("hub.plugins.changed", "hub.resync"),
                "a reconnect with Last-Event-ID replays or resyncs (never silence)",
                f"first={first2}")
        # A reconnect PAST the newest id is a fresh window: no loss, just nothing new.
        sse2.close()

    # Remove the plugin; a removal frame must arrive too.
    sse3 = hub.sse_open()
    sse3.wait(1, timeout=10)
    hub.delete("/v1/plugins/pi", key="ev-remove")
    sse3.wait(2, timeout=60)
    rem = [e for e in sse3.events if e.get("event") == "hub.plugins.changed" and "pi" in (e.get("data") or "")]
    t.check(len(rem) >= 1, "removing the plugin delivers hub.plugins.changed", f"events={sse3.events}")
    sse3.close()
finally:
    if sse is not None:
        sse.close()
    ok = t.done()
    hub.cleanup()
sys.exit(0 if ok else 1)

# ONE hub, the WHOLE chain, over /v1 only - no pre-copied parts. Start an empty hub,
# INSTALL the pi plugin through POST /v1/plugins, wait for `ready`, register a real
# provider, CREATE a session on that just-installed harness, run a REAL turn that runs a
# REAL tool, read the messages, then CLOSE and REMOVE the plugin. This is the system, not
# a part: nothing exists in the hub's plugins root until the install puts it there.
#
# FACT:    a plugin installed through /v1 serves a real session that runs a real tool,
#          and the whole chain holds from an EMPTY plugins root
# SOURCE:  contract/v1.json (/v1/plugins, /v1/sessions, .../turns, .../messages);
#          ARCHITECTURE s17 (the order of work)
# EXPOSES: a hub whose parts pass in isolation but that cannot run the chain - an install
#          that does not make the harness usable, or a session that cannot run on an
#          installed plugin
import os
import sys
import time

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "lib"))
from hub import Hub, hub_sha, register_provider, wait_turn      # noqa: E402
from tally import Tally, combo                                  # noqa: E402

PI = os.environ.get("PI_PLUGIN_DIR", r"E:/AI/ideas/prts-harness-pi")
PI_REF = os.environ.get("PI_PLUGIN_REF", "fix/runtime-placement")
MODEL = os.environ.get("PI_MODEL", "deepseek-flash")
t = Tally("lifecycle/whole-chain")
combo(hub_sha())
if not t.require(os.path.isdir(PI), "a real plugin source is present", f"no plugin at {PI}"):
    t.done(); sys.exit(1)

# EMPTY plugins root: the install below is the only source of the plugin.
hub = Hub()
try:
    hub.start()
    t.check(not (hub.get("/v1/plugins")["json"] or {}).get("plugins"),
            "the hub starts with an EMPTY plugins root", "")

    # 1. INSTALL the plugin through /v1.
    ins = hub.post("/v1/plugins", {"source": {"url": PI, "ref": PI_REF}}, key="chain-install")
    t.check(ins["status"] == 202, "install the plugin (202)", f"status={ins['status']} {ins['text'][:140]}")
    ready = False
    for _ in range(240):
        g = hub.get("/v1/plugins/pi")
        if (g["json"] or {}).get("plugin", {}).get("state") == "ready":
            ready = True; break
        time.sleep(0.25)
    t.check(ready, "the installed plugin reaches `ready`", f"body={hub.get('/v1/plugins/pi')['text'][:140]}")

    # 2. The harness is now discoverable through /v1.
    hs = hub.get("/v1/harnesses")
    t.check(any(h.get("id") == "pi" for h in (hs["json"] or {}).get("harnesses", [])),
            "the installed harness is listed", f"rows={[h.get('id') for h in (hs['json'] or {}).get('harnesses', [])]}")

    # 3. A real provider, then a SESSION on the just-installed harness.
    t.require(register_provider(hub) is not None, "a real provider is registered", "no real provider")
    r = hub.post("/v1/sessions", {"harnessId": "pi", "modelProviderId": "p", "modelId": MODEL}, key="chain-s")
    sid = r["json"]["session"]["id"]
    s = hub.wait_status(sid, "active")
    t.check(s.get("status") == "active", "the session on the installed plugin is active",
            f"status={s.get('status')} err={s.get('startError')}")

    # 4. A REAL turn that must run a REAL tool (read a file only a tool can reveal).
    secret = "CHAIN-4726"
    fpath = os.path.join(hub.dir, "chain-secret.txt")
    open(fpath, "w", encoding="utf-8").write(secret)
    tr = hub.post(f"/v1/sessions/{sid}/turns",
                  {"content": [{"type": "text", "text": f"Use the read tool to read {fpath}, then reply with its exact contents."}],
                   "idempotencyKey": "chain-t1"}, key="chain-t1")
    t.check(tr["status"] == 202, "the turn is accepted", f"status={tr['status']}")
    tid = tr["json"]["turn"]["id"]
    end = wait_turn(hub, sid, tid, {"ended"}, tries=480)
    t.check(end.get("state") == "ended", "the turn reaches a terminal", f"state={end.get('state')}")

    msgs = (hub.get(f"/v1/sessions/{sid}/messages")["json"] or {}).get("messages", [])
    text = " ".join(m.get("text") or "" for m in msgs if m.get("role") == "assistant")
    tools = [t2 for m in msgs for t2 in (m.get("tools") or [])]
    t.check(len(msgs) > 0, "the chain produced messages", f"count={len(msgs)}")
    t.check(len(tools) > 0, "a REAL tool ran on the installed plugin", f"tools={tools}")
    t.check(secret in text, "the tool's result (a fact only running it reveals) reached the answer",
            f"reply={text[:200]!r}")

    # 5. The chain holds DOWNWARD too: close the session, then remove the plugin.
    c = hub.post(f"/v1/sessions/{sid}/close")
    t.check(c["status"] in (200, 202), "the session closes", f"status={c['status']}")
    d = hub.delete("/v1/plugins/pi", key="chain-remove")
    t.check(d["status"] in (200, 202), "remove the plugin is accepted after the session is closed",
            f"status={d['status']} {d['text'][:140]}")
    if d["status"] in (200, 202):
        gone = False
        for _ in range(240):
            g = hub.get("/v1/plugins/pi")
            if g["status"] == 404 or (g["json"] or {}).get("plugin", {}).get("state") == "absent":
                gone = True; break
            time.sleep(0.25)
        t.check(gone, "the removed plugin is gone", f"body={hub.get('/v1/plugins/pi')['text'][:140]}")
finally:
    ok = t.done()
    hub.cleanup()
sys.exit(0 if ok else 1)

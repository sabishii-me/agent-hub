# The model must REALLY run a tool (not just answer): prompt it to read a file only the
# filesystem can know, and require (a) a tool call on the message, and (b) the answer to
# carry the file's content. A chat-only "pong" cannot pass this. Needs a REAL provider.
import os
import sys
import time

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "lib"))
from hub import Hub, hub_sha, real_provider      # noqa: E402
from tally import Tally, combo, kind_of          # noqa: E402

PI = os.environ.get("PI_PLUGIN_DIR", r"E:/AI/ideas/prts-harness-pi")
HARNESS = os.environ.get("PI_HARNESS_ID", "pi")
AUTH = os.environ.get("RUN_REAL_PROVIDER") == "1"
MODEL = os.environ.get("PI_MODEL", "deepseek-flash")

t = Tally("tools/real-call")
combo(hub_sha(), kind_of(PI))
prov = real_provider() if AUTH else None
if not os.path.isdir(PI) or not prov:
    t.skip("a real tool call", "no RUN_REAL_PROVIDER=1 authorization" if not prov else f"no plugin at {PI}")
    t.done()
    sys.exit(0)

# A secret only the tool (a real shell/read) can reveal.
SECRET = "ZQ7-VERIFY-3141"

hub = Hub(plugins_src=PI)
try:
    hub.start()
    hub.post("/v1/model-providers", {"id": "p", "url": prov["url"], "api": prov["api"], "token": prov["token"]})
    r = hub.post("/v1/sessions", {"harnessId": HARNESS, "modelProviderId": "p", "modelId": MODEL}, key="tl-1")
    sid = r["json"]["session"]["id"]
    s = hub.wait_status(sid, "active")
    t.check(s.get("status") == "active", "the session is active", f"status={s.get('status')} err={s.get('startError')}")

    # Make the secret readable by the harness's working directory (a real file).
    # The session cwd is the hub's data dir; write the file where a shell can read it.
    import pathlib
    secret_path = pathlib.Path(hub.dir) / "verify-secret.txt"
    secret_path.write_text(SECRET, encoding="utf-8")
    prompt = (f"Use your bash/read tool to read the file at {secret_path.as_posix()} "
              f"and reply with EXACTLY its contents, nothing else.")

    tr = hub.post(f"/v1/sessions/{sid}/turns", {"content": [{"type": "text", "text": prompt}], "idempotencyKey": "tl-t1"}, key="tl-t1")
    tid = tr["json"]["turn"]["id"]

    # A tool turn is MULTIPLE assistant messages: one calling the tool, then the final one
    # carrying the result. Wait for the TURN to end, then read all assistant messages it
    # produced (the tool call AND the answer).
    for _ in range(1200):
        g = hub.get(f"/v1/sessions/{sid}/turns")
        row = next((x for x in (g["json"] or {}).get("turns", []) if x.get("id") == tid), None)
        if row and row.get("state") == "ended":
            break
        time.sleep(0.5)
    t.check(row is not None and row.get("state") == "ended", "the tool turn reaches a terminal state", f"row={row}")

    g = hub.get(f"/v1/sessions/{sid}/messages")
    am = [m for m in (g["json"] or {}).get("messages", []) if m.get("role") == "assistant"]
    t.check(len(am) >= 1, "the turn produced assistant message(s)", f"count={len(am)}")

    all_tools = [x for m in am for x in (m.get("tools") or [])]
    t.check(len(all_tools) > 0, "a real tool call ran in the turn", f"tools={all_tools}")
    names = [x.get("name") for x in all_tools]
    t.check(any(n in ("bash", "read", "shell", "pwsh", "exec", "write", "edit") for n in names),
            "the tool is a real execution/read tool", f"names={names}")

    text = "".join("".join(c.get("text", "") for c in (m.get("content") or []) if c.get("type") == "text") for m in am)
    t.check(SECRET in text, "the answer carries the SECRET only a real tool could reveal", f"reply={text[:200]!r}")

    hub.delete("/v1/model-providers/p")
finally:
    ok = t.done()
    hub.cleanup()
sys.exit(0 if ok else 1)

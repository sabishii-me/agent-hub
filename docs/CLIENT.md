# Connecting to a hub

For the desktop team. Short version: **a hub publishes its connection material in
`<PRTS_DATA_DIR>/endpoint.json`, and that IS the supported discovery mechanism.**
There is no environment variable for the token, no fixed port, and no way to turn
authentication off.

```
$PRTS_DATA_DIR/endpoint.json          (written after the port is bound, removed on exit)
{
  "port": 58200,                      // loopback port, different every boot
  "token": "bca71a88…",               // 32 random bytes, NEW every boot
  "pid": 9576,                        // the hub process that owns this file
  "protocol": { "name": "prts-hub", "version": "2026-09-16" },
  "buildId": "9336145c3771…",
  "startedAt": "2026-09-16T16:10:11.236Z"
}
```

Default data dir: `~/.prts-core` (`%USERPROFILE%\.prts-core` on Windows). **One hub
per data dir** — the file is a single slot, and the hub deletes it before binding,
so its existence means "a hub believes it is up".

## Right now, for development

```bash
# 1. start a hub against a data dir of your own (a terminal, or from your app)
PRTS_DATA_DIR=/tmp/prts-dev node apps/harness-hub/server.mjs
#    it prints:  agent-hub listening 127.0.0.1:58200

# 2. get the url + token, in shell form or as JSON
node apps/harness-hub/scripts/hub-connect.mjs --data-dir /tmp/prts-dev
node apps/harness-hub/scripts/hub-connect.mjs --data-dir /tmp/prts-dev --json
node apps/harness-hub/scripts/hub-connect.mjs --data-dir /tmp/prts-dev --pid 9576   # only this process
node apps/harness-hub/scripts/hub-connect.mjs --data-dir /tmp/prts-dev --wait 30    # wait for startup

# 3. talk to it (the helper prints shell exports, so this works in one line)
eval "$(node apps/harness-hub/scripts/hub-connect.mjs --data-dir /tmp/prts-dev)"
curl -s "$PRTS_URL/v1/harnesses"                                    # no token needed
curl -s -H "Authorization: Bearer $PRTS_TOKEN" "$PRTS_URL/v1/hub/status"
```

`scripts/hub-connect.mjs` is also the reference implementation of the rule below —
about 40 lines, and it prints *why* it refused when it refuses.

## The rule (what to implement)

1. **Wait for the file.** Poll or watch the directory (see "the file is replaced"
   below). Nothing else is a readiness signal — including the stdout line
   `agent-hub listening 127.0.0.1:<port>`, which is a human hint and carries no
   token.
2. **Check `pid`.** If you spawned the hub, require `endpoint.pid === child.id()`
   (your sidecar) or `pid` is not the process you are waiting for (dev). A hub that
   was killed (`SIGKILL`, a crash, a laptop lid) leaves the file behind; its `pid`
   will be dead. Never hand a stale token to the rest of the app.
3. **Probe without the token.** `GET /v1/harnesses` is the ONE route that needs no
   token. `200` proves the port answers *and* that it is a hub (not something else
   that got the port). Anything else (connection refused, 404, a JSON error) means
   not ready.
4. **Then use the token** for everything else: `Authorization: Bearer <token>`.
5. **Confirm identity once, after connecting:** `GET /v1/hub/status` returns
   `pid`, `port`, `startedAt`, `buildId`, `contract` and counts. Compare `pid` with
   the child you spawned. If it does not match, you are talking to someone else's
   hub — stop.

Measured behaviours (all four, on Windows, against real hubs):

| situation | what the client sees |
|---|---|
| nothing started | no `endpoint.json` |
| hub up | file present, `pid` alive, `GET /v1/harnesses` → 200 |
| hub killed hard | file **still there**, `pid` dead, port refused — stale, ignore or delete |
| hub restarted on the same data dir | file replaced, `pid` and **token** different; the OLD token gets `401 unauthorized` from the new hub |

## Restarts, and the one trap

The token changes on every boot, so a client that caches it must treat **`401`** as
"re-read `endpoint.json`" — never as "the user is not authorised". That is exactly
how a desktop app survives a hub restart without asking anyone to paste anything.

The file is written **atomically** (temp file + rename), which means it is a new
inode each time: watching the file itself follows a dead inode and will never fire
again. **Watch the directory**, or re-read after any `401`. `hub-connect.mjs
--watch` shows the pattern.

Never put the token in a URL, a log, an error message, or a crash report: it is the
only thing between a local process and this hub. The file is written `0600` on
POSIX (a no-op on Windows, where the data dir is already per-user).

## Two integration facts to know before writing UI code

* **`EventSource` cannot be used for the turn stream.** It cannot set an
  `Authorization` header, and the hub takes the token only that way (never in a
  query string — that would put it in logs and history). Read the
  `text/event-stream` yourself and split it on blank lines:

  ```js
  const res = await fetch(`${url}/v1/sessions/${id}/turns`, {
    method: 'POST',
    headers: { authorization: `Bearer ${token}`, 'content-type': 'application/json' },
    body: JSON.stringify({ content: [{ type: 'text', text: 'hello' }] }),
  });
  const reader = res.body.getReader(); const dec = new TextDecoder();
  let buf = '';
  for (;;) {
    const { done, value } = await reader.read(); if (done) break;
    buf += dec.decode(value, { stream: true });
    let i;
    while ((i = buf.indexOf('\n\n')) >= 0) {
      const frame = buf.slice(0, i); buf = buf.slice(i + 2);
      let event = 'message', data = '';
      for (const line of frame.split('\n')) {
        if (line.startsWith('event: ')) event = line.slice(7);
        else if (line.startsWith('data: ')) data += line.slice(6);
      }
      if (data) handle(event, JSON.parse(data));   // 13 events, see below
    }
  }
  ```

  The event vocabulary and each event's payload are in `contract/v1.json` (and in
  `contract/openapi.json` as a `oneOf`).
* **The hub sends no CORS headers.** A webview `fetch` from a `tauri://` origin is
  therefore blocked (and the `Authorization` header makes it a preflighted request,
  which has no allowed origin either). Two ways forward, both fine:
  * **call the hub from the Rust side** and stream to the webview over Tauri IPC —
    this is what a sidecar wants anyway: the token stays in the process that owns
    the child, and the UI never sees a port or a token; or
  * ask us for an allowlisted origin (`Access-Control-Allow-Origin: tauri://localhost`
    + `Allow-Headers: authorization, content-type`) if you would rather do HTTP from
    the webview — a deliberate, narrow change, not a `*`.

## What the hub does not do

* It does not accept the token in a query parameter, a cookie, or a body.
* It does not write the token anywhere except `endpoint.json` (`0600`).
* It does not implement a "dev mode without auth". If a client cannot read the
  file, it cannot connect; that is the feature.
* It does not open a browser or a window, ever, and it binds loopback only.

## The sidecar shape (for when you build it)

```text
desktop                          hub
  ├─ spawn: node server.mjs  ──────►  binds 127.0.0.1:0
     env: PRTS_DATA_DIR=<app data dir>   writes endpoint.json (atomic, 0600)
  ├─ watch <data dir> for endpoint.json
  ├─ require endpoint.pid == child.id()
  ├─ GET /v1/harnesses  (no token) until 200
  ├─ read token, keep it in the Rust side
  ├─ GET /v1/hub/status  →  pid must match
  └─ on exit: SIGTERM → hub removes endpoint.json and exits
```

A `SIGTERM` (or `SIGINT`) makes the hub delete the file and exit; a hard kill leaves
it, which the `pid` check catches.

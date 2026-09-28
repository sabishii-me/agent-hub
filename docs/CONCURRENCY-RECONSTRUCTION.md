# The hub's concurrency reconstruction — plan

Status: **plan, for review. No code yet.**
Authority: this is the hub's migration plan for ADR-0009 (the hub is a concurrent
service), ADR-0010 (transport/infrastructure use mature components) and ADR-0011 (the
contract is the interface; implementations are private). The decisions live in the desktop
repository's ADR log; this file is how they are carried out here. Everything below is the
hub's own implementation plus one contract change; a consumer is never described here.

## The defect, reproduced

The hub is a single Node process with one event loop. Write routes are **synchronous
RPC**: `POST /v1/hub/plugins`, `POST /v1/sessions`, `DELETE /v1/hub/plugins/{id}` and
others `await` the whole operation and only then reply, and some steps call synchronous
filesystem functions on the loop. One operation therefore freezes every client.

Reproduced against a real hub: deleting an installed **jouzu** plugin (363 MB, 28,582
files) made **eight consecutive `/v1/hub/status` polls time out** (3 s each, ~9 s with no
answer), and a client saw `An existing connection was forcibly closed by the remote host
(os error 10054)`. Starting a session stalls visibly too. Two connections is already too
many.

The fix is **not** to replace `fs.rmSync` with `fs.promises.rm` one call at a time (that
was PR #10; it was closed). That treats one symptom and leaves the model.

## The target model

Two rules (ADR-0009):

1. **No request handling may block the event loop.** No `fs.*Sync` on paths that can touch
   large or unbounded data, no `execFileSync`/`spawnSync`, no busy-wait, no CPU loop with a
   deadline. I/O is asynchronous everywhere.
2. **Long work is a command, not a call.** An operation that can take more than a moment
   returns **immediately** with a handle, runs as an independent async background task, and
   reports state/progress on the event stream. A client issues and subscribes; it never
   holds a connection waiting for the work.

### The shape — standard HTTP, no invented concept

There is **no new "operation" object**. The design follows the HTTP and SSE standards
directly, so it is traceable rather than invented:

- **Submitting long work** uses **`202 Accepted`** — RFC 9110 §15.3.3:
  > "The 202 (Accepted) status code indicates that the request has been accepted for
  > processing, but the processing has not been completed."

  The response names **where the state lives** with a **`Location`** header (RFC 9110
  §10.2.2). For a resource that already has a URL, `Location` is that URL (a plugin install
  returns `Location: /v1/hub/plugins/<the plugin id>`); there is nothing else to invent.

- **Reading the state** is a plain `GET` of that URL. The **resource's own state is the
  single source of truth** — a plugin's `state`/`detail` (already in `GET /v1/hub/plugins`)
  is the install's progress; a session's `activeTurn` is the turn's. No second object
  (no "operation") duplicates it.

- **Being told it changed** uses **Server-sent Events** — the WHATWG HTML Living Standard,
  "Server-sent events" (`text/event-stream`). The hub already emits
  `hub.plugins.changed`; the standard's `id:` field and the `Last-Event-ID` request header
  (sent by `EventSource` on reconnect) are used so a client that dropped a connection can
  resync rather than miss changes. The event says **when to re-read**; the read says what is
  true (ADR-0001).

```
POST /v1/hub/plugins {source}
   -> 202 Accepted
      Location: /v1/hub/plugins/harness-adapter-jouzu   (RFC 9110 §15.3.3 + §10.2.2)

GET /v1/hub/plugins/harness-adapter-jouzu   -> the resource: { state: "preparing", detail, ... }
GET /v1/hub/events (SSE)                     -> id: <n> event: hub.plugins.changed data: {...}
                                                (client reconnects with Last-Event-ID)
```

Note on `Location` with `202`: RFC 9110 defines `Location` for `201`/`3xx`; using it on
`202` to point at the resource whose state can be watched is the established convention
(and what `Location` means — "the specific resource"), stated here rather than assumed.

This is why the fix is a *model* change, not a patch: today the route returns `200`/`201`
**only when the work is finished** (it holds the connection); the standard says long work is
`202` **now**, with the state read from the resource and changes pushed on SSE.

## Route classification (all 41 write routes)

**Long — change to `202 Accepted` + `Location`, work in the background, report on SSE:**

| route | the work |
|---|---|
| `POST /v1/hub/plugins` | clone/download, verify, unpack, land, materialize runtime |
| `DELETE /v1/hub/plugins/{id}` | stop processes, delete files (large tree), reconcile |
| `POST /v1/hub/plugins/{id}/prepare` | materialize the runtime |
| `POST /v1/hub/orphans/{id}` (DELETE) | delete a leftover tree |
| `POST /v1/hub/registry/refresh` | fetch the registry URL |
| `POST /v1/sessions` | spawn the adapter, handshake, prepare |
| `POST /v1/sessions/{id}/fork` | open the source, fork, adopt |
| `POST /v1/sessions/{id}/compact` | adapter compaction (a model call) |
| `POST /v1/sessions/{id}/turns` | a turn (long by nature) |
| `POST /v1/sessions/{id}/close|reopen|repair|cancel` | adapter round-trips |
| `POST /v1/harnesses/{id}/auth` / providers auth | an interactive authorization flow |
| `POST /v1/hub/providers` / `.../models/refresh` | module load / network fetch |

**Short — stay request/response, but their internals must be async I/O:**

enable/disable, PATCH a field, DELETE a connection, connections validate, create/patch a
provider record, approvals/questions answers, skills/read, shutdown.

The classification is about **whether the work can take more than a moment**; short routes
still obey rule 1 (no synchronous I/O inside them).

## Blocking-point audit

Every path a request can reach is audited for synchronous I/O, synchronous child
processes, and busy-waits. Measured: 140 `fs.*Sync` calls, of which 23 are recursive
`rmSync`/`cpSync`.

**Ordered by risk, not swept at once:**

1. **Recursive / large-tree operations first** (the 23): these are the confirmed freeze
   (`rmSync` of a 363MB / 28,582-file plugin blocked the loop ~9s). Every one moves to
   `fs.promises.*`; each is proven with the concurrent-poll measurement below.
2. **Then the small synchronous file reads** (`readFileSync` of a contract/manifest): cheap,
   but moved to async so the rule is absolute, not "cheap enough".
3. **Then synchronous child processes / busy-waits**, if any remain on a request path.

Sweeping all 140 at once is a large, low-signal diff; the risk order fixes the real freeze
first and keeps each change provable.

## Transport and infrastructure (ADR-0010)

- **Hono** for HTTP, routing, params, body, and **`streamSSE`** (replaces the hand-written
  `server.mjs` server + `sseWrite`).
- **`ajv`** validating requests/responses against the **existing** `contract/v1.json` and
  `contract/adapter-v1.json`, replacing the hand-written field checks.
- **a zip library** replacing `zip.mjs`; **`p-queue`** replacing the hand-rolled 8-worker
  download; **`undici`** for downloads.
- **kept**: the hub↔adapter **contract** (unchanged — how the hub talks to an adapter is
  private to the implementation, so this plan does not describe its transport) and the
  `/v1` business surface.

The hub stops being zero-dependency; its packed artifact bundles these. That is the cost,
accepted for a transport that is not hand-written. **None of this is contract** (ADR-0011):
a consumer depends on `contract/`, not on Hono, a framework, or a library.

## The contract change (the only part a consumer sees)

Made **on the contract** (ADR-0011), so any conforming consumer follows it. This plan does
**not** describe how any particular client reacts — that is that client's, in its own
repository, against the contract.

1. Long write routes answer **`202 Accepted` with a `Location` header** (RFC 9110 §15.3.3 +
   §10.2.2) instead of `200`/`201` with the finished result. Today they return only when the
   work is done; after this they return at once, pointing at the resource's URL.
2. The state is read with `GET` on that URL — the **resource's existing shape** (a plugin row
   in `GET /v1/hub/plugins`; a session). No new read route, no new object.
3. The SSE stream gains **no new event names** for this: the hub already emits
   `hub.plugins.changed` (and a session's own events). It is brought to the standard:
   `id:` per event and honouring **`Last-Event-ID`** on reconnect (WHATWG HTML, Server-sent
   events), so a dropped client resyncs instead of missing changes.
4. `contract/v1.json` + `contract/openapi.json` updated (status codes + `Location`); the boot
   self-check keeps the surface and the contract in step.

## Order (from ADR-0009 / ADR-0010)

1. **A — model**: long routes return `202 Accepted` + `Location` and do the work in the
   background; the resource's state is the truth; SSE is brought to the standard
   (`id`/`Last-Event-ID`); no request path blocks. Do it before the framework swap so the
   behaviour is right, then the transport carries it.
2. **B — transport**: Hono + the library list.
3. **C — direct adapter connection**: **not in this plan.** Whether a client reaches an
   adapter directly is a separate decision (it re-opens ADR-0001 and the hub↔adapter
   contract). Recorded so it is not silently attempted; a plan of its own if taken up.

Each step lands with a **measurement** (a concurrent poll while the work runs — the method
that reproduced the freeze), not an assertion.

## Acceptance (ADR-0009), and how it is measured

- ≥100 concurrent connections; no in-flight work times a connection out.
- Target: thousands of idle concurrent connections without degradation.
- Two heavyweight operations at once proceed without starving each other or any connection.

**The measurement (the method that reproduced the freeze).** Start a hub. Begin the
operation under test (e.g. `DELETE` a plugin whose runtime is large). On a loop, issue
`GET /v1/hub/status` every ~150ms with a 3s timeout, and record any gap >500ms or timeout.
Before the fix: eight consecutive 3s timeouts. After: zero. The same harness scales to the
connection target (open N idle SSE/HTTP connections, then run one operation and confirm the
others keep answering). This is mechanical, repeatable, and is the pass/fail for each step
— never "it looked fine".

## What this plan does not do

It does not perform the migration; it is the plan for review. It does not design the exact
response bodies (done with the code, in the contract). It does not change the adapter
topology.

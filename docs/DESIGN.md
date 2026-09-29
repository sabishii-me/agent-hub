# The hub's design

This is the hub's architecture, top to bottom: what the service is, what modules exist, what
each owns, how they talk, and only then the details. It exists because the previous
`server.mjs` was 4,748 lines with every domain in it, and a change to one domain (the
concurrency model) could not be made correctly - long routes were missed because there was
no boundary to see them against. A file with no seams is the defect, not the style.

Read order: the decisions live in the desktop repository's ADR log (ADR-0001, 0009, 0010,
0011 among them); this file is how they are carried out here.

---

## 1. What the hub is (the whole)

The hub is **one local service** that a client reaches over **one HTTP+SSE surface** (`/v1`),
and behind which it drives **plugin-provided adapters** over a separate contract
(`contract/adapter-v1.json`). It owns no UI and no harness code.

Three facts define it:

- **It is concurrent and non-blocking (ADR-0009).** It serves every client, session and
  plugin operation on one event loop. No request path may block that loop; long work returns
  immediately and reports through the event stream. This is the whole reason the service is
  shaped the way it is.
- **The contract is the interface (ADR-0011).** `contract/v1.json` (what clients use) and
  `contract/adapter-v1.json` (what adapters use) are what the outside depends on. Everything
  inside - framework, file layout, module names - is private.
- **Infrastructure is mature, business logic is ours (ADR-0010).** HTTP/routing/SSE come from
  Hono; validation from `ajv` over the existing contracts; zip, concurrency and HTTP-client
  from libraries. The domains (plugins, sessions, providers, connections) are the hub's own.

## 2. The runtime shape (how one request flows)

```
client ──HTTP──▶ [ transport ] ──▶ [ router ] ──▶ [ domain handler ]
                     │                                    │
                     │                                    ├─ short: answer now (async I/O)
                     │                                    └─ long: 202 + Location, work detached
                     │                                              │
                     └────────── SSE stream ◀───────────────────────┘
                                  (state changes push; client re-reads the resource)
```

- **transport** owns the socket, parsing, auth, and the streaming Response. It knows nothing
  about plugins or sessions.
- **router** is the list of endpoints (method + path + which domain handles it). It is data.
- **a domain** owns its routes, its state, and its persistence. It answers short requests and
  turns long ones into a detached task that mutates its own resource state.
- **SSE** is how every state change reaches a client. An event says *when to re-read*; the
  resource read says *what is true* (ADR-0001). There is no second copy of any state.

## 3. Modules (the parts, and the one thing each owns)

Each unit is one job. A small unit is one file; a domain is a directory whose files are its
parts (its state, its routes, its commands) and whose `index.mjs` is its only interface -
nothing reaches into a domain's internals.

| module | owns (the single thing) |
|---|---|
| `main.mjs` | **entry**: read env, pick the data dir, boot, run the self-check, start the transport, wire the router. No domain logic. (`server.mjs` is deleted once the last domain has moved; see 8.) |
| `transport.mjs` | the HTTP listener + auth + the streaming Response. Turns a request into `{ method, path, params, body, client }` and a handler's writes into a Response. Owns nothing else. |
| `router.mjs` | the **table** of endpoints (method, path, domain, long/short). The one place the surface is described; the self-check compares it to `contract/v1.json`. |
| `sse.mjs` | the event streams: subscribe, frame (`event:`/`data:`/`id:`), the bounded `Last-Event-ID` log. The only place bytes are written as SSE. |
| `state/` | **persistence**: read/write the data dir's files atomically, async, with one `mutateJson` entry. Owns the file layout; no domain decides how it is stored. |
| `plugins/` | the **plugin** domain: which plugins exist, their manifest, install/remove/prepare, the runtime a plugin carries, the registry. |
| `sessions/` | the **session** domain: create/close/reopen/fork/turns/compaction, the turn state machine. |
| `adapter/` | the **hub↔adapter** link: spawn, stdio JSON-RPC, the event/human-wait plumbing. Owns the adapter contract; no other module spawns a process. |
| `providers/` | the **provider** domain: provider records, catalog, and the authorization flow. |
| `connections/` | the **connection** domain (enable/disable/delete/validate) and the secret store's policy. |
| `harnesses/` | the **harness registry**: the hub's row per harness (enabled, extensions, skills), reconciled with what is installed. |
| `skills/` | the skills the hub holds, installed into a harness's own dir and read back. |
| `humans/` | **approvals and questions**: the two shapes a harness asks a person to decide. |

`runtime.mjs`, `zip.mjs`, `secret-store.mjs`, `resources.mjs`, `build-id.mjs` are already
separate and stay; they become imports of the domains that need them.

## 4. The rules every module obeys (the invariants)

These are not guidelines; a module that breaks one is wrong.

1. **No module blocks the loop on a request path (ADR-0009).** All I/O is `fs.promises.*` /
   awaited processes. A synchronous call is allowed only at boot, before serving, and only on
   small, bounded data.
2. **A long operation is expressed once, in its domain, as a command.** The domain declares
   which of its routes are long. A long route calls one helper - `accepted(location, work)` -
   which answers `202 Accepted` + `Location` (RFC 9110 15.3.3/10.2.2) immediately and runs
   `work` as a detached task. It never holds the connection.
3. **The resource is the only truth.** A domain's state is read through its own GET routes.
   There is no "operation" object, no job id, no second store of progress. A detached task
   mutates the resource's state and emits an event; it never writes a result to the
   connection it was detached from.
4. **No compatibility, no migration (owner's rule).** A module reads the current format and
   nothing older. There is no `adopt`, no `legacy` alias, no "tolerated during migration". A
   machine from an older layout starts fresh.
5. **One domain owns one file/table.** Two domains never read-modify-write the same file. The
   persistence layer exposes `mutateJson(path, fn)` so there is one entry per file.
6. **The adapter link is one module.** Only `adapter/` spawns a process and speaks stdio; a
   domain asks it to start/stop/ask a session. The adapter contract lives there.
7. **Auth and errors are transport/router concerns.** A domain throws/returns a typed error;
   the router maps it to a status via `contract/errors.json`. No domain writes HTTP status
   numbers except through that mapping.

## 5. How a domain is written (the shape, in detail)

Every domain module is the same four things, in order:

1. **its state** - in memory and on disk, named, with the persistence call it uses;
2. **its read routes** - the GETs that expose that state (the truth a client reads);
3. **its short commands** - the writes that answer immediately;
4. **its long commands** - the writes that return `202 + Location` and detach.

A domain exports `routes()` (its slice of the router table) and nothing else the router
needs. Tests and other modules import only what the domain explicitly exports.

## 6. The service model (how concurrency is guaranteed, not hoped for)

- **The floor is structural.** `fs.*Sync` and `execFileSync` on a request path are forbidden
  by rule 1; the way to keep them out is that the transport and router are the only code
  between a socket and a handler, and they do no I/O. A reviewer reading a domain sees only
  `await`ed I/O.
- **Long work is detached by construction**: a domain's long route calls one helper
  (`accepted(location, work)`) that answers 202 and runs `work` after the response is sent.
  Miss-ing it looks wrong in review because the route returns that helper's call, not a
  `json(...)` of a finished result.
- **Progress is free**: because the resource's state is already what a GET returns, a
  detached task only has to mutate that state and emit an event. There is nothing to invent.
- **The connection count is the transport's job**: an event-driven server holds thousands of
  idle connections because nothing on the loop waits. Keeping the loop free *is* the capacity
  plan - there is no per-connection thread or buffer to size.

## 7. What this design does not decide

- The exact response bodies of each route (they are the contract, settled with the code and
  written in `contract/v1.json`).
- The adapter's stdio topology (that is the hub<->adapter contract; unchanged).
- C - a client connecting directly to an adapter (a separate decision; it re-opens ADR-0001).

## 8. The order of work (architecture first, then inward)

1. **transport + router + sse** (the frame): the Hono listener, the router table, the event
   stream with `Last-Event-ID`. The business routes are moved onto it **behaviour unchanged**
   (still synchronous answers) so the frame is proven on its own, before any domain is
   rewritten. The long/short split is applied per domain in step 3, where the route lives.
2. **state** (the floor under every domain): one async, atomic persistence layer; each
   domain moves onto it.
3. **domains, one at a time, outside-in**: plugins first (it has the reproduced defect and
   the richest state), then sessions, then adapter, then providers / connections / harnesses
   / skills / humans. Each domain moves with its routes, its state and its long/short split.
4. **delete `server.mjs`** when the last domain has moved; the entry becomes `main.mjs`.

Every step is proven by the hub's adversarial suite against the same surface, and the
concurrency acceptance is measured with a concurrent poll (ADR-0009), not asserted.

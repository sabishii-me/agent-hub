# The hub's architecture

The hub's code today is one 4,748-line `server.mjs` holding every domain; a change to one
domain cannot be made correctly there (long routes were missed because there was no boundary
to see them against). A file with no seams is the defect. This is what the hub is moving to.

This is a **modular monolith**: one process, one HTTP+SSE surface, but the code is cut into
units small enough that **each can be read, changed and proven on its own**. The structure
exists to make a change *local*, and to be executed **from the whole to the part** - run the
frame, then one domain, then the next - instead of editing a monolith and hoping.

The owner's words, which this architecture follows: "这种的好处才能模块里从大到小的执行。而不是一个巨大的文件你都没法改。"

The decisions live in the desktop repository's ADR log (ADR-0001, 0009, 0010, 0011); this
file is how they are carried out here. It is the architecture, target and all: where it and
the code disagree today, the code is behind and this is what it is moving to.

---

## 1. What the hub is

One local service. A client reaches it over one HTTP+SSE surface; behind it, it drives
plugin-provided adapters over a separate contract (`contract/adapter-v1.json`). It owns no UI
and no harness code.

- **Concurrent and non-blocking (ADR-0009).** One event loop serves every client, session and
  plugin operation. No request path may block it; long work returns at once and reports on the
  event stream.
- **The contract is the interface (ADR-0011).** `contract/v1.json` and
  `contract/adapter-v1.json` are what the outside depends on; everything inside is private.
- **Mature infrastructure, our business logic (ADR-0010).** HTTP, routing, SSE framing and
  validation come from Hono and its Validator over the existing contracts; zip/queue/http
  client from libraries. The domains are ours.

## 2. The shape, and why it is cut this way

The transport and routing are Hono's, used the way Hono documents a larger app: **each area
is its own Hono app, exported and mounted with `app.route()`** - the entry stays tiny and a
route change touches one small file, not one giant one.

```
agent-hub/
|
|- main.mjs                     entry: new Hono(); Bearer auth; app.route() each area; listen;
|                                open the db; run the boot self-check
|
|- contract/                    the contracts (unchanged): v1.json, adapter-v1.json,
|                                errors.json, openapi.json
|
|- events/                      the event bus (NOT the frame format - Hono frames SSE)
|   |- index.mjs                subscribe / emit / the bounded Last-Event-ID log
|
|- server/                      request/response helpers shared by every area (not a layer)
|   |- respond.mjs              json(c, ...) and accepted(c, location, work) -> 202 + Location
|   |- errors.mjs               typed error -> status, over contract/errors.json
|   |- contract.mjs             load contract/*.json; the boot self-check
|
|- db/                          the data: one file per table (SQLite via node:sqlite);
|   |                            only these files speak SQL
|   |- open.mjs                 open the database
|   |- plugins.mjs              the plugins table (+ the plugin directory)
|   |- sessions.mjs             the sessions table
|   |- providers.mjs            the providers table
|   |- harnesses.mjs            the harnesses table
|   |- connections.mjs          the connections table
|   |- skills.mjs               the skill registrations
|
|- routes/                      one Hono app per area (export default)
|   |- status.mjs               the hub itself: status, surface, events, shutdown, orphans
|   |- plugins.mjs              install, remove, prepare, list, icons, registry, catalog
|   |- harnesses.mjs            the harness registry, enable/disable
|   |- sessions.mjs             create, list, get, delete, patch, turns, cancel, close,
|   |                            reopen, fork, compact, repair
|   |- providers.mjs            provider records, catalog, models, the auth flow
|   |- connections.mjs          enable/disable/delete/validate
|   |- skills.mjs               list/read/write/delete
|   |- humans.mjs               approvals and questions
|
|- domain/                      the domain logic the routes call - one subdirectory per
|   |                            domain, a file per concern (named for what it holds)
|   |- plugins/
|   |   |- state.mjs            the plugin state machine
|   |   |- install.mjs          install / remove / prepare
|   |   |- registry.mjs         the fetched registry + catalog
|   |- sessions/
|   |   |- lifecycle.mjs        create / close / reopen / fork / compact / repair
|   |   |- turns.mjs            the turn state machine
|   |- adapters/
|   |   |- index.mjs            spawn, stdio JSON-RPC, the hub<->adapter link
|   |- providers/
|   |   |- auth.mjs             the authorization flow
|   |- humans/
|   |   |- wait.mjs             approval / question: waiting and settling
|   |- ...
|
|- lib/                         existing standalone pieces (unchanged)
|   |- runtime.mjs
|   |- zip.mjs
|   |- secret-store.mjs
|   |- resources.mjs
|   |- build-id.mjs
|
|- scripts/                     pack / record-runtime / emit-openapi (unchanged)
|- tests/                       the adversarial suite (unchanged)
|- docs/
```

**Why this cut, in one line each:**

- **`routes/` are Hono apps**, so the entry is a list of `app.route()` calls and a route
  change lives in one file. Hono's own guidance ("don't build Rails-like Controllers; use
  `app.route()`") is followed: a route file holds its handlers, not a controller/service/db
  stack.
- **`domain/` is grouped by domain, a file per concern.** A domain's logic is in one place,
  named for what it does - `plugins/state.mjs`, `plugins/install.mjs` - not spread across
  invented layers.
- **`db/` is one file per table** and the only place SQL exists; the rest of the code asks
  `db/plugins.mjs` for a row.
- **`events/` is the bus, not the wire.** SSE framing is Hono's `streamSSE`; the bus only says
  who is subscribed and what changed (so `events/index.mjs` contains no `event:`/`data:` text).
- **`server/`** holds only the two answers and the error mapping - the smallest set that every
  area needs.

## 3. The route surface, and `/hub/`

The current surface is inconsistent: some paths carry a `/hub/` prefix (`/v1/hub/plugins`,
`/v1/hub/providers`) and some do not (`/v1/harnesses`, `/v1/sessions`). The prefix marks
nothing reliable - a client is *always* talking to the hub, so `/v1/hub/plugins` reads as
"the hub's plugins" for no gain, while `/v1/harnesses` is the same kind of thing without it.

**Target: one prefix, `/v1/`, for everything.**

```
/v1/status      /v1/surface     /v1/events      /v1/shutdown     /v1/orphans
/v1/plugins     /v1/plugins/{id}  /v1/registry  /v1/catalog
/v1/harnesses   /v1/sessions    /v1/providers   /v1/connections  /v1/skills
```

**This is a breaking contract change** (`contract/v1.json`, `contract/openapi.json`, and every
consumer). It is stated here so it is a decision, not a silent drift. It is separable from the
concurrency model and may ship as its own change; when it ships, the contract changes on the
contract (ADR-0011) and a conforming client follows.

## 4. Data

**The hub's own state moves to a single SQLite database** (`node:sqlite`), reached only
through `db/`. Today the state is JSON files read-modify-written, which is where "two writers"
and "half a file" come from; one database gives transactions and one writer.

- The database holds the hub's relational state: plugins, sessions, providers, harnesses,
  connections, skill registrations.
- **Blobs stay files**: an installed plugin's directory, session artifacts, and secrets (the
  OS store). The database holds their path and metadata, not their bytes.
- `db/<table>.mjs` is the only place its table is named in SQL.

**Caveat, stated honestly:** `node:sqlite` is experimental (it prints a warning). If that is
unacceptable, keep the layout and swap the engine behind `db/open.mjs`; the rest is unaffected.

## 5. The rules (invariants)

1. **No blocking on a request path (ADR-0009).** All I/O is async (`fs.promises`, the db,
   awaited processes). Synchronous calls are allowed only at boot, before serving.
2. **A long operation is one command.** A long route calls
   `accepted(c, location, work)` from `server/respond.mjs`, which answers `202 Accepted` +
   `Location` (RFC 9110 15.3.3/10.2.2) and runs `work` detached. It never holds the connection.
3. **The resource is the only truth.** State is read through the GET routes. No "operation"
   object, no job id, no second store of progress. A detached task mutates the resource and
   emits an event; it never writes a result to the connection it left.
4. **No compatibility, no migration.** A `db/*.mjs` reads the current schema and nothing
   older. No `adopt`, no `legacy` alias, no "tolerated during migration". An older machine
   starts fresh.
5. **A route is thin and a domain is called, not duplicated.** A route validates, calls a
   `domain/` function, and answers. The same logic is not re-implemented in a route.
6. **Only `domain/adapters/` spawns a process and speaks stdio.** A domain asks it to
   start/stop/ask a session.
7. **Errors are typed and mapped once.** A domain throws/returns a typed error; the route maps
   it via `server/errors.mjs`. No domain writes an HTTP status.
8. **Dependencies are injected at the entry.** `main.mjs` builds the db and the areas and
   passes what is needed; a file does not import a mutable global singleton.

## 6. How one area is written (detail)

A route file (`routes/<area>.mjs`) is a Hono app:

```js
import { Hono } from 'hono'
export default function createArea(deps) {
  const app = new Hono()
  app.get('/', (c) => c.json(...))               // a read: the resource's state
  app.post('/', (c) => accepted(c, loc, work))   // a long command: 202 + Location
  return app
}
```

- **reads** answer `c.json(...)` from `db/`;
- **short commands** do their async work and answer;
- **long commands** return `accepted(c, location, work)`; `work` returns nothing - it changes
  the resource in `db/` and calls `events/emit`.

A `domain/<x>/<file>.mjs` is plain functions; it knows nothing about HTTP and never writes a
status.

## 7. How the structure makes a change local (from the whole to the part)

This is the point of the cut, and the failure it prevents:

1. **Run the frame first.** `main.mjs` + `server/` + the mounted apps, with the existing
   routes moved onto it behaviour-unchanged. The surface is proven before any domain changes.
2. **Then one area at a time.** Pick `plugins/`; its route file, its `domain/plugins/`, and its
   `db/plugins.mjs` are the *entire* blast radius. Read them, change them, prove them - without
   a monolith to hold the whole thing in your head.
3. **A missed long route is visible.** A long route returns `accepted(...)`; a short route
   returns `json(...)`. They look different in review - which is how the previous attempt's
   missed long routes (`prepare`, `orphans`, provider create) will not be missed.
4. **The self-check holds the seam.** `main.mjs` compares the mounted surface to
   `contract/v1.json` at boot, so a route that was forgotten is a refusal to start, not a
   silent gap.

## 8. What this architecture does not decide

- The exact response bodies (they are `contract/v1.json`, settled with the code).
- The adapter's stdio topology (the hub<->adapter contract; unchanged).
- C - a client connecting directly to an adapter (a separate decision; re-opens ADR-0001).

## 9. The order of work (architecture first, then inward)

1. **the frame**: `main.mjs`, `server/respond.mjs`, `server/errors.mjs`, `server/contract.mjs`,
   `events/index.mjs`, and the `routes/*` apps with the current handlers moved on
   **behaviour-unchanged**. Proven: the hub's adversarial suite against the same surface.
2. **the data layer**: `db/open.mjs` + the per-table modules; each area moves onto them.
3. **domains, one area at a time, outside-in**: `plugins` first (the reproduced defect, the
   richest state), then `sessions`, then `adapters`, then `providers` / `connections` /
   `harnesses` / `skills` / `humans`. Each area's long/short split is applied where its routes
   now live.
4. **the `/hub/` prefix is removed** on the contract, as its own change (see 3).
5. **delete `server.mjs`** when the last area has moved.

Every step is proven by the hub's adversarial suite against the same surface; the concurrency
acceptance is measured with a concurrent poll (ADR-0009), not asserted.

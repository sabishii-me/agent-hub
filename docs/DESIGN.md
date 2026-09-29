# The hub's design

The hub's architecture, top to bottom. A previous version put everything in one 4,748-line
`server.mjs`; a change to one domain could not be made correctly there (long routes were
missed because there was no boundary to see them against). A file with no seams is the
defect. This is the redesign, written before the code, and it uses an **ordinary layered
architecture** - the kind every Web service has - rather than anything invented.

Read order: the decisions live in the desktop repository's ADR log (ADR-0001, 0009, 0010,
0011); this file is how they are carried out here. Where a pattern comes from an established
source it is named.

---

## 1. What the hub is

One local service. A client reaches it over one HTTP+SSE surface (`/v1`); behind it, it
drives plugin-provided adapters over a separate contract (`contract/adapter-v1.json`). It
owns no UI and no harness code.

- **Concurrent and non-blocking (ADR-0009).** One event loop serves every client, session and
  plugin operation. No request path may block it; long work returns at once and reports on the
  event stream.
- **The contract is the interface (ADR-0011).** `contract/v1.json` and
  `contract/adapter-v1.json` are what the outside depends on; everything inside is private.
- **Mature infrastructure, our business logic (ADR-0010).** HTTP/SSE from Hono, validation
  from `ajv` over the existing contracts, zip/queue/http-client from libraries; the domains
  are ours.

## 2. The layers (the ordinary shape)

Five layers, top to bottom. Each may only call the layer below it.

```
        entry          main.mjs        boot: env, data dir, self-check, start listener
          │
        transport      transport/      the socket, auth, Web Request/Response, SSE framing
          │
        routes         routes/         one file per domain: path + method -> a service call
          │                           (validation + map errors to status; no logic)
        services       services/       the business logic and the state machines (per domain)
          │
        repositories   repositories/   read/write the data dir + the database (per domain)
          │
        adapters       adapters/       the hub<->adapter link (spawn, stdio JSON-RPC)
```

This is the standard layering of a Web application (route -> controller/service ->
repository). It is deliberately unremarkable. Cloudflare Workers was the reference for the
*shape of the entry and the handler*, not for the layers:

- **one module entry, a `fetch(request, env)` handler** - a request in, a Response out, no
  hidden global server state (Workers' ES-module `export default { fetch }`);
- **dependencies are passed in, not imported as globals** - Workers injects everything
  through `env` (bindings, services). Here, the services and repositories are built once in
  `main.mjs` and passed to the routes; a route file never reaches for a global singleton;
- **Web-standard `Request`/`Response`** at the transport boundary (Workers' whole model; also
  what Hono speaks);
- **stateless handlers** - a handler reads the resource, does the work, answers; it keeps no
  per-request state outside the repositories (Workers' isolation model).

## 3. The modules, and the one thing each owns

**Top: `main.mjs`** - the entry. Reads env, chooses the data dir, opens the database, builds
the repositories, builds the services, builds the router from the route files, runs the boot
self-check, starts the listener. It contains **no domain logic**; it is wiring.

**`transport/`** - the socket and the wire.

| file | owns |
|---|---|
| `transport/server.mjs` | the Hono app + the Node listener (`@hono/node-server`). The only file that binds a port. |
| `transport/auth.mjs` | the bearer-token check (the token from `endpoint.json`). |
| `transport/sse.mjs` | the SSE stream: subscribe, frame (`event:`/`data:`/`id:`), the bounded `Last-Event-ID` log. The only place bytes are written as SSE. |
| `transport/respond.mjs` | the short answers and the long answer: `json(res, …)` and `accepted(res, location, work)` (202 + Location, work detached). |

**`routes/`** - one file per domain, each exporting its endpoints. A route file does exactly
three things and no more: **validate** the request, **call** a service, **map** the result or
a typed error to a status (`contract/errors.json`). No business logic.

| file | the endpoints |
|---|---|
| `routes/hub.mjs` | `/v1/hub/status`, `/surface`, `/events`, `/shutdown`, `/orphans` |
| `routes/plugins.mjs` | install, remove, prepare, list, icons, registry refresh, catalog |
| `routes/sessions.mjs` | create, list, get, delete, patch, turns, cancel, close, reopen, fork, compact, repair |
| `routes/providers.mjs` | provider records, catalog, models, auth flow |
| `routes/connections.mjs` | connections enable/disable/delete/validate |
| `routes/harnesses.mjs` | the harness registry + enable/disable |
| `routes/skills.mjs` | skills list/read/write/delete |
| `routes/humans.mjs` | approvals and questions |
| `routes/index.mjs` | assembles the above into the router table; the self-check compares it to `contract/v1.json` |

**`services/`** - the business logic and state, one file per domain. This is where the work
is; a route is thin on purpose.

| file | owns |
|---|---|
| `services/plugins.mjs` | install/remove/prepare, the plugin state machine, the runtime a plugin carries, the registry |
| `services/sessions.mjs` | session lifecycle, the turn state machine, compaction, fork |
| `services/adapters.mjs` | the hub<->adapter link: spawn, stdio JSON-RPC, event/human-wait plumbing |
| `services/providers.mjs` | provider records, catalog, authorization flow |
| `services/connections.mjs` | connections and the secret store's policy |
| `services/harnesses.mjs` | the harness registry (row per harness, reconciled with what is installed) |
| `services/skills.mjs` | the skills the hub holds |
| `services/humans.mjs` | approvals and questions |

**`repositories/`** - the data layer. No other layer touches the filesystem or the database
directly.

| file | owns |
|---|---|
| `db/open.mjs` | opening the database (see 4) and the data dir |
| `repositories/plugins.mjs` | `installed.json` + the plugins directory |
| `repositories/sessions.mjs` | `sessions.json` |
| `repositories/providers.mjs` | `providers/<id>.json` |
| `repositories/harnesses.mjs` | `harnesses.json` |
| `repositories/secrets.mjs` | the OS secret store (`secret-store.mjs`), wrapped |
| `repositories/registry.mjs` | `registry.json` (the fetched catalog) |

**`adapters/`** are the hub<->adapter contract implementation (spawn, stdio); they sit under
`services/adapters.mjs`'s use but are the boundary to another process, so they are their own
sub-tree the services call.

`runtime.mjs`, `zip.mjs`, `secret-store.mjs`, `resources.mjs`, `build-id.mjs` stay and become
imports of the layer that needs them.

## 4. Data: the database, and why

**Decision: the hub's own state moves to a real database (SQLite via `node:sqlite`), behind
`repositories/`.** Today the state is a pile of JSON files read and written with
read-modify-write, which is exactly where "two writers" and "half-written file" come from; a
single-file SQLite database gives transactions and one writer for free, and Node ships
`node:sqlite` (no native build step). File layout stops being the source of truth; the
database is.

- The **database is the hub's relational state**: plugins, sessions, providers, harnesses,
  connections, skills registrations.
- **Blobs stay files**: an installed plugin's directory, a session's artifacts, the secrets
  (the OS store). The database holds their **path and metadata**, not their bytes.
- `repositories/` is the only layer that speaks SQL. A service asks a repository for a row,
  never for a query.

This is a change from JSON files and is called out here as a decision, not smuggled in.

**Caveat, stated honestly:** `node:sqlite` is marked experimental (it prints a warning), which
sits badly with "stability". Two ways to hold the decision:
- if the experimental API is acceptable for the hub's own state (it is a plain
  single-process, single-writer local store, and the API surface used is `exec`/`prepare`/
  `run`/`get`/`all`), use `node:sqlite` and pin the Node version;
- otherwise, keep the same layering and use a bundled pure-JS SQLite (e.g. `sql.js`) or a
  maintained binding, behind the same `repositories/` interface - the REST of the design is
  unaffected, which is the point of the repository layer.

Either way the layer boundary is what matters; the engine is a repository detail.

## 5. The rules every layer obeys (invariants)

1. **No blocking on a request path (ADR-0009).** All I/O is async (`fs.promises`, the
   database, awaited processes). Synchronous calls are allowed only at boot, before serving.
2. **A long operation is one command.** A route marks itself long; it calls
   `accepted(res, location, work)` from `transport/respond.mjs`, which answers `202 Accepted`
   + `Location` (RFC 9110 15.3.3/10.2.2) and runs `work` detached. It never holds the
   connection.
3. **The resource is the only truth.** State is read through the domain's GET routes. No
   "operation" object, no job id, no second store of progress. A detached task mutates the
   resource and emits an event; it never writes a result to the connection it left.
4. **No compatibility, no migration.** A repository reads the current schema and nothing
   older. No `adopt`, no `legacy` alias, no "tolerated during migration". An older machine
   starts fresh.
5. **A layer calls only the layer below.** A route never touches a repository or the database;
   a repository never calls a service. (This is what keeps a route thin and a service
   testable.)
6. **The adapter link is one boundary.** Only the adapter layer spawns a process and speaks
   stdio; a service asks it to start/stop/ask a session.
7. **Errors are typed and mapped once.** A service throws/returns a typed error; the route
   maps it to a status via `contract/errors.json`. No service writes an HTTP status.
8. **Dependencies are injected at the entry.** Services and repositories are built in
   `main.mjs` and passed down; a file does not import a mutable global singleton.

## 6. How a domain is written (the shape, in detail)

A domain is three files that share a name:

- `routes/<domain>.mjs` - the endpoints, thin: validate, call the service, map the result.
- `services/<domain>.mjs` - the logic and the state machine: its read functions, its short
  commands, its long commands (each returning a location + a `work` function).
- `repositories/<domain>.mjs` - the SQL for that domain's rows.

A service exports plain functions; a route imports the service it was handed and nothing
else. A long command's `work` returns nothing; it changes the resource and emits an event.

## 7. How concurrency is guaranteed (not hoped for)

- **The floor is structural.** The transport and routes are the only code between a socket
  and a service, and they do no I/O; a reviewer reading a service sees only awaited I/O.
- **Long work is detached by construction.** A long route returns `accepted(...)`; a
  synchronous route returns `json(...)` of a finished result. The two look different in
  review, which is how a missed long route becomes visible - the failure of the previous
  attempt.
- **Progress is free.** The resource's state is already what a GET returns, so a detached task
  only mutates it and emits an event. Nothing to invent.
- **Capacity is the loop.** Thousands of idle connections are cheap because nothing on the
  loop waits; keeping the loop free *is* the capacity plan.

## 8. What this design does not decide

- The exact response bodies (they are `contract/v1.json`, settled with the code).
- The adapter's stdio topology (the hub<->adapter contract; unchanged).
- C - a client connecting directly to an adapter (a separate decision; re-opens ADR-0001).

## 9. The order of work (architecture first, then inward)

1. **the frame**: `main.mjs` + `transport/` (Hono listener, auth, Web Request/Response, SSE
   with `Last-Event-ID`) + `routes/index.mjs`. The existing business routes are moved onto it
   **behaviour unchanged** (still synchronous answers), so the frame is proven on its own.
2. **the data layer**: open the database, write the base repositories.
3. **domains, one at a time, outside-in**: `plugins` first (it has the reproduced defect and
   the richest state), then `sessions`, then `adapters`, then `providers` / `connections` /
   `harnesses` / `skills` / `humans`. Each domain moves as its three files, and its long/short
   split is applied where its routes now live.
4. **delete `server.mjs`** when the last domain has moved.

Every step is proven by the hub's adversarial suite against the same surface; the concurrency
acceptance is measured with a concurrent poll (ADR-0009), not asserted.

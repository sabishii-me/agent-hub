# The hub's architecture (Rust)

The hub's code today is one 4,748-line Node `server.mjs` holding every domain; a change to one
domain cannot be made correctly there (long routes were missed because there was no boundary to
see them against). A file with no seams is the defect.

This is the hub **rewritten in Rust**, from the whole to the part. Rust was chosen only after the
two things that would have blocked it were removed by decision:

- the **harness-adapter** is already an **out-of-process** contract (stdio JSON-RPC), so an
  adapter's language is private - a Rust adapter, a Node one, either fits;
- the **model provider** stops being an in-process JS module and becomes **data** (see ROUTES-REVIEW,
  "Model providers are DATA"), so **no plugin requires the hub to load foreign code in its process**.

With those, nothing in the hub requires Node, and Rust gives the concurrency, the isolation and
the process model structurally instead of by discipline.

The decisions live in the desktop repository's ADR log (ADR-0001, 0009, 0010, 0011); this file is
how they are carried out here. Where this file and the code disagree today, the code is behind and
this is what it is moving to.

---

## 1. What the hub is

One local service. A client reaches it over one HTTP+SSE surface (`/v1`); behind it, it drives
plugin-provided harness adapters over a separate contract (`contract/adapter-v1.json`). It owns no
UI and no harness code.

- **Concurrent and non-blocking (ADR-0009).** One runtime serves every client, session and plugin
  operation. In Rust this is structural: async tasks on a multi-threaded executor, and **blocking
  work is a visible boundary** (`spawn_blocking`), not a rule someone must remember.
- **The contract is the interface (ADR-0011).** `contract/v1.json` and `contract/adapter-v1.json`
  are language-neutral and are what the outside depends on; everything inside - the language, the
  framework, the crate layout - is private.
- **Mature infrastructure, our business logic (ADR-0010).** The transport is a proven framework
  (axum/hyper + SSE); validation is driven by the existing contract files; zip/http/queue come
  from crates. The domains are ours.

## 2. The shape, and why it is cut this way

The transport and routing are a framework's, used the way a Rust web service is: **each area is
its own router, mounted on the app**. The entry stays tiny and a route change touches one small
module. A **Cargo workspace makes the seams physical**: a crate that does not declare a dependency
cannot reach another's internals.

```
agent-hub/
|  Cargo.toml                     the workspace
|
|- hub/                           the binary crate: the entry (main.rs)
|   |- main.rs                    boot: read env; open the db; build state; mount the routers;
|   |                             run the boot self-check; serve
|   |- state.rs                   the shared AppState handed to every router (the deps)
|
|- crates/
|   |- contract/                  loads contract/*.json; the boot self-check; the typed
|   |                             request/response structs (serde) the surface uses
|   |- transport/                 the axum app: listeners, bearer auth, the SSE stream, the two
|   |                             answers (json / accepted 202+Location)
|   |- events/                    the event bus: subscribe, emit, the bounded Last-Event-ID log
|   |- db/                        data access: one module per table (SQLite via rusqlite); only
|   |                             this crate speaks SQL
|   |- plugins/                   the plugin domain: lifecycle, version, install/remove,
|   |                             metadata, enable/disable; the plugin registry
|   |- harnesses/                 the harness domain: the top-level projection; each harness's
|   |                             runtime answers (models/presets/tools/auth/connections)
|   |- sessions/                  the session domain: lifecycle, turns, approvals, questions
|   |- adapter/                   the hub<->adapter link: spawn, stdio JSON-RPC, the event and
|   |                             human-wait plumbing (the only crate that spawns a process)
|   |- providers/                 the model-provider domain: the record (data), and the hub-owned
|   |                             protocols it names (http, auth, catalog dialect)
|   |- skills/                    the skills domain (top-level mechanism)
|   |- extensions/                the extension domain: the adapter-shipped extensions, placed
|   |                             outside the agent's workspace (see 3)
|   |- registry/                  the fetched registry + catalog
|- contract/                      v1.json, adapter-v1.json, errors.json, openapi.json
|- tests/                         the adversarial suite (drives /v1)
|- docs/
```

**Why this cut, in one line each:**

- **a framework router per area**, so the entry is a list of mounts and a route change lives in
  one module - and the **Cargo workspace** makes "a crate calls only what it declares" a
  compile-time fact, not a convention;
- **the contract crate is the only place the wire shapes live**; every router uses the same typed
  structs, so the surface and the contract cannot drift (the boot self-check still holds the seam);
- **`db/` is one module per table and the only crate that speaks SQL**;
- **`events/` is the bus, not the wire**: SSE framing is the transport framework's; the bus only
  says who is subscribed and what changed (so no file contains `event:`/`data:` text);
- **`adapter/` is the only crate that spawns a process and speaks stdio**.

## 3. Extensions, skills, and the security boundary

These are not code the hub imports; they are **resources with a placement rule**.

- **Extensions** are two kinds (see ROUTES-REVIEW): **adapter-shipped** (part of the adapter, now)
  and **user-authored** (plugin-ized later). The adapter places them for its harness.
- **Skills** are a top-level hub mechanism; the content comes from a plugin, and they are handed
  to a harness the same way.
- **The delivery rule (security).** The agent must not be able to rewrite a skill or edit a
  trust-bearing extension (today it can edit the gating extension in its workspace and bypass
  approval). The rule: **place them in a hub-owned directory outside the agent's workspace, and
  launch the harness with discovery off and explicit paths**; the gate rests on the adapter, which
  sees every tool call and cannot be edited by the agent.

The hub's part is to **own the placement and the content**, and to hand the adapter paths - never
to run the harness's code.

## 4. Model providers are DATA (decided)

A provider is **data**, not a module. The hub owns the function: HTTP, authentication (device-code,
api-key, ...), catalog fetch and field mapping; a provider declares
`{ id, name, protocol, endpoint, auth { method, ... }, catalog { dialect } }`. A vendor whose flow
needs more than the parameters allow gets a **named hub dialect**, never its own code.

This is what removes in-process foreign code (a provider can no longer run inside the hub) and is
why Rust is possible at all. (Full table of what moves from provider to hub: ROUTES-REVIEW,
"Model providers are DATA".)

## 5. Data

**A single SQLite database** (via `rusqlite`, bundled so there is no system dependency), reached
only through `db/`. Today the state is JSON files read-modify-written, which is where "two writers"
and "half a file" come from; one database gives transactions and one writer.

- The database holds the hub's relational state: plugins, sessions, providers, harnesses,
  connections, skill registrations.
- **Blobs stay files**: an installed plugin's directory, session artifacts, and secrets (the OS
  store / a vault). The database holds their path and metadata, not their bytes.
- `db/<table>.rs` is the only place its table is named in SQL.

## 6. The rules (invariants)

1. **No blocking on a request path (ADR-0009).** I/O is async (`tokio::fs`, the db, awaited child
   processes). Blocking work (the bundled `rusqlite` calls, a synchronous fs op) runs in
   `spawn_blocking`; a blocking call on the async path is a defect, and the type system makes it
   visible.
2. **A long operation is one command.** A long route calls `accepted(state, location, work)` from
   `transport/`, which answers `202 Accepted` + `Location` (RFC 9110 15.3.3/10.2.2) and runs `work`
   as a detached task. It never holds the connection.
3. **The resource is the only truth.** State is read through the GET routes. No "operation" object,
   no job id, no second store of progress. A detached task mutates the resource and emits an event;
   it never writes a result to the connection it left.
4. **No compatibility, no migration.** `db/` reads the current schema and nothing older. No `adopt`,
   no `legacy` alias, no "tolerated during migration". An older machine starts fresh.
5. **A crate calls only the crates it declares.** The workspace enforces it; there is no reaching
   into another domain's internals.
6. **Only `adapter/` spawns a process and speaks stdio.** A domain asks it to start/stop/ask a
   session.
7. **Errors are typed and mapped once.** A domain returns a typed error; the transport maps it via
   `contract/errors.json`. No domain writes an HTTP status.
8. **Dependencies are built at the entry.** `main.rs` builds the `AppState` and passes it to the
   routers; a module does not reach for a mutable global.

## 7. How the structure makes a change local (from the whole to the part)

1. **Run the frame first.** `main.rs` + `transport/` + the mounted routers, with the current routes
   moved on **behaviour-unchanged**. The surface is proven before any domain changes.
2. **Then one area at a time.** `plugins/`; its router, its domain crate and its `db/` module are
   the *entire* blast radius - the workspace makes that literal.
3. **A missed long route is visible.** A long route returns `accepted(...)`; a short route returns
   `json(...)`. They look different in review.
4. **The self-check holds the seam.** `main.rs` compares the mounted surface to `contract/v1.json`
   at boot, so a forgotten route is a refusal to start.

## 8. What this architecture does not decide

- The exact response bodies (they are `contract/v1.json`, settled with the code).
- The adapter's stdio topology (the hub<->adapter contract; unchanged).
- C - a client connecting directly to an adapter (a separate decision; re-opens ADR-0001).

## 9. The order of work (architecture first, then inward)

1. **the frame**: `hub/main.rs`, `crates/transport` (axum listener, bearer auth, SSE, `accepted`),
   `crates/contract` (load + self-check), `crates/events`. The existing routes are moved on
   **behaviour-unchanged**.
2. **the data layer**: `crates/db` + the SQLite schema; each domain moves onto it.
3. **domains, one at a time, outside-in**: `plugins` first (the reproduced defect, the richest
   state), then `sessions`, then `adapter`, then `providers` (data-only), then `connections` /
   `harnesses` / `skills` / `extensions` / `humans`.
4. **delete `server.mjs`** when the last domain has moved.

Every step is proven by the hub's adversarial suite against the same surface; the concurrency
acceptance is measured with a concurrent poll (ADR-0009), not asserted.

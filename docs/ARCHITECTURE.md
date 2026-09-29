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

With those, **the hub itself requires nothing from Node**; Rust gives the concurrency, the
isolation and the process model structurally instead of by discipline. (The harnesses and their
adapters are still Node - but they are separate processes the hub drives, not code the hub runs;
their language is private, so one of them may become Rust without touching the hub.)

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
- **PRTS capabilities are baseline.** Presets, per-command approval, plan/review and the rest
  are **the hub's baseline capabilities for every harness** - defined and owned by the hub, not
  optional and not per-adapter. A harness that lacks one is given it through the hub's extension
  mechanism; the adapter only places what the hub hands it (see 6).

## 2. Infrastructure is crates; the hub hand-rolls nothing (ADR-0010, carried over)

Stability comes from infrastructure that is already proven, not from writing it again. This is
the rule ADR-0010 set for the Node hub, and it holds harder in Rust: the hub's own code is the
domains; **everything below the domains is a maintained crate.**

| concern | today (hand-rolled in Node) | the crate |
|---|---|---|
| HTTP server, routing, SSE | hand-written `node:http` + `sseWrite` | **axum** (on **hyper**/**tokio**) + `axum::response::sse` |
| JSON | `JSON.parse` | **serde** + **serde_json** |
| contract validation | hand-written field checks | **jsonschema** over the **`endpoints[].request`/`.response` fragments** in `contract/*.json` (they use JSON-Schema vocabulary and `$ref` to `defs`; the top-level file is the hub's own route table, not a schema document) |
| SQLite | hand-rolled JSON files | **rusqlite** (bundled SQLite; blocking, run in `spawn_blocking`) |
| HTTP client (downloads) | hand fetch loop | **reqwest** (+ **rustls**) |
| zip (plugin artifacts) | hand-written `zip.mjs` (326 lines) | the **zip** / **async_zip** crate |
| bounded concurrency, retry, backoff | hand-rolled 8-worker loop, no retry | **tokio::sync::Semaphore** + **backoff** / **retry** |
| process spawn (adapter) | `child_process` | **tokio::process** |
| filesystem walks, temp dirs | hand loops | **walkdir**, **tempfile** |
| globs, mime, time, uuid, hex/hash | hand or ad hoc | **globset**, **mime_guess**, **time**, **uuid**, **hex**, **sha2** |
| secrets in the OS store | hand-written `secret-store.mjs` + PowerShell | **keyring** (OS keychain) - this settles ADR-0010's "to be evaluated" (keytar-style OS keychain vs the PowerShell path) in favour of the OS keychain; **zeroize**/**secrecy** for in-memory handling |
| platform dirs | hand-built paths | **directories** |
| CLI | hand argument parsing | **clap** |
| logging / tracing | `console.log` | **tracing** + **tracing-subscriber** |
| error types | ad-hoc objects with a `code` | **thiserror** (typed) + **anyhow** at the edges |
| semver (plugin/host versions) | hand comparison | **semver** |

**The rule, stated once:** if a piece of infrastructure has a maintained crate, the hub uses it;
it does not write its own zip reader, its own HTTP client, its own retry, its own keychain, or
its own glob. **Hand-written infrastructure is where the defects live** - the Node hub proved it
(a hand-written zip parser, a hand-rolled downloader with no backoff, a hand-written HTTP/SSE
layer that froze under two connections).

**What stays ours** (it is not infrastructure, and no crate does it): the domains - plugins,
sessions, providers, connections, harnesses, skills, extensions, humans - the `/v1` semantics,
the hub<->adapter contract, and the plugin placement rules. The split is exactly ADR-0010:
**infrastructure is borrowed; the business and the contract are ours.**

## 3. The shape, and why it is cut this way

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
|   |- sessions/                  the session domain: lifecycle, turns
|   |- humans/                    approvals and questions (a harness asking a person to decide)
|   |- adapter/                   the hub<->adapter link: spawn, stdio JSON-RPC, the event and
|   |                             human-wait plumbing (the only crate that spawns a process)
|   |                             the ADAPTER-side common layer (see 6) is a separate crate/artifact
|   |                             the adapters build on - the hub<->adapter contract, the shared
|   |                             lifecycle, the transcript, the skills hook, the catalog probe
|   |- providers/                 the model-provider domain: the record (data), and the hub-owned
|   |                             protocols it names (http, auth, catalog dialect)
|   |- skills/                    the skills domain (top-level mechanism)
|   |- extensions/                the extension domain: the hub's side of extensions - which ids a
|   |                             plugin ships, and the placement RULE (hub-owned path, discovery
|   |                             off). The adapter does the placement; this crate owns the rule
|   |                             and the content, never the harness (see 4)
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

## 4. Extensions, skills, and the security boundary

These are not code the hub imports; they are **resources with a placement rule**.

- **Extensions** are two kinds (see ROUTES-REVIEW): **adapter-shipped** (part of the adapter - an
  approval mode, the preset mechanism - kept in the adapter repo now) and **user-authored**
  (plugin-ized later). The adapter places them for its harness.
- **Skills** are a top-level hub mechanism (across harnesses, no harness id context). The content
  **comes from a plugin** (adapter-shipped today, since there are no user plugins yet), and the
  hub hands it to a harness the way extensions are handed.
- **The delivery rule (security).** The agent must not be able to rewrite a skill or edit a
  trust-bearing extension (today it can edit the gating extension in its workspace and bypass
  approval). The rule: **place them in a hub-owned directory outside the agent's workspace, and
  launch the harness with discovery off and explicit paths**.
- **The gate is the adapter's, per adapter.** The approval DECISION already flows through the
  adapter to the hub; what must not be editable is the GATING. Each adapter implements the gate
  its harness allows (pi/jouzu: a tool-call interception extension loaded from the hub-owned path,
  out of the agent's reach; dsh: its own mechanism). The hub gives the adapter what to gate; the
  adapter enforces it, and nothing the agent can write changes it. (Per-adapter flags are recorded
  in ROUTES-REVIEW, o6.)

The hub's part is to **own the placement and the content**, and to hand the adapter paths - never
to run the harness's code.

**Skills are delivered through an adapter-side hook (decided; verified).** The harnesses read
skills from the filesystem (`--skill <path>`, `DSH_AGENTS_HOME/skills`), and all three run **as
Node** (`#!/usr/bin/env node`; pi's bundle too). The adapter owns the spawn, so it can inject a
**`node:fs` hook** (`NODE_OPTIONS=--require <hook>`), which resolves a **`skills://` URI to
hub-provided content** - the harness sees a logical URI, never a real path, and the content is
hub-owned. Verified: a `--require` hook intercepts `readFileSync("skills://…")` in an ESM Node
child and returns virtual content.

This is **only skills**, not a whole sandbox; and its limits are stated:

- it hooks the fs calls a skill loader uses (`readFileSync`/`readdirSync`/`statSync`; pi/jouzu/dsh
  all read via `fs`);
- it does **not** by itself confine `child_process` - so the **real path must never leak** (not in
  argv, env, or `--skill`), which is what makes "the agent cannot find it" true;
- it requires the harness to run under Node; a harness shipped as a compiled binary (SEA/bun)
  would need a different insertion point. All current harnesses are Node.

**Where the hook lives:** either in each adapter, or in **a shared adapter layer** (a Rust adapter
common layer) that all adapters use, so the hook is written once. The hub coordinates (it owns the
content and the `skills://` namespace); the injection is adapter-side, because the adapter owns
the harness's spawn. The hub may be Rust throughout, since the hook is injected into the Node
harness by the adapter, not by the hub.

## 5. Model providers are DATA (decided)

A provider is **data**, not a module. The hub owns the function: HTTP, authentication (device-code,
api-key, ...), catalog fetch and field mapping; a provider declares
`{ id, name, protocol, endpoint, auth { method, ... }, catalog { dialect } }`. A vendor whose flow
needs more than the parameters allow gets a **named hub dialect**, never its own code.

This is what removes in-process foreign code (a provider can no longer run inside the hub) and is
why Rust is possible at all. (Full table of what moves from provider to hub: ROUTES-REVIEW,
"Model providers are DATA".)

## 6. The adapter boundary: the adapter TRANSLATES; it does not implement

The adapter exists to **translate** between the hub's contract and one harness's dialect. It has
drifted: it now implements whole features that belong to the harness (which already has them) or
to the hub (which owns them). Read from `pi-adapter.cjs`:

| in the adapter today | who already owns it | verdict |
|---|---|---|
| `send`/`piRequest`/`handleBusMessage`/`handlePiMessage` | - | **adapter** (this is the translation) |
| `resolvePi`/`startPi` (spawn, argv, cwd) | - | **adapter** (the harness's dialect) |
| `credentialState`, `auth print-api-key`-style handling | **the harness** - pi has `auth check`/`auth print-api-key`/`auth print-bearer-token`; jouzu has `/login`; dsh has auth | **hub protocol + harness login; the adapter only translates** - it does not re-implement |
| `probeModels` (an HTTP GET /models + parse) | the **hub** (provider catalog is the hub's, decided) | **hub** |
| `scanModels`/`managedModels`/`modelDecl`/`configThinkingLevels`/`buildInjectedDir`/`applyInjectedProvider` | the **harness** has model adapters (pi `list-models`/providers, jouzu catalogs, dsh models); the **hub** holds the provider record (data) | **harness + hub; the adapter translates** |
| `ensureTranscript`/`load`/`save`/`append`/`transcriptPath` | the **hub** (ADR-0001: the hub is the session truth) | **hub** - the adapter must not keep a second session store |
| `listShippedPresets`/`writeActivePreset`/`installAgentPresetsExt`/`planCommand`/`reviewCommand` | **the hub** - these are **PRTS baseline capabilities**, defined by the hub for every harness | **hub**; the adapter only PLACES the extension the harness needs |
| `copyTree`/`readJsonFile` | - | **shared adapter layer** (not copied per adapter) |

**Decided boundary:**

1. **The adapter translates only.** hub<->harness protocol dialect, plus the harness-specific
   PLACEMENT of what the hub hands it. It implements no feature the harness already has and none
   the hub owns.
2. **Login/credentials**: the harness has its own login; the hub's auth is data + a protocol; the
   adapter translates the hub's auth contract to the harness's login. No per-adapter auth.
3. **Models/providers**: the harness has model adapters; the hub holds the provider record as
   data; the adapter translates. Provider catalog fetch (HTTP) is the hub's.
4. **Session state**: the hub is the session truth (ADR-0001). The adapter keeps **no** second
   transcript store.
5. **PRTS capabilities (preset/plan/review) are hub baseline capabilities**: the hub defines them
   for every harness; the adapter places the harness's extension for them. The capability is not
   the adapter's and is not optional per harness.

The shared common part (protocol loop, framing, the transcript SHAPE the hub expects, the catalog
probe shape, the helpers, the skills hook) lives in the **shared adapter layer** (next section),
written once.

## 7. The adapter layer is shared, not copied

Every harness adapter today re-implements the SAME logic. Measured: pi (1,676 lines) and jouzu
(1,806) share **34 function names** (`send`, `piRequest`, `handleBusMessage`, `copyTree`,
`placeExtension`, `resolvePi`, the transcript reader/writer, `probeModels`, ...), and their
`history-content.cjs` files are **byte-identical**. That is the same defect as the old
`server.mjs`: no shared seam, so the common part is copy-pasted, and a fix must be made N times.

**Decided: a shared adapter layer owns the common logic; an adapter is only the harness-specific
part.** The common layer (a Rust crate / a small binary the adapters build on, matching
"don't hand-roll, don't copy"):

- the **hub<->adapter stdio JSON-RPC** loop, framing, request/response, errors
  (`contract/adapter-v1.json`);
- the **session lifecycle** plumbing the contract defines (start/resume, config, prompt, abort,
  history paging) as the shapes the hub expects;
- the **transcript** reader/writer (identical today across adapters);
- the **skills hook** (the `node:fs` interposer that resolves `skills://` to hub content) and the
  extension placement RULE; the adapter supplies the harness-specific placement the harness needs;
- the **model catalog probe** shape and the model/declaration mapping helper;
- the common helpers (`copyTree`, JSON file IO, credential-state shape).

An adapter keeps ONLY what is the harness's own: how to spawn its harness, its CLI/rpc dialect,
its extension/preset mechanics, its auth flow specifics. The split is the same rule as the hub:
**the contract and the lifecycle are shared; the harness's dialect is the adapter's.**

This is also where the **skills hook** lives once (see 4), so it is written once, not per adapter,
and a Rust adapter gets it from the shared layer.

## 8. Data

**A single SQLite database** (via `rusqlite`, bundled so there is no system dependency), reached
only through `db/`. Today the state is JSON files read-modify-written, which is where "two writers"
and "half a file" come from; one database gives transactions and one writer.

- The database holds the hub's relational state: plugins, sessions, providers, harnesses,
  connections, skill registrations.
- **Blobs stay files**: an installed plugin's directory, session artifacts, and secrets (the OS
  store / a vault). The database holds their path and metadata, not their bytes.
- `db/<table>.rs` is the only place its table is named in SQL.

## 9. Concurrency: how ADR-0009's numbers are met in Rust

ADR-0009 is a floor and a target, measured, not asserted:

- **the floor**: >= 100 concurrent connections, and any operation in flight does not make another
  connection time out;
- **the target**: thousands of idle concurrent connections without degradation;
- **concurrent work**: two heavyweight operations at once proceed without starving each other or
  any connection.

How Rust meets each, structurally:

| requirement | mechanism |
|---|---|
| thousands of idle connections | **tokio multi-thread runtime**, one async task per connection; an idle connection is a parked task, no thread and no buffer held. axum/hyper handle keep-alive and back-pressure. |
| no request path blocks the loop | handlers are `async`; the only blocking work (bundled `rusqlite`, some fs) is sent to **`spawn_blocking`**, so it runs on a blocking pool and never stalls the reactor. A blocking call on the async path is a defect the type system makes visible. |
| a long operation does not hold a connection | it is **detached** (`accepted(location, work)` -> `tokio::spawn`); the work mutates the resource and emits an event. The client is answered `202` at once. |
| two heavyweight operations do not starve each other | a heavyweight op is a task among tasks on the multi-thread runtime; the **bounded** parts (downloads) use an explicit **`tokio::sync::Semaphore`** so a single install cannot exhaust the pool, and CPU/IO inside one op never runs synchronously on the reactor. |
| bounded resources under load | per-concern limits (download concurrency, open files) are semaphores with a chosen ceiling, not "whatever the loop allows". |

**The measurement is the same one that reproduced the freeze** (a concurrent poll while a large
operation runs), turned into the acceptance test - see ADR-0009 and the adversarial suite. The
target is not a hope: an event-driven runtime holds thousands of idle connections because nothing
on the loop waits; in Rust the loop is the runtime's, and blocking is an explicit, checked
boundary rather than a rule to remember.

## 10. The rules (invariants)

1. **No blocking on a request path (ADR-0009).** I/O is async (`tokio::fs`, the db, awaited child
   processes). Blocking work (the bundled `rusqlite` calls, a synchronous fs op) runs in
   `spawn_blocking`; a blocking call on the async path is a defect, and the type system makes it
   visible.
2. **A long operation is one command.** A long route calls `accepted(location, work)` from
   `transport/`, which answers `202 Accepted` + `Location` (RFC 9110 15.3.3/10.2.2) and hands `work`
   to the runtime as a detached task (a plain `tokio::spawn`; it needs no shared state). It never
   holds the connection.
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

## 11. How the structure makes a change local (from the whole to the part)

1. **Run the frame first.** `main.rs` + `transport/` + the mounted routers, with the current routes
   moved on **behaviour-unchanged**. The surface is proven before any domain changes.
2. **Then one area at a time.** `plugins/`; its router, its domain crate and its `db/` module are
   the *entire* blast radius - the workspace makes that literal.
3. **A missed long route is visible.** A long route returns `accepted(...)`; a short route returns
   `json(...)`. They look different in review.
4. **The self-check holds the seam.** `main.rs` compares the mounted surface to `contract/v1.json`
   at boot, so a forgotten route is a refusal to start.

## 12. What this architecture does not decide

- The exact response bodies (they are `contract/v1.json`, settled with the code).
- The adapter's stdio topology (the hub<->adapter contract; unchanged).
- C - a client connecting directly to an adapter (a separate decision; re-opens ADR-0001).

## 13. The order of work (architecture first, then inward)

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

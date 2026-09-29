# agent-hub's architecture (Rust)

This is **agent-hub**, rewritten from scratch in Rust. There is **no migration and no legacy**:
the previous Node `server.mjs` monolith failed badly, and nothing of it is carried over - not a
line, not a format, not a compatibility path. The old file is deleted; this is a new program.

The problem it removes: one 4,748-line file held every domain, so a change to one domain could not
be made correctly (long routes were missed because there was no boundary to see them against). A
file with no seams is the defect. A **Cargo workspace** makes the seams physical.

Two things make Rust possible; both are decisions of this design, not accidents:

- a **harness adapter** is an **out-of-process** contract (stdio JSON-RPC), so its language is
  private - a Rust adapter, a Node one, either fits;
- a **model provider** is **data**, not an in-process module (see 5), so **no plugin runs code
  inside the hub's process**.

With those, **the hub itself requires nothing from Node**; Rust gives the concurrency, the
isolation and the process model structurally instead of by discipline.

The decisions live in the desktop repository's ADR log (ADR-0001, 0009, 0010, 0011); this file is
how they are carried out here.

---

## 1. What agent-hub is

One local service. A client reaches it over one HTTP+SSE surface (`/v1`); behind it, it drives
harness adapters over a separate contract (`contract/adapter-v1.json`). It owns no UI and no
harness code.

- **Concurrent and non-blocking (ADR-0009).** One runtime serves every client, session and plugin
  operation. In Rust this is structural: async tasks on a multi-threaded executor, and **blocking
  work is a visible boundary** (`spawn_blocking`), not a rule someone must remember.
- **The contract is the interface (ADR-0011).** `contract/v1.json` and `contract/adapter-v1.json`
  are language-neutral and are what the outside depends on; everything inside - the language, the
  framework, the crate layout - is private.
- **Mature infrastructure, our business logic (ADR-0010).** The transport is a proven framework
  (axum/hyper + SSE); validation is driven by the existing contract files; zip/http/queue come from
  crates. The domains are ours.
- **Baseline capabilities are agent-hub's convention.** agent-hub **requires** every harness to
  provide **preset, approval, plan and review**. The hub states the requirement and its contract;
  the **adapter implements it** for its harness (using the harness's own mechanism, or an extension
  the hub provides when the harness lacks one). These are not optional and not the hub's to
  implement - they are the adapter's job, the same way driving the harness is.

## 2. Infrastructure is crates; nothing is hand-rolled

Stability comes from infrastructure that is already proven, not from writing it again (ADR-0010).
Nothing in this program is hand-written below the domains. The crates are chosen, and the ones the
old code hand-rolled are named:

| concern | the old hand-rolled thing | the crate |
|---|---|---|
| HTTP server, routing, SSE | hand-written `node:http` + `sseWrite` | **axum** (on **hyper**/**tokio**) + `axum::response::sse` |
| JSON | `JSON.parse` | **serde** + **serde_json** |
| contract validation | hand-written field checks | **jsonschema** over the `endpoints[].request`/`.response` fragments in `contract/*.json` (JSON-Schema vocabulary + `$ref` to `defs`; the top-level file is agent-hub's own route table, not a schema document) |
| SQLite | hand-rolled JSON files | **rusqlite** (bundled SQLite; blocking, run in `spawn_blocking`) |
| HTTP client (downloads) | hand fetch loop | **reqwest** (+ **rustls**) |
| zip (plugin artifacts) | a 326-line hand-written `zip.mjs` | the **zip** / **async_zip** crate |
| bounded concurrency, retry, backoff | hand-rolled 8-worker loop, no retry | **tokio::sync::Semaphore** + **backoff** / **retry** |
| process spawn (adapter) | `child_process` | **tokio::process** |
| filesystem walks, temp dirs | hand loops | **walkdir**, **tempfile** |
| globs, mime, time, uuid, hex/hash | ad hoc | **globset**, **mime_guess**, **time**, **uuid**, **hex**, **sha2** |
| secrets in the OS store | hand-written PowerShell path | **keyring** (OS keychain); **zeroize**/**secrecy** in memory |
| platform dirs | hand-built paths | **directories** |
| CLI | hand argument parsing | **clap** |
| logging / tracing | `console.log` | **tracing** + **tracing-subscriber** |
| error types | ad-hoc objects with a `code` | **thiserror** (typed) + **anyhow** at the edges |
| semver (plugin/host versions) | hand comparison | **semver** |

**The rule:** if infrastructure has a maintained crate, use it. Do not write a zip reader, an
HTTP client, a retry loop, a keychain, or a glob. **Hand-written infrastructure is where the
defects live** - the old code proved it (a hand-written zip parser, a downloader with no backoff,
an HTTP/SSE layer that froze under two connections).

**What is ours** (no crate does it): the domains - plugins, sessions, providers, connections,
harnesses, skills, extensions, humans - the `/v1` semantics, the hub<->adapter contract, the
placement rules. Infrastructure is borrowed; the business and the contract are ours.

## 3. The shape: a Cargo workspace

The transport and routing are a framework's: **each area is its own router, mounted on the app**.
The entry stays tiny and a route change touches one module. The **workspace** makes "a crate calls
only what it declares" a compile-time fact.

```
agent-hub/
|  Cargo.toml                      the workspace
|
|- hub/                            the binary crate: the entry
|   |- main.rs                     boot: read env; open the db; build state; mount the routers;
|   |                              run the boot self-check; serve
|   |- state.rs                    the AppState handed to every router (the deps)
|
|- crates/                         agent-hub's own code
|   |- contract/                   loads the contract FILES (../contract/*.json); the boot
|   |                              self-check; the typed request/response structs (serde)
|   |- transport/                  the axum app: listeners, bearer auth, the SSE stream, the two
|   |                              answers (json / accepted 202+Location)
|   |- events/                     the event bus: subscribe, emit, the bounded Last-Event-ID log
|   |- db/                         data access: one module per table; the only crate that speaks SQL
|   |- plugins/                    the plugin domain: lifecycle, version, install/remove,
|   |                              metadata, enable/disable; the plugin registry
|   |- harnesses/                  the harness domain: the top-level roster (a thin projection);
|   |                              each harness's runtime answers (models/presets/tools/auth/
|   |                              connections)
|   |- sessions/                   the session domain: lifecycle, turns
|   |- humans/                     approvals and questions (a harness asking a person to decide)
|   |- adapter/                    the hub SIDE of the hub<->adapter link: spawn, stdio
|   |                              JSON-RPC, the event and human-wait plumbing (the only crate
|   |                              that spawns a process)
|   |- providers/                  the model-provider domain: the record (data) and the protocols
|   |                              it names (http, auth, catalog dialect)
|   |- skills/                     the skills domain: the hub's side (which skills, which
|   |                              session/workspace gets them; content from a plugin)
|   |- extensions/                 the extension domain: the hub's side (which ids a plugin ships;
|   |                              the placement rule - hub-owned path, discovery off)
|   |- registry/                   the fetched registry + the catalog
|
|- contract/                       the contract FILES: v1.json, adapter-v1.json, errors.json,
|                                  openapi.json  (data, language-neutral - not Rust)
|- tests/                          the adversarial suite (drives /v1)
|- docs/
```

**Why this cut:**

- **a router per area**, so the entry is a list of mounts;
- **the contract crate is the only place the wire shapes live**; the boot self-check keeps the
  surface and the contract in step;
- **`db/` is the only crate that speaks SQL**;
- **`events/` is the bus, not the wire**: SSE framing is the transport framework's; no file
  contains `event:`/`data:` text;
- **`adapter/` is agent-hub's side only.** The adapters' shared library is **not here** (see 7).

## 4. The adapter's shared layer is a LIBRARY agent-hub provides

An adapter (pi's, jouzu's, dsh's) is written against a **shared adapter library that agent-hub
provides** - Rust code (and/or an injected API) an adapter calls, so the common logic is written
once and never copied.

```
adapters/                       SEPARATE from the hub - the adapters' shared layer, provided
  adapter-lib/                  the library an adapter builds on / calls:
    - the hub<->adapter contract loop: framing, JSON-RPC, requests/responses, errors
    - the session lifecycle shapes the hub expects (start/resume, config, prompt, abort,
      history paging)
    - the transcript SHAPE the hub expects
    - the skills hook (the node:fs interposer that resolves skills:// to hub content)
    - the extension PLACEMENT rule
    - the catalog-probe SHAPE
    - helpers (copy tree, json file io, credential-state shape)
```

**Two ways to use it**, both allowed:

- **a Rust adapter** links the library and implements only its harness's dialect;
- **another language (e.g. a Node adapter)** gets the same capabilities through an **injected API**
  (the hook, the framing), from the same library.

An adapter keeps **only its harness's dialect**: how to spawn it, its CLI/rpc, its
extension/preset mechanics, its login. The split: **the contract and the lifecycle shapes are in
the shared layer (written once); the harness's dialect is the adapter's.**

Why this exists (measured): pi (1,676 lines) and jouzu (1,806) share **34 function names** and
their `history-content.cjs` is **byte-identical**; the adapters differ by ~358 lines. That is the
same defect as the old monolith - no shared seam, so the common part is copy-pasted.

## 5. Model providers are DATA (decided)

A provider is **data**, not a module. The hub owns the function: HTTP, authentication
(device-code, api-key, ...), catalog fetch and field mapping; a provider declares
`{ id, name, protocol, endpoint, auth { method, ... }, catalog { dialect } }`. A vendor whose flow
needs more than the parameters allow gets a **named hub dialect**, never its own code.

This removes in-process foreign code (a provider cannot run inside the hub) and is why Rust is
possible at all. (Full table of what moves from the provider to the hub: ROUTES-REVIEW,
"Model providers are DATA".)

## 6. The adapter boundary: the adapter TRANSLATES and IMPLEMENTS the baseline

An adapter does two things, and no more:

1. **translate** between the hub's contract and the harness's dialect (spawn, CLI/rpc, events,
   errors); and
2. **implement agent-hub's baseline capabilities** for its harness - **preset, approval, plan,
   review** - using the harness's own mechanism, or an extension the hub provides when the harness
   lacks one.

It does **not** implement what the harness already owns, and it does not implement what agent-hub
owns. Read from `pi-adapter.cjs`:

| in an adapter today | who owns it | verdict |
|---|---|---|
| `send`/`piRequest`/`handleBusMessage`/`handlePiMessage` | - | **adapter** (this IS the translation) |
| `resolvePi`/`startPi` (spawn, argv, cwd) | - | **adapter** (the harness's dialect) |
| `listShippedPresets`/`writeActivePreset`/`planCommand`/`reviewCommand`/approval gating | **the adapter** - preset/approval/plan/review are the **baseline** it implements | **adapter** (implement the baseline) |
| `credentialState`, login handling | the **harness** has its own login (pi `auth check`/`print-api-key`; jouzu `/login`; dsh auth) | the adapter **translates** the hub's auth contract to the harness's login; it does not re-implement login |
| `probeModels` (an HTTP GET /models) | **agent-hub** (the provider catalog is the hub's) | **hub** |
| `scanModels`/`managedModels`/`modelDecl`/`configThinkingLevels`/provider injection | the **harness** has model adapters (pi `list-models`, jouzu catalogs, dsh models); the **hub** holds the provider record | **harness + hub; the adapter translates** |
| `ensureTranscript`/`load`/`save`/`append` | **agent-hub** (ADR-0001: the hub is the session truth) | **hub** - an adapter keeps no second session store |
| `copyTree`/`readJsonFile` | - | the **shared adapter library** (4), not copied |

**So:** an adapter = the harness's dialect **plus** the baseline capabilities, built on the shared
library. It never keeps a second copy of session state, and it never re-implements the harness's
login or model adapters.

## 7. Extensions, skills, and the security boundary

These are resources with a placement rule, not code the hub imports.

- **Extensions** are two kinds (see ROUTES-REVIEW): **adapter-shipped** (part of the adapter - an
  approval mode, the preset mechanism) and **user-authored** (plugin-ized later). The adapter
  places them for its harness.
- **Skills** are a top-level hub mechanism; the content comes from a plugin, and the hub hands it
  to a harness the way extensions are handed.
- **The delivery rule (security).** The agent must not be able to rewrite a skill or edit a
  trust-bearing extension (it can edit the gating extension in its workspace and bypass approval).
  The rule: **place them in a hub-owned directory outside the agent's workspace, and launch the
  harness with discovery off and explicit paths**.
- **Skills through a hook.** All current harnesses run as Node; the adapter owns the spawn, so it
  injects a **`node:fs` hook** (`NODE_OPTIONS=--require <hook>`) that resolves a **`skills://` URI
  to hub content** - the harness sees a logical URI, never a real path. Verified: a `--require`
  hook intercepts `readFileSync("skills://…")` in an ESM Node child. It is **only skills**, not a
  sandbox, and because it does not confine `child_process`, **the real path must never leak** (not
  in argv, env or `--skill`). Requires the harness to run under Node (all current ones do).

## 8. Data

**One SQLite database** (via `rusqlite`, bundled - no system dependency), reached only through
`db/`. The old code kept JSON files read-modify-written, which is where "two writers" and "half a
file" came from; one database gives transactions and one writer.

- The database holds the hub's relational state: plugins, sessions, providers, harnesses,
  connections, skill registrations.
- **Blobs stay files**: an installed plugin's directory, session artifacts, secrets (the OS
  keychain). The database holds path and metadata, not bytes.
- `db/<table>.rs` is the only place its table is named in SQL.
- **Schema**: the database is created by the program with its own schema and its own version
  marker. There is no migration from anything (see the top) and no compatibility with any old
  format.

## 9. Concurrency: how ADR-0009's numbers are met in Rust

ADR-0009 is a floor and a target, measured, not asserted: **>= 100 concurrent connections** and no
in-flight operation times another out; **thousands of idle connections**; two heavyweight
operations at once without starving each other.

| requirement | mechanism |
|---|---|
| thousands of idle connections | **tokio multi-thread runtime**, one async task per connection; an idle connection is a parked task - no thread, no buffer. axum/hyper handle keep-alive and back-pressure. |
| no request path blocks the loop | handlers are `async`; the only blocking work (bundled `rusqlite`, some fs) goes to **`spawn_blocking`**. A blocking call on the async path is a defect the type system makes visible. |
| a long operation does not hold a connection | it is **detached** (`accepted(location, work)` -> `tokio::spawn`); the work mutates the resource and emits an event; the client is answered `202` at once. |
| two heavyweight operations do not starve each other | each is a task among tasks; **bounded** parts (downloads) sit behind a **`tokio::sync::Semaphore`**, so one install cannot exhaust the pool. |
| SSE to many subscribers | the bus broadcasts; a slow subscriber is bounded (bounded channel / drop policy), so one client cannot stall the loop for the rest. |

The measurement is the concurrent-poll method that reproduced the freeze in the old code, turned
into the acceptance test.

## 10. The rules (invariants)

1. **No blocking on a request path (ADR-0009).** I/O is async; blocking work runs in
   `spawn_blocking`.
2. **A long operation is one command.** A long route calls `accepted(location, work)` from
   `transport/`, which answers `202 Accepted` + `Location` (RFC 9110 15.3.3/10.2.2) and runs `work`
   detached. It never holds the connection.
3. **The resource is the only truth.** State is read through the GET routes. No "operation"
   object, no job id, no second store of progress. A detached task mutates the resource and emits
   an event.
4. **No compatibility, no migration, no legacy.** A fresh program; nothing reads an old format.
5. **A crate calls only the crates it declares.** The workspace enforces it.
6. **Only `adapter/` spawns a process and speaks stdio.**
7. **Errors are typed and mapped once.** A domain returns a typed error; the transport maps it via
   `contract/errors.json`. No domain writes an HTTP status.
8. **Dependencies are built at the entry.** `main.rs` builds the `AppState` and passes it to the
   routers; no module reaches for a mutable global.

## 11. How the structure makes a change local

1. **Build the frame first.** `hub/` + `crates/transport` + `crates/contract` + `crates/events`,
   with the routers mounted. The surface is proven against `contract/v1.json` before any domain.
2. **Then one area at a time.** `plugins/`; its router, its domain crate and its `db/` module are
   the *entire* blast radius - the workspace makes that literal.
3. **A missed long route is visible.** A long route returns `accepted(...)`; a short route returns
   `json(...)`.
4. **The self-check holds the seam.** `main.rs` compares the mounted surface to `contract/v1.json`
   at boot, so a forgotten route is a refusal to start.

## 12. What this architecture does not decide

- The exact response bodies (they are `contract/v1.json`, settled with the code).
- The adapter's stdio topology (the hub<->adapter contract; unchanged).
- C - a client connecting directly to an adapter (a separate decision; re-opens ADR-0001).

## 13. The order of work

1. **the frame**: `hub/`, `crates/transport`, `crates/contract`, `crates/events`.
2. **the data layer**: `crates/db` + the schema.
3. **domains, one at a time, outside-in**: `plugins` first (the reproduced defect, the richest
   state), then `sessions`, then `adapter`, then `providers`, then `connections` / `harnesses` /
   `skills` / `extensions` / `humans`.
4. **the adapter library** (`adapters/adapter-lib`) so the adapters stop copying, and the baseline
   capabilities land once.

Every step is proven by the adversarial suite against `/v1`; the concurrency acceptance is
measured with a concurrent poll (ADR-0009), not asserted.

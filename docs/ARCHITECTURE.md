# agent-hub's architecture (Rust)

This is the **target** for agent-hub, rewritten from scratch in Rust (*what follows describes the
target, not a claim that this commit has already deleted the old implementation*). There is **no
migration and no legacy**: the previous Node `server.mjs` monolith failed badly, and nothing of it
is carried over - not a line, not a format, not a compatibility path. The old file is to be
deleted; this is a new program.

The problem it removes: one 4,748-line file held every domain, so a change to one domain could not
be made correctly (long routes were missed because there was no boundary to see them against). A
file with no seams is the defect. A **Cargo workspace** makes the seams physical.

Two things make Rust possible; both are decisions of this design, not accidents:

- a **harness adapter** is an **out-of-process** contract (stdio JSON-RPC), so its language is
  private - a Rust adapter, a Node one, either fits;
- a **model provider** is **data**, not an in-process module (see 5), so **no plugin runs code
  inside the hub's process**.

With those, **the hub itself requires nothing from Node**. Rust is chosen for its **runtime**
(async tasks on a multi-threaded executor, an out-of-process adapter, a spawned-blocking boundary);
it makes blocking *visible*, but it does **not** by itself prove the concurrency numbers - those are
measured (see 12).

The decisions live in the **desktop repository's** ADR log (`sabishii-dev-agent-desktop`,
`docs/decisions/`). The ones this file carries out:

- **ADR-0001** - the hub owns session/turn state; a client renders its snapshot (events trigger a
  re-read, they are never state).
- **ADR-0009** - the hub is a concurrent, non-blocking service; long work reports through the
  event stream; >= 100 concurrent connections.
- **ADR-0010** - transport and infrastructure use mature components; business and contract logic
  stay ours.
- **ADR-0011** - the contract is the interface; implementations are private.

This file is how they are carried out here.

---

## 1. What agent-hub is

One local service. A client reaches it over one HTTP+SSE surface (`/v1`); behind it, it drives
harness adapters over a separate contract (`contract/adapter-v1.json`). It owns no UI and no
harness code.

- **Concurrent and non-blocking (ADR-0009).** One runtime serves every client, session and plugin
  operation. The runtime is a multi-threaded async executor, and blocking work is put on a
  **spawned blocking pool** so it is a *visible boundary* rather than a rule to remember - but
  visibility is a mechanism, not a measurement; the numbers are a runtime gate (see 12).
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
| contract validation | hand-written field checks | **jsonschema**, but **only after the normalization in 9** - the current `endpoints[].request`/`.response` fragments are a compact DSL, **not** valid JSON Schema, and must not be handed to `jsonschema` as-is |
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
| `ensureTranscript`/`load`/`save`/`append` | the **hub** owns session/turn control state; the harness owns its **native conversation** | the hub's control state and the harness's native history are **different objects**; an adapter keeps **no duplicate control state** but **may** keep what the harness requires (native history passthrough and the ID/ref mapping that cannot be lost - ADR-0001 forbids a *competing* state, it does not forbid the harness's own) |
| `copyTree`/`readJsonFile` | - | the **shared adapter library** (4), not copied |

**So:** an adapter = the harness's dialect **plus** the baseline capabilities, built on the shared
library. It never keeps a **competing** copy of session/turn control state, and never re-implements
the harness's login or model adapters. But ADR-0001 ("the hub owns control state") must **not** be
read as "delete every adapter-side mapping": the native conversation and the ID/ref mapping that
the harness needs are **not** hub control state and must not be removed by that reasoning
(TASK-037 F01, TASK-042 F01/F02).

### What the shared layer must and must not do (G5)

- **Session/turn control state vs native conversation** are distinguished; the hub holds the
  former, the harness the latter, and the mapping between them is explicit.
- **The skills hook is a verification-gated delivery**, not "only flags left": a local
  `readFileSync` probe is not the acceptance. The delivery must handle the **directory loader**,
  **relative resources**, the **effective set** and **reload**, on real artifacts (TASK-043 F01).
- **Workspace/session layering**: the design states how the **effective set** is composed
  (workspace inherited + session-private) and **when it is applied**; exact field names land in
  the contract later (TASK-043 F02).
- **The shared library is a reuse layer, not a mandate**: it shares the protocol and the lifecycle
  shapes and the baseline mechanics; it does **not** mean a conforming adapter must depend on it,
  and an adapter that implements the baseline itself is not excluded for having a private
  implementation (TASK-037 F09, TASK-044 F01/F04).
- **The Rust/Node bridge and the two-platform delivery** is a **deferred gate**: a choice with a
  reason, or explicitly deferred - the docs PR does not have to ship the full ABI/binary
  (TASK-044 F02).
- **"Switch preset anytime" vs the current idle-only contract** is a **requirement ambiguity**
  (P2) for the owner to define the allowed window; it is not claimed that today's is wedged
  (TASK-042 F04).
- **Kept constraints**: native history, fork source isolation, and "abort ACK != stopped" stay.

## 7. Extensions, skills, and the security model

These are resources with a placement rule, not code the hub imports.

- **Extensions** are two kinds (see ROUTES-REVIEW): **adapter-shipped** (part of the adapter - an
  approval mode, the preset mechanism) and **user-authored** (plugin-ized later). The adapter
  places them for its harness.
- **Skills** are a top-level hub mechanism; the content comes from a plugin, and the hub hands it
  to a harness the way extensions are handed.
- **Skills through a hook.** All current harnesses run as Node; the adapter owns the spawn, so it
  injects a **`node:fs` hook** (`NODE_OPTIONS=--require <hook>`) that resolves a **`skills://` URI
  to hub content** - the harness sees a logical URI, not a real path. (A local `--require` hook was
  shown able to intercept `readFileSync("skills://…")` in an ESM Node child; that is a mechanism
  probe, **not** the delivery acceptance - see below.) The mechanism is a **verification-gated
  delivery**, not an approved implementation: it must cover the directory loader, relative
  resources, the effective set and reload, on real artifacts (ROUTES-REVIEW o6). Requires the
  harness to run under Node; a compiled-binary harness would need a different insertion point.

### Security: what placement does and does NOT guarantee (G1)

**Placement is not an authorization boundary, and this file must not claim it is.** Putting an
extension or skill in a *hub-owned* directory and turning discovery off reduces *accidental
loading and path exposure*; it does **not**, by itself, prove a malicious agent cannot modify a
skill or a trust-bearing extension. A cross-review confirmed this as a **design gap** (TASK-038
F01/F02, TASK-043 F03, TASK-037 F08) - **not** as a demonstrated bypass.

The gap is that no **execution/permission boundary** is defined. The following are preconditions
the design must state, and the parts it cannot guarantee:

- **What is protected**: plugin-provided **skills** and **adapter-shipped trust-bearing
  extensions** (and their **source copies, the loader/hook, and the reboot load chain** - not just
  the currently-loaded copy).
- **Precondition (must be defined, not assumed)**: the OS identity/permissions of the hub, the
  adapter, the harness and its child processes (UID/SID, ACLs, read-only mounts). "Hub-owned" is a
  logical owner, **not** a proven OS access difference; if the harness runs as the same principal
  with write access to a path, moving the directory does not remove that access.
- **The hook is not a sandbox.** It intercepts the `fs` calls a skill loader uses; it does **not**
  confine `child_process`/shell/native code. So with a same-principal, shell-capable agent, path
  hiding alone is insufficient - `write` accepting absolute paths is on the record
  (ROUTES-REVIEW). Which tools each harness actually enables is read from
  `GET /v1/harnesses/{id}/tools`, never assumed.
- **Management trust boundary**: the management surface answers approvals and mutates plugins and
  skills. `endpoint.json` carries the bearer on loopback; loopback is **not** inter-principal
  isolation, and the design does not yet define that an agent cannot read the token. A bearer only
  proves possession, not that the holder is the trusted human. **Every route requiring the token
  (ROUTES-REVIEW o5) is part of the trust boundary, not a substitute for one.**
- **Not guaranteed**: that a same-principal, unconstrained agent cannot alter a resource between
  reboots. The valid options are to (a) **narrow the claim** - say plainly this reduces accidental
  loading and exposure, and rely on the tool/adapter gate, or (b) **define a real execution
  boundary** (a distinct principal, OS read-only enforcement, or a tool policy that cannot be
  bypassed). This file does not mandate a specific mechanism; it forbids claiming tamper-proof
  integrity from directory placement alone. **Unknown/missing permission must fail closed.**

**The runtime verification of the security claim is an implementation task, not a claim here**
(`docs/review/VERIFICATION-TASKS.md`, T2): absolute-path write, shell/child, links, the reboot load
chain, and management-API/approval access by an agent identity, against pi/jouzu/dsh on real
artifacts. Two of its parts can still change the design and are listed as feasibility unknowns with
exit conditions (T2a - is an OS execution boundary reachable; T2b - does the skills hook deliver on
each real harness).

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

## 9. The contract is a DSL; validation needs one normalization authority (G3)

`contract/v1.json` is agent-hub's own route table with a **compact DSL**: `endpoints[].request` /
`.response` fragments use JSON-Schema vocabulary plus `$ref: "#/defs/…"`, but they are **not**
valid JSON Schema documents. A cross-review independently regenerated the OpenAPI projection at
the fixed SHA and confirmed real semantic/legality defects in it - **not** an outdated artifact
and **not** broken refs (TASK-039 F01/F02/F03, TASK-037 F03):

- a PATCH session's **optional** fields project as **all required**;
- 4 places emit `type: []`; `string[]|null?` loses the array branch;
- 12 places keep `nullable: true`, which does **not** carry JSON-Schema-2020-12 null semantics;
- 1 place writes `type: "binary"` into an `application/json` schema;
- `'manual'|string?` collapses to `const: manual`.

**Decision (design boundary):** there is **one normalization authority** from the source DSL to a
standard JSON Schema, and every generated artifact (OpenAPI), every server-side validator and the
serde types derive from **that same normalization** or are checked against it. The DSL's grammar
(required/nullable/extension rules), the full `$ref` registry, and the handling of unknown
keywords/tokens are declared; validation does not "guess" two languages by keywords.

**The existing projection is not a trustworthy validation base** - it must be regenerated from the
normalization and verified (positive and negative samples) before `jsonschema` or serde consume
it. The old projection's defects are the **owning repository's to fix** (they are not introduced
by this PR): this section states the boundary and the gate, and does not fix the old projection
here. Running a Rust validator against real `/v1` requests (accept/reject) is a later verification.

## 10. Plugin install/replace: recovery rules, not "a transaction" (G2)

An install or replace touches two stores that are **not** one transaction: the plugin's directory
tree (rename moves) and the database row. A cross-review confirmed the recovery rules are a
**design gap** (TASK-041 F01/F02/F03, TASK-037 F06) - **not** a demonstrated data loss.

"Two renames plus a boot sweep" and "SQLite gives atomicity" do **not** answer what a crash leaves.
The design must state, for the operation, **what is kept, advanced or rolled back** at each
boundary:

| boundary (a crash between these) | on restart: |
|---|---|
| before any move | nothing to do; the old plugin is intact |
| after `old -> outgoing`, before `staging -> target` | the old copy is the **last committed/activatable** version: restore it, drop the staging dir (a `staging` tree may itself be complete, but it is not the committed version) |
| after `staging -> target`, before the DB commit | the new copy is in place but uncommitted: either finish the commit (the new copy is complete) or roll back to `outgoing`; the state must say which |
| after the DB commit, before deleting `outgoing` | the replacement is done; the leftover `outgoing` is transient and swept |
| rollback itself failed | keep **both** copies and a record that says so; the state is "not usable, recovery copy retained", never a silent half-state |
| recovery interrupted again | restart resumes from the recorded state, not from scratch |

Rules that follow:

- **Recovery runs before garbage collection**, and GC's basis is **rebuildability and references**
  (is this content reconstructible; does any row/session reference it) - **not** a directory name
  prefix. A "transient" directory may be the only recovery copy.
- **The same plugin's prepare/delete/replace must not let an older completion contaminate a newer
  instance** (a late completion belongs to the instance it started for).
- An explicit **"unusable but with a recovery copy retained"** terminal state is allowed; not
  every failure must auto-recover to `ready`.
- **Kill and power-loss are separate cases.** Power loss additionally needs file/dir durability and
  the SQLite journal-mode argument on each platform.

The **algorithm is the implementer's choice**; what is not allowed is writing "SQLite transaction"
or "roll back on failure" in place of these rules. Runtime acceptance (real `/v1` + disk evidence
across kill vs power-loss, Windows file-locking, disk-full) is **not done**.

## 11. Long operations: acceptance, result ownership, recovery, SSE convergence (G4)

`202 + spawn + GET` says where to look, not what a command guarantees. A cross-review confirmed
this as a **design gap** (TASK-040 F01/F02/F03, TASK-042 F02/F03, TASK-045 F03, cross TASK-046 F03)
- **not** a requirement to add a generic "job".

**No generic job object.** What is required is that the **resource** honestly answers: was the
command **accepted**, is it **in progress**, did it **fail**, and is the result **unknown** - and
whether a stale runner can still submit a result.

### Command identity is not resource identity (R1)

A resource can receive **several different legitimate commands in a row**: fork source S twice (C1,
then a new C2), install P@v1 then upgrade P@v2, run two turns. **A resource identity (the plugin id,
the session id) does not identify a command** - so "idempotent by target / by session+kind" is
wrong: it conflates a retry with a new intent, which either swallows the second legitimate command
or re-runs a retry. The two identities are separate:

- **resource identity** (`{id}`) locates a resource and **serializes conflicting commands** on it
  (one at a time, or a stated conflict rule). It does **not** identify an intent.
- **logical command identity** identifies **one intent and its retries**, across the response the
  client did not receive. It is carried on the request (an `idempotencyKey`, a client-reserved
  resource id, a resource `revision`/conditional command, or an equivalent) - the exact field is
  settled in the contract.

The rule set:

1. **Same command identity + same semantic request** (a retry): return / point at the **original**
   command's resource result; do **not** execute twice.
2. **Same command identity + different parameters**: an explicit **conflict** (never silently apply
   the new parameters under the old identity).
3. **A new intent uses a new identity** - even if target, kind and parameters are identical (a
   second fork of the same source is a new command, not a retry).
4. **For start/fork, the identity must be able to relate to the reserved result resource even when
   the first response is lost** - the server may mint the id, but the association must be derivable
   from the command identity, not only from a target the client has never seen.
5. **Acceptance completes only when the command association and the queryable resource state are
   durably recorded** (or there is an explicit recoverable basis). An unknown external side effect
   is never silently replayed.
6. **The association survives restart for a stated retention window**; after it expires, the
   contract states whether a repeat is a new command, refused, or unknown - exactly-once is **not**
   promised beyond the window.

This does not require a generic job, a public generation, a distributed transaction, or a permanent
log - only that the four cases above are distinguishable.

The semantics table:

| operation | acceptance point | what `Location` names | result/error readability | same-resource conflict | retry vs new | restart | cancel |
|---|---|---|---|---|---|---|---|
| install / remove (plugin) | after validity; association recorded | the plugin row | state `installing`/`removing` -> `ready`/`absent`/`failed`+detail | a conflicting command on the same plugin is refused or joined (resource identity) | by **command identity**: same+same returns the original; same+different conflicts; new identity = new intent (install v1 then v2) | recovery per 10 | stops the work, state says so |
| start / fork / compact (session) | after validity; association recorded | the created/affected session | session `status`/`activeTurn` | `session_busy` while running | by **command identity**: a second fork of the same source is a **new** command | re-attach or explicit failed | per the operation |
| turn | on admit; association recorded | the session's `activeTurn` | turn state + events | refused while a turn runs | an unknown turn is **not** replayed; a deliberate second turn is a new command | resync on reconnect | adapter cancel ACK != stopped |
| auth flow | on start | the auth sub-operation | `pending/approved/failed/expired/cancelled` | one flow per provider | by **command identity** | sub-operation survives a restart | cancel |

- **Unknown/interrupted terminal states are allowed**; exactly-once beyond the retention window is
  not claimed. A late completion from an old runner must not be accepted as the result of a newer
  instance (same rule as 10).
- **`Location` and the result must be explainable after they expire** - a deleted resource may
  return `404`, but the contract must say whether the result is retained or expired, and why.
- **"No operation object" means no generic job**; existing sub-resources (an auth operation) stay.

**SSE convergence.** WHATWG framing does not solve delivery. The contract must state, and the
implementation must converge on: read-after-subscribe (subscribe, then re-read the resource),
beginning-GET/subscribe window (no change lost between the initial read and the subscription),
slow-subscriber overflow (bounded channel; drop policy), an **expired/invalid `Last-Event-ID`**
(server cannot say what was missed -> client re-reads), restart, and a change arriving during the
resync. The old wording ("draw the event's state directly" vs "re-read") is a real conflict and
**re-read is the target**: an event says *when to re-read*, the resource says what is true.

Runtime acceptance (real `/v1` crash/restart/concurrency scenarios) is **not done**.

## 12. Concurrency: how ADR-0009's numbers are met in Rust

ADR-0009 is a floor and a target, measured, not asserted: **>= 100 concurrent connections** and no
in-flight operation times another out; **thousands of idle connections**; two heavyweight
operations at once without starving each other.

**These are mechanisms, not proof.** A cross-review (TASK-046 F01-F04, TASK-037 F02/F07/F10,
TASK-044 F03) correctly requires the wording to be limited to what a mechanism gives, with the
numbers as **measured later**, not asserted:

| requirement | mechanism (what it gives - and does not) |
|---|---|
| thousands of idle connections | **tokio multi-thread runtime**, one async task per connection; an idle connection is a parked task, **not** a dedicated thread. This is the ordinary result for an event-driven server; it is still to be **measured**, not guaranteed by the runtime. |
| no request path blocks the loop | handlers are `async`; the only blocking work (bundled `rusqlite`, some fs) goes to **`spawn_blocking`**. Treating "no blocking call on the async path" as an **explicit blocking boundary is a code-review convention**, not something the type system proves; review and measurement decide. |
| a long operation does not hold a connection | it is **detached** (`accepted(location, work)` -> `tokio::spawn`); the client is answered `202` at once. |
| bounded acceptance, not only bounded execution | **there must be a bound on how many commands are ACCEPTED**, not just how many run: an explicit budget per operation class, active vs waiting counted separately, fair scheduling by operation/resource, and short reads/cancel still served. A `Semaphore` bounds concurrency; it does **not** by itself promise fair progress. |
| two heavyweight operations do not starve each other | each is a task; bounded parts (downloads) sit behind a `Semaphore`. Fairness is a **policy to define and verify**, not a property the primitive grants. |
| DB locks do not span long work | `rusqlite` access is short and inside `spawn_blocking`; a DB lock is **never** held across a long operation. The lock policy and the mix of install/remove with session writes are to be stated and measured. |
| SSE to many subscribers | the bus broadcasts; a slow subscriber is bounded (bounded channel / drop policy), so one client cannot stall the loop. The exact bound is a chosen number, frozen at implementation. |
| overload/disconnect/cancel/exit | documented behaviour for: stdio/SSE overload, a disconnect mid-operation, cancel, and process exit while work is pending (which child processes are reaped, by whom). |

The measurement is the concurrent-poll method that reproduced the freeze in the old code, turned
into the acceptance test. **ADR-0009's numbers (>= 100 connections, thousands idle, two heavy
operations) are a runtime gate, not yet run.** The minute/second/connection-count numbers proposed
in review are suggestions, not an approved SLO.

## 13. The rules (invariants)

1. **No blocking on a request path (ADR-0009).** I/O is async; blocking work runs in
   `spawn_blocking`.
2. **A long operation is one command.** A long route calls `accepted(location, work)` from
   `transport/`, which answers `202 Accepted` + `Location` (RFC 9110 15.3.3/10.2.2) and runs `work`
   detached. It never holds the connection.
3. **The resource is the only truth.** State is read through the GET routes. There is **no
   generic "job" object** and no second store of progress; a command's outcome is the resource's
   own state (see 11 for the acceptance/result/unknown semantics, and the allowed case of an
   **unknown or interrupted terminal state**). A detached task mutates the resource and emits an
   event. A sub-resource that is itself a real resource (an auth flow) is not a "job".
4. **No compatibility, no migration, no legacy.** A fresh program; nothing reads an old format.
5. **A crate calls only the crates it declares.** The workspace enforces it.
6. **Only `adapter/` spawns a process and speaks stdio.**
7. **Errors are typed and mapped once.** A domain returns a typed error; the transport maps it via
   `contract/errors.json`. No domain writes an HTTP status.
8. **Dependencies are built at the entry.** `main.rs` builds the `AppState` and passes it to the
   routers; no module reaches for a mutable global.

## 14. Traceability of the decisions (G6)

**Why Rust.** ADR-0010 requires a mature transport and forbids hand-rolled infrastructure; it does
not name a language. ADR-0011 says the implementation is private. So Rust is a **choice**, not a
consequence of the ADRs, and it is recorded as a choice with a reason: with the adapter
out-of-process and providers reduced to data, nothing in the hub needs Node, and Rust's runtime
(a multi-threaded executor, a blocking pool, out-of-process I/O) is a good fit for what the ADRs
require. **But the fit is not the fulfilment**: the ADRs' concurrency numbers are measured (12),
and the components are proposed, not proven (below). The old transport (Hono) being a Node choice
does **not** argue against Rust (ADR-0011), and Rust does **not** argue that the old choice was
wrong.

**The crates.** Each row in section 2 is a **proposed substitution** for a hand-rolled piece, to
be confirmed at implementation (a maintained crate with a compatible licence and a maintained
release), not an accepted ADR. Where a crate is a platform choice with alternatives (a keychain
library, a SQLite binding, a TLS backend), the alternative is named and the reason recorded at
implementation time.

**Test policy scope.** This repository's testing policy governs **this** repository. A finding
that this PR "does not restore the hub's test suite" is a **cross-document policy conflict**
(TASK-037 F10) for the owners to settle: a reviewer cannot lift a local restriction, and this
repository's rule is **not** pushed onto another repository as a permanent ban. The adversarial
suite that drives `/v1` is the hub's own; it is not resurrected by this docs PR.

## 15. How the structure makes a change local

1. **Build the frame first.** `hub/` + `crates/transport` + `crates/contract` + `crates/events`,
   with the routers mounted. The surface is proven against `contract/v1.json` before any domain.
2. **Then one area at a time.** `plugins/`; its router, its domain crate and its `db/` module are
   the *entire* blast radius - the workspace makes that literal.
3. **A missed long route is visible.** A long route returns `accepted(...)`; a short route returns
   `json(...)`.
4. **The self-check holds the seam.** `main.rs` compares the mounted surface to `contract/v1.json`
   at boot, so a forgotten route is a refusal to start.

## 16. What this architecture does not decide

- The exact response bodies (they are `contract/v1.json`, settled with the code).
- The adapter's stdio topology (the hub<->adapter contract; unchanged).
- C - a client connecting directly to an adapter (a separate decision; re-opens ADR-0001).

**Stage and gates.** This is the **architecture stage**: it states decisions and boundaries. The
runtime verification of the decisions and the feasibility unknowns is **turned into implementation
tasks with exit conditions** in `docs/review/VERIFICATION-TASKS.md` (T1-T6), not claimed here. No
task is a merge condition for this stage; two of them (T2a execution boundary, T2b skills hook per
harness) can still change a route and are called out as such.

## 17. The order of work

1. **the frame**: `hub/`, `crates/transport`, `crates/contract`, `crates/events`.
2. **the data layer**: `crates/db` + the schema.
3. **domains, one at a time, outside-in**: `plugins` first (the reproduced defect, the richest
   state), then `sessions`, then `adapter`, then `providers`, then `connections` / `harnesses` /
   `skills` / `extensions` / `humans`.
4. **the adapter library** (`adapters/adapter-lib`) so the adapters stop copying, and the baseline
   capabilities land once.

Every step is proven by the adversarial suite against `/v1`; the concurrency acceptance is
measured with a concurrent poll (ADR-0009), not asserted.

## 18. Implementation status

This section records where the work actually stands. It is a **status**, not a new decision.

**It was wrong before.** An earlier version of this section called whole domains "landed and
verified" and pointed at component tests as if they proved the product capability. A cross-review
(TASK-048) showed the opposite: sessions were never handed to an adapter yet answered `active` /
`running` / `fork` success; a "concurrency" test proved a test-only route, not the product; the
`202` install task did synchronous work off `tokio::spawn`. Those claims are **retracted**. What
follows distinguishes a real, narrow component fact from a product capability.

### What is a real, narrow component fact

- **T1 contract normalization** (`crates/contract`): the `contract/v1.json` DSL normalizes to
  valid JSON Schema 2020-12 and `emit-openapi` regenerates `contract/openapi.json`. This is a
  property of the normalizer over the contract file; it does not depend on any domain.
- **T3 recovery rules** (`crates/db`): the install/replace step spine and its boot sweep recover
  across a crashed writer. This is a property of the data layer over real files.
- **The event bus** (`crates/events`): monotonic ids, a bounded replay window, and resync.
- **The transport primitives** (`crates/transport`): `Accepted` (`202 + Location`), bounded
  admission (`503 + Retry-After`), and SSE framing. These are mechanisms, not product behaviour.

### What is NOT a product capability (and is currently dishonest if it says so)

- **sessions** (now): a session **owns a real adapter process** (`crates/sessions/src/runtime.rs`).
  `create` runs `session/start` + `config/set` against a real adapter and reports `active` **only**
  when both succeeded; the native `ref` is a real file. Create carries a **command identity**
  (`Idempotency-Key`, R1): a retry returns the same session, a different key is a new session, the
  same key with a different body is `idempotency_conflict`. `close` stops that session's process
  (record stays, status `readonly`); `reopen` re-attaches on the stored ref. Verified live against
  the pi adapter. Still unavailable: **turn / fork / compact / patch** answer `501` - they need the
  turn lifecycle, which is not built. No `active` is ever written on a row alone.

  `POST /v1/sessions` is a **long command**: `202 Accepted` + `Location` (a header; the body is
  `{session}`), the resource answers `starting` -> `active` / `starting_failed` (`startError`).
  The command identity (`Idempotency-Key`) is decided **atomically in one transaction** that
  inserts the `starting` row and the command association together, BEFORE any body validation:
  the same key returns the original, the same key with ANY different request is
  `idempotency_conflict`, a new key is a new command. A refused body (an unsupported field:
  `modelProviderId`/`modelId`/`presetId`/`plan`/`review`/`additionalDirectories`) undoes the
  reservation and returns the refusal - a `starting` row is never left for a refused command.
  The **accept path is side-effect-free** (an existence check only); the placement work and the
  adapter start run in the **detached** start. A start interrupted by a restart is reconciled at
  boot: a `starting` session -> `starting_failed`; an `active` session whose process is gone ->
  `needs-repair` (an orphaned tail, reopenable on its stored ref); any open turn is settled
  (`failed`, or `interrupted` when it was `cancelling`). `close` confirms the process exited (a
  stop failure is surfaced, not claimed as release) and is idempotent; `reopen` refuses a session
  that is still running and re-grants a managed provider. A database created by an older build
  gains the new columns through the additive `migrate` path on open.

  **create also accepts a hub-managed provider and the session knobs**: `modelProviderId`/`modelId`
  (resolved through an injected resolver -> `credentials/grant` -> `config/set`, with the adapter's
  `applied` identity CONFIRMED and persisted), `presetId` (confirmed against `applied.preset`),
  and `plan`/`review` (confirmed against `applied.plan`/`applied.review`). `null` plan/review means
  "do not intervene". `PATCH /v1/sessions/{id}` is now real: policy knobs
  (`plan`/`review`) apply during a running turn, model/provider/preset/thinking require
  an idle turn (`409 session_busy`), a title is renamed IN the harness, and a switch
  runs the same resolver -> grant -> `config/set` -> applied-confirmation path. `messages`/`stats`/`skills` are
  now real read-through views (a read starts the process if needed, caches nothing).
  `compact` and `fork` are
  now real (`compact` reports the harness's own result; `fork` starts a new session
  from a completed-turn anchor, source untouched). Still `501`: `artifacts`, `repair`,
  `resources`.

- **turns** (now): `POST /v1/sessions/{id}/turns` is a long command (`202 + Location`). The turn
  identity is the body `idempotencyKey`, reserved **durably** by the UNIQUE
  `(session_id, idempotency_key)`; the same key with the same content returns the original turn,
  the same key with different content is `idempotency_conflict`, and a **deliberate new turn while
  one runs is refused** (`session_busy`, never queued). The turn runs detached: the hub sends
  `session/prompt` on the **session's own process** and pumps the adapter's notifications into the
  event bus (`turn.*`, `message.*`). `GET /v1/sessions/{id}/turns` lists the turns; `cancel` sends
  `session/abort` and the turn ends on the adapter's end. **A real model call needs a provider
  credential the hub does not have here, so a turn without one ends `failed` with the adapter's
  own reason - honest, not a fake success.** The plumbing (admit -> prompt -> events -> terminal
  state) is verified against the real pi adapter; the model call itself is a credential dependency,
  not claimed.
- **providers** (now): a provider's relationship state (endpoint, protocol, declarations,
  selection, cached catalog) is **rows in the database** (`providers` table, TASK-048 P1 - the
  old `{id}.json` file store is gone). Its **credential** is in the **OS secret store**
  (`crates/secrets`, `keyring`) as a **per-instance-namespaced reference** (`<instance>:provider-<id>`),
  never a value in a row or a file. The store is **probed at boot with a unique name** (it cannot
  clobber a real entry) and scoped to the instance; unreachable -> the credential path refuses
  (`501`). `create` writes the row FIRST (a duplicate `already_exists` has no credential side
  effect) then the secret; `delete` removes the credential AND the row; `logout` removes the
  credential only. `tokenConfigured` is **read back from the secret store** at GET/list (never a
  persisted flag). A provider row carries an **`incarnation`** (minted on insert) and a
  **`revision`**; `save_provider` is guarded by `(id, incarnation)` and `refresh` re-reads after
  its network await and refuses a changed incarnation/revision, so a stale writer cannot overwrite
  a rebuilt provider. `PATCH /v1/model-providers/{id}/models`
  replaces the enabled selection (stored on the provider, so a refresh never changes it).
  Still `501`: `GET /v1/model-providers/types` (the provider-type data surface) and
  `POST /v1/model-providers/{id}/auth` + `GET`/`cancel` (the OAuth/device-code flow).
- **the served binary** (now): every route requires a **bearer token** (the inbound boundary
  exists). That is a possession check, not an authorization boundary (§7). It also serves its own
  metadata: `/v1/surface` (the mounted routes + contract identity with its sha256 + the event
  names), `/v1/openapi.json` (byte for byte as generated from the contract) and `/v1/shutdown`.
- **skills / humans**: the hub stores skills and lists/removes them; approvals and questions are
  routed to the adapter. Neither is a finished product surface (skills file read/write is `501`).
- **connections** (now): the hub-managed connections are real (rows in the `connections` table,
  credential in the OS keychain as a per-instance reference, an incarnation/revision guard,
  enable/disable/delete). `disabled` means zero materialization; `materialize()` hands only
  ENABLED connections to a session, and a session's adapter receives them as env vars
  named by each connection's `envName` (injected resolver; disabled => zero). Still NOT built:: there is no managed-connection domain. `HarnessEnv.connection_env`
  exists but is always empty; the `/v1/connections*` and `/v1/harnesses/{id}/connections*` routes
  are not mounted. This is the largest remaining domain (§17 order: after providers).
- **harnesses**: the thin projection (`GET /v1/harnesses`, and `presets`/`models`/`tools`/
  `extensions` gated capability calls) is real, and `PATCH /v1/harnesses/{id}/extensions` selects
  the DURABLE extension set the next start installs. Harness `auth` is not mounted.
- **plugins**: install/get/list/remove/prepare/recovery are real; `enable`/`disable` are real and
  DURABLE (a `harness_status` table; a disabled harness refuses session create/turns and survives a
  restart). The catalog (`GET /v1/plugins/catalog`, a verbatim restatement of the registry file
  with a `fault` when absent), registry refresh (`POST /v1/plugins/registry/refresh`, the ONLY place
  `AGENT_HUB_REGISTRY_URL` is contacted) and the icon route are real.

### What was fake and is being removed

- component tests that drive a **fake adapter** or a **test-only route** and are described as
  product acceptance (retracted above; the tests stay as component tests, not product evidence);
- a "concurrency" number that counted **status requests**, not concurrent idle connections;
- an install task that is `tokio::spawn`ed but runs synchronous fs/SQLite work.

### Order of work now

1. Correct `contract/v1.json` to the **decided** surface (`ROUTES-REVIEW.md` decided sections),
   because the contract is the interface (ADR-0011) and it currently encodes the "before" surface.
2. Make every unconnected capability **honestly unavailable** (no `active`/`running`/`fork`
   success without an adapter; no plaintext secret accepted; no unauthenticated mutators served
   as a product).
3. Retract unsupported wording (this section and the READMEs).
4. Only then continue, one **real** product capability at a time, end to end.

The T2a / T2b feasibility unknowns (an OS execution boundary; the skills hook per harness) are
still unresolved and tracked in `docs/review/VERIFICATION-TASKS.md`.

## 19. The provider→session grant chain (TASK-048 REVIEW-495ce94)

- **Instance identity is PERSISTED** (`hub_instance` row, created once at open with a
  random id). The secret store service is `agent-hub:<instance-id>`. It is **not** a
  hash of the path, so a **moved data dir keeps its credentials** and two dirs never
  collide. `Db::instance_id()` is the single source.
- **The row owns its credential reference.** `secret_ref` is written when a credential
  is stored and cleared when it is removed; `tokenConfigured` is still read from the
  store (never a stale flag).
- **One operation lock per provider id.** Every operation that touches BOTH the row and
  the keychain (create/patch/delete/logout) holds it for the whole operation, so a
  create cannot interleave with a delete and leave an orphan credential. The DB mutex
  only guards a single SQL call; this covers the two-store span.
- **The credential-transition journal** (`provider_ops`) records an in-flight
  transition BEFORE the keychain write. A boot sweep (`Providers::recover_pending`)
  resolves what remains: a completed delete is finished, a create whose credential
  landed without the row is removed, a row without its credential clears the
  reference. Cross-store failure is thus an **inspectable, recoverable state**, not an
  unowned secret.
- **Turn admission is one atomic decision** (`Db::admit_turn`): identity (UNIQUE
  `(session_id, idempotency_key)`) AND "is the session busy" are decided in a single
  transaction. A refused admission leaves **no** `admitted` row.
- **The turn terminal is the adapter's own `turn_end`.** `session/prompt` returns
  `ok|aborted|failed`; that is the terminal (`completed|cancelled|failed`), never a
  pre-prompt cancel snapshot and never "any RPC success = completed". An unreadable
  state is `failed`. `Db::end_turn` is guarded by `WHERE ended IS NULL`, so the settle
  happens once and a late non-terminal write cannot resurrect a settled turn; the
  event is published **only** when this call won the settle. Cancel checks abort
  delivery (`abort-failed`) and does not settle the turn itself.
- **The prompt does not hold the lifecycle lock.** The request handle is cloneable and
  lives outside the session process mutex, so `abort`/`stop` are not blocked by a
  running prompt.
- **Grant chain (delivered)**: a session with `modelProviderId`/`modelId` is resolved
  by an injected resolver (composition root; `sessions` does not depend on
  `providers`). Start order is `session/start` → `credentials/grant`
  (`{connectionId,value,url,declarations}`, memory-only) → `config/set`; a reopen
  **re-grants**. The adapter names the provider `hub-<id>`; `applied.modelProviderId`
  and the model are read from the adapter's **proof** and stored. Verified live: a
  registered provider reached pi as `provider="hub-mockp"`, `model="deepseek-flash"`,
  and the endpoint received a real `POST /v1/chat/completions`.

## 20. Provider versioning, recovery ownership, grant confirmation (TASK-048 REVIEW-ed896102)

- **Recovery never guesses (F1).** `recover_pending` removes a journal entry ONLY
  when every necessary step is confirmed. An unreadable credential or row is an
  ERROR that keeps the entry, so the orphan stays visible and retryable. `begin_op`
  replaces the previous entry in ONE transaction. `with_configured`/`resolve_grant`
  read the ROW's own `secret_ref`; a row with none is simply unauthorized - never a
  re-derived key, so a rebuilt id cannot re-acquire an old credential.
- **Every write participates in the version decision (F2).** A provider row carries
  an `incarnation` (minted on INSERT) and a `revision`. `save_provider` is guarded by
  `(id, incarnation)`: a row read before a delete+recreate has a different
  incarnation and its save is refused `Conflict`. `refresh` holds the provider lock,
  re-reads after the network await, and refuses a changed incarnation/revision
  (`revision_conflict`). `set_selection` is serialized on the same lock.
- **Grant is confirmed, not assumed (F3).** `config/set` sends `connectionId` (the
  owning contract's provider selection) and `model`. After it returns, the hub
  verifies the adapter's `applied.modelProviderId` equals the requested provider and
  `applied.model` equals the requested model; a mismatch or a missing `applied` stops
  the start. The confirmed `applied.modelProviderId`/`applied.connectionId`/model are
  returned to the service and PERSISTED (`applied_provider`/`applied_route`/
  `applied_model`), and exposed on the session view. A reopen re-grants and re-checks.
- **Cancel holds busy until a real stop (F4).** The cancel intent is recorded
  durably; `run_turn` refuses to dispatch a prompt for an already-cancelled or
  settled turn. An abort SEND failure does NOT settle the turn (the prompt may still
  run) - the turn stays held and the caller sees `abort-failed`. The terminal comes
  from the prompt's own return; a cancel whose prompt never returns is settled
  `interrupted` by `reconcile_stalled_cancels` at boot. `settle_turn` commits the
  terminal in the database (retrying a transient failure) and publishes only on a
  confirmed write.
- **Grant errors keep their identity (F5).** `provider_unauthorized`,
  `provider_not_found`, `provider_catalog_failed`, `revision_conflict` and
  `catalog_not_loaded` are distinct; the resolver returns `<code>|<message>` and the
  sessions domain preserves the code to the response.

## 21. Restart reconciliation and non-blocking placement (TASK-048 N2/N3/N4)

- **N2 — restart reconciliation.** At boot no session process runs. Any session
  claiming to run is repaired: `starting` -> `starting_failed`; `active` (process
  gone) -> `needs-repair` (an orphaned tail; `reopen` restarts it on its stored
  `nativeRef`); `readonly` is left alone. Any turn still open (`ended IS NULL`) is
  settled honestly (`failed`, or `interrupted` if it was `cancelling`), so an
  orphan can never hold `busy`. Verified live: kill-without-close -> restart ->
  `needs-repair` -> `reopen` -> `active`.
- **N3 — no live process behind a broken row.** Both `run_start` and `reopen`
  stop the process if the session row cannot be persisted after a successful
  spawn, and mark the session `needs-repair` rather than leaving a live adapter.
- **N4 — placement off the request path.** The accept path does only a cheap
  existence check with no side effect. The blocking placement (deleting/creating
  the shared harness dir) runs in `spawn_blocking` inside the detached start/reopen,
  so it never stalls the async runtime.

## 22. Session presets (TASK-048)

`presetId` is a session composition (which tools/persona the agent runs), not a
knob. The hub:

- lists a harness's declared presets (`GET /v1/harnesses/{id}/presets` ->
  `presets/list`), pointing the adapter at the plugin's own `<plugin>/presets/` dir
  via `AGENT_HUB_PRESETS_DIR` (the harness declares the `presets` capability);
- accepts `presetId` on create and sends it in the first `config/set`
  (`config.presetId`);
- **confirms** `applied.preset` equals the requested id before the session is
  `active`; a mismatch or a missing confirmation fails the start;
- persists `preset_id`/`applied_preset` and exposes them on the session view;
- a reopen re-sends the preset the same way.

A preset is fixed once a turn has run (the adapter answers `agent-preset-locked`
and the hub surfaces it as a start failure), never a silent keep of the old one.
An unknown preset is refused by the adapter and the session ends `starting_failed`
with the adapter's reason.

## 23. plan / review at create (TASK-048)

`plan` and `review` are session-scoped knobs (`config.plan` / `config.review`),
confirmed against the adapter's `applied.plan` / `applied.review` and persisted
(`applied_plan` / `applied_review`, exposed as `appliedPlan` / `appliedReview` on
the session view). A requested value the adapter does not confirm fails the start -
we never record a knob the harness did not apply. `null` means "do not intervene".
Verified live: `{plan:true, review:true, presetId:standard}` -> active with
`appliedPlan=true`, `appliedReview=true`, `appliedPreset=standard`.


## 25. Review follow-up: PROVIDER-TURN-REVIEW-3b6730b3

- **F1**: a second `begin_provider_op` is REFUSED while one is pending (no overwrite);
  `finish_provider_op_id` clears the exact op; a no-token create begins/finishes
  nothing; `patch` reads the row's reference and aborts on a read error.
- **F2**: `resolve_grant` is async, holds the provider lock, and refuses while a
  transition is pending — config+credential are one snapshot.
- **F3**: the native route (`applied.connectionId`) must be present; `accept_turn` takes
  the session lock a PATCH holds; a post-`config/set` failure quarantines the session
  (stop + `needs-repair`).
- **F4**: cancel takes the session lock; `run_turn` re-checks the durable cancel state
  right before dispatch; a core cancel timeout stops the adapter and settles
  `interrupted`.
- **F5**: `internal_error` passes through; the bus keeps the adapter's typed `data`.
- **F4 refinement**: `StartError::Refused` distinguishes an adapter answer from a
  transport error; the execution timeout covers both an unconfirmed cancel and a stuck
  `running` turn.
- **N4**: `harness_env` holds a per-harness placement lock across the shared-tree
  rebuild, so concurrent starts cannot half-swap it.

## 24. The remaining surface (single source of truth)

Compiled from `contract/v1.json` (74 endpoints) against the mounted route tables.
Regenerate whenever a route lands. A route not mounted is unfinished work, not a defect: `selfcheck`
logs the count and refuses to START only if the hub serves a route the contract does not declare.
**`docs/tasks/remaining-surface.md` records, per group, which work OUTSIDE the hub blocks each
remaining route** (adapter protocol, the provider-type data model, the plugin-sourced skills
model, artifact recording).

### Real (mounted, real handler) - 56

```
DELETE /v1/connections/{id}
DELETE /v1/model-providers/{id}
DELETE /v1/plugins/{id}
DELETE /v1/sessions/{id}
DELETE /v1/skills/{id}
GET /v1/connections
GET /v1/events
GET /v1/harnesses
GET /v1/harnesses/{id}/extensions
GET /v1/harnesses/{id}/models
GET /v1/harnesses/{id}/presets
GET /v1/harnesses/{id}/tools
GET /v1/model-providers
GET /v1/model-providers/{id}
GET /v1/model-providers/{id}/models
GET /v1/models
GET /v1/openapi.json
GET /v1/plugins
GET /v1/plugins/catalog
GET /v1/plugins/{id}
GET /v1/plugins/{id}/icon/{variant}
GET /v1/sessions
GET /v1/sessions/{id}
GET /v1/sessions/{id}/approvals
GET /v1/sessions/{id}/messages
GET /v1/sessions/{id}/questions
GET /v1/sessions/{id}/skills
GET /v1/sessions/{id}/stats
GET /v1/sessions/{id}/turns
GET /v1/skills
GET /v1/status
GET /v1/surface
PATCH /v1/connections/{id}
PATCH /v1/harnesses/{id}/extensions
PATCH /v1/model-providers/{id}
PATCH /v1/model-providers/{id}/models
PATCH /v1/sessions/{id}
POST /v1/connections
POST /v1/model-providers
POST /v1/model-providers/{id}/logout
POST /v1/model-providers/{id}/models/refresh
POST /v1/plugins
POST /v1/plugins/registry/refresh
POST /v1/plugins/{id}/disable
POST /v1/plugins/{id}/enable
POST /v1/plugins/{id}/prepare
POST /v1/sessions
POST /v1/sessions/{id}/approvals/{aid}
POST /v1/sessions/{id}/cancel
POST /v1/sessions/{id}/close
POST /v1/sessions/{id}/compact
POST /v1/sessions/{id}/fork
POST /v1/sessions/{id}/questions/{qid}
POST /v1/sessions/{id}/reopen
POST /v1/sessions/{id}/turns
POST /v1/shutdown
```

### Mounted but `501` - 8

```
GET /v1/model-providers/types
GET /v1/model-providers/{id}/auth/{op}
GET /v1/sessions/{id}/artifacts
GET /v1/sessions/{id}/resources
POST /v1/model-providers/{id}/auth
POST /v1/model-providers/{id}/auth/{op}/cancel
POST /v1/sessions/{id}/repair
POST /v1/sessions/{id}/resources/read
```

### Not mounted - 10

```
DELETE /v1/harnesses/{id}/connections/{cid}
GET /v1/harnesses/{id}/auth/{op}
GET /v1/harnesses/{id}/connections
GET /v1/harnesses/{id}/connections/schema
GET /v1/skills/{id}/files/{file...}
POST /v1/harnesses/{id}/auth
POST /v1/harnesses/{id}/auth/{op}/cancel
POST /v1/harnesses/{id}/connections
POST /v1/harnesses/{id}/connections/validate
PUT /v1/skills/{id}/files/{file...}
```

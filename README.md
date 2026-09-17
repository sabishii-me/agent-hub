# sabishii-me agent hub

A local hub that manages agent harnesses. One plain Node service — `server.mjs` —
that owns sessions, harnesses, providers, credentials, approvals and the harness
adapters, and speaks HTTP+SSE on loopback. It has no UI of its own.

It was extracted from the PRTS monorepo, where a Rust agent-bus shell was retired in
its favour. That repository is not a dependency: nothing here imports it, and this
directory runs on its own.

Everything HERE travels together: the contract and the OpenAPI projection are read
from this directory at start, and nothing outside is imported. The **harnesses are not
here**: one harness = one plugin = one directory + manifest, and each plugin is its own
repository. A deployment composes them into a plugins directory and hands it to the hub
(`PRTS_PLUGINS_DIR`), so adding or removing a harness is a deployment decision, not a
change to this repository. A hub started without one serves the contract and no
harness, and says so on stdout — so this whole directory can move to a
repository of its own without a code change (verified by running it from an unrelated
working directory).

**Connecting a client (desktop / sidecar):** [docs/CLIENT.md](docs/CLIENT.md) — discovery via
`<PRTS_DATA_DIR>/endpoint.json`, readiness, stale files, restarts, and the two
integration facts (`EventSource` cannot send the token; there are no CORS headers).

**Feature map:** [docs/FEATURES.md](docs/FEATURES.md) — the same ground as this
README, organised by feature: what each one promises, which routes and events carry
it, which harness capability gates it, how it was verified, and what is honestly not
covered.

**Where we are / how to pick this up on another machine:** [docs/STATE.md](docs/STATE.md).

## Fresh checkout (a new machine)

```
node -v                       # node 22+ (developed on 24); nothing else is required
node server.mjs               # prints: agent-hub listening 127.0.0.1:<port>
                              # ...and says it has no harness, because the hub ships none
```

Then install a harness, or point the hub at checkouts you already have:

```
# 1. the hub installs a plugin with git, into its own root (<PRTS_DATA_DIR>/plugins)
curl -s -X POST \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{"source":{"url":"<a plugin repository>","ref":"develop"}}' \
  http://127.0.0.1:<port>/v1/hub/plugins

# 2. ...or read plugins you already have on disk (nobody's tree is written to)
PRTS_PLUGINS_DIR=<a directory of plugin directories> node server.mjs
```

A **plugin** is a directory carrying a `manifest.json`; every harness lives in its own
repository and this repository contains none. A harness runtime (an official release, a
build product) is **not committed** anywhere and is **not installed by the hub either**:
the plugin's own `runtime/prepare` method materialises exactly the version its manifest
pins under `<plugin>/runtime/`, and the hub asks for that, waits, and verifies the
declared command exists. A fresh machine therefore needs network once per plugin, at
install time or at the first session — after that the hub has no network dependency of
its own.

State (sessions, providers, secrets, per-harness homes) lives in `PRTS_DATA_DIR`
(default `~/.prts-core`) and is **not** in the repo. Provider tokens live in the OS
secret store (`secrets/*.dpapi` on Windows), never in a provider file, so on a new
machine you re-enter a token once. Nothing else has to be recreated: the contract,
the adapters and the plugin manifests are all in the tree.

## Run it

```
node server.mjs
```

It binds loopback on an OS-assigned port and prints a line containing
`127.0.0.1:<port>`. Pairing material is written to `<PRTS_DATA_DIR>/endpoint.json`
(`{ port, token }`); the token is a fresh random value per boot (no env override).
Every route needs `Authorization: Bearer <token>` except `GET /v1/harnesses`.

| Env | Meaning |
|---|---|
| `PRTS_DATA_DIR` | state dir (default `~/.prts-core`): endpoint file, sessions, secrets, per-harness homes |
| `PRTS_PLUGINS_DIR` | a directory of harness plugins the hub may READ. Not required: the hub searches the roots listed under [Installing a harness](#installing-a-harness), and prints them at startup. The root it WRITES to is always `<PRTS_DATA_DIR>/plugins` |
| `PRTS_APPROVAL_TIMEOUT_MS` | approval deadline (default 120000) |
| `PRTS_CANCEL_TIMEOUT_MS` | cancel deadline (default 15000) |
| `PRTS_TURN_TIMEOUT_MS` | turn deadline (default 0 = unbounded) |

## Installing a harness

A hub with no harnesses is a working hub: it serves the contract, and `GET /v1/harnesses`
is an empty list that is TRUE (it means "none are installed", because the hub is the one
that knows what is installed).

```
GET    /v1/hub/plugins                # what the hub can see, from which root, runtime on disk?
POST   /v1/hub/plugins                # {source:{url, ref?}} -> git clone into <DATA_DIR>/plugins/<id>
POST   /v1/hub/plugins/{id}/prepare   # ask the plugin to materialise its runtime
```

Installing is git and nothing else: the hub clones the repository, reads the id out of
its manifest, refuses an id that would step outside its own root, and records where the
plugin came from. The runtime is then the plugin's business — the hub calls the
adapter's `runtime/prepare` method, which installs exactly the version the manifest pins,
and the hub verifies the declared command exists afterwards. A plugin that brings its own
runtime declares no `runtime` capability and answers `501 unsupported`, which is the
truth about it rather than a guess.

### Where a hub looks for plugins

A hub that cannot find its harnesses is useless, so this is a search path, printed at
every startup (with `(none)` on the roots that do not exist) — not one directory
somebody has to remember to pass in:

| # | root | notes |
| --- | --- | --- |
| 1 | `PRTS_PLUGINS_DIR` | explicit; **read-only** to the hub |
| 2 | `<this directory>/plugins` | a hub that carries its own plugins |
| 3 | `<deployment>/plugins` | used when this hub sits at `<deployment>/apps/<name>` — i.e. when it is checked out as part of a deployment that composes plugins |
| 4 | `<PRTS_DATA_DIR>/plugins` | the hub's **own** root: `POST /v1/hub/plugins` installs here and nowhere else |

A directory a deployment put on that path is somebody else's tree: the hub only reads it.
The same harness id in two roots is a conflict refused at startup, not a silent
preference. And an empty `GET /v1/harnesses` says where it looked (`note`), because an
empty catalogue with no reason is indistinguishable from a broken hub.

## Surface

Everything is under `/v1` (contract: `contract/v1.json`; expect it to keep moving —
it is not frozen). **The contract is the authority, not this table:** it lists
every route with its shapes, the hub reports the routes it implements at
`GET /v1/hub/surface`, and the hub refuses to start if the two disagree. The
table below is a guide to the routes a consumer usually needs.

| Method | Path | Purpose |
|---|---|---|
| GET | `/v1/harnesses` | list harnesses (**the only unauthenticated route**) |
| GET | `/v1/hub/surface` | the surface this process implements: routes, SSE event names, contract sha256 |
| POST | `/v1/harnesses/{id}/enable` \| `/disable` | toggle a harness |
| GET | `/v1/harnesses/{id}/presets` \| `/models` \| `/tools` | harness catalog |
| GET/POST | `/v1/hub/providers` | hub-managed providers: `{id, label, url, api, declarations, selection, catalogRevision, tokenConfigured}` — the file also carries the catalog and the endpoint it was fetched from; token write-only |
| — | **model availability is the hub's fact for the hub's providers**: a managed model is available iff the hub holds a credential for that provider (the harness cannot know — a managed token reaches it only at session start). A harness's own routes keep its own verdict; the user's own dsh credentials are not copied into the isolated home, so their `deepseek-official` routes read `needs-auth`. |
| GET/PATCH/DELETE | `/v1/hub/providers/{id}` | read / change / remove one provider |
| POST | `/v1/hub/providers/{id}/logout` | drop the stored token, keep the registration |
| GET | `/v1/hub/providers/models/list` | API 1: the managed providers' catalogs |
| GET/PATCH | `/v1/hub/providers/{id}/models` | cached catalog / replace the enabled selection |
| POST | `/v1/hub/providers/{id}/models/refresh` | re-read the provider's catalog |
| GET/PATCH | `/v1/hub/harnesses` · `/v1/hub/harnesses/{id}` | harness registry / change its installed extensions, skills, enrolment |
| POST | `/v1/hub/harnesses/{id}/enable` \| `/disable` | enrolment |
| GET/POST/PATCH/DELETE | `/v1/hub/skills/{id}` · `/files/{file...}` | one skill: read, write a file, remove the directory |
| GET/POST | `/v1/hub/connections` | connection registry |
| PATCH/DELETE | `/v1/hub/connections/{id}` | change / remove a connection |
| GET | `/v1/hub/skills` | the skills the hub holds (one directory per skill) |
| GET | `/v1/hub/status` | core status |
| GET | `/v1/hub/openapi.json` | the generated OpenAPI 3.1 document for this surface |
| POST | `/v1/hub/shutdown` | shut the core down |
| GET/POST | `/v1/sessions` | list / create sessions |
| GET/DELETE/PATCH | `/v1/sessions/{id}` | read / delete / switch model+preset+tools; `title` renames it in the HARNESS too |
| POST | `/v1/sessions/{id}/turns` | **send a turn — Server-Sent Events** |
| POST | `/v1/sessions/{id}/cancel` | cancel the active turn |
| GET | `/v1/sessions/{id}/messages` | transcript page (`beforeId`, `limit`) |
| POST | `/v1/sessions/{id}/approvals/{aid}` | answer (`decision`, optional `reason`) |
| POST | `/v1/sessions/{id}/repair` | repair an orphaned tail |
| GET | `/v1/sessions/{id}/turns` | per-turn entries (details come from `/messages`) |
| GET | `/v1/sessions/{id}/approvals` \| `/questions` | what is waiting on an answer right now |
| POST | `/v1/sessions/{id}/questions/{qid}` | answer (`answers[]`, one entry per asked question) |
| GET | `/v1/sessions/{id}/artifacts` | the turn's artifacts (empty until a harness produces some) |
| POST | `/v1/sessions/{id}/close` \| `/reopen` | free the session's resources, keep the session / bring it back |
| POST | `/v1/sessions/{id}/fork` | start a NEW session ending at a completed turn (`afterTurnId`); no anchor = the whole conversation |
| GET | `/v1/sessions/{id}/stats` | the harness's own token/cost/context numbers (read-through, nothing cached) |
| GET | `/v1/sessions/{id}/skills` | the skills THIS session's harness actually has, read from the harness itself |
| POST | `/v1/sessions/{id}/compact` | ask the harness to compact its own conversation (`instructions?`); refused while a turn is running |

**Turn stream** (`POST /v1/sessions/{id}/turns`, `text/event-stream`): `turn.admitted`,
`turn.running`, `message.delta` (`kind: text|reasoning`), `message.completed`,
`tool.started`, `tool.ended`, `approval.requested`, `approval.resolved`,
`plan.changed`, `session.compacting`, `session.compacted`, `session.renamed`, `turn.ended`
(`ended` ∈ `completed|cancelled|failed`, plus a `cause`), and `session.stats` as the
last fact of a turn. The two compaction events carry BOTH halves of the promise:
the compaction a caller asked for, and the one the harness did on its own (pi calls
it `threshold`/`overflow`, dsh `auto`) — a full context compacts mid-turn, and that
is exactly the moment a reader of this stream must not be left in the dark.
`session.renamed` is the same idea for a name: dsh titles a session by itself from
the first prompt, and the hub follows the harness (`appliedBy: harness`) instead of
keeping a title only it believes in.

**Providers**: one provider is one file — `providers/<id>.json` in the hub's data dir —
and that file is the provider's whole definition:

```json
{ "id": "a17", "label": "A17", "scope": "system",
  "endpoint": { "url": "http://…/v1", "api": "anthropic-messages" },
  "models": { "gpt-6-astra": { "input": ["text","image"], "reasoning": { "efforts": ["low","high","max"], "default": "high" } } },
  "selection": ["gpt-6-astra", "…"],
  "catalog": { "revision": 1, "fetchedAt": "…", "models": [ { "id": "…", "name": "…", "available": true } ] },
  "endpointRevision": 1, "createdAt": "…", "updatedAt": "…" }
```

- **`endpoint.api`** is the wire protocol the provider speaks, in the harness
  vocabulary (`openai-completions`, `anthropic-messages`, `openai-responses`, …).
  The adapter writes it into the harness's own provider entry, which is why it
  belongs in the definition rather than being assumed: the pools here speak
  `anthropic-messages`, and every adapter used to hardcode `openai-completions`.
- **`models`** is the declaration: what each model accepts. Providers do not
  publish this (DeepSeek's own `/models` answers `{id, object, owned_by}`), so it
  is stated here — thinking levels (`reasoning.efforts` + `default`), modalities
  (`input: ["text","image"]`), display name, cost, window, token limit. It reaches
  the harness through the grant, translated by the adapter into that harness's own
  spelling; a field a harness has no place for is left out rather than smuggled
  into a key it does not define (dsh's model profile has no cost field, so a
  declared cost is simply not passed to it).
  **A model whose modalities are not declared is advertised as text-only**: every
  adapter used to claim `["text","image"]` for every injected model, so a
  text-only model looked like it took images and the image was dropped at
  runtime. Not claiming a modality is the honest default; claiming a missing one
  is a lie that surfaces as a lost input.
- **`selection`** is which models are enabled; `null` means all of them.
- **`catalog`** is what the provider last served. Derived and refetchable, kept
  in the same file so one file describes one provider completely.
- The token is **not** in the file: it lives in the secret store, and responses
  say `tokenConfigured: true` instead of echoing it.

Editing the file by hand is supported: the hub reads it on every operation. A file
that does not parse, or whose name does not match its `id`, is left alone and
reported in `broken` by `GET /v1/hub/providers` — never silently skipped, because a
provider that vanished without a word reads as data loss, and a write must not
delete a file it did not just write.

**Import and export are a copy.** The file is the whole definition, so exporting a
provider is copying `providers/<id>.json` out, and importing one is putting that
file into another hub's `providers/` directory — no endpoint is involved and none
is planned, because an endpoint would only re-describe the same bytes.

The one thing a file cannot carry is the credential: the token lives in the secret
store (per hub, per machine) and never enters a file. So an imported provider
arrives with `tokenConfigured: false` and refuses to run until a credential is
supplied — `PATCH /v1/hub/providers/{id}` with `{ "token": "…" }` — which is also
what keeps an exported definition safe to hand around.

**Approvals**: a harness with a reason channel (pi/jouzu) offers `Reject with Reason`
and the reason round-trips on `approval.resolved.reason`; a harness without one
(deepseek — dsh's outcome is exactly `allowed-once`/`rejected`) does not. Do not assume
the option exists.

## Harness / adapter model

- A **harness** is `<plugins dir>/<id>/` = `manifest.json` + a `*-adapter.cjs`, and the
  directory name is the harness id. Each one is a repository of its own, pinned by the
  deployment that composes the plugins directory; this hub ships none.
- An **adapter** speaks line-delimited JSON-RPC over stdio. Core spawns **one adapter
  process per session** (v1 topology). **How the adapter reaches its harness is its own
  business**: pi/jouzu start a private child; deepseek attaches to a shared `dsh web`
  server (default port 3080) so many sessions share one server. Core neither knows nor
  decides this.
- A harness declares what it can do through `manifest.json#capabilities`
  (`models`, `providers`, `presets`, `plan`, `review`) and the `agent-v1` protocol.
  **Nothing may special-case a harness by id.**
- A harness declares the **runtime it drives** (`manifest.json#runtime`:
  `package`, `version`, `command`), the **extensions it installs**
  (`manifest.json#extensions`, resolving to the directories in that plugin's own
  `extensions/`) and the **presets it lists** (`<plugin>/presets/`, when it has any).
  The runtime is an official release that the plugin's own `runtime/prepare` method
  installs into `<plugin>/runtime/` (gitignored — it is a build product, not source);
  the hub resolves the command against the plugin directory and hands the argv to the
  adapter, which looks nothing up.

### The hub manages harnesses; the hub installs

`harnesses.json` in the data dir is the registration: for each harness, whether it is
enabled, which extensions are installed for it, which skills it is given, and when it
was registered. A plugin that is present is registered on first sight, carrying the
extensions its manifest ships; a plugin that is gone keeps its row, marked `missing`.

**A fork is a new conversation, and the child's own process makes it.** `POST
/v1/sessions/{id}/fork` spawns the child's adapter and tells it where to branch
from, so the source session's log, ref and live process are untouched — pi's fork
rebinds whichever process runs it, which is why it is never run on the source's
own process. The anchor is a completed turn because that is the only cut every
harness makes exactly (pi forks *before* a message, dsh snaps to the end of a
turn), and `GET /v1/sessions/{id}/turns` is therefore the fork menu. A harness that cannot fork
answers `501 unsupported` rather than being handed an empty conversation — the
capability is declared by the plugin, never assumed from its id.

**Installation is the hub's job, so adding an extension to a harness is a registry
change, not a code change in every adapter.** Extensions are the first thing the
hub installs, not the only one: a harness's skills, and later its MCP servers and
whatever else it carries, belong in the same registry and are installed the same
way — one row entry and one installer each, added when that thing is actually
needed. Nothing here is designed as a closed set. Before a harness runs, the hub writes
that harness's selected extensions into `<DATA_DIR>/agents/<id>/extensions/` and passes
the path as `PRTS_INSTALLED_EXTENSIONS_DIR`. An adapter only *places* what it finds
there into the layout its own harness reads (pi/jouzu: `<cwd>/.pi/extensions/<name>/`;
dsh: its home's `.agent-presets` plus the node_modules walk its composition resolves
through) — it never decides what to install. Removing an extension from the registry
removes it from the next install.

| endpoint | purpose |
|---|---|
| `GET /v1/hub/harnesses` | the registry: enabled, extensions, skills, runtime, timestamps, plus the catalogue of installable extensions |
| `PATCH /v1/hub/harnesses/{id}` | change `extensions`, `skills`, `enabled`; an unknown extension name is refused with the list |
| `POST /v1/hub/harnesses/{id}/enable` · `/disable` | enrollment (the `/v1/harnesses/...` twins remain) |

- **No config scope.** Every session runs its harness in the hub's OWN data dir
  (`<DATA_DIR>/dsh-home` for dsh, the agent dir under `<DATA_DIR>/agents/<id>` for pi
  and jouzu). There is no `system` scope and no `scope` field: it pointed the harness
  home at the user's real config (`~/.dsh`), attached to a dsh daemon the user happened
  to have running, and let the USER'S OWN settings decide the permission policy — so a
  session could run with approvals suppressed while the hub believed it answered them.
  A harness's configuration is still borrowed READ-ONLY where it must be (pi's agent
  dir files are copied into the injected dir; dsh's `settings.yaml` is migrated once,
  minus its permission section); nothing is ever written back to the user's home.

## Known open ends

Not gaps we forgot to mention — things that are honestly not there yet, recorded
so nobody reads absence as presence.

| open end | what is true today |
|---|---|
| **artifacts** | The per-artifact route is **not in the contract**: it was declared once and the hub never had it. `GET /v1/sessions/{id}/artifacts` answers `[]` because no harness produces artifacts yet. When one does, the route comes back with a real producer and a contract entry in the same change. |
| **skills** | Shipped: the hub installs a harness's selected skills into `<DATA_DIR>/agents/<harness>/skills` before its process starts (like extensions), the adapter points its harness at that directory (pi/jouzu `--no-skills --skill <dir>`, dsh `DSH_AGENTS_HOME`), and `GET /v1/sessions/{id}/skills` reads back what the harness itself reports. A harness may still refuse one silently — dsh drops a non-kebab name, a missing description, camelCase invocation keys or a nested bundle — which is why the read-back exists and why the answer is `known:false` rather than an empty list when a harness does not answer. |
| **`tools` capability** | `tools/list` is declared in the adapter contract and gated by the `tools` capability. No plugin declares it, so `GET /v1/harnesses/{id}/tools` answers `known:false` for every harness: an empty catalog is never faked. |
| **config scope (removed)** | `scope: system\|private` is gone from the session and provider APIs and from the dsh adapter. It was the default, and it meant: the harness home was the user's own (`~/.dsh`), the hub's presets were installed into it, a running dsh on the default port was attached to, a server the hub had not started was left running on exit, and the user's own permission preset decided whether approvals were asked. Re-adding it means re-adding all of that; `docs/FEATURES.md` records the removal.
| **adapter-internal `providers` surface** | deepseek implements `connections/*` and `auth/*`; the hub calls none of them (the hub keeps its own provider files). Declared in `capabilitySurface` as adapter-internal rather than described as reachable. |

## The contract, and the OpenAPI projection

`contract/v1.json` is the authority: routes, request and response shapes, the SSE
event vocabulary (each event with a payload schema) and the error codes. The hub
reads it at start — it refuses to start without it — and reports its sha256 at
`GET /v1/hub/surface`. The hub refuses to start unless the file and the surface it
implements agree in both directions (see "Verifying it").

`contract/openapi.json` is a **projection** for tools (client generators, Swagger
UI, validators), produced by `scripts/emit-openapi.mjs` from `v1.json` +
`errors.json`. It is never a second source: edit the contract and regenerate. The
generator is deterministic, refuses to skip anything it cannot express (an unknown
type token throws with the endpoint that carries it), and `GET /v1/hub/openapi.json`
serves exactly those bytes while `/v1/hub/surface` reports their sha256.

What the projection cannot carry, and where it lives instead: the adapter protocol
(stdio JSON-RPC — `contract/adapter-v1.json`: every manifest, every request the hub
sends, every event an adapter emits and every event the hub handles is declared there)
and the reasoning behind each decision (the contract's `description` fields, carried
through verbatim).

## What a hand-run looks like, and what is still not covered

A feature is driven the same way a consumer would drive it: start a hub with a data
dir of its own, read `endpoint.json` for the port and token, then ask `/v1`. The
facts that come back — a status, an SSE event, a file on disk, a number the harness
reported — go into the commit message. No assertions, no green line: a claim that
cannot be read as a fact is a claim nobody can check.

Known places where a hand-run covers less than it looks like, kept visible:

| caveat | what was not driven |
|---|---|
| the capability gate (501 `unsupported`) | Never exercised: every installed harness declares the capability, so the refusal path has never run. Exercising it needs a stub plugin without the capability. |
| `thinking` on deepseek | The pool counts `reasoning_tokens` and returns no `reasoning_content` (measured), so what the stream carried is the upstream's behaviour at that moment, not a promise about the upstream. |
| harness registry mutations, provider model refresh/PATCH | The hub answers them and the boot self-check keeps them in the contract, but their SEMANTICS were checked once by hand, not driven since. |
| dsh's automatic compaction | Seen once (a 40000-token window made dsh compact during a turn, `reason:'auto'`), but only in that configuration. |
| skills on a harness that is slow to warm up | jouzu's first catalogue answer measured 9s once and 70s once, then 24ms: the hub bounds the read-back at 60s and says `known:false` with a reason when it does not arrive. |

## Decided, not built yet

The order these land in is the order they are written here. Each one is a decision
already taken, not a wish: recorded so it cannot be quietly forgotten.

| next | what it is |
|---|---|
| ~~1. session stats~~ **done** | Shipped: `GET /v1/sessions/{id}/stats` plus a `session.stats` event on the turn stream after `turn.ended`, read through from the harness (pi/jouzu `get_session_stats`; dsh `tokenUsage` + `contextPressure` projections). Differences are kept honest rather than smoothed: dsh states no cost, so `cost` is absent there, and the window it reports is its own route capacity, not the provider file's declared one. |
| **1b. session stats (original note)** | `GET /v1/sessions/{id}/stats` — tokens, cost and **context-window usage**, read through from the harness (pi/jouzu `get_session_stats`, dsh `session.history` projections: `sessionStats`, `tokenUsage`, `contextPressure`, `contextBreakdown`). Deliberately a PULL snapshot plus a `session.stats` event on the turn stream after `turn.ended` — the numbers change when a turn ends, and the caller is already reading that stream. There is NO separate stats stream: dsh pushes projection updates, but pi and jouzu push nothing, and a hub-made "live" stream on pi would be polling in disguise. |
| ~~2. compact~~ **done** | Shipped: `POST /v1/sessions/{id}/compact` and the harness-initiated half on the turn stream (`session.compacting` / `session.compacted`). Read through, never computed: pi reports the span it replaced plus its own estimate of the rebuilt context; dsh reports the span it replaced plus its own occupancy projection read after the rewrite. A harness with nothing worth compacting answers `compacted:false` in its own words; a compaction that failed is `compact_failed`, never a quiet success. Refused (409 `session_busy`) during a turn — pi's compact aborts the running turn. Measured: pi compacts by itself on `threshold` and on `overflow` (and reports a failed recovery with its own reason); dsh compacts from its own pressure measurement (`reason:'auto'`) and likewise says why when a summary does not fit. |
| ~~3. rename reaches the harness~~ **done** | Shipped: `PATCH /v1/sessions/{id} {title}` pushes the name into the harness (pi/jouzu `set_session_name` read back from `get_state`; dsh `session.rename`, which answers with the title it normalised and accepted) and records BOTH — `title` what was asked, `appliedTitle` what the harness holds, because they differ: dsh collapses runs of whitespace and flattens newlines/tabs, pi flattens a newline. A title at create is pushed the same way. Measured: with no title, dsh names the session itself from the first prompt (`session.renamed`, `appliedBy: harness`) and the hub follows; after a turn the name is in the harness's own storage (pi/jouzu `agents/<id>/sessions/<session>.jsonl`, dsh's storage file). Refused (409) mid-turn and for an empty title, and a field the route does not take is refused by name. |
| later | **steering/queue** (`steer`, `follow_up`, `clear_queue`): the hub refuses a second turn with `session_busy` by decision (C-9), which is a deliberate difference from what pi can do — revisit when a caller needs it. |
| not planned | **`get_tree`**: pi's session file is an append-only tree, so it can show abandoned branches. The hub's model is one branch = one session (each fork is its own session with its own ref), so the tree is a harness-internal view; and only pi/jouzu have it at all (dsh is linear with a seed boundary). |
| tracking | **`thinking` intermittently empty on deepseek**: the pool counts `reasoning_tokens` but returns no `reasoning_content` (7 direct probes, streamed and not, all empty), so the test's expectation depends on the upstream. The dsh mapping is correct; a dedicated debug pass is planned. |

## Legacy core (for the agent retiring it)

The desktop shell's Rust (a sibling application's `src-tauri/src/*.rs`, ~5.5k lines) is the retired
legacy core plus
desktop utilities. The hub calls none of it and shares nothing with it: the adapters
live here, and the shell reaches the hub over HTTP only.

| Rust module | Role |
|---|---|
| `agent_bus.rs` | **the legacy core's plugin seam** — spawns/streams adapters, per-session processes |
| `agent_runtime.rs` | legacy run machinery (bus events → `agent-event`) |
| `harness_config.rs` | legacy PROTOCOL §6 config-plane client |
| `shell_store.rs` | projects/agents/settings document |
| `account_auth.rs`, `asr_contract.rs`, `provider_registry.rs` | account / ASR / provider registry |
| `lib.rs` | Tauri command surface + window utilities |

`lib.rs` states its own scope: *"the Rust side is the agent bus plus a few window
utilities"* — i.e. the agent-bus trio (`agent_bus`, `agent_runtime`, `harness_config`)
**is** the legacy core, and the rest is shell/account/store.

Today the desktop shell reaches it through its own `src/services/backend.ts`, whose
`directWorkspace()` (`isTauriContext() && !hostedRelay`) routes to `invoke('…')` when
true and to `webRequest('/api…')` otherwise. **`/api` is the hosted/relay surface, not
this core** — this core exposes only `/v1`. So a local desktop build currently has no
path to this core at all; that cut-over belongs to the front-end work.

**Which Rust commands already have a core equivalent (by `#[tauri::command]`):**

| Group | Commands | Core equivalent |
|---|---|---|
| agent/session | `agent_start/stop/prompt/approve/sessions/list/context_usage/compact/cancel`, `new_agent_session` | **yes** → `/v1/sessions/*` |
| model/preset catalog | `list_models`, `list_harness_presets`, `harness_catalog` | **yes** → `/v1/harnesses/{id}/{models,presets}` |
| provider registry / secrets | `provider_registry_*`, `connection_secret_*`, `provider_auth_status` | **yes** → `/v1/hub/providers`, `connections` |
| project / workspace | `create/close/reopen/list/merge_project`, `link/unlink_project_location`, `update_project`, `add/remove/create_project_agent`, `open_agent_folder` | no |
| chat | `create_chat` | no |
| account / cloud | `account_status`, `cloud_status` | no |
| provider login | `provider_login_start/prompt/submit/finish/cancel`, `provider_logout` | no |
| desktop shell | `hide_main_window`, `desktop_startup_status`, `retry_desktop_startup`, `open_markdown_link`, `set_desktop_host_bash`, `run_concurrency_settings`, `local_workspace_descriptor`, `app_version` | no |
| local store | `domain_store_get/set` | no |
| escape hatch | `harness_invoke` | no |

The agent-bus trio is what this core replaces. The groups marked "no" are shell/account
concerns that are not part of the agent runtime; whether they belong in this core is an
open question, not something this README answers.

## Verifying it

There is no test suite. A hub checks itself where it cannot be skipped — **at boot**: if
its routing table and `contract/v1.json` disagree in either direction, if it could emit
an event nobody declared (or declares one it cannot emit), if it can answer with an
error code `errors.json` does not carry, or if `contract/openapi.json` describes a
different surface, it prints the differences and exits 1. That takes milliseconds, needs
no provider, and there is nothing to remember to run.

```
PRTS_DATA_DIR=<dir> node apps/harness-hub/server.mjs     # prints its port, writes endpoint.json
curl http://127.0.0.1:<port>/v1/harnesses                 # discovery needs no token
curl -H "Authorization: Bearer <token>" http://127.0.0.1:<port>/v1/hub/status
```

Everything else — a session that runs a turn, an approval that is answered, a fork that
copies a conversation, a compaction a harness performed by itself — is driven **by hand**
through `/v1` when the feature is touched, and the observed facts go into the commit
message. That is deliberate. The alternative was a suite of 29 model-driven scenarios
that took tens of minutes, needed an upstream model to say anything, and mostly
re-verified what a hand-run shows in a minute; it was deleted rather than tuned. A trial
nobody reads is a trial nobody trusts.

The contract still earns its keep without a suite: the hub refuses to start unless it can
read `contract/v1.json`, `contract/errors.json` and `contract/openapi.json`, and
`GET /v1/hub/surface` reports the routes it actually matches, the events it can emit and
the sha256 of both documents — so a consumer can prove which surface it is talking to.

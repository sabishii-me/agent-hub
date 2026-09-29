# Route review (for approval)

Every route in `contract/v1.json` today: method, path, what it does (from the contract),
and a verdict. **Nothing here is changed yet** - this is the review you asked for. The
verdicts are proposals.

Legend: **KEEP** (fine as is) · **RENAME** (path/naming) · **MERGE** (duplicate) · **MOVE**
(belongs to another class) · **ADDAUTH?** (open without a token, on purpose or not).

A fact that decides several verdicts: a client is **always** talking to the hub, so `/v1/hub/`
marks nothing - the prefix is present on some routes and absent on others for no rule.

---

## 1. discovery - open, no token

| route | what it does | verdict |
|---|---|---|
| `GET /v1/harnesses` | the harnesses this hub has registered (a plugin present is registered on first sight, with its capabilities and status). **The one route that answers without a token**; `GET /v1/hub/harnesses` is the management view of the same registry. | **KEEP**, but see class 4 (the two views should be one class) |

## 2. status - the process itself

| route | what it does | verdict |
|---|---|---|
| `GET /v1/hub/status` | core status: identity, what the hub is managing now, the contract it was built against. | **RENAME** `/v1/status` |
| `GET /v1/hub/surface` | the surface this process implements, as data: the route table, the SSE event names, the identity. | **RENAME** `/v1/surface` |
| `GET /v1/hub/events` | the hub-level SSE stream: plugin-set changes. A client showing plugins subscribes once and re-reads on the event. | **RENAME** `/v1/events` |
| `POST /v1/hub/shutdown` | stop the hub. (No description in the contract.) | **RENAME** `/v1/shutdown`; **add a description** |
| `GET /v1/hub/openapi.json` | the OpenAPI 3.1 document for this surface, as generated. | **MOVE** here from `/hub/`; a document is not a resource - keep it but name it for what it is |

## 3. plugins - what the hub installs and carries

| route | what it does | verdict |
|---|---|---|
| `GET /v1/hub/plugins` | which plugin directories the hub can see, where they came from, whether each runtime is on disk. | **RENAME** `/v1/plugins` |
| `POST /v1/hub/plugins` | install a plugin (git or release artifact); an existing id is replaced. | **RENAME** `/v1/plugins` |
| `DELETE /v1/hub/plugins/{id}` | remove a plugin the HUB installed (refuses a deployment's tree, and a harness with open sessions). | **RENAME** `/v1/plugins/{id}` |
| `POST /v1/hub/plugins/{id}/prepare` | ask the adapter to materialise the runtime its manifest pins (idempotent). | **RENAME** `/v1/plugins/{id}/prepare` |
| `GET /v1/hub/plugins/{id}/icon/{variant}` | one variant (light/dark) of a plugin's own icon. | **RENAME** `/v1/plugins/{id}/icon/{variant}` |
| `POST /v1/hub/registry/refresh` | read the configured registry URL and write it where the hub reads the catalog; the ONLY way the URL is contacted. | **RENAME**; `refresh` is an action, not a resource - see "open questions" (a). |
| `GET /v1/hub/catalog` | the plugin catalog the hub was shipped with (the registry file restating where each plugin's releases are). | **RENAME** to say it is the registry cache, and place it by plugins |

**REMOVE (not a route):** `GET/DELETE /v1/hub/orphans`. Leftover manifestless directories are an
INTERNAL problem - the hub's install/replace is not atomic, so it leaves half-directories,
and the route handed that cleanup to the user. The fix is internal: (1) make the replace
atomic (old dir -> `.outgoing`, staging -> dir, then delete `.outgoing`; roll back on failure,
so a directory is never half), and (2) sweep transient dirs (`.staging-*`, `.outgoing-*`,
`.runtime-carry-*`) at boot. The user never sees a leftover and has no route for it.

## 4. harnesses - the hub's harness registry, and each harness's own surface

Two owners share the word "harness", which is correct but must stay clear:
**the hub's row** (enabled, extensions, skills) and **the harness's own answers**
(models, presets, tools, its auth, its own connections).

| route | what it does | verdict |
|---|---|---|
| `GET /v1/hub/harnesses` | the hub's harness registry: per harness, whether it is enabled and which extensions/skills are installed. | **RENAME** `/v1/harnesses/registry`? see "open questions" (b) |
| `PATCH /v1/hub/harnesses/{id}` | change what the hub installs for one harness: `extensions`, `skills`. | **RENAME** likewise |
| `POST /v1/hub/harnesses/{id}/enable` | enable a registered harness. | **KEEP one of the two** - see MERGE below |
| `POST /v1/hub/harnesses/{id}/disable` | disable a registered harness (session create/turns refused 409). | **KEEP one of the two** |
| `POST /v1/harnesses/{id}/enable` | identical (the contract calls the `/hub/` one "the hub twin of" this). | **MERGE** with the `/hub/` one -> delete this |
| `POST /v1/harnesses/{id}/disable` | identical. | **MERGE** -> delete this |
| `GET /v1/harnesses/{id}/models` | the models THIS HARNESS CAN RUN: its own catalog plus every hub-managed provider it can reach. The authoritative source for model selection. | **KEEP** |
| `GET /v1/harnesses/{id}/presets` | the harness's preset roster (opaque ids), capability-gated. | **KEEP** |
| `GET /v1/harnesses/{id}/tools` | the tool catalog, capability-gated. | **KEEP** |
| `POST /v1/harnesses/{id}/auth` | start the harness's OWN authorization for a provider (browser/device code). | **KEEP** |
| `GET /v1/harnesses/{id}/auth/{op}` | the state of that authorization operation. | **KEEP** |
| `POST /v1/harnesses/{id}/auth/{op}/cancel` | cancel it. | **KEEP** |
| `GET /v1/harnesses/{id}/connections` | the connections THIS HARNESS itself holds (a private connection belongs to the harness). | **KEEP** |
| `POST /v1/harnesses/{id}/connections` | create/update one of those. | **KEEP** |
| `GET /v1/harnesses/{id}/connections/schema` | what the harness lets a person connect (its own providers + fields), forwarded from the harness. | **KEEP** |
| `POST /v1/harnesses/{id}/connections/validate` | ask the harness to validate a draft (may probe; persists nothing). | **KEEP** |
| `DELETE /v1/harnesses/{id}/connections/{cid}` | delete one; the harness drops its credential too. | **KEEP** |

**MERGE (the one real duplicate):** `POST /v1/harnesses/{id}/enable|disable` and
`POST /v1/hub/harnesses/{id}/enable|disable` are the same action, same auth. Keep one.
Proposal: keep the harness-registry pair, drop the `/v1/harnesses/{id}/enable|disable` pair
(they have no description, which is a sign they were the later add).

## 5. model-providers - the model services the hub manages (and their plugin types)

The system's word is **model provider** (a service that serves models: endpoint + api +
credential + catalog), not a generic "provider" - that word was occupying too much. The
plugin kind is `model-provider` (the twin of `harness-adapter`). Distinct owner from class
4's harness-connections: these are the **hub-managed** model providers.

| route | what it does | verdict |
|---|---|---|
| `GET /v1/hub/provider-types` | the installed model-provider plugins, as data (each imported from a plugin). | **RENAME** `/v1/model-providers/types` |
| `GET /v1/hub/providers` | the model providers the hub manages (one file each; endpoint, protocol, declarations). | **RENAME** `/v1/model-providers` |
| `POST /v1/hub/providers` | register a model provider. | **RENAME** `/v1/model-providers` |
| `GET /v1/hub/providers/{id}` | read one model provider by id. | **RENAME** `/v1/model-providers/{id}` |
| `PATCH /v1/hub/providers/{id}` | change a model provider (label, url, api, token, declarations). | **RENAME** `/v1/model-providers/{id}` |
| `DELETE /v1/hub/providers/{id}` | remove a model provider and its credential. | **RENAME** `/v1/model-providers/{id}` |
| `POST /v1/hub/providers/{id}/logout` | drop the credential, keep the row. | **RENAME** `/v1/model-providers/{id}/logout` |
| `GET /v1/hub/providers/models/list` | API 1: the models the HUB manages (across its model providers) - what the hub owns and can inject. Explicitly NOT the model-selection source and does NOT include a harness's own models. | **MOVE** to its own class -> `/v1/models` (see class 10) |
| `GET /v1/hub/providers/{id}/models` | the cached catalog of ONE model provider (no network). | **RENAME** `/v1/model-providers/{id}/models` |
| `PATCH /v1/hub/providers/{id}/models` | replace the enabled selection. | **RENAME** `/v1/model-providers/{id}/models` |
| `POST /v1/hub/providers/{id}/models/refresh` | refresh one model provider's catalog (a network fetch). | **RENAME** `/v1/model-providers/{id}/models/refresh`; an action - see "open questions" (a) |
| `POST /v1/hub/providers/{id}/auth` | start the model-provider plugin's authorization (device-code/browser). | **RENAME** `/v1/model-providers/{id}/auth` |
| `GET /v1/hub/providers/{id}/auth/{op}` | the state of that authorization. | **RENAME** `/v1/model-providers/{id}/auth/{op}` |
| `POST /v1/hub/providers/{id}/auth/{op}/cancel` | cancel it. | **RENAME** `/v1/model-providers/{id}/auth/{op}/cancel` |

## 6. connections - what the hub manages

Distinct owner from class 4's harness-connections: these are the **hub-managed**.

| route | what it does | verdict |
|---|---|---|
| `GET /v1/hub/connections` | the connections the hub manages, credential-free. | **RENAME** `/v1/connections` |
| `POST /v1/hub/connections` | register a connection (scheme is free; token to the secret store). | **RENAME** `/v1/connections` |
| `PATCH /v1/hub/connections/{id}` | change a connection (a new token replaces the stored one). | **RENAME** `/v1/connections/{id}` |
| `DELETE /v1/hub/connections/{id}` | remove a connection and its credential. | **RENAME** `/v1/connections/{id}` |

## 7. skills - the skills the hub holds

| route | what it does | verdict |
|---|---|---|
| `GET /v1/hub/skills` | the skills the hub holds (one directory each). | **RENAME** `/v1/skills` |
| `GET /v1/hub/skills/{id}/files/{file...}` | read one file of a skill, byte for byte. | **RENAME** `/v1/skills/{id}/files/{file...}` |
| `PUT /v1/hub/skills/{id}/files/{file...}` | write one file (the hub is a courier; it does not interpret). | **RENAME** likewise |
| `DELETE /v1/hub/skills/{id}` | remove one skill directory. | **RENAME** `/v1/skills/{id}` |

## 8. sessions - one harness conversation

| route | what it does | verdict |
|---|---|---|
| `GET /v1/sessions` | list sessions (deleted never listed; closed unless `?includeClosed`). | **KEEP** |
| `POST /v1/sessions` | create a session (spawn the adapter, handshake, prepare). | **KEEP** |
| `GET /v1/sessions/{id}` | read one session. (No description.) | **KEEP**; **add a description** |
| `PATCH /v1/sessions/{id}` | change session configuration (model provider, credential grant, ...). | **KEEP** |
| `DELETE /v1/sessions/{id}` | ACP session/delete: remove from `session/list` (record kept, file untouched). | **KEEP** |
| `POST /v1/sessions/{id}/turns` | send a user message; SSE stream follows. | **KEEP** |
| `GET /v1/sessions/{id}/turns` | thin per-turn entries. | **KEEP** |
| `POST /v1/sessions/{id}/cancel` | cancel the running turn (idempotent). | **KEEP**; **add a description** |
| `GET /v1/sessions/{id}/messages` | read-through native message history (`history/page`). | **KEEP** |
| `POST /v1/sessions/{id}/compact` | ask the harness to compact its own conversation. | **KEEP** |
| `POST /v1/sessions/{id}/fork` | start a new session whose conversation ends at a completed turn of this one. | **KEEP** |
| `POST /v1/sessions/{id}/close` | ACP session/close: cancel work, free resources; the session is kept. | **KEEP** |
| `POST /v1/sessions/{id}/reopen` | bring a closed session back (re-attach on the stored ref). | **KEEP** |
| `POST /v1/sessions/{id}/repair` | recover a session whose cancelled turn never confirmed its end. | **KEEP** |
| `GET /v1/sessions/{id}/stats` | session statistics as the harness reports them (read-through). | **KEEP** |
| `GET /v1/sessions/{id}/artifacts` | (No description.) | **KEEP**; **add a description** |
| `GET /v1/sessions/{id}/skills` | the skills THIS session's harness actually has, read from the harness. | **KEEP** |
| `GET /v1/sessions/{id}/resources` | the read-only resource catalogue the session's skill selection authorizes (`skills://` URIs). | **KEEP** |
| `POST /v1/sessions/{id}/resources/read` | read one `skills://` resource (no filesystem fallback; traversal refused). | **KEEP** |

## 9. humans - a harness asking a person to decide

| route | what it does | verdict |
|---|---|---|
| `GET /v1/sessions/{id}/approvals` | (No description.) the approval requests of this session. | **KEEP**; **add a description** |
| `POST /v1/sessions/{id}/approvals/{aid}` | answer a pending approval (allow/deny/always or an offered option). | **KEEP** |
| `GET /v1/sessions/{id}/questions` | the questions the harness asked this session (answer-seeking, not permission). | **KEEP** |
| `POST /v1/sessions/{id}/questions/{qid}` | answer a pending question. | **KEEP** |

## 10. models - the models themselves, three views, three owners

The word "models" hides three different lists. They are kept apart on purpose; each answers a
different question and one must not be derived from another (no reverse coupling):

| route | what it is | owner |
|---|---|---|
| `GET /v1/models` (new; today `GET /v1/hub/providers/models/list`) | **the models the HUB manages** - across its model providers. What the hub owns and can inject. Does **not** include a harness's own models. | the hub |
| `GET /v1/harnesses/{id}/models` | the view **through this harness** = its own models **+** the hub-managed ones it can reach. The selection source for a session on that harness. | the harness (+ hub, merged) |
| `GET /v1/model-providers/{id}/models` | ONE model provider's catalog (cached, no network). | that provider |

**MOVE** `providers/models/list` -> `/v1/models` (top-level: a model is what a user picks, not
a sub-detail of the hub's provider records).


---

## Decided (from the review conversation)

- **Drop `/hub/`.** One prefix, `/v1/`. (A breaking contract change; ships on the contract.)
- **The prefix already forced the duplicates into the open:** once `/hub/` is gone, the
  "discovery `/v1/harnesses`" and the "management `/v1/hub/harnesses`" collide on one path, and
  `enable|disable` has two routes on one path. They must become one route each.
- **`providers` -> `model-providers`.** The system's word is *model provider* (a service
  serving models); "provider" alone took too much. `/v1/hub/provider-types` ->
  `/v1/model-providers/types`.
- **`/v1/models` — three views, three owners, never derived from one another:**
  - `/v1/models` = the models the HUB manages (its model providers only; NOT a harness's own);
  - `/v1/harnesses/{id}/models` = the view through that harness (its own + the hub's it reaches);
  - `/v1/model-providers/{id}/models` = one provider's cached catalog.
- **`orphans` is not a route.** Leftover manifestless directories are an internal defect
  (replace is not atomic); fix it internally (atomic replace + sweep transient dirs at boot).
  The user never sees one.
- **Actions may be verbs in a path.** A resource can have many actions
  (`refresh`, `auth`, `logout`, `enable`, `disable`, `prepare`, ...). REST's noun-style is a
  preference, not a rule; ADR-0009 only says a LONG action returns `202 + Location`, it says
  nothing about verbs. (This reverses an earlier note in this file.)
- **Three layers, kept apart (the abstraction the owner stated):**
  - **plugin** — the *general* plugin logic only: lifecycle, version, install/uninstall,
    metadata. It does NOT implement each plugin's specifics and does NOT leak their detail.
    `enabled` (on/off) is here too, as plugin lifecycle.
  - **extensions** — a top-level mechanism, but each is a **harness-specific** extension.
    Being specific, a bare id is meaningless across harnesses, so it is **private** and lives
    under the harness id: `/v1/harnesses/{id}/extensions` (list + manage, harness context).
  - **skills** — a top-level mechanism, meant to work **across all harnesses**; it has **no
    harness id context**: `/v1/skills`. Content comes from a plugin, but the mechanism and its
    entry point are top-level.
- **plugin API scope**: plugin lifecycle, version, install/uninstall, metadata, enable/disable.
  Not extensions, not skills, not provider records.

## Extensions: two kinds (decided)

There are two kinds of extension, and they are not the same thing:

1. **adapter-shipped** - aligned with the adapter's own function (an approval mode, the preset
   mechanism; for pi/jouzu `agent-presets`/`plan`, for dsh `dsh-presets`/`hub-command-approval`).
   This is **part of the adapter**. It stays in the adapter repository now. Later it may move
   into an extension package, and then the **adapter declares a dependency on it**.
2. **user-authored** - a user's own extension. This is what would be **plugin-ized** later.

**Today there are no user extensions, so both live in the adapter.** No extension plugin kind
is built now. (The facts: an extension here is real files - pi/jouzu TypeScript, loaded by pi
through jiti; dsh `.js`/`.yml` - in each adapter repository's `extensions/<id>/`, declared by
the adapter manifest, copied by the hub and placed by the adapter.)

## Skills and extensions: granularity and delivery (decided direction)

**Decision: extensions and skills must support MORE THAN ONE granularity, not be pinned to
one.** A session **inherits the workspace's** set, and may also carry a **session-private**
set. The layers compose: workspace set (inherited) + session-private additions/removals.

This is not a product preference imposed on the code; the harnesses already work this way (read
from their sources, packed from npm as `@earendil-works/pi-coding-agent`, `jouzu`,
`@deepseek-ai/dsh`):

- **pi** (`dist/cli/args.js`): `--extension/-e <path>` (repeatable), `--no-extensions/-ne`
  (discovery off; explicit paths still load), `--skill <path>` (repeatable),
  `--no-skills/-ns`. So pi loads extensions and skills from **any absolute path** with
  discovery off - the workspace is a choice, not a requirement. **One adapter process per
  session**, spawned with the session `cwd`.
- **jouzu**: the same (`--extension <path>` / `--skill <path>`).
- **dsh**: extensions live in `$DSH_HOME/profiles/...` (outside the workspace); its workspace
  root is the invoking directory; skills resolve **per session's project root**
  (`skill.list({ sessionId })`).

**Consequence for the hub:** the hub must not store a single set on the harness row. The
selection is **layered** - workspace and session - and the adapter resolves the effective set,
then hands the harness explicit paths with discovery off. Today the hub is pinned to the
coarsest (one set per harness); that is the thing to remove.

**SECURITY (the delivery rule).** The agent must not be able to rewrite its skills, and must
not be able to reach (edit) its extensions - today it can edit the gating extension and bypass
approval. The delivery rule that fixes both:

1. place extensions and skills in a **hub-owned directory outside the agent's workspace**;
2. launch the harness with **discovery off** and explicit paths (pi/jouzu: `-ne -e <path> ...`,
   `--no-skills --skill <path>`; dsh: already outside);
3. the agent can then neither add nor edit a trust-bearing extension, nor write a skill;
4. the approval gate rests on the **adapter** (which already sees every tool call) plus an
   extension loaded from the hub-owned dir - never on a file in the workspace.

Where the placement lives is the adapter's choice, and today pi/jouzu place extensions into
`<cwd>/.pi/extensions` with `--approve`. Moving the placement to a hub-owned directory with
discovery off is adapter-side; no harness change. (The adapter-shipped extensions stay part of
the adapter - see "Extensions: two kinds" - the point here is only WHERE they are placed.)

## Model providers are DATA; the hub provides the function (DECIDED)

Owner: **"the function must be provided by the hub; a provider can only be data."** And, on the
same defect seen from another side: **the same login must not be re-implemented inside each
provider.**

What the installed providers do today (read from the plugins), and where each piece belongs:

| today, inside the provider module | who must own it |
|---|---|
| HTTP client: fetch, timeout, redirect policy, size limit, JSON read | **the hub** |
| device-code OAuth: `/device/code`, poll `/device/token`, `authorization_pending`/`slow_down`/`expired_token`/`access_denied`, link ack | **the hub** - one implementation, reused |
| api-key handling | **the hub** |
| catalog fetch: GET, bounded read, JSON, id/name extraction, dedupe | **the hub** (by protocol) |
| vendor catalog dialect (`modelOfferings`, a `vnd.*` Accept header) | a **named hub catalog dialect**, selected by data |
| vendor -> hub field mapping (thinking levels, modalities, limits) | **the hub** for standard dialects; a named hub dialect for a vendor-specific one |
| credential id derivation (e.g. `installId` from hostname) | **the hub** (a general capability) |
| constants: client id, gateway host, protocol version, auth method | **provider DATA** |
| display name/labels, which protocol, config fields | **provider DATA** |

So a provider record is **only data**, e.g.:

```
{ id, name, protocol: "openai-completions",
  endpoint,
  auth: { method: "device-code", gateway, clientId, ... },   // or { method: "api-key", ... }
  catalog: { dialect: "openai-models" } }
```

**Why this matters:** if a provider can embed its own implementation, it can embed **a whole
other ecosystem** - its own HTTP, its own login, its own protocol - and the hub has no boundary.
It cannot audit it, share it, test it once, or guarantee it. Today the same device-code flow is
copied per provider; the hub cannot give "login" one contract (state, cancel, timeout, audit).

**Auth is a hub protocol.** device-code, api-key, and whatever comes next are implemented **once
in the hub**; a provider declares `auth.method` + parameters. A vendor whose flow needs more gets
a **named hub dialect** (hub-implemented), never its own code. Same for the catalog dialect.

**With this, a provider plugin is a data artifact**, not a module the hub runs. That removes
in-process foreign code and is the precondition for the Rust hub.

**(o4) DECIDED - `/v1/harnesses` is a thin, top-level projection that hides "harness is a
plugin".** A harness is a **top-level resource** (the thing a user runs), not a view of the
plugin list. `/v1/harnesses` stays a **thin projection** of the plugin list filtered to
`pluginType === 'harness-adapter'`, for two reasons the owner gave: it **conveniences the UI**
with a stable harness shape, and it **hides the internal fact that a harness is a plugin**. It is
derived from `/v1/plugins`, never a second store. The **runtime answers stay per harness** at
`/v1/harnesses/{id}/...` (models/presets/tools/auth/connections), and
`/v1/harnesses/{id}/extensions` is the extension sub-resource.

**(o5) DECIDED - no anonymous discovery.** `/v1/harnesses` was the only token-free route; the
owner: there is no reason for it. The hub is loopback and a client already holds the token (from
`endpoint.json`). **Every route requires the token** - a contract change.

**(o6-a) can the hub give the harness a virtual filesystem, so skills are never a writable
directory?  ANALYSED - possible but fragile; the robust boundary is the tool layer.**

Facts (from the packed harnesses):

- The harnesses read skills through **`node:fs`** directly. pi's loader is
  `loadSkillsFromDir`/`loadSkillFromFile` (`readdirSync`/`readFileSync`/`statSync`), and its
  `--skill` takes a **path**; **there is no in-memory / content-injection entry**. So
  `skills://` (the hub's read-only, versioned, no-paths protocol) **cannot be handed to a
  harness** - the harness reads a directory, not a protocol. `skills://` stays a **client-facing
  read view**, not a delivery to the harness.
- pi runs as a **Node process** (`engines: node >=22.19.0`, bundled `dist/bundle/cli.js`) and
  imports `fs`. So a virtual filesystem is *possible* by interposing `node:fs` in the adapter's
  spawn (a `--require` hook). But it is **fragile**: pi is bundled, and any fs method the
  interposer misses leaks to the real disk.
- **The agent's `write` tool is NOT confined**: it accepts "relative or absolute" paths
  (`createWriteToolDefinition`: `resolveToCwd(path, ...)`), so an absolute path outside the
  workspace is writable. Without a boundary, the agent can locate and write a hub skill dir.
- **pi has a tool-call interception hook** (`dist/core/extensions/wrapper.js`: "Tool call and
  tool result interception is handled by AgentSession via agent-core hooks"; bash.js mentions
  "extensions that intercept user_bash"). An **extension can intercept a tool call and block
  it**. That extension can be adapter-shipped and loaded from a hub-owned path with discovery
  off - out of the agent's reach.

**So the boundary options, in order of robustness:**

1. **Tool interception (recommended)**: an adapter-shipped extension loaded from a hub-owned
   path (discovery off) intercepts `write`/`bash` and refuses anything outside the workspace;
   skills live in a hub dir the agent then cannot write. Not a VFS - a gate the agent cannot
   edit.
2. **`--exclude-tools`**: blunt (removes a tool entirely); only if interception is unworkable.
3. **A virtual filesystem** (fs interposer in the adapter): possible, but fragile and fights
   the harness; not the first choice.

**(o6) is the security delivery rule settled enough to implement?** The direction is decided
(above): hub-owned paths outside the workspace, discovery off, adapter-enforced gate. What is
left is not a design question but a per-adapter one - confirm each harness accepts the exact
flags (pi/jouzu verified: `-e`/`--extension`, `--skill`, `-ne`/`--no-skills`; dsh already keeps
extensions in `$DSH_HOME`), and decide whether skills are handed as a directory the agent
cannot write, or through the `skills://` read-only protocol the hub already models (nothing
consumes it today).

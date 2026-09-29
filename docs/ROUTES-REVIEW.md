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

## Open questions (the ones still real)

**(o1) skills' granularity: harness-based, session-based, or workspace-based?**
Today a skill's *selection* is stored on the **harness row** (`harnessRow.skills`: null = all,
or an id list) and installed into `<DATA_DIR>/agents/<harness>/skills` before the harness
process starts — so it is **per-harness** today: every session of a harness shares one set.
But `GET /v1/sessions/{id}/skills` reports **per session** (it asks the harness's own
`skills/list` with a sid). So selection (harness) and report (session) do not have the same
granularity.

The question: should a skill set be chosen **per session** or **per workspace** (a session has
a `cwd`), so different sessions/projects can carry different skills? This is a product
granularity decision, not derivable from the code. **Blocked on:** whether the harness
adapters (pi, dsh) can even *do* workspace-based skills (see o2).

**(o2) can the adapters do workspace-based skills?  ANSWERED (adapter sources read). ALL THREE can.**

- **pi** and **jouzu**: **one adapter process per session**, spawned with the session's `cwd`;
  their **extensions are already installed per-workspace** (the adapter copies the extension
  into `cwd/.pi/extensions/`). Skills ride the spawn argv as `--no-skills --skill <dir>`, so a
  per-session/per-workspace skills dir is achievable - the process is already per session.
- **dsh**: one shared server per home, but **dsh's own skill resolution is per session's
  project root** - the adapter queries `skill.list({ sessionId })` and dsh returns "the
  user-invocable skills for this session's project root". So dsh is **already workspace-based
  in its own mechanism**; `DSH_AGENTS_HOME` is only where the hub-installed skills live.

**So all three harnesses can carry skills at session/workspace granularity**, each through its
own mechanism (pi/jouzu: per-session process + argv; dsh: its own per-session project root).
That is exactly the adapter's job (ADR-0010: the adapter translates to the harness's own
dialect) - the hub must not assume one granularity.

**The inconsistency this exposes:** today the HUB stores a skill selection on the **harness
row** and installs one fixed directory per harness, while extensions are installed
**per-workspace** and the harnesses' own skill resolution (dsh) is **per session**. The hub is
the side pinned to the coarsest granularity.

**(o3) extensions/skills "selection" ownership.** With the three layers above: extensions'
selection is harness-scoped (`/v1/harnesses/{id}/extensions`). If skills become
session/workspace-based (o1), their selection moves to the session/workspace, not the harness
row. Pending o1/o2.

**(o4) harness registry vs harness runtime answers.** `/v1/harnesses` today mixes the hub's
registry row (enabled/extensions/skills) with the harness's own runtime answers
(models/presets/tools/auth/connections). With `enabled` going to the plugin lifecycle and
`extensions` to `/v1/harnesses/{id}/extensions` and `skills` to its own mechanism, what is
left of the registry row, and does `GET /v1/harnesses` still exist (or is "which harnesses"
just `GET /v1/plugins?type=harness-adapter`)? Pending the above.

**(o6) SECURITY: the agent must not be able to rewrite its skills, and must not be able to
reach its extensions.** (Owner: skills must not be rewritable; extensions must not be directly
accessible to the agent - today the agent can edit an extension and **bypass approval gating**.)

Facts, from the adapters:

- **Skills** are installed to the hub's data dir (`<DATA_DIR>/agents/<id>/skills`, outside the
  agent's workspace) but nothing enforces read-only. If the harness process can write there, a
  session silently changes what later sessions read (ADR-0001 broken).
- **Extensions are placed into the AGENT'S WORKSPACE** for pi and jouzu
  (`<cwd>/.pi/extensions/`, because that is where pi/jouzu discover them). The agent's own file
  tools can edit them. That is the bypass: the gating extension can be edited to stop asking.
- The **approval DECISION** already flows through the adapter/hub (pi: `extension_ui_request`
  -> `approval_need` -> the hub answers); what is editable is the **GATING** (which tool calls
  ask), which is an extension in the workspace.
- **dsh** places extensions into `$DSH_HOME` (hub data dir, not the workspace), which is
  already outside the workspace - the two pi-family adapters are the exposure.

**Feasible, because the adapters are ours.** Directions (a decision is needed):

1. **The enforcement point must be the adapter, not a workspace extension.** The adapter
   already sees every tool call; the gate that decides "ask the hub" belongs there, where the
   agent cannot edit it. A trust-bearing extension must not be the gate.
2. **Extensions the agent must not touch must live outside the workspace**, or be re-placed
   from the hub's copy before they are trusted (so an edit does not survive), or be
   OS-read-only to the agent. For pi/jouzu this needs the harness to load an extension from
   outside `<cwd>/.pi/extensions`, or a re-verify-and-replace before each turn.
3. **Skills are handed read-only**, or as content the harness cannot write back (which is what
   the `skills://` mechanism already models: read-only, versioned by sha256, no paths - but
   nothing consumes it today and the harnesses read a directory, not the protocol).



**(o6 analysis - from the harness sources, read directly.)**

The harnesses the plugins drive are npm packages; their real discovery rules were read from
the packed packages (pi `@earendil-works/pi-coding-agent`, jouzu `jouzu`, dsh `@deepseek-ai/dsh`):

- **pi** (`dist/cli/args.js`) has explicit-path flags:
  `--extension, -e <path>` (repeatable), `--no-extensions, -ne` (disable discovery; explicit
  `-e` still counts), `--skill <path>` (repeatable), `--no-skills, -ns`. So **pi can load
  extensions and skills from ANY absolute path with discovery off** - the workspace is a
  CHOICE the adapter makes, not a pi requirement.
- **jouzu** runs the same shape (`--extension <path>` / `--skill <path>` from its resolved
  paths).
- **dsh** already keeps extensions in `$DSH_HOME/profiles/...` (outside the workspace); its
  workspace root is the invoking directory and its skills come from `DSH_AGENTS_HOME/skills`.

**So the exposure is the hub's own choice, not a harness limit.** pi and jouzu adapters place
extensions into `<cwd>/.pi/extensions` and pass `--approve`, which puts the GATING extension
inside the agent's writable workspace. The fix is entirely adapter-side and requires no harness
change:

1. place extensions and skills in a **hub-owned directory outside the workspace**;
2. launch with **discovery off** and explicit paths:
   pi: `-ne -e <hub>/ext/<id> ...` and `--no-skills --skill <hub>/skills`;
   jouzu: the same; dsh: already outside;
3. the agent then cannot add or edit a trust-bearing extension (discovery off, paths outside),
   and skills are handed from a directory it cannot write;
4. the approval gate rests on the adapter (it already sees every tool call) plus an extension
   loaded from the hub-owned dir - never on a file in the workspace.

**(o5) anonymous discovery.** `/v1/harnesses` is today the only token-free route. Once it
becomes the management view (o4), does it stay token-free, or does it require the token like
everything else (discovery on loopback needs no anonymity)? A contract change either way.

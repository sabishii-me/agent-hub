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

## Summary of the proposals

- **RENAME**: drop the inconsistent `/hub/` prefix; one prefix `/v1/` (classes 2-7).
- **MERGE (1 real duplicate)**: `POST /v1/harnesses/{id}/enable|disable` == the `/hub/`
  twin; keep one.
- **MOVE**: `registry`, `catalog` -> the plugins class; `openapi.json` -> the status class;
  `providers/models/list` -> `/v1/models` (class 10).
- **REMOVE A ROUTE (internal, not user-facing)**: `plugins/orphans` - see class 3.
- **RENAME a verb out of a path**: drop `list` where it is a verb (now moot for models,
  which moved to `/v1/models`).
- **ADD DESCRIPTIONS** (6 routes have none): harness enable/disable, `GET /sessions/{id}`,
  `sessions/{id}/approvals`, `sessions/{id}/artifacts`, `sessions/{id}/cancel`,
  `/hub/shutdown`.
- **KEEP** everything else (the harness/hub pairs of models/auth/connections are two
  different owners, not duplicates).

## Open questions (need your call before I write the final route docs)

**(a) Action-shaped paths.** `registry/refresh` and `model-providers/{id}/models/refresh` are
verbs. ADR-0009 says a long action should be `202 + Location` on a resource, not a verb path.
Options: keep `POST /v1/<resource>/refresh` (clear, common), or model it as
`POST /v1/<resource>` (the resource refreshes itself). Your call.

**(b) The harness registry's path.** The hub's registry row and the harness's own answers
share `/v1/harnesses`. Proposal: keep the harness's own surface at `/v1/harnesses/{id}/...`
and put the registry at a distinct sub-path. Which name - `/v1/harnesses` for the registry
and `/v1/harnesses/{id}/owned` for the rest, or another split? Your call.

**(c) The `/v1/` prefix removal is a breaking contract change** (every consumer). Do it now
with the model change, or as its own change after? (It can be both: the rename is a contract
change that ships on the contract.)

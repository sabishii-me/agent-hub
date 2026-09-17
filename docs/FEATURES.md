# What the hub does — feature by feature

This file is the map: for each feature, what it promises, which routes and events
carry it, which harness capability gates it, which trial drives it, and what is
honestly **not** covered yet.

Three things it is not:

* **not the contract.** `contract/v1.json` is the authority for shapes, routes,
  statuses and error codes; `contract/openapi.json` is its generated projection.
  Where this file and the contract disagree, the contract wins and this file is
  wrong. The hub refuses to start if the two disagree, and prints the differences.
* **not a promise about a harness.** The hub manages harnesses; what a harness
  does with what it is given is the harness's business. Where the two halves
  differ, this file says which half is speaking (the hub's record, or the
  harness's own answer).
* **not a test report.** Trials print facts (exit 0 = ran to the end, 1 = something
  stopped it, 2 = refused to run, 124 = the runner's timeout). There is no PASS
  line anywhere and nothing here claims a green run means more than it says.

Everything the hub needs is inside this directory. The trials locate the hub
relative to their own file, so this whole directory can be moved to another
repository without touching a path.

## The shape of it

```
/v1/harnesses...           discovery + per-harness catalogues (models, presets, tools)
/v1/hub/...                the management surface: registry, providers, connections,
                           skills, status, surface, openapi.json, shutdown
/v1/sessions...            sessions, turns (SSE), approvals, questions, repair,
                           fork, compact, stats, close/reopen
```

An adapter process speaks a different protocol (JSON-RPC over stdio) and is not
HTTP: `contract/adapter-v1.json`. It is checked against every manifest, every
request the hub sends, every event an adapter emits, and every event the hub
handles.

## Discovery

| | |
|---|---|
| routes | `GET /v1/harnesses` (the ONE route that needs no token) |
| promises | which harnesses exist, whether each is enabled, what it declares (capabilities, runtime version, base) |
| verified | by hand through `/v1`: (prints the no-token status) |
| not covered | nothing gates a disabled harness here; the refusal happens on session create (`harness_disabled`) and is printed by the same trials |

## The management surface (`/v1/hub`)

| | |
|---|---|
| routes | `harnesses` (+`/enable`, `/disable`, `PATCH`), `providers` (+CRUD, `/models`, `/models/list`, `/models/refresh`, `/models` PATCH, `/logout`), `connections` (+CRUD), `skills` (+files GET/PUT/DELETE), `status`, `surface`, `openapi.json`, `shutdown` |
| promises | the hub manages harnesses: for each one it holds enabled + which extensions and skills are installed. Installing is the hub's job — it writes the selected extension directories into that harness's data dir before the adapter starts; the adapter only places what it finds where its harness reads it. |
| verified | by hand through `/v1`: (every declared route must answer, and prints the ones that do not), (manifests vs contract vs code) |
| not covered | the *semantics* of registry mutations (`PATCH /v1/hub/harnesses/{id}` extensions/enable/disable) and of provider `models/refresh` are reached but not driven end to end; `GET /v1/hub/status` content is not checked beyond its shape; `POST /v1/hub/shutdown` is driven once (the endpoint file must disappear) |

## Providers: one provider is one file

| | |
|---|---|
| routes | `GET/POST /v1/hub/providers`, `GET/PATCH/DELETE /v1/hub/providers/{id}`, `…/models/refresh`, `…/models` PATCH, `GET /v1/hub/providers/models/list`, `POST …/logout` |
| promises | one provider = one file under `<DATA_DIR>/providers/<id>.json`, self-describing (endpoint, api, catalog, revision) and hand-editable; import/export is a copy. The token is NOT in the file — it goes to the OS secret store (`secret-store.mjs`) and only its presence is reported (`tokenConfigured`). |
| declarations | `declarations` is keyed by model id and carries what a provider's model list cannot tell anyone: `input` modalities, `contextWindow`, `maxTokens`, `reasoning.efforts`, `cost`, `name`. A model is never advertised as taking images, or as offering a thinking level, unless it was declared. |
| availability | **one rule, every harness.** A model from a hub-managed provider is available iff the HUB has a credential for it (`hasCredential`, computed from the secret store) — the harness cannot answer that question, because a managed token reaches it only at session start. It used to be two answers from the wrong sides: the pi family hardcoded `available: true` for anything the hub sent (so a provider with no token looked usable) while dsh asked its own credential store (so every managed model read `needs-auth`, the screenshot bug). A harness's OWN routes keep the harness's verdict: pi reads its config, dsh its credential store — `deepseek-official` reads `needs-auth` in a managed session because the user's own dsh credentials are deliberately not copied into the isolated home. |
| stale routes | the hub's provider routes used to be written into the harness's own settings (the removed system scope wrote them into the user's `~/.dsh`); the one-time settings migration carried them along, and a route pointing at an env ref nobody sets shows up as a wall of `needs-auth` models the hub does not even manage (measured: 160 entries). The adapter that STARTS a dsh now sweeps every `prts-*` route out of its settings — only the starting process, so an attached adapter cannot delete a route a running session injected. |
| unknown fields | refused **by name** (`validation_failed: unknown field 'models'; accepts …`) — a field nobody reads is a request that did not happen, and it used to be ignored in silence (a declared `contextWindow` that never reached the harness). |
| verified | by hand through `/v1`: (a declared modality actually reaches the model) |
| not covered | refresh semantics (what happens to the selection when the catalog shrinks) and `PATCH /v1/hub/providers/{id}` revision conflicts are not driven |

## Sessions

| | |
|---|---|
| routes | `POST/GET /v1/sessions`, `GET/DELETE/PATCH /v1/sessions/{id}`, `POST …/close`, `…/reopen` |
| promises | a session names a harness + a directory + optionally a managed provider/model/preset/plan/review/thinking level/title/disabled tools. Every knob has an `applied*` counterpart holding what the HARNESS holds (never what was asked for). A knob the adapter could not confirm is reported as unconfirmed, not stored as if it took effect. |
| isolation | every session runs its harness in the hub's OWN data dir (`<DATA_DIR>/agents/<harness>/…`). There is no config scope: a harness home is never the user's. What is borrowed from the user is read-only (pi's agent-dir files are copied for credential injection; dsh's `settings.yaml` is migrated once, minus its permission section) and never written back. |
| refusals | `unknown_session` (never silently created), `session_busy` (a turn is running; refused, not queued), `session_closed`, `session_deleted`, `needs_repair`, `harness_disabled`, `requires_new_session` (harness changed), `agent_preset_locked` |
| verified | by hand through `/v1`: (two sessions do not share history), (the user's home is untouched: a before/after listing) |
| not covered | `title` push at create is driven; a title changed in the harness *while no turn runs* is only seen if the harness reports it (dsh does, on the first prompt) |

## Turns and the turn stream

| | |
|---|---|
| routes | `POST /v1/sessions/{id}/turns` (SSE), `POST …/cancel`, `GET …/turns`, `GET …/messages`, `POST …/repair` |
| events | `turn.admitted`, `turn.running`, `message.delta` (`kind: text\|reasoning`), `message.completed`, `tool.started`, `tool.ended`, `approval.requested`, `approval.resolved`, `question.requested`, `question.cancelled`, `plan.changed`, `session.compacting`, `session.compacted`, `session.renamed`, `turn.ended`, and `session.stats` as the last fact of a turn |
| promises | the native assistant message id is a MUST (a client must never have to guess which message a delta belongs to). `turn.ended` says `completed\|cancelled\|failed` plus a cause. The stream is closed by one memoised closer, so neither the adapter's event nor the prompt reply can cut it before the closing facts arrive. |
| verified | by hand through `/v1` |
| not covered | steering / queueing (`steer`, `follow_up`, `clear_queue`) is deliberately not exposed: a second turn during a turn is `session_busy` |

## Approvals

| | |
|---|---|
| routes | `GET /v1/sessions/{id}/approvals`, `POST /v1/sessions/{id}/approvals/{aid}` (`decision`, optional `reason`) |
| promises | an approval asks for PERMISSION and is answered; a deadline-denied dialog is auto-cancelled rather than left hanging; the hub reports what it answered and what the harness did with it. |
| verified | by hand through `/v1`: (allow and reject; the reason reaches the harness), (cancelling while an approval is pending) |
| not covered | no harness-specific approval *policy* is imposed by the hub beyond what `config/set` carries |

## Questions

| | |
|---|---|
| routes | `GET /v1/sessions/{id}/questions`, `POST /v1/sessions/{id}/questions/{qid}` |
| promises | a question is not an approval: it wants an ANSWER. Answers are returned per asked question, in order. |
| verified | by hand through `/v1`: (plan mode asks the model's exit question) |
| not covered | multi-question ordering beyond what a harness sends |

## Plan mode

| | |
|---|---|
| capability | `plan` (every plugin we run declares it) |
| routes / events | `POST/GET/PATCH /v1/sessions` carry `plan`/`appliedPlan`; `session` streams `plan.changed` |
| promises | the state is the HARNESS'S. The hub follows it: when the model leaves plan mode by having its plan approved, the harness changes state and the hub records what the harness holds, instead of assuming its own last request still stands. A plan-mode turn that should not touch the tree does not touch it. |
| verified | by hand through `/v1`: (created on/off, keep planning leaves the file unwritten, approve flips the state and the write lands, the state survives a hub restart, a PATCH during a turn is `409 session_busy`) |
| not covered | the `501 unsupported` gate: every installed harness declares the capability, so the refusal path has never run (it needs a stub plugin without it) |

## Review switch

| | |
|---|---|
| capability | `review` |
| routes | `PATCH /v1/sessions/{id}` `{review}`; `appliedReview` |
| promises | the switch changes whether the harness asks for approval before acting. It is this project's own plugin for the pi family, mounted by a dsh preset; the hub only drives it and records what came back. |
| verified | by hand through `/v1`: (an approval is raised when it is on and not when it is off) |
| not covered | a harness without the plugin answers `unsupported` rather than pretending |

## Fork

| | |
|---|---|
| capability | `fork` |
| routes | `POST /v1/sessions/{id}/fork` (`afterTurnId?`) |
| promises | a NEW session whose conversation ends at a COMPLETED TURN of this one. The anchor is a turn, never a raw harness id (pi entry ids and dsh seq numbers never leave the adapter). The source session is untouched: same log, same ref, still usable, and it does not need a live process. The CHILD's own process performs the fork — pi's fork rebinds whichever process runs it. |
| errors | `unsupported`, `unknown_turn`, `fork_failed` |
| verified | by hand through `/v1`: (both harness shapes, the whole-conversation case, and the fact that the capability gate is not exercised) |
| not covered | `get_tree` is not exposed: one branch = one session, and only the pi family has a tree at all |

## Statistics

| | |
|---|---|
| capability | `stats` |
| routes / events | `GET /v1/sessions/{id}/stats`; `session.stats` on the turn stream right after `turn.ended` |
| promises | read-through: the hub asks the harness and caches nothing. A field the harness does not report is ABSENT — a zero and an unmeasured value are different claims. There is no separate stats stream: dsh pushes projection updates, the pi family pushes nothing, and a hub-made "live" stream there would be polling in disguise. |
| verified | by hand through `/v1` |
| not covered | `cost` is absent on dsh (it has no cost field), and the window each harness reports is its own (dsh publishes its route capacity, pi uses the declared one) — printed rather than smoothed |

## Compaction

| | |
|---|---|
| capability | `compact` |
| routes / events | `POST /v1/sessions/{id}/compact`; `session.compacting` then `session.compacted` on the turn stream |
| promises | the harness rewrites its own conversation; the hub compacts nothing and computes no numbers. A harness with nothing worth compacting answers `compacted:false` with its own words; a compaction that FAILED answers `compact_failed`. Both harnesses report the compaction they performed ON THEIR OWN (pi: `threshold`/`overflow`; dsh: `auto`), which is the half a caller cannot get from the route's reply. Refused with `409 session_busy` during a turn (pi's compact aborts the running turn). |
| verified | by hand through `/v1`: (manual + harness-initiated, on both harnesses, with the context occupancy before/after and a follow-up turn proving the session still works) |
| not covered | what a harness keeps versus summarises is its own policy, and the hub does not second-guess it |

## Rename

| | |
|---|---|
| capability | `rename` |
| routes / events | `PATCH /v1/sessions/{id}` `{title}`, `title` and `appliedTitle` on the session; `session.renamed` on the turn stream |
| promises | the name reaches the HARNESS, not just the hub (a harness shows it in its own listings). Both sides are reported: `title` is what was asked, `appliedTitle` is what the harness accepted — they differ (dsh collapses whitespace and flattens newlines; pi flattens a newline). A name the harness gives itself (dsh titles a session from the first prompt) is reported back with `appliedBy: harness`. |
| verified | by hand through `/v1`: (every harness installed at the time — pi, jouzu, deepseek; the witness is the harness's own storage after a turn, plus the read-back from its own state; refusals for an empty title, an unknown field, and a mid-turn PATCH) |
| not covered | a rename performed in a harness UI while no turn runs is only visible if the harness reports it |

## Materialization: what a session carries into its harness

| | |
|---|---|
| promises | the hub assembles a session's material from its own registries — installed extensions, installed skills (a directory the adapter is pointed at, not a payload), enabled connections — and hands it over whole; the adapter translates it into its native dialect. `config/set` carries the connections block only: skills travel as files (see the skills row below), so there is one source of truth for what is installed. Credential VALUES never appear there: a connection's token reaches the adapter as an env var named by the materialization (`envName`), so it cannot land in a config payload or in conversation history. |
| verified | by hand through `/v1`: (the provider is injected and the turn runs through it) |
| skills | the hub holds them (`/v1/hub/skills`, byte-level round trip, one directory per skill), selects them per harness (`PATCH /v1/hub/harnesses/{id}` `{skills}`) and installs the selected ones into `<DATA_DIR>/agents/<harness>/skills` before that harness's process starts — the same install step extensions use. The adapter only points its harness at that directory: pi/jouzu `--no-skills --skill <dir>`, dsh `DSH_AGENTS_HOME` (whose `skills/` child dsh reads, so the adapter passes the parent). **Nothing is read from or written to the user's own skill directories** (`~/.agents/skills`), which a hand-run confirmed with a marker skill that never appeared. |
| verified | by hand on every harness installed at the time (pi, jouzu, deepseek), no model call needed: a well-formed skill and a `Bad_Name` skill were PUT into the hub; the hub installed both; the harness's own catalogue (read through `GET /v1/sessions/{id}/skills`) reported pi `["alpha-skill","Bad_Name"]`, jouzu `["multiloop","alpha-skill","Bad_Name"]` (multiloop is jouzu's own), dsh `["alpha-skill"]` — dsh silently dropped the non-kebab name, and the hub reported exactly that difference instead of the installed list. |
| not covered | a harness that is slow to warm up: jouzu's first catalogue answer measured 9s once and 70s once (then 24ms), so the hub bounds the read-back at 60s and answers `known:false` with a reason rather than an empty catalogue. A skill a harness refuses is invisible until this read-back is asked for; the hub does not parse skill content to predict it. |
| not covered | `tools/list` is declared in the adapter contract and gated by a `tools` capability no plugin declares, so `GET /v1/harnesses/{id}/tools` answers `known:false` for every harness — an empty catalogue is never faked |
| not covered | artifacts have no producer: `GET /v1/sessions/{id}/artifacts` answers `[]` honestly |

## Errors

| | |
|---|---|
| source | `contract/errors.json` — 42 codes, each with a status and whether retrying unchanged can work |
| promises | the table decides the status (`failError` reads it) and the body carries `retryable` next to the code, so body, status and table cannot disagree. An unknown field in a request body is refused by name. |
| verified | by hand through `/v1`: (prints codes the hub can answer with that the table does not declare, and status literals that disagree with it) |

## The contract, its projection, and the boot self-check

| | |
|---|---|
| files | `contract/v1.json` (authority), `contract/errors.json` (error master table), `contract/adapter-v1.json` (stdio protocol + capability surface), `contract/openapi.json` (generated) |
| routes | `GET /v1/hub/surface` (routes, events, contract sha256, openapi sha256), `GET /v1/hub/openapi.json` (the generated document) |
| promises | the hub refuses to start if it cannot read its contract or the projection. `scripts/emit-openapi.mjs` generates the projection deterministically and refuses to skip anything it cannot express. |
| verified | by hand through `/v1`: (differences in both directions, `$ref` resolution, freshness by byte comparison, the served bytes equal the file), (manifest fields/protocol/runtime, capability ↔ methods, requests sent ↔ declared, events emitted/handled ↔ declared) |
| falsified | the boot check has been shown to refuse startup: renaming a route in the hub's table (`the hub answers GET /v1/sessions/{id}/statz, which the contract does not declare`), renaming a method in a plugin adapter (`pi: declares 'compact' but its adapter does not handle 'session/compact'`), and editing the contract without regenerating the projection. Both mutations were restored byte-for-byte. |

## How to drive it

```
PRTS_DATA_DIR=<dir> node server.mjs                  # prints its port, writes endpoint.json
node scripts/hub-connect.mjs --data-dir <dir>        # prints the url + token for a client
node scripts/emit-openapi.mjs                        # after editing contract/v1.json
```

The hub itself: `PRTS_DATA_DIR=<dir> node server.mjs` — it prints the loopback
port and writes `<dir>/endpoint.json` (port + token). There is no build step; the
harness runtimes are materialised by `scripts/prepare-runtimes.mjs` from each
plugin's manifest and are not committed.

## Boundaries

* **Not a security boundary.** The hub is a front end, not a security boundary. A harness's own approvals and
  sandboxing are the harness's; the hub relays them and refuses to *pretend* they
  happened.
* **Not the harness's UI.** Only the headless harness protocol is wrapped. The
  hub never drives a TUI, and a managed spawn never opens a window.
* **No silent downgrade.** A change that did not happen is an error. A field a
  harness cannot carry is left out. A number a harness did not report is absent.
  A capability a plugin did not declare answers `unsupported`.

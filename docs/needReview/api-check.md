# Pass 2 — the full /v1 API check (all 74 endpoints)

Source of truth: `contract/v1.json` (74 endpoints). Mounted routes read from each domain's
`crates/*/src/routes.rs`. Status values: **real** (a real handler does the work), **gated**
(mounted, real, but answers `unsupported`/501 when a precondition is absent), **stale-doc**
(the code is ahead of what a doc claims). Every row below is from the code this pass read; it is
NOT yet runtime-verified — that is Pass 2's acceptance step.

## Counts
- contract endpoints: **74**
- mounted: **74** (the two `skills/{id}/files/{file...}` routes ARE mounted via `get_as`/`put_as`
  with the contract's `{file...}` spelling — the "not mounted 2" claim in `ARCHITECTURE.md` §24 is
  **STALE**).
- handlers that can answer `501 unsupported` (capability-gated, not stubs):
  - `GET /v1/sessions/{id}/resources`, `POST /v1/sessions/{id}/resources/read` — gated on a wired
    skills/resource resolver (`crates/sessions/src/routes.rs`); no resolver -> `unsupported`.
  - `POST /v1/model-providers/{id}/auth` (and its status/cancel) — gated: refuses BEFORE any side
    effect when the provider names no type, no plugin ships the type, or the type declares no
    interactive method the hub implements (`crates/providers/src/routes.rs`).

## By group (endpoint => mounted => handler status => note/contradiction)

### meta (5)
| endpoint | mounted | status | note |
|---|---|---|---|
| GET /v1/surface | yes | real | mounted routes + contract sha256 + event names |
| GET /v1/openapi.json | yes | real | byte-for-byte the generated projection |
| GET /v1/status | yes | real | |
| POST /v1/shutdown | yes | real | |
| GET /v1/events | yes | real | SSE |

### harnesses (14)
GET /v1/harnesses, /{id}/presets, /models, /tools, /extensions, /connections/schema,
/connections, POST /{id}/connections, /connections/validate, DELETE /{id}/connections/{cid},
POST /{id}/auth, GET /{id}/auth/{op}, POST /{id}/auth/{op}/cancel, PATCH /{id}/extensions.
All mounted + forwarded; capability-gated by the manifest's `capabilities`. A harness that does
not ship a capability answers `unsupported`, never a fabricated answer.
CONTRADICTION to confirm: docs say the pi adapter answers no `tools/list` (`known:false`); the
route is mounted and forwards — is that "real" or "gated for pi only"?

### sessions (23)
create/list/get/patch/delete, turns (GET/POST), cancel, repair, approvals (GET/POST),
questions (GET/POST), artifacts, skills, stats, fork, close, reopen, compact, resources,
resources/read, messages.
All mounted; `resources`/`resources_read` gated (see above). Everything else real per §18.
CONTRADICTION to confirm: three status tables disagree (ARCHITECTURE §18/§24, system-alignment,
CURRENT-STATE) on what is real.

### providers (13)
list/create/get/patch/delete, logout, models (GET/PATCH), models/refresh, types, auth (POST),
auth/{op} (GET), auth/{op}/cancel (POST). All mounted; auth is gated (see above); `types`
scans installed plugins for a `provider.json` (absent descriptor => empty list).

### plugins (10)
list/post/get/delete/prepare/enable/disable, catalog, registry/refresh, icon.
All mounted and real per §18.
CONTRADICTION to confirm: `tasks/plugin-lifecycle.md` says the catalog/refresh/icon are "not
built"; §18 says they are real. (`plugin-lifecycle.md` is STALE.)

### connections (4) / models (1) / skills (4)
connections CRUD — mounted, real. `GET /v1/models` — mounted, real.
skills: list/delete/files(GET/PUT) — mounted, real (directory model). NOTE: the skills model is
the one flagged for replacement by the plugin-sourced model (`remaining-surface.md`); this is a
design-owner question, not a code defect.

## What Pass 2 must still do (runtime acceptance, not done here)
- For each mounted route: drive it through `/v1` and record the status/shape vs the contract.
- Record every place the contract's request/response disagrees with the handler.
- Confirm the `gated` routes answer `unsupported` with a reason and NO side effect.

## Contradictions this pass already found (record, then resolve by owner decision)
1. `ARCHITECTURE.md` §24 "Not mounted - 2" is FALSE; both are mounted.
2. `tasks/plugin-lifecycle.md` "still not built" (catalog/refresh/icon) is FALSE.
3. Three status tables (§18, system-alignment, CURRENT-STATE) disagree.
4. The direct-address policy text disagrees with the settled `approve-unsafe` direction.

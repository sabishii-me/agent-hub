# What remains, and why it is not hub-ownable yet

Verified at `3426411` against `contract/v1.json` (74 endpoints): 56 real, 8 mounted
but `501`, 10 not mounted. Every remaining route is blocked on work OUTSIDE the hub's
own code, not on a hub omission. This file records the reason per group, so the next
session does not re-derive it.

## Adapter protocol (a separate project)

The pi adapter implements 15 methods; `contract/adapter-v1.json` declares 26. The hub
can only forward what the adapter answers. Missing adapter-side:

- `connections/{schema,list,validate,save,delete}` -> the harness-private connection
  routes (`/v1/harnesses/{id}/connections*`, 5 routes).
- `auth/{start,status,cancel}` -> the harness auth routes (`/v1/harnesses/{id}/auth*`,
  3 routes).
- `tools/list` -> `GET /v1/harnesses/{id}/tools` is mounted but the adapter cannot
  answer it (it stays `501`).
- `approval_need` / `question_need` -> the humans domain routes are mounted and
  forward, but a real adapter must push them.
- `session/repair` -> `POST /v1/sessions/{id}/repair` (a cancelled turn whose end was
  never confirmed).
- `resources/list` / `resources/read` -> `GET /v1/sessions/{id}/resources` and
  `POST .../resources/read`.

## The provider-type data model

`GET /v1/model-providers/types` and the provider `auth/*` routes
(`POST /v1/model-providers/{id}/auth`, `GET .../auth/{op}`, `POST .../auth/{op}/cancel`)
read a **type descriptor a model-provider plugin ships** (id, version, owner, name,
authMethods, configuration, catalog dialect). No such plugin/descriptor exists yet, so
there is nothing to serve and nothing to run an auth flow from. A type no plugin ships
stays absent by the contract's own wording.

## The plugin-sourced skills model

`GET`/`PUT /v1/skills/{id}/files/{file...}` stay `501` by a **decided** correction
(TASK-048 C2): the earlier hub-authored skills store was the excluded model. The
decided direction is a plugin-sourced, layered delivery with a `skills://` `node:fs`
hook (ARCHITECTURE §7), which is a verification-gated unknown (T2b). Re-enabling the
routes over the old model would revert an approved correction.

## Artifact recording

`GET /v1/sessions/{id}/artifacts` lists the release artifacts a session's harness
depends on. The hub does not yet record an install's artifact (the `artifact` field on
a plugin row is always null); recording it is plugin-domain work that must land first.

## Not blocked (done since the last audit)

sessions create/turn/cancel/compact/fork/patch/close/reopen + the read-through views
(messages/stats/skills); the provider grant chain, presets, plan/review, model
selection; the connections domain; plugin enable/disable, catalog, registry refresh,
icon; harness extension selection; the metadata routes (surface/openapi/shutdown).

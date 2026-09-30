# What remains, and why it is not hub-ownable yet

Verified at the plugin-install-sources + repair commit against `contract/v1.json`
(74 endpoints): 58 real, 6 mounted but `501`, 10 not mounted. Every remaining route is blocked on work OUTSIDE the hub's
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

## Artifact recording (DONE)

`POST /v1/plugins` now supports BOTH declared sources: a git clone (`{url, ref?}`)
and a release artifact (`{artifact:{url,sha256,id,pluginType,version,size?}}`),
verified size-first then sha256 before unpacking (the mature `zip` crate, ADR-0010).
An artifact install is RECORDED on the plugin row and reported by
`GET /v1/sessions/{id}/artifacts` (the session's harness plugin's artifact). See
`docs/tasks/plugin-install-sources.md`.

## Not blocked (done since the last audit)

sessions create (including additionalDirectories, now passed to the adapter as
AGENT_HUB_ADDITIONAL_DIRS)/turn/cancel/compact/fork/patch/close/reopen + the read-through
views (messages/stats/skills/artifacts); the provider grant chain, presets, plan/review,
model selection; the connections domain; plugin install from a git ref OR a release
artifact (verified + recorded), enable/disable, catalog, registry refresh, icon; harness
extension selection; session repair (hub-side re-abort + process replace + re-attach);
the metadata routes (surface/openapi/shutdown).

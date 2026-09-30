# Remaining surface, classified (per A1: NOT "all hub-ownable done")

Each remaining route is classified as one of:
- **HUB-TODO** — a hub-side implementation still owed (do not call it an external blocker).
- **EXT-DEP** — blocked on a concrete artifact/work in another project/plugin.
- **UNAUTH-CALL** — needs a real credential / paid vendor call that is not authorized.

A missing plugin or credential never turns a HUB-TODO into an EXT-DEP: the hub must still
implement its own consumer/executor and simply have no input yet.

## Provider-type: HUB-TODO (+ EXT-DEP for input)

`GET /v1/model-providers/types` is now REAL: `ProvidersState` scans installed
`model-provider` plugins for a `provider.json` descriptor and returns `{types, broken}`
(an invalid descriptor is `broken[]`, an absent one stays absent). The provider
`auth/*` routes remain HUB-TODO: the **auth executor** (device-code/browser flows as
DATA, run in the hub, outliving the request), the operation store and cancel.

**EXT-DEP**: no model-provider plugin ships a descriptor yet, so `types` would be an
empty list and no auth flow can start. That does not remove the HUB-TODO.

## New skills model (approved; C2 kept): HUB-TODO + verification (T2b)

C2 excluded the OLD hub-authored skills store. The APPROVED new model is a
plugin-sourced, layered delivery with a `skills://` `node:fs` hook (ARCHITECTURE §7) and
is a **verification-gated unknown (VERIFICATION-TASKS T2b)**. The hub-side pieces
(`GET /v1/skills`, `DELETE /v1/skills/{id}`, `GET/PUT /v1/skills/{id}/files/{file...}`,
`GET /v1/sessions/{id}/resources`, `POST .../resources/read`) stay `501` until that model
lands. This is HUB work, not restored legacy.

## Harness connections / auth: EXT-DEP (adapter protocol)

`/v1/harnesses/{id}/connections*` (5 routes) and `/v1/harnesses/{id}/auth*` (3 routes)
forward to the adapter's `connections/*` and `auth/*`, which the pi adapter does not
implement. HUB-TODO portion: mount and forward these routes (returning a typed
`unsupported`/absent answer) rather than leaving them unmounted. EXT-DEP: the adapter
methods that make them return real data.

## Small hub-owned gaps: HUB-TODO

- `GET /v1/harnesses/{id}/tools` forwards `tools/list`; the pi adapter does not answer it
  (`known:false`). HUB-TODO: none beyond forwarding (already capability-gated).
- Artifact recording: DONE (`POST /v1/plugins` artifact source records it;
  `GET /v1/sessions/{id}/artifacts` reports it).

## Delivered on the current chain (verified)

sessions create/turn/cancel/compact/fork/patch/close/reopen/repair + read-through views
(messages/stats/skills/artifacts); provider grant chain, presets (**switchable without
wedging, restart+resume**), plan/review, model selection with a full pending barrier;
connections domain; plugin install (git or verified artifact), enable/disable, catalog,
registry refresh, icon; harness extension selection (**complete immutable snapshot per
start**); metadata routes (surface/openapi/shutdown).

`contract/v1.json` = 74 endpoints; the mounted-vs-contract split is in
`docs/ARCHITECTURE.md` §24.

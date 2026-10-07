# 20261007-200000 - `prepare` was split out of install, so an install is a fake install

**State:** OPEN (fixing)  
**Severity:** T0 - removing the runtime into a separate command made `ready` false and made an
install need TWO commands to become usable.

## The owner's rule (authoritative)

- An adapter that does not prepare its runtime **is not usable**. `ready` after landing only the
  adapter tree is a lie.
- `POST /v1/plugins` MUST install completely - adapter + runtime - in ONE command. There is no
  "install twice".
- `POST /v1/plugins/{id}/prepare` is REMOVED: it maintained a second local state that drifts.

## What the contract ALREADY says

`contract/openapi.json` POST /v1/plugins: "...The response says `updated`, and the plugin's runtime
**is prepared by the adapter in the background**." So install was ALWAYS meant to prepare the
runtime. The separate `/v1/plugins/{id}/prepare` endpoint (added later) contradicts it and was the
mistake.

## Measured now (dev)

```
POST /v1/plugins {artifact pi}  -> 202
GET  /v1/plugins/pi             -> ready (0.1s), runtimeReady:false, runtime.target:null
   the plugin dir holds the ADAPTER only; no runtime/dist/cli.js
```

The runtime is produced ONLY by `POST /v1/plugins/{id}/prepare` (npm install, tens of seconds).

## The change (contract review required)

1. REMOVE the `/v1/plugins/{id}/prepare` path from `contract/openapi.json`.
2. POST /v1/plugins keeps its description (it already promises the background prepare) and its
   state now runs `installing` -> `preparing` -> `ready` (or `failed`).
3. The implementation prepares the runtime in the install's detached task and verifies the declared
   command exists before announcing `ready`.

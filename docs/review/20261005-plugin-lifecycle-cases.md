# Plugin install/remove — the case matrix (every case, including error expectations)

Recorded 2026-10-05. Purpose: enumerate EVERY case before writing a test - success AND failure,
because an error's code and message are as much the contract as a success. Sources: the contract
(`v1.json`, `errors.json`), ARCHITECTURE, and the OLD adversarial set (archived:
`tests/archive/old-node-hub-suite/` — `interruption/removal.mjs`,
`runtime/a-provider-is-usable-as-soon-as-it-installs.mjs`,
`runtime/a-runtime-starts-and-imports-its-own-code.mjs`,
`runtime/an-update-replaces-a-stale-runtime.mjs`).

Legend: **[C]** contract-pinned (code/message fixed), **[?]** the contract does NOT pin it → must be
decided by the owner, not invented. Status: `tested` / `TODO`.

## A. Install — success

| # | case | expected | src | status |
|---|---|---|---|---|
| A1 | empty root, install a git source (local path) | 202 -> `ready`, landed under `<DATA_DIR>/plugins/<id>` | v1 POST /v1/plugins | tested (whole-chain) |
| A2 | install an artifact (zip) | size verified, then sha256, THEN unpack; 202 -> ready | v1 POST | TODO |
| A3 | install from a git URL (not a local path) | clone -> ready | v1 POST | TODO |
| A4 | re-install the SAME id already installed by the hub | REPLACED (update); response `updated:true`; old moved aside, removed after new is in | v1 POST ("An id the hub already installed is REPLACED") | TODO |
| A5 | install a model-provider plugin; its TYPE is usable immediately | `/v1/model-providers/types` offers it at once, no restart | old `a-provider-is-usable...` | TODO (hard - 050000 was the harness twin) |
| A6 | install a harness; `/v1/harnesses` lists it at once | listed at once, no restart | 050000 fixed | tested (whole-chain) |

## B. Install — failure / refusal (error code AND message)

| # | case | expected | src | status |
|---|---|---|---|---|
| B1 | id lives in a directory the hub does NOT own (deployment) | **409**, naming the directory | v1 POST ("refused with 409") | TODO |
| B2 | a harness with OPEN SESSIONS exists for that id | **409 `plugin_in_use`**, naming the sessions | v1 POST | TODO |
| B3 | concurrent install of the SAME id | one 202, the rest **409** conflict | owner's rule (20261005-070000) | tested (6/6) |
| B4 | same `Idempotency-Key`, same body (a retry) | original result, NO second install | v1 idempotency | TODO |
| B5 | same `Idempotency-Key`, different body | **409 `idempotency_conflict`** | errors.json | TODO |
| B6 | source names neither url nor artifact | **400 validation_failed** | v1 POST ("must name a git url or an artifact") | TODO |
| B7 | git source whose dir has NO manifest.json | refused, error names it; **no half tree left** | v1 POST | TODO |
| B8 | artifact sha256 mismatch | refused BEFORE unpacking; error names it | v1 POST ("verifies ... before anything is unpacked") | TODO |
| B9 | manifest needs a newer hub (min_hub_version) | refused (ADR-0008), naming the reason | ADR-0008 | TODO |
| B10 | install fails MIDWAY after an old copy existed | the OLD plugin is left EXACTLY as it was | v1 POST | TODO |
| B11 | install fails mid-way (no old copy) | state `failed` (not stuck `installing`), detail set | v1 "state ... failed" | TODO |

## C. Remove — success

| # | case | expected | src | status |
|---|---|---|---|---|
| C1 | remove a hub-installed plugin, no sessions | 202 -> absent (404) | v1 DELETE | tested |
| C2 | remove a PREPARED plugin (runtime materialised) | the process is stopped first; really removed | 20261005-060000 | tested (16/16) |
| C3 | after remove, the plugin RECORD and the directory are gone | GET 404; list drops it; harness reads missing | v1 DELETE | tested |
| C4 | after remove, the harness row KEEPS its config; sessions are NOT deleted | sessions still readable | v1 DELETE ("Sessions are never deleted") | TODO |

## D. Remove — failure / refusal

| # | case | expected | src | status |
|---|---|---|---|---|
| D1 | remove a DEPLOYMENT directory | **409**, naming the directory | v1 DELETE | tested (409) |
| D2 | remove while OPEN SESSIONS use it | **409 `plugin_in_use`**, naming the sessions | v1 DELETE | TODO |
| D3 | remove an id that does not exist | **[?]** 404 vs idempotent 204 | NOT pinned | **decide** |
| D4 | remove a plugin whose delete FAILS (files held) | terminal `failed` (not stuck `removing`) | 060000 fixed | tested |
| D5 | concurrent remove of the same id | **[?]** one 202, the rest? | NOT pinned | **decide** |

## E. Mixed / ordering / restart / events

| # | case | expected | src | status |
|---|---|---|---|---|
| E1 | install A, install B, remove A, remove B (interleaved) | each plugin's state is independent and correct; no cross-talk in the state | ? | TODO |
| E2 | events: `hub.plugins.changed {id,state}` per change, in order, with ids | id matches the plugin; installing before ready | v1 /v1/events | tested (basic) |
| E3 | MIXED A/B: does A's change ever carry B's id? | never | ? | TODO |
| E4 | SSE DISCONNECT during changes, reconnect with Last-Event-ID | Replay the missed frames, or Resync; never silent loss | v1 /v1/events | tested (basic) |
| E5 | hub KILLED mid-remove | on restart it FINISHES the removal (no half state) | old `interruption/removal.mjs` | TODO |
| E6 | hub KILLED mid-install (`installing` on disk) | on restart the state is honest+consistent | old adversarial intent | TODO |
| E7 | restart: state agrees with disk (`/v1/plugins`, `/v1/harnesses`) | no lie after restart | ? | TODO |
| E8 | a failed INSTALL after an old copy: restart keeps the old, usable | old still works | v1 POST | TODO |

## F. Runtime (the deeper adversarial ones from the old set)

| # | case | expected | src | status |
|---|---|---|---|---|
| F1 | a plugin installs, settles `ready`, but its runtime is missing a package it imports -> a REAL process start | the hub SURFACES the failure; `ready` is not a lie | old `a-runtime-starts-and-imports-its-own-code.mjs` | TODO |
| F2 | install the CURRENT release over a STALE runtime | REPLACES the runtime, does not carry it forward | old `an-update-replaces-a-stale-runtime.mjs` | TODO |

## The 4 cases the contract does NOT pin (owner decides)

D3 (remove a missing id), D5 (concurrent remove), E1/E3 (cross-talk semantics are implied by
"each plugin's state" but not spelled out), E6 (restart mid-install's exact state).

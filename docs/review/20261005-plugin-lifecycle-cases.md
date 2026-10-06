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

---

# The TEST LIST (what each test asserts) — no code, just the list

Each row is one test file's job: the SETUP, the ACTION, and the EXACT assertions (code + what the
message must say). Error rows are as important as success rows. `[?]` = needs the owner's decision
before it can be written.

## T-A: install success

- **T-A1 install-from-a-local-git-path** — setup: empty plugins root. action: `POST /v1/plugins
  {source:{url:<local pi path>, ref}}`. assert: 202; body `pluginId=="pi"`; GET `/v1/plugins/pi`
  reaches `state=="ready"`; the dir exists under `<DATA_DIR>/plugins/pi` with `manifest.json`.
- **T-A2 install-from-a-git-URL** — same but `url` is a real remote git URL. assert: 202 -> ready.
- **T-A3 install-an-artifact** — setup: a real release zip + its sha256. action: `{source:{artifact:
  {url,sha256,id,pluginType,version,size}}}`. assert: 202 -> ready.
- **T-A4 reinstall-same-id-is-an-update** — setup: `pi` installed+ready. action: install `pi` again.
  assert: response `updated==true`; still exactly ONE `pi`; ends `ready`; a broken NEW copy leaves
  the OLD one working (see T-B10).
- **T-A5 an-installed-provider-is-usable-at-once** (old `a-provider-is-usable...`) — setup: no
  provider installed. assert: `/v1/model-providers/types` has NO custom-compatible; install the
  `compatible` provider; assert the type `custom-compatible` is offered IMMEDIATELY, no restart.
- **T-A6 an-installed-harness-is-usable-at-once** (20261005-050000) — assert: `/v1/harnesses` lists
  `pi` at once after install; a session opens. [done: whole-chain]

## T-B: install refusal / failure (code + message)

- **T-B1 deployment-dir-id-is-409** — setup: put `pi` under `AGENT_HUB_PLUGINS_DIR` (deployment).
  action: install `pi`. assert: 409; message NAMES the directory.
- **T-B2 install-with-open-sessions-is-409-plugin_in_use** — setup: `pi` installed, a session open.
  action: install `pi` again. assert: 409; error `plugin_in_use`; message NAMES the session id(s).
- **T-B3 concurrent-install-same-id** (20261005-070000) — setup: none. action: N parallel installs
  of `pi`. assert: exactly one 202, rest 409; ends ONE `ready`. [done]
- **T-B4 idempotent-retry** — setup: none. action: two `POST /v1/plugins` with the SAME
  `Idempotency-Key` and SAME body. assert: same result; NO second install (one dir, one op).
- **T-B5 idempotency-conflict** — action: same key, DIFFERENT body. assert: 409
  `idempotency_conflict`.
- **T-B6 source-missing** — action: `POST /v1/plugins {}`. assert: 400 `validation_failed`; message
  says the source must name a git url or an artifact.
- **T-B7 no-manifest** — action: install a git path whose dir has no `manifest.json`. assert: 4xx
  naming the missing manifest; NO half tree left in `<DATA_DIR>/plugins`.
- **T-B8 bad-sha256** — action: artifact with a WRONG sha256. assert: refused; message names the
  sha256 mismatch; nothing unpacked.
- **T-B9 needs-newer-hub** — action: install a plugin whose `minHubVersion` is too high. assert:
  refused (ADR-0008), message names the version requirement.
- **T-B10 failed-update-keeps-old** — setup: `pi` installed+ready. action: re-install `pi` from a
  BROKEN source (e.g. no manifest). assert: the OLD `pi` is still installed and `ready`; no
  partial overwrite.
- **T-B11 failed-install-is-not-stuck** — action: install something that fails midway. assert:
  final `state=="failed"` with a `detail`, NEVER stuck `installing`.

## T-C: remove success

- **T-C1 remove-hub-installed** — assert: 202 -> GET 404; gone from `/v1/plugins`. [done]
- **T-C2 remove-prepared** (20261005-060000) — setup: prepare `pi` (runtime materialised). action:
  remove. assert: really removed (404); the adapter process is gone. [done]
- **T-C3 remove-drops-record-and-dir** — assert: dir under `<DATA_DIR>/plugins/pi` gone; the plugin
  record gone; the harness reads `missing`. [partly done]
- **T-C4 remove-keeps-sessions-and-harness-row** — setup: `pi` installed, a session created then
  CLOSED. action: remove `pi`. assert: the (closed) session is STILL readable; its row kept its
  config; only the plugin/harness is missing.

## T-D: remove refusal / failure

- **T-D1 remove-deployment-is-409** — assert: 409, message names the directory. [done: code only]
- **T-D2 remove-with-open-sessions-is-409-plugin_in_use** — setup: `pi` installed, session open.
  action: remove. assert: 409 `plugin_in_use`; message names the session(s).
- **T-D3 remove-a-missing-id** — action: `DELETE /v1/plugins/nope`. assert: **[?] 404 vs 204.**
- **T-D4 failed-remove-is-terminal** (20261005-060000) — assert: a delete that fails lands
  `failed`, never stuck `removing`. [done]
- **T-D5 concurrent-remove-same-id** — action: N parallel removes. assert: **[?] one 202, rest?**

## T-E: mixed / order / restart / events

- **T-E1 interleaved-A-B** — action: install A, install B, remove A, remove B (and a mixed order).
  assert: each plugin's `state` at every step is its OWN; no cross-talk.
- **T-E2 events-per-change** — assert: `hub.plugins.changed {id,state}` for each change, in order,
  each with an id; `installing` before `ready`. [basic done]
- **T-E3 no-cross-id-in-events** — action: interleave A and B. assert: no A frame carries B's id
  and vice versa.
- **T-E4 sse-reconnect-replays** — action: disconnect during changes, reconnect with
  `Last-Event-ID`. assert: replay/resync, no silent loss, final state matches the resource. [basic
  done]
- **T-E5 killed-mid-remove-finishes** (old `interruption/removal.mjs`) — action: start a remove,
  KILL the hub. restart. assert: the removal is FINISHED (gone), not half.
- **T-E6 killed-mid-install-is-honest** — action: start an install, KILL the hub while `installing`.
  restart. assert: the state is honest and consistent with disk (no `installing` forever, no lie).
- **T-E7 restart-state-agrees-with-disk** — action: install, restart. assert: `/v1/plugins` and
  `/v1/harnesses` agree with the filesystem; no phantom, no missing.

## T-F: runtime (deep adversarial, from the old set)

- **T-F1 ready-but-runtime-dies** (old `a-runtime-starts-and-imports-its-own-code.mjs`) — setup: a
  plugin whose runtime imports a package the artifact omits. action: install, then a REAL start.
  assert: the hub SURFACES the failure; `ready` is not a lie (a real process start proves it).
- **T-F2 update-replaces-stale-runtime** (old `an-update-replaces-a-stale-runtime.mjs`) — setup: a
  stale runtime on disk. action: install the CURRENT release. assert: the runtime is REPLACED, not
  carried forward.

## Blocked rows (owner decision)

D3, D5, E1/E3 (cross-talk strictness), E6 (exact restart-mid-install state).

# Review: plugin REMOVE must release what the plugin started (a service lifecycle gap)

Status: **PROPOSAL — needs owner review before any contract/code edit.**
Recorded 2026-10-09. Extends `docs/needReview/review/20261005-plugin-lifecycle-cases.md` (C/D) and
the ARCHITECTURE lifecycle table (operation `install / remove (plugin)`).

## The gap

The design models TWO process lifetimes:

1. the hub process;
2. the session's adapter process — owned by the hub, stopped by `close`
   (ARCHITECTURE 602-606, `crates/sessions/src/runtime.rs`).

It does NOT model a third:

3. a harness SERVICE the adapter starts at run time — e.g. `dsh web`. The dsh adapter spawns one
   server per `DSH_HOME`, DETACHED, so every session of that home shares it. It therefore OUTLIVES
   every adapter.

Remove is a plugin lifecycle op: `removing` -> `finish_remove` deletes the directory
(`crates/plugins/src/service.rs`). Nothing reaches lifetime (3): the service is not a hub process,
and the hub is contractually forbidden to know a harness's internals (ARCHITECTURE 15/500: only
`adapter/` spawns processes). Today `crates/plugins/src/routes.rs` does `stop_harness` (stops a
CAPABILITY adapter, not the service) + `sleep(300ms)` + `remove_dir_all`. On Windows the live service
holds `runtime/**`, so the delete returns `io: Access is denied. (os error 5)` and the remove ends
`failed`. This is a DESIGN GAP surfacing as a bug, not a platform quirk.

ARCHITECTURE line 479 already names this open: "process exit while work is pending (which child
processes are reaped, by whom)." Never answered. dsh forces it.

## What must be true (owner's rules)

- Closing a session must NOT stop the shared service. A session ending is not the service ending;
  other sessions use it.
- Only removing the PLUGIN releases the plugin's resources.
- The release is PLUGIN-scoped, not "a request answered by some adapter process". A plugin-level
  lifecycle fact must not depend on an adapter happening to be alive.
- The hub owns the LIFECYCLE; the plugin owns WHAT IT STARTED and HOW TO STOP IT. The hub never
  enumerates or guesses about processes it cannot see.

## The proposal

Uninstall gains a RELEASE phase, symmetric to install's PREPARE phase.

- Install today: materialise the runtime (prepare) -> `ready`.
- Remove today: `removing` -> delete the directory.
- Remove proposed: `removing` -> RELEASE what the plugin started -> delete the directory.

Properties:
- The release is a PLUGIN-LEVEL lifecycle step in the contract, not a private hub hook and not a
  session call. It is IDEMPOTENT (releasing twice is a no-op) and BEST-EFFORT-WITH-TRUTH: if the
  release cannot complete, the remove still attempts the delete and reports the truth (`failed` +
  reason), never a fake success and never a hang.
- WHO performs it: the plugin declares it (its adapter already contains the code — the same path that
  stops dsh on stdin EOF). The hub DRIVES the release during remove and ENSURES THE PLUGIN IS ASKED
  (it does not rely on an adapter being alive).
- ORDERING: release runs AFTER `begin_remove` admits and AFTER the plugin leaves the live registry,
  and BEFORE `finish_remove` (the delete).

## Contract shape (for review — NOT yet written)

Add a core lifecycle request to `contract/adapter-v1.json` `coreCompliance.requests`, so EVERY
adapter implements it and the boot self-check + conformance suite bind it. Working name
`plugin/cleanup` (owner sets the final name):

- `params`: `{}`.
- `result`: `{}`.
- `rule`: "the plugin is being REMOVED; release everything the plugin owns on the harness side
  (services, helper processes, temporary files). Idempotent. The hub calls this during a remove,
  before deleting the plugin directory, and does not treat its failure as a successful release."

The hub then has ONE place that reconciles plugin lifecycle over adapters; the adapter has ONE place
that releases the harness service.

## Why this is not a workaround

- Removes the "hard-delete and hope" loop: the delete runs only after release (or reports a truthful
  failure).
- Restores symmetry: install prepares a runtime; remove releases what that runtime started.
- Answers ARCHITECTURE 479 for services, which the design left open.

## Cases (extend 20261005-plugin-lifecycle-cases.md)

- C5 (new) — remove a plugin whose adapter started a long-lived SERVICE (dsh): the service is stopped,
  the directory deletes, state -> `absent`. (today: FAILS `Access is denied`.)
- C6 (new) — close a session of a service-using plugin: the SERVICE STAYS UP (shared); only remove
  stops it.
- D4 (exists) — remove whose delete fails: terminal `failed`, not stuck `removing`.
- C7 (new) — the release is idempotent: a second remove attempt (or a crash mid-remove, E5) still
  converges to `absent`.

## Open question for the owner (one decision)

When the remove runs and NO adapter is alive for the plugin, the release is a PLUGIN-LEVEL lifecycle
step the hub performs; the hub must obtain what it needs to release the plugin's resources WITHOUT
depending on a running session. This proposal assumes the release is driven THROUGH the plugin (the
plugin's own code), the hub ensuring the plugin is asked. CONFIRM the release is plugin-level
(hub-driven) rather than "send a request to whichever adapter is alive."

## Files this will touch (after approval)

1. `contract/adapter-v1.json` (+ openapi.json/v1.json prose) — the new core lifecycle request.
2. `crates/plugins/src/routes.rs` + `service.rs` — the remove sequence (release before delete).
3. `crates/adapter/src/manager.rs` — reaching the plugin's release during remove.
4. `prts-harness-deepseek/deepseek-adapter.cjs` — implement the release (stop dsh), reusing the
   existing shutdown path.

## Sync note (session compact)

- Task: ADAPT the dsh adapter to dsh 0.2.0, then make plugin REMOVE clean (this doc).
- dsh adapter 0.2.0 adaptation is essentially DONE and verified: auth (`?token=` -> `dsh-auth`
  cookie), endpoint vocabulary (`ns/method`), `{args}` payloads, `session/prompt` `requestId`, the
  `remote.mux` + `session/follow` + `$events` event streams, approvals via `$events/result`, tools
  recorded into the transcript. Whole-chain for deepseek: 13/14 (install, ready, session, real turn,
  REAL tool + result in the answer, close, remove ACCEPTED). Only `the removed plugin is gone` fails,
  for the reason above.
- Adapter commits: 1f4bc97, 34c57f9, 6bb9eed (prts-harness-deepseek).
- Hub commits this task: 69968a4 (provider id rule), 8b71d19 (harness catalog), d142ef7 (thinking
  level), 632dea2 (_CUSTOM- + close stdin before kill).
- Canonical py runner: PI_PLUGIN_DIR=/e/AI/ideas/prts-harness-deepseek PI_HARNESS_ID=deepseek
  PI_MODEL=gpt-6-astra python
  tests/lifecycle/the-whole-chain-plugin-to-session-to-tools-over-v1.py

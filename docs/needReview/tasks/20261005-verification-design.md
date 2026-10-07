# Verification design — the real user task, end to end (FOR REVIEW)

Date 2026-10-05. Owner order: stop patching tests. The registry is UNPUBLISHED, so the real plugin
chain does not exist and the hub's plugin-driver capability CANNOT be proven now. This document is
the acceptance DESIGN only. Nothing here is run; nothing is claimed.

Guiding rule: **prove the hub can complete a user's task**, not that a test script can be made to
print a preset result. No test-side stand-in for a hub responsibility (directory fetch, plugin
selection, install, runtime preparation).

---

## The real user task (what is actually being accepted)

A user who has a hub binary with no plugins wants to: **get the official plugin catalog, install a
harness plugin (e.g. `pi`) the hub's registry lists, have its runtime prepared, open a session on
it, run one turn against a real provider, and read the answer.**

Every step below must be performed BY THE HUB through its own `/v1` surface. The test may only:
choose which id to ask for, and read responses.

---

## The chain, step by step

Each row: the step, WHO does it (the hub), the `/v1` operation, the user-observable completion, and
the explicit FAILURE condition.

### S0. Start from an empty hub
- precondition: a hub binary started with an EMPTY plugins root and NO plugins field. No env
  override of the registry address (a release build ignores it; a dev build must have none set).
- hub responsibility: expose its own state.
- observe: `GET /v1/plugins` -> `plugins: []`; `GET /v1/harnesses` -> `harnesses: []` with a note.
- FAIL: the hub reports a plugin/harness it was not given.

### S1. The hub obtains the OFFICIAL directory by ITS OWN mechanism
- who: the HUB (not the test).
- operation: `POST /v1/plugins/registry/refresh` (the hub fetches ITS compiled-in official address).
  The test supplies NOTHING — no url, no file, no loopback.
- observe: 2xx; then `GET /v1/plugins/catalog` lists the official plugin ids.
- FAIL: refresh cannot reach the official address; or the catalog is empty after a 2xx; or the hub
  needed the test to tell it where the registry is.
- BLOCKED TODAY: the official `registry` release asset is UNPUBLISHED (HTTP 404). Until it exists,
  S1 cannot pass and everything below is unverified.

### S2. The hub installs a plugin the registry LISTS, by ID
- operation: `POST /v1/plugins {source:{id:"pi"}}` — the caller names an ID; the hub resolves the
  release (url+sha256) from ITS OWN registry and installs it. (NOTE: today the route takes a
  caller url/artifact and reconciles it; the id-only source is the target shape — see the proposal.)
- hub responsibility: resolve the id, fetch, verify sha256+size, land under `<DATA_DIR>/plugins/pi`.
- observe: 202, then `GET /v1/plugins/pi` reaches `state:"ready"`.
- FAIL: an id the registry does not list installs; a listed id does not install; bytes that do not
  match the pinned sha256 install.

### S3. The hub prepares the runtime by ITS OWN mechanism
- operation: `POST /v1/plugins/pi/prepare` (or the hub does it before session start).
- hub responsibility: the adapter materialises the runtime the manifest pins; the hub VERIFIES the
  declared command exists.
- observe: `GET /v1/plugins/pi` -> `runtimeReady:true` with the resolved runtime target.
- FAIL: `runtimeReady` true but the command file is absent; or a session starts on a missing runtime.

### S4. The hub exposes the installed harness
- operation: `GET /v1/harnesses`.
- observe: `pi` is listed, with its real capabilities.
- FAIL: the installed harness is not listed without a restart; or a capability is faked.

### S5. A real provider is registered through the hub
- operation: `POST /v1/model-providers` with a REAL provider (a genuine endpoint + credential).
- hub responsibility: store it, inject it at session start.
- observe: `GET /v1/model-providers/{id}/models` (or the harness model list) answers with real models.
- FAIL: injection fails (the adapter reports the provider unreachable).
- NOTE: this needs a working provider; the host's provider is currently answering 400 — a separate,
  real precondition (20261005-150000).

### S6. The hub opens a session on the installed harness
- operation: `POST /v1/sessions {harnessId:"pi", modelProviderId, modelId}`.
- observe: session `status:"active"`, `appliedPlan/appliedReview` per the contract, `cwd` set.
- FAIL: `starting_failed`; or an `appliedX` field claimed without the adapter applying it.

### S7. The user's task completes: a REAL turn, a REAL tool, a REAL answer
- operation: `POST /v1/sessions/{id}/turns` with a task that requires a tool whose effect the test
  can observe independently (e.g. write a file with a RANDOM token, then read it back; or read a
  file the test created in the session's cwd with a random token).
- observe: the turn reaches `ended`; the assistant message CONTAINS the random token (proof the tool
  really ran on THIS turn), not an echo or a guess.
- FAIL: the token absent; or present without any tool call; or the turn never terminal.

### S8. The chain holds DOWNWARD
- operation: close the session, then `DELETE /v1/plugins/pi`.
- observe: the session is readable after close; remove is accepted; `GET /v1/plugins/pi` -> 404.
- FAIL: remove refused while no session holds it; or the record/dir left half-deleted.

---

## Failure conditions that are the POINT (each must be testable)
- caller cannot install an unlisted source (verified today: 403 `plugin_not_in_registry`);
- the hub cannot be pointed at a foreign registry (verified today: release ignores the override);
- an empty hub does not invent a plugin (verified today);
- registry unreachable -> a NAMED registry error, never a silent empty catalog;
- a session does not start on a missing runtime.

## Blocked today (registry UNPUBLISHED) — the whole chain S1..S8 beyond S0
Reason: the real plugin registry release is not published, so no usable plugin environment can be
established through the hub's own mechanism. This is NOT 'the hub cannot do it'; it is 'not yet
established'. S5 is additionally blocked by the host provider (400).

## Anti-requirements (explicitly forbidden in the acceptance)
- the test must not fetch the directory, choose a url, copy a plugin dir, or prepare a runtime;
- no mock registry, no `AGENT_HUB_REGISTRY_URL`/`AGENT_HUB_REGISTRY_FILE` override to fake a source;
- no field echo, 202, empty status, natural end, or scaffold output as proof of a chain step.

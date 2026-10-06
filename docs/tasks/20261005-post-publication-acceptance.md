# Post-publication acceptance — the chain to VERIFY once the registry is published

Date 2026-10-05. To be run ONLY after the official registry is published and a REAL provider is
reachable. Everything is performed BY THE HUB through /v1; the test only names an id and reads.

## Preconditions (all real, all external)
- the official registry release asset exists (HTTP 200) at the hub's compiled-in address;
- the host provider answers 2xx for its own /models with a valid token (today: 400 — 20261005-150000);
- a RELEASE hub binary built from the commit under test.

## The acceptance run (in order; any step's failure stops it)
1. start an EMPTY hub (no override set); `GET /v1/plugins` = [] and `GET /v1/harnesses` = [].
2. `POST /v1/plugins/registry/refresh` -> 2xx; `GET /v1/plugins/catalog` lists the official ids.
3. `POST /v1/plugins {source:{id:"pi"}}` -> 202 -> `GET /v1/plugins/pi` reaches `ready`.
4. `POST /v1/plugins/pi/prepare` -> `runtimeReady:true` with a resolved target.
5. `GET /v1/harnesses` lists `pi` with real capabilities.
6. `POST /v1/model-providers` with the real provider; the harness model list answers.
7. `POST /v1/sessions {harnessId:"pi", ...}` -> `active`; `appliedX` per contract.
8. `POST /v1/sessions/{id}/turns` doing a task with an INDEPENDENT observable (random-token file);
   the assistant answer contains the token (a real tool ran this turn).
9. close the session; `DELETE /v1/plugins/pi`; `GET /v1/plugins/pi` -> 404.

## What each step must NOT rely on
- no test-supplied url, file, or loopback registry;
- no `AGENT_HUB_REGISTRY_URL`/`AGENT_HUB_REGISTRY_FILE` override;
- no copy of a plugin dir; no test-run prepare; no echo/202/empty-status as proof.

## Recorded per step
the exact /v1 request, the raw response, and the on-disk fact (dir exists, command file exists,
file written) that proves the step; the raw log archived with the commit sha.

## Until then
Every step 1..9 beyond step 1 is UNVERIFIED. Do not publish the registry early; do not declare the
capability accepted. Publication is for verifying the chain, not for blessing earlier results.

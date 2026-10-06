# Entry reconciliation — what the current entry keeps, what left it, and why

Date 2026-10-05. The official registry is UNPUBLISHED and no usable plugin environment exists. The
DEFAULT test entry therefore verifies only LOCAL boundaries; the plugin-driven user chain is
UNVERIFIED. This is the account of the entry change. No test run beyond the entry itself; no registry;
no model; no new scaffold. No assertion changed.

## A. KEPT in the entry (no registry needed) — and exactly what each proves

| file | proves (narrow) | does NOT prove |
|---|---|---|
| contract/status-surface-openapi-and-models-answer | the served metadata shapes: openapi equals the committed file; `/v1/models` is an ARRAY of {id,providerId} | that any plugin/model feature works |
| contract/the-served-surface-equals-the-contract | the served ROUTE SET equals the contract; no parameterless GET is 501 | that a route is capable |
| concurrency/one-hundred-connections-do-not-time-out | 120 raw sockets held open at once; a held SSE (first byte, not ended) does not block a GET | any plugin/user capability |
| plugins/a-caller-cannot-install-from-an-arbitrary-url | with NO registry, an unlisted source -> 403 `plugin_not_in_registry`, nothing lands | that a published registry authorizes correctly |
| plugins/install-reconciles-against-the-registry | with NO registry, all sources refused 403 (authorized path BLOCKED inside) | the authorized install |
| plugins/install-refusals-have-honest-codes | with NO registry, git/artifact refusals use contract codes (sha256 path BLOCKED inside) | the sha256 check |
| plugins/registry-refresh-updates-the-local-registry | refresh ERROR codes (dead/500/non-JSON/no-array) name the case (success path BLOCKED inside) | that the registry is usable |
| plugins/the-release-hub-error-surface | release build: 400/404/403 error codes (refresh BLOCKED inside) | registry capability |
| plugins/the-release-hub-ignores-the-registry-override | a RELEASE binary does not contact a canary registry (measured by server hits) | that the registry is trusted |
| harnesses/an-uninstalled-harness-cannot-be-used | an empty hub lists no harness; a session is 404 harness_not_found | any installed-harness behaviour |
| skills/skills-crud-over-the-real-surface | skills CRUD; delete proven by the file's 404 | any plugin/user task |
| connections/connections-are-crud-and-delete-is-real | connections CRUD; PATCH read back; delete proven by a 200 list lacking the id | any plugin/user task |

These four are PARTIAL (an internal BLOCKED sub-check): install-reconciles, install-refusals,
registry-refresh, the-release-hub-error-surface. "PARTIAL" is NOT a capability pass.

## B. MOVED out of the default entry (to `--materials`, informational) — and why

All 33 files that need a plugin installed through the hub: lifecycle/* (3), interrupt/* (10),
approvals/* (3), concurrency/* (4 minus the local one), provider/* (1), plugins lifecycle/events/
concurrent (5), harnesses/discovery (1), presets/* (2), tools/* (1), model/* (1), contract/events (1).

Reason: each requires a plugin environment that can only be established through the hub's own
directory mechanism, which needs the PUBLISHED registry. With no registry they cannot run; with a
FABRICATED one they test the scaffold, not the hub. Their result is not a capability result and is
not an acceptance entry. NOT deleted — they remain as unverified materials.

## C. PRODUCT GAPS (not working operations; do not present as available)

1. **Install by ID is NOT implemented.** `POST /v1/plugins` accepts a caller `url`/`artifact` (a git
   ref, or an artifact url+sha256) — it does NOT accept `{source:{id}}`. So 'the hub resolves a
   plugin from its registry by id' does not exist as an operation. A test must not assemble an
   artifact to fill this in. (Proposal only, unreviewed: docs/review/20261005-contract-proposal...)
2. **The registry is not published**, so even the caller-url path has no authoritative source.

## D. WAITING on the published registry + a real provider (the chain to verify)

The whole user chain: obtain the directory via the hub -> install a LISTED plugin by id -> prepare the
runtime -> open a session -> a real tool turn with an independent observable -> close/remove. Design:
docs/tasks/20261005-verification-design.md; ordered run: docs/tasks/20261005-post-publication-acceptance.md.

## E. What is NOT claimed
The entry result is NOT 'hub capabilities passed'. No plugin/user capability is accepted. No plugin
lifecycle result stands. The registry stays unpublished; no model verification is run; no scaffold
is added. Awaiting review.

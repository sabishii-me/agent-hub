# Entry reconciliation — what the current entry keeps, what left it, and why

Date 2026-10-05. The official registry is UNPUBLISHED and no usable plugin environment exists. The
DEFAULT test entry therefore verifies only LOCAL boundaries; the plugin-driven user chain is
UNVERIFIED. This is the account of the entry change. No test run beyond the entry itself; no registry;
no model; no new scaffold. No assertion changed.

## A. KEPT in the default entry (PURE-LOCAL: no override, no local registry server, no external refresh)

| file | proves (narrow) | does NOT prove |
|---|---|---|
| contract/status-surface-openapi-and-models-answer | served metadata shapes: openapi equals the committed file; `/v1/models` is an ARRAY of {id,providerId} | any plugin/model feature |
| contract/the-served-surface-equals-the-contract | the served ROUTE SET equals the contract; no parameterless GET is 501 | that a route is capable |
| concurrency/one-hundred-connections-do-not-time-out | 120 raw sockets held open; a held SSE (first byte, not ended) does not block a GET | any plugin/user capability |
| plugins/a-caller-cannot-install-from-an-arbitrary-url | with NO registry, an unlisted source -> 403 plugin_not_in_registry, nothing lands | that a published registry authorizes correctly |
| plugins/install-reconciles-against-the-registry | with NO registry, all sources refused 403 (authorized path BLOCKED inside) | the authorized install |
| plugins/install-refusals-have-honest-codes | with NO registry, git/artifact refusals use contract codes (sha256 path BLOCKED inside) | the sha256 check |
| harnesses/an-uninstalled-harness-cannot-be-used | an empty hub lists no harness; a session is 404 harness_not_found | any installed-harness behaviour |
| skills/skills-crud-over-the-real-surface | skills CRUD; delete proven by the file's 404 | any plugin/user task |
| connections/connections-are-crud-and-delete-is-real | connections CRUD; PATCH read back; delete proven by a 200 list lacking the id | any plugin/user task |

Default entry, measured: **9 files -> 7 PASS, 2 PARTIAL, 0 FAIL, 0 BLOCKED**. PARTIAL = the two
files with an internal BLOCKED sub-check. This is NOT a capability result.

## B. REMOVED from the default entry (and why)

Three files LEFT THE ENTRY (not the tree) because they use a registry substitute or an external
registry request - exactly what the entry must not do:
- plugins/registry-refresh-updates-the-local-registry.py (registry override + a local registry server)
- plugins/the-release-hub-ignores-the-registry-override.py (registry override + a local canary server)
- plugins/the-release-hub-error-surface.py (sends a registry refresh; runs a RELEASE binary)

They are NOT deleted. They are listed under `--materials` (path + reason).

The other 33 chain files (lifecycle/interrupt/approvals/concurrency/provider/plugin-lifecycle/
events/harness-discovery/presets/tools/model) also left the entry: each needs a plugin installed
through the hub, which needs the PUBLISHED registry. 36 files total are listed under `--materials`.

## C. `--materials` is a LIST, not an entry

`python tests/run.py --materials` prints each material path, why it is not run, and the product gaps,
and **starts NO subprocess** (verified: 0 processes). It cannot execute installs, credentials, or
models. Retaining the source files does not retain an execution entry.

## D. PRODUCT GAPS (not available operations)

1. install by id is NOT implemented: POST /v1/plugins takes a caller url/artifact, not {source:{id}}.
   A test must not assemble an artifact to fill this in. (Proposal only, unreviewed.)
2. the official registry release is UNPUBLISHED, so no authoritative plugin source exists.

The full user chain (obtain directory via the hub -> install a LISTED plugin by id -> prepare the
runtime -> session -> a real tool turn -> close/remove) is designed in
docs/tasks/20261005-verification-design.md and ordered in
docs/tasks/20261005-post-publication-acceptance.md.

## E. What is NOT claimed
The entry result is NOT 'hub capabilities passed'. No plugin/user capability is accepted. No plugin
lifecycle result stands. The registry stays unpublished; no model verification is run; no scaffold
is added. Awaiting review.

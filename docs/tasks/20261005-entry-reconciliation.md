# Entry reconciliation — per-file: what it proves, its premise, its false-green test, and its class

Date 2026-10-05. STATIC only: no test run, no hub started, no credential read, no model called. The
official registry stays UNPUBLISHED. This table REPLACES the earlier version. One table, no new doc
system.

## Fixed boundary (not changed this round)
- the official registry stays unpublished; the dev `AGENT_HUB_REGISTRY_URL`/`_FILE` override is KEPT
  (not deleted, not banned);
- NO dev registry is configured this round; no substitute server; no override used to obtain green;
- the CURRENT contract stands: `source.artifact` is a legal input; the id-only shape is NOT imposed;
  no product interface/field/behaviour is added for tests;
- running the entry is NOT required this round; conclusions below are static and add no PASS.

## The default entry = PURE-LOCAL files that spawn a real hub for a local boundary.
`tests/run.py` default list; measured 9 files -> 7 PASS, 2 PARTIAL, 0 FAIL, 0 BLOCKED (a previous
observation, not re-run this round, not a capability result).

| file : lines | fact it actually asserts | premise (how established) | scaffold role | false-green counterexample (or why kept) | class | conclusion withdrawn |
|---|---|---|---|---|---|---|
| tests/contract/status-surface-openapi-and-models-answer.py:24-53 | `/v1/status`,`/v1/surface`,`/v1/openapi.json`,`/v1/models` answer 200; served openapi == committed file; `models` is an ARRAY of {id,providerId} | real hub on an empty data dir | none | a hub returning the right ARRAY shape with WRONG model data still passes - the file does NOT claim model correctness, only the shape; named so | KEEP (narrow) | none; claim limited to shapes |
| tests/contract/the-served-surface-equals-the-contract.py:27-66 | the served ROUTE SET equals the contract; no parameterless GET is 501 (a non-501 4xx/5xx is recorded BLOCKED, not passed) | real hub; probes real GETs | none | a hub whose surface LISTS all routes but every handler is a no-op still passes the route-set check - the file's ONLY claim; capability explicitly NOT claimed | KEEP (narrow) | any 'no stub'/'all routes capable' reading |
| tests/concurrency/one-hundred-connections-do-not-time-out.py:35-126 | held-open raw sockets count; a held SSE (opened + first byte + not ended) does not block a GET | real hub; N real sockets held before release; SSE confirmed via first byte | none | if the SSE thread errors, `opened=False` -> the precondition check FAILS (no false green); the held-count check measures the real count | KEEP (narrow) | the earlier '>=100 fails' conclusion (a measurement artefact) |
| tests/plugins/a-caller-cannot-install-from-an-arbitrary-url.py:37-53 | with NO registry, an unlisted source -> 403 `plugin_not_in_registry`; nothing lands | `Hub()` empty, no registry | none | a hub that refuses EVERYTHING with that code passes - exactly the narrow claim (no registry -> refuse); it does NOT claim a published registry authorizes | KEEP (narrow) | any 'injection-proof' beyond the no-registry case |
| tests/plugins/install-reconciles-against-the-registry.py:45-72 | with NO registry, an unlisted url / a real release / a git source are ALL refused 403; the AUTHORIZED path is BLOCKED | `AGENT_HUB_REGISTRY_FILE=""` -> no registry | none | a hub that refuses all installs passes; the authorized path is explicitly BLOCKED (not passed) | KEEP (narrow, PARTIAL) | any 'reconcile works' claim |
| tests/plugins/install-refusals-have-honest-codes.py:46-106 | with NO registry: empty source -> 400 validation_failed; git -> 403; artifact -> 403; nothing lands; the sha256-specific refusal is BLOCKED | `Hub()` empty, no registry | none | a hub refusing all with 403 passes for B7/B8; the sha256 claim is BLOCKED, not asserted | KEEP (narrow, PARTIAL) | the sha256-mismatch claim |
| tests/harnesses/an-uninstalled-harness-cannot-be-used.py:21-43 | an empty hub lists no plugin/harness; `POST /v1/sessions {pi}` -> 404 `harness_not_found`; a turn on a missing session -> 404 | real hub, empty root | none | a hub that 404s a session for ANY harness passes; the claim is 'an UNINSTALLED harness is refused', named so | KEEP (narrow) | none |
| tests/skills/skills-crud-over-the-real-surface.py:19-44 | skills write/read/list; path-escape 400; delete proven by the file GET 404 + list 200 | real hub | none | a no-op delete leaves the file readable -> the 404 check fails; a 500 post-delete fails the '200' check | KEEP (narrow) | none |
| tests/connections/connections-are-crud-and-delete-is-real.py:19-57 | connections create/list/patch/delete; token never echoed; PATCH read back; delete proven by a 200 list lacking the id | real hub | none | a no-op PATCH fails the read-back; a 500 post-delete fails the '200' check | KEEP (narrow) | none |

---

## REMOVED from the default entry (source kept; listed via `--materials`, NOT executed)

| file | fact it tries to assert | premise problem | class | withdrawn conclusion |
|---|---|---|---|---|
| tests/plugins/registry-refresh-updates-the-local-registry.py | refresh's ERROR codes at dead/500/non-JSON/no-array | uses `AGENT_HUB_REGISTRY_URL` at a LOCAL server -> the TEST supplies the registry source; the success path was a scaffold (a self-invented id became the catalog) | EXIT | any 'the registry works' reading from a loopback source |
| tests/plugins/the-release-hub-ignores-the-registry-override.py | a RELEASE binary ignores the override (measured by server hits) | uses a local canary server + the override; the premise is real (debug DOES hit it) but it is a SECURITY PROBE, not a capability | EXIT | any 'capability' reading |
| tests/plugins/the-release-hub-error-surface.py | release-build error codes | sends a registry REFRESH to the official address + runs a release binary | EXIT | any registry capability reading |

## The 33 plugin-chain files (lifecycle/interrupt/approvals/concurrency/provider/plugins-lifecycle/
## events/harness-discovery/presets/tools/model)
Fact they try to assert: a plugin installed through the hub drives a session/turn/etc.
Premise problem: each needs a plugin installed via the hub, which needs the PUBLISHED registry; a
dev override / fabricated registry would have the TEST establish the plugin environment (the hub's
own job) and then call the result "the hub works".
Class: **UNVERIFIED** (not run; not a product failure).
Withdrawn: every earlier per-file PASS/FAIL on these (all produced with a scaffold-supplied
environment). Listed under `--materials` with reasons; started by NO entry.

## PRODUCT GAPS (not available operations; not filled in by the suite)
1. **install by id is NOT implemented**: `POST /v1/plugins` takes a caller `url`/`artifact`, not
   `{source:{id}}`. The suite must not assemble an artifact to fill this in. (id-only is a PROPOSAL,
   unreviewed; not imposed this round.)
2. **the official registry is UNPUBLISHED**, so no authoritative plugin source exists.

## The runner itself (a code defect, NOT a capability claim)
`tests/run.py`: the entry (a) runs only the 9 pure-local files, (b) `--materials` LISTS and starts NO
subprocess, (c) exits non-zero on FAIL/CRASH OR when nothing was verified (0 PASS). Any remaining
runner defect is a TOOLING matter and must not be read as "the functional tests are valid".

## Not required this round
The registry need not be published; the plugin chain need not run; nothing must be green. The point
is that each item on the entry has a stated object, an established premise, an attributable result,
and a conclusion not exceeding the evidence - and that any item with a known false-green path is no
longer used as acceptance evidence.

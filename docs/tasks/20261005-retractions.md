# Retractions — conclusions withdrawn because they exceed the evidence

Date 2026-10-05. The registry is UNPUBLISHED and the real plugin chain does not exist. Everything
below is WITHDRAWN. None of it may be reported as hub capability.

## Withdrawn claims (and why)
1. **"N of the Python tests PASS, so N hub capabilities are accepted."** WITHDRAWN. A suite file's
   PASS is a statement about that file's own narrow check, not about a user capability. The
   PASS/FAIL counts are NOT a hub-capability result.
2. **"7 (or 8) hub capabilities verified."** WITHDRAWN. Those files check interface shape, CRUD,
   error codes and security probes — local facts about surfaces, NOT the plugin-driving chain.
3. **"the hub supports >=100 concurrent connections."** WITHDRAWN as a capability claim. It is
   measured now (120 raw sockets held), but that is a connection-count observation, not a user-task
   capability; it must not be listed among accepted hub capabilities.
4. **"the hub FAILS to support 100 concurrent connections" (the earlier FAIL).** WITHDRAWN. That was
   a MEASUREMENT artefact (the counter ran after urlopen() returned), not a product failure.
5. **"install is safe / cannot inject" beyond the no-registry case.** WITHDRAWN. Verified only:
   with NO registry, an unlisted source is refused 403 `plugin_not_in_registry`. That a published
   registry would correctly authorize/reject is UNVERIFIED.
6. **"the release hub cannot fetch its registry" as a hub defect.** RECLASSIFIED. The asset is
   UNPUBLISHED; the hub's refresh correctly reports it cannot reach it. Not a defect; a missing
   precondition.
7. **Any plugin lifecycle pass (install/remove/prepare/events/full-chain).** WITHDRAWN. All were
   produced with a test-side or fabricated registry or a caller-supplied source; the hub's own
   directory mechanism was never exercised. They are UNVERIFIED, not passed.

## Withdrawn as ACCEPTANCE (kept only as narrow, labelled observation, if kept at all)
- contract/served-surface: exactly "the route set matches, and no parameterless GET is 501". Not
  "every route is capable".
- contract/status-surface: "/v1/models is an array of {id,providerId}". Not "the model surface is
  correct".
- plugins/a-caller-cannot-install: "no registry -> 403 plugin_not_in_registry". Not "injection-proof".
- plugins/release-ignores-override: "a release binary ignores the env override". Not "the registry
  is trusted".
- skills/connections CRUD: "these rows store/read back/delete with an authoritative read". Not part
  of any user task acceptance.
- the-release-hub-error-surface: the error codes. Not registry capability.

## What remains PROVEN (factual, narrow, not capabilities)
- the served route set equals the contract; no parameterless GET answers 501;
- `/v1/models` is a JSON array of objects with id+providerId;
- with NO registry, an unlisted source is refused 403 `plugin_not_in_registry` and nothing lands;
- a RELEASE binary ignores `AGENT_HUB_REGISTRY_URL` (debug/release opposite outcomes);
- an empty hub lists no plugin/harness and refuses a session with 404 harness_not_found;
- connections/skills CRUD perform an authoritative read-back (patch changes; delete -> 404/absence
  in a 200 list);
- the hub holds 120 simultaneously-open connections and answers a fresh GET while holding them;
- the SSE stream opens (first byte) and stays open, and does not block a short GET.
These are observations about surfaces. NONE of them proves the plugin-driver capability.

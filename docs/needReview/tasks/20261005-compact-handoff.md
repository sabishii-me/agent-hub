# COMPACT HANDOFF — paste this to resume (2026-10-05, hub 19e5410)

## GOAL
Implement the Rust agent-hub per the approved architecture, delivering REAL, usable `/v1`
capabilities against REAL adapters/plugins (§17 order of work). The deliverable is the PRODUCT, not
green tests. A capability is DONE only when driven through `/v1` against a REAL adapter/plugin.

## HARD RULES (owner, do not break)
- No fake anywhere: no mock registry, no test-side install/prepare, no test stand-in for a hub
  responsibility. A missing real precondition is BLOCKED/unverified, never a green.
- A change to `contract/` or an ADR needs the OWNER's review BEFORE it is written. Do NOT impose a
  contract change (e.g. id-only install) to make tests pass.
- NEVER `git checkout --`/reset/discard uncommitted work. Docs first, then code.
- Diagnoses need a CONTROL; suspect MY inputs first; never experiment on the owner's system; do not
  read real credentials or call models unless asked.
- Do NOT treat PASS/FAIL counts as hub-capability acceptance.

## ONE BLOCKER
The official plugin registry is UNPUBLISHED. Hub compiles in the address
`https://github.com/sabishii-me/agent-hub/releases/download/registry/registry.json` (a fixed
`registry` release asset); it is HTTP 404 (the repo has 0 releases). Until it is published, the
plugin-driven user chain cannot be established -> UNVERIFIED.
SECOND blocker: the host provider (`~/.pi/agent/models.json` -> 192.168.31.29:8990) answers 400 to
its own `/models` with the host token -> session/turn tests blocked (20261005-150000).

## STATE OF THE SUITE (the "no fake" entry)
`python tests/run.py` = a fixed 10-file PURE-LOCAL entry -> 8 PASS, 2 PARTIAL, 0 FAIL, 0 BLOCKED:
served metadata shapes; route set == contract; **registry-online rehearsal 7/7** (a real HTTP
registry serving the repo's registry.json verbatim; the hub refreshes ITS registry, the catalog
lists real ids, the hub installs a release the CATALOG names - test supplies the ADDRESS only ->
ready); no-registry refusals 403 plugin_not_in_registry (3 files); release ignores the env override
(probe); empty hub -> session 404 harness_not_found; skills + connections CRUD; 120 held sockets +
SSE.
`python tests/run.py --materials` LISTS the 33 plugin-chain files (path + why) + product gaps and
starts NO subprocess. Their earlier PASS/FAIL are WITHDRAWN (a scaffold supplied the environment).
Docs: docs/tasks/20261005-{retractions,entry-reconciliation,verification-design,
post-publication-acceptance,convergence-report}.md.

## PRODUCT FIXES THIS SESSION (real, committed)
- T0 20261005-130000 FIXED: install RECONCILES the source against the hub's loaded registry
  (service.rs begin_install -> authorized(); registry.rs Registry::authorizes: url AND sha256). With
  no registry nothing installs; with a registry an unlisted source is refused 403.
- 20261005-120000 FIXED: install of a deployment-owned id -> 409 conflict, tree untouched.
- 20261005-140000/-141000 FIXED: missing-id remove -> 404 not_found (no false "deployment
  directory"); body missing `source` -> 400 validation_failed (not 422).
- 20261005-110000 A FIXED + the contract code `registry_unavailable` (502, owner-approved) added and
  mapped; `RegistryUrlMissing` now displays its payload.
- The registry ADDRESS is fixed at build time: release = official URL, NO env override (verified 0
  occurrences in the release binary); dev = AGENT_HUB_REGISTRY_URL override (compiled out of release).

## KEY FILES
- crates/plugins/src/{registry.rs,service.rs,routes.rs} — address, reconcile, error codes.
- contract/errors.json — includes registry_unavailable; contract/openapi.json — install source is
  caller url/artifact (id-only is an UNREVIEWED proposal in docs/review/).
- tests/lib/hub.py — Hub(); install_plugins drives the HUB's own refresh+catalog; no _registry().
- tests/run.py — the entry (default pure-local; --materials lists, runs nothing).
- docs/tasks/CURRENT-STATE.md — the durable handoff (read first).

## NEXT (in order)
1. Owner publishes the `registry` release of sabishii-me/agent-hub; fix the host provider.
2. Run the chain per docs/tasks/20261005-post-publication-acceptance.md — through the hub's own
   mechanism, no test-side stand-in. Only then are the 33 UNVERIFIED files runnable and the
   plugin-driver capability establishable.
3. Do NOT publish the registry to make tests green; publication is for verifying the real chain.

## OPEN ISSUES
130000 [T0, fix done, verify on publish]; 100000 (registry is a local file for tests; official
release missing); 150000 (host provider 400); 20261004-060000/-070000 (jouzu residual);
20261003-122000 (contract close/readonly); the unratified turn_end clientMessageId edit.

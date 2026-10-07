# CURRENT STATE — read this first (durable handoff)

Updated 2026-10-05, hub commit `19746ff` (branch `feat/hub-modular-redesign`). Working tree clean.

## The task (unchanged)

Implement the Rust agent-hub per the approved architecture, delivering REAL, usable `/v1`
capabilities **against the REAL adapters/plugins**. §17 of `docs/ARCHITECTURE.md` is the order of
work. A capability is DONE only when driven through `/v1` against a REAL adapter/plugin.

## The rule (repeated; do not break)

- A FAKE anywhere makes the whole chain false. A missing REAL dependency FAILS the test - no fake
  green, no scaffold substituting for the product.
- NEVER `git checkout --` / `reset --hard` / discard uncommitted work.
- Docs FIRST, then code. A change to `contract/` or an ADR needs the OWNER's review BEFORE it is
  written. Do NOT impose a contract change to make tests pass.
- A diagnosis needs a CONTROL; one observation is a guess. Suspect MY inputs first.
- NEVER experiment on the owner's system; never read real credentials or call models unless the
  task says so.

## Where things stand (2026-10-05)

**The single blocker: the official plugin registry is UNPUBLISHED**, so the plugin-driven user
chain cannot be established and is UNVERIFIED. The registry address is compiled into the hub:
`https://github.com/sabishii-me/agent-hub/releases/download/registry/registry.json` (a fixed
`registry` release asset) — currently HTTP 404 (the repo has 0 releases).

### Verified, narrow (the default test entry — `python tests/run.py`, 10 files -> 8 PASS, 2 PARTIAL)
- served metadata shapes (openapi == committed file; `/v1/models` is an ARRAY of {id,providerId});
- the served ROUTE SET equals the contract; no parameterless GET is 501;
- **registry-online rehearsal (7/7)**: a real HTTP registry serving the repo's `registry.json`
  verbatim; the hub refreshes ITS registry, the catalog lists the real ids, and the hub installs a
  release the CATALOG names (the test supplies the ADDRESS only, never a url/sha256) -> `ready`;
- with NO registry, an unlisted source is refused 403 `plugin_not_in_registry` (install-reconciles,
  install-refusals, a-caller-cannot-install);
- a RELEASE binary ignores `AGENT_HUB_REGISTRY_URL` (probe);
- an empty hub lists no plugin/harness; a session is 404 `harness_not_found`;
- skills + connections CRUD (authoritative read-back);
- 120 raw sockets held open; a held SSE does not block a GET.

### UNVERIFIED (registry unpublished)
The 33 plugin-chain files (lifecycle/interrupt/approvals/concurrency/provider/plugins-lifecycle/
events/harness-discovery/presets/tools/model) need a plugin installed THROUGH the hub, which needs
the published registry. Listed via `python tests/run.py --materials` (path + reason + product gaps);
`--materials` starts NO subprocess. Their earlier PASS/FAIL are WITHDRAWN (a scaffold supplied the
environment). See docs/tasks/20261005-retractions.md and .../entry-reconciliation.md.

### Product changes made this session (real, not test fakes)
- **T0 `20261005-130000` FIXED**: install now RECONCILES the source against the hub's loaded
  registry (`crates/plugins/src/service.rs` begin_install -> authorized(); `registry.rs`
  Registry::authorizes: url AND sha256). With no registry, nothing installs.
- `20261005-120000` FIXED: installing an id owned by a deployment dir -> 409 conflict, tree untouched.
- `20261005-140000`/`-141000` FIXED: a missing-id remove -> 404 not_found (not a false "deployment
  directory"); a body missing `source` -> 400 validation_failed (not 422 axum text).
- `20261005-110000` A FIXED (`RegistryUrlMissing` displays its payload); B: the contract code
  `registry_unavailable` (502) ADDED (owner-approved) and mapped.
- The registry ADDRESS is fixed at build time (release: the official URL, no env override; dev:
  `AGENT_HUB_REGISTRY_URL` override, compiled out of release).

### Second, independent blocker
The host's real provider (`~/.pi/agent/models.json` -> `192.168.31.29:8990`) answers 400 to its own
`/models` with the host token (verified outside the hub). Session/turn tests are blocked by it too
(`20261005-150000`).

### dsh
BLOCKED (upstream runtime cannot boot here). docs/tasks/dsh-blocked.md.

## Open issues (see docs/issues/INDEX.md)
- `20261005-130000` [T0] closes the injection hole; verify against the PUBLISHED registry.
- `20261005-100000` the registry is still a checked-in local file for tests (the harness now drives
  the hub's own refresh/catalog; the official release is what is missing).
- `20261005-150000` host provider 400; `20261004-060000/-070000` jouzu residual;
  `20261003-122000` contract close/readonly; the unratified `turn_end` clientMessageId edit.

## What is next (in order)
1. Publish the `registry` release of `sabishii-me/agent-hub` (the fixed asset name) and fix the host
   provider. Then run the chain per docs/tasks/20261005-post-publication-acceptance.md — through the
   hub's own mechanism, no test-side stand-in.
2. Only then can the 33 UNVERIFIED files be run and the plugin-driver capability be established.

## Do NOT
- do not publish the registry to make tests green; publication is for verifying the real chain;
- do not fabricate a registry, override a source, or use a test-side install to "prove" the chain;
- do not modify `contract/` without the owner's review; do not add `{id}` install without that review;
- do not treat PASS/FAIL counts as hub-capability acceptance.

# Test gap — registry + install combinations (owner request 2026-10-08)

## The rules (owner)
- EVERY test must be REAL. A mock replaces an EXTERNAL dependency only, and the name says so.
- A **dev** test carries `dev`; a **mock** test carries `mock` (the label is explicit).
- Registry coverage: inject a MOCK registry in TWO modes — **no-auth** and **token-auth** — and
  run the FULL set of install combinations under each.
- Cover: concurrent install / remove / mixed states; and whether the SSE stream LOSES events.
- The suite of install/SSE tests that existed is listed below; whatever is missing is added.

## The existing install/SSE tests (the "complete test" to restore)
| file | gets a plugin by | currently |
|---|---|---|
| tests/plugins/a-plugin-is-installed-listed-enabled-and-removed.py | a local plugin DIR (git source) | BLOCKED (require_registry_or_blocked) |
| tests/plugins/concurrent-install-and-remove.py | a local plugin DIR | BLOCKED |
| tests/plugins/concurrent-mixed-install-remove.py | two local plugin DIRs | BLOCKED |
| tests/plugins/a-hub-killed-mid-remove-finishes-the-removal.py | a local plugin DIR | BLOCKED |
| tests/contract/the-events-stream-delivers-plugin-changes.py | a local plugin DIR | BLOCKED |
| tests/lifecycle/the-whole-chain-plugin-to-session-to-tools-over-v1.py | a local plugin DIR | BLOCKED |
All use `require_registry_or_blocked()` and install from a **local dir**, never an artifact.

## The two install-source shapes (both must be covered)
1. **artifact**: `{artifact:{url, sha256, id, pluginType, version, size}}` - url+sha256 reconciled
   against the registry. Tested by `registry-injection-matrix.py` (no-auth mock).
2. **git**: `{url:<local path>, ref}` - a git clone; NOT a registry artifact, so it needs the
   registry to allow it. Today the hub REFUSES every git source (`NotInRegistry`). Whether a
   registry may authorize a git source is an OPEN policy question.

## The two MOCK registry modes (names must say MOCK)
- **mock-no-auth**: a loopback HTTP server serving a real built registry (real zips, real sha256).
  A dev hub reads it via `AGENT_HUB_REGISTRY_FILE`/`_URL`. Proven working (matrix cases 2/4/6/7).
- **mock-token-auth**: the same, but the server requires a token header. **NOT POSSIBLE TODAY**: the
  hub's `refresh_registry` sends NO Authorization header and there is NO token env var
  (`crates/plugins/src/service.rs`). This mode needs a decision:
  1. WHERE the token comes from (an env var like `AGENT_HUB_REGISTRY_TOKEN`? a URL credential?).
  2. Whether it is dev-only or also compiled into a release (a real registry may be private).

## Install combinations to cover under EACH mock mode
- install a listed plugin -> 202 -> ready
- install a NOT-listed plugin -> 403 plugin_not_in_registry
- install a git source -> (today 403) - policy open
- install the SAME id twice (replace) -> converges to ONE ready
- install N of the same id concurrently -> ONE ready, no torn dir
- install + remove concurrently (different ids) -> both terminal
- install -> remove -> install (sequence) -> converges, never stuck
- remove a non-existent id -> 404
- remove a deployment dir -> 409
- a bad/absent/token-protected/HTML-200 registry -> 502 registry_unavailable (NOT 403)

## SSE (does it lose events?)
- `GET /v1/events` (hub.plugins.changed) delivers every install/remove change, in order, with ids.
- a reconnecting subscriber with `Last-Event-ID` replays or is told to Resync - never silently
  drops. Covered by `the-events-stream-delivers-plugin-changes.py` (currently BLOCKED).

## What is MISSING and must be added
1. a MOCK registry the blocked install tests can use (no-auth), so they run instead of blocking;
2. the token-auth mode (BLOCKED on the decision above);
3. artifact-source coverage for the concurrency/SSE tests (they use a dir today);
4. the "name says mock/dev" labelling in the harness;
5. an SSE loss test under concurrent install+remove.

## Decisions needed BEFORE the token mode (do not implement on a guess)
- the token source and whether it exists in a release build.

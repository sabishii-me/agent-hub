# Review handoff

**Fixed SHA: `1abd5bf2ab8a19dc0e9804061233a0c42dcd4b79`**
(`origin/feat/hub-modular-redesign`, PR #12)

Previous reviewed base: `3b6730b3befece01777208638a587aa7f8ad785b`. The range
`3b6730b3..1abd5bf` closes F1-F5 of PROVIDER-TURN-REVIEW-3b6730b3 plus N4; see
`docs/tasks/review-3b6730b3.md`. This document states what changed, the evidence,
and the remaining surface, so a review does not re-derive any of it.

## What to read first

- `README.md` — the continuous main task, the current vertical chain, the authorization
  boundary. `AGENTS.md` — the working rules.
- `docs/ARCHITECTURE.md` — §17 the order of work (the plan of record), §18 status,
  §24 the remaining surface (machine-generated from the contract).
- `docs/tasks/*.md` — per-capability work records, each with its verified evidence.
- `docs/tasks/remaining-surface.md` — **per group, which work OUTSIDE the hub blocks each
  remaining route.** This is the answer to "why is X not done" and must not be re-guessed.

## How to reproduce

```
cargo build --workspace                 # expect ZERO warnings
AGENT_HUB_TEST_PLUGIN_DIR=<a real pi plugin dir> cargo test --workspace
```

The real-hub integration tests (`hub/tests/session_slice.rs`, `connection_slice.rs`,
`provider_secret.rs`) spawn the ACTUAL binary against a temp data dir and SKIP (not fail)
when the gate is absent. Measured at this SHA: **0 warnings; all test result sets ok; 5
real-hub session tests pass (≈202s); 0 failures.** They need a real pi plugin dir; at the
time of writing the fresh clone is `/e/AI/ideas/_sess2/plugins/pi`.

Concurrency (ADR-0009), MEASURED:
`node tests/concurrency/measure.mjs <base> <token> 200` -> 400 requests, 0 errors,
≈344ms. And a long plugin install answers `202` immediately while 15/15 `/status` polls
answer `200` (the TASK-036 freeze defect does not reproduce).

## What changed since ed896102 (by area)

**New real capabilities** (each verified live against `/v1`):
- `PATCH /v1/sessions/{id}` — mid-session config; policy knobs (`plan`/`review`) apply
  during a running turn, model/provider/preset/thinking require an idle turn (`409
  session_busy`); a title is renamed IN the harness; a provider switch runs resolver ->
  `credentials/grant` -> `config/set` with the applied identity CONFIRMED and persisted.
- `GET /v1/sessions/{id}/messages|stats|skills` — READ-THROUGH (a read starts the
  session's process if needed; nothing cached).
- `POST /v1/sessions/{id}/compact|fork` — compact reports the harness's own result;
  fork starts a NEW session from a completed-turn anchor (source untouched).
- create accepts `presetId` (confirmed `applied.preset`), `plan`/`review` (confirmed),
  and `additionalDirectories` (passed as `AGENT_HUB_ADDITIONAL_DIRS`).
- The connections domain: `GET/POST /v1/connections`, `PATCH/DELETE /v1/connections/{id}`
  (rows + OS-keychain credential + incarnation/revision guard); enabled connections
  reach a session's adapter as env vars.
- Plugin lifecycle: enable/disable DURABLE across restart; catalog (verbatim registry
  restatement with a `fault`), registry refresh (the only place the URL is contacted),
  icon.
- Harness extension selection: `PATCH /v1/harnesses/{id}/extensions` (durable; an
  unknown id is `validation_failed`, not `unsupported`).
- Provider `PATCH /v1/model-providers/{id}/models` (selection lives on the provider).
- Metadata: `GET /v1/surface` (routes + contract sha256 + events), `GET /v1/openapi.json`
  (byte-identical to `contract/openapi.json`), `POST /v1/shutdown`.

**Fixes:**
- A one-shot capability adapter (pi exits after `models/list` etc.) is now reusable:
  `RequestHandle` carries an `alive` flag; `ensure_started` respawns a dead handle.
  `Harnesses::gated` adds the `harnessId`/`known`/list-key envelope.
- A DEAD session adapter is no longer reported running: `Runtime::is_running` consults
  `is_alive`; `reopen` after the adapter dies restarts it.
- A missing `AGENT_HUB_TEST_PLUGIN_DIR` is a SKIP, not a panic.

## The remaining surface, and why

74 contract endpoints at this SHA: **56 real, 8 mounted-but-501, 10 not mounted**
(`ARCHITECTURE.md` §24). Every remaining route is blocked OUTSIDE the hub's own code
(`docs/tasks/remaining-surface.md`):

- **adapter protocol** (a separate project): `connections/*`, `auth/*`, `tools/list`,
  `session/repair`, `resources/*`, `approval_need`/`question_need`.
- **the provider-type data model**: `/v1/model-providers/types` and provider `auth/*`
  (no plugin ships a type descriptor).
- **the plugin-sourced skills model** (a decided correction, TASK-048 C2): the skills
  file routes stay `501`; re-enabling them would revert the approved correction.
- **artifact recording**: `GET /v1/sessions/{id}/artifacts` (install records no artifact).

## Authorization

No real credential was read, no apiKey imported, no real vendor call made. The one honest
gap on the vertical chain is a REAL model turn, which needs a credential authorization;
it blocks only that call, not the system implementation.

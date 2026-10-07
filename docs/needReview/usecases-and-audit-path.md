# User use cases, the test flow, and the audit path

This is the user's point of view (not the domain layout), the flow a test must follow, and the
order in which we audit. Nothing here is verified yet; it is the plan for the review.

## The user's use cases (what a person actually does, in order)

- **U1. Obtain the hub and run it.** `cargo build --workspace`; run the binary; it writes
  `endpoint.json` (loopback URL + bearer). The user (or their client) holds the token.
- **U2. Get plugins into the hub.** The user installs a plugin from the **official registry**
  (by naming an id/version the registry lists, or by giving a release url+sha256 the registry
  lists). Unlisted sources are refused by default; the user may FORCE an install in a known-unsafe
  state (`approve-unsafe`). Then prepare its runtime.
- **U3. Register a model provider.** The user creates a `model-provider` (endpoint + credential),
  fetches its catalog, chooses models.
- **U4. Start a session on a harness.** The user picks a harness (a plugin), optionally a
  provider/model, a preset/plan/review, and creates a session. The session is `active` only when a
  real adapter process is up and the config is confirmed.
- **U5. Send a turn and watch it.** The user sends a message; the hub streams events; the turn
  ends on the adapter's own terminal. Approvals/questions are answered through `/v1` while it runs.
- **U6. Manage the session.** messages/stats, cancel, compact, fork, close/reopen, repair.
- **U7. Tear down.** remove the plugin / delete the provider / shutdown.

## The test flow (one chain, in order)

Every test follows the SAME flow, and each step FAILS (never skips) if its real dependency is
missing:
1. build/run the hub (U1) — the binary must exist;
2. get ONE plugin in (U2) — through the hub's own registry mechanism or a fixture registry the hub
   reads; never a test-side copy;
3. a session on that plugin (U4) — through `/v1`;
4. one real turn (U5) — needs a provider credential; without it, the turn is expected to end
   `failed` with the adapter's reason (honest), and that is recorded, not faked;
5. teardown (U7).

## The audit path (what we review, in order)

- **A1. The API surface** — `api-check.md`: all 74 endpoints, mounted vs gated vs stale-doc.
- **A2. The plugin-enters-the-hub chain** (U2) — the registry/reconcile/`approve-unsafe` path.
- **A3. The session+turn chain** (U3–U5) — provider grant -> session start -> one turn.
- **A4. Session management** (U6) and **teardown** (U7).
- **A5. The architecture** (Pass 3) — after A1–A4, because the architecture should describe what
  actually holds.

## The FIRST chain to audit (the one we can test now)

**A2 — "a plugin enters the hub and is usable."** It is the lowest chain that exercises the real
product and needs the least external state:

1. the hub boots with a registry it reads (the fixture registry, or the official one when
   published);
2. `POST /v1/plugins` with a source the registry LISTS -> accepted (202), reaches `ready`;
3. `POST /v1/plugins` with a source the registry does NOT list -> refused 403
   `plugin_not_in_registry`, nothing lands;
4. `POST /v1/plugins` with the SAME unlisted source and the explicit unsafe approval -> installed
   (the settled `approve-unsafe` direction);
5. `POST /v1/plugins/{id}/prepare` -> runtime prepared (or an honest failure);
6. the plugin appears in `GET /v1/plugins` and drives a session (A3).

Why this one first: steps 1–3 exist as code today; step 4 is the settled-but-unwritten
`approve-unsafe`; step 5/6 are the bridge to A3. Auditing it now pins the policy contradictions
(direct address, registry file vs release) and produces the first honest end-to-end result.

## Blocked until decided (do not implement yet)
- the exact `approve-unsafe` contract field/header and its status code;
- the registry contradiction (checked-in file vs published release asset);
- which status table is authoritative.

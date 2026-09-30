# The provider→session grant chain

Status: **delivered** (TASK-048 REVIEW-495ce94).

A session may name a hub-managed `modelProviderId` (+ `modelId`). The hub resolves it
and, at start, runs:

1. `session/start` (the adapter spawns its harness);
2. `credentials/grant` `{connectionId, value, url, declarations}` — the credential
   comes from the OS keychain; the adapter materialises a `hub-<id>` provider and does
   not persist the value (memory-only);
3. `config/set` with `{modelProviderId: <id>, model?}`.

The adapter's `applied.modelProviderId` / model are the **proof**; they are stored as
the session's applied identity, never assumed from the request. A reopen **re-grants**
(a restarted adapter holds nothing). `sessions` does not depend on `providers`: the
composition root injects the resolver.

Verified live (mock provider): session accepted → active, adapter log shows
`provider="hub-mockp" model="deepseek-flash"`, endpoint received `POST /v1/chat/completions`.
The mock's reply shape was not a valid turn, so the turn ended `failed` with the
adapter's reason — honest, and not a fake `completed`.

## Review follow-up (TASK-048 REVIEW-ed896102)

- `config/set` sends the owning contract's `connectionId` selection (+ `model`).
- The adapter's `applied.modelProviderId` / `applied.connectionId` / `applied.model`
  are VERIFIED against the request; a mismatch or missing `applied` fails the start.
- The confirmed identity is persisted (`applied_provider`/`applied_route`/
  `applied_model`) and shown on the session view; a reopen re-grants and re-checks.
- Grant errors keep their contract code (`provider_unauthorized`, ...).

## Session presets (TASK-048)

`presetId` accepted -> first `config/set` (`config.presetId`) -> `applied.preset`
confirmed -> persisted (`preset_id`/`applied_preset`) and shown on the session view.
The adapter reads `<plugin>/presets/` via `AGENT_HUB_PRESETS_DIR` (declared
`presets` capability). Verified live: `presets/list` returns `standard` and
`heavy-review`; a valid preset applies (`appliedPreset=standard`); an unknown preset
is refused (`starting_failed`, adapter reason).

## PATCH /v1/sessions/{id} (the next capability)

Mid-session configuration. Serialized on the session lock with start/close/reopen
and turn admission. Two classes of knob (owning contract):

- **policy** (`plan`/`review`): allowed during a running turn; the adapter's
  `applied.plan`/`applied.review` is confirmed before the row is updated.
- **model/provider/preset/thinking**: require an idle turn (`409 session_busy` while
  one runs). A provider switch runs the SAME path as create/reopen: resolver ->
  `credentials/grant` -> `config/set`, with `applied.modelProviderId`/`applied.model`/
  `applied.connectionId` confirmed and persisted. An unconfirmed `thinkingLevel` is
  reported as null plus a warning, never the requested value.
- **title**: renamed IN the harness (`session/rename`); the session reports the title
  the harness ACCEPTED (a title the harness did not accept is an error).

Verified live: title rename (`title=Renamed Via Patch`); `plan`/`review` ->
`appliedPlan`/`appliedReview` true; a model change during a running turn ->
`409 session_busy`; a plan change during a running turn -> `200`; an unknown session
-> `unknown_session`. Real-hub gated test `a_session_patch_renames_and_sets_policy`.

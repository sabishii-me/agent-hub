# Review follow-up: PROVIDER-TURN-REVIEW-9647b0d

Accepted this round: pending ops are not overwritten; patch no longer swallows a secret
read error; the resolver shares the provider lock and refuses a pending transition; turn
admission and a config switch share the session lock; a missing native route is refused;
storage `internal_error` is preserved; the periodic timeout is wired; same-harness
placement writers are serialized.

## F1 — recovery no longer blesses a mismatched pairing (P1)

The journal now records the ROW STATE the operation intended to write
(`expected_incarnation`, `expected_revision`). `recover_one`:
- if the row STILL matches the intent, it confirms the credential and saves the ref;
- if the row CHANGED under the failed write (a URL+token patch that did not persist, a
  rebuild), it DROPS the credential and clears a stale reference rather than attaching a
  new token to an old/other row.
`create` REFUSES while a pending op exists for the id (a rebuild cannot leave a row the
journal still describes).

## F3 — the route must RELATE to the request; config failures fail closed (P1)

- `runtime`/`patch_session` require the native route to be non-empty AND to be the
  requested provider's route (`<id>` or its injected name `hub-<id>`, the contract's
  naming rule). A route pointing at a DIFFERENT provider is a mismatch.
- `patch_session`: a `credentials/grant` or `config/set` REQUEST error now quarantines
  the session instead of returning `?` — config may have applied before the response
  failed.
- `quarantine_session` inserts an **in-memory gate** FIRST (before attempting the stop),
  so even a failed stop or a failed DB write blocks new turns; `accept_turn` checks it.
  A reopen clears it.

## F4 — dispatch/cancel atomic, timeout confirms and targets (P1)

- DISPATCH is now an atomic DB claim (`claim_running`: `admitted` -> `running`, only if
  still `admitted`). `cancel_turn` sets `cancelling` from `admitted`/`running` in the
  same row, so claim and cancel are mutually exclusive: exactly one wins and a cancelled
  turn never dispatches.
- The timeout sweep takes the session lock, RE-CHECKS that the OLD turn is still the
  active one in the state it saw (a finished turn + a new turn is skipped, so it cannot
  kill the new turn's process), and ONLY releases occupancy after the stop is CONFIRMED:
  a failed stop keeps the turn held and retries next tick.

## F5 — adapter typed data kept (P2)

`StartError::Refused` carries the adapter's typed `data`; `code()` passes through the
adapter's code ONLY when it is one the contract declares (a closed set), else
`adapter_crash`.

## N4 — atomic placement publication

`install_for_harness` builds the new set in a staging dir and RENAMES it into place, so a
reader sees either the old complete tree or the new complete tree, never a half-copied
one. (The writer lock remains; this removes the half-copy a concurrent reader could see.)

## Verified

- `cargo build --workspace` zero warnings; `cargo test --workspace` all green (39 result
  sets).
- Live: a mock adapter answering `config/set` with `connectionId: hub-WRONG` for provider
  `m` -> the session ends `starting_failed` with a precise reason.

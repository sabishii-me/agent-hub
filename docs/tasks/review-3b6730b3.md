# Review follow-up: PROVIDER-TURN-REVIEW-3b6730b3

The reviewer accepted (kept closed): the recovery sweep keeps its journal on failure;
the refresh/selection lifecycle lock + incarnation guard; the connectionId wiring,
provider/model check and applied persistence; abort failure not releasing busy; normal
restart reconciliation, reopen DB-failure stop, and placement in the blocking pool.

## F1 — pending ownership can no longer be overwritten or cleared (P1)

- `begin_provider_op` now **REFUSES** (`Conflict`) when a pending op already exists
  for the provider; it no longer deletes+replaces. An unrelated write cannot overwrite
  an unresolved intent.
- `finish_provider_op_id` clears the EXACT op the caller began (by id), never "whatever
  is pending for this provider". The boot sweep finishes by the id it resolved.
- `create` with no token begins no op and **finishes nothing** (it used to call
  `finish_op` unconditionally).
- `patch` reads the ROW's reference (never a re-derived key), and a secret-read ERROR
  aborts the patch instead of silently clearing the reference; `set_selection` and
  `delete`/`logout` use the row's reference too.

## F2 — the resolver is now a consistent snapshot (P1)

`resolve_grant` is **async** and takes the provider's operation lock for the whole
read, and REFUSES (`revision_conflict`) while a transition is pending. Config and
credential are read as one snapshot, so an old endpoint can no longer be paired with a
new token. `sessions::resolve_grant` and the injected resolver became async; the
composition root returns a boxed future.

## F3 — native route confirmed, switch serialized with admission (P1)

- `runtime` now requires `applied.connectionId` (the resolved native route) to be
  PRESENT before a session starts; a missing route fails the start.
- `accept_turn` is async and takes the SAME session lock a PATCH holds across its
  grant/config, so a turn cannot be admitted during a config switch.
- `patch_session`: after `config/set` has run, a mismatch or a DB-save failure
  **stops the adapter and marks the session `needs-repair`** (`quarantine_session`),
  instead of leaving it active against its old applied identity.

## F4 — cancel/dispatch, ACK timeout, DB failure (P1)

- `cancel_turn` takes the session lock (the intent write is serialized with admission
  and the `running` commit) and FAILS if the durable `cancelling` write fails.
- `run_turn` re-reads the durable turn state AND the cancel set immediately before
  sending the prompt; a cancel that landed in between settles the turn `cancelled`
  WITHOUT dispatching.
- A core-side **cancel timeout**: `timeout_unconfirmed_cancels(30)` (spawned every 5s)
  STOPS the adapter and settles an abort-delivered-but-unconfirmed turn `interrupted`,
  so execution occupancy is released on a real action, not a hope.

## F5 — storage and adapter typed errors kept (P2)

- `internal_error` passes through the sessions allowlist (a storage failure is not a
  request-body error).
- `BusError::Rpc` carries the adapter's typed `data` (its contract code), so the
  identity is no longer dropped at the bus boundary.

## F4 refinement: an adapter ANSWER is not a transport error

`StartError::Refused` now carries the adapter's own RPC refusal (`code`+`message`),
distinct from a transport failure. `run_turn`:
- an adapter REFUSAL is authoritative (the execution did not run) -> settle;
- a TRANSPORT error with the process still alive proves nothing -> leave the turn
  `running` for the execution timeout, never fabricate a terminal.

The execution timeout (`timeout_unconfirmed_cancels(30, 1800)`) now covers BOTH an
unconfirmed cancel (30s) and a `running` turn whose prompt never returned (1800s): it
stops the adapter and settles `interrupted`. `running_at` records when a turn entered
`running`, so the age is known.

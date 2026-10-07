# Test findings — real runs only

Each entry is a REAL behaviour a real test observed. A finding is not "softened" into an
expectation; the test keeps failing until the product (or the contract) is corrected.

## F1 — a confirmed cancel settles the turn `failed`, not `cancelled`/`interrupted`
- Test: `tests/interrupt/a-cancelled-turn-releases-the-session.py` (RUN_REAL_PROVIDER=1).
- Real combo: pi plugin 0.1.9, pi runtime 1.0.0, provider HOME-JP-prod.
- Observed: turn running -> `/cancel` -> 200 (`cancelling`); the adapter log shows
  `<-pi response:abort` (the abort WAS delivered and answered); the turn ends **`failed`**.
- The hub maps the adapter's run-end `aborted` -> `cancelled`, `failed` -> `failed`
  (`sessions/service.rs` run_end_of). So the ADAPTER reported the aborted run as `failed`.
- Adapter logic (`pi-adapter.cjs`): `session/abort` sets `abortRequested=true`; at
  `agent_settled`, `abortRequested` -> `aborted`, else `lastStopReason==='error'` -> `failed`.
  A `turn_end` that arrives while `turnActive` is false clears `abortRequested`; a later
  `agent_settled` then classifies by `lastStopReason` and yields `failed`.
- Verdict: the adapter mis-classifies an aborted run. Owner: PLUGIN (`prts-harness-pi`).
  The hub side (bind the abort to the turn's process generation, bounded wait) is separate
  and already present; the terminal label is the adapter's to get right.

## F2 — `POST /v1/sessions/{id}/fork` rejects an empty body with 400
- Test: `tests/lifecycle/a-session-closes-reopens-and-forks-without-touching-its-source.py`.
- Observed: `POST .../fork` with NO body -> `400 Failed to parse the request body as JSON:
  EOF`. With `{}` it is 200.
- The contract's request is `{afterTurnId?: string?}` - an OPTIONAL body. A route whose body is
  optional must accept an absent body. Owner: HUB.

## F3 — the contract contradicts itself on the closed session status
- The `/v1/sessions/{id}/close` description says the session is kept with status='closed',
  but `defs.session.status` has NO `closed` - it is `readonly` ("readonly = closed"). A
  consumer reading the description would look for a status that cannot occur.
  Owner: CONTRACT.

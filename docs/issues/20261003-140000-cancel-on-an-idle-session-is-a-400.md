# 20261003-140000 — cancel on a session with no turns returns 400, contract says idempotent 200

Recorded: 2026-10-03. Found by: `tests/interrupt/cancel-is-idempotent-and-bounded.py`.
Owner: **HUB**. Status: recorded, NOT fixed.

## Repro (real)

A session is `active` with no turns yet. `POST /v1/sessions/{id}/cancel`:

```
400 {"error":"validation_failed","detail":"validation failed: the session has no turns"}
```

## Why it is a defect

The contract's `POST /v1/sessions/{id}/cancel`: "idempotent; terminal state returns current
state without error". Cancelling an idle session is a no-op that returns the current state
(200); a 400 `validation_failed` is neither idempotent nor "without error". A client that
cancels defensively (a stop button pressed when nothing runs) gets an error for a valid
request.

## Fix direction

When there is no active turn (or no turns at all), respond 200 with the current turn state
(idle), like any other idempotent terminal case. Only a genuinely malformed request is 400.

## Verify

`python tests/interrupt/cancel-is-idempotent-and-bounded.py` -> "cancel on an idle session is
clean (not 500)" (a 200) must pass.

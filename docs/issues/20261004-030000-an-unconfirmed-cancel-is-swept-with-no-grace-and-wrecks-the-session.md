# 20261004-030000 — an unconfirmed cancel is swept with ZERO grace, marking a healthy session `needs-repair`

Recorded: 2026-10-04. Found by: the adversarial probe (tests/adversarial) - cancel a running
turn, then IMMEDIATELY send a new turn. Owner: **HUB**. Status: recorded, NOT fixed.

## Repro (real, reproducible)

- combo: hub `ab2f1fa`, pi plugin 0.1.9, pi runtime `1.0.0`, provider HOME-JP-prod.
- a real turn is running; `POST .../cancel`; IMMEDIATELY `POST .../turns` (a new turn).
- the cancelled turn settles `ended/cancelled` (fast, normally).
- **but the session becomes `needs-repair`**, and every later turn is
  `400 validation_failed: the session is needs-repair`.
- the hub log shows: `settled unconfirmed cancels as interrupted timed_out=1`.

Reproduced ~1 in 6; the settle path is a race with the sweep.

## Root cause

The sweep `settle_unconfirmed_cancels` (crates/sessions/src/service.rs) selects EVERY turn in
`state = 'cancelling'` with NO minimum age:

```sql
SELECT id, session_id, state FROM turns WHERE ended IS NULL AND state = 'cancelling'
```

(`crates/db/src/sessions.rs` `unconfirmed_cancels`.)

The sweep runs every 5s. A NORMAL cancel sets the turn `cancelling`, then the adapter
confirms within tens of milliseconds and the turn ends. If the 5s tick lands INSIDE that
window, the sweep treats the in-flight cancel as "delivered but never confirmed": it STOPS
the adapter and marks the session `needs-repair`.

Removing the previous `30s` grace (this session, to delete the invented number) left the
window at ZERO, so a cancel that was ABOUT to be confirmed is now raced by the sweep. The
mechanism (a core cancel timeout, adapter-v1:387) is required; the GRACE must not be zero.

## Fix direction

Restore a MINIMUM OBSERVATION WINDOW for an unconfirmed cancel: the sweep considers a
`cancelling` turn only after it has been `cancelling` for a short, named, frozen interval
(the core's cancel timeout). The cancel REQUEST still returns at once (0s wait) - only the
sweep's judgement waits, so a normal confirmed cancel is never misjudged. The interval is a
design value, not zero; the earlier hardcoded 30s was the wrong value, not the wrong idea.

## Verify

`tests/adversarial/...` -> cancel + immediate resend never yields `needs-repair`; the session
is `active` and a later turn is accepted, repeatedly.

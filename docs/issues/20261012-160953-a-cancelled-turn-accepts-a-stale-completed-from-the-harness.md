# 20261012-160953 - a turn the hub CANCELLED accepts a stale `completed` from the harness

**State:** OPEN (defect in the hub). Found while adding interrupt case A4.
**Severity:** a turn the hub asked to abort is settled `completed` if the harness's `turn_end`
reports `state: ok`. The cancel is silently overridden by a stale terminal - the caller is told the
turn finished cleanly when it was, in fact, aborted.

## Reproduced (real, env-driven)

`tests/interrupt/a-stale-completed-from-an-aborted-turn-is-not-trusted.py` against a REAL adapter
(`tests/fixtures/misbehaving`, `MISBEHAVE=abort-stale-completed`) that speaks the real protocol and,
on `session/abort`, sends a `turn_end` claiming `state: ok` for the turn it was asked to stop:

    ok   the session is active
    ok   the turn is accepted
    ok   the turn is running before the abort
    ok   the cancel is accepted (not a 5xx)
    ok   the turn reaches a terminal
    FAIL an aborted turn's stale `completed` is NOT trusted - ended=completed cause=None
    [interrupt/stale-completed-after-abort] 7/8 ok, 1 failed

The turn is `cancelling` at the moment the harness reports `ok`; the hub maps it to `completed`.

## Root cause

`crates/sessions/src/service.rs` `terminal_of_event_state`:

    "ok" | "completed" => ("completed", None),
    "aborted"          => ("cancelled", None),

It maps the harness's reported state to a terminal WITHOUT consulting whether the hub had CANCELLED
this turn (the same `self.cancelled` set the fallback path already checks). So a turn the hub asked
to stop is reported `completed` on a stale `ok`.

## Contract basis

`contract/adapter-v1.json` invariants: the abort ack is not a stop proof; the turn's own `turn_end`
proves it stopped. That makes the harness authoritative for the terminal in the NORMAL case - but a
`completed` for a turn the hub CANCELLED contradicts the cancel the hub issued and the state it
recorded (`cancelling`). The hub must not publish a clean `completed` for a turn it asked to abort:
when the turn was cancelled, an `ok` from the harness is a stale snapshot, and the honest terminal is
`cancelled`/`interrupted`, never `completed`.

## Fix direction (for the record; not yet written)

In the terminal mapping, when the turn is in `self.cancelled` (the hub asked to stop it), an `ok`
must NOT become `completed`; it settles `cancelled` (the cancel the hub issued) or, if the harness's
own terminal says `aborted`, `cancelled` as today. The set already exists and is already consulted
on the "no terminal seen" path; the mapped-terminal path must consult it too.

## Blocks

Interrupt case A4 stays RED until this is fixed. It is the FIRST interrupt case that needed a
misbehaving harness (`docs/needReview/issues/20261003-123000-interrupt-coverage-is-incomplete.md`).

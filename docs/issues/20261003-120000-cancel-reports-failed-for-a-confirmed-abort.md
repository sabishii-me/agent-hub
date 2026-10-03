# 20261003-120000 — a CONFIRMED cancel settles the turn `failed`, not `cancelled`

Recorded: 2026-10-03. Found by: `tests/interrupt/a-cancelled-turn-releases-the-session.py`
(RUN_REAL_PROVIDER=1). Owner: **PLUGIN (`prts-harness-pi`)**. Status: recorded, NOT fixed.

## Repro (real)

- combo: hub `13668f3`, pi plugin 0.1.9, pi runtime `@earendil-works/pi-coding-agent@1.0.0`,
  provider HOME-JP-prod.
- a real turn is running -> `POST /v1/sessions/{id}/cancel` -> 200 (`cancelling`).
- the adapter log shows `<-pi response:abort` (the abort WAS delivered to pi and answered).
- the turn ends **`failed`**; the test expects `cancelled`/`interrupted`.

## Why

The hub maps the adapter's run-end `aborted` -> `cancelled`, `failed` -> `failed`
(`crates/sessions/src/service.rs`, `run_end_of`). So the ADAPTER reported `failed`.

`pi-adapter.cjs`: `session/abort` sets `abortRequested=true`; at `agent_settled`,
`abortRequested` -> `aborted`. But a `turn_end` that arrives while `turnActive` is false
clears `abortRequested` before `agent_settled`; `agent_settled` then classifies by
`lastStopReason==='error'` and yields `failed`.

## Fix direction

The adapter must keep the abort intent across a stray `turn_end` (do not clear
`abortRequested` on a `turn_end` that is not this turn's terminal), OR classify on the
abort flag at the ACTUAL settle. The hub side already binds the abort to the turn's process
generation with a bounded wait; the terminal LABEL is the adapter's to get right.

## Verify

`RUN_REAL_PROVIDER=1 python tests/interrupt/a-cancelled-turn-releases-the-session.py` ->
"a cancelled turn ends cancelled/interrupted" must pass.

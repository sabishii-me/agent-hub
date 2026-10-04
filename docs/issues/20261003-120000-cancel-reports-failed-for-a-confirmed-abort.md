# 20261003-120000 — a turn's terminal is read from the wrong place; every pi turn settles `failed`

Recorded: 2026-10-03. Rewritten 2026-10-03 with the VERIFIED root cause (the earlier
"abortRequested cleared by a stray turn_end / PLUGIN" diagnosis was WRONG; see "History").
Found by: `tests/interrupt/a-cancelled-turn-releases-the-session.py`. Owner: **HUB** (and a
contract-vocabulary mismatch, below).

## Symptom

Every turn driven against the REAL pi adapter settles `ended: failed` - not only a cancelled
one. Proven with a plain, uncancelled turn:

- combo: hub `1a2efbc`, pi plugin 0.1.9, pi runtime `@earendil-works/pi-coding-agent@1.0.0`,
  provider HOME-JP-prod.
- `POST .../turns` with `"Reply with just: ok"` -> the turn reaches `ended`, and
  `ended = failed`, `cause = None`. No cancel involved.

## Root cause (verified against the real wire)

The terminal STATE is read from the `session/prompt` RPC *result*, but the adapter reports the
terminal as a `turn_end` *event*.

- contract `adapter-v1.json`: `session/prompt` -> `"result": "turn-ended"` (a marker, not a
  state); the terminal is the EVENT `turn_end` with `field: "state: ok|aborted|failed"`.
- pi adapter `pi-adapter.cjs` (~1013): on settle it sends the event
  `{method:'event', params:{data:{type:'turn_end', status}}}` and answers the prompt RPC with
  `result: {}` (or an RPC error only when `status==='failed'`).
- hub `crates/sessions/src/service.rs` `run_turn` (~1749) settles the turn from the prompt
  RESULT via `run_end_of(&result)`. `run_end_of({})` hits the `_ => ("failed", ...)` arm.

So an empty prompt result (the normal pi answer) is mapped to `failed`. The `turn_end` event
the adapter DID send is not what settles the turn.

Observed order from the adapter's own `<-pi` trace (live): `turn_start`, three
`message_start/message_end`, `turn_end`, `agent_end`, `agent_settled` - all present; the
prompt RPC is answered `{}`; the hub settles `failed`.

## Second, smaller defect (contract vocabulary)

The contract's event field is `state: ok|aborted|failed`. The pi adapter emits
`status: completed|aborted|failed` (it uses `completed`, not `ok`). The hub's `run_end_of`
expects `ok`. So even reading the event, `completed` would not match. Either the adapter must
emit `ok`, or the contract must say `completed` - this is an owning-contract decision, not a
local patch.

## Fix direction

The hub must settle the turn from the `turn_end` EVENT (`state`), and the prompt RPC result
only means "the call returned". Reconcile the vocabulary (`ok` vs `completed`) at the owning
boundary (contract or adapter), not by adding a synonym in the hub. The abort flag in the
adapter is NOT the cause; do not change it for this.

## History (why the first diagnosis was wrong)

The first version blamed the pi adapter (`turn_end` while `!turnActive` clears
`abortRequested` -> `failed`). That was an ASSUMPTION, never verified. Instrumenting the real
wire showed the prompt RPC result is `{}` and the turn settles `failed` even WITHOUT a cancel
- so the abort flag is not involved. The adapter change was reverted; the file is back at
HEAD. This issue is HUB-owned.

## Verify

`python tests/model/a-real-model-answers-with-a-confirmed-identity.py` and
`python tests/interrupt/a-cancelled-turn-releases-the-session.py` -> a normal turn ends
`completed`; a cancelled turn ends `cancelled`/`interrupted`.

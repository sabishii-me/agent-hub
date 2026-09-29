# Next vertical capability: the turn lifecycle

A work record for the next real `/v1` capability, per the continuous-execution rule.
Not a new domain: it completes `sessions` (the turn is the session's work).

## Goal

`POST /v1/sessions/{id}/turns` drives a real turn: the hub admits the turn against
its session's adapter (`session/prompt`), the adapter's notifications become the
contract's events (`turn.*`, `message.*`), and the turn reaches a terminal state
(`ended: completed|cancelled|failed`). **No model request is required** to prove the
plumbing; the model call is whatever the adapter's runtime does and is not asserted.

## Scope (in)

- `POST /v1/sessions/{id}/turns`: a LONG command - `202 Accepted` + `Location`; the
  turn is admitted with a durable command identity (`idempotencyKey` in the body).
  Same key + same content = the original turn; same key + different content =
  `idempotency_conflict`; a different key while a turn runs = `session_busy` (never
  queued); an **unknown turn is never replayed**.
- The turn runs detached: the hub sends `session/prompt` to the session's adapter and
  forwards its notifications as events.
- `GET /v1/sessions/{id}/turns`: the thin per-turn entries (state + ended + cause).
- `POST /v1/sessions/{id}/cancel`: idempotent; sends `session/abort`; the terminal
  state is reached when the adapter confirms (an adapter ACK is NOT "stopped").
- The turn's terminal state is persisted; `turn.ended` is published.

## Scope (out)

- fork/compact/patch/messages/stats/skills/repair/resources: stay `501`.
- No new domain, no provider/secret expansion, no model request assertion.

## Dependencies

- The session slice (`5ea9bdc`): a session owns a real adapter process; the runtime
  exposes the bus so a turn can send `session/prompt` on the SAME process.
- The event bus and the `Accepted` transport helper (both exist).

## Dependency found during implementation

A **real model call** needs a provider credential the hub does not have in this
environment. The turn PLUMBING is complete and verified (admit -> `session/prompt`
on the session's process -> the adapter's own answer -> a terminal state); a turn
without a credential ends `failed` with the adapter's own reason ("No API key found
for the selected model"), which is honest. Asserting a completed model answer is
**not claimed** and needs a credential (a minimal secret store) as an explicit
prerequisite - the same conclusion the first slice reached. No fake provider or
plaintext token is used to cross it.

## Exit conditions

1. On the REAL hub: create -> active; `POST /turns` -> `202 + Location`; the turn
   appears in `GET /turns` and reaches `ended`; `turn.*`/`message.*` events are
   delivered on `/v1/events` (or the session stream).
2. Same `idempotencyKey` + same content returns the original turn; a new key while
   running is `session_busy`; an unknown turn is not replayed.
3. `cancel` is idempotent and reaches a terminal state only on the adapter's end.
4. A turn admitted to a session with no running process fails honestly (no fake
   `running`).
5. Not-wired routes stay `501`; workspace zero warnings; the real-hub integration
   test covers the slice (gated on the plugin dir).

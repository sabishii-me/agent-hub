# 20261003-160000 — a turn refused while one runs is 400 `validation_failed`, contract says 409 `session_busy`

Recorded: 2026-10-03. Found by: `tests/concurrency/a-second-turn-while-one-runs-is-refused.py`
(C3). Owner: **HUB**. Status: recorded, NOT fixed.

## Repro (real)

- combo: hub `1a2efbc`, pi plugin 0.1.9, pi runtime `1.0.0`, provider HOME-JP-prod.
- a real turn is running.
- `POST /v1/sessions/{id}/turns` with a NEW `idempotencyKey`:

```
400 {"error":"validation_failed","detail":"validation failed: the session has a running
turn; a new turn is refused, not queued"}
```

## Why it is a defect

The contract (`contract/v1.json`) states a deliberate second turn while one runs is refused
with **409 `session_busy`** (the same code every busy PATCH knob returns). The behaviour is
right (refused, not queued), but the code and status are wrong: `validation_failed`/400 tells a
client the REQUEST was malformed, when the session is simply busy - a client cannot
distinguish "fix your body" from "try later".

## Root cause (exact)

`crates/sessions/src/service.rs`, `admit_turn`:

```rust
agent_hub_db::TurnAdmission::Busy(_) => {
    return Err(SessionError::Validation(
        "the session has a running turn; a new turn is refused, not queued".into(),
    ));
}
```

`SessionError::Busy` already maps to `409 session_busy` (`service.rs` `Busy => "session_busy"`,
and the transport maps `session_busy` -> 409). The busy admission must use it.

## Fix direction

Return `SessionError::Busy` for `TurnAdmission::Busy` so the refusal is `409 session_busy`,
consistent with every other busy path. No behaviour change beyond the code/status.

## Verify

`python tests/concurrency/a-second-turn-while-one-runs-is-refused.py` -> the second turn is
`409` with `error == "session_busy"`, and no second turn row exists.

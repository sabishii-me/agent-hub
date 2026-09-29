# agent-hub-sessions

**Not wired.** A session is not handed to an adapter yet, so there is no execution boundary behind
`active` / `running` / `fork`. An earlier version answered those successes against a database row
alone (TASK-048 F01); that was a fake success. Every `/v1/sessions` route now answers
`501 not_implemented` until the adapter boundary is real.

The control-state types and the command-identity rules (a turn's `idempotencyKey`) remain as
groundwork; they are used once a session is genuinely started.

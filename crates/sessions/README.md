# agent-hub-sessions

The sessions domain (`ARCHITECTURE` §6): the hub's **control state** over sessions and
turns, wired to the frame. The hub owns session identity, the requested/applied model and
modes, the working directory, the status, and the **admission** of a turn; the harness owns
the conversation.

## Turn identity (ARCHITECTURE §11, R1)

A turn carries a logical command identity (`idempotencyKey`, contract-mandated):

- a retry with the **same** key returns the **same** turn - a lost `202` never starts a
  second turn;
- a **different** key while a turn runs is refused `409 session_busy`, not queued;
- closing is not deleting; a fork records the source session and the turn it forked after.

## Tests

`cargo test -p agent-hub-sessions` runs the HTTP lifecycle (create/get/patch/close/reopen/
delete), the turn-idempotency rules, fork, and validation.

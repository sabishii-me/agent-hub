# 20261009-100000 - the dsh adapter's turn_end omitted clientMessageId, so NO turn got a real terminal

**State:** FIXED (adapter). Owner: adapter (event payload).
**Severity:** every deepseek turn was settled by the hub's 5s "no terminal seen" FALLBACK, not by the
harness's own terminal: a normal turn was recorded `failed`, and a cancelled one `interrupted`. The
whole-chain test still passed because it asserts only that the turn REACHED a terminal, not that the
terminal matched the harness.

## Measured (dev, real dsh 0.2.0-rc.2, Windows)

A turn that the adapter reported `turn/end {reason:{kind:completed}}` was stored `ended: failed`. In a
cancel-then-send test, the post-cancel turn (`"say hi again"`) ended `failed` though dsh completed it.

Adapter traces (temporary) showed the adapter DID emit the terminal:

    FRAME turn/end active=true reason={"kind":"completed"}

but the stored turn was `failed`.

## Cause

`crates/sessions` keys the awaited terminal by `clientMessageId` (adapter-v1 `events.turn_end`
`requires: ["clientMessageId"]`): the pump delivers the state to the turn ONLY when it sees BOTH the
`clientMessageId` and the state. The dsh adapter emitted `{type:'turn_end', status}` and never sent
`clientMessageId`, so the pump's `if let (Some(cmid), Some(state))` never matched and the awaiting
turn timed out (5s) into the honest-but-wrong fallback (`failed`, or `interrupted` when cancelled).

## Fix (adapter)

Remember the in-flight turn's `clientMessageId` (it already IS the prompt's `requestId`) and echo it
on `turn_end`:

    currentTurnCid = params.clientMessageId;                    // at session/prompt
    emit({ type: 'turn_end', status: statusOf(reason), clientMessageId: currentTurnCid });
    currentTurnCid = null;

After the fix: turn -> `completed`; an aborted turn -> `cancelled` (not `interrupted`); cancel then
send -> the post-cancel turn `completed`.

## Follow-up (test honesty)

`tests/lifecycle/the-whole-chain-plugin-to-session-to-tools-over-v1.py` asserts `state == ended` only.
It should also pin the KIND (`ended == completed`) for the happy turn, so a fallback terminal can
never pass as success again.

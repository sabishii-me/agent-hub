# 20261004-040000 — a turn's terminal is stored per SESSION, so the NEXT turn can take it

Recorded: 2026-10-04. Found by: `tests/interrupt/send-immediately-after-cancel.py` (1 in 3).
Owner: **HUB** (+ a contract gap). Status: recorded, NOT fixed.

## Symptom

After a cancel, the NEXT turn on the same session sometimes settles `failed` (cause None),
while the same sequence succeeds most of the time.

Repro (real): cancel a running turn, send a new turn, let it run. ~1 in 3 the new turn ends
`failed` with no cause, though the model answered.

## Root cause

`crates/sessions/src/runtime.rs` records the adapter's `turn_end` terminal in a map keyed by
SESSION:

```rust
terminals: Arc<Mutex<HashMap<String, String>>>   // sid -> state
```

`crates/sessions/src/service.rs` `run_turn` reads it with `self.runtime.take_terminal(&session_id)`
(takes and REMOVES by sid). The contract's `turn_end` event carries NO turn id
(`adapter-v1.json` `events`: `{type: turn_end, field: state: ok|aborted|failed}` - sid and
state only), and the adapter sends it as `{sid, data:{type:'turn_end', status}}`.

So the terminal of the CANCELLED turn (which settles just before the next turn begins) can be
stored, then TAKEN by the NEXT turn's `run_turn`, which settles the new turn `failed`. The
terminal was never bound to a turn.

This was introduced by the F1 fix (reading the terminal from the event instead of the prompt
RPC result). The event source is right; the LACK OF A TURN ID is the hole.

## Fix direction (one fix, not a choice)

`turn_end` must carry the identity of the turn it ends. The prompt already names its turn
(`session/prompt` params include `clientMessageId`); the terminal event must echo it back, so
a terminal is bound to the exact turn instead of to the session. Any hub-only workaround
(clear the map at admit, order by arrival) is a clock/ordering guess and is rejected for the
same reason the sweep no longer uses a timer: the hub must key on IDENTITY, never on order.

Changes:
1. `contract/adapter-v1.json`: `turn_end` event gains a required `clientMessageId` field (the
   `clientMessageId` of the prompt it ends). Owning-contract change - owner review first.
2. the adapter emits `turn_end` with `clientMessageId` (pi: the prompt's clientMessageId).
3. the hub records and takes the terminal BY TURN ID (`clientMessageId`), not by session.

## Verify

`python tests/interrupt/send-immediately-after-cancel.py` -> the post-cancel turn ends
`completed`, repeatedly (10 runs at least), and the cancelled turn ends `cancelled`.

## FINAL root cause (2026-10-04, after instrumenting)

The terminal event and the prompt RPC result travel on the SAME adapter stdout, event first
(pi-adapter.cjs: the `turn_end` event is sent before the prompt waiter resolves). But the hub
consumes them on TWO tasks: the pump task INSERTs the terminal into the map, and `run_turn`
READS it with `take_terminal` right after the prompt RPC returns. There is NO ordering between
those two tasks, so `run_turn` often reads BEFORE the pump inserted -> no terminal for its turn
-> the fallback settles `failed`.

The turn-id binding (clientMessageId) is necessary and in place; the remaining defect is the
map being read across tasks instead of a per-turn WAIT. Any added logging changes the timing
and hides it, which is why it only shows without instrumentation.

Fix: the terminal must be DELIVERED to the awaiting turn (a per-turn oneshot the pump fires),
not polled from a shared map. No timer, no order guess.

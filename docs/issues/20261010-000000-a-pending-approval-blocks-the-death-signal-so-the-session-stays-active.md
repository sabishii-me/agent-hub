# 20261010-000000 - a pending approval blocks the pump, so a dead adapter leaves the session `active`

**State:** FIXED (verified). Owner: hub (`crates/sessions/src/runtime.rs` pump task).
**Severity:** if an approval (or question) is pending when the adapter dies, the pump task is BLOCKED
inside the reverse handler and never reaches the code that signals the adapter's death — so the
session keeps reporting `active` with no process behind it, forever (not just slowly), and no
reconciliation runs.

## Measured (dev, real pi adapter, Windows)

`tests/interrupt/an-approval-pending-when-the-adapter-dies-does-not-hang.py`:
- heavy-review preset; a write raises a REAL approval; the adapter is killed with the approval
  UNANSWERED.
- the TURN settles honestly (`failed`, not a clean `completed`) — good; the prompt's wait is bounded.
- but the SESSION never leaves `active` (polled 40s). The turn settling did not quarantine it.

## Cause

`crates/sessions/src/runtime.rs`, the pump task:

    tokio::spawn(async move {
        while let Some(n) = notifications.recv().await {
            if n.wants_reply() {
                let result = handler(pump_sid.clone(), method, n.params.clone()).await;  // <-- awaits the approval
                ...
            }
            ...
        }
        // ONLY HERE, after the loop ends, is the adapter's death signalled:
        let _ = pump_deaths.send((pump_sid.clone(), gen));
    });

The approval is a REVERSE REQUEST: the pump awaits the human decision (`await_approval`) inside the
loop. When the adapter dies during that await, `notifications.recv()` never returns `None` (the loop
never iterates again), so the pump never exits and **never sends the death**. `spawn_death_watcher`
therefore never reconciles the session, and it stays `active`.

A session death must NOT depend on the pump task being idle.

## Fix (designed)

Signal the death independently of the pump's control flow — e.g.:
- the pump should NOT block on a reverse handler: spawn it (`tokio::spawn` the handler + reply), so
  the loop keeps draining and reaches the death signal promptly; and/or
- the death should be observed by a dedicated watcher (the `Child`'s `wait`/`try_wait`), not inferred
  from the pump loop's exit.

Also: a pending approval whose adapter is gone must be FAILED (the waiter released), not left hanging.

## Fix (applied)

`crates/sessions/src/runtime.rs` pump: a reverse request (`approval_need`/question) is now answered in
a SPAWNED task, not inline. The pump loop keeps draining, so it reaches the loop-exit death signal
even while an approval waits for a human. Verified: `an-approval-pending-when-the-adapter-dies...`
8/8 on pi and jouzu, the session leaves `active` when the adapter dies mid-approval.

(deepseek cannot drive this case yet: it ships no approval-gating preset - see 20261010-001000.)

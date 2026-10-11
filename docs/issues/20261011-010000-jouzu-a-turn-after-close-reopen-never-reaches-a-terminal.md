# 20261011-010000 - jouzu: a turn after close+reopen never starts (pi accepts the prompt, no agent runs)

**State:** OPEN (root-caused to the jouzu/pi RESUME path). Owner: the jouzu adapter / its pinned pi
runtime.
**Severity:** after a session is CLOSED and REOPENED, the next turn is accepted and acknowledged but
NO agent turn runs, so its terminal never arrives: the turn stays `running` forever and the session's
occupancy is held. Blocks C4 and A9 on jouzu.

## Measured (dev, real jouzu adapter, Windows) - instrumented pi RPC trace

Two turns on the SAME session, with the raw pi-runtime RPC written to a trace file:

TURN 1 (fresh session) - WORKS:

    -> prompt
    response          <- pi acked the prompt
    agent_start       <- an agent STARTS
    turn_start
    message_start / message_end ...

TURN 2 (after CLOSE then REOPEN on the same ref) - HANGS:

    -> get_state / get_entries      (readiness + history)
    -> prompt
    extension_ui_request notify
    response                        <- pi acked the prompt
    (NO agent_start, NO turn_start, NO message_*)

So the resumed pi RUNTIME ANSWERS the prompt (the RPC returns) but never launches a turn. The hub
dispatched correctly and is waiting for a terminal that pi will never emit. Reproducible on two
consecutive runs; pi and deepseek are 14/14 on the same flow.

## What was ruled out

- It is NOT the hub's dispatch: the hub sends `session/prompt` and pi answers.
- It is NOT a missing `clientMessageId`: jouzu echoes it on `turn_end` already, and no `turn_end` is
  produced at all.
- A readiness wait was TRIED (`harnessUpProof()` before acking `session/start`, so the core cannot
  dispatch into a not-yet-ready harness). It did NOT fix it - after resume pi answers `get_state`
  (it is "up") yet still does not start the turn. Reverted.
- Startup UI (`setWidget`/`setStatus`) floods in after resume; those are ignored by the adapter and
  are not the blocker.

## Where to look next (the resume path)

`startPi(resumeRef)` spawns `jouzu pi --mode rpc --session <existing ref> --session-dir ...`. On a
NEW file it works; on an EXISTING ref the harness loads the session but the first prompt after spawn
does not run an agent. Candidates:
- the pinned pi's `--session <existing>` resume needs a different trigger (a warm-up, a
  `continue`/`resume` RPC, or waiting for a specific readiness event rather than any answer);
- or pi's rpc mode refuses to run a turn on a resumed session until some initialization completes.
This must be chased in the jouzu adapter's `startPi` / the pinned pi runtime, NOT in the hub.

## Disposition

Two tests stay RED on jouzu (real finding, not weakened): `a-late-event-from-an-old-turn...` (C4) and
`a-session-is-reusable-after-every-terminal-path` (A9). They must pass once the resume path is fixed.

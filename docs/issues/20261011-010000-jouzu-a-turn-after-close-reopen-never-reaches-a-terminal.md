# 20261011-010000 - jouzu: a turn after close+reopen never starts (pi accepts the prompt, no agent runs)

**State:** OPEN - RECORDED, DO NOT FIX NOW (blocked on the jouzu runtime upgrade).
**Owner:** the jouzu adapter's PINNED pi runtime (an OLD version).

**Owner's call (2026-10-11):** pi ITSELF does NOT hang - the same close+reopen+turn flow is 14/14 on
the pi plugin. jouzu runs an OLDER pinned pi, and jouzu is about to sync to the latest pi, so fixing
the resume behaviour here now would be thrown away. RECORD and WAIT for the jouzu upgrade. Do NOT
change the hub (verified not the hub) and do NOT work around it in the adapter.
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

## Control (owner asked): pi itself does NOT hang

`a-late-event-from-an-old-turn-never-reaches-the-new-process.py` on the pi plugin: **14/14**. The
same close->reopen->new-turn flow works. So the defect is specific to the pi version JOUZU pins, not
the hub and not pi today.

## Disposition

Two tests stay RED on jouzu (real finding, not weakened): `a-late-event-from-an-old-turn...` (C4) and
`a-session-is-reusable-after-every-terminal-path` (A9). They must pass once the resume path is fixed.

## Addendum (20261011, while adding P5): the SAME old-pi family, a second symptom

Building the P5 mid-turn test (a real HTTP proxy to the real provider that cuts the stream after
the first token) surfaced a second jouzu symptom with the same root:

- pi: 9/9, deepseek: 9/9 - the proxy cuts the stream mid-turn and the turn settles `failed`.
- jouzu: the turn fails BEFORE the cut. Tracing the proxy shows the real provider answers the
  runtime's `POST /v1/messages?beta=true` with **HTTP 400 Bad Request** (257 bytes). The proxy then
  forwards a short body and the connection ends, so the injected cut never fires and the turn ends
  `failed` for the WRONG reason (the runtime's request was rejected, not the injected fault). The
  test's `cut_done` assertion catches this correctly (jouzu 8/9).

This is the same OLD pinned pi runtime as the resume-hang: a model call from it is rejected by the
provider. It is NOT the hub and NOT the test proxy (the proxy forwards the full 75671-byte body
faithfully - verified). RECORD with the resume-hang; both should clear when jouzu syncs to the
latest pi.

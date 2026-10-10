# 20261011-010000 - jouzu: a turn after close+reopen never reaches a terminal (hangs `running`)

**State:** OPEN (reproducible). Owner: the jouzu adapter (and/or the resume path).
**Severity:** after a session is CLOSED and REOPENED, the next turn is accepted and admitted but its
terminal never arrives: the turn stays `running` forever and the session's occupancy stays held.

## Measured (dev, real jouzu adapter, Windows)

`tests/interrupt/a-late-event-from-an-old-turn-never-reaches-the-new-process.py` on jouzu:
- turn 1 runs, the session is CLOSED (turn 1 settles `failed`), then REOPENED (200, session
  `active`), then turn 2 is accepted (202) - and turn 2 never reaches `ended` (polled 100s+).
- reproducible: two consecutive runs, same result. On pi and deepseek the same flow is 14/14.

The hub log shows nothing after the reopen: the hub dispatched the prompt to the new adapter process
and no terminal came back.

## Why this matters

This is the C4 case (a NEW process on the same stored ref). pi and deepseek pass; jouzu does not. So
either (a) the jouzu adapter does not deliver a turn terminal after a resume, or (b) the hub's
post-reopen dispatch is wrong for jouzu. The `turn_end`/`clientMessageId` keying (20261009-100000)
is the likely place to look first: if the resumed jouzu adapter's turn terminal does not carry the
new turn's clientMessageId, the hub waits and never settles (the same class of bug already fixed for
deepseek).

## Disposition

The test is ADDED and stays RED on jouzu (a real finding, not weakened). It must pass once the
adapter/resume path is fixed. No fake harness is involved: a real adapter, a real close/reopen.

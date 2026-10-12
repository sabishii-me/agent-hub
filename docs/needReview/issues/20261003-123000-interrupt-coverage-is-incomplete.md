# 20261003-123000 — the interrupt test coverage is incomplete (a taxonomy)

Recorded: 2026-10-03. Updated: 2026-10-03 (active + passive faults, concurrency and
multi-turn mixing; the RUN_REAL_PROVIDER gate and the hardcoded 30/1800 sweep are gone, so
the timeout cases are now drivable). Owner: **HUB** (coverage + the defects it exposes).
Status: recorded; the layer is being filled in.

A real turn can be interrupted in many ways. There are TWO families, and each needs REAL
tests (no fake; a missing real dependency is a FAILURE, not a skip):

- **ACTIVE** — the CLIENT asks to stop: `POST /v1/sessions/{id}/cancel` (and close/repair,
  which stop the process). The hub performs a stop.
- **PASSIVE** — the world breaks the run with no client action: the hub process dies (power
  cut), the adapter process dies, the adapter's stdout closes while it keeps working, the
  provider/network fails mid-turn (an API error), or the provider hangs (no first token).

## ACTIVE (client-initiated stop)

| # | condition | test now |
|---|---|---|
| A1 | cancel CONFIRMED -> turn `cancelled`/`interrupted` | yes (exposed F1: pi reports `failed`) |
| A2 | cancel DELIVERED but NEVER confirmed (half-dead harness) | NO |
| A3 | cancel arrives BEFORE the prompt is sent | NO |
| A4 | cancel WHILE the prompt is running (a stale snapshot says completed) | NO |
| A5 | cancel TIMEOUT (unconfirmed) -> the sweep stops the adapter, settles `interrupted` | NO |
| A6 | repeated cancel is idempotent | yes (exposed F5: idle cancel = 400) |
| A7 | cancel on an IDLE session (no turn) is a clean 200 | yes (exposed F5) |
| A8 | close ACROSS an unconfirmed stop; reopen re-attaches | NO |
| A9 | the session is reusable after every terminal path | partial |
| A10 | a KNOWN terminal intent is not overwritten by the sweep/orphan | NO |

## PASSIVE (no client action)

| # | condition | test now |
|---|---|---|
| P1 | the HUB is killed (power cut) -> recovery | yes (`a-killed-hub-recovers-its-sessions.py`, 6/6) |
| P2 | the ADAPTER is killed mid-turn -> session state is honest | yes (`killing-the-adapter-leaves-an-honest-state.py`, 5/5) |
| P3 | the adapter closes stdout but keeps working (run_turn sees Closed) | NO |
| P4 | the adapter ANSWERS the prompt then dies (turn stuck `admitted`) | NO |
| P5 | the PROVIDER fails mid-turn (an API error / non-200) -> the turn ends honestly | NO |
| P6 | the PROVIDER hangs (no first token) -> the turn does not hang forever | NO |
| P7 | the NETWORK drops mid-turn (a killed/blackholed route) -> honest terminal | NO |
| P8 | the hub's DB write fails while settling a turn -> no dishonest state | NO |

## CONCURRENCY / MIXING (the cases a single-turn test cannot see)

| # | condition | test now |
|---|---|---|
| C1 | N sessions cancelled at once -> each settles, none blocks another | NO |
| C2 | cancel one session while ANOTHER runs a turn -> no cross-talk | NO |
| C3 | a SECOND turn is refused while one runs (`session_busy`), never queued | NO |
| C4 | two turns on the SAME session across a stop/reopen: a LATE event from the OLD turn
      never reaches the NEW process | NO |
| C5 | a stop (cancel/close/repair) DURING a concurrent turn admission -> one wins, no
      double-dispatch | NO |
| C6 | killing the adapter while two sessions use it -> both become honest, none wedged | NO |
| C7 | an approval/question pending when the adapter dies -> the wait does not hang | NO |

## What "real" means here

Every case is driven against the REAL hub binary and the REAL pi adapter; a case that needs
a misbehaving harness uses REAL fault injection on REAL processes (kill the child, close its
stdout, point the provider at a dead route) - the hub is NEVER bypassed and the adapter is
NEVER faked. A missing real dependency FAILS the test; it is never skipped and never
softened.

## Coverage by method (how each is produced)

- Hostile-input / ordering -> the REAL adapter, no auth.
- Process death (P1/P2/P3/P4, C6) -> kill/close the real child process.
- Provider faults (P5/P6/P7) -> a REAL provider entry whose endpoint is a controlled
  failure (a dead host / a route that errors), not a fake provider server.
- Timeout/sweep (A5/A10) -> the sweep is now uncoupled from a hardcoded wall clock; the test
  drives the real path and reads the resource.
- Live model (A1/A4, P5/P6) -> the real provider in `~/.pi/agent/models.json`.

## Order

1. A2..A10 (active, no auth) - the interrupt layer.
2. P3/P4/P8 (process fault, no auth).
3. C1..C7 (concurrency / mixing, no auth).
4. A1/A4 + P5/P6/P7 (live provider).

## Progress (updated 2026-10-10)

Covered by a real, env-driven test (green on pi/jouzu; deepseek where the harness allows):
A1, A3, A6, A7, A8, A10, P1, P2, P4, P5 (start-time), P6, P7, C1, C2, C3, C6, C7.

Covered since: A5 (`a-cancel-that-is-never-confirmed-is-swept.py`, 11/11 all three), C4
(`a-late-event-from-an-old-turn-never-reaches-the-new-process.py`: 14/14 pi, 14/14 deepseek; RED on
jouzu - a turn after close+reopen never reaches a terminal, `docs/issues/20261011-010000`), C5
(covered by `send-immediately-after-cancel.py`: a cancel in flight + an immediate turn -> defined
outcome, no double-dispatch, not wedged).

Covered since 2026-10-11: P5 mid-turn (a real stateful HTTP proxy cut, see above); P8 (a real
`BEGIN EXCLUSIVE` lock on the live store through the settle, boot-reconciled - see above).

Covered since: A9 (`a-session-is-reusable-after-every-terminal-path.py`: 13/13 pi,
13/13 deepseek; RED on jouzu at the needs-repair reopen turn, the same 20261011-010000 hang).

Still MISSING (with what each needs):
- ~~**A2**~~ DONE (`a-cancel-that-is-never-confirmed-by-a-live-harness-is-swept.py`): the `misbehaving`
  fixture ACKs the abort and never confirms, and stays ALIVE; the hub's cancel timeout settles the
  turn `interrupted`, never `completed` (10/10).
- ~~**A4**~~ DONE (`a-stale-completed-from-an-aborted-turn-is-not-trusted.py`): the fixture answers an
  abort with a stale `turn_end(state:ok)`; the hub must not trust it. **This EXPOSED A HUB DEFECT**
  (`docs/issues/20261012-160953`): a cancelled turn accepted the stale `completed`. FIXED in
  `terminal_of_event_state_for` (a cancelled turn's `ok` settles `interrupted`). 8/8.
- ~~**P3**~~ DONE (`an-adapter-that-closes-stdout-but-stays-alive-settles-honestly.py`): the fixture
  closes its own stdout and keeps working; the in-flight turn settles honestly (`failed`/`interrupted`),
  never `completed` (7/7).
- ~~**P5 mid-turn**~~ DONE (`the-provider-errors-mid-turn-and-the-turn-settles.py`, `tests/lib/httpfault.py`):
  a REAL HTTP proxy to the real provider cuts the `text/event-stream` after the first
  `content_block_delta` (sticky, so a retry cannot pass). pi 9/9, deepseek 9/9; jouzu 8/9 - its OLD
  pi runtime's own model call is answered 400 by the provider before the cut, so the injected fault
  never fires (the same old-pi family as `20261011-010000`; the test's `cut_done` assertion catches
  it honestly).
- ~~**P8**~~ DONE (`a-db-write-failure-while-settling-never-lies.py`): holds a REAL `BEGIN EXCLUSIVE`
  lock on the live `hub.sqlite` through the settle, so BOTH the terminal-intent write and the
  terminal write fail. Proves the hub never announces a terminal the store does not hold (API and
  store agree, never a false `completed`), and a boot reconciles the open turn `failed`. pi/deepseek/
  jouzu 7/7 each.
The misbehaving harness is a REAL adapter fixture (`E:/AI/ideas/prts-harness-misbehaving`) that speaks
the real protocol and violates it in ONE switchable way (`MISBEHAVE=abort-silent|abort-stale-completed|
close-stdout`). It is fault injection, never a fake provider. A2/A4/P3 are the only cases that needed
it, and they are now DONE. A4 exposed one real hub defect, fixed.
Harness limits found while filling: deepseek ships no approval-gating preset, so C7 cannot run on it
(`docs/issues/20261010-001000`).

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

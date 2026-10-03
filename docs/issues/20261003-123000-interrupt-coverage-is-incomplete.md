# 20261003-123000 — the interrupt test coverage is incomplete (a taxonomy)

Recorded: 2026-10-03. Owner: **HUB** (test coverage + the defects it exposes). Status:
recorded; the layer is being filled in.

A real turn can be interrupted in many ways; the history already named them. Today's suite
covers only 2 of the 14. Each needs a REAL test (no fake); the ones needing a live model run
only with RUN_REAL_PROVIDER=1.

| # | condition | history | test now |
|---|---|---|---|
| I1 | cancel CONFIRMED -> turn `cancelled`/`interrupted` | bb23f7d A3; 495ce94 | yes (exposed F1) |
| I2 | cancel DELIVERED but NEVER confirmed (half-dead harness) | 20260920-230000 root cause | NO |
| I3 | cancel arrives BEFORE the prompt is sent | 3b6730b3:65; 495ce94:28 | NO |
| I4 | cancel WHILE the prompt is running (stale snapshot says completed) | 495ce94:28 | NO |
| I5 | adapter ANSWERS the prompt then dies (turn stuck `admitted`) | 20260920-230000 | NO |
| I6 | adapter closes stdout but keeps working (run_turn sees Closed) | 3f7a2e6:43 | NO |
| I7 | the HUB is killed (power cut) -> recovery | this session | yes |
| I8 | the ADAPTER is killed mid-turn -> session state is honest | 3f7a2e6 S3 | **yes** (`killing-the-adapter-leaves-an-honest-state.py`, 5/5) |
| I9 | cancel TIMEOUT (unconfirmed) -> needs-repair, session not wedged | 20260920-230000 | NO |
| I10 | a KNOWN terminal intent is not overwritten by timeout/orphan | 3f7a2e6:53 | NO |
| I11 | close/reopen ACROSS an unconfirmed stop | 3f7a2e6 R2 | NO |
| I12 | repeated cancel is idempotent | contract | **yes** (exposed F5: idle cancel = 400) |
| I13 | a LATE event from an old turn never reaches a new process | bb23f7d A3/S2 | NO |
| I14 | the session is reusable after every terminal path | 20260920-230000 | partial |

## What "real" means here

Many of these need a harness that can be made to misbehave on purpose WITHOUT faking the
hub: e.g. I2/I5/I6 need an adapter whose child is killed or made silent at a precise moment.
That is a REAL fault injection on a REAL process - the hub is never bypassed. Where a
condition cannot be produced with the real adapters, the test says SKIP with the reason; it
does not fake the adapter and does not soften the assertion.

## Testability gap (blocks I2/I9/I10)

The sweep intervals are HARDCODED (`hub/src/main.rs:363` -> `timeout_unconfirmed_cancels(30,
1800)`), so the "cancel delivered but never confirmed -> bounded recovery" path cannot be
driven in a test in reasonable time. `AGENT_HUB_CONTROL_TIMEOUT_SECS` already exists as a
test knob for a control request; the sweep intervals need the same treatment (a knob, not a
functional change) before I2/I9/I10 can run REAL. Adding it is a HUB task; the test reads it.

## Order

1. I14/I12/I13 (no auth; lifecycle/events) — extend the interrupt layer.
2. I7/I8/I11 (process fault, no auth) — kill the hub or the adapter mid-life.
3. I9/I10 (timeout/recovery; a short AGENT_HUB timeout for the test) — needs a knob.
4. I1..I6 (need a live model) — RUN_REAL_PROVIDER=1.

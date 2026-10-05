# 20261004-060000 — jouzu adapter: three real gaps (pre-existing, NOT from the 0.1.18 upgrade)

Recorded: 2026-10-04. Found by the real suite run against the jouzu plugin
(`PI_PLUGIN_DIR=prts-harness-jouzu PI_HARNESS_ID=jouzu`) while upgrading jouzu 0.1.13 -> 0.1.18.
Owner: **PLUGIN** (`prts-harness-jouzu`) - but see UPDATE: NOT yet located to jouzu code.
Status: recorded; the label 'jouzu gaps' is UNPROVEN.

## Why this is recorded

During the jouzu 0.1.18 upgrade the suite ran 24/28 against jouzu. Four files failed. Each was
re-checked against the OLD 0.1.13 runtime and **fails there too** — so these are pre-existing
jouzu adapter gaps, NOT regressions from the upgrade. (The upgrade itself is clean: see the
upgrade record; approvals 8/8 + 2/2 + 9/9 pass on 0.1.18/pi 0.87.1.)

## The three gaps (real, reproducible; each also fails at 0.1.13)

1. **`lifecycle/a-session-closes-reopens-and-forks-without-touching-its-source.py`** — after
   close -> reopen -> `POST /v1/sessions/{id}/fork {}`, the adapter closes its stdout and the
   fork answers `502 adapter_crash` ("the session could not be started: adapter protocol: the
   adapter closed its stdout"). It is timing-sensitive: a fork from a plain `active` session
   succeeds, and a close->reopen->fork sometimes succeeds; the test's ordering hits it. The
   same test passes 10/10 against pi. Owner: the jouzu adapter's fork/re-attach path.

2. **`interrupt/send-immediately-after-cancel.py`** — the post-cancel turn settles
   `ended=failed` instead of `completed` (7/8; the turn is accepted and the session is not
   wedged, but a later clean turn still ends `failed`). Owner: the jouzu adapter's terminal
   for a turn after a cancel.

3. **`concurrency/cancelling-one-session-does-not-disturb-another.py`** — session A's turn is
   not admitted/`session B is active` fails while cancelling A (the test then trips on a
   missing `turn`). Owner: the jouzu adapter under concurrent cancel + start.

(A fourth, `model/a-real-model-answers-with-a-confirmed-identity.py`, failed once mid-suite but
passes standalone on 0.1.18 — treat as flake until it reproduces with the adapter stderr; not
recorded as a gap.)

## What is NOT the cause

- **Not the 0.1.18 upgrade**: all three fail identically at 0.1.13.
- **Not the hub**: the same tests pass 10/10 (fork) and 8/8 (model) against pi; the hub code is
  unchanged between the two runs.

## Fix direction

Adapter-owned: correct fork/re-attach after a reopen, the post-cancel turn terminal, and the
concurrent cancel+start path. Then re-run the three files against jouzu (must be green), and
the full suite against jouzu (target 28/28). Do NOT special-case them in the hub.

## UPDATE (2026-10-05): the failing set is now measured, and it is pre-existing

Ran the HARNESS-PARAMETERIZED tests (the ones that read `PI_HARNESS_ID`) against jouzu
(`PI_PLUGIN_DIR=.../prts-harness-jouzu PI_HARNESS_ID=jouzu`). The pi-hardcoded tests
(plugins/lifecycle, harnesses/routes) are NOT parameterized - running them with `PI_HARNESS_ID`
gives FALSE failures (they still expect the harness `pi`). Do not count those as jouzu gaps.

The parameterized set has **4 real failures** on jouzu:

| test | result | first failing assertion |
|---|---|---|
| lifecycle/...forks... | 6/7 | `fork is accepted` -> 502 adapter_crash (adapter closed stdout) |
| model/...identity | 6/7 | (standalone now) |
| interrupt/send-immediately-after-cancel | 7/8 | the post-cancel turn ends `failed` |
| concurrency/cross-talk | 1/3 | `session A is active` -> start does not reach active |

CONTROL (required before any cause claim): stashing the 20261005-010000 extension-delivery
change (jouzu back to `63d57d7`) and re-running gives the SAME results; cross-talk is 1/3 on
three runs both with and without the change. So these are NOT caused by that change - but
they are ALSO **not yet located to a jouzu code path**. They are four symptoms. Do NOT leave
this worded as 'three jouzu adapter gaps' until each is traced to a line of jouzu code (or
shown hub-side). The cross-talk and fork symptoms in particular need a trace of the jouzu
spawn/fork path, not an assumption.

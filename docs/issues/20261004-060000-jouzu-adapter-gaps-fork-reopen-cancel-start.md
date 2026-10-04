# 20261004-060000 — jouzu adapter: three real gaps (pre-existing, NOT from the 0.1.18 upgrade)

Recorded: 2026-10-04. Found by the real suite run against the jouzu plugin
(`PI_PLUGIN_DIR=prts-harness-jouzu PI_HARNESS_ID=jouzu`) while upgrading jouzu 0.1.13 -> 0.1.18.
Owner: **PLUGIN** (`prts-harness-jouzu`). Status: recorded, NOT fixed.

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

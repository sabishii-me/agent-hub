# 20261008-010000 — a mixed concurrent install/remove loses ONE plugin's SSE events

## Status
OPEN — reproduced 3/3. NOT yet root-caused (needs a controlled comparison, not a guess).

## Observed (dev + MOCK registry, real hub)
`tests/plugins/mock-concurrent-mixed-install-remove.py` runs >=6 mixed install/remove ops on TWO
plugins (pi, deepseek) concurrently, while a subscriber holds `GET /v1/events` open. The stream is
expected to name BOTH ids (hub.plugins.changed per id).
Measured, 3 runs:
- run 1: ids = {'pi'}        (deepseek's changes absent)
- run 2: ids = {'pi'}        (deepseek's changes absent)
- run 3: ids = {'deepseek'}  (pi's changes absent)
The other checks (no 5xx; both plugins converge to a terminal state) PASS every run. So the
resource lands correctly; the EVENT for one id does not arrive at the subscriber.

## Why it matters
The contract (ARCHITECTURE s11) requires that a subscriber RECEIVES the changes (no silent loss) and
that the beginning-GET/subscribe window is handled. A stable loss of one plugin's events is exactly
the "does SSE lose events" defect the owner asked to test for.

## NOT yet a diagnosis
Three candidates, none confirmed:
1. the hub's bus genuinely drops an id's events under concurrent emit;
2. the test's SSE consumer reads only part of the frames (a test-side buffer/timing defect);
3. the events fired BEFORE the subscription was live (the begin-GET/subscribe window) — a real
   window the contract says must not lose changes, but then it is a WINDOW defect, not a mid-stream
   loss.

## The controlled comparison needed (do NOT fix on a guess)
- subscribe, then WAIT until the stream is confirmed live (e.g. the stream's first frame / an
  ack), THEN emit a single install of one id; count the frames. Repeat with a single id and with
  two ids emitted sequentially (not concurrently). That isolates "concurrency" from "the window".
- print the hub-side emitted events (a temporary, reverted diagnostic) and diff against what the
  subscriber received.

## Evidence
`tests/plugins/mock-concurrent-mixed-install-remove.py` (dev + MOCK registry).

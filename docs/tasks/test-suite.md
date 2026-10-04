# Test suite — real, adversarial, layered (design of record)

Status: design of record, updated 2026-10-03 (after F1/F5; the RUN_REAL_PROVIDER gate and the
hardcoded 30/1800 sweep are gone). Language: **Python 3.13 (stdlib only)** — deliberately NOT
the hub's language (Rust), so a test cannot import hub internals or hand-build a fake. Runner:
`python tests/run.py [layer]`.

## The one rule

**No fake, anywhere, ever.** A step is either driven for REAL (real hub binary, real plugin,
real runtime, real provider, real fault injection) or the test **FAILS** with the missing
thing. A SKIP is NOT allowed: a capability that could not be exercised is not proven, so a
missing real dependency is a FAILURE (this replaced the old `RUN_REAL_PROVIDER` env gate,
which made the real tests not run by default and the suite look green while proving nothing).
Every case prints its version combo (hub SHA + plugin SHA + runtime version + provider +
platform + isolated data dir).

Why Python, not Rust: the hub is Rust; a Rust test can construct hub types directly and
"prove" a behaviour the wire never shows. A different language can only speak the real
surfaces (HTTP + SSE + child processes + filesystem).

## What "fake green" looked like (the review that caused this rewrite)

The suite passed 14/14 while real defects were present, because:

1. **An env gate hid the real tests.** `RUN_REAL_PROVIDER=1` guarded every model/tool/interrupt
   test; with no env they SKIPPED, so a default run proved nothing and printed green. REMOVED.
2. **Shape assertions instead of behaviour.** 17 of 120 checks only asserted a status threshold
   or a permissive tuple (`status < 300`, `in (200, 501)`, `in (200, 404)`) — they pass for a
   stub as readily as for the product.
3. **Whole behaviours had NO test.** The interrupt taxonomy (below) had 2 of ~20; concurrency
   mixing, provider failure, network drop, approval-while-dead had zero. "Green" was "we did
   not look".

The bar now: a check must assert an OBSERVED behaviour the wire shows (a persisted row, a real
model answer, a real tool result, an honest terminal state, a process that is really gone).

## Layout

```
tests/
  run.py                 # selector: all | a layer name; non-zero exit on any failure
  lib/hub.py             # spawn the real binary, read endpoint.json, HTTP, kill (power-cut)
  lib/tally.py           # check/require/done; require() FAILS on a missing real dependency
  contract/  lifecycle/  model/  tools/  interrupt/  approvals/
  concurrency/  provider/  skills/  connections/  harnesses/  plugins/
```

## Coverage today (honest, 2026-10-03)

Real, no env, all against the real hub + real pi adapter + real provider where a turn is
needed. **14 files, 120 checks.** Per file (what it proves):

| file | proves | result |
|---|---|---|
| contract/status-surface-openapi-and-models | status/surface/openapi/models answer real | 8/8 |
| contract/the-served-surface-equals-the-contract | served surface == contract (no extra/missing/stub) | 4/4 |
| lifecycle/session-closes-reopens-forks | create->close->reopen->fork; source untouched | 10/10 |
| lifecycle/session-crud-and-read-through | list/get/patch/turns/messages/stats/skills/artifacts/compact/repair-preview/delete | 19/19 |
| interrupt/cancel (real turn cancelled) | a real turn running, cancelled -> terminal, session reusable | 7/7 (fixed F1) |
| interrupt/kill-recover | hub killed (power cut) -> honest recovery | 6/6 |
| interrupt/cancel-idempotent | idle/repeated cancel is idempotent 200 | 5/5 (fixed F5) |
| interrupt/kill-adapter | adapter child killed -> honest state, turn settles | 5/5 |
| concurrency/one-hundred-connections | >=100 concurrent, none times out | 3/3 |
| provider/crud | types from a descriptor; unknown type refused before write; token never echoed | 13/13 |
| tools/real-call | the model REALLY runs a tool; the answer carries the file's secret | 6/6 |
| model/identity | real model answers; identity confirmed; turn 2 recalls turn 1 (4242) | 8/8 |
| skills/routes | the /v1/skills directory routes | 7/7 |
| connections/crud | connection CRUD; row gone after delete; token never echoed | 8/8 |
| concurrency/a-second-turn-while-one-runs-is-refused | a second turn while one runs is 409 session_busy, not queued | 5/5 (found 20261003-160000) |
| concurrency/cancelling-one-session-does-not-disturb-another | two turns at once; cancelling one leaves the other running | 9/9 |
| concurrency/many-sessions-cancel-at-once | N sessions cancelled together all settle, none wedged | 15/15 |
| interrupt/killing-the-adapter-mid-turn | adapter killed WHILE a real turn runs -> turn settles honestly | 8/8 |
| interrupt/a-provider-that-cannot-be-reached-is-refused-not-faked | an unreachable provider fails the START honestly, naming the real error | 5/5 |
| interrupt/a-provider-that-hangs-does-not-hang-the-turn | a hanging provider (black-hole route) does not freeze the hub or fake a session | 5/5 |
| interrupt/the-network-drops-mid-turn | provider reached through a REAL TCP forwarder; the forwarder is killed mid-turn -> honest terminal | 8/8 |

Total: 20 files. A missed real dependency FAILS (never skips): `tally.require`.

## The gap (why 14/14 is still too little) — the expansion

`docs/issues/20261003-123000-...` holds the full taxonomy. Nothing below has a test today.

### ACTIVE (client-initiated stop)
- A2 cancel DELIVERED but never confirmed (a half-dead harness) -> swept, not wedged
- A3 cancel BEFORE the prompt is sent -> not dispatched
- A4 cancel WHILE the prompt runs (stale snapshot says completed)
- A5 cancel timeout -> adapter stopped, turn `interrupted`, session `needs-repair`
- A6/A7 cancelled/closed/idle idempotency across every terminal path
- A8 close/reopen ACROSS an unconfirmed stop
- A9 reusable after EVERY terminal path
- A10 a KNOWN terminal is not overwritten by the sweep/orphan

### PASSIVE (no client action)
- P3 the adapter closes stdout but keeps working
- P4 the adapter answers the prompt then dies (turn stuck `admitted`)
- P5 the PROVIDER fails mid-turn (an API error/non-200) -> honest terminal
- P6 the PROVIDER hangs (no first token) -> the turn does not hang forever
- P7 the NETWORK drops mid-turn -> honest terminal
- P8 the hub's DB write fails while settling -> no dishonest state

### CONCURRENCY / MIXING (a single-turn test cannot see these)
- C1 N sessions cancelled at once -> each settles, none blocks another
- C2 cancel one session while ANOTHER runs -> no cross-talk
- C3 a SECOND turn while one runs -> `session_busy`, never queued
- C4 a LATE event from an OLD turn never reaches a NEW process
- C5 a stop during a concurrent admission -> one wins, no double-dispatch
- C6 killing the adapter with two sessions -> both honest, none wedged
- C7 an approval/question pending when the adapter dies -> no hang

### FAKE-GREEN TO REPLACE (shape -> behaviour)
- harnesses: `in (200,501)` -> assert the ACTUAL capability-gated answer per harness (pi has no
  `providers` -> `unsupported`; a capable route answers real data)
- lifecycle close/fork `in (200,202)` -> assert the RESULTING state (close -> the row's status;
  fork -> a distinct session whose history diverges)
- provider delete `in (200,202,204)` -> assert the row is GONE from a subsequent list
- plugins delete of a deployment dir `in (409,403)` -> assert 409 with the contract's code

## Order of execution

1. FAKE-GREEN replacements (cheap, no new deps).
2. ACTIVE A2..A10 (no auth; lifecycle/process).
3. PASSIVE P3/P4/P8 (process fault, no auth) then P5/P6/P7 (a controlled failing provider).
4. CONCURRENCY C1..C7 (no auth; real processes).
5. Any case needing a live model uses the real provider in `~/.pi/agent/models.json` (no gate).

## DEFERRED: skills (timestamped 2026-10-03)

skills is deferred pending the plugin-sourced model decision (a `skill` plugin kind, layered
selection, PUT/DELETE fate). Recorded in review-bb23f7d-findings.md A1.

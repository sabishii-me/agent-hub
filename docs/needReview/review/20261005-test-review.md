# Test review — every fake assertion, and the real bug it hid

Recorded 2026-10-05. Rule: the deliverable is the PRODUCT, not the tests. A test that passes
whatever happens is `print green`; it is reviewed here, the REAL behaviour it should assert is
stated, and where the product is wrong the bug goes to `docs/issues/` with a code snapshot so
each fix can be reconciled.

## How a test is judged

- A `t.check(ok, ...)` is only a REAL test if `ok` is a SPECIFIC fact the contract/design
  requires. `x is not None`, `x in (<many>)`, and truthiness of a variable that is always set are
  NOT tests: they accept the failure the test exists to catch.
- For each such assertion: state the contract/design requirement, what the product ACTUALLY does
  (measured), and the bug id if it is wrong.
- The fix updates the PRODUCT; the test then asserts the now-true fact. Never the reverse.

## Ledger

| # | test:line | fake assertion | required (source) | measured | verdict |
|---|---|---|---|---|---|
| 1 | interrupt/killing-the-adapter-leaves-an-honest-state.py:43 | `st is not None` ("hub still answers") | after the adapter child dies the session must NOT claim `active` (the file's own docstring) | status stays `active` | **BUG 20261005-020000** (pre-existing, control at `ab2f1fa^`) |
| 2 | interrupt/killing-the-adapter-leaves-an-honest-state.py:57 | `settled is not None` (any terminal) | a turn with a dead adapter must not be `completed` | `failed` (so passes, but the assertion allowed `completed`) | assertion tightened |
| 3 | interrupt/a-provider-that-hangs-does-not-hang-the-turn.py:49 | `final is not None` + `!= active` | TBD (measure) | TBD | pending |
| 4 | interrupt/a-killed-hub-recovers-its-sessions.py:46 | `st in ("active","needs-repair","readonly","starting_failed")` — accepts `active` | ARCHITECTURE §21 N2: after restart an `active` session with no process -> `needs-repair` | product CORRECT: `needs-repair` | FIXED: assertion now `== "needs-repair"`; passes |
| 5 | interrupt/a-cancelled-turn-releases-the-session.py:73 | `ended in ("cancelled","interrupted")` | a CONFIRMED cancel -> `cancelled` (contract/v1.json turn.ended; `interrupted` is the unconfirmed/restart case, §21 N2) | pi yields `cancelled` | FIXED: assertion now `== "cancelled"`; passes |
| 6 | interrupt/killing-the-adapter-during-a-cancel-settles-the-turn.py:43 | `ended in ("cancelled","interrupted","failed")` | a turn `cancelling` when the adapter died -> `interrupted` (ARCHITECTURE §21 N2) | not yet re-measured | FIXED assertion (`== "interrupted"`); verify pending (needs >10s) |
| 7 | interrupt/killing-the-adapter-mid-turn...:47 | `ended in (3)` | killed mid-turn, no cancel -> `failed` (§21 N2) | `failed` | FIXED `== "failed"` |
| 8 | interrupt/killing-the-adapter-mid-turn...:54 | `st in (...,"active")` | must not claim active | `needs-repair` (after the 020000 fix) | FIXED `!= "active"` |
| 9 | interrupt/the-network-drops...:63 | `ended in (3)` | dropped network, no cancel -> `failed` | verify pending (>10s) | FIXED `== "failed"` |
| 10 | interrupt/send-immediately-after-cancel.py:47 | `status in (4 codes)` | a DEFINED outcome, never 5xx (2xx or 4xx) | verify pending | FIXED: `< 500` + (2xx or 4xx) |
| 11 | interrupt/cancel-is-idempotent-and-bounded.py:39 | `status in (3 codes)` | not wedged = a defined, non-5xx outcome | verify pending | FIXED: `< 500` |
| 12 | concurrency/many-sessions-cancel-at-once.py:43 | `started >= 1` (of N) | N turns AT ONCE -> all N observed running | verify pending | FIXED `started == N` |
| 13 | presets/plan-mode-is-applied-and-reports.py:40 | `reported is True or warning` | pi ships the plan ext -> plan must be APPLIED | verify pending | FIXED `reported is True` |
| 14 | lifecycle/...forks...:56 | `src in ("active","closed")` | fork leaves the source UNCHANGED -> `active` (contract) | FIXED `== "active"` | fixed |
| 15 | model/...identity:48,67 | `a1 is not None` | a message must exist; identity checked separately (appliedProvider/Model) | NOT fake (None fails) | keep |
| 16 | plugins/...:42,51 | `in (409,403)` / `in (200,404)` | deployment-dir remove -> 409; the plugin SHIPS the icon -> 200 | FIXED `== 409` / `== 200` | fixed |
| 17 | presets/...:43 | `in (202,400,422)` | contract: accepted-then-fails OR refused-before; both branches handled | NOT fake | keep |
| 18 | provider/...:27,52 | `in (400,422)` / `in (200,202,204)` | contract permits both codes; delete allows several | NOT fake | keep |
| 19 | lifecycle/crud:63,67 | `in (200,409)` / `in (200,202,204)` | contract permits both; delete allows several | NOT fake | keep |
| 20 | connections/...:39 | `in (200,204,404)` | delete idempotent: contract permits these | NOT fake | keep |
| 21 | tools/...:64 | `any(n in (7 tools))` | a REAL tool ran (the model chooses which); any real tool is valid | NOT fake | keep |

## Rules for this review

- The snapshot of the code under test is taken at the moment the bug is recorded
  (`docs/review/snapshots/<issue-id>-<sha>.txt`), so the fix can be reconciled against it.
- A test is not "fixed" by relaxing it. If a required fact is not yet produced by the product,
  the bug is recorded and the test asserts the required fact (it goes RED until the product is
  fixed).

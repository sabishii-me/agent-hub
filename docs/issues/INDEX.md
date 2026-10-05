# Issue index (true status, 2026-10-05)

The `Status:` line in each file is the record; this table is the overview. A status is
`FIXED` only with a real run as evidence (recorded in the issue). `NOT FIXED` = open.

| id | owner | what | status | evidence / where |
|---|---|---|---|---|
| 20261003-120000 | HUB | cancel reported `failed` for a confirmed abort | **FIXED** | terminal from the `turn_end` event; `interrupt/*` green |
| 20261003-121000 | HUB | fork rejects an absent body | **FIXED** | `routes.rs` fork: `Option<Json<ForkRequest>>` + `unwrap_or_default()` |
| 20261003-122000 | CONTRACT | `close` says closed but the enum has `readonly` | OPEN (contract) | needs an owner/contract decision |
| 20261003-123000 | HUB | interrupt coverage incomplete | SUPERSEDED | the interrupt layer now exists (12 files) |
| 20261003-130000 | HUB | deployment-dir remove returns 500 | **FIXED** | `crates/plugins` -> 409 conflict |
| 20261003-140000 | HUB | cancel on an idle session is a 400 | **FIXED** | verified: idle cancel -> 200 idempotent |
| 20261003-150000 | HUB | T0: cancel/timeouts diverge from the design | SUPERSEDED | folded into 030000/040000 |
| 20261003-151000 | HUB | T0: audit my deviations | SUPERSEDED | a work record, not a live defect |
| 20261003-160000 | HUB | a busy turn admission is 400 not 409 | **FIXED** | `SessionError::Busy` -> 409 `session_busy` |
| 20261004-030000 | HUB | an unconfirmed cancel is swept with no grace | NOT FIXED | open |
| 20261004-040000 | HUB | a turn's terminal can be taken by the next turn | NOT FIXED | open |
| 20261004-050000 | HUB/PLUGIN | a selected preset is reported applied but not enforced | **FIXED** | verified `approvals/preset-gates` 8/8 (real gate) |
| 20261004-060000 | PLUGIN(jouzu) | jouzu failures - UNLOCATED | NOT FIXED | two causes located; A fixed on jouzu; B (jouzu runtime) open |
| 20261004-070000 | PLUGIN | `PATCH review:true` races the next turn | **FIXED on pi** | verified 3x 9/9 (was ~20% fail); jouzu residual open (see 060000) |
| 20261004-080000 | HUB(test) | the OS secret store looked unreachable | **RESOLVED** | it was the SUITE leaking credentials; cleanup fixed |
| 20261005-010000 | PLUGIN | the adapter puts extensions in the workspace | **RESOLVED** | pi `7b84cb0`, jouzu `c007b35` (hub-owned dir + discovery off) |
| 20261005-020000 | HUB | killing the adapter leaves the session `active` | **FIXED** | `crates/sessions` death watcher; verified idle+mid-turn |

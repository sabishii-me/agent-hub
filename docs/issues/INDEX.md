# Issue index (true status, 2026-10-05)

A status is FIXED only with a real run as evidence. `OPEN` = not fixed.

| id | owner | what | status | evidence |
|---|---|---|---|---|
| 20261003-120000 | HUB | cancel reported `failed` for a confirmed abort | FIXED | terminal from the `turn_end` event |
| 20261003-121000 | HUB | fork rejects an absent body | FIXED | `routes.rs` fork: `Option<Json<..>>` |
| 20261003-122000 | CONTRACT | `close` says closed but the enum has `readonly` | OPEN (contract) | needs an owner decision |
| 20261003-123000 | HUB | interrupt coverage incomplete | SUPERSEDED | the interrupt layer exists (12 files) |
| 20261003-130000 | HUB | deployment-dir remove returns 500 | FIXED | -> 409 conflict |
| 20261003-140000 | HUB | cancel on an idle session is a 400 | FIXED | idle cancel -> 200 |
| 20261003-150000 | HUB | T0: cancel/timeouts diverge from the design | SUPERSEDED | folded into 030000/040000 |
| 20261003-151000 | HUB | T0: audit my deviations | SUPERSEDED | a work record |
| 20261003-160000 | HUB | a busy turn admission is 400 not 409 | FIXED | `SessionError::Busy` -> 409 |
| 20261004-030000 | HUB | an unconfirmed cancel is swept with no grace | FIXED | the sweep skips a live adapter (liveness, not a clock) |
| 20261004-040000 | HUB | a turn's terminal can be taken by the next turn | FIXED | per-turn `clientMessageId` binding; send-after-cancel 9/9 |
| 20261004-050000 | HUB/PLUGIN | a selected preset is reported applied but not enforced | FIXED | preset-gates 8/8 |
| 20261004-060000 | PLUGIN(jouzu) | jouzu fork vs single-attachment | OPEN | cross-talk was MY isolation (retracted); fork: "already owned by another attachment" |
| 20261004-070000 | PLUGIN | `PATCH review:true` races the next turn | FIXED on pi; OPEN on jouzu | pi 3x 9/9; jouzu residual |
| 20261004-080000 | HUB(test) | the OS secret store looked unreachable | RESOLVED | the suite leaked credentials; cleanup fixed |
| 20261005-010000 | PLUGIN | the adapter puts extensions in the workspace | RESOLVED | hub-owned dir + discovery off (pi+jouzu) |
| 20261005-020000 | HUB | killing the adapter leaves the session `active` | FIXED | death watcher; verified idle+mid-turn |
| 20261005-030000 | PLUGIN(jouzu) | jouzu cannot start two sessions at once (profile lock) | FIXED | profile applied once; N=8 5x, N=32/48/100 ok |
| 20261005-040000 | HUB | the snapshot sweep deletes a snapshot in use | FIXED | swept by age; pi N=32 3x, N=100 all active |

## Still OPEN (real)

- **20261003-122000** (contract: `close`/`readonly`) - needs the OWNER's decision.
- **20261004-060000** (jouzu fork vs single-attachment) - the remaining jouzu item.
- **20261004-070000** jouzu residual (review race on jouzu).
- **Unratified contract edit**: `turn_end` now requires `clientMessageId` in
  `contract/adapter-v1.json`; added WITHOUT owner review (20261004-040000). Must be reviewed.

## Missing test coverage (recorded, not hidden)

- 20261004-030000: no current test drives the cancel/sweep race.
| 20261005-050000 | HUB | a plugin installed through /v1 is not usable until a restart | FIXED | install/remove re-scan; whole-chain 13/13 |
| 20261005-060000 | HUB | removing a prepared plugin fails and hangs at `removing` | FIXED | kill the cached adapter before remove; failed remove records `failed`; plugins 16/16 |
| 20261005-100000 | HUB | the plugin registry is a checked-in local file; the tests fabricate it | OPEN | the test reads registry.json and injects the url; refresh route untested |
| 20261005-110000 | HUB(contract) | registry/refresh reuses plugin_install_failed (false, retryable) | OPEN | needs a new contract code -> OWNER review |
| 20261005-100000 | HUB | the plugin registry is a checked-in local file; the tests fabricate it | OPEN | OWNER: registry = AGENT_HUB_REGISTRY_URL only; suite must install by id |
| 20261005-110000 | HUB | registry error surface is false (plugin_install_failed) + detail discarded the payload | A FIXED / B OPEN | B needs a contract code (registry_unavailable) -> OWNER review |
| 20261005-120000 | HUB | POST /v1/plugins HANGS for an id owned by a deployment dir | OPEN | needs 409 conflict; exact blocking call not yet located |
| **20261005-130000** | **HUB T0** | **install never consults the registry: with NO registry at all, any plugin still installs (reproduced)** | OPEN | url+sha256 must reconcile against the official registry; needs contract+product change |
| 20261005-140000 | HUB | DELETE a nonexistent id says it "is a deployment directory" (a false detail) | FIXED | begin_remove: no row + no dir -> 404 not_found; release test |
| 20261005-141000 | HUB | a body missing `source` returned 422 axum text, not a contract code | FIXED | install catches JsonRejection -> 400 validation_failed; release test |

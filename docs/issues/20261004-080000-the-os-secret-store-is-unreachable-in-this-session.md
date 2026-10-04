# 20261004-080000 — the OS secret store became unreachable mid-session, blocking every real-provider test

Recorded: 2026-10-04. Environment issue (HOST), not code. Status: recorded, NOT resolved.

## Symptom

`POST /v1/model-providers` now answers `501 {"error":"not_implemented","detail":"this hub has
no secret store; a credential is refused rather than stored in plaintext"}`, so
`register_provider` fails and every test that needs a REAL provider cannot run (session create
-> `provider_not_found`). The earlier runs the same day passed (28/28) using the same code.

## Evidence

- `cargo test -p agent-hub-secrets -- --nocapture` prints
  `SKIP: OS secret store unavailable on this host` and the round-trip test is filtered out —
  i.e. the store's own probe (`keyring::Entry::new` -> set/get/delete a throwaway entry) fails
  in this process.
- The Windows pieces are healthy: `Get-Service VaultSvc` = Running; `cmdkey /generic:... /pass:...`
  adds and deletes a credential successfully.
- The hub's `SecretStore::probe` refuses when the `keyring` crate's read-back/delete step fails;
  it does not fall back to plaintext (by design).

## What this is NOT

- Not a code change: `crates/secrets` and the current hub binary are unchanged since the runs
  that passed today.
- Not orphaned processes: 7 stray `agent-hub.exe` from earlier interrupted runs were killed; the
  501 persists with zero hub/node processes running.

## Impact

- The preset/approval and review-toggle verification for the in-flight adapter change
  (20261004-070000) cannot be run until the store is reachable. A missing real dependency FAILS
  (by rule); the tests are correct to fail rather than skip.

## Next

- Investigate why the `keyring`/Windows credential backend is unreachable from a freshly spawned
  shell in this session (it was reachable earlier). Likely a logon-session/security-context
  change, not the hub. Re-run `cargo test -p agent-hub-secrets -- --nocapture` as the probe:
  when it no longer prints SKIP, the real suite can run again.

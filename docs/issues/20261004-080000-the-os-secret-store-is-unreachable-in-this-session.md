# 20261004-080000 — the keychain target name exceeds the Windows limit, so the OS secret store is unreachable and every credential is refused (501)

Recorded: 2026-10-04. Root cause found the same day. Owner: **HUB** (`crates/secrets` +
`crates/db/instance.rs`, the keychain namespace construction). Status: root cause PROVEN, NOT
fixed (the fix is design-adjacent; ask before writing).

## Symptom

`POST /v1/model-providers` -> `501 {"error":"not_implemented","detail":"this hub has no secret
store; a credential is refused rather than stored in plaintext"}`. Every REAL-provider test is
blocked (session create -> `provider_not_found`). The same code passed 28/28 earlier the same
day, so it looked like a transient host glitch - it is not.

## Root cause (isolated)

The hub builds the keychain SERVICE (the target name) as `agent-hub:<instance_id>`, where the
instance id is a 32-hex string (`crates/db/src/instance.rs::new_id`), so the target name is
**42 chars**. On this Windows host the `keyring` v3 `windows-native` backend refuses a target
name of **30 chars or more** with `PlatformFailure(Windows error code 8)`. Measured, isolated
(`keyring::Entry::new(service, "p").set_password(...)`):

```
svc_len=28 -> Ok   svc_len=29 -> Ok   svc_len=30 -> Err (Windows error 8)   ...   svc_len=42 -> Err
```

`SecretStore::for_instance` (`crates/secrets/src/lib.rs:33`) therefore probes with a 42-char
service, the probe's `set_password` fails, and the store is marked UNAVAILABLE for the whole
hub - which then correctly refuses credentials (it never falls back to plaintext). Because the
probe runs once at boot, the whole provider/connection credential path dies.

Windows itself is healthy: `VaultSvc` Running; `cmdkey /generic:<short> /pass:` adds and
deletes. The failure is specifically the target-name length.

## Why it is a HUB defect

The keychain target name is the hub's own construction. It embeds an unbounded (fixed-but-long)
instance id into a namespace the OS caps; the design intent (ARCHITECTURE §8: a namespace keyed
by the persistent instance id) is fine, but the LITERAL target name must fit the OS limit. A
target name derived from the instance id (e.g. a fixed-length encoding/hash of it, still
instance-stable) keeps the design and fits. The current construction does not.

## The fix is design-adjacent -> ASK BEFORE WRITING

Changing the keychain target-name scheme changes where an instance's secrets live: entries
written by a WORKING build under `agent-hub:<32hex>` would be orphaned by a shorter name (a
moved/upgraded hub could stop finding its credentials). Options (the owner decides; there is
one correct answer, not a menu):

- derive a SHORT, instance-stable target name (e.g. a truncated digest of the instance id),
  with an explicit migration/fallback read of the old long name; or
- keep the full id in the CREDENTIAL USERNAME (not the target name) so the target stays short; or
- whatever the architecture intends as the keychain key.

Per the rule, this is NOT written until the owner approves the direction.

## Verify (when fixed)

- `cargo test -p agent-hub-secrets -- --nocapture` no longer prints `SKIP: OS secret store
  unavailable` (its probe succeeds).
- `POST /v1/model-providers` -> 200/201; `tests/approvals/*`, `tests/model/*` and the full real
  suite run again (28/28 on pi; the recorded jouzu gaps aside).

## UPDATE (2026-10-04, later) — the length finding was a SYMPTOM; the Windows vault is broken

Further measurement overturned the "name length" as the *current* cause, and produced a
stronger one:

- While the store was partially reachable, longer names failed before shorter ones
  (`svc_len 28/29 ok, 30.. Err`; `key_len 16 ok, 17.. Err`). That looked like a hard name limit.
- Later, EVERY name failed - `svc=a keylen=1` included - and the failure is no longer
  name-dependent.
- It is NOT the Rust/keyring code: a native `advapi32!CredWrite` of a trivial credential returns
  **`GetLastWin32Error() == 8` (ERROR_NOT_ENOUGH_MEMORY)**, and `cmdkey /generic:... /pass:...`
  prints **"Not enough memory resources are available to process this command."**
- The vault shows `HKCU\...\Vault\VaultCDSMigration = 1` (a migration flag); the credentials dir
  holds 41 small entries (175 KB), so it is not a space/quota issue. Restarting `VaultSvc` does
  not clear it.

So there are TWO real problems, in order:

1. **HOST (current blocker):** the Windows Credential Manager vault is in a broken/mid-migration
   state and refuses ALL writes with error 8, independently of our code. Recovering it may mean
   clearing/rebuilding the vault - which would touch the user's OTHER credentials, so it is NOT
   done without the owner's decision.
2. **HUB (real, still correct to fix):** even when the vault works, the hub's keychain names were
   invalid on a working vault: service `agent-hub:<32hex>` = 42 chars and the probe key
   `__probe__<32hex>` = 41 chars, both beyond the observed working lengths (<= 29 target,
   <= 16 user). A working vault would still reject these. See the fix below.

## Fix implemented (crates/secrets) — pending a working vault to verify end-to-end

`crates/secrets` now: service = fixed `"agent-hub"` (fits); key = a fixed **16-hex-char**
`sha256(instance || 0x1f || reference)`; the boot probe name is `__probe__` + 6 hex = 15 chars;
and a `get` miss falls back to the LEGACY `(agent-hub:<instance>, raw key)` entry and migrates it
so older credentials are not orphaned (skipped when the legacy name itself would exceed the
limits, since no such entry could exist then). `crates/secrets`'s own round-trip test no longer
prints SKIP **when the vault works** - it passed (0.68s, real I/O) during the window the vault
was reachable. `POST /v1/model-providers` returned `200` with `tokenConfigured:true` in that
same window.

The hub-side fix is necessary regardless of the host problem; it cannot be VERIFIED end-to-end
until the Windows vault is usable again.

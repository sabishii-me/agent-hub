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

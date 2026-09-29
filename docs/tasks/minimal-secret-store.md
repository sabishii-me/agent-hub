# Next vertical capability: a minimal secret store

A work record for the next real capability (the continuous-execution rule).

## Goal

Store and read **model-provider credentials in the OS secret store** (the `keyring`
crate chosen in ARCHITECTURE §2), so a credential is never written in plaintext and a
provider's auth path becomes real. This unlocks the credential dependency the session
and turn slices recorded (a real model call needs a credential).

## Scope (in)

- One secret per provider, keyed by provider id, in the OS keychain (Windows
  Credential Manager / macOS Keychain / Secret Service). No plaintext file.
- The provider record gains **no** credential field (already true): the credential is
  only ever in the keychain; `tokenConfigured` reflects presence.
- The provider CRUD routes that were `501` become real for the **credential path**:
  accepting a `token` stores it in the keychain; `logout` deletes it; a provider's
  `tokenConfigured` reports it. The catalog fetch uses it when present.
- A capability **probe** at boot records whether the OS store is reachable; if it is
  not, the credential path stays `501 not_implemented` (honest), never a plaintext
  fallback.

## Scope (out)

- No OAuth/device-code flow, no provider protocol dialects beyond the existing fetch,
  no multi-platform CI, no migration of any existing plaintext (none exists).
- Not the whole provider data layer; only the credential path.

## Dependencies

- The provider domain (`crates/providers`) and its routes are present (all `501` now).
- `keyring` must work on this platform; a probe decides before the route is enabled.

## Exit conditions

1. A probe stores/reads/removes a test secret in the OS store; if it fails, the
   credential routes stay `501` and this is recorded (not faked).
2. `POST /v1/model-providers` with a `token` stores it in the OS store; the provider
   file contains no credential; `tokenConfigured` is true.
3. `POST /v1/model-providers/{id}/logout` deletes the stored secret.
4. The catalog fetch authenticates with the stored credential when present.
5. On the REAL hub: create a provider with a token, read it back, logout; verify no
   plaintext on disk and the keychain holds it. No model call is required.
6. Workspace zero warnings; the real-hub integration test covers the credential path
   (gated on the OS store being available).

## Review follow-up (TASK-048 PROVIDER-TURN-REVIEW-6d495c3)

- **Data boundary**: the provider store is now the **database**, not `{id}.json` files (P1
  regression closed).
- **Instance namespace**: the secret store service is `agent-hub:<instance>` (instance = a hash of
  the data dir); the probe uses a unique name and confirms the delete (P1).
- **No side effect on refusal**: `create` inserts the row first (a duplicate is `already_exists`
  with no secret written), then the secret; a failed secret write rolls the row back (P1).
- **Honest configured state**: `tokenConfigured` is derived from the store on GET/list, and an
  unreadable credential is an error, never read as "not configured" (P2).
- **Delete removes the credential** with the row (P1).

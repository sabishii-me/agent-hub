# The connections domain (hub-managed)

The next domain per ARCHITECTURE §17 (after providers). A connection is a
hub-managed object: `{id, name, scheme, endpoint, envName, state,
credentialConfigured}`. It is **enable/disable/delete only**; `disabled` means
cut-off, which means **zero materialization** (its credential is handed to no
session).

## What is real

- Rows in the database (`connections` table, with an `incarnation`/`revision` guard:
  a stale save is refused, and one lock per connection id serialises the two-store
  operation).
- The credential is in the **OS secret store** as a per-instance reference
  (`<instance>:connection-<id>`), never a value in a row or a file. `create` writes
  the row first (a duplicate is `already_exists` with no credential side effect),
  then the secret; a failed secret write removes the credential. `delete` removes the
  credential AND the row. `credentialConfigured` is read from the store using the
  row's OWN reference (a rebuilt id cannot re-acquire an old credential).
- `materialize()` returns `(envName, value)` for ENABLED connections only; a disabled
  one, or one with no envName or no credential, contributes nothing.
- Routes: `GET/POST /v1/connections`, `PATCH/DELETE /v1/connections/{id}`.

## Verified live

create (credential-free view, `credentialConfigured:true`), list, disable, a bogus
state -> `validation_failed`, a duplicate -> `already_exists`, delete, a second delete
-> `connection_not_found`; no plaintext credential on disk. Real-hub gated test
`connection_slice`. Domain unit tests: no-store refuses a token; disabled
materialises nothing; a duplicate is `already_exists`.

## Still not built

The **harness-private** connections (`/v1/harnesses/{id}/connections*`, forwarded to
the adapter's `connections/*` methods) and handing a materialized connection to a
session's adapter via `HarnessEnv.connection_env` (which exists but is still empty).

## Handing a connection to a session's adapter

A session's adapter receives every ENABLED connection's credential as an environment
variable named by the connection's `envName` (never in the config payload or the
conversation). `Sessions::with_connection_resolver` (the composition root injects
`Connections::materialize()`) resolves them; a disabled connection, or one with no
envName or no credential, contributes NOTHING (zero materialization). Verified by
unit tests on `build_env` (the credential lands as the named var; no connection means
no var) and on `materialize` (disabled => empty). Live: a connection created, a
session started active with the resolver wired.

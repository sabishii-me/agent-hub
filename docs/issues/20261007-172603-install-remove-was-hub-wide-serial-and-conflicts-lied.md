# 20261007-172603 - install/remove serialized the whole hub; conflicting commands on the SAME plugin answered 404/502, not 409

**State:** FIXED (product change, verified)  
**Severity:** T0 - the concurrency the contract promised did not exist, and the refusal codes lied.

## What was wrong

`crates/plugins/src/service.rs` held **one global** `op_lock` and staged every install into
**one shared** `.stage/install`. Consequences, measured with ops issued in one ~1 ms window:

| case | observed (before) | contract |
|---|---|---|
| install pi + install deepseek (different ids, concurrent) | `409`, `409` - refused each other | different plugins are independent; both `202` |
| remove pi WHILE pi installs (same id) | `404` ("not found") | `409 conflict` |
| install pi WHILE pi installs (same id) | `502` (staging collision) | `409 conflict` |

The two `409`s on different ids are why a mixed concurrent install/remove never had BOTH plugins
change state, so `hub.plugins.changed` named only ONE id - long mis-reported as "the change stream
loses one plugin's events" (20261008-010000). The cause was NOT a lost event; the operation never
happened.

## The fix

- the lock and the staging dir are **per plugin id** (`op_locks: HashMap<id, Arc<Mutex>>`,
  `staging_dir(id) = .stage/<id>`); the global `op_lock` is removed. **Different ids run
  concurrently.**
- `begin_install` / `begin_remove` first check the id's in-flight ops (`busy(id)`); a conflicting
  command on the **same** plugin is refused **`409 conflict`** (`PluginError::InProgress` -> code
  `conflict`, distinct from `idempotency_conflict`), BEFORE any lock or staging.

## Verified (dev, `tests/plugins/mock/concurrent-conflicting-commands.py` TC-PLUGIN-CONFLICT)

- remove while the same plugin installs -> `409 {"error":"conflict"}`
- second install while the same plugin installs -> `409 {"error":"conflict"}`
- different plugins concurrently -> `202`, `202`
- `tests/plugins/mock/concurrent-mixed-install-remove.py` -> 5/5, including "the change stream
  named EXACTLY the two plugin ids" (the assertion was NOT changed).

The rule, stated by the owner: **while a plugin is installing it must not be installable again and
must not be removable by itself; different plugins install concurrently.**

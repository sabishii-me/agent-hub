# Plugin / harness lifecycle: enable and disable

`POST /v1/plugins/{id}/enable` and `.../disable` set a plugin's lifecycle status.
For a harness (a plugin is a harness too) this is `defs.harness.status`
(`enabled|disabled`), and it is **durable**: a `harness_status` table records it, the
adapter scan RE-APPLIES it on boot (a rescan never silently re-enables a disabled
harness), and `set_status` persists it.

A disabled harness is refused at **session create** (`harness_exists` returns false
for a disabled harness -> `harness_not_found`) and at the detached **start**
(`resolve` errors). Verified live: disable pi -> `GET /v1/harnesses` shows
`disabled`, create is refused; a RESTART keeps `disabled`; re-enable restores it.
DB test: the status survives a reopen. The plugin routes reach the adapter registry
through `PluginsState.adapters`, so `plugins` does not implement the lifecycle itself.

## Still not built

`GET /v1/plugins/catalog`, `POST /v1/plugins/registry/refresh`, and
`GET /v1/plugins/{id}/icon/{variant}` (the registry-file catalog and the manifest's
icon delivery) are not mounted yet.

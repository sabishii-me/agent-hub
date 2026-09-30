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

## The catalog, registry refresh and icon

- `GET /v1/plugins/catalog` — the registry file (`AGENT_HUB_REGISTRY_FILE`, else
  `<DATA_DIR>/registry.json`) restated VERBATIM; the hub does not resolve, rank or
  rewrite it. A missing/invalid file is a `fault`, never a 500.
- `POST /v1/plugins/registry/refresh` — read `AGENT_HUB_REGISTRY_URL` and write it
  where the hub reads the catalog. The URL is contacted ONLY here (never at startup,
  never silently).
- `GET /v1/plugins/{id}/icon/{variant}` — serve one variant (light|dark) of a plugin's
  own icon from the file its manifest declares (`icons: {light, dark}`); the hub serves
  the bytes and never inlines them; a traversal outside the plugin is refused.

Verified live: catalog over a seeded registry (6 plugins, no fault); a missing file ->
`fault`; icon/light -> `200 image/svg+xml` with the real bytes; a bad variant ->
`not_found`; refresh with no URL -> an honest refusal.

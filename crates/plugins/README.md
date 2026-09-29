# agent-hub-plugins

The plugins domain (`ARCHITECTURE` §10, task T3): install, remove, prepare, and the
listing, wired to the frame.

- `state.rs` - the hub's **one verdict** on a plugin. Concurrent operations are ranked
  (`removing > installing > preparing > failed`), so a plugin is never reported in two
  states; `failed` recorded on disk outlives the operation.
- `identity.rs` - logical **command identity** (`ARCHITECTURE` §11, R1): a resource id
  locates and serializes; a command id tells a retry from a deliberate new command
  (install v1, then v2). Same key + same request replays; same key + different request
  conflicts; a new key is a new intent.
- `service.rs` - the lifecycle over `agent-hub-db`; every transition publishes
  `hub.plugins.changed`.
- `routes.rs` - `GET/POST /v1/hub/plugins`, `DELETE /v1/hub/plugins/{id}`,
  `POST /v1/hub/plugins/{id}/prepare`.

Runtime prepare is delegated to the adapter (a later domain); without one it reports the
runtime not-ready rather than faking it.

## Tests

`cargo test -p agent-hub-plugins` runs the state and identity units plus an HTTP test:
install -> list -> replace (`updated: true`) -> remove, an invalid manifest refused, and
`hub.plugins.changed` delivered on the SSE stream.

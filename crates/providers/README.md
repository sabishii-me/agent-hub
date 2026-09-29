# agent-hub-providers

The providers domain (`ARCHITECTURE` §5): a model provider is **data**.

One provider is one JSON file in the hub's data dir (`providers/<id>.json`): the endpoint,
the wire protocol, the per-model declarations, the selection, and the cached catalog. The
hub owns the HTTP request, the auth, the catalog fetch and the field mapping; **no plugin
runs code inside the hub's process**.

- `record.rs` - the stored shape, plus the merge of catalog facts and declarations.
- `store.rs` - the file store; a malformed file is reported in `broken`, never dropped.
- `catalog.rs` - the fetch and the dialect mapping (openai-completions / anthropic-messages
  / custom-compatible); a failure keeps the previous catalog.
- `service.rs` - CRUD, the selection, and the refresh rules: first refresh enables new
  models, later refreshes preserve the choice and enable genuinely new ones, removed models
  are retained as unavailable. Changing url/api/credential **bumps the revision**, so the
  catalog reports `stale` until refreshed.
- `routes.rs` - the `/v1/hub/providers` surface; **the token never leaves** (only
  `tokenConfigured`).

## Tests

`cargo test -p agent-hub-providers` runs CRUD, revision staleness, selection validation and
survival across a refresh, a corrupt file reported, and a **real catalog fetch** against a
local upstream.

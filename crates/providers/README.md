# agent-hub-providers

Model providers are **data**: one JSON file per provider (`model-providers/<id>.json`) holding the
endpoint, protocol, per-model declarations, the selection and the cached catalog. The hub owns the
HTTP request, the auth, the catalog fetch and the field mapping; no plugin runs in-process.

**No credential is accepted yet.** A token is a secret and belongs in an OS secret store; this
record has **no `token` field at all**, so a secret cannot be persisted here by construction
(TASK-048 F02). Until a secret store exists, the API refuses a credential (`not_implemented`) and
the catalog fetch is unauthenticated.

- `GET/POST /v1/model-providers`, `GET/PATCH/DELETE /v1/model-providers/{id}`, `.../logout`.
- `GET/PATCH /v1/model-providers/{id}/models`, `POST .../models/refresh`.
- `GET /v1/models` - the models the hub manages (top-level; not a provider sub-detail).

## Tests

`cargo test -p agent-hub-providers` runs CRUD, revision staleness, selection validation and
survival across a refresh, a corrupt file reported, and a catalog fetch against a local upstream.

# agent-hub-harnesses

A thin top-level projection of the adapter registry (o4): a harness is a top-level resource that
hides the internal fact that a harness is a plugin, derived from the plugin list.

- `GET /v1/harnesses` - each registered harness, with **both** the adapter version and the runtime
  version (ADR-0004). Requires the bearer token (o5: no anonymous discovery).
- `GET /v1/harnesses/{id}/{presets,models,tools}` - routed to the adapter, gated on the harness
  declaring the capability; an undeclared one is `known: false` (never a faked empty list).
- `GET /v1/harnesses/{id}/extensions` - the extension ids this harness may be given
  (harness-scoped: a bare id is meaningless across harnesses).

enable/disable is **plugin lifecycle** (`/v1/plugins/{id}/enable|disable`), not a harness route.

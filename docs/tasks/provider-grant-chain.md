# The provider→session grant chain

Status: **delivered** (TASK-048 REVIEW-495ce94).

A session may name a hub-managed `modelProviderId` (+ `modelId`). The hub resolves it
and, at start, runs:

1. `session/start` (the adapter spawns its harness);
2. `credentials/grant` `{connectionId, value, url, declarations}` — the credential
   comes from the OS keychain; the adapter materialises a `hub-<id>` provider and does
   not persist the value (memory-only);
3. `config/set` with `{modelProviderId: <id>, model?}`.

The adapter's `applied.modelProviderId` / model are the **proof**; they are stored as
the session's applied identity, never assumed from the request. A reopen **re-grants**
(a restarted adapter holds nothing). `sessions` does not depend on `providers`: the
composition root injects the resolver.

Verified live (mock provider): session accepted → active, adapter log shows
`provider="hub-mockp" model="deepseek-flash"`, endpoint received `POST /v1/chat/completions`.
The mock's reply shape was not a valid turn, so the turn ended `failed` with the
adapter's reason — honest, and not a fake `completed`.

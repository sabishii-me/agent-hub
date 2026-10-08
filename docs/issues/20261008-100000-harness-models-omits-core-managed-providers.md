# 20261008-100000 - `/v1/harnesses/{id}/models` returns EMPTY, omitting core-managed providers

**State:** OPEN. Owner: HUB.
**Severity:** the contract's harness catalog is the "single authoritative source for model selection";
it omits every managed provider, so a caller cannot see the models a session can actually use.

## Measured (dev, real provider)

```
register + refresh a managed provider -> GET /v1/model-providers/<id>/models  -> 137 models
                                         GET /v1/harnesses/pi/models          ->   0 models
```

## Contract

`contract/openapi.json` GET /v1/harnesses/{id}/models: "the models THIS HARNESS CAN RUN — the
harness's own catalog **plus every core-managed provider the harness can reach during a session**,
so an entry means the same thing whichever harness answered... An injected provider is named
`hub-<coreProviderId>`... `thinkingLevels` is the list the model's configuration states."

## Cause (to confirm)

`crates/harnesses/src/service.rs::models` calls the adapter's `models/list` only
(`self.gated(id, "models", "models/list", "models")`); it does NOT merge the managed providers'
catalogs. So an injected provider's models (and their `thinkingLevels`) are absent.

## Impact

A client that selects a model from the harness catalog cannot see a managed provider's models at
all; the thinking-level test (and any UI model picker) has nothing to select.

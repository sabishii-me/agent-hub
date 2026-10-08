# 20261008-090000 - a provider id must be GIVEN, not randomly generated

**State:** OPEN (fixing). Owner: HUB / CONTRACT.

## The defect

`POST /v1/model-providers` with no `id` made the hub invent one: `new_id("prov")` ->
`prov-<hex>-<n>` (crates/providers/src/service.rs). The id is the provider's STABLE identity: a
caller selects it in `modelProviderId` (session create) and PATCH, reads it from GET, and refers to
it in logs. A random, unpredictable id is unusable for that - a caller registers a provider, then
cannot name it without reading back an opaque string, and two runs never agree.

## The rule

The provider's `id` is the caller's stable identifier (its name). It is REQUIRED on create: an
absent `id` is `400 validation_failed`, never a generated value. A duplicate id is already refused
409; that is the correct, predictable identity.

## Fix

- contract/v1.json + openapi.json: `id` is required; drop "the id is generated when absent".
- crates/providers/src/service.rs: `id` is required (no `unwrap_or_else(new_id)`); remove the dead
  `new_id("prov")` use for providers.
- tests: use a stable, real id (the provider's own name, e.g. `HOME-JP-prod`), never a generated one.

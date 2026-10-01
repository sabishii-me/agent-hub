# agent-hub-contract

The **single normalization authority** for the wire contract (ARCHITECTURE §9, task T1).

`contract/v1.json` is a compact DSL, not JSON Schema. This crate turns it into valid
**JSON Schema 2020-12** in one place, so no other component has to interpret the DSL and the
reproduced projection defects cannot recur.

## What it does

- Normalizes type strings (`string?`, `string|null`, `'manual'|string`, `integer[]`,
  `text`, `file`, `secret`, `binary`, `#/defs/x`) into JSON Schema.
- Folds the legacy `"nullable": true` into the node's `type` array, and rewrites
  `"type": "binary"` to a base64 string - the two forms that are **not** valid 2020-12.
- Projects the whole contract into an OpenAPI 3.1 document whose component and operation
  schemas all validate against the 2020-12 meta-schema.

## Regenerating

```
cargo run -p agent-hub-contract --bin emit-openapi -- .
```

writes `contract/openapi.json` deterministically. The committed artifact is this program's
output; it is never hand-edited.

## Tests

```
cargo test -p agent-hub-contract
```

`tests/normalize_contract.rs` is the T1 acceptance: the projected document carries none of the
reproduced defects (`type: []`, `nullable`, `type: binary`, a bare `const: manual`), every schema
validates against JSON Schema 2020-12, `PATCH /v1/sessions/{id}` no longer marks its 8 optional
source fields required, and the projection is deterministic with one operation per source endpoint.

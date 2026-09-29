# agent-hub-harnesses

The harnesses domain (`ARCHITECTURE` §3): a thin top-level projection of the adapter
registry, plus the capability-gated runtime answers routed to the adapter.

- `GET /v1/harnesses` - each registered harness as the contract's `harness` object, with
  **both** the adapter version and the runtime version (ADR-0004: two independent facts, no
  bare `version`).
- `GET /v1/harnesses/{id}/{presets|models|tools}` - routed to the adapter, gated on the
  harness **declaring** the capability. An undeclared capability is `known: false` - the rule
  the contract states twice: **never a faked empty list**.
- `GET /v1/hub/harnesses` - the management view.
- enable/disable change the registry, and the projection shows it.

## Tests

`cargo test -p agent-hub-harnesses` registers a real subprocess adapter and checks the
projection, the routing of a declared capability, `known:false` for an undeclared one, the
contract error code for an unknown harness, and enable/disable.

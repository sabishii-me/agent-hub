# Metadata routes (surface / openapi / shutdown)

The three process-level routes, all hub-side (no adapter needed):

- `GET /v1/surface` — what this process actually implements, as data: the mounted
  routes `{method, path, auth}` (from the SAME per-module tables the router is built
  from), the SSE event names, and the contract identity `{file, protocol, version,
  sha256}` (sha256 of `contract/v1.json`).
- `GET /v1/openapi.json` — the OpenAPI document, byte for byte as generated from the
  contract (a projection for tools, never a second source).
- `POST /v1/shutdown` — asks the process to stop (a watch channel the serve loop
  selects on alongside Ctrl-C).

Verified live: `/v1/surface` reports the real sha256 (matches the file), 60 routes, 19
events; `/v1/openapi.json` is BYTE-IDENTICAL to `contract/openapi.json`;
`POST /v1/shutdown` returns `{ok:true}` and the process stops.

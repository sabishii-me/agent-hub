# agent-hub (the served binary)

The hub process: a non-blocking `axum` server over the shared `Transport`, with the domains
mounted. `cargo run -p agent-hub` (or `AGENT_HUB_ADDR=127.0.0.1:8080 cargo run -p agent-hub`).

On start it writes `endpoint.json` (the URL and bearer token) under the data dir, prints the
bound address, and shuts down on Ctrl-C.

Every `/v1` route requires the bearer token from `endpoint.json`. A session owns a real
adapter process and accepts a hub-managed provider, a preset, and `plan`/`review`. A route for
a capability that is not built answers `501`.

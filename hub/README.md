# agent-hub (the served binary)

The hub process: a non-blocking `axum` server over the shared `Transport`, with the domains
mounted. `cargo run -p agent-hub` (or `AGENT_HUB_ADDR=127.0.0.1:8080 cargo run -p agent-hub`).

On start it writes `endpoint.json` (the URL and bearer token) under the data dir, prints the
bound address, and shuts down on Ctrl-C.

**Status: not a product yet.** Every route requires the bearer token. Sessions own a real
adapter process and accept a hub-managed provider / preset / plan+review (see the root
`README.md` and `docs/ARCHITECTURE.md` §18, §24). Everything unbuilt answers `501`.

See the repository root `README.md` for the continuous main task, the current vertical chain,
the next step and the authorization boundary.

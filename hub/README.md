# agent-hub

The hub process: a non-blocking axum server over the shared `Transport`, with the domains mounted.

```
cargo run -p agent-hub
AGENT_HUB_ADDR=127.0.0.1:8080 cargo run -p agent-hub
```

On start it writes `endpoint.json` (the URL and bearer token) under the data dir, prints the bound
address, and shuts down on Ctrl-C.

**Status: not a product yet.** A cross-review (TASK-048) found the served binary had no inbound
authentication, wrote provider secrets in plaintext, and answered session `active`/`running`/`fork`
with no adapter behind them. The honest state now: every route requires the bearer token; sessions
answer `501 not_implemented` until an adapter is wired; providers accept no credential (no secret
store). See `docs/ARCHITECTURE.md` §18.

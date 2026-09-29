# agent-hub

The hub process (`ARCHITECTURE` §17, the frame). A non-blocking axum server over the
shared `Transport`. Domains mount onto `transport::routes()` as they are implemented;
today it serves `/v1/hub/status` and the `/v1/hub/events` SSE stream.

```
cargo run -p agent-hub
AGENT_HUB_ADDR=127.0.0.1:8080 cargo run -p agent-hub
```

It prints the bound address (useful with port `0`) and shuts down on Ctrl-C.

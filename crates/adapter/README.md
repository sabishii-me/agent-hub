# agent-hub-adapter

The adapter domain (`ARCHITECTURE` §4, `contract/adapter-v1.json`): the hub owns the
adapter's **process**.

- `bus.rs` - the **agent-bus** client: JSON-RPC 2.0 over the adapter's stdin/stdout,
  one LF-terminated line per message. The connection is **split** into a cloneable
  [`RequestHandle`] (send a request, await its reply) and [`Notifications`] (the adapter's
  events), so requests and notifications never contend - a pump that held the same lock
  would deadlock.
- `manifest.rs` - the adapter `manifest.json`. A plugin is a directory; the directory name
  is its id. `protocol` must equal the hub's adapter protocol or the plugin is refused.
- `manager.rs` - the harness registry and the running adapters. A present adapter plugin is
  registered on first sight; a capability call is **gated on the harness declaring it**
  (`Unsupported`, never a silent fallback); the adapter's notifications become the
  contract's events, each carrying its `harnessId`.

No adapter code runs inside the hub's process.

## Tests

`cargo test -p agent-hub-adapter` spawns a **real Node subprocess** (`tests/fixtures/`) and
exercises: capability routing, an undeclared capability refused, an RPC error surfaced, the
adapter's notifications delivered as events, a wrong protocol version refused, and a disabled
harness not started.

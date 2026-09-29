# agent-hub-extensions

The extensions domain: the hub's side of the placement rule (`ARCHITECTURE` §6).

An extension is a directory a plugin ships (`<plugin>/extensions/<id>`). Before a harness runs,
the hub **writes that harness's selected extension directories into its data dir**
(`<DATA_DIR>/agents/<harness>/extensions`), and the adapter places them where its harness reads
them with **discovery off**. The hub owns the placement; the adapter only points the harness at
it.

- `install_for_harness` writes the selected set, replacing the previous one (so a removed
  extension is gone from the next run). An id the plugin does not ship is refused **with the
  available list** - the contract's rule.
- The plugin's shipped ids come from `AdapterManifest::shipped_extensions` (the
  `extensions/` directories plus the manifest's declared ids); the union is what
  `GET /v1/hub/harnesses` reports as `availableExtensions`.

Placement is the hub's; it is **not** an authorization boundary - it keeps the agent from editing
a trust-bearing extension through its workspace; it does not confine a same-principal agent.

## Tests

`cargo test -p agent-hub-extensions`; the install path and `availableExtensions` are also
exercised by the adapter and harnesses domain tests.

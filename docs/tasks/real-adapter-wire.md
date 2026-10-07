# Real-adapter wiring (against committed plugin SHAs)

The REAL plugins are the three repos the deployment composes: `prts-harness-pi`
(`23ae330d2beb5d567b556a358117c3f7c768a027`), `prts-harness-jouzu`
(`453eff2ea06d05438320b1d30eeac18f74a498d2`), `prts-harness-deepseek`
(`580aca8984978cead650c2389aa189a4313e96ad`). No `_`-prefixed scratch dirs.

## How a real plugin is composed (the normal operation)

The hub scans a plugins directory for directories with a `manifest.json`; the
deployment puts the plugin repo directory there. `AGENT_HUB_PLUGINS_DIR` names that
directory (else `<data_dir>/plugins`). The hub never installs a harness itself: the
plugin materialises its own runtime through its `runtime` capability's
`runtime/prepare` method (`npm pack` + unpack into `<plugin>/runtime`, then `npm
install` the dependencies), and the hub VERIFIES the declared command now exists.

Materialising the pi runtime by hand (what the adapter does): with the plugin
directory as CWD, `npm pack @earendil-works/pi-coding-agent@0.85.1`, unpack it into
`runtime/`, then `npm install --omit=dev`. `node runtime/dist/cli.js --version`
answers `0.85.1`. The runtime is never committed (`.gitignore`).

## Hub gaps fixed

- **`AGENT_HUB_PLUGINS_DIR` was not read.** The plugin README and the hub's own
  "no harness plugins" note name it as the search path, but `main.rs` always used
  `<data_dir>/plugins`. Now honoured.
- **`POST /v1/plugins/{id}/prepare` fabricated a stub** ("no adapter is attached").
  The contract (`adapter-v1.json` capability `runtime`) says the hub ASKS the
  adapter (`runtime/prepare`) and verifies the declared command exists. Now it does:
  no `runtime` capability -> `ready:true` ("brings its own runtime"); the adapter's
  own `ready` is not taken on faith - the declared command is checked on disk.

## Real acceptance: session lifecycle through /v1 against the REAL pi adapter

`AGENT_HUB_TEST_PLUGIN_DIR=E:/AI/ideas/prts-harness-pi AGENT_HUB_TEST_HARNESS=pi
cargo test --workspace` -> **39 result sets ok, zero warnings**, NO SKIPs. The gated
tests started a real session and drove it: `real_hub_session_202_start_close`,
`a_session_with_a_managed_provider_is_accepted_and_granted` (a real grant),
`a_session_patch_renames_and_sets_policy`, `compact_and_fork_are_real`,
`read_through_routes_start_the_process_and_answer_from_the_harness`.

Version combo: hub `prts-hub` (this branch) + `prts-harness-pi@23ae330` adapter +
`@earendil-works/pi-coding-agent@0.85.1` + node v24.8.0 + Windows x64 + an isolated
data dir under `%TEMP%`.

NOTE: the hub's OS-keychain probe still runs at boot (the credential-backed
`a_session_with_a_managed_provider_is_accepted_and_granted` test needs it); this run
used the real keychain. Record the service name + the entries and confirm deletion.

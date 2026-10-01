# dsh (DeepSeek Harness) is BLOCKED — needs an upstream/runtime upgrade

Recorded 2026-10-01 at hub `94895454e2c6bb1de633220c071307269cb670ac`.
Owner: PLUGIN/UPSTREAM (`prts-harness-deepseek` + `@deepseek-ai/dsh`), NOT the hub.

## Symptom

`AGENT_HUB_TEST_PLUGIN_DIR=prts-harness-deepseek AGENT_HUB_TEST_HARNESS=deepseek`
-> session `starting_failed`:
`adapter protocol: rpc error -32000: session/start failed: dsh rpc settings.update: fetch failed`

Cause: `dsh web` does not stay up, so the adapter's RPC to it fails. Reproduced DIRECTLY
(`node runtime/lib/bin.js web --no-open --port 0` with any `DSH_HOME`): the port prints,
then the process crashes before serving.

## Root causes (three, all real and reproducible)

1. **The adapter's `runtime/prepare` materialises a MIXED tree.** It runs
   `npm pack @deepseek-ai/dsh@0.1.0-rc.7` then `npm install`; the runtime's dependency
   ranges are `^0.1.0-rc.7`, so npm resolves `@deepseek-ai/dsh-base`/`-web-app` to
   **0.1.0-rc.8** while the app is rc.7 -> inconsistent -> boot crash.
2. **`runtime.sources.json` was INCOMPLETE**: it recorded
   `@deepseek-ai/dsh-session-title-first-prompt-llm` but NOT the peer it imports,
   `@deepseek-ai/dsh-session-title-llm` (its `lib/index.js` does
   `import ... from "@deepseek-ai/dsh-session-title-llm"`). Materialising from the record
   gave `plugin tree failed to load: @deepseek-ai/dsh-session-title-first-prompt-llm`.
   (Added the missing source with the registry's real integrity; 491 -> 492. The tree then
   loads further but still fails, see 3.)
3. **dsh's own boot requires the HMR service it disables.** `@deepseek-ai/dsh-web-app`'s
   patch sets the `hmr` row `disabled: true`; `profile-boot` guards creating HMR on
   `ctx.get("loader") !== void 0` (the loader service is absent in this profile), yet calls
   `watchUserPatches(ctx, ...)` OUTSIDE that guard, which throws
   `dsh: user patch-layer watching requires the Cordis HMR service`. This is an upstream dsh
   ordering bug (watchUserPatches must be inside the guard, or the loader/HMR must be
   mounted).

## What THIS means

- The hub is NOT at fault: it delegates to the adapter's `runtime/prepare` (contract), and
  the adapter + runtime are what fail.
- The capability stays NOT accepted. No mock/green is recorded for dsh.

## Action when we upgrade

- Upgrade `@deepseek-ai/dsh` to a **consistent released version** (no `^`-range mixing: pin
  the whole closure or use a complete `runtime.sources.json`).
- Make the adapter's `runtime/prepare` install from the **recorded closure**
  (`runtime.sources.json`, exact pins + integrity) instead of `npm pack`+`npm install`
  (the hub's `runtime.mjs` already does this; the adapter should too).
- Ensure `runtime.sources.json` is COMPLETE (every import resolvable; the record-runtime
  completeness check must catch peer-of-bundle holes — it did not).
- Re-run: `PI_PLUGIN_DIR=prts-harness-deepseek PI_HARNESS_ID=deepseek PI_MODEL=deepseek-flash node tests/e2e/real-provider.mjs`.

## Local WIP (uncommitted, in the PLUGIN repo)

- `prts-harness-deepseek/deepseek-adapter.cjs`: dialect-aware model probe (`/models` vs
  `/v1/models`), the same real defect found in pi/jouzu. KEEP.
- `prts-harness-deepseek/runtime.sources.json`: + the missing `dsh-session-title-llm` source
  (real integrity). KEEP.

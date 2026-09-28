# fixtures — the real released plugins

These are the **actual release artifacts** of this deployment's plugins, byte for byte as
they were published, plus `registry.json` naming each one's version, sha256 and size:

| artifact | what it is |
|---|---|
| `harness-adapter-deepseek-0.1.4.zip` | the DeepSeek harness adapter, with its runtime closure descriptor |
| `harness-adapter-jouzu-0.1.5.zip` | the Jouzu harness adapter |
| `harness-adapter-pi-0.1.4.zip` | the pi harness adapter |
| `model-provider-compatible-0.1.0.zip` | the compatible (OpenAI-wire) model provider |
| `model-provider-deepseek-0.1.0.zip` | the DeepSeek model provider |
| `model-provider-shisa-0.1.0.zip` | the Shisa model provider |

The tests install THESE, over loopback, through the real install path. A test never
fabricates a manifest: a manifest shaped to satisfy the hub proves nothing about the
plugins that ship.

## Refreshing

The fixtures are the release zips the registry already names. To refresh them after a
plugin is released, from this directory:

```sh
node refresh.mjs
```

It reads `../../registry.json` (the hub's own catalog, the authority for what is
published), downloads each entry's zip, verifies its sha256, and rewrites the fixture
registry. Nothing is invented: a fixture is the published bytes or it is not included.

## Why the zips are here and not fetched at test time

A test must run with no network, and the same bytes every time. The plugin's own release
is the source of truth; this directory is that release, pinned. `refresh.mjs` is the one
place that goes and gets it.

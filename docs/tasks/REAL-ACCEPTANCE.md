# Real acceptance — full real mechanism, honest results

Runner: `tests/e2e/real-provider.mjs`. Chain (NOTHING mocked):
Rust hub (`target/debug/agent-hub.exe`) -> the REAL pi adapter
(`prts-harness-pi@23ae330d2beb5d567b556a358117c3f7c768a027`, spawned via its manifest
`command`) -> the REAL pi runtime (`@earendil-works/pi-coding-agent@0.85.1`) -> the REAL
provider `HOME-JP-prod` (`http://192.168.31.29:8990`, `anthropic-messages`, the real API
key from `~/.pi/agent/models.json`).

## Result (2026-10-01)

```
POST /v1/model-providers -> 200 ok        (generic built-in type custom-compatible)
POST /v1/sessions -> 202
session status=active
POST /turns -> 202 (accepted)
===== REAL MODEL ANSWER =====
pong
assistantId=a-04947ba5-247d-44a8-b124-f87817919e81
```

The injected provider on disk: `hub-jp -> http://192.168.31.29:8990 anthropic-messages
models=137` (137 models fetched from the real endpoint). The model `deepseek-flash`
answered the prompt "Reply with exactly the word: pong" with exactly `pong`.

## Three REAL bugs the real chain exposed (only a real run shows them)

1. **pi adapter** `probeModels` always hit `<url>/models`. An `anthropic-messages`
   provider's models live at `<url>/v1/models` (its SDK appends `/v1`). Fixed: probe the
   dialect's path first, then the other; take whichever answers.
2. **pi adapter** `session/start`'s applied snapshot set `applied.connectionId` (native
   name) but NOT `applied.modelProviderId` (the hub's id), so the hub correctly refused an
   unconfirmed identity. Fixed: an injected provider reports BOTH, exactly as `config/set`
   already did (`adapter-v1` line ~388).
3. **hub** omitted `api` from `credentials/grant`, so the adapter defaulted to
   `openai-completions` and called the anthropic provider on the wrong wire protocol (pi
   retried, the turn yielded no message). Fixed: the grant carries the provider's `api`.

## pi AND jouzu both pass (2026-10-01)

With `PI_PLUGIN_DIR=prts-harness-jouzu PI_HARNESS_ID=jouzu PI_MODEL=deepseek-flash`,
the SAME chain answers `pong` (jouzu 0.1.13 runtime). jouzu's adapter needed the same two
of the three fixes as pi (dialect-aware model probe; report the injected provider at
`session/start`). Its injected provider: `hub-jp -> http://192.168.31.29:8990
anthropic-messages models=137`.

Full workspace suite against EACH real plugin, NO SKIPs:
- `AGENT_HUB_TEST_PLUGIN_DIR=prts-harness-pi   AGENT_HUB_TEST_HARNESS=pi   cargo test --workspace` -> 39 ok
- `AGENT_HUB_TEST_PLUGIN_DIR=prts-harness-jouzu AGENT_HUB_TEST_HARNESS=jouzu cargo test --workspace` -> 39 ok

A real harness is SLOW to start (jouzu measured 9s-70s) and, with no provider, a turn
has NO default deadline (adapter-v1:259). The lifecycle test therefore CANCELS the
turn to reach a terminal state (harness-independent) and REPAIRS the session before
racing admissions - instead of assuming pi's fast-fail. This is the hub's real cancel
and repair path, exercised on a real harness.

## DeepSeek via a MODEL-PROVIDER PLUGIN (2026-10-01)

`tests/e2e/deepseek-provider.mjs`: the hub reads a model-provider plugin's
`provider.json` DESCRIPTOR (data), owns the HTTP/catalog, and the person supplies only
the key. Chain: hub -> real pi adapter -> real pi runtime -> the type's OWN endpoint.

```
GET /v1/model-providers/types -> 200 [deepseek@1(hub)] broken=[]
descriptor endpoint: {"url":"https://api.deepseek.com","api":"openai-completions"}
POST /v1/model-providers {token, providerType:deepseek} -> 200  (url/api from the type)
POST /v1/model-providers/ds/models/refresh -> 200  (real HTTPS catalog:
     deepseek-flash "DeepSeek-V4.1-Flash", deepseek-v4-pro)
POST /v1/sessions {modelProviderId:ds, modelId:deepseek-flash} -> 202 -> active
POST /turns -> 202 -> real answer "pong"
```

Two REAL bugs this exposed (only a real HTTPS provider shows them):
1. **The hub could not speak HTTPS at all**: `reqwest` had `default-features=false`
   and NO TLS feature, so `https://api.deepseek.com/models` failed with "error sending
   request" while curl worked. Fixed: `rustls-no-provider` + install the `ring` provider
   once at hub startup (keeps ONE crypto backend).
2. **A type's OWN endpoint was ignored**: the descriptor's fixed endpoint is now applied
   on create, and a caller override is refused (`deepseek` owns
   `https://api.deepseek.com`; `custom-compatible` still takes the caller's url/api).

The plugin side: `prts-providers/deepseek/provider.json` is the DESCRIPTOR the Rust hub
reads (the old `provider.mjs` module protocol is NOT what the hub uses - the contract
says the hub never imports plugin code).

### BOTH provider plugins, same DeepSeek HTTPS endpoint

- **A - `deepseek` plugin**: the TYPE OWNS the endpoint; the person supplies only the
  key. `POST /v1/model-providers {token, providerType:deepseek}` -> url/api come from
  the descriptor; an override is refused.
- **B - `compatible` plugin (`custom-compatible`)**: the CALLER supplies url/api. `POST
  /v1/model-providers {url:"https://api.deepseek.com", api:"openai-completions", token,
  providerType:custom-compatible}`. No fixed endpoint in the descriptor.

Both fetched the real catalog (`/models/refresh -> 200`: deepseek-flash, deepseek-v4-pro)
and a real turn answered `pong`. The descriptor differs only in
`configuration.endpoint` (present for `deepseek`, absent for `compatible`).

## Shisa via a device-code sign-in (2026-10-01)

A REAL device-code sign-in (account `colin@shisa.ai`, workspace "colin's workspace")
produced the credential. `tests/e2e/shisa-provider.mjs` then drove the full chain:

```
GET /v1/model-providers/types -> 200 [shisa] broken=[]
POST /v1/model-providers {token, providerType:shisa} -> 200
     url=https://api.shisa.ai/openai/v1 api=openai-completions  (type owns endpoint)
POST /models/refresh -> 200  real catalog: 11 models
     (glm-5.2, qwen3.7-flash/max/plus, qwen3.8-27b, shisa-ai/*, ...)
POST /v1/sessions {modelProviderId:sh, modelId:qwen3.7-flash} -> 202 -> active
POST /turns -> 202 -> real answer "pong"
```

`prts-providers/shisa/provider.json` declares `authMethods:["device-code"]` and an
`auth` block (gateway, paths, clientId) as DATA, plus the account-owned endpoint the
type supplies.

HONEST BOUNDARY: the credential used here came from a device-code flow run OUT OF BAND
(the plugin's own protocol: `POST gateway.shisa.ai/device/code` -> the user visits
`platform.shisa.ai/connect?...` -> `POST /device/token`). The RUST hub's
`POST /v1/model-providers/{id}/auth` does NOT yet execute that flow in-process (G4
refuses honestly rather than fabricate). So shisa's SIGN-IN is exercised plugin-side; the
provider RECORD, catalog and a real model turn are exercised through /v1. Making the hub
own the device-code flow in-process is the remaining shisa work.

## What this is and is NOT

- IS: a real end-to-end model turn through the hub, the real adapter, the real runtime and
  the real provider. No fake HTTP provider, no fake key, no mock adapter.
- NOT yet: jouzu and dsh through the same chain; the security/enumeration surfaces; the
  remaining §17 items. Each is a separate acceptance.

## Side effects (recorded, and cleaned)

- OS keychain: service `agent-hub:<instance>`; one `__probe__<hex>` entry at boot
  (deleted by the probe) and `provider-jp` for this run's credential.
- Real provider call to `192.168.31.29:8990` (a real request, real billing).
- Runtime materialisation: `npm pack` + `npm install` of the pi runtime into
  `prts-harness-pi/runtime/` (never committed).

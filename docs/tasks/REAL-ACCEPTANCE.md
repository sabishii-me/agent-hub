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

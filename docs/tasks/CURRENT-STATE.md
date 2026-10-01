# CURRENT STATE — read this first (durable handoff)

Updated at hub SHA `b9bef5a459fed2cd2b7b6ec22aa1143de84c71de` (branch
`feat/hub-modular-redesign`). Working tree has UNCOMMITTED, UNFINISHED edits (see below).

## The task (unchanged)

Implement the Rust agent-hub per the approved architecture, delivering REAL, usable `/v1`
capabilities **against the REAL adapters/plugins**. NOT: fixing reviewer numbers, making
tests green, mounting routes, or mock-driven "verification". §17 of `docs/ARCHITECTURE.md`
is the order of work. A reviewer report is correction INPUT, not the task.

## Rules established with the user (do not repeat)

- A MOCK anywhere = the whole chain is fake. Acceptance uses the REAL adapters/plugins.
- NEVER `git checkout` / `git reset --hard` / discard uncommitted work. Keep it.
- `_`-prefixed scratch dirs are forbidden. Use the real repos.
- Docs FIRST, then code. Update the ledger per change.
- If a document LACKS a definition, SAY SO plainly (it is missing external input); do NOT
  silently invent a schema, and do NOT blindly attack the doc either.
- No invented concepts. The contract is the interface; a field it does not define is not
  mine to add in code.

## Real inputs present locally (reviewed SHAs)

- pi adapter:      `/e/AI/ideas/prts-harness-pi`     @ `23ae330d2beb5d567b556a358117c3f7c768a027` (runtime materialised: pi-coding-agent 0.85.1)
- jouzu adapter:   `/e/AI/ideas/prts-harness-jouzu`  @ `453eff2ea06d05438320b1d30eeac18f74a498d2` (runtime jouzu 0.1.13)
- deepseek adapter:`/e/AI/ideas/prts-harness-deepseek` (not yet run)
- provider plugins:`/e/AI/ideas/prts-providers/{compatible,deepseek,shisa}` (each now ships `provider.json`, a DATA descriptor the Rust hub reads; the hub never runs plugin code)
- real provider creds: `~/.pi/agent/models.json` provider `HOME-JP-prod` (anthropic-messages)

## Real acceptance DONE (recorded)

- `docs/tasks/REAL-ACCEPTANCE.md`: pi + jouzu (real adapters + real runtimes + HOME-JP-prod)
  -> session active, real turn answers "pong"; workspace suite 39 ok against each, no SKIPs.
- deepseek via BOTH provider plugins (deepseek type-owned endpoint; compatible caller endpoint)
  -> /models/refresh 200 (real catalog), session active, real turn "pong".
- shisa via the shisa provider plugin: a REAL device-code sign-in (colin@shisa.ai) was run
  OUT OF BAND; the hub then read the descriptor, used the type's owned endpoint
  (https://api.shisa.ai/openai/v1), fetched 11 real models, session active, real turn "pong".

## UNCOMMITTED WORK IN PROGRESS (do not lose; do not commit until the schema question is settled)

Files: `crates/providers/src/{auth.rs,routes.rs,service.rs}`, `crates/providers/Cargo.toml`,
`Cargo.lock`. It implements a HUB-OWNED device-code flow driven by the provider type's
descriptor (contract 2262: the hub owns the flow, the type declares it as data).

BLOCKER before committing: the FIELD SCHEMA is NOT defined by any document.
- `contract/v1.json` says only `next: object`; my field names (`userCode`,`verifyUrl`,
  `expiresInSeconds`,`intervalSeconds`) appear ZERO times in the contract.
- `ROUTES-REVIEW.md:308` writes `auth: { method:"device-code", gateway, clientId, ... }` — the
  field is `method`; I wrote `kind`. The `...` (clientVersion/codePath/tokenPath/linkAckPath)
  is NOT documented.
- `ARCHITECTURE.md` §438 says the auth sub-operation must SURVIVE A RESTART; my `AuthStore`
  is in-memory (does not).
- The edits do NOT compile yet (`into_response_ok_created`, `op_id_provider` do not exist).

DECISION OWED BY THE USER: either (1) the user/contract defines the `auth` block + `next`
step fields and I implement exactly that; or (2) the user authorises me to take the fields
from a named existing fact (the plugin `beginAuth`/`device/token` shapes) AND register them
as an explicit contract change in `contract/v1.json`. Until then: do NOT commit, do NOT
invent further.

## Blocked (owner = PLUGIN/UPSTREAM, not the hub)

- **dsh (DeepSeek Harness)** cannot start: its runtime does not boot on this machine.
  Three real causes recorded in `docs/tasks/dsh-blocked.md`; needs an upstream upgrade to a
  consistent released version + a complete recorded closure + the HMR/loader fix. Do NOT
  mark dsh as accepted. pi and jouzu are unaffected.

## Per-capability status

`docs/tasks/system-alignment.md` (table + delivery log). Legend for evidence: STATIC vs
RUN-MOCK (not acceptance) vs REAL (through /v1 against a real adapter).

## Docs map

- `README.md` (target), `docs/ARCHITECTURE.md` (§17 order, §18 status, §5 providers-are-DATA,
  §438 auth flow), `docs/ROUTES-REVIEW.md` (§ Model providers are DATA; auth is a hub protocol),
  `contract/v1.json` (the interface), `docs/tasks/{system-alignment,real-adapter-wire,REAL-ACCEPTANCE}.md`.

## Command facts

- `cargo test --workspace` with `AGENT_HUB_TEST_PLUGIN_DIR=<real plugin> AGENT_HUB_TEST_HARNESS=<id>`.
- e2e runners: `node tests/e2e/{real-provider,deepseek-provider,shisa-provider}.mjs`.
- Run needs `AGENT_HUB_CONTRACT_DIR`, `AGENT_HUB_DATA_DIR`, `AGENT_HUB_ADDR`.
- Tooling: the read/write/edit tools target a wrong cwd; use bash heredocs to absolute paths.

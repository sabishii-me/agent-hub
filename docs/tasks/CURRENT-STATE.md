# CURRENT STATE — read this first (durable handoff)

Updated at hub SHA `85db400512a761fde47e10c496385e90573531d5` (branch
`feat/hub-modular-redesign`, pushed). Working tree clean.

## The task (unchanged)

Implement the Rust agent-hub per the approved architecture, delivering REAL, usable `/v1`
capabilities **against the REAL adapters/plugins**. NOT: fixing reviewer numbers, making
tests green, mounting routes, or mock-driven "verification". §17 of `docs/ARCHITECTURE.md`
is the order of work. A reviewer report is correction INPUT, not the task.

## The rule I violated (do not repeat)

Using a MOCK adapter to drive `/v1` and calling it "verified" is FORBIDDEN. A mock proves
the hub's half, not the capability. The three REAL adapters are present locally at the
exact reviewed SHAs and must be used:

- pi:      `/e/AI/ideas/_sess2/plugins/pi` (also `prts-harness-pi`) @ `23ae330d2beb5d567b556a358117c3f7c768a027`
- jouzu:   `/e/AI/ideas/prts-harness-jouzu` @ `453eff2ea06d05438320b1d30eeac18f74a498d2`
- deepseek:`/e/AI/ideas/prts-harness-deepseek` @ `580aca8984978cead650c2389aa189a4313e96ad`

## The ONE blocker to a real run (needs authorization)

The hub UNCONDITIONALLY probes the real OS keychain at startup
(`crates/secrets/src/lib.rs::probe_store`: writes/reads/deletes a random `__probe__<hex>`
entry under service `agent-hub:<instance>`). That side effect is NOT authorized, and the
handoff forbids dodging it with a test switch / alternate backend / borrowed instance.

**Authorization needed (minimal):** allow starting the hub against an ISOLATED data dir
while it touches the real OS keychain with that random probe (and any provider credential
entries I create). I will record the service name + entries and confirm deletion after.

Secondary (only for a real model turn): a real vendor/paid call authorization. WITHOUT it,
approval / session / preset / connections / restart acceptance can still be run.

Until that authorization: **no real acceptance is run; every capability stays NOT
accepted. Never fill green.**

## Per-capability status (REAL evidence classes)

Legend: STATIC = source comparison against the real adapter; RUN-MOCK = driven through
/v1 with a mock (NOT acceptance); REAL = driven through /v1 against a real adapter (NONE
yet — blocked by the keychain authorization).

| Link | Code state | Real acceptance |
|---|---|---|
| G1 adapter bidirectional control (approval_need round-trip) | CODE DONE (`5a7139c`), reply vocabulary aligned to the real `{approved, reason:'allowed'|'denied'}` (`85db400`) | NOT RUN |
| G2 harness connections/auth wire mapping (`connections/delete` -> `id`) | CODE DONE (`4aa9b99`), confirmed against real dsh `rows.find(r=>r.id===p.id)` | NOT RUN |
| G3 provider type authority (create/availability/grant) | CODE DONE (`57bcd3d`) | NOT RUN |
| G4 auth route does not fabricate a pending op | CODE DONE (`714f9be`) | NOT RUN |
| G5 preset restart failure -> needs-repair; composite PATCH runs all fields | CODE DONE (`bb952ad`) | NOT RUN |
| G6 minHubVersion gate at accept + activation | CODE DONE (`3274034`) | NOT RUN |
| A2 preset switch without wedging | CODE DONE (`248f6be`..) | NOT RUN |

Static wire findings vs the REAL adapters: `docs/tasks/real-adapter-wire.md`.

## Open hub-side remainder (only real acceptance unblocks "done")

- G3 remainder (CONTRACT+PLUGIN): publish the `provider.json` descriptor artifact in the
  owning contract; upgrade the provider plugin.
- HUB: real declarative auth flow (needs the step schema in the contract); new
  plugin-sourced skills/resources (T2b); shared adapter layer; `prompt` bound to the
  turn's process generation (old §E remainder).

## Docs map

- `README.md` — target + open/closed links by owner; authorization boundary.
- `docs/ARCHITECTURE.md` §17 (order), §18 (CURRENT STATUS block), §24 (route census —
  a COUNT, NOT a capability claim).
- `docs/tasks/system-alignment.md` — per-capability table + delivery log.
- `docs/tasks/real-adapter-wire.md` — static comparison against the real adapters.
- `docs/review/VERIFICATION-TASKS.md` — T1–T6 exit conditions.

## Command facts

- `cargo test --workspace` green (39 result sets); zero warnings. Mock rehearsal dirs live
  under `E:/AI/ideas/_mock` (NOT in the repo).
- Build: `cargo build --workspace`; run needs `AGENT_HUB_CONTRACT_DIR`,
  `AGENT_HUB_DATA_DIR`, `AGENT_HUB_ADDR`. Keys: `AGENT_HUB_CONTROL_TIMEOUT_SECS`,
  `AGENT_HUB_TEST_PLUGIN_DIR` (gated real-adapter tests; a missing dir prints SKIP).
- Tooling note: the `write`/`edit`/`read` tools target a wrong cwd (`E:\e\...`); use
  bash heredocs to absolute paths.

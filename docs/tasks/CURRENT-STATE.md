# CURRENT STATE — read this first (durable handoff)

Updated 2026-10-03 at hub `14fd7159df919abdd19a65c62e4c95cd8cc79cf1` (branch
`feat/hub-modular-redesign`). Working tree clean.

## The task (unchanged)

Implement the Rust agent-hub per the approved architecture, delivering REAL, usable `/v1`
capabilities **against the REAL adapters/plugins**. NOT: reviewer numbers, green tests, mounted
routes, or mock-driven "verification". §17 of `docs/ARCHITECTURE.md` is the order of work.

## The rule (repeated; do not break)

- A FAKE anywhere makes the whole chain false. No fake provider, no stub adapter, no mocked
  response. A test that cannot run REAL prints SKIP + the missing thing. Never fake a pass.
- NEVER `git checkout --` / `git reset --hard` / discard uncommitted work.
- No `_`-prefixed scratch dirs; use the real repos / a documented layout.
- Docs FIRST, then code. If a document lacks a definition, SAY SO; do not invent a schema.
- A change to `contract/` or an ADR needs the OWNER's review BEFORE it is written.
- Acceptance is REAL runs against real adapters, with the version combo recorded.

## Where things stand (2026-10-03)

- **Surface**: `/v1/surface` == the contract (74 == 74), no stub route. `cargo test --workspace`
  is 39 result sets ok. This is a ROUTING fact, not a capability claim.
- **Real acceptance done**: pi (now runtime **1.0.0**, plugin 0.1.9) and jouzu answer a real
  model turn; deepseek + shisa provider plugins work; the hub-owned device-code sign-in works
  (a real shisa login this session). Evidence: docs/tasks/REAL-ACCEPTANCE.md.
- **dsh**: BLOCKED (upstream runtime cannot boot here). docs/tasks/dsh-blocked.md.
- **Open review findings A1-A7** (docs/tasks/review-bb23f7d-findings.md): skills fell back to
  the excluded hub-authored store (A1); resource symlink check is last-component only (A2);
  auth result not bound to incarnation / cancel can still approve (A3); auth writes endpoint
  before the journal (A4); auth id repeats across restarts + the "generic" device-code is one
  vendor's shape (A5); min-host refusal overridable by enable + provider types bypass it (A6);
  approval visible-before-register race (A7). ALL UNFIXED.

## CURRENT WORK: rebuild the test suite (docs/tasks/test-suite.md)

The product has enough capability to test seriously. The suite is being rebuilt:
- **Python 3.13 (stdlib)** — deliberately NOT the hub's language (Rust), so a test cannot
  import hub internals or hand-build a fake. Runner `python tests/run.py [layer]`.
- Real hub binary + real plugin + real runtime + real provider; **no fakes**; layered
  L1-L9; SKIP (never fake) when a real dependency (auth, skills) is absent.
- This REPLACES the fake-provider Rust integration test and the ping-pong e2e runners as the
  acceptance bar; those are not deleted blindly (Rust unit tests stay as function tests).
- The old `tests/*.mjs` suite drives the OLD Node hub (`server.mjs`, `/v1/hub/...`) — archived.

## DEFERRED (timestamped)

- **skills**: 2026-10-03. Resume after the skill-content decision (proposed: a `skill` plugin
  kind, artifact + shared lifecycle), the layered-selection field names, and the PUT/DELETE
  fate. L9-skills does not run until then.

## Docs map

README.md; docs/ARCHITECTURE.md (§17 order, §18 status); docs/tasks/test-suite.md (the suite);
system-alignment.md; review-bb23f7d-findings.md (A1-A7); dsh-blocked.md; REAL-ACCEPTANCE.md;
real-adapter-wire.md; contract/v1.json (the interface).

## Command facts

- `cargo build --workspace`; run needs `AGENT_HUB_CONTRACT_DIR`, `AGENT_HUB_DATA_DIR`,
  `AGENT_HUB_ADDR`. Real e2e: `node tests/e2e/real-provider.mjs` with PI_PLUGIN_DIR etc.
- Real plugin dirs (reviewed SHAs in real-adapter-wire.md): pi/jouzu/deepseek under
  `E:/AI/ideas/prts-harness-*`; provider plugins under `E:/AI/ideas/prts-providers/*`.
- Tooling: the read/write/edit tools target a wrong cwd; use bash heredocs to absolute paths.

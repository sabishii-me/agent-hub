# CURRENT STATE — read this first (durable handoff)

## UPDATE 2026-10-11 (later) — P5 mid-turn DONE; jouzu resume-hang RECORDED (wait for its pi upgrade)

### Verified last run (real harnesses, over /v1)
- whole-chain: pi/jouzu/deepseek PASS (deepseek 15/15).
- interrupt: **pi 21/21 files, deepseek 20/21** (C7 cannot run: no approval preset), **jouzu 19/21**
  (C4 + A9 RED = the recorded resume-hang; P5 8/9 = jouzu's old-pi model call 400s before the cut).
  New: **P5 mid-turn** (`the-provider-errors-mid-turn-and-the-turn-settles.py` + `tests/lib/httpfault.py`)
  - a real HTTP proxy to the real provider cuts the SSE stream after the first token; pi 9/9, deepseek 9/9.
- Presets test: deepseek 8/8 (the `agentPresets/select` fix works).

### Where the task stands
The task "adapt the deepseek (dsh) adapter to dsh 0.2.0 and make the checks real" is DONE; the
current work is the **interrupt (fault-injection) test layer** being filled in against the real
harnesses.

- **dsh 0.2.0 adapter adaptation: DONE** (auth cookie, `ns/method` endpoints, `{args}` payloads,
  `session/prompt` requestId, `remote.mux`+`session/follow`+`$events`, approvals via `$events/result`,
  tools recorded, `turn_end` carries clientMessageId, `session/list` uses `_request`,
  `agentPresets/select` uses flat `{agentId,agentPreset}`). Adapter repo `prts-harness-deepseek`.
- **Plugin REMOVE fixed**: the hub delivers a one-way `plugin/removing` EVENT before deleting; the
  plugin keeps itself clean (deepseek stops its shared dsh server; pi/jouzu stop their harness child).
  Contract `coreCompliance.notifications`; bus `notify`; adapter-manager `notify_removing`.
- **Whole-chain (over /v1, real adapters): pi/jouzu/deepseek all PASS** (deepseek 15/15 incl. the
  `ended == completed` assertion).

### Interrupt coverage (taxonomy `docs/needReview/issues/20261003-123000-interrupt-coverage-is-incomplete.md`)
- **Covered, green on pi/jouzu/deepseek:** A1, A3, A5, A6, A7, A8, A9*, A10, P1, P2, P4,
  P5(start-time), P6, P7, C1, C2, C3, C4*, C5, C6, C7*(*C7 deepseek cannot: no approval preset;
  *A9/C4 GREEN on pi+deepseek but RED on jouzu - the jouzu reopen hang below).
- **Still MISSING:** A2, A4, P3 (need a deliberately misbehaving harness - DEFERRED per owner:
  finish the real-harness cases first), P8 (needs a careful `BEGIN EXCLUSIVE` lock of the live DB).
  P5(mid-turn) is now DONE.
- `tests/interrupt/` now has 20 files, all env-driven (`PI_PLUGIN_DIR`/`PI_HARNESS_ID`/`PI_MODEL`;
  C7 also `PI_APPROVAL_PRESET`).

### OPEN issues (real, recorded)
- **`20261011-010000` [RECORDED - DO NOT FIX NOW]** - jouzu: a turn after CLOSE+REOPEN never starts
  (pi accepts the prompt, no agent runs). Root-caused to the OLD pi version jouzu PINS; pi itself is
  14/14 on the same flow (control run). jouzu is about to sync to the latest pi, so fixing it now is
  wasted work. C4 and A9 stay RED on jouzu until then. Do NOT change the hub (verified not the hub).
- **`20261010-001000`** - deepseek ships no approval-gating preset, so C7 cannot run on it.
- **`20261009-110000`** - dead-adapter detection latency; NOT a hub defect in a clean env (the ~1s
  is the reader's EOF debounce, by design; the 20s cases were machine contention). Tests poll.
- FIXED this stretch: `20261010-000000` (pending approval blocked the death signal),
  `20261009-030000` (detached dsh server outlived remove), `20261009-090000`/`-100000`/`-101000`.

### The immediate next task
**P8** - the hub's DB write fails while settling a turn -> no dishonest terminal. The product path
exists (`settle_turn`: record `terminal_intent`, retry the write, never publish on failure, leave it
to reconcile) but has only unit coverage. Needs a REAL injection: hold `BEGIN EXCLUSIVE` on the live
`hub.sqlite` at the settle instant - fragile (can wedge the hub), so it is its own careful task.
A2/A4/P3 stay deferred (they need a misbehaving harness). The jouzu resume-hang (and P5's jouzu 400)
are RECORDED and wait for jouzu's pi upgrade.

### Commits (hub `feat/hub-modular-redesign`)
`632dea2` (_CUSTOM- + close stdin), `9e78b7e`/`957dcd5`/`6f2e49d` (remove event),
`6d79640`/`40c73d7`/`15c63a6`/`b2b65ef`/`2bfee3b`/`cae1b81`/`0d4b36a`/`6b5a486`/`3bef072`
(interrupt tests A3/A5/A8/A9/A10 + C4/C6/C7, the pump fix, coverage docs).
Adapter repos: `prts-harness-deepseek` (`1892c0b` turn_end clientMessageId + session/list `_request`,
`ad0bcc1` agentPresets/select flat args), `prts-harness-pi`/`prts-harness-jouzu`
(`plugin/removing` handling).

### Canonical runners (py, env-driven — do NOT fork a new test system)
- whole chain: `PI_PLUGIN_DIR=<repo> PI_HARNESS_ID=<pi|jouzu|deepseek> PI_MODEL=gpt-6-astra python tests/lifecycle/the-whole-chain-plugin-to-session-to-tools-over-v1.py`
- interrupt: any `tests/interrupt/*.py` the same way. Clean state between runs: kill `agent-hub.exe`
  and any `node` under `*test-home*`, then `rm -rf .test-home` (orphans make runs flaky).

### Guardrails
- Do NOT downgrade the dsh pin (`0.2.0-rc.2`). No external `taskkill`/`tasklist` from child processes
  (endpoint protection flags it); product code uses Node's own `child.kill()`, tests use CIM + Node's
  `os.kill`. Provider ids live in `_CUSTOM-<name>` (a const). A change to `contract/`/ADR needs the
  OWNER's review BEFORE it is written. Never weaken a test to make it pass.

## UPDATE 2026-10-09 — current task: adapt the dsh adapter to dsh 0.2.0 (+ clean plugin remove)

**Read `docs/needReview/review/20261009-remove-must-release-plugin-resources.md` first.** It has the
task state, commits, and the open decision. Summary:

- **Task**: make the DeepSeek harness (`deepseek` plugin / `dsh`) work on its pinned runtime
  **`@deepseek-ai/dsh 0.2.0-rc.2`**, driving the REAL adapter through `/v1`, then make plugin REMOVE
  clean (stopping the dsh SERVICE the adapter starts).
- **dsh 0.2.0 adapter adaptation: DONE + verified** (auth cookie, `ns/method` endpoints, `{args}`
  payloads, `session/prompt` requestId, `remote.mux`+`session/follow`+`$events`, approvals
  `$events/result`, tools recorded). Adapter commits `1f4bc97`,`34c57f9`,`6bb9eed`
  (prts-harness-deepseek).
- **deepseek whole-chain: 13/14.** Only `the removed plugin is gone` FAILS: remove does not stop the
  detached `dsh web` service, so `remove_dir_all` returns `Access is denied`. This is a **design gap**
  (a service lifetime the architecture never modelled — see the review doc), to be fixed with a
  plugin-level RELEASE phase before delete. **Needs owner review of the contract shape.**
- **Canonical runner (py, env-driven)**: `PI_PLUGIN_DIR=/e/AI/ideas/prts-harness-deepseek
  PI_HARNESS_ID=deepseek PI_MODEL=gpt-6-astra python
  tests/lifecycle/the-whole-chain-plugin-to-session-to-tools-over-v1.py`. Do NOT fork a new test
  system; do NOT debug by hacking the test script.
- **Guardrails**: no downgrade of the dsh pin; no external `taskkill`/`tasklist` from child processes
  (endpoint protection flags it); provider ids live in `_CUSTOM-<name>` (a constant).


Updated 2026-10-05, hub commit `19746ff` (branch `feat/hub-modular-redesign`). Working tree clean.

## The task (unchanged)

Implement the Rust agent-hub per the approved architecture, delivering REAL, usable `/v1`
capabilities **against the REAL adapters/plugins**. §17 of `docs/ARCHITECTURE.md` is the order of
work. A capability is DONE only when driven through `/v1` against a REAL adapter/plugin.

## The rule (repeated; do not break)

- A FAKE anywhere makes the whole chain false. A missing REAL dependency FAILS the test - no fake
  green, no scaffold substituting for the product.
- NEVER `git checkout --` / `reset --hard` / discard uncommitted work.
- Docs FIRST, then code. A change to `contract/` or an ADR needs the OWNER's review BEFORE it is
  written. Do NOT impose a contract change to make tests pass.
- A diagnosis needs a CONTROL; one observation is a guess. Suspect MY inputs first.
- NEVER experiment on the owner's system; never read real credentials or call models unless the
  task says so.

## Where things stand (2026-10-05)

**The single blocker: the official plugin registry is UNPUBLISHED**, so the plugin-driven user
chain cannot be established and is UNVERIFIED. The registry address is compiled into the hub:
`https://github.com/sabishii-me/agent-hub/releases/download/registry/registry.json` (a fixed
`registry` release asset) — currently HTTP 404 (the repo has 0 releases).

### Verified, narrow (the default test entry — `python tests/run.py`, 10 files -> 8 PASS, 2 PARTIAL)
- served metadata shapes (openapi == committed file; `/v1/models` is an ARRAY of {id,providerId});
- the served ROUTE SET equals the contract; no parameterless GET is 501;
- **registry-online rehearsal (7/7)**: a real HTTP registry serving the repo's `registry.json`
  verbatim; the hub refreshes ITS registry, the catalog lists the real ids, and the hub installs a
  release the CATALOG names (the test supplies the ADDRESS only, never a url/sha256) -> `ready`;
- with NO registry, an unlisted source is refused 403 `plugin_not_in_registry` (install-reconciles,
  install-refusals, a-caller-cannot-install);
- a RELEASE binary ignores `AGENT_HUB_REGISTRY_URL` (probe);
- an empty hub lists no plugin/harness; a session is 404 `harness_not_found`;
- skills + connections CRUD (authoritative read-back);
- 120 raw sockets held open; a held SSE does not block a GET.

### UNVERIFIED (registry unpublished)
The 33 plugin-chain files (lifecycle/interrupt/approvals/concurrency/provider/plugins-lifecycle/
events/harness-discovery/presets/tools/model) need a plugin installed THROUGH the hub, which needs
the published registry. Listed via `python tests/run.py --materials` (path + reason + product gaps);
`--materials` starts NO subprocess. Their earlier PASS/FAIL are WITHDRAWN (a scaffold supplied the
environment). See docs/tasks/20261005-retractions.md and .../entry-reconciliation.md.

### Product changes made this session (real, not test fakes)
- **T0 `20261005-130000` FIXED**: install now RECONCILES the source against the hub's loaded
  registry (`crates/plugins/src/service.rs` begin_install -> authorized(); `registry.rs`
  Registry::authorizes: url AND sha256). With no registry, nothing installs.
- `20261005-120000` FIXED: installing an id owned by a deployment dir -> 409 conflict, tree untouched.
- `20261005-140000`/`-141000` FIXED: a missing-id remove -> 404 not_found (not a false "deployment
  directory"); a body missing `source` -> 400 validation_failed (not 422 axum text).
- `20261005-110000` A FIXED (`RegistryUrlMissing` displays its payload); B: the contract code
  `registry_unavailable` (502) ADDED (owner-approved) and mapped.
- The registry ADDRESS is fixed at build time (release: the official URL, no env override; dev:
  `AGENT_HUB_REGISTRY_URL` override, compiled out of release).

### Second, independent blocker
The host's real provider (`~/.pi/agent/models.json` -> `192.168.31.29:8990`) answers 400 to its own
`/models` with the host token (verified outside the hub). Session/turn tests are blocked by it too
(`20261005-150000`).

### dsh
BLOCKED (upstream runtime cannot boot here). docs/tasks/dsh-blocked.md.

## Open issues / What is next
SUPERSEDED by the top block (2026-10-11). The authoritative list is at the top of this file:
- OPEN: `20261011-010000` (jouzu resume-hang; RECORD, wait for jouzu's pi upgrade),
  `20261010-001000` (deepseek has no approval preset), `20261009-110000` (death latency; NOT a
  clean-env defect), `20261003-123000` (interrupt cases still to add: A2, A4, P3, P5-mid, P8).
- FINISHED: the whole chain on pi/jouzu/deepseek; plugin remove; the dsh 0.2.0 adaptation; and the
  interrupt cases A1/A3/A5/A6/A7/A8/A9/A10, P1/P2/P4/P5-start/P6/P7, C1..C7.
- NEXT: P5 (mid-turn provider error) and P8 (DB write fails while settling), on the REAL harnesses.
  A2/A4/P3 stay deferred (they need a deliberately misbehaving harness).

## Do NOT
- do not publish the registry to make tests green; publication is for verifying the real chain;
- do not fabricate a registry, override a source, or use a test-side install to "prove" the chain;
- do not modify `contract/` without the owner's review; do not add `{id}` install without that review;
- do not treat PASS/FAIL counts as hub-capability acceptance.

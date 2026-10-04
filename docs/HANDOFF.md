# HANDOFF — resume here (2026-10-04)

Context is about to be lost. Read this, then `docs/tasks/alignment-code-vs-goal.md`, then
`docs/issues/20261004-050000-*`. Do NOT act before reading the OPEN PROBLEM below.

Hub HEAD: `76e968c` (branch `feat/hub-modular-redesign`), working tree clean.
Plugin SHAs: pi `9fb8c1c` (+ an UNCOMMITTED `pi-adapter.cjs` change: the turn_end
clientMessageId fix), jouzu `31ce603`, dsh `580aca8` (+ uncommitted dialect/sources fixes).

## THE TASK (unchanged)

Implement the Rust agent-hub per the approved architecture, delivering REAL, usable `/v1`
capabilities against the REAL adapters. §17 is the order of work. NOT: green tests, mounted
routes, mock-driven proof. A capability is DONE only when driven through `/v1` against a real
adapter. A reviewer report is correction input, NOT the task. Do NOT treat a review result as
a target/precondition.

## THE OPEN PROBLEM (this is what to continue)

**A selected preset is reported APPLIED but is not ENFORCED.** Concretely: a session created
with `presetId: "heavy-review"` (`approve:true`, "every tool call asks before it runs") shows
`appliedPreset: heavy-review` and runs the model's `read` tool with **NO approval** - the
harness never sends `extension_ui_request`. Full record: `docs/issues/20261004-050000-*`.

### What is PROVEN (do not re-litigate)

1. **Not a stale adapter, not a missing upgrade.** The pi adapter's preset path
   (`writeActivePreset`, `installAgentPresetsExt`, the config/set restart) is UNCHANGED since
   `72e5a0e` (its first commit, v0.1.3) - the code the old TS hub drove.
2. **Driven directly (no hub), the adapter is correct**: `session/start` -> `config/set
   {presetId:heavy-review}` returns `applied.preset: "heavy-review"`, writes
   `<cwd>/.pi/agent-presets.json` = `{"heavy-review":{"approve":true}}`, places
   `<cwd>/.pi/extensions/agent-presets/index.ts`.
3. **The Rust hub matches the TS hub**: both send `session/start` -> `credentials/grant` ->
   `config/set {config.presetId}`. The hub CONFIRMS `applied.preset == requested` (fails the
   start otherwise), and here it PASSED -> the adapter reported applied honestly; the hub did
   its job.
4. **Instrumented REAL hub run** (a temporary probe, since reverted) printed:
   `SPAWN#1 activePresetId=null`, `SPAWN#2 activePresetId=null`, `CFG presetId=heavy-review`,
   `SPAWN#3 activePresetId=heavy-review envCfg=<the valid config path>`, and `PROMPT spawnId=3`.
   So the pi that RUNS the turn is the one with `AGENT_PRESETS_CONFIG` set to the right
   content; the extension is on disk; the argv has `--approve`.
5. **ISOLATION reproduces the GATE**: pi 1.0.0 spawned `--mode rpc --session ... --session-dir
   ... --approve --no-skills --skill ...`, with `AGENT_PRESETS_CONFIG` + `PI_CODING_AGENT_DIR`
   set (the exact hub shape), prompted to `read` a file -> **emits `extension_ui_request`
   (method "input", title "tool-review/v2")**. So the extension DOES gate under every condition
   reproducible outside the adapter.

### The single remaining unknown

In the real hub run the same inputs do NOT produce `extension_ui_request`. Everything the
contract/adapter/extension needs is present (env, files, argv). The unexplained difference is
inside the **adapter's own `child_process.spawn` of pi** - a runtime fact, NOT a contract or
code difference. **Next step (do this): instrument the adapter's `startPi` to dump the CHILD's
real environment and capture pi's extension-load diagnostics for ONE run, and diff it against
the working isolate.** Then fix the responsible side (adapter, if the child env/spawn differs;
hub, only if the adapter is given something wrong). Do NOT invent a new mechanism, and do NOT
relax the adapter's guard.

Candidate sub-hypotheses to check with that dump (none confirmed):
- the adapter's `env` object is built but something (a later spread) drops/reorders
  `AGENT_PRESETS_CONFIG` for the child;
- pi loads the extension but `AGENT_PRESETS_CONFIG` is not visible to the extension (env
  inherited vs rebuilt by the adapter's spawn options);
- the extension is loaded but `tool_call` for a `read` tool does not run its handler in this
  spawn (mode/flag interaction), unlike the isolate.

## ALSO OPEN (lesser)

- Preset is a hub convention the ADAPTER implements (ARCHITECTURE §5/§6: adapter owns
  preset/approval/plan/review). The hub's job is only: list, send `config.presetId`, confirm
  `applied.preset`, persist (§22). All verified working.
- Coverage gaps (no test): `/questions`, `model-providers/{id}/models/refresh`,
  `harnesses/{id}/connections/validate`, `plugins/registry/refresh`, `harnesses/{id}/auth*`,
  `extensions PATCH`, skills on the plugin-sourced model. See `docs/tasks/misalignment.md`.
- `docs/ROUTES-REVIEW.md`, `docs/review/*`, `docs/tasks/review-*.md` were DELETED (expired
  reviews). Do not recreate them. The review documents are gone; the design is
  `docs/ARCHITECTURE.md` + the contract + the ADRs (in prts-web).

## THIS SESSION'S WORK (committed, all real)

- Deleted all review docs; repointed ARCHITECTURE; wrote
  `docs/tasks/alignment-code-vs-goal.md` (capability-by-capability, with REAL test results) and
  `docs/tasks/misalignment.md` (observed reality only).
- Hub fixes (real): F1 (turn terminal read from the `turn_end` EVENT, bound per turn via
  `clientMessageId` + a per-turn oneshot - `crates/sessions/src/runtime.rs`,
  `crates/sessions/src/service.rs`); F5 (idle cancel 200); a busy turn admission is 409
  `session_busy`; the unconfirmed-cancel sweep judges by PROCESS LIVENESS, never a timer
  (`crates/db/src/sessions.rs`). Contract: `turn_end` now `requires clientMessageId`; the
  missing `plugins/PROTOCOL.md` (the contract's `proseAuthority`) was RESTORED.
- The pi adapter's `turn_end` now carries `clientMessageId` + `state` (UNCOMMITTED in
  prts-harness-pi).
- Real test suite (Python, `python tests/run.py [layer]`): 28 files / 234 checks, NO env gate,
  NO skip (a missing real dependency FAILS). Layers: contract, lifecycle, model, tools,
  interrupt, approvals, presets, concurrency, provider, skills, connections, harnesses,
  plugins. Last full run: **25/27 files pass; the 2 RED are the approval tests
  (`tests/approvals/*`) - the open problem above.**

## THE RULE THIS RUN MUST HOLD

- When a document is silent, DO NOT invent; stop and ask for a design decision.
- A "choice" question is often a downgrade in disguise - there is usually ONE correct answer;
  find it, do not offer二选一.
- NEVER `git checkout --`/reset/discard uncommitted work; no `_`-prefixed scratch dirs.
- A contract/ADR change needs the owner's review BEFORE it is written.
- Tests are REAL (no fake, no mock, no test-only switch); a missing dependency FAILS.
- Review results have a shelf life; they are NOT targets or preconditions.
- Do NOT add debug to shipped code to chase a bug; use the adapter's existing traces, or a
  temporary diagnostic you REVERT.

## NEXT ACTION

Instrument the adapter's `startPi` spawn (dump the child env; capture pi's extension-load
diagnostics) for one real hub run, diff against the working isolate, and fix the responsible
side. That is the ONE thing that will close `20261004-050000`.

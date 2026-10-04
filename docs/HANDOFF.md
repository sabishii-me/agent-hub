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

## RESOLVED THIS RUN: 20261004-050000 (a preset reported APPLIED but not enforced)

**ROOT CAUSE (proven by observation inside pi):** the adapter starts pi once WITHOUT the
preset (`session/start`, before `config/set` names it). That spawn's `agent-presets` extension
loaded with `review.on=false` and, on `session_start`, wrote `hub-review/state {asking:false}`
into the SESSION LOG. `config/set` then restarted pi WITH the preset (`review.on=true`), but
that spawn RESUMES THE SAME LOG, and the extension's `session_start` restored the newest
`hub-review/state` - the stale `{asking:false}` - OVER its own preset's `approve:true`. Same
process: load `review.on=true`, `tool_call` `review.on=false`. Gate off; tool ran unapproved.

**FIX (all committed):**
- Adapter `prts-harness-pi` `7a23419` (mirrored `prts-harness-jouzu` `235d5a0`): the extension
  records `hub-review/state` only when the run has a real review state (a preset in force, an
  explicit `/review on|off`, or a restored record). A pre-preset spawn leaves no trace. The
  restore rule is unchanged, so `/review on|off` and resume-in-last-mode still work.
- Hub `prts-hub` `7427344`: `list_approvals`/`list_questions` returned answered rows too (they
  are the PENDING set); and the reverse handler only treated a decision as a denial if it
  contained `reject` - the contract's vocabulary is `allow|deny|always`, so a bare `deny` was
  delivered as `approved:true` and the denied tool ran.
- Test `prts-hub` `c8f739d`: `tests/approvals/review-remains-switchable-after-a-preset.py`
  (on -> off -> on); goes RED before the adapter fix, GREEN after.

**RESULTS (real runs, hub `c8f739d` + pi `7a23419` + pi 1.0.0, real provider):**
`tests/approvals/*` 8/8 + 2/2 + 9/9; `tests/presets/*` green; **full suite 28/28 files**.

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

The preset/approval capability is now real and enforced end-to-end (20261004-050000 closed).
Continue §17: the next capability that is OPEN/unproven in `docs/tasks/alignment-code-vs-goal.md`
(see ALSO OPEN above). Keep the rule: when a document is silent, stop and ask - do not invent.

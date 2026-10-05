# HANDOFF — resume here (2026-10-05)

Read this, then `docs/tasks/alignment-code-vs-goal.md`, then the open issues below.

Hub HEAD: `628b4d8` (branch `feat/hub-modular-redesign`), working tree clean.
Plugin SHAs (branch `fix/runtime-placement`): pi `7a23419`, jouzu `89e17d4` (runtime 0.1.18),
dsh `580aca8`. The pi `turn_end clientMessageId` change is COMMITTED (`7a23419`).

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
- The pi adapter's `turn_end` carries `clientMessageId` + `state` (COMMITTED: pi `7a23419`).
- Real test suite (Python, `python tests/run.py [layer]`): 28 files / 234 checks, NO env gate,
  NO skip (a missing real dependency FAILS). Layers: contract, lifecycle, model, tools,
  interrupt, approvals, presets, concurrency, provider, skills, connections, harnesses,
  plugins. Latest full run: **28/28 files pass** (the keychain-leak fixture was fixed; see
  the 080000 section below).

## RESOLVED: 20261004-080000 (the OS store was full of our leaked test credentials)

The suite leaked OS credentials: every `Hub()` made a fresh temp data dir -> a fresh hub
instance id -> a fresh `<id>:provider-p.agent-hub:<id>` keychain entry, and `cleanup()` only
deleted the dir. Accumulated to 532 of 589 entries; the Windows store's per-user cap filled
and `CredWriteW` then returned error 8 for EVERY write, so the hub's boot probe failed and
every credential write was 501. NOT a broken vault (my earlier claim - retracted).

Fixed: cleaned the 532 entries (kept the user's 57) and `tests/lib/hub.py` `cleanup()` now
deletes each provider/connection it created via `/v1` (the production path) before the data
dir goes away, releasing the keychain entry. Commits: hub `f816076` (fix), `a8f772f` (issue
resolved), `628b4d8` (postmortem `docs/postmortems/20261005-000000-*`). Verified: provider
CRUD twice -> 13/13 each, entries 0 before/after; full suite 28/28 with a flat count. Reverted
the earlier `crates/secrets` name-length change (`a3b00db`) - it was based on a retracted
theory and fixed nothing.

## ALSO DONE: jouzu upgraded 0.1.13 -> 0.1.18

- `prts-harness-jouzu` `89e17d4`: plugin 0.1.9 -> 0.1.10, runtime pin -> 0.1.18 (bundles pi
  0.87.1); `runtime.sources.json` re-recorded. Real `/v1`: session active, turn ended;
  approvals 8/8 + 2/2 + 9/9 pass on the newer pi.

## OPEN (recorded, NOT fixed)

- `docs/issues/20261004-060000` - three jouzu suite failures. **WARNING: MISLABELED.** They
  were recorded as "jouzu adapter gaps" but NEVER located to jouzu code; they are UNLOCATED.
  Re-check and trace before believing the wording (they also predate the 0.1.18 upgrade).
- `docs/issues/20261004-070000` - `PATCH review:true` is reported applied but the next turn can
  run ungated (~4/20). A real, separately located defect (adapter `reviewCommand` treats pi's
  acceptance as the switch taking effect). `prts-harness-pi` `stash@0` holds an UNVERIFIED
  partial fix (4/20 -> 2/20); do not commit until the residual is fixed and verified.

## 070000 RESIDUAL (investigated 2026-10-05, NOT closed)

The stale-log half is FIXED (pi `096759f`, jouzu `63d57d7`): reviewCommand resolves only
from a NEW hub-review/state entry, and settle() no longer clobbers applied.review. Verified
pi 1.0.0: review-toggle 9/9, 20x loop 0/20, suite 32/32.

REMAINING on jouzu (pi 0.87.1): ~3/12. Isolated, NOT the review gate: the extension gates
correctly and sends approval_need; the adapter sees {approved:true,reason:"allowed"} while
the hub's reverse handler raised/resolved ONLY the step-1 approval - turn 3's was never
raised. So the failing approval_need is answered on a path that does not run the reverse
handler (stale waiter / wrong session runtime) - same family as 20261004-040000. See the
issue's UPDATE 2. NOT root-caused; the pi-1.0 path is unaffected.

## EXTENSION DELIVERY: pi FIXED; jouzu OWED (20261005-010000)

The owner: the EXTENSION was REDESIGNED and must NOT go into the workspace. The design
(`crates/extensions`; ARCHITECTURE 136/273; contract/adapter-v1 "exactly like extensions" for
skills): hub-owned path + DISCOVERY OFF. The adapter points the harness at the hub dir.

pi DONE (adapter `7b84cb0`, 0.1.10): spawns `pi --no-extensions --extension
<AGENT_HUB_INSTALLED_EXTENSIONS_DIR>/<id>`; the agent-presets config is written into the
hub-owned harness dir; `--approve` + the workspace copy are gone. Real: presets 2/2, approvals
8/8+2/2+9/9, full suite 32/32; a real run writes `agents/pi/agent-presets.json` and creates NO
workspace `.pi`. The gate never needed the workspace shape.

jouzu OWED: `jouzu-adapter.cjs` has the same wrong shape BUT jouzu is a FORK (its own file, its
own spawn `... 'pi', '--mode', ...`). Do NOT mirror the pi diff - change it on its own code and
verify on jouzu's own suite (`PI_PLUGIN_DIR=.../prts-harness-jouzu PI_HARNESS_ID=jouzu`).

Reassess (pi half) 050000/070000 now that the `--approve` discovery path is gone.

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
- A diagnosis needs a CONTROL (change one variable, hold state fixed); one observation is a
  guess, and it is reported as a guess, never as a cause.
- Enumerate the real STATE before theorizing (one `CredEnumerateW` ended a day of guessing).
- Suspect MY inputs first (my test, my run, what I put on the host) before the environment.
- NEVER experiment on the owner's system (no `git credential-manager`, `cmdkey`, registry edits).
- NEVER make a test green by weakening it; a red test pointing at a real defect is correct output.
- An unlocated failure is UNLOCATED; never attribute it to a component I did not trace it to.

## NEXT ACTION

1. jouzu extension delivery (20261005-010000): change `jouzu-adapter.cjs` ON ITS OWN CODE
   (it is a fork; do NOT mirror the pi diff), then verify on jouzu's own suite. Reassess the
   jouzu half of 070000 in the new shape.
2. Reassess the pi half of 050000/070000 now that `--approve` discovery is gone (pi 32/32 is
   green, but confirm the gate still holds under the explicit-`-e` delivery across repeats).
3. Re-trace 20261004-060000 (unlocated jouzu failures - not worded as a plugin defect until
   located).
4. Then resume section 17 via `docs/tasks/alignment-code-vs-goal.md` (the next OPEN/unproven
   capability). Suite is 32/32 and no longer pollutes the host.

Keep the rules above (control before cause; no SKIP; never experiment on the owner's system;
unlocated is unlocated; do NOT mirror pi<->jouzu).

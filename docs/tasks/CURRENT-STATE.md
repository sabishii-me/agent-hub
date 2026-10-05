# CURRENT STATE — read this first (durable handoff)

Updated 2026-10-05 at hub `f30442b` (branch `feat/hub-modular-redesign`). Working tree clean.

## The task (unchanged)

Implement the Rust agent-hub per the approved architecture, delivering REAL, usable `/v1`
capabilities **against the REAL adapters/plugins**. NOT: reviewer numbers, green tests, mounted
routes, or mock-driven "verification". §17 of `docs/ARCHITECTURE.md` is the order of work.

## The rule (repeated; do not break)

- A FAKE anywhere makes the whole chain false. No fake provider, no stub adapter, no mocked
  response. A missing REAL dependency FAILS the test - NO SKIP (SKIP enables 'fake green').
- NEVER `git checkout --` / `git reset --hard` / discard uncommitted work.
- No `_`-prefixed scratch dirs; use the real repos / a documented layout.
- Docs FIRST, then code. If a document lacks a definition, SAY SO; do not invent a schema.
- A change to `contract/` or an ADR needs the OWNER's review BEFORE it is written.
- Acceptance is REAL runs against real adapters, with the version combo recorded.
- A diagnosis needs a CONTROL (change one variable, hold state fixed); one observation is a
  guess, reported as a guess, never as a cause. Enumerate the real STATE before theorizing.
- Suspect MY inputs first (my test, my run, what I put on the host) before the environment.
- NEVER experiment on the owner's system (no git credential-manager, cmdkey, registry edits).
- An unlocated failure is UNLOCATED; never attribute it to a component not traced to it.

## Where things stand (2026-10-05)

- **Surface**: `/v1/surface` == the contract (74 == 74), no stub route.
- **Real acceptance**: pi (runtime 1.0.0, plugin `7a23419`) and jouzu (runtime 0.1.18 -> pi
  0.87.1, plugin `89e17d4`) answer real model turns; preset/approval/review is REAL on pi
(approvals/preset-gates 8/8, deny-blocks 2/2, review-toggle 9/9). Full suite `python
tests/run.py` = **32/32** (the layer list now covers plugins/harnesses/presets).
- **dsh**: BLOCKED (upstream runtime cannot boot here). docs/tasks/dsh-blocked.md.
- **HEADLINE FINDING (2026-10-05), docs/issues/20261005-010000**: the pi adapter delivers
EXTENSIONS the WRONG way - it copies the hub's snapshot into the USER WORKSPACE
(`<cwd>/.pi/extensions`) and spawns pi with `--approve`, so pi loads them by workspace
discovery. The design (`crates/extensions` doc; ARCHITECTURE 136/273; contract/adapter-v1
'exactly like extensions' for skills) is: hub-owned path + DISCOVERY OFF - the adapter
points the harness at the hub dir, i.e. pi `--no-extensions --extension <dir>`. The SAME
adapter already does this correctly for SKILLS (`--no-skills --skill <dir>`); extensions are
the deviation. The whole preset/review gate currently rides the wrong (workspace+--approve)
shape. NOT fixed - the change is an adapter redesign; scope was to be confirmed with the
owner. CHECK IT FIRST next session.

## Open defects (recorded, NOT fixed)

- **20261004-070000**: PATCH review:true could run the next turn ungated. pi 1.0.0 FIXED
  (0/20); jouzu residual ~3/12 = an approval answered `allowed` WITHOUT the hub raising it
  (adapter bus reply shows `{approved:true,reason:allowed}`; the hub's reverse handler never
  raised that turn). Same family as 20261004-040000. See the issue's UPDATE 2.
- **20261004-060000**: three jouzu suite failures - recorded as 'jouzu gaps' but NEVER
  located to jouzu code; treat as UNLOCATED, re-trace before believing the wording.
- **20261004-080000**: RESOLVED (the suite leaked 532 OS credentials and filled the store;
  fixed by cleanup() deleting what it created). See docs/postmortems/20261005-000000-*.
- **20261005-010000**: the extension-delivery deviation above (the headline).

## Docs map

README.md; docs/ARCHITECTURE.md (sec 17 order, sec 18 status); docs/tasks/alignment-code-vs-goal.md
(capability-by-capability, REAL results); docs/HANDOFF.md (resume here); docs/postmortems/*;
docs/issues/* (open + resolved); docs/tasks/misalignment.md; dsh-blocked.md; contract/v1.json
(the interface). The review documents were DELETED (expired) - do not cite docs/review/*.

## Command facts

- Build: `cargo build --workspace` (rebuild after crate changes; kill stray `agent-hub.exe`
  first or the link fails 'Access is denied').
- Run needs `AGENT_HUB_CONTRACT_DIR`, `AGENT_HUB_DATA_DIR`, `AGENT_HUB_ADDR`.
- Real suite: `python tests/run.py [layer]` (default runs every layer). Plugin is chosen by
  `PI_PLUGIN_DIR` / `PI_HARNESS_ID` (default pi at `E:/AI/ideas/prts-harness-pi`).
- Real plugin dirs: pi/jouzu/deepseek under `E:/AI/ideas/prts-harness-*`
  (pi `7a23419`, jouzu `89e17d4` runtime 0.1.18, dsh `580aca8`, on `fix/runtime-placement`);
  provider plugins under `E:/AI/ideas/prts-providers/*`.
- Do NOT let the suite leak OS credentials again: `tests/lib/hub.py` cleanup() now deletes
  what it created; if the keychain fills, that check regressed.

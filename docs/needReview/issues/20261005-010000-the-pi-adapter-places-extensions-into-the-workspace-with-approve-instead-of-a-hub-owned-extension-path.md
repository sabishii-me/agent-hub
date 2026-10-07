# 20261005-010000 — the pi adapter places extensions into the USER WORKSPACE with `--approve`, instead of pointing pi at the hub-owned extension path with discovery off

Recorded: 2026-10-05. Found by: reading the design after the owner's correction ("the extension
is not meant to go into the workspace; it was redesigned"). Owner: **PLUGIN**
(`prts-harness-pi`; the same shape is in `prts-harness-jouzu`). Status: **RESOLVED on pi and jouzu** - pi adapter `7b84cb0` (0.1.9 -> 0.1.10); jouzu adapter
`c007b35` (0.1.10 -> 0.1.11). jouzu was changed on its OWN code (a fork, NOT a mirror).

## The design (what the shape must be)

- `crates/extensions/src/lib.rs`: *"before a harness runs the hub writes that harness's
  selected extension directories into its **data dir**, and the adapter places them with
  **discovery off**."*
- `docs/ARCHITECTURE.md:136`: the placement rule is **hub-owned path, discovery off**;
  `:273` - a **hub-owned** directory and turning discovery off.
- `contract/adapter-v1.json` (skills, and it says **"exactly like extensions"**): the hub
  installs into `<DATA_DIR>/agents/<harness>/...` and hands the path over as
  `AGENT_HUB_INSTALLED_*_DIR`; **"the adapter only points its harness at that directory —
  pi/jouzu with `--no-skills --skill <dir>` (so the user's own ... directories stay out of a
  hub-managed session)"**.
- The hub already builds an **immutable snapshot** and hands
  `AGENT_HUB_INSTALLED_EXTENSIONS_DIR` (`crates/extensions/src/service.rs::install_for_harness`;
  `crates/sessions/src/runtime.rs:682`).

So an extension must be delivered as `pi --no-extensions --extension <hub dir>` (discovery off,
explicit path) - the same shape the adapter ALREADY uses for skills
(`--no-skills --skill <dir>`).

## What the pi adapter does instead (the deviation)

`pi-adapter.cjs`:
- spawns pi with **`--approve`** (line ~497) "so its project-local resources load" - i.e. it
  trusts the workspace so pi's normal discovery loads what the adapter copied there;
- `placeExtension(id, cwd)` copies the hub snapshot into **`<cwd>/.pi/extensions/<name>`**
  (`cwd` = `AGENT_HUB_CWD`, the USER's workspace).

That is exactly the "put the extension into the workspace" shape the design rejects. It also
means the hub's extension lands in the user's project directory, and the whole preset/approval
path rides on pi's workspace discovery + `--approve` rather than on the hub-owned path.

Skills already do it right (`--no-skills --skill <AGENT_HUB_INSTALLED_SKILLS_DIR>`); extensions
do not.

## The fix (adapter-owned, shape change)

- Spawn pi with **`--no-extensions`** and **`--extension <AGENT_HUB_INSTALLED_EXTENSIONS_DIR>/<id>`**
  for each selected extension (the snapshot is complete/immutable), instead of `--approve` +
  copying into `<cwd>/.pi/extensions`.
- Stop writing the extension (and the `agent-presets.json` config, and the plan extension) into
  the user workspace; read from the hub-owned dirs (`AGENT_HUB_INSTALLED_EXTENSIONS_DIR`,
  `AGENT_HUB_PRESETS_DIR`).
- Keep the workspace as the harness's cwd for its FILE/command tools, but do not rely on it for
  resource discovery.

Likely consequences to re-verify after the change: the preset gate (050000/070000) rides the
explicit `-e` path now, not `--approve` discovery - re-run `tests/approvals/*` and
`tests/presets/*` on pi and jouzu.

## Why this matters for 050000/070000

The preset/review gate has been driven entirely through this workspace+`--approve` path. If the
delivery is wrong, the gate's fragility (the stale `hub-review/state`, the review race, the
jouzu approval binding) may be symptoms of the wrong delivery shape, not only of the log/state
handling. Fix the delivery shape FIRST, then re-assess the gate defects.

## RESOLUTION (pi, 2026-10-05)

`pi-adapter.cjs` now spawns pi with **`--no-extensions --extension <AGENT_HUB_INSTALLED_EXTENSIONS_DIR>/<id>`**
for each selected extension (the same shape the skills path already used), and writes
`agent-presets.json` into the **hub-owned harness dir** (`AGENT_HUB_HARNESS_DIR`) instead of
`<cwd>/.pi`. `--approve` is gone; the copy-into-workspace (`copyTree`/`placeExtension`) is gone;
the mid-session install*() calls are gone (the extension is loaded at spawn, so a preset restart
re-adds `-e` and plan/review drive the loaded extension).

Evidence (real, pi runtime 1.0.0):
- presets 2/2, approvals 8/8 + 2/2 + 9/9, full suite **32/32** - the SAME real gate now passes
  with discovery OFF and no workspace placement.
- a real run writes `agent-presets.json` under `<DATA_DIR>/agents/pi/agent-presets.json` and
  creates NO `.pi` in any workspace (walked the whole hub data dir).

So the gate never needed the workspace+`--approve` shape; the correct delivery carries it too.
Consequence to reassess: this removes the `--approve` discovery path 050000/070000 rode on -
re-check those (pi half) in this shape.
JOUZU: the same WRONG shape exists in `jouzu-adapter.cjs`, but jouzu is a FORK (its own file, its
own spawn: `... 'pi', '--mode', ...`), so its fix is a separate change to verify on jouzu's own
suite - NOT a mirror of this diff.

## RESOLUTION (jouzu, 2026-10-05)

`jouzu-adapter.cjs` now spawns `jouzu ... --mode rpc ... --no-extensions --extension
<AGENT_HUB_INSTALLED_EXTENSIONS_DIR>/<id>` and writes `agent-presets.json` into the hub-owned
harness dir. Changed on jouzu's own code (its spawn carries an extra `pi` arg; it re-roots
`JOUZU_HOME`), NOT by mirroring the pi diff. adapter `c007b35` (0.1.11).

Evidence (real, jouzu 0.1.18): presets 2/2, approvals 8/8 + 2/2 + 9/9; a real run writes
`agents/jouzu/agent-presets.json` and creates no workspace `.pi`.

CONTROL on the pre-existing jouzu failures: the parameterized suite has 4 failures on jouzu
(cross-talk 1/3, send-after-cancel 7/8, fork 6/7, model-identity 6/7). Stashing this change
(back to `63d57d7`) and re-running gives the SAME failures, cross-talk 1/3 x3 both before and
after - so this change causes none of them; they are 20261004-060000.

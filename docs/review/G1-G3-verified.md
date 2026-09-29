# PR #12 revision — what was actually verified (vs. what remains a gate)

Baseline before revision: `8ab9dafaf14c4b905d4c4a94e8162211860125ea`.
Revision commit: `1847048c936c6e24c4340715366774bec4f23172`.

This file separates **verified now** from **design-only / not executed**, so a re-review does not
have to guess which parts carry evidence.

## Verified now (static / reproduction; no product Rust run - there is no Rust code yet)

### G3 — the projection defects are real (reproduced)

`node scripts/emit-openapi.mjs` regenerates `contract/openapi.json` byte-identically modulo line
endings, so **the projection is not stale** and its refs are not all broken (confirms TASK-039-F03).
The defects are in the generated content, not in staleness:

| defect | count in `contract/openapi.json` |
|---|---|
| `"type": []` (empty type array) | **4** |
| `"nullable": true` (not JSON-Schema-2020-12 null semantics) | **12** |
| `"type": "binary"` inside an `application/json` schema | **1** |
| `"const": "manual"` (the source is `'manual'|string?`) | **1** |
| **PATCH `/v1/sessions/{id}`**: source declares 8 **optional** fields (`modelId?`, `presetId?`, `disabledTools?`, `plan?`, `review?`, `modelProviderId?`, `thinkingLevel?`, `title?`), the projection marks **all 8 required** | 8/8 |

Reproduce: `node -e` counts above, or read `contract/openapi.json` lines 1393/2030/2986 (type:[]),
7908/7912/7916 (nullable), 8050 (binary); PATCH body at `paths['/v1/sessions/{id}'].patch`.

**Conclusion:** `contract/v1.json` is a compact DSL, not JSON Schema, and handing the current
fragments to a validator would over-constrain (PATCH) and mis-type. ARCHITECTURE §9 now states the
single normalization authority and that this projection is not a trustworthy validation base. The
fix belongs to the owning repository; this PR states the boundary only.

### G1 — the security section's factual premises (reproduced)

| premise | check |
|---|---|
| all three harnesses run as Node | pi `dist/bundle/cli.js`, jouzu `dist/cli.js`, dsh `lib/bin.js` all start `#!/usr/bin/env node` |
| the `node:fs` hook can intercept a skill read | a `--require` hook intercepted `readFileSync("skills://…")` in an ESM Node 24 child (mechanism probe) |
| the agent's `write` is not confined | pi `dist/core/tools/write.js`: `Path to the file to write (relative or absolute)` + `resolveToCwd(path, cwd)` |
| the hook does not confine child_process | pi exposes a bash tool; a `node:fs` interposer does not touch it |

**Conclusion:** the facts the security section rests on are confirmed; the **guarantee** is not.
ARCHITECTURE §7 now says placement is not an authorization boundary, names the missing OS-principal
precondition, and marks the security claim as an **open verification gate**.

## NOT verified (design-only until Rust exists; runtime gates stay open)

- **G2** (recovery rules): stated as a table; no crash/restart run exists (no Rust).
- **G4** (async acceptance/result/SSE): stated as semantics; no `/v1` crash/restart/concurrency run.
- **G5** (adapter/skills/session boundaries): real-artifact skills closure and cross-language SDK
  not run.
- **G6** (concurrency numbers, bounded acceptance): ADR-0009's >= 100 connections / thousands idle /
  two heavy operations are **not measured**; the numbers in review are suggestions, not an SLO.
- **G1** runtime acceptance: absolute-path write, shell/child, links, reboot load chain, and
  management/approval access by an agent identity - not executed (needs a real attributable artifact).

## Why this is the right boundary

The report's own split applies: **design revisions before merge; runtime acceptance after a real
artifact exists.** This revision does the first and does not fake the second. A re-review can accept
the design changes and mark the runtime items as gates.

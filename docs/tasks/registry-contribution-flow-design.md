# Plugin contribution flow: separate repos -> reviewed PR -> official registry (DESIGN PROPOSAL)

Status: PROPOSAL for owner review. Nothing implemented, nothing published, no contract change.

## The goal (owner, 2026-10-05)
Plugins live in their OWN repos. Adding one to the official catalog must be a reviewed, checked
CHANGE (like Dify's plugin marketplace: a PR that passes CI and a review), NOT a manual step someone
remembers. Later plugin additions must not be error-prone or hard to configure.

## The real topology (measured)
- `sabishii-me` is a GitHub ACCOUNT (not an org). Plugin repos already exist:
  `agent-hub-harness-adapter-{pi,deepseek,jouzu}`, `agent-hub-model-provider-{compatible,deepseek,
  shisa}` — one repo per plugin.
- `agent-hub` is the hub repo (holds `registry.json` + `scripts/pack-plugins.mjs`). No CI workflows
  today.
- The hub compiles in ONE registry address:
  `https://github.com/sabishii-me/agent-hub/releases/download/registry/registry.json`.

## The principle
**The registry is a reviewed ARTIFACT, not a hand-edited file and not an automated push.** A new
plugin enters the catalog by a PR to the hub repo that carries only the registry ENTRY (derived from
the plugin's own manifest/release), and CI + a reviewer decide. The plugin's own repo is where the
plugin's code and its release live; the hub repo is where the ALLOW-LIST lives.

## The flow (one path; a plugin author does NOT need the hub checked out)

1. Author builds and releases their plugin in ITS OWN repo:
   `node scripts/pack-plugins.mjs --write --publish --yes` (in the plugin repo) -> the zip + icon are
   uploaded to that repo's own GitHub Release `v<version>`; a `entry.json` fragment is produced.
2. Author opens a PR to `sabishii-me/agent-hub` that **adds one file**:
   `registry.d/plugins/<pluginType>-<id>-<version>.json` (the release entry, DERIVED — id, pluginType,
   name, summary, capabilities, icon, one version {version,url,sha256,size}).
   The PR touches NOTHING else. No hand-editing `registry.json`.
3. **CI (the CHECK) runs on the PR** and must pass:
   - the entry's JSON matches the schema (schema 2 rules: required fields, https url, 64-hex sha256);
   - `(id, pluginType, version)` does not already exist with different bytes;
   - the url is reachable AND its bytes hash to the stated sha256 (download + verify) — the release
     exists and is what the entry claims;
   - the plugin's `manifest.json` (fetched from the plugin repo at that tag) agrees with the entry's
     id/pluginType/version;
   - the name/icon are present (no silent missing icon);
   - the runtime is a DECLARATION, not bytes (no runtime in the zip).
4. **A REVIEWER (CODEOWNERS on `registry.d/`) approves.** Only then can it merge.
5. **On merge to `main`, a workflow COMPILES the fragments**: `registry.d/plugins/*.json` ->
   `registry.json` (sorted, deduped, older versions kept) -> published as the single `registry`
   release asset, overwritten in place (the URL never moves). Compilation is DETERMINISTIC from the
   fragments: `registry.json` is a build output, not a source file a human edits.
6. The desktop/hub pick up the new catalog on the next `registry/refresh`. No hub release needed.

## Why this shape (each avoids a concrete failure)
- **A PR, not a push**: an automated publish would let a compromised plugin repo inject itself; a
  reviewed PR puts a human between "new repo" and "official catalog".
- **Fragments, not a hand-edited registry**: two authors adding plugins never conflict on one file;
  `registry.json` is generated, so it can never drift from the entries that were reviewed.
- **CI downloads and hashes the release**: the digest is verified BEFORE it is trusted, closing the
  "entry lies about its artifact" gap at review time instead of at a user's install.
- **CODEOWNERS on the fragments dir**: the review is enforceable (branch protection requires it),
  not a convention.
- **The address never moves**: the hub keeps one compiled URL; publishing a plugin never needs a hub
  release; the desktop's pointer never goes stale.

## What stays the same
- The hub's read path (boot loads the local copy; refresh GETs the fixed address; install reconciles
  url+sha256). Unchanged.
- The plugin's own repo, zip format, and `manifest.json` as the single description of a plugin.
- `schema 2` invariants (from registry-update-and-release-design.md) are what CI enforces.

## What needs the owner's review BEFORE it is written
1. **Fragments in the hub repo (`registry.d/`) + a compile step**: this changes how `registry.json`
   is produced (build output, not a tracked source). It is contract-adjacent (the desktop reads the
   compiled `registry.json`). OWNER review.
2. **Schemas/CI rules** (what a PR must pass) — the enforcement contract.
3. **CODEOWNERS / branch protection** on `registry.d/` (a repo-setting decision, the owner's).
4. Whether `schema: 1 -> 2` goes in the same change or after.

## Not in this proposal
- No GitHub org migration (the account can host repos + CODEOWNERS; an org is a separate call).
- No id-only install change.
- Nothing published now; the `registry` release stays unpublished until this is accepted.

## Alternative considered and rejected
**Publish straight from each plugin repo to the shared registry** (a plugin's CI overwrites the
catalog asset). Rejected: a plugin repo could add itself to the official allow-list without review;
the catalog would have no single reviewed source of truth; concurrent publishes would race on one
asset. The PR+fragments flow makes review and determinism structural.

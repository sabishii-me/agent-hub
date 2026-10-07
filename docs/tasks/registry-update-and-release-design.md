# Registry: how it is structured, updated and published (DESIGN PROPOSAL — owner review)

Status: PROPOSAL. Nothing here is implemented or published. A change to `contract/` or an ADR needs
the owner's review BEFORE it is written. This document states ONE answer, not a menu.

## The goal (owner's words, 2026-10-05)
Adding a plugin later must NOT be error-prone or hard to configure. The registry is the hub's
allow-list and the desktop's catalog; it must have one obvious shape, one obvious update path, and
one obvious publish step.

## What already exists (measured, not remembered)
- `registry.json` is git-tracked, `schema: 1`, `{schema, note, plugins: [...]}` (`pack-plugins.mjs`
  writes it; the hub reads it via `Registry::parse`).
- Each plugin is `{id, pluginType, name, summary, capabilities, icon, versions:[{version,url,sha256,
  size,releasedAt}]}`. Identity is **(pluginType, id)** — a `harness-adapter` and a `model-provider`
  may share an id.
- `scripts/pack-plugins.mjs` generates the registry FROM each plugin's own `manifest.json` (its
  `release.repository`), zips the plugin (runtime excluded), hashes it, and with `--write` rewrites
  the entry; with `--publish --yes` it uploads each zip to that plugin's own GitHub Release and
  overwrites the hub's `registry` release asset in place.
- The hub compiles in ONE address:
  `https://github.com/sabishii-me/agent-hub/releases/download/registry/registry.json`. A release
  build does NOT read any env override; a dev build may via `AGENT_HUB_REGISTRY_URL`.
- The hub REFRESHES (`POST /v1/plugins/registry/refresh`) at a user action; install RECONCILES the
  requested url+sha256 against the loaded registry (`Registry::authorizes`).

## Problems to fix (each is concrete)
P1. **Address drift in the contract.** `contract/openapi.json` still describes registry/refresh as
    "Read the configured registry URL (AGENT_HUB_REGISTRY_URL)". The code now compiles the address in.
    One of them is wrong; the contract must state the fixed-at-build-time rule.
P2. **`schema: 1` has no validator.** A malformed registry (missing `versions`, a `url` that is not
    https, a duplicate `(pluginType,id,version)`) is only caught at install time. Adding a plugin
    should fail at BUILD time, not at a user's install.
P3. **The hub's own release of the registry is decoupled from the hub release** (good) but the
    generator that publishes it lives in the hub repo and needs the plugin repos checked out locally
    (`--plugins`). Adding a plugin from a new repo is a manual, remembered step.
P4. **`icon` is a release-asset URL**, uploaded beside the zip; a missing/renamed icon asset is
    silent. The `--publish` icon path was silently wrong once already (the `path.sep` bug it now
    fixes).
P5. **No "how to add a plugin" runbook** in one place. The rules exist as comments inside
    `pack-plugins.mjs`.

## The design (ONE shape, ONE update path, ONE publish step)

### A. The registry document (schema 2 — the one change that needs the contract)
Keep `{schema, note, plugins:[...]}`. Tighten it so a BUILDER can reject a bad entry and a READER can
trust it:
- `schema: 2`.
- every entry: required `id`, `pluginType`, `name`, `versions` (non-empty); `versions[*]` required
  `version`, `url` (https), `sha256` (64 hex), `size`. `summary`/`capabilities`/`icon` optional (two
  official providers publish no icon). The entry does NOT carry `runtime`.
- `(id, pluginType)` unique; within an entry, `version` unique; `sha256` unique across versions.
- `url` must start `https://`; the digest is the anchor (already how install reconciles).
- the hub's reader is unchanged in shape (`plugins` array) — schema 2 is additive; the hub keeps
  accepting schema 1 (read verbatim), so a published registry stays backward-compatible.
This is CONTRACT-ADJACENT (the desktop reads `plugins`), so it needs the owner's review. Proposed,
not written.

### B. Adding a plugin (the error-proof part)
ONE file per plugin: its own `manifest.json`, in the plugin's own repo, already the source of truth:
```json
{ "id":"<id>", "pluginType":"harness-adapter|model-provider", "name":"…", "summary":"…",
  "version":"<x.y.z>", "capabilities":[...],
  "release": { "repository":"sabishii-me/<repo>", "exclude":[...] } }
```
Nothing else to configure. The registry entry is DERIVED, never hand-edited. New plugin = new repo
with a manifest + a `--plugins <dir>` entry in the publish command (or a small `plugins.txt` list of
plugin repo dirs checked out under a conventional path). The generator already walks a directory of
plugins; the runbook names that directory.

### C. Updating (publish) — the one path
`node scripts/pack-plugins.mjs --write --publish --yes --registry-repo sabishii-me/agent-hub`
(plugin dirs from `--plugins`). It:
1. zips each plugin (runtime excluded), computes sha256;
2. uploads the zip (+icon) to THAT plugin's own GitHub Release `v<version>`;
3. rewrites `registry.json` (keeps other entries verbatim, keeps older versions, refuses to rewrite a
   published version's bytes);
4. overwrites the hub's single `registry` release asset in place — the URL never moves.
A hub release is NOT required to ship a plugin. A plugin release is NOT required to ship the hub.

### D. Verifying before publish (new, closes P2)
A `--check` mode (or a validator the generator runs) that fails the build when an entry violates
A's rules, when a manifest lacks `id`/`pluginType`/`version`, or when two entries collide on
`(id, pluginType, version)`. This is the "not error-prone" guarantee: the mistake is caught before it
reaches a user.

### E. The hub's read path (unchanged)
- boot: load `<DATA_DIR>/registry.json` (the local copy); refresh replaces it.
- refresh: GET the fixed address (or the dev override), parse, store; 502 `registry_unavailable` on
  any failure.
- install: reconcile `url`+`sha256` against the loaded registry; 403 `plugin_not_in_registry`
  otherwise. No caller and no env can point a release hub at a foreign registry.

## What needs the owner's review BEFORE it is written
1. `schema: 2` (the validator's rules) — a contract/registry-shape change.
2. the contract text for registry/refresh (fix P1: fixed-at-build-time address).
3. the conventional `--plugins` directory / `plugins.txt` list (the runbook's anchor).
Everything else (the generator modes, the runbook doc) is tooling/docs and can follow.

## Not in this proposal
- Do NOT add an id-only install to the contract (an unreviewed proposal elsewhere); install still
  names a url+sha256 that reconciles against the registry.
- Do NOT publish anything now. The official `registry` release stays unpublished until this design is
  accepted.

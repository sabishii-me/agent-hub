# Plugin contribution flow: any repo -> reviewed ENTRY -> official registry (DESIGN PROPOSAL)

Status: PROPOSAL for owner review. Nothing implemented, nothing published, no contract change.

## The goal (owner, 2026-10-05)
Plugins live in their OWN repos - **and NOT all of those repos are in our org.** Adding a plugin to
the official catalog must be a reviewed, checked CHANGE (like Dify's marketplace: submit -> CI check
-> human review -> appears in the official catalog), NOT a manual step, and NOT something that
requires us to control the plugin's repository.

## The fact that decides the design
Most plugins will be THIRD-PARTY repos: another user's or org's GitHub repo (or GitLab/self-hosted),
where we have NO write access, cannot add CI, and cannot enforce anything. So the reviewed unit is
NOT "a PR in the plugin's repo". It is **the registry ENTRY** - the `{id, pluginType, version, url,
sha256}` record - which the AUTHOR submits to US, and WE review. We never need rights on their repo.

This is exactly what ADR-0007 already fixes: **a plugin is consumed as an ARTIFACT (url + sha256),
not as a repository**; one repo may publish many artifacts; the isolation posture is ORDINARY (a
review of the input's declared shape), deliberately NOT a signing/attestation scheme. So the review
is a review of the ENTRY + the BYTES it points at, not of the author's source tree or CI.

## What is in our org vs not
- **First-party** (our org, `sabishii-me`): we can use org teams, CODEOWNERS, rulesets, required
  workflows. Convenient, but only for our own plugins.
- **Third-party** (anywhere): we have nothing. The flow below must work with ZERO cooperation from
  the plugin's repo beyond "it hosts a release at a URL".

## The flow (ONE path; works for first-party AND third-party)

1. **Author releases the plugin in THEIR repo, however they like.** The artifact is
   `<pluginType>-<id>-<version>.zip` at some URL, with a `manifest.json` inside stating id,
   pluginType, name, capabilities, version. They compute the sha256. Nothing about their repo/CI is
   ours.

2. **Author submits ONE registry ENTRY to us** - by a PR to the hub repo adding
   `registry.d/plugins/<pluginType>-<id>-<version>.json`, OR by an ISSUE with the same JSON when the
   author cannot/should not open a PR. The entry is:
   ```json
   { "id":"x", "pluginType":"harness-adapter", "name":"X", "summary":"…",
     "capabilities":[...], "icon":{"light":"…","dark":"…"},
     "version":"1.2.3", "url":"https://…/harness-adapter-x-1.2.3.zip",
     "sha256":"<64 hex>", "size":12345 }
   ```
   This is the ONLY thing submitted. Our repo; our review; their repo untouched.

3. **OUR CI (the CHECK) runs on the submission and must pass:**
   - the entry matches the schema (required fields; `url` is `https://`; `sha256` is 64 hex);
   - `(id, pluginType, version)` is unique; a re-submitted version with a DIFFERENT sha256 is a
     conflict (a published version is never silently rewritten);
   - **the URL is downloaded and its bytes hash to the stated sha256** - the artifact exists and is
     what the entry claims (this is the whole point: we verify the DIGEST, not the source);
   - the downloaded zip's `manifest.json` AGREES with the entry (same id, pluginType; the version is
     the manifest's `version` OR its `package.json.version` - the same fallback the builder uses,
     because the three official model-providers declare no `version`) - so the entry cannot claim one
     thing and ship another;
   - the zip contains NO `runtime/` (the runtime is a DECLARATION fetched at install time, per the
     existing rule) and no path escapes;
   - IF an `icon` is stated, both light and dark resolve to an image (the icon is OPTIONAL: two
     official providers publish none); a declared-but-unresolvable icon is an error (closes the
     silent-drop bug the pack tooling already hit);
   - the artifact is immutable-addressable (the URL is a release asset that will not be overwritten
     - a moving URL with a pinned digest is a trap; if it is a moving URL, reject).
4. **A human REVIEWER approves.** For entries under our org, CODEOWNERS + an org `registry-review`
   team + rulesets make this REQUIRED. For third-party entries, the submission is a PR/issue in OUR
   repo, so the same rule (a maintainer must approve) holds - we do NOT need rights on their repo.
   The review is bounded by ADR-0007: we check the declared shape and the bytes' digest, NOT their
   source (ordinary isolation is the decided posture).

5. **On merge to `main`, OUR workflow COMPILES the fragments**: each fragment is ONE version;
   compilation GROUPS fragments by `(pluginType, id)` and MERGES each version into that entry,
   KEEPING any older version already in `registry.json` that is not superseded (the official registry
   keeps 5 adapter versions / 3 provider versions, which no longer appear in any manifest - so the
   compiled registry is NOT a pure function of the current manifests). Result sorted by pluginType,id
   and deterministic -> published as the single `registry` release asset, overwritten in place (URL
   never moves). `registry.json` becomes a BUILD OUTPUT, not a hand-edited source. The registry entry
   does NOT carry `runtime` (the runtime declaration lives inside the artifact's manifest, read at
   install time) - see the review doc.

6. The hub/desktop pick it up on the next `registry/refresh`. No hub release needed.

## Why this shape holds for BOTH kinds of plugin
- The reviewed unit is an ENTRY we host, so **third-party repos need no cooperation** - the author
  only needs "a release at a URL".
- We verify the DIGEST, so we do not need their source or CI to trust the bytes.
- One fragment per (plugin, version) means two authors (in different orgs) never conflict on one
  file, and the compiled `registry.json` can never drift from what was reviewed.
- The address never moves: publishing a plugin never needs a hub release.

## Enforcement (available because sabishii-me IS an org) - applied to OUR repo only
- `registry.d/` has CODEOWNERS pointing at a `registry-review` team; org rulesets on `main` require
  the check + a review. This gates OUR repo, which is where the allow-list lives - independent of
  where the plugin repo is.
- Optionally, an org-required workflow can run the SAME check on first-party plugin repos' PRs, but
  that is a convenience for our repos, never a requirement on third parties.

## What stays the same
- The hub's read path (boot loads the local copy; refresh GETs the fixed address; install reconciles
  url+sha256). Unchanged.
- ADR-0007: artifact = (url, sha256); a repo may publish many; ordinary isolation, no signing scheme.
- `schema 2` invariants (registry-update-and-release-design.md) are what CI enforces.

## What needs the owner's review BEFORE it is written
1. **Fragments (`registry.d/`) + a compile step** making `registry.json` a build output (contract-
   adjacent: the desktop reads it).
2. **The CI rule set** above (what a submission must pass) - the enforcement contract.
3. **CODEOWNERS / `registry-review` team / org rulesets** on `registry.d/` (an org setting).
4. **Submission channel**: PR-only, or PR + issue-fallback for authors who cannot open a PR. (A
   third-party author CAN open a PR to a public repo without write access, so PR-only may suffice;
   the owner decides.)
5. Whether `schema 1 -> 2` rides the same change.

## Not in this proposal
- No requirement that a plugin repo be in our org; no rights on third-party repos.
- No signing/attestation gate (ADR-0007 decides ordinary isolation).
- No id-only install change.
- Nothing published now; the `registry` release stays unpublished until this is accepted.

## Alternative considered and rejected
**Each plugin repo's own CI overwrites the shared registry asset.** Rejected: a third-party repo
cannot be given write to our catalog; even for first-party repos it removes review (a compromised
repo injects itself) and races on one asset. The submitted-entry flow keeps review and determinism
in OUR repo, which is the only place we control - and works for repos we do not control.

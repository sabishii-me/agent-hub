# Review of the registry design against the OFFICIAL plugins (measured)

Status: REVIEW of `registry-contribution-flow-design.md` + `registry-update-and-release-design.md`.
No code, no contract, no publication. The design is revised where the real plugins contradict it.

## The six official plugins, as they actually are

| pluginType | id | manifest `version` | package.json | registry newest | registry old versions | `icons` in manifest | `icon` in registry | `release.repository` |
|---|---|---|---|---|---|---|---|---|
| harness-adapter | deepseek | 0.1.8 | 0.1.3 | 0.1.8 | 0.1.7..0.1.4 (4) | logo.svg/logo.svg | yes | agent-hub-harness-adapter-deepseek |
| harness-adapter | jouzu | 0.1.11 | 0.1.4 | **0.1.9** | 0.1.8..0.1.5 | logo.png/logo.png | yes | agent-hub-harness-adapter-jouzu |
| harness-adapter | pi | 0.1.10 | 0.1.3 | **0.1.8** | 0.1.7..0.1.4 | logo-light/dark.svg | yes | agent-hub-harness-adapter-pi |
| model-provider | compatible | absent | 0.1.3 | **0.1.2** | 0.1.1, 0.1.0 | absent | **no** | agent-hub-model-provider-compatible |
| model-provider | deepseek | absent | 0.1.2 | 0.1.2 | 0.1.1, 0.1.0 | logo.svg/logo.svg | yes | agent-hub-model-provider-deepseek |
| model-provider | shisa | absent | 0.1.2 | 0.1.2 | 0.1.1, 0.1.0 | absent | **no** | agent-hub-model-provider-shisa |

Every newest release was fetched and re-hashed: **all six match their registry sha256 and size**
(deepseek 87114, jouzu 635045, pi 54566, compatible 2320, deepseek-provider 2450, shisa 5665). So the
published bytes are correct; the shape is what needs care.

## What the evidence CHANGES in the design (the design was wrong or incomplete here)

D1. **`version` is not always in the manifest.** The three providers have NO `version`; the
    generator falls back to `package.json.version`. So a CI rule "entry.version == manifest.version"
    would reject every provider. **Revised rule:** the version is the plugin's own declared version
    OR its `package.json.version`; CI must accept the same fallback the builder uses, and must
    require exactly one of them (never silently pick neither).

D2. **The entry is NOT a pure function of the current manifest.** The registry keeps OLD versions
    (5 for adapters, 3 for providers) that no longer appear in the manifest. So "one fragment per
    submission = current version" is right, but the COMPILED registry must MERGE a submitted version
    into the existing entry and KEEP the older ones. My earlier "compile the fragments" text implied
    the fragments are the whole entry; they are not - they are versions ADDED to an entry.
    **Revised:** fragments are per (pluginType, id, version); compilation groups them and preserves
    any older version already in `registry.json` that is not superseded.

D3. **Drift is real and is the exact failure to prevent.** The local trees are AHEAD of the registry:
    jouzu manifest 0.1.11 vs registry 0.1.9; pi manifest 0.1.10 vs registry 0.1.8; compatible
    package.json 0.1.3 vs registry 0.1.2. A human cannot tell from the manifest alone what is
    published. This CONFIRMS the design's premise (the registry must be a reviewed, checked artifact,
    not a hand-edited file), and ADDS a requirement: **the submission must state the version
    explicitly and CI must verify that the version's URL exists and matches** - never infer "latest".

D4. **`icon` is optional and asymmetric.** Two providers (compatible, shisa) have no icons and no
    `icon` key; the others have both light and dark. So a CI rule "icon required" is wrong.
    **Revised:** `icon` is optional; IF present, BOTH light and dark must resolve (the builder already
    fills the missing one from the other); a declared-but-unresolvable icon is an error (closes the
    silent-drop bug the pack tooling already hit once).

D5. **`runtime` is NOT in the registry entry.** The adapter manifests declare `runtime
    {package,version,command}`, but the published registry entries carry NO `runtime` field. My
    earlier comment (in pack-plugins.mjs) that "the registry entry carries the runtime as a
    declaration" is NOT what the data shows. **Revised:** the registry entry does NOT carry the
    runtime; the runtime declaration is inside the artifact's manifest, read at install time. The
    design must not invent a `runtime` field in the registry. (If the intent was to carry it, that is
    a separate change to approve - the current data says it is not carried.)

D6. **A repo CAN publish several artifacts; a plugin CAN be a single small zip.** Provider zips are
    ~2-6 KB; adapters ~55-635 KB. The check must not assume a size floor. `size` is present and
    correct for all six; keep it as a checked field (compare against the downloaded byte count).

D7. **`releasedAt` is present (`YYYY-MM-DD`) and is the only date.** The design should keep it and
    treat it as the release date recorded at build time, not a promise.

## What the evidence CONFIRMS (keep)
- The registry lists `(pluginType, id)` entries with `versions:[{version,url,sha256,size,releasedAt}]`
  - matches the design.
- `url` is `https://github.com/<repo>/releases/download/v<version>/<type>-<id>-<version>.zip` for all
  six - a release asset on the plugin's OWN repo. The design's "any repo, any host" holds (these
  happen to be our org; a third party would be theirs).
- The digest is the anchor; all six verify. The design's "download and hash the URL" is the right
  check and would have caught any of D1/D3 if the entry lied.
- `(pluginType, id)` identity (two `deepseek`) - matches.

## Revised CI rule set (the enforcement contract, after this review)
For a submitted entry `{id, pluginType, name, summary?, capabilities?, icon?, version, url, sha256, size}`:
1. required: `id`, `pluginType` (in the fixed set), `version`, `url`, `sha256`, `size`;
2. `url` is `https://`; `sha256` is 64 hex; `size` a positive integer;
3. **download `url`; its byte count == `size`; sha256(bytes) == `sha256`;**
4. the zip's `manifest.json`: its `id`/`pluginType` equal the entry's; its version is its
   `manifest.version` OR `package.json.version` and equals the entry's `version`;
5. the zip contains NO `runtime/`, NO `.git`, and no path escapes;
6. `(id, pluginType, version)` is new, OR re-submitted with the SAME sha256 (a different sha256 for a
   published version is a conflict);
7. if `icon` is present, both light and dark resolve to an image; if absent, fine;
8. `name` present; `summary`/`capabilities` optional.
Pass -> a maintainer reviews -> merge -> compile.

## The remaining design questions for the owner (unchanged, prioritised)
1. fragments (`registry.d/plugins/<type>-<id>-<version>.json`) + a compile step making `registry.json`
   a build output (contract-adjacent);
2. the CI rule set above as the enforcement contract;
3. submission channel: PR-only, or PR + issue-fallback for third parties;
4. `registry-review` team / org rulesets on `registry.d/`;
5. whether `schema 1 -> 2` rides the same change.

## Consequence for the two design docs
`registry-update-and-release-design.md` and `registry-contribution-flow-design.md` are revised in
place by this review: D1 (version fallback), D2 (merge, not replace), D5 (no `runtime` in the
registry), D4 (icon optional). The rest stands. Nothing is implemented; nothing is published.

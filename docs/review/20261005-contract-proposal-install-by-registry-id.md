# Contract proposal (FOR OWNER REVIEW) — install may name a url, but the hub RECONCILES it against the official registry

## Why
Today `POST /v1/plugins` runs whatever SOURCE the caller names (a git url / local path, or an
`artifact.url`) with NO check against any registry (docs/issues/20261005-130000). So anyone who can
reach the route can make the hub download a virus and run it as itself. The registry exists to be
the trust anchor; install never consults it.

BUT the owner's direction: do NOT forbid a direct url. A direct url is a CONVENIENCE, and the hub
must RECONCILE it against the official registry. A source the registry lists is AUTHORIZED; a source
it does not list is UNAUTHORIZED and refused by default (a deployment may later choose to allow
unlisted sources; that judgement is left for the future, not hard-coded here).

This is a CONTRACT change. It is NOT written to `contract/` until the owner approves.

## The reconciliation rule (the security core)
The registry pins, per plugin id, one or more releases `{version, url, sha256, size}`. To install,
the caller may supply a source; the hub MUST find a registry entry that matches it:

- **artifact**: the source's `url` MUST equal a registry `url`, AND the source's `sha256` MUST equal
  that registry entry's `sha256`. BOTH, or it is unauthorized.
  - WHY BOTH: matching only the url lets an attacker PROXY that url and serve different bytes. The
    sha256 is the anchor: the bytes are pinned, so a proxied/ MITM url cannot substitute code.
  - The hub then downloads the url and verifies the bytes against that sha256 (and size) before
    unpacking (already implemented), so a proxy is caught twice: at reconcile and at verify.
- **git url / local path**: the official registry publishes release ZIPS only, so a git source
  matches no entry -> UNAUTHORIZED by default. (A future deployment rule may allow it.)

A match authorizes the install. No match -> UNAUTHORIZED (a named code; see below). The default is
REFUSE; the hub does not invent a "trust me" path.

## Proposed `source` (url KEPT for convenience, plus an id path)
```jsonc
"source": {
  "type": "object",
  "description": "where the plugin comes from. Either NAMING the plugin (`{id, version?}` - the hub resolves it from the official registry) or GIVING a release (`{url, sha256, size?}` or a git `{url, ref?}`). A given source is installed ONLY if it RECONCILES against the official registry: an artifact's url AND sha256 must match a registry entry; a git source matches none and is unauthorized unless a deployment allows it.",
  "properties": {
    "id":      { "type": "string", "description": "a plugin id the official registry lists; the hub resolves url+sha256 from the registry." },
    "version": { "type": "string", "description": "optional with id; a version the registry lists. Absent = the registry's newest." },
    "url":     { "type": "string", "description": "a release url or a git url/path; for an artifact it MUST match a registry url exactly." },
    "ref":     { "type": "string", "description": "with a git url: branch/tag/commit." },
    "artifact": { "$ref": "#/defs/artifact" }
  }
}
```
(The final field set - keep `artifact` as a nested object as today, or flatten - is a detail; the
RULE above is the substance.)

## Codes (owner to confirm names/status)
- `plugin_not_in_registry` — proposed **403**, `retryable:false`: "this source is not in the official
  registry; an unlisted source is refused (a deployment may allow unlisted sources explicitly; not
  yet implemented)."
- registry absent/unreadable/bad -> `registry_unavailable` (502, `retryable:false`) - ALREADY ADDED.
- pin mismatch stays `artifact_digest_mismatch` (502) when the downloaded bytes do not match.

## What does NOT change
- landing rule (dir with manifest under `<DATA_DIR>/plugins/<id>`, replace-as-update, old moved
  aside, deployment dirs refused 409).
- `catalog` restates the registry verbatim.

## Impact on the suite
`tests/lib/hub.py::install_plugins` should install a first-party plugin by **id** (the hub resolves
it); a direct-url test must assert BOTH the authorized case (url+sha256 of a registry release ->
installed) and the unauthorized case (a url/sha256 not in the registry -> 403, nothing lands). The
test must not read `registry.json` to supply an identity the hub should own.

## Owner decision needed
1. Approve: direct url KEPT, BUT only an entry matching url+sha256 installs; unlisted -> refused.
2. Name/status for the unauthorized code (proposed `plugin_not_in_registry`, 403).
3. Confirm the official registry address compiled into the hub (already: OFFICIAL_REGISTRY_URL) and
   that a dev-only env override exists for tests (already implemented).

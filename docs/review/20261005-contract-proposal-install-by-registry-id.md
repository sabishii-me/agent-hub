# Contract proposal (FOR OWNER REVIEW) — install NAMES a plugin id; the registry is the only source

## Why
`POST /v1/plugins` currently accepts a CALLER-supplied `url` (a git url or local path) or a
caller-supplied `artifact.url`. The hub fetches and runs whatever the caller names. There is no
allow-list. This makes the hub an arbitrary-code-execution vector: whoever can reach the route can
make the hub download a virus and run it as itself (docs/issues/20261005-130000). The registry exists
to be the trust anchor that prevents exactly this, but install never consults it
(docs/issues/20261005-100000).

This is a CONTRACT change. It is NOT written to `contract/` until the owner approves it.

## Proposed change (one correct shape, not an either/or)

### `source` becomes: EITHER `{id}` (registry) OR `{artifact}` (an exact, pre-verified release)
```jsonc
"source": {
  "type": "object",
  "description": "where the plugin comes from. NORMALLY `{id}`: the hub resolves the id against ITS registry (AGENT_HUB_REGISTRY_URL) and installs the release the registry pins. A caller never supplies a url for a normal install.",
  "properties": {
    "id": {
      "type": "string",
      "description": "a plugin id the hub's REGISTRY lists. The hub resolves it to url+sha256 from the registry and installs THAT. An id the registry does not list is refused. This is the only way a caller names a plugin."
    },
    "version": {
      "type": "string",
      "description": "optional; a version the registry lists for that id. Absent = the registry's newest. A version the registry does not list is refused."
    }
  }
}
```

### The caller-supplied `url` / `artifact` are REMOVED from the public route
- REMOVE `source.url` and `source.ref` and `source.artifact` from `POST /v1/plugins`.
- Rationale: a caller-supplied url is the vulnerability. If ANY install path accepts one, the hole
  is open. The git/artifact primitives stay as INTERNAL mechanics the hub uses to fetch what the
  registry pins - they are not a caller surface.

### Registry resolution rules (what `{id}` means)
- The registry is the JSON at `AGENT_HUB_REGISTRY_URL` (the format `registry.json` already has:
  `plugins:[{id,pluginType,versions:[{version,url,sha256,size}]}]`).
- `{id}` -> find `plugins[].id == id`; pick `version` (or the newest); the release is
  `{url, sha256, size}`. The hub downloads THAT url and verifies THAT digest - the caller controls
  neither.
- NO registry configured (`AGENT_HUB_REGISTRY_URL` unset and no cached file) -> refused
  `registry_unavailable` (502). NO entry for the id -> refused (a `not_found`-family code, to be
  named: suggest `plugin_not_in_registry`, 404). The version not listed -> refused (same family).
- The hub MAY cache the registry on disk (that is what `registry/refresh` writes); a cached copy is
  used when the URL is not set. The URL is contacted at `registry/refresh` and (proposed) once at
  boot when set.

### What does NOT change
- The landing rule (dir with manifest under `<DATA_DIR>/plugins/<id>`, replace-as-update, old moved
  aside, deployment dirs refused 409) is unchanged.
- `catalog` keeps restating the registry file verbatim.

## Impact on the suite (the point of this)
`tests/lib/hub.py::install_plugins` must post `{source:{id}}` and the hub must resolve it. The test
must NOT read `registry.json` itself. Then a green install means "the HUB resolved and installed from
its registry", not "the test handed the hub a url".

## Owner decision needed
1. Approve `source` = `{id, version?}` only, and REMOVE the caller-supplied url/artifact.
2. Name the "id not in the registry" code (proposed `plugin_not_in_registry`, 404) and the version
   code.
3. Confirm the registry env/format and whether the hub fetches once at boot when the URL is set.

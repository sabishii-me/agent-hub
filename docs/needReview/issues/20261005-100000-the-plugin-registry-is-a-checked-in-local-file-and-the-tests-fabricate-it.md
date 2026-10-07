# 20261005-100000 — the plugin "registry" is a checked-in local file, and every install test fabricates it

## Status
OPEN

## The facts (verified this session, not inferred)

1. **The hub never consults a registry to install.** `POST /v1/plugins` takes
   `source.artifact = {url, sha256, id, pluginType, version, size}` (crates/plugins/src/routes.rs:207,
   source.rs:26). The hub downloads THAT url and verifies THAT digest. It resolves nothing and asks
   no registry which artifact is current.

2. **The test IS the registry.** `tests/lib/hub.py::Hub._registry` (line 126) reads `registry.json`
   from the repo, `install_plugins` (line 111) builds the `artifact` dict from it, and posts it. The
   URL the hub downloads is chosen by the test out of a file the test read. If that file is wrong or
   empty, the test supplies the lie. That is the mock: the suite fabricates the registry.

3. **`GET /v1/plugins/catalog` contacts nothing.** service.rs:102 `catalog()` restates
   `registry_file()` verbatim; the file is `AGENT_HUB_REGISTRY_FILE`, else `<DATA_DIR>/registry.json`
   (service.rs:86-95). The suite's data dir has no registry, so the catalog is a fault — the tests do
   not install from the catalog; they read the repo file.

4. **The only code that fetches a real registry is untested and unreachable in the suite.**
   `POST /v1/plugins/registry/refresh` -> `refresh_registry()` (service.rs:147) reads
   `AGENT_HUB_REGISTRY_URL` and writes it. `grep -rn AGENT_HUB_REGISTRY_URL tests/` -> none;
   `grep -rn registry/refresh tests/` -> none (before this session). So "how do I update the local
   registry" had no tested, working answer.

5. **The contract says the catalog is DERIVED from each plugin's own release metadata.**
   `contract/adapter-v1.json:48`: the manifest carries `release:{repository,...}`, and
   **"the catalog is derived from it."** The shipped product instead restates a hand-maintained
   `registry.json` produced by `pack-plugins.mjs --write`. The code does not do what the contract says.

## What is NOT true (I must not overclaim)
The GitHub release URLs are not obviously dead: a real GET of pi 0.1.8 returned 200, 54566 bytes, a
real zip, sha256 `aa8a345f…b55f50b8`, which matches `registry.json`'s `sha256` and `size` exactly.
So "the registry URLs 404" is NOT supported. The defect is that **the hub has no registry service and
the suite plays one** — a different and real claim.

## Why this is a defect
- The delivered install surface is "download the url the caller names" — a downloader, not a
  registry-backed installer. A user cannot say "install the current pi" and have the hub resolve it.
- The one route that could establish a registry (refresh) had no test and no default URL.
- Every install test proves the downloader while presenting itself as "install from a registry".

## Fix (OWNER DECISION — contract change, review BEFORE writing)
Pick the product shape:
- **A:** the hub resolves the current artifact from a registry it owns/refreshes (fetched via
  `AGENT_HUB_REGISTRY_URL` at a defined time; both `catalog` and install read it), and the suite
  installs by naming a PLUGIN ID — the test never reads a repo file; OR
- **B:** the product is explicitly "install = caller supplies url+sha256", the contract/docs say so,
  and the suite stops pretending it has a registry.

## Acceptance
- A test installs a plugin by ID against a hub whose registry came from a real source, the test never
  reading a repo file to build the artifact.
- `registry/refresh` has a test (added this session: tests/plugins/registry-refresh-updates-the-local-registry.py).

## Related
`20261005-110000` — the error code of the refresh route is false.

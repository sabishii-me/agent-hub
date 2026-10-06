# 20261005-130000 [T0] — install never consults the registry: with NO registry at all, any plugin still installs

## Status
OPEN — **T0** (the highest severity). Located. Has reproducible evidence. Needs a contract + product change (not a single-point patch).

## The defect, in one sentence
`POST /v1/plugins` downloads and installs ANY source the caller names. There is NO step that asks
the registry whether that source is authorized. So the registry's existence is irrelevant to
installation: with no registry, a nonexistent registry, or a foreign one, an arbitrary plugin still
installs and runs.

## Reproducible evidence (real hub, this session)
A fresh hub whose registry is ABSENT, then install a plugin that exists in NO registry:
```
[1] GET /v1/plugins/catalog  -> {"plugins":[],"fault":"no registry at <dir>/registry.json"}
[2] POST /v1/plugins {"source":{"url":"<arbitrary local git dir>","ref":"HEAD"}}
        -> 202 {"pluginId":"arbitrary-plugin","state":"installing"}
[3] GET /v1/plugins/arbitrary-plugin -> 200 {"plugin":{...,"origin":"hub"}}
    plugins root: ['arbitrary-plugin']
```
The `evil` case (tests/plugins/a-caller-cannot-install-from-an-arbitrary-url.py) is the same:
**0/4 RED** — the plugin the registry never listed is installed.

## Why the registry cannot currently stop it
- The install source is CALLER-supplied: `{url,ref?}` (git/local path) or `{artifact:{url,sha256,...}}`
  (contract/openapi.json `POST /v1/plugins`; routes.rs:211-228 takes it verbatim; source.rs clones/
  downloads it).
- `begin_install` (service.rs:336-430) stages and lands it with NO registry check.
- The registry is read ONLY by `POST /v1/plugins/registry/refresh` (service.rs:151). Install and the
  registry are not connected at all.
- The official address (crates/plugins/src/registry.rs, now the hub's `registry` release asset) is
  currently 404 (unpublished), which changes NOTHING here: install does not use it.

## Threat
Anyone who can reach `POST /v1/plugins` makes the hub download and run arbitrary code AS ITSELF. The
hub is the attack vector; the registry — the one thing meant to be the trust anchor — is never
consulted. (Owner's framing: put a virus through hub install, then accuse the official hub.)

## The fix is a SHAPE, not a patch (owner collects the whole thing before changing)
The rule the owner set: a caller MAY give a url (convenience), but the hub MUST RECONCILE it against
the official registry before installing:
- the source's `url` AND `sha256` must match a registry entry -> AUTHORIZED, install;
- url matching alone is NOT enough: a proxied/MITM url serves different bytes, so sha256 is the
  anchor;
- no match -> UNAUTHORIZED, refused with a named code, NOTHING downloaded/landed;
- the default is REFUSE; later a deployment may choose to allow unlisted sources (left for the
  future, not hard-coded here).
Proposal text: docs/review/20261005-contract-proposal-install-by-registry-id.md;
security design: docs/review/20261005-plugin-system-security-design.md.

## Related (same root: the registry is not wired into installation)
- 20261005-100000 (the registry is a local file the tests fabricate);
- 20261005-110000 (the registry error surface);
- 20261005-120000 (install overwrites a deployment dir — also a missing pre-check).

## Test that must go green (and only becomes honest once the hub resolves the registry itself)
tests/plugins/a-caller-cannot-install-from-an-arbitrary-url.py — currently 0/4.

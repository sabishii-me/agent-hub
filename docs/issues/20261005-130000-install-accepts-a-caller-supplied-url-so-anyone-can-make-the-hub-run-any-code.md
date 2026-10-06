# 20261005-130000 — POST /v1/plugins accepts a caller-supplied url, so anyone who can reach the hub can make it download and run arbitrary code

## Status
OPEN — a SECURITY defect and a contract change. The contract edit needs the OWNER's review before it is written.

## The defect (verified in code, this session)
`POST /v1/plugins` takes a SOURCE the CALLER chooses:
- `{source:{url:"<git url or local path>", ref?}}`  -> the hub runs `git clone` on it and uses the result;
- `{source:{artifact:{url, sha256, ...}}}`          -> the hub downloads that url and unpacks it.

Contract: `contract/openapi.json` `POST /v1/plugins` requestBody `source.properties` = {url, ref,
artifact}. Code: `crates/plugins/src/routes.rs:211-228` `InstallSource::into_source` takes the
caller's `url`/`artifact` verbatim; `crates/plugins/src/source.rs` clones/downloads it.

There is **no allow-list, no registry check, no signed source**. Whatever the caller names, the hub
fetches and runs.

## Why this is the registry's whole point
The registry exists to be a **trust anchor**: the set of plugins the DEPLOYMENT trusts, with the
exact release (url + sha256) each id maps to. A hub that installs from a caller's url is not using a
registry at all: it makes "install a plugin" equal "run code from an address the caller picked".

Consequence the owner named: anyone who can reach `POST /v1/plugins` (or trick a caller into calling
it) can make the hub download a virus and run it AS THE HUB, then point at the official hub and say
it shipped the virus. The hub becomes the attack vector. That is the opposite of what a registry is
for.

## What the correct shape is (to be decided by the owner; likely a contract change)
- Install NAMES a plugin id: `POST /v1/plugins {source:{id}}`.
- The hub resolves that id against ITS registry (`AGENT_HUB_REGISTRY_URL` -> the registry JSON:
  id -> {pluginType, versions:[{version,url,sha256,size}]}).
- The hub installs ONLY an id the registry lists, at the url+digest the registry pins. A caller can
  never supply a url.
- A registry with no entry for the id -> refused with a named error. A registry that cannot be read
  -> refused (see 20261005-110000, code `registry_unavailable`).

NOTE: the CURRENT contract explicitly allows the caller url (`source.properties.url` says "a git URL
or a local path"). So this is a CONTRACT change. Do not edit `contract/` until the owner approves the
new `source` shape (id-only) and the resolution rule.

## Evidence that nothing else can be trusted today
`tests/lib/hub.py::install_plugins` reads the repo's `registry.json` itself and posts
`{source:{artifact:{url,sha256,...}}}`. So the TEST is the registry. A hub with no test in front of it
knows ZERO plugins (verified: a bare hub answers `plugins:[]`, `harnesses:[]`, `catalog: no registry`).
This is the same root as 20261005-100000, seen from the security side.

## Acceptance (after the contract is agreed)
- `POST /v1/plugins {source:{id}}` installs the id ONLY when the hub's registry lists it, at the url
  and digest the registry pins; the caller supplies no url.
- an id not in the registry -> refused, named.
- a registry that is absent/unreadable/bad -> refused, named (registry_unavailable).
- a test installs by id against a REAL registry the hub reads; the test never supplies a url.
- a test proves a caller CANNOT make the hub install from an arbitrary url (the attack above).

# Plugin-system security design (FOR OWNER REVIEW — no contract/code change yet)

## The attack the owner named (verified in code, this session)
An `AGENT_HUB_REGISTRY_URL` (or `AGENT_HUB_REGISTRY_FILE`) that anyone can point at a fake registry
lets them inject ANY plugin. And today an API client can ALREADY swap the registry out at runtime:
`POST /v1/plugins/registry/refresh` (crates/plugins/src/routes.rs:54) fetches
`AGENT_HUB_REGISTRY_URL` and WRITES it into the file the hub reads
(crates/plugins/src/service.rs:151-174). Plus install itself takes a CALLER-supplied `url`
(routes.rs:211-228). So there is no trust boundary at all: the registry is only a suggestion.

`main.rs` never reads the registry at boot; it is read only by the refresh route. So the registry,
as shipped, is not an anchor — it is a mutable file.

## The principle the design must obey
A registry is a TRUST ANCHOR only if it is:
- set by the DEPLOYER (the party running the hub), not by an API caller;
- fixed for the life of the process (not swappable through `/v1`);
- VERIFIABLE by the hub against something the attacker cannot change.

Environment variables may POINT AT a registry; they must not DEFINE trust. If an attacker can set the
environment, no registry scheme can help (they can also set `PATH`, etc.) — but the meaningful line
is: **an API caller, and a fake registry served at the configured URL, must not be able to introduce
code the deployer did not approve.**

## Proposed design (single shape)

### 1. The registry is loaded ONCE, at boot, from a deployer-controlled source, and is immutable
- Source precedence, fixed at startup: `AGENT_HUB_REGISTRY_FILE` (a local path) if set, else the
  cached file, else `AGENT_HUB_REGISTRY_URL` fetched once.
- `POST /v1/plugins/registry/refresh` is REMOVED, or becomes a READ-ONLY "when did I last load it"
  endpoint. A caller must never be able to change what the hub will install.
- The loaded registry is held in memory; install resolves against that, never against a mutable file.

### 2. Install NAMES an id; the caller never supplies a url
- `POST /v1/plugins {source:{id, version?}}`. The hub resolves id -> {url, sha256} from ITS loaded
  registry and installs exactly that.
- `source.url` / `source.ref` / `source.artifact` are REMOVED from the public route (the internal
  git/artifact fetch stays as a mechanic, not a caller surface).
- This alone closes "caller supplies a url".

### 3. The registry must be VERIFIABLE — this is what makes a fake registry useless
Choose ONE (owner's call); the anchor must be compiled into the hub, never taken from env:
- **(S) Signed registry**: each registry carries a deployer signature; the hub verifies it against a
  compiled-in public key. A fake registry fails the signature.
- **(T) Trust-on-first-use + pin**: the hub stores a digest of the registry the first time; later
  loads must match, or are refused unless the deployer explicitly re-pins (a local, human action —
  never an API call). A suddenly-different registry is refused.
- **(B) Bundled, read-only registry**: the registry ships inside the hub artifact and is not
  runtime-modifiable. Simplest; no network trust at all.
Recommended: **(B) for the first-party set, with (S) as the mechanism to extend it** — because a
bundled registry has no remote trust to attack, and a signed registry lets a deployer add their own
trusted plugins without opening an arbitrary-url hole.

### 4. Plugin integrity is pinned per release
Every registry entry pins `{version, url, sha256, size}`. The hub verifies BOTH size and sha256
before unpacking (already implemented in source.rs). A registry that lists a url whose bytes do not
match the pinned digest installs nothing.

### 5. Defence in depth (not a substitute for 2/3)
- A plugin runs under the hub only through the adapter/child-process boundary that already exists;
  document the blast radius (a plugin is code with the hub's privileges) and consider running the
  child with reduced rights. This is secondary; trust comes from 1-3.

## What must change (for the owner to approve)
1. CONTRACT: `POST /v1/plugins source` = `{id, version?}` only; remove caller `url`/`artifact`.
2. CONTRACT: remove (or neuter) `POST /v1/plugins/registry/refresh` as a mutation.
3. PRODUCT: load the registry once at boot; resolve installs from it; verify via (B) or (S)/(T).
4. Compile the trust anchor (bundled registry and/or public key) INTO the hub.
5. TESTS: install by id only; a test proves a caller CANNOT install from an arbitrary url, and that
   a swapped/fake registry is refused.

## Open owner decisions
- Which verification: (B) bundled read-only, (S) signed, or (T) TOFU-pin? (Recommend B + S.)
- Is `registry/refresh` removed, or kept read-only?
- Where does the deployer's public key come from (compiled default + optional key FILE the deployer
  places, but never an env var an API caller can influence)?

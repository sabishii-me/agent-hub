# 20261008-040000 - the runtime is installed BY THE HUB, not by the adapter's package manager

**State:** FIXED (product change, verified).  
**Severity:** the design boundary was broken - the hub is the supervisor, yet the heavy runtime
install ran inside each adapter, through whatever `npm` the machine happened to have.

## The boundary (the owner's rule)

The hub must PROVIDE/host the runtime toolchain. An adapter that depends on the system's node/npm
dies on a machine that has none, or forces every adapter to bundle its own - a mess. The plugin
DECLARES its runtime; the HUB performs the install, supervised, with no package manager and no
system toolchain.

## What was wrong

The install path called the adapter's `runtime/prepare`, which ran `npm pack` + `tar` + `npm install`
against the SYSTEM npm. The runtime therefore depended on the machine, and a plugin could run an
arbitrary package manager - it could "break the hub".

## The fix

`crates/plugins/src/runtime.rs`: the hub installs the runtime from the closure the plugin SHIPS
(`runtime.sources.json`): every source is an official tarball URL + npm `integrity`. The hub
downloads, verifies the sha512 integrity, unpacks the `.tgz` (honouring `path`/`strip` and the
npm `os`/`cpu` filters), and swaps the whole target in only when EVERY source succeeded. The
adapter's `runtime/prepare` is no longer on the install path.

No package manager, no system node: a plugin cannot reach the machine, and the hub supervises it.

## Verified (dev)

pi installs (the hub downloads 146 official tarballs, verifies each, unpacks) and reaches `ready`
with `runtime/dist/cli.js` on disk: `tests/plugins/mock/a-plugin-is-installed-listed-enabled-and-removed.py`
19/19, runtime materialisation ~102s. (Serial download is the current cost; download is IO-bound and
can be bounded-parallel next.)

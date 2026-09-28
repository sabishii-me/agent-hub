// Record a plugin's RUNTIME as the official distribution it is, into the plugin's
// own runtime.sources.json (shipped in its artifact), not the registry.
//
//   node scripts/record-runtime.mjs [--plugins <dir>] [--only <id,id>] [--write]
//                                   [--lockfile <file>] [--resolve]
//
// What gets written is a LIST OF SOURCES: the vendor's package tarball plus every
// dependency its metadata names — each with the URL the vendor publishes and the
// `integrity` from the same registry metadata npm itself verifies against. The hub
// installs from that list with no package manager at all (runtime.mjs). npm runs HERE
// and only here: on a build machine, at release time.
//
// Where the closure comes from, in order:
//   1. `--lockfile <file>`, or the lockfile a prepared runtime carries
//      (`runtime/npm-shrinkwrap.json`, else `runtime/package-lock.json`): npm's own
//      resolution record. `resolved`/`integrity` are the registry's values, and entries
//      marked `inBundle` are already inside the vendor's tarball.
//   2. `--resolve`: ask pnpm to resolve it now (`pnpm install --lockfile-only`).
//
// Two checks run before anything is written, because a runtime that is missing a
// package is a session that dies in someone's face later:
//   * COMPLETENESS — every dependency named by any recorded package must itself be in
//     the record (a vendor shrinkwrap can be a subset of the real closure, and one was);
//   * INTEGRITY — every source that is not bundled must end up with an integrity value,
//     taken from the registry's own metadata for that exact tarball, never invented.

import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { readTarGz } from '../zip.mjs';

const HERE = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const argv = process.argv.slice(2);
const arg = (name, fallback = null) => {
  const i = argv.indexOf(`--${name}`);
  return i >= 0 && argv[i + 1] && !argv[i + 1].startsWith('--') ? argv[i + 1] : fallback;
};
const flag = (name) => argv.includes(`--${name}`);

// The deployment root (the checkout owning `plugins/` and `apps/`). A recorded path
// is relative to it, never absolute: this file is published, and one machine's disk
// layout must not travel inside it.
const repoRoot = path.resolve(HERE, '..', '..');
// Reduce a path to one relative to the deployment root. A published record carrying
// this machine's disk is worse than a failed pack, so a path outside the root is
// refused rather than written.
function toRepoRelative(target) {
  const rel = path.relative(repoRoot, path.resolve(target));
  if (!rel || rel.startsWith('..') || path.isAbsolute(rel)) {
    throw new Error(`refusing to record a non-portable path: ${target} is outside the deployment root ${repoRoot}`);
  }
  return rel;
}
const pluginsDir = path.resolve(arg('plugins', path.join(HERE, '..', '..', 'plugins')));
const only = (arg('only', '') || '').split(',').map((s) => s.trim()).filter(Boolean);
const readJson = (file, fallback = null) => { try { return JSON.parse(fs.readFileSync(file, 'utf8')); } catch { return fallback; } };
const recordedFromFile = (file) => JSON.stringify(readJson(file, null), null, 2);

// Registry metadata for one package, read once over HTTP (one request for the whole
// package, not one process per field). The fields asked for are the shape npm view
// returns, so callers do not change.
const REGISTRY = process.env.AGENT_HUB_NPM_REGISTRY || 'https://registry.npmjs.org';
const metaCache = new Map();
async function npmView(spec, fields) {
  const at = spec.lastIndexOf('@');
  const name = at > 0 ? spec.slice(0, at) : spec;
  const want = at > 0 ? spec.slice(at + 1) : 'latest';
  let doc = metaCache.get(name);
  if (!doc) {
    const res = await fetch(`${REGISTRY}/${name.replace('/', '%2f')}`, { signal: AbortSignal.timeout(120_000) });
    if (!res.ok) throw new Error(`${name}: the registry answered ${res.status}`);
    doc = await res.json();
    metaCache.set(name, doc);
  }
  const version = want === 'latest' ? doc['dist-tags']?.latest : want;
  const v = (doc.versions || {})[version];
  if (!v) throw new Error(`${name}@${version}: the registry has no such version`);
  const out = { version, name: v.name || name };
  for (const f of fields) {
    if (f === 'version') continue;
    const parts = f.split('.');
    let cur = v;
    for (const part of parts) cur = cur == null ? undefined : cur[part];
    if (cur !== undefined) out[f] = cur;
  }
  return out;
}

/** Every package name a path can denote, so a dependency can be looked up. */
function nameOf(key, entry) {
  const tail = key.split('node_modules/').pop();
  return entry && entry.name ? entry.name : tail;
}

function completeness(lock, topName) {
  const names = new Set([topName]);
  const entries = [];
  for (const [key, entry] of Object.entries(lock.packages || {})) {
    if (!key || entry.link) continue;
    entries.push([key, entry]);
    names.add(nameOf(key, entry));
  }
  const missing = [];
  for (const [key, entry] of entries) {
    for (const dep of Object.keys({ ...(entry.dependencies || {}), ...(entry.optionalDependencies || {}) })) {
      if (!names.has(dep)) missing.push(`${key} needs ${dep}`);
    }
  }
  return { entries, missing };
}

/**
 * A minimal reader for pnpm-lock.yaml: only the three top-level maps this needs.
 */
function readPnpmLock(text) {
  const out = { packages: {}, snapshots: {}, importers: {} };
  let section = null, key = null;
  for (const raw of text.split('\n')) {
    const line = raw.replace(/\r$/, '');
    if (!line.trim() || line.trimStart().startsWith('#')) continue;
    if (/^[a-zA-Z]/.test(line)) { section = line.replace(/:.*/, ''); key = null; continue; }
    if (!section || !out[section]) continue;
    const indent = line.length - line.trimStart().length;
    if (indent === 2 && line.trimEnd().endsWith(':')) {
      key = line.trim().slice(0, -1).replace(/^'|'$/g, '');
      out[section][key] = {};
      continue;
    }
    if (key && indent >= 4) {
      const b = out[section][key].__block;
      const m = line.trim().match(/^([^:]+):\s*(.*)$/);
      if (!m) continue;
      const name = m[1].trim().replace(/^'|'$/g, '');
      const value = m[2].trim().replace(/^'|'$/g, '');
      // Inside a block (dependencies:, optionalDependencies:, ...) every line is an
      // entry of that block; only a shallower line (indent 4 with a trailing colon)
      // starts a new block.
      if (b && indent >= 6) { out[section][key][b][name] = value; continue; }
      if (value !== '') out[section][key][name] = value;
      else { out[section][key][name] = {}; out[section][key].__block = name; }
    }
  }
  return out;
}

/**
 * Resolve a runtime closure with pnpm, handed back in the npm lockfile shape the rest of
 * this file reads: a flat `packages` map keyed `node_modules/<name>` with
 * version/integrity/dependencies. pnpm is used because npm's resolution HANGS on some
 * closures (deepseek's @deepseek-ai/dsh never finished; pnpm did it in seconds). pnpm's
 * lock carries no url; the caller already backfills a missing url from the registry.
 */
async function resolveWithPnpm(spec, log) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'resolve-runtime-'));
  // The spec goes into package.json: pnpm only expands a dependency's own tree when
  // it is a declared dependency, not when it is passed on the command line.
  const at = spec.lastIndexOf('@');
  const depName = at > 0 ? spec.slice(0, at) : spec;
  const depVersion = at > 0 ? spec.slice(at + 1) : 'latest';
  fs.writeFileSync(path.join(dir, 'package.json'), JSON.stringify({ dependencies: { [depName]: depVersion } }, null, 2) + String.fromCharCode(10));
  log(`asking pnpm to resolve ${spec} (this is the release machine doing the resolving)`);
  execFileSync('pnpm', ['install', '--lockfile-only', '--prod', '--ignore-scripts'], {
    cwd: dir, stdio: ['ignore', 'ignore', 'pipe'], windowsHide: true, shell: process.platform === 'win32', timeout: 900_000,
  });
  const text = fs.readFileSync(path.join(dir, 'pnpm-lock.yaml'), 'utf8');
  fs.rmSync(dir, { recursive: true, force: true });
  const y = readPnpmLock(text);

  // Peel "<name>@<version>" and, for snapshots, a peer suffix "(...)".
  //
  // STRIP THE PARENTHESES FIRST. A peer suffix can itself contain an `@`
  // (`jouzu@0.1.13(ws@8.22.0)`), and `lastIndexOf('@')` then lands inside the peer
  // suffix and returns name `jouzu@0.1.13(ws`, version `8.22.0)`. The snapshot is
  // filed under a key nothing else names, its `dependencies` are lost, and the walk
  // from the real `jouzu@0.1.13` sees an empty tree - which is how a runtime that
  // imports `@sinclair/typebox` was recorded with one source and no dependency at all.
  const split = (k) => {
    const plain = k.includes('(') ? k.slice(0, k.indexOf('(')) : k;
    const at = plain.lastIndexOf('@');
    return at <= 0 ? null : { name: plain.slice(0, at), version: plain.slice(at + 1) };
  };
  const byVersion = new Map();   // "name@version" -> {integrity, deps}
  for (const [k, v] of Object.entries(y.packages)) {
    const p2 = split(k); if (!p2) continue;
    byVersion.set(`${p2.name}@${p2.version}`, { integrity: v.resolution ? String(v.resolution.integrity || '') : '', deps: {} });
  }
  const edges = [];              // { from: "name@version", dep: "name@version" }
  for (const [k, v] of Object.entries(y.snapshots)) {
    const p2 = split(k); if (!p2) continue;
    const from = `${p2.name}@${p2.version}`;
    if (!byVersion.has(from)) byVersion.set(from, { integrity: '', deps: {} });
    for (const [dn, dv] of Object.entries(v.dependencies || {})) {
      const ver = String(dv).split('(')[0];
      byVersion.get(from).deps[dn] = ver;
      edges.push({ from, dep: `${dn}@${ver}` });
    }
  }

  // A layout Node can resolve. pnpm allows many versions of one name; a tree does not,
  // so the FIRST name to be reached at a level takes `node_modules/<name>` there, and a
  // second version of the same name is nested under the parent that needs it. Every
  // `name@version` is placed exactly once, so the walk cannot fan out: it is a graph
  // traversal, not a path enumeration.
  const packages = {};
  const placedAt = new Map();      // "name@version" -> path
  const placed = new Set();        // paths already taken
  const rootSplit = split(spec);
  const rootId = `${rootSplit.name}@${rootSplit.version}`;
  const place = (id, path) => {
    const p2 = split(id);
    const held = byVersion.get(id) || { integrity: '', deps: {} };
    packages[path] = { name: p2.name, version: p2.version, ...(held.integrity ? { integrity: held.integrity } : {}), ...(Object.keys(held.deps).length ? { dependencies: held.deps } : {}) };
    placedAt.set(id, path);
    placed.add(path);
  };
  // Where this dependency would live under a given ancestor path, honouring the name
  // rule: free at that level -> that level; taken by a different version -> nested.
  const wantPath = (ancestor, name, id) => {
    const base = ancestor ? `${ancestor}/node_modules/${name}` : `node_modules/${name}`;
    const holder = placedAt.get(id);
    if (holder) return holder;
    const conflict = [...placedAt.entries()].some(([otherId, otherPath]) => otherPath === base && otherId !== id);
    if (conflict) {
      // nested one level under every parent that needs it would explode; pnpm chooses one
      // subtree per instance, and a single extra level under the FIRST such parent is what
      // Node needs to resolve it there.
      return `${ancestor ? `${ancestor}/node_modules/${name}` : `node_modules/${name}`}`;
    }
    return base;
  };
  const queue = [{ id: rootId, path: '' }];
  while (queue.length) {
    const { id, path } = queue.shift();
    if (!placedAt.has(id)) place(id, path);
    const deps = (byVersion.get(id) || {}).deps || {};
    for (const [dn, dv] of Object.entries(deps)) {
      const child = `${dn}@${dv}`;
      if (placedAt.has(child)) continue;
      const childPath = path ? `${path}/node_modules/${dn}` : `node_modules/${dn}`;
      if (placed.has(childPath)) continue;
      queue.push({ id: child, path: childPath });
    }
  }
  return { lock: { lockfileVersion: 3, packages }, where: 'pnpm install --lockfile-only (release machine)' };
}


/**
 * What the vendor's tarball actually contains, as a set of paths below the package root.
 *
 * npm's `inBundle` flag is a claim, and it is not always true: jouzu's lockfile marks
 * `node_modules/@sinclair/typebox` as bundled while the tarball has no such file at all,
 * and an install built on that claim produced a runtime that died on its first import.
 * So the flag is checked against the bytes, and an entry that is NOT in the tarball is
 * recorded like any other dependency.
 */
async function tarballEntries(url, log) {
  const res = await fetch(url, { redirect: 'follow', signal: AbortSignal.timeout(600_000) });
  if (!res.ok) throw new Error(`${url} answered ${res.status}`);
  const file = path.join(fs.mkdtempSync(path.join(os.tmpdir(), 'record-runtime-')), 'package.tgz');
  fs.writeFileSync(file, Buffer.from(await res.arrayBuffer()));
  const names = new Set(readTarGz(file).map((entry) => entry.name.replace(/^package\//, '')));
  fs.rmSync(path.dirname(file), { recursive: true, force: true });
  log(`vendor tarball carries ${names.size} file(s); bundle claims are checked against them`);
  return names;
}

let failed = false;
let changed = false;
const runtimeNames = fs.existsSync(path.join(pluginsDir, 'manifest.json')) ? ['.'] : fs.readdirSync(pluginsDir).sort();
for (const name of runtimeNames) {
  const dir = name === '.' ? pluginsDir : path.join(pluginsDir, name);
  const manifest = readJson(path.join(dir, 'manifest.json'), null);
  if (!manifest || !manifest.runtime || !manifest.runtime.package || !manifest.runtime.version) continue;
  if (only.length && !only.includes(manifest.id)) continue;
  const log = (line) => console.log(`[${manifest.id}] ${line}`);
  const spec = `${manifest.runtime.package}@${manifest.runtime.version}`;
  const command = Array.isArray(manifest.runtime.command) ? manifest.runtime.command.filter((part) => part !== 'node') : [];
  const target = command.length > 1 && command[0].includes('/') ? command[0].split('/')[0] : 'runtime';
  console.log(`[${manifest.id}] runtime ${spec} -> ${target}/`);
  try {
    const dist = await npmView(spec, ['dist.tarball', 'dist.integrity', 'version']);
    if (!dist['dist.tarball'] || !dist['dist.integrity']) throw new Error(`the registry states no tarball/integrity for ${spec}`);
    const sources = [{ url: dist['dist.tarball'], integrity: dist['dist.integrity'], path: '' }];

    let lock = null;
    let where = null;
    // --resolve means "ask npm NOW": a lockfile recorded earlier can be a subset of the
    // real closure (one was: it lacked nine platform variants of an optional dependency,
    // and the completeness check below is what caught it).
    if (flag('resolve')) ({ lock, where } = await resolveWithPnpm(spec, log));
    const candidates = [
      arg('lockfile', null),
      path.join(dir, 'runtime', 'npm-shrinkwrap.json'),
      path.join(dir, 'runtime', 'package-lock.json'),
    ].filter(Boolean);
    if (!lock) {
      for (const candidate of candidates) {
        const found = readJson(candidate, null);
        // Relative to the deployment root, so the record reproduces elsewhere.
        if (found && found.packages) { lock = found; where = toRepoRelative(candidate); break; }
      }
    }
    if (!lock) throw new Error(`no lockfile to read (looked at ${candidates.join(', ')}) and --resolve was not passed`);

    // A prepared runtime's lockfile may belong to the previous pin. Do not mix
    // the new top-level tarball with a dependency closure recorded for the old one.
    const packageKey = `node_modules/${manifest.runtime.package}`;
    const rootPackage = lock.packages?.[packageKey] ?? lock.packages?.[''];
    const rootName = rootPackage?.name ?? (lock.packages?.[packageKey] ? manifest.runtime.package : lock.name);
    if (rootName !== manifest.runtime.package || rootPackage?.version !== manifest.runtime.version) {
      throw new Error(`lockfile ${where} belongs to ${rootName ?? 'unknown'}@${rootPackage?.version ?? 'unknown'}, expected ${spec}; resolve this exact pin on the release machine`);
    }
    // npm --package-lock-only creates a wrapper project. Runtime is installed at
    // its package root, not at wrapper/node_modules/<name>; normalize those paths
    // before checking bundled files or placing dependencies.
    if (lock.packages?.[packageKey]) {
      const normalized = { '': { ...lock.packages[packageKey], name: manifest.runtime.package } };
      for (const [key, value] of Object.entries(lock.packages)) {
        if (!key || key === packageKey) continue;
        const relative = key.startsWith(packageKey + '/') ? key.slice(packageKey.length + 1) : key;
        if (normalized[relative]) {
          // Hoisted and bundled copies can name the same exact package. Never
          // flatten two different versions into a silently changed runtime.
          const held = normalized[relative];
          if (held.version !== value.version || (held.integrity && value.integrity && held.integrity !== value.integrity)) {
            throw new Error(`ambiguous runtime dependency placement: ${relative}`);
          }
          normalized[relative] = { ...held, ...value, inBundle: held.inBundle || value.inBundle };
          continue;
        }
        normalized[relative] = value;
      }
      lock = { ...lock, name: manifest.runtime.package, packages: normalized };
    }
    const { entries, missing } = completeness(lock, manifest.runtime.package);
    // npm's own lockfile can omit the platform variants of an optional dependency (it
    // kept nine @napi-rs/canvas binaries out of a jouzu install). They are named by the
    // parent's `optionalDependencies` with exact pins, so they are completed here from
    // the registry's metadata — never guessed, and only ever adding what the parent
    // itself names.
    const completed = [];
    if (missing.length) {
      for (const line of missing) {
        const [parentKey, dep] = line.split(' needs ');
        const parent = (lock.packages || {})[parentKey] || {};
        const range = (parent.optionalDependencies || {})[dep] ?? (parent.dependencies || {})[dep];
        if (!range) throw new Error(`${parentKey} names no range for ${dep}`);
        const version = /^[0-9]/.test(range) && /^\d+\.\d+\.\d+/.test(range)
          ? range
          : String((await npmView(`${dep}@${range}`, ['version'])).version ?? '').split(',')[0];
        if (!version) throw new Error(`${dep}@${range}: no version satisfied the range`);
        const info = await npmView(`${dep}@${version}`, ['dist.tarball', 'dist.integrity', 'os', 'cpu', 'dependencies', 'optionalDependencies']);
        if (!info['dist.integrity']) throw new Error(`${dep}@${version}: the registry publishes no integrity`);
        if (info.dependencies && Object.keys(info.dependencies).length) throw new Error(`${dep}@${version} has its own dependencies: this fill cannot be trusted for it`);
        completed.push({
          url: info['dist.tarball'],
          integrity: info['dist.integrity'],
          path: `${parentKey}/node_modules/${dep}`,
          ...(Array.isArray(info.os) && info.os.length ? { os: info.os } : {}),
          ...(Array.isArray(info.cpu) && info.cpu.length ? { cpu: info.cpu } : {}),
          optional: true,
        });
      }
      log(`${missing.length} optional dependency variant(s) the lockfile left out were completed from the registry (${missing.map((m) => m.split(' needs ')[1]).join(', ')})`);
    }

    const inTarball = await tarballEntries(dist['dist.tarball'], log);
    let bundled = 0;
    let claimedButAbsent = 0;
    // Lockfile entries that carry no integrity value are collected here and filled from
    // the registry's own metadata below; the completed variants already have theirs.
    const pending = [];
    sources.push(...completed);
    for (const [key, entry] of entries) {
      if (entry.inBundle) {
        // Trusting the flag is what produced a runtime missing @sinclair/typebox.
        if (inTarball.has(`${key}/package.json`)) { bundled++; continue; }
        claimedButAbsent++;
      }
      const source = {
        url: entry.resolved,
        integrity: entry.integrity || null,
        path: key,
        ...(Array.isArray(entry.os) && entry.os.length ? { os: entry.os } : {}),
        ...(Array.isArray(entry.cpu) && entry.cpu.length ? { cpu: entry.cpu } : {}),
        ...(entry.libc ? { libc: [].concat(entry.libc) } : {}),
        ...(entry.optional ? { optional: true } : {}),
      };
      // A package that npm called bundled, that is not in the tarball, has no resolution
      // recorded either: its URL and digest come from the registry's metadata below, the
      // same place npm would have taken them from.
      if (!source.url || !source.integrity) pending.push({ source, entry, key });
      sources.push(source);
    }

    // Anything the lockfile left without an integrity value gets it from the registry's
    // own metadata for that exact tarball: the same value npm would have verified.
    for (const { source, key, entry } of pending) {
      const name = nameOf(key, entry);
      const version = entry.version;
      if (!version) throw new Error(`${key}: no version to look the registry up by`);
      const info = await npmView(`${name}@${version}`, ['dist.integrity', 'dist.tarball']);
      if (!info['dist.integrity']) throw new Error(`${key}: the registry publishes no integrity for ${name}@${version}`);
      if (source.url && info['dist.tarball'] && info['dist.tarball'] !== source.url) {
        throw new Error(`${key}: the recorded URL ${source.url} is not the registry's ${info['dist.tarball']}`);
      }
      if (!source.url) {
        source.url = info['dist.tarball'];
        log(`${name}@${version} was claimed bundled but is not in the tarball: url and integrity taken from the registry`);
      } else {
        log(`integrity for ${name}@${version} taken from the registry's metadata (the lockfile had none)`);
      }
      source.integrity = info['dist.integrity'];
    }
    for (const source of sources) {
      if (!source.url || !source.integrity) throw new Error(`${source.path || 'the top package'}: recorded without a URL or an integrity`);
    }

    const platformSpecific = sources.filter((s) => s.os || s.cpu).length;
    log(`${sources.length} source(s) from ${where}${bundled ? `, ${bundled} bundled inside the vendor tarball` : ''}; ${platformSpecific} declare os/cpu and are filtered on the installing machine`);
    if (claimedButAbsent) log(`${claimedButAbsent} package(s) marked 'inBundle' are NOT in the vendor tarball and were recorded as sources instead (the flag lied; the bytes decide)`);
    const record = {
      package: manifest.runtime.package,
      version: manifest.runtime.version,
      target,
      strip: 1,
      recordedFrom: where,
      sources,
    };
    // The runtime closure is the artifact's install detail, so it is written beside
    // the plugin's manifest - the archive carries it and the hub reads it at install
    // time. It does NOT go into the registry, which stays a small catalog.
    const recordFile = path.join(dir, 'runtime.sources.json');
    if (recordedFromFile(recordFile) !== JSON.stringify(record, null, 2)) changed = true;
    if (flag('write')) fs.writeFileSync(recordFile, JSON.stringify(record, null, 2) + '\n');
  } catch (e) {
    console.error(`[${manifest.id}] ${e.message}`);
    failed = true;
  }
}

if (changed && !flag('write')) console.log('--write was not passed: no runtime.sources.json was written');
if (failed) process.exitCode = 1;

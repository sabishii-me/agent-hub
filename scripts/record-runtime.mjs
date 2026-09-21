// Record a plugin's RUNTIME as the official distribution it is, into registry.json.
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
//   2. `--resolve`: ask npm to resolve it now (`npm install --package-lock-only`).
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
const registryFile = path.resolve(arg('registry', path.join(HERE, 'registry.json')));
const only = (arg('only', '') || '').split(',').map((s) => s.trim()).filter(Boolean);
const readJson = (file, fallback = null) => { try { return JSON.parse(fs.readFileSync(file, 'utf8')); } catch { return fallback; } };

const npmView = (spec, fields) => {
  const out = execFileSync('npm', ['view', spec, ...fields, '--json'], { encoding: 'utf8', windowsHide: true, shell: process.platform === 'win32', timeout: 120_000 });
  return JSON.parse(out);
};

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

async function resolveWithNpm(spec, log) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'resolve-runtime-'));
  const file = path.join(dir, 'package.json');
  fs.writeFileSync(file, '{}\n');
  log(`asking npm to resolve ${spec} (this is the release machine doing the resolving)`);
  execFileSync('npm', ['install', '--package-lock-only', '--omit=dev', '--no-audit', '--no-fund', spec], {
    cwd: dir, stdio: ['ignore', 'ignore', 'pipe'], windowsHide: true, shell: process.platform === 'win32', timeout: 900_000,
  });
  const lock = readJson(path.join(dir, 'package-lock.json'), null);
  fs.rmSync(dir, { recursive: true, force: true });
  if (!lock) throw new Error(`npm resolved ${spec} but wrote no package-lock.json`);
  return { lock, where: 'npm install --package-lock-only (release machine)' };
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

const registry = readJson(registryFile, null);
if (!registry || !Array.isArray(registry.plugins)) { console.error(`${registryFile}: no plugins array`); process.exit(1); }

let failed = false;
let changed = false;
for (const name of fs.readdirSync(pluginsDir).sort()) {
  const dir = path.join(pluginsDir, name);
  const manifest = readJson(path.join(dir, 'manifest.json'), null);
  if (!manifest || !manifest.runtime || !manifest.runtime.package || !manifest.runtime.version) continue;
  if (only.length && !only.includes(manifest.id)) continue;
  const log = (line) => console.log(`[${manifest.id}] ${line}`);
  const spec = `${manifest.runtime.package}@${manifest.runtime.version}`;
  const command = Array.isArray(manifest.runtime.command) ? manifest.runtime.command.filter((part) => part !== 'node') : [];
  const target = command.length > 1 && command[0].includes('/') ? command[0].split('/')[0] : 'runtime';
  console.log(`[${manifest.id}] runtime ${spec} -> ${target}/`);
  try {
    const dist = npmView(spec, ['dist.tarball', 'dist.integrity', 'version']);
    if (!dist['dist.tarball'] || !dist['dist.integrity']) throw new Error(`the registry states no tarball/integrity for ${spec}`);
    const sources = [{ url: dist['dist.tarball'], integrity: dist['dist.integrity'], path: '' }];

    let lock = null;
    let where = null;
    // --resolve means "ask npm NOW": a lockfile recorded earlier can be a subset of the
    // real closure (one was: it lacked nine platform variants of an optional dependency,
    // and the completeness check below is what caught it).
    if (flag('resolve')) ({ lock, where } = await resolveWithNpm(spec, log));
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
          : String(npmView(`${dep}@${range}`, ['version']).version ?? '').split(',')[0];
        if (!version) throw new Error(`${dep}@${range}: no version satisfied the range`);
        const info = npmView(`${dep}@${version}`, ['dist.tarball', 'dist.integrity', 'os', 'cpu', 'dependencies', 'optionalDependencies']);
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
      const info = npmView(`${name}@${version}`, ['dist.integrity', 'dist.tarball']);
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
    const entry = registry.plugins.find((p) => p && p.id === manifest.id);
    if (!entry) { console.error(`[${manifest.id}] is not in ${registryFile}: pack the plugin first`); failed = true; continue; }
    if (JSON.stringify(entry.runtime || null) !== JSON.stringify(record)) changed = true;
    const release = entry.versions?.find(v => v.version === manifest.version);
    if (!release) throw new Error(`no release for plugin ${manifest.id}@${manifest.version}`);
    // What must be immutable is the ARTIFACT description - the sources and their
    // integrity - because those are the bytes an installing machine verifies.
    // `recordedFrom` is provenance annotation that nothing reads, so it is excluded:
    // correcting it must not demand a plugin version bump. A real change to the
    // sources still throws.
    const artifact = (r) => { if (!r) return null; const { recordedFrom: _omit, ...rest } = r; return rest; };
    if (release.runtime?.sources && JSON.stringify(artifact(release.runtime)) !== JSON.stringify(artifact(record))) {
      // This is a real difference in the artifact description, and the published
      // record is the authority: re-running this tool on a different machine must
      // never replace digests an installing machine may already have verified.
      // It fires today for jouzu@0.1.2 (164 published vs 151 rebuilt). Do NOT
      // silence it by regenerating; resolve which lockfile is correct first, or
      // bump the plugin version so the difference is a new release.
      const a = artifact(release.runtime).sources || [];
      const b = artifact(record).sources || [];
      console.error(`[${manifest.id}] published record has ${a.length} source(s); this machine rebuilds ${b.length} - refusing to overwrite a verified record`);
      throw new Error(`runtime record for ${manifest.id}@${manifest.version} is immutable; bump the plugin version`);
    }
    release.runtime = record;
    entry.runtime = record;
  } catch (e) {
    console.error(`[${manifest.id}] ${e.message}`);
    failed = true;
  }
}

if (changed && flag('write')) {
  fs.writeFileSync(registryFile, `${JSON.stringify(registry, null, 2)}\n`);
  console.log(`registry: ${registryFile} updated`);
} else if (changed) {
  console.log('--write was not passed: the registry was not touched');
}
if (failed) process.exitCode = 1;

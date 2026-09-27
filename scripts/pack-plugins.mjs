// Release tooling for plugin artifacts: pack a plugin directory into the zip the hub
// installs, and (re)write the registry entry that points at it.
//
//   node scripts/pack-plugins.mjs [--plugins <dir,dir,...>] [--out <dir>]
//                                 [--registry <file>] [--repo-template <template>]
//                                 [--only <id,id>] [--write] [--publish --yes]
//
// The FORMAT is the hub's (zip.mjs writes exactly what the hub reads). Where a
// plugin's release lands, and what it is called in a list, are the plugin's own
// business, so a plugin may carry a `release.json` beside its manifest:
//
//   { "name": "Pi", "summary": "…", "repository": "sabishii-me/sabishii-dev-harness-pi",
//     "exclude": ["docs", "examples", "*.md"] }
//
// Nothing here invents metadata: fields the plugin does not state are left out, and
// the registry file keeps whatever the plugins said, verbatim.
//
// The runtime is NOT included, and never should be: `<plugin>/runtime/` is an install
// THIS machine ran (a package plus its dependency closure, resolved for this platform —
// deepseek's tree is win32 sharp/koffi binaries, jouzu's ships windows textguard
// executables, pi's carries darwin and win32 prebuilds). Packing that would ship one
// machine's snapshot as a release. The artifact is the PLUGIN; the runtime comes from
// the official distribution, installed by the plugin's own `runtime/prepare` with the
// package manager that ships with the app. The registry entry therefore carries the
// runtime as a DECLARATION ({package, version}), not as bytes.
//
// Publishing (`--publish`) uploads each zip to the plugin repository's GitHub Release
// with `gh`. It is a separate, explicit step: nothing here touches a remote unless it
// is asked, and without `--yes` it only prints the commands.

import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { execFileSync } from 'node:child_process';
import { writeZip } from '../zip.mjs';

const HERE = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const argv = process.argv.slice(2);
const arg = (name, fallback = null) => {
  const i = argv.indexOf(`--${name}`);
  return i >= 0 && argv[i + 1] && !argv[i + 1].startsWith('--') ? argv[i + 1] : fallback;
};
const flag = (name) => argv.includes(`--${name}`);

const DEFAULT_REPO_TEMPLATE = 'https://github.com/{repository}/releases/download/v{version}/{id}-{version}.zip';
const repoTemplate = arg('repo-template', DEFAULT_REPO_TEMPLATE);
const outDir = path.resolve(arg('out', path.join(HERE, 'dist', 'plugins')));
const registryFile = path.resolve(arg('registry', path.join(HERE, 'registry.json')));
const only = (arg('only', '') || '').split(',').map((s) => s.trim()).filter(Boolean);
const pluginDirs = (arg('plugins', path.join(HERE, '..', '..', 'plugins')) || '').split(',').map((s) => s.trim()).filter(Boolean);

// What never ships: version control, installed dependencies, caches, and the locally
// materialised runtime. A plugin that needs something else excluded says so in
// release.json; the runtime is not negotiable.
const ALWAYS_EXCLUDED = ['.git', '.github', 'node_modules', 'runtime', '.venv', '__pycache__', '.cache', '.DS_Store'];

function matchesAny(relPath, patterns) {
  for (const raw of patterns) {
    const p = raw.replace(/^\.\//, '');
    if (!p) continue;
    if (p.startsWith('*.')) { if (relPath.endsWith(p.slice(1))) return true; continue; }
    if (relPath === p || relPath.startsWith(`${p}/`)) return true;
  }
  return false;
}

function collectFiles(dir, exclude, prefix = '') {
  const out = [];
  for (const entry of fs.readdirSync(dir, { withFileTypes: true }).sort((a, b) => a.name.localeCompare(b.name))) {
    const rel = prefix ? `${prefix}/${entry.name}` : entry.name;
    if (matchesAny(rel, ALWAYS_EXCLUDED) || matchesAny(rel, exclude)) continue;
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) out.push(...collectFiles(full, exclude, rel));
    else if (entry.isFile()) out.push({ name: rel, full });
  }
  return out;
}

function readJson(file, fallback = null) {
  try { return JSON.parse(fs.readFileSync(file, 'utf8')); } catch { return fallback; }
}

// The version of the PLUGIN — the adapter and the extensions — never of the runtime it
// drives. The two are separate releases on purpose: an adapter fix that keeps the same
// runtime pin must still be a new version, or the update is invisible and the registry
// would silently overwrite a published artifact with different bytes.
function versionOf(dir, manifest) {
  if (typeof manifest.version === 'string' && manifest.version.trim()) return manifest.version.trim();
  const pkg = readJson(path.join(dir, 'package.json'), null);
  if (pkg && typeof pkg.version === 'string' && pkg.version.trim()) return pkg.version.trim();
  return null;
}

const entries = [];
const packed = [];
let failed = false;

for (const root of pluginDirs) {
  if (!fs.existsSync(root)) { console.error(`no such plugins directory: ${root}`); failed = true; continue; }
  for (const name of fs.readdirSync(root).sort()) {
    const dir = path.join(root, name);
    const manifestFile = path.join(dir, 'manifest.json');
    if (!fs.existsSync(manifestFile)) continue;
    const manifest = readJson(manifestFile, null);
    if (!manifest || typeof manifest.id !== 'string') { console.error(`${dir}: manifest.json has no id — not a plugin`); failed = true; continue; }
    if (only.length && !only.includes(manifest.id)) continue;
    const release = readJson(path.join(dir, 'release.json'), {}) || {};
    if (release.name && typeof release.name === 'object') { console.error(`${dir}: release.json name must be a string`); failed = true; continue; }

    const version = versionOf(dir, manifest);
    // A plugin with no version cannot be a release: it is a directory somebody else
    // installs (the provider plugins are deployment plugins exactly like that). Named
    // explicitly it is an error; found by walking the directory it is a note.
    if (!version) {
      const what = `${dir}: no version to release (the plugin's own manifest.version or package.json.version)`;
      if (only.includes(manifest.id)) { console.error(what); failed = true; } else console.log(`skipped ${manifest.id}: ${what.split(': ')[1]}`);
      continue;
    }
    const kind = Array.isArray(manifest.command) ? 'harness' : manifest.provider ? 'provider' : null;
    if (!kind) { console.error(`${dir}: manifest declares neither a command nor a provider module`); failed = true; continue; }

    const exclude = Array.isArray(release.exclude) ? release.exclude : [];
    const files = collectFiles(dir, exclude);
    const known = readJson(registryFile, null);
    const file = `${manifest.id}-${version}.zip`;
    fs.mkdirSync(outDir, { recursive: true });
    const zipPath = path.join(outDir, file);
    // --keep reuses a zip this exact version already produced, so a registry-only
    // pass does not re-deflate hundreds of megabytes to write the same bytes.
    if (flag('keep') && fs.existsSync(zipPath)) console.log(`${manifest.id} ${version}: keeping the existing ${file}`);
    else writeZip(zipPath, files.map((f) => ({ name: f.name, file: f.full })));
    const bytes = fs.statSync(zipPath).size;
    const sha256 = crypto.createHash('sha256').update(fs.readFileSync(zipPath)).digest('hex');
    const repository = release.repository || null;
    const url = repository
      ? repoTemplate.replaceAll('{repository}', repository).replaceAll('{version}', version).replaceAll('{id}', manifest.id)
      : release.url || null;
    if (!url) { console.error(`${dir}: no repository in release.json and no --repo-template to build a URL from`); failed = true; continue; }

    const entry = { id: manifest.id, kind };
    // The runtime as a declaration, verbatim from the manifest: what the plugin will
    // fetch from the official distribution when it is installed. When the entry already
    // carries a recorded closure (sources, digests — scripts/record-runtime.mjs), that
    // record is kept: packing a plugin must never drop the list of official bytes.
    if (manifest.runtime && typeof manifest.runtime.package === 'string' && typeof manifest.runtime.version === 'string') {
      const recorded = known && Array.isArray(known.plugins) ? known.plugins.find((p) => p && p.id === manifest.id) : null;
      const held = recorded && recorded.runtime && Array.isArray(recorded.runtime.sources) ? recorded.runtime : null;
      entry.runtime = held && held.package === manifest.runtime.package && held.version === manifest.runtime.version ? held : { package: manifest.runtime.package, version: manifest.runtime.version };
    } else {
      const recorded = known && Array.isArray(known.plugins) ? known.plugins.find((p) => p && p.id === manifest.id) : null;
      if (recorded && recorded.runtime) entry.runtime = recorded.runtime;
    }
    for (const key of ['name', 'summary', 'description']) if (typeof release[key] === 'string' && release[key].trim()) entry[key] = release[key];
    // The harness's own mark, from the manifest's {light, dark} file names, inlined as
    // data URIs. A harness that has NOT been installed has no files on disk - the
    // registry entry is all a client sees - so the bytes travel in the entry itself
    // rather than as a path nothing can resolve. Read from this plugin directory; a
    // declared file that is missing is left out (the client then draws no mark).
    if (manifest.icons && typeof manifest.icons === 'object') {
      const inline = (name) => {
        if (typeof name !== 'string' || !name) return null;
        const full = path.resolve(dir, name);
        if (full !== dir && !full.startsWith(dir + path.sep)) return null;
        try {
          const buf = fs.readFileSync(full);
          const ext = path.extname(full).toLowerCase();
          const mime = ext === '.svg' ? 'image/svg+xml' : ext === '.png' ? 'image/png' : ext === '.webp' ? 'image/webp' : 'application/octet-stream';
          return `data:${mime};base64,${buf.toString('base64')}`;
        } catch { return null; }
      };
      const light = inline(manifest.icons.light ?? manifest.icons.dark);
      const dark = inline(manifest.icons.dark ?? manifest.icons.light);
      if (light || dark) entry.icons = { light: light ?? dark, dark: dark ?? light };
    }
    if (Array.isArray(release.capabilities)) entry.capabilities = release.capabilities;
    else if (Array.isArray(manifest.capabilities)) entry.capabilities = manifest.capabilities;
    const versionEntry = { version, url, sha256, size: bytes, ...(entry.runtime ? { runtime: entry.runtime } : {}), releasedAt: new Date().toISOString().slice(0, 10) };
    const previous = known && Array.isArray(known.plugins) ? known.plugins.find((p) => p && p.id === manifest.id) : null;
    const published = previous && Array.isArray(previous.versions) ? previous.versions.find((v) => v && v.version === version) : null;
    if (published && published.sha256 !== sha256) {
      console.error(`${manifest.id} ${version} is already in ${registryFile} with sha256 ${published.sha256};`);
      console.error(`  these bytes hash to ${sha256}. A published version is not rewritten: bump the plugin version (manifest.version).`);
      failed = true;
      continue;
    }
    const older = previous && Array.isArray(previous.versions) ? previous.versions.filter((v) => v && v.version !== version) : [];
    entry.versions = [versionEntry, ...older];
    entries.push(entry);
    packed.push({ id: manifest.id, version, file: zipPath, bytes, sha256, repository });
    console.log(`${manifest.id} ${version}: ${file} (${(bytes / 1024).toFixed(0)} KB, ${files.length} files)`);
    console.log(`  sha256 ${sha256}`);
    if (entry.runtime) console.log(`  runtime: ${entry.runtime.package}@${entry.runtime.version} (fetched from the official distribution at install time)`);
  }
}

if (packed.length && (flag('write') || flag('publish'))) {
  const registry = readJson(registryFile, {}) || {};
  const others = Array.isArray(registry.plugins) ? registry.plugins.filter((p) => p && !entries.some((e) => e.id === p.id)) : [];
  const next = {
    ...registry,
    schema: typeof registry.schema === 'number' ? registry.schema : 1,
    note: registry.note || 'First-party plugins, generated by apps/harness-hub/scripts/pack-plugins.mjs --write. Each entry names the release zip of that plugin\u2019s own repository; the digest is of the zip this tooling built.',
    plugins: [...entries, ...others].sort((a, b) => a.id.localeCompare(b.id)),
  };
  fs.writeFileSync(registryFile, `${JSON.stringify(next, null, 2)}\n`);
  console.log(`registry: ${registryFile} (${next.plugins.length} entries)`);
} else if (packed.length) {
  console.log('\n--write was not passed: the registry was not touched. Entries that would be written:');
  console.log(JSON.stringify({ plugins: entries }, null, 2));
}

if (flag('publish')) {
  const targets = packed.filter((p) => p.repository);
  for (const p of targets) {
    const args = ['release', 'create', `v${p.version}`, p.file, '--repo', p.repository, '--title', `${p.id} ${p.version}`, '--notes', `Plugin artifact for ${p.id} ${p.version} (sha256 ${p.sha256}).`];
    if (flag('yes')) {
      // A version that is already published is left alone: re-uploading an asset
      // would change the bytes behind a digest somebody already verified.
      try {
        execFileSync('gh', ['release', 'view', `v${p.version}`, '--repo', p.repository], { stdio: 'pipe' });
        console.log(`already published: ${p.repository} v${p.version} — uploading the asset to it`);
        execFileSync('gh', ['release', 'upload', `v${p.version}`, p.file, '--repo', p.repository, '--clobber'], { stdio: 'inherit' });
      } catch {
        execFileSync('gh', args, { stdio: 'inherit' });
      }
    } else {
      console.log(`gh ${args.join(' ')}`);
    }
  }
  if (!targets.length) console.log('nothing to publish: no plugin stated a repository in release.json');
  if (!flag('yes')) console.log('\n--yes was not passed: nothing was uploaded.');
}

if (failed) process.exitCode = 1;

// Pack the hub into the artifact the desktop stages.
//   node scripts/pack-hub.mjs [--out <dir>] [--publish --yes]

import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { execFileSync } from 'node:child_process';
import { writeZip } from '../zip.mjs';
import { getBuildId } from '../build-id.mjs';

const HERE = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const argv = process.argv.slice(2);
const arg = (name, fallback = null) => {
  const i = argv.indexOf(`--${name}`);
  return i >= 0 && argv[i + 1] && !argv[i + 1].startsWith('--') ? argv[i + 1] : fallback;
};
const flag = (name) => argv.includes(`--${name}`);

const pkg = JSON.parse(fs.readFileSync(path.join(HERE, 'package.json'), 'utf8'));
const version = pkg.version;
if (!version) throw new Error('package.json has no version');
const repository = pkg.repository && typeof pkg.repository === 'string' ? pkg.repository
  : typeof pkg.repository === 'object' ? pkg.repository.url : null;
const outDir = path.resolve(arg('out', path.join(HERE, 'dist')));
const file = `agent-hub-${version}.zip`;
const zipPath = path.join(outDir, file);

const RUN_FILES = ['server.mjs', 'transport.mjs', 'cli.mjs', 'resources.mjs', 'runtime.mjs', 'secret-store.mjs', 'unpack-worker.mjs', 'zip.mjs', 'build-id.mjs', 'debate.mjs'];
const entries = [];
for (const name of RUN_FILES) {
  const full = path.join(HERE, name);
  if (!fs.existsSync(full)) throw new Error(`${name} is missing from the hub directory`);
  entries.push({ name, file: full });
}
// The hub is no longer zero-dependency (ADR-0010): the transport is Hono. Its runtime
// dependencies are bundled into the artifact, so the staged hub runs with no install step.
function addNodeModules(dir, prefix) {
  let count = 0;
  for (const name of fs.readdirSync(dir)) {
    if (name === '.bin' || name === '.package-lock.json') continue;
    const full = path.join(dir, name);
    const rel = `${prefix}/${name}`;
    if (fs.statSync(full).isDirectory()) count += addNodeModules(full, rel);
    else { entries.push({ name: rel, file: full }); count++; }
  }
  return count;
}
const nodeModules = path.join(HERE, 'node_modules');
if (!fs.existsSync(nodeModules)) throw new Error('node_modules is missing; run npm install before packing (the hub bundles its runtime dependencies)');
const bundled = addNodeModules(nodeModules, 'node_modules');
for (const name of fs.readdirSync(path.join(HERE, 'contract')).sort()) {
  entries.push({ name: `contract/${name}`, file: path.join(HERE, 'contract', name) });
}
entries.push({ name: 'package.json', contents: JSON.stringify({ name: pkg.name, version, type: 'module', private: true, dependencies: pkg.dependencies || {} }, null, 2) + '\n' });
entries.push({ name: 'build-id.json', contents: JSON.stringify({ buildId: getBuildId(), generatedAt: new Date().toISOString() }, null, 2) + '\n' });

fs.mkdirSync(outDir, { recursive: true });
writeZip(zipPath, entries);
const sha256 = crypto.createHash('sha256').update(fs.readFileSync(zipPath)).digest('hex');
const bytes = fs.statSync(zipPath).size;
console.log(`hub artifact: ${zipPath} (${bytes} bytes, sha256 ${sha256})`);
console.log(`  ${entries.length} entries (${bundled} bundled node_modules file(s))`);

if (repository && flag('publish')) {
  const tag = `v${version}`;
  const args = ['release', 'create', tag, zipPath, '--repo', repository, '--title', `hub ${version}`, '--notes', `Hub artifact ${version} (sha256 ${sha256}).`];
  if (flag('yes')) {
    try {
      execFileSync('gh', ['release', 'view', tag, '--repo', repository], { stdio: 'pipe' });
      console.log(`already published: ${repository} ${tag} — uploading the asset`);
      execFileSync('gh', ['release', 'upload', tag, zipPath, '--repo', repository, '--clobber'], { stdio: 'inherit' });
    } catch {
      execFileSync('gh', args, { stdio: 'inherit' });
    }
  } else {
    console.log(`gh ${args.join(' ')}`);
  }
} else if (flag('publish')) {
  console.error('--publish needs package.json to state a repository');
  process.exitCode = 1;
} else if (flag('publish') === false) {
  console.log('\n--publish was not passed: nothing was uploaded.');
}

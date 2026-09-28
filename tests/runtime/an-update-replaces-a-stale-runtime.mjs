// A user's machine holds an install from BEFORE a packaging fix: the plugin directory
// is there, its runtime was materialised from a one-source record, and it cannot start.
// Installing the plugin's CURRENT release must REPLACE that runtime, not carry it
// forward - "an adapter update keeps its runtime" is right only while the runtime the
// artifact pins is the same one, which is exactly what a fix changes.
//
// Nothing is faked but the OLD install, which is written as the old hub wrote it (a
// runtime directory with no marker). The new side is the real published artifact,
// installed through a real hub; the proof is that the adapter FILE LOADS afterwards.
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import { Hub, ReleaseServer, publishedRegistry, sleep, tally } from '../lib/hub.mjs';

const t = tally();

const reg = await publishedRegistry();
const jouzu = reg.plugins.find((p) => p.pluginType === 'harness-adapter' && p.id === 'jouzu');
console.log('catalog jouzu version:', jouzu.versions[0].version);

const rel = await new ReleaseServer().start();
const asset = await rel.stage(jouzu);

// Build a hub data dir that ALREADY has the old jouzu installed: installed.json plus
// enough of the old tree that the hub sees it as an install to REPLACE.
const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'upd-'));
const plugins = path.join(dir, 'plugins');
fs.mkdirSync(path.join(plugins, 'harness-adapter-jouzu'), { recursive: true });
const oldManifest = { id: 'jouzu', pluginType: 'harness-adapter', version: '0.1.7', name: 'Jouzu', summary: 'old', capabilities: ['runtime'], runtime: { package: 'jouzu', version: '0.1.13', command: ['node', 'runtime/dist/cli.js'] } };
fs.writeFileSync(path.join(plugins, 'harness-adapter-jouzu', 'manifest.json'), JSON.stringify(oldManifest, null, 2));
fs.writeFileSync(path.join(plugins, 'harness-adapter-jouzu', 'runtime.sources.json'), JSON.stringify({ package: 'jouzu', version: '0.1.13', sources: [{ url: 'https://registry.npmjs.org/jouzu/-/jouzu-0.1.13.tgz', path: '' }] }, null, 2));
// a marker-less runtime dir, as the old hub left it
fs.mkdirSync(path.join(plugins, 'harness-adapter-jouzu', 'runtime', 'node_modules'), { recursive: true });
fs.writeFileSync(path.join(plugins, 'harness-adapter-jouzu', 'runtime', 'package.json'), '{"name":"jouzu"}');
fs.writeFileSync(path.join(plugins, 'installed.json'), JSON.stringify({ 'harness-adapter-jouzu': { artifact: { id: 'jouzu', version: '0.1.7', url: 'old', sha256: 'x' }, installedAt: new Date().toISOString() } }, null, 2));

const hub = new Hub(dir);
await hub.start();
try {
  const api = hub.api();
  const before = (await api.plugins()).find((p) => p.id === 'harness-adapter-jouzu');
  t.check(before && before.version === '0.1.7', 'the stale install is the one on disk first', String(before && before.version));

  const ins = await api.install(asset);
  t.check(ins.updated === true, 'the current release replaces the install', JSON.stringify(ins).slice(0, 80));
  for (let i = 0; i < 900; i++) { const p = (await api.plugins()).find((x) => x.id === 'harness-adapter-jouzu'); if (p && ['ready', 'failed'].includes(p.state)) break; await sleep(1000); }
  const p = (await api.plugins()).find((x) => x.id === 'harness-adapter-jouzu');
  t.check(p && p.version === '0.1.9' && p.state === 'ready', 'it settles on the new version, ready', p ? `${p.version} ${p.state} ${p.detail || ''}` : 'gone');

  const sources = JSON.parse(fs.readFileSync(path.join(plugins, 'harness-adapter-jouzu', 'runtime.sources.json'), 'utf8'));
  t.check(sources.sources.length > 1, 'the runtime record is the new one, not the carried one', `${sources.sources.length} source(s)`);
  const hasTypebox = fs.existsSync(path.join(plugins, 'harness-adapter-jouzu', 'runtime', 'node_modules', '@sinclair', 'typebox'));
  t.check(hasTypebox, 'the dependency the old runtime lacked is on disk');

  const { execFileSync } = await import('node:child_process');
  try {
    execFileSync(process.execPath, ['-e', "import('./dist/camoufox-adapter.js').then(()=>{}).catch(e=>{console.error(e.message);process.exit(1)})"], { cwd: path.join(plugins, 'harness-adapter-jouzu', 'runtime'), stdio: 'pipe' });
    t.check(true, 'the adapter FILE the old install could not import now loads');
  } catch (e) { console.log('camoufox-adapter.js FAILS:', (e.stderr || '').toString().split('\n').find((l) => /Cannot find/.test(l))); }
} finally { await hub.stop(); await rel.stop(); }

console.log(t.failures ? `${t.failures} failure(s)` : 'the stale runtime was replaced and the adapter loads');

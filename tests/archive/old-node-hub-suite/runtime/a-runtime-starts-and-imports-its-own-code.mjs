// A plugin can install, settle `ready`, and STILL be dead: its runtime can be missing a
// package its own code imports. The artifact is what the hub installs, so the artifact is
// what this test attacks - through the real hub, on the real published release, ending in
// a REAL process start of the runtime the manifest names.
//
// This is the test that did not exist while `record-runtime` dropped the dependencies of
// every peer-suffixed pnpm snapshot: jouzu installed "successfully" and then died on
// `Cannot find package '@sinclair/typebox'`.
import fs from 'node:fs';
import path from 'node:path';
import { spawn } from 'node:child_process';
import { Hub, ReleaseServer, publishedRegistry, sleep, tally } from '../lib/hub.mjs';

const t = tally();
const reg = await publishedRegistry();
const which = process.argv[2] || 'jouzu';
const adapter = reg.plugins.find((p) => p.pluginType === 'harness-adapter' && p.id === which);
if (!adapter) throw new Error(`the published registry has no ${which} harness-adapter`);

const rel = await new ReleaseServer().start();
const asset = await rel.stage(adapter);
const key = `${adapter.pluginType}-${adapter.id}`;
console.log(`runtime: ${key}@${asset.version} from ${adapter.versions[0].url}`);

const hub = new Hub();
await hub.start();
try {
  const api = hub.api();
  const ins = await api.install(asset);
  t.check(!!ins.plugin, 'the published artifact installs', JSON.stringify(ins).slice(0, 120));
  let p = null;
  for (let i = 0; i < 600; i++) { p = (await api.plugins()).find((x) => x.id === key); if (p && ['ready', 'failed'].includes(p.state)) break; await sleep(1000); }
  t.check(!!p && p.state === 'ready', 'it settles ready', p ? `${p.state} ${p.detail || ''}` : 'gone');

  // The runtime is on disk under the plugin; the manifest states the command.
  const dir = path.join(hub.plugins, key);
  const manifest = JSON.parse(fs.readFileSync(path.join(dir, 'manifest.json'), 'utf8'));
  const runtimeDir = manifest.runtime.target || 'runtime';
  const command = manifest.runtime.command;
  t.check(Array.isArray(command) && command.length > 0, 'the manifest states a command to start', JSON.stringify(command));

  const runtimePath = path.join(dir, runtimeDir);
  t.check(fs.existsSync(runtimePath), 'the runtime directory is on disk', runtimePath);
  const cwd = runtimePath;

  // Every package this runtime's code imports must be resolvable. The adapter source is
  // the file the error named; ask Node, from the runtime directory, for each bare
  // specifier it imports. A missing one is the bug, named.
  // What the runtime's OWN ENTRY imports, statically. This is the closure that must
  // resolve for the process to start; scanning a whole tree would flag specifiers that
  // live behind a platform branch or an optional feature and are never loaded. The
  // decisive check is the real START below - this names the missing package when one is.
  const specs = new Set();
  const entry = path.join(dir, ...command.slice(1));
  const entryDir = path.dirname(entry);
  const frontier = [entry];
  const seen = new Set();
  while (frontier.length) {
    const file = frontier.shift();
    if (!file || seen.has(file) || !fs.existsSync(file) || !/\.(m?js|cjs)$/.test(file)) continue;
    seen.add(file);
    const src = fs.readFileSync(file, 'utf8');
    for (const m of src.matchAll(/(?:^|[^\w.])(?:import|export)\s[^;]*?from\s*["']([^"'.][^"']*)["']/g)) specs.add(m[1]);
    for (const m of src.matchAll(/import\s*\(\s*["']([^"'.][^"']*)["']\s*\)/g)) specs.add(m[1]);
    for (const m of src.matchAll(/require\(\s*["']([^"'.][^"']*)["']\s*\)/g)) specs.add(m[1]);
    // follow only relative imports, one level at a time
    for (const m of src.matchAll(/from\s*["'](\.\.?\/[^"']*)["']/g)) {
      const base = path.resolve(entryDir, m[1]);
      for (const cand of [base, base + '.js', base + '.mjs', base + '.cjs', path.join(base, 'index.js')]) if (fs.existsSync(cand) && !seen.has(cand)) frontier.push(cand);
    }
  }
  // A harness may load a module of its own on demand (jouzu loads
  // `dist/camoufox-adapter.js` when a session starts). Importing every local module that
  // is not reachable from the entry would flag optional plugins; importing the ones the
  // manifest's own package names as its adapter is the product's real path.
  const declared = (() => { try { const pkg = JSON.parse(fs.readFileSync(path.join(runtimePath, 'package.json'), 'utf8')); return Object.values({ ...(pkg.exports || {}), ...(pkg.bin || {}) }).map(String); } catch { return []; } })();
  const onDemand = ['dist/camoufox-adapter.js', ...declared].filter((rel) => rel.endsWith('.js') && fs.existsSync(path.join(runtimePath, rel)));
  const bare = [...specs].filter((x) => !x.startsWith('node:') && !x.startsWith('data:') && !x.startsWith('http'));
  console.log(`  (probing ${bare.length} bare specifier(s) the runtime's own code imports)`);
  for (const spec of bare) {
    const res = await new Promise((resolve) => {
      const c = spawn(process.execPath, ['-e', `import(${JSON.stringify(spec)}).then(()=>process.exit(0)).catch((e)=>{console.error(e.message);process.exit(1)})`], { cwd, stdio: ['ignore', 'ignore', 'pipe'], windowsHide: true });
      let err = ''; c.stderr.on('data', (d) => (err += d)); c.on('close', (code) => resolve(code === 0 ? true : err.trim()));
    });
    t.check(res === true, `the runtime can import ${JSON.stringify(spec)}`, res === true ? '' : String(res).split('\n')[0].slice(0, 160));
  }

  // Import each module the product loads on demand, with the runtime directory as cwd so
  // resolution is exactly the runtime's.
  for (const rel of onDemand) {
    const res = await new Promise((resolve) => {
      const file = path.join(runtimePath, rel);
      const c = spawn(process.execPath, ['-e', `import(process.argv[1]).then(()=>process.exit(0)).catch((e)=>{console.error(e.message);process.exit(1)})`, file], { cwd: runtimePath, stdio: ['ignore', 'ignore', 'pipe'], windowsHide: true });
      let err = ''; c.stderr.on('data', (d) => (err += d)); c.on('close', (code) => resolve(code === 0 ? true : err.trim()));
    });
    const miss = res === true ? null : String(res).split(String.fromCharCode(10)).find((l) => /Cannot find (module|package)/.test(l));
    t.check(!miss, `the runtime can load its own ${rel}`, miss || 'loaded');
  }

  // And really START it: the manifest's own command, from the runtime directory. A harness
  // adapter that dies on a missing import is the shape of the failure this test catches.
  const ended = await new Promise((resolve) => {
    const c = spawn(command[0], command.slice(1), { cwd: dir, stdio: ['ignore', 'pipe', 'pipe'], windowsHide: true });
    let out = ''; c.stdout.on('data', (d) => (out += d)); c.stderr.on('data', (d) => (out += d));
    const timer = setTimeout(() => { c.kill(); resolve({ how: 'still running (killed after 20s)', out }); }, 20000);
    c.on('close', (code, sig) => { clearTimeout(timer); resolve({ how: `exited code=${code} sig=${sig}`, out }); });
  });
  const missing = ended.out.split('\n').find((l) => /Cannot find (module|package)|ERR_MODULE_NOT_FOUND/.test(l));
  t.check(!missing, 'starting the runtime does not fail on a missing import', missing || ended.how);

} finally {
  await hub.stop();
  await rel.stop();
}

console.log(t.failures ? `\n${t.failures} failure(s)` : '\nall checks passed');
process.exit(t.failures ? 1 : 0);

// The concurrency measurement (ADR-0009). This is the pass/fail for every step of
// the reconstruction: it runs ONE long operation and, at the same time, asks the hub
// something trivial over and over. A hub that blocks its event loop cannot answer the
// trivial question while the long work runs, and that is exactly the defect.
//
// It does not assert on a string in a page. It measures whether the PROCESS kept
// answering, which is the thing that was broken.
//
//   node tests/concurrency/measure.mjs
//
// It builds its own data dir (a throwaway under the OS temp dir), installs a plugin
// whose runtime tree is large enough that a synchronous delete is visible, starts a
// hub against it, and measures. Nothing outside its temp dir is touched.

import { spawn } from 'node:child_process';
import fs from 'node:fs';
import fsp from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import http from 'node:http';
import { fileURLToPath } from 'node:url';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const HUB_ROOT = path.resolve(HERE, '..', '..');

// How the delete is made heavy. jouzu's real runtime was 363 MB / 28,582 files; this
// keeps the same shape (many small files) at a size a test can create quickly, which
// is enough to expose a synchronous recursive delete.
const TREE_FILES = Number(process.env.MEASURE_TREE_FILES || 20000);
const TREE_DIRS = Number(process.env.MEASURE_TREE_DIRS || 200);

// The poll: ask /v1/hub/status often, with a deadline. A gap bigger than this, or a
// timeout, is a freeze - the number recorded is the evidence, not a pass/fail word.
const POLL_INTERVAL_MS = 150;
const POLL_TIMEOUT_MS = 3000;
const FREEZE_THRESHOLD_MS = 500;

function log(msg) { process.stdout.write(`${msg}\n`); }

async function request(port, token, method, pathname, { timeout = POLL_TIMEOUT_MS } = {}) {
  return await new Promise((resolve) => {
    const started = Date.now();
    const req = http.request({ host: '127.0.0.1', port, method, path: pathname,
      headers: { authorization: `Bearer ${token}` }, timeout }, (res) => {
      let data = '';
      res.on('data', (c) => { data += c; });
      res.on('end', () => resolve({ ok: true, status: res.statusCode, body: data, ms: Date.now() - started }));
    });
    req.on('timeout', () => { req.destroy(new Error('timeout')); });
    req.on('error', (e) => resolve({ ok: false, error: e.message, ms: Date.now() - started }));
    req.end();
  });
}

// A throwaway plugin whose removal is a large, recursive delete. The manifest is the
// minimum a harness-adapter plugin must declare to be accepted (see manifestFault).
async function makeHeavyPlugin(dataDir) {
  const pluginsRoot = path.join(dataDir, 'plugins');
  const id = 'harness-adapter-stress';
  const dir = path.join(pluginsRoot, id);
  await fsp.mkdir(dir, { recursive: true });
  const protocol = JSON.parse(await fsp.readFile(path.join(HUB_ROOT, 'contract', 'adapter-v1.json'), 'utf8')).version;
  await fsp.writeFile(path.join(dir, 'manifest.json'), JSON.stringify({
    id: 'stress', pluginType: 'harness-adapter', name: 'Stress', protocol,
    command: ['node', 'adapter.mjs'], runtime: { package: 'stress', version: '0.0.0', command: ['node', 'runtime.mjs'] },
  }, null, 2));

  // A tree of many small files, like a real harness runtime: this is what makes a
  // synchronous recursive delete block the loop long enough to be seen.
  const perDir = Math.max(1, Math.floor(TREE_FILES / TREE_DIRS));
  for (let d = 0; d < TREE_DIRS; d++) {
    const sub = path.join(dir, 'runtime', 'node_modules', `pkg-${d}`);
    await fsp.mkdir(sub, { recursive: true });
    for (let f = 0; f < perDir; f++) {
      await fsp.writeFile(path.join(sub, `file-${f}.js`), `// ${d}/${f}\nmodule.exports = ${f};\n`);
    }
  }
  // The record that says the hub installed it (so removePlugin accepts it as 'hub').
  await fsp.writeFile(path.join(pluginsRoot, 'installed.json'), JSON.stringify({
    [id]: { source: 'local://stress', installedAt: new Date().toISOString() },
  }, null, 2));
  let count = 0;
  const walk = async (p) => { for (const e of await fsp.readdir(p, { withFileTypes: true })) {
    const q = path.join(p, e.name); if (e.isDirectory()) await walk(q); else count++; } };
  await walk(dir);
  return { id, dir, count };
}

async function startHub(dataDir) {
  const child = spawn(process.execPath, [path.join(HUB_ROOT, 'server.mjs')], {
    env: { ...process.env, AGENT_HUB_DATA_DIR: dataDir },
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  let out = '';
  child.stdout.on('data', (d) => { out += d; });
  child.stderr.on('data', (d) => { out += d; });
  // The hub writes endpoint.json once it is listening; wait for it.
  const endpoint = path.join(dataDir, 'endpoint.json');
  for (let i = 0; i < 100; i++) {
    await new Promise((r) => setTimeout(r, 100));
    if (fs.existsSync(endpoint)) {
      const info = JSON.parse(await fsp.readFile(endpoint, 'utf8'));
      return { child, port: info.port, token: info.token, output: () => out };
    }
    if (child.exitCode !== null) throw new Error(`hub exited early (${child.exitCode}):\n${out}`);
  }
  child.kill();
  throw new Error(`hub did not come up:\n${out}`);
}

async function main() {
  const dataDir = await fsp.mkdtemp(path.join(os.tmpdir(), 'hub-measure-'));
  log(`data dir: ${dataDir}`);
  const plugin = await makeHeavyPlugin(dataDir);
  log(`plugin: ${plugin.id} (${plugin.count} files) at ${plugin.dir}`);

  const hub = await startHub(dataDir);
  log(`hub up on 127.0.0.1:${hub.port}`);

  try {
    // Baseline: the hub answers a trivial poll when nothing heavy is running.
    const base = await request(hub.port, hub.token, 'GET', '/v1/hub/status');
    if (!base.ok) throw new Error(`baseline poll failed: ${base.error || base.status}`);
    log(`baseline poll: ${base.ms}ms`);

    // Start the long operation (the removal) and, at once, poll on a loop.
    const samples = [];
    let stopped = false;
    const poller = (async () => {
      let last = Date.now();
      while (!stopped) {
        const r = await request(hub.port, hub.token, 'GET', '/v1/hub/status');
        const gap = Date.now() - last;
        last = Date.now();
        samples.push({ gap, ok: r.ok, ms: r.ms, error: r.error, status: r.status });
        await new Promise((r2) => setTimeout(r2, POLL_INTERVAL_MS));
      }
    })();

    const del = await request(hub.port, hub.token, 'DELETE', `/v1/hub/plugins/${plugin.id}`, { timeout: 60000 });
    stopped = true;
    await poller;

    const timeouts = samples.filter((s) => !s.ok);
    const bigGaps = samples.filter((s) => s.gap > FREEZE_THRESHOLD_MS && s.ok);
    const worst = Math.max(0, ...samples.map((s) => s.gap));

    log('');
    log(`DELETE -> ${del.ok ? del.status : del.error} in ${del.ms}ms`);
    log(`polls: ${samples.length}`);
    log(`  timeouts (>${POLL_TIMEOUT_MS}ms no answer): ${timeouts.length}`);
    log(`  gaps > ${FREEZE_THRESHOLD_MS}ms: ${bigGaps.length}`);
    log(`  worst gap: ${worst}ms`);
    for (const t of timeouts.slice(0, 8)) log(`    timeout after ${t.ms}ms: ${t.error || t.status}`);
    for (const g of bigGaps.slice(0, 8)) log(`    gap ${g.gap}ms`);

    // The verdict is mechanical: a freeze is any timeout, or any gap over the
    // threshold, while the operation ran.
    const froze = timeouts.length > 0 || bigGaps.length > 0;
    log('');
    log(froze ? 'RESULT: FROZE (the hub stopped answering during the operation)' : 'RESULT: OK (the hub kept answering throughout)');
    process.exitCode = froze ? 1 : 0;
  } finally {
    hub.child.kill();
    await new Promise((r) => setTimeout(r, 300));
    await fsp.rm(dataDir, { recursive: true, force: true }).catch(() => {});
  }
}

main().catch((e) => { log(`measure failed: ${e.stack || e.message}`); process.exitCode = 2; });

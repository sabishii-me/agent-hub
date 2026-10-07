// The plugin check: does a plugin actually ENTER the hub from the registry?
//
// It starts the built hub on a temp data dir, points its registry at THIS repo's
// registry.json (a real registry), refreshes, and installs the FIRST harness-adapter the
// registry lists by the url+sha256 the registry states. Success = the hub reconciled the
// source against its registry and ACCEPTED the install (not refused). This is the object
// the workflow exists for: a plugin's entry into the hub, not the hub's own compile.
import { spawn, spawnSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

const repo = path.resolve(path.dirname(new URL(import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, '$1')), '..');
const dataDir = fs.mkdtempSync(path.join(os.tmpdir(), 'hub-plugin-check-'));
fs.copyFileSync(path.join(repo, 'registry.json'), path.join(dataDir, 'registry.json'));
const reg = JSON.parse(fs.readFileSync(path.join(repo, 'registry.json'), 'utf8'));
const entry = reg.plugins.find((p) => p.pluginType === 'harness-adapter');
if (!entry) { console.error('FAIL: the registry lists no harness-adapter to install'); process.exit(1); }
const v = entry.versions[0];
console.log(`checking the hub can take ${entry.pluginType}/${entry.id} ${v.version} from the registry`);

const bin = process.platform === 'win32' ? 'target/debug/agent-hub.exe' : 'target/debug/agent-hub';
const hub = spawn(path.join(repo, bin), [], {
  cwd: repo,
  env: { ...process.env, AGENT_HUB_DATA_DIR: dataDir, AGENT_HUB_ADDR: '127.0.0.1:8791', AGENT_HUB_REGISTRY_FILE: path.join(dataDir, 'registry.json') },
  stdio: ['ignore', 'pipe', 'pipe'],
});
let out = '';
hub.stdout.on('data', (d) => { out += d; });
hub.stderr.on('data', (d) => { out += d; });

const t0 = Date.now();
async function waitEndpoint() {
  for (;;) {
    try { return JSON.parse(fs.readFileSync(path.join(dataDir, 'endpoint.json'), 'utf8')); } catch {}
    if (Date.now() - t0 > 20000) throw new Error('the hub did not write endpoint.json in 20s:\n' + out);
    await new Promise((r) => setTimeout(r, 200));
  }
}
const call = (ep, method, p, body) => fetch(`${ep.url}${p}`, {
  method, headers: { authorization: `Bearer ${ep.token}`, 'content-type': 'application/json' },
  body: body ? JSON.stringify(body) : undefined,
});

try {
  const ep = await waitEndpoint();
  // The hub loads its registry at BOOT from AGENT_HUB_REGISTRY_FILE (the copy of this repo's
  // registry.json below). No refresh here: refresh would fetch the compiled-in address, which
  // is a different thing. We check the ENTRY path against the loaded registry.
  const cr = await call(ep, 'GET', '/v1/plugins/catalog');
  const cat = await cr.json();
  console.log('catalog:', cr.status, 'entries:', (cat.plugins || []).length, 'source:', cat.source);
  if (!(cat.plugins || []).some((p) => p.id === entry.id && p.pluginType === entry.pluginType))
    throw new Error('the hub did not load the registry: the entry is not in its catalog');

  // A plugin ENTERS: install the exact release the registry names.
  const ir = await call(ep, 'POST', '/v1/plugins', { source: { artifact: { url: v.url, sha256: v.sha256, id: entry.id, pluginType: entry.pluginType, version: v.version, size: v.size } } });
  const body = await ir.text();
  console.log('install:', ir.status, body.slice(0, 200));
  if (ir.status >= 400) throw new Error(`the hub REFUSED the registry's own release: ${ir.status} ${body}`);

  // And it must not be refused by reconciliation (the point of the entry).
  if (ir.status === 403) throw new Error(`the hub did not recognize its own registry entry: ${body}`);
  console.log('OK: the hub accepted a plugin from its registry');
} catch (e) {
  console.error('FAIL:', e.message);
  process.exitCode = 1;
} finally {
  hub.kill('SIGKILL');
}

// Start a REAL hub process and drive it over its REAL HTTP surface. Nothing is faked:
// a plugin is the real published artifact, fetched from the registry the deployment
// itself publishes. This is the ground the interruption tests stand on.
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import http from 'node:http';
import crypto from 'node:crypto';
import { spawn, spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const HERE = path.dirname(fileURLToPath(import.meta.url));
export const HUB = path.resolve(HERE, '..', '..', 'server.mjs');
export const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/** The registry this deployment publishes - the same URL release.json names. */
export async function publishedRegistry() {
  const releaseFile = path.resolve(HERE, '..', '..', '..', 'prts-web', 'apps', 'desktop', 'src-tauri', 'resources', 'release.json');
  const release = JSON.parse(fs.readFileSync(releaseFile, 'utf8'));
  const r = await fetch(release.registry.url, { redirect: 'follow', signal: AbortSignal.timeout(60000) });
  if (!r.ok) throw new Error(`${release.registry.url} answered ${r.status}`);
  return r.json();
}

/** A loopback server that serves the REAL published artifacts (their bytes), so a hub can
 *  install them without every test re-downloading the network, while the digests remain
 *  the registry's. */
export class ReleaseServer {
  constructor() { this.files = new Map(); }
  async start() { this.server = http.createServer((req, res) => { const f = this.files.get(req.url.replace(/^\//, '')); if (!f) { res.writeHead(404); return res.end(); } const b = fs.readFileSync(f); res.writeHead(200, { 'content-type': 'application/octet-stream', 'content-length': b.length }); res.end(b); }); await new Promise((r) => this.server.listen(0, '127.0.0.1', r)); this.base = `http://127.0.0.1:${this.server.address().port}`; return this; }
  stop() { return new Promise((r) => { try { this.server.close(() => r()); } catch { r(); } }); }
  async stage(entry) {
    const v = entry.versions[0];
    const name = path.basename(new URL(v.url).pathname);
    const dest = path.join(fs.mkdtempSync(path.join(os.tmpdir(), 'rel-')), name);
    const r = await fetch(v.url, { redirect: 'follow', signal: AbortSignal.timeout(600000) });
    if (!r.ok) throw new Error(`${v.url} answered ${r.status}`);
    fs.writeFileSync(dest, Buffer.from(await r.arrayBuffer()));
    this.files.set(name, dest);
    return { id: entry.id, pluginType: entry.pluginType, version: v.version, url: `${this.base}/${name}`, sha256: v.sha256, size: v.size };
  }
}

/** A hub under test: its own data dir, its own process, restartable on the same dir. */
export class Hub {
  constructor(dataDir) { this.dir = dataDir || fs.mkdtempSync(path.join(os.tmpdir(), 'hub-')); this.plugins = path.join(this.dir, 'plugins'); fs.mkdirSync(this.plugins, { recursive: true }); this.child = null; }
  async start() {
    const log = fs.openSync(path.join(this.dir, 'hub.log'), 'a');
    this.child = spawn(process.execPath, [HUB], { env: { ...process.env, AGENT_HUB_DATA_DIR: this.dir, AGENT_HUB_PLUGINS_DIR: this.plugins }, stdio: ['ignore', log, log], windowsHide: true });
    this.ep = await this.endpoint();
    return this;
  }
  async endpoint() {
    // The endpoint of THIS hub, not a stale one a previous life left in the
    // same data directory: a restarted hub reuses the dir and rewrites endpoint.json
    // after it binds, so match the pid this hub spawned.
    for (let i = 0; i < 200; i++) {
      try { const ep = JSON.parse(fs.readFileSync(path.join(this.dir, 'endpoint.json'), 'utf8')); if (ep.pid === this.child.pid) return ep; } catch { /* not written yet */ }
      await sleep(100);
    }
    throw new Error('no endpoint for pid ' + (this.child && this.child.pid) + '; log: ' + this.log());
  }
  log() { try { return fs.readFileSync(path.join(this.dir, 'hub.log'), 'utf8'); } catch { return '(no log)'; } }
  /** Kill it as a power cut would: no cleanup, no chance to finish anything. */
  kill() { if (this.child && this.child.exitCode === null) { try { spawnSync('taskkill', ['/PID', String(this.child.pid), '/T', '/F'], { stdio: 'ignore' }); } catch { /* */ } this.child.kill('SIGKILL'); } return sleep(300); }
  async stop() { if (this.child && this.child.exitCode === null) this.child.kill(); await sleep(300); }
  api() { const base = `http://127.0.0.1:${this.ep.port}`; const H = { authorization: `Bearer ${this.ep.token}`, 'content-type': 'application/json' }; return {
    plugins: async () => (await (await fetch(`${base}/v1/hub/plugins`, { headers: H })).json()).plugins || [],
    install: async (a) => await (await fetch(`${base}/v1/hub/plugins`, { method: 'POST', headers: H, body: JSON.stringify({ source: { artifact: a } }) })).json(),
    remove: async (id) => await (await fetch(`${base}/v1/hub/plugins/${id}`, { method: 'DELETE', headers: H })).json(),
  }; }
}

export function tally() { let f = 0; const check = (ok, n, d = '') => { console.log(`  ${ok ? 'ok  ' : 'FAIL'} ${n}${d ? ' - ' + d : ''}`); if (!ok) f++; }; return { check, get failures() { return f; } }; }
export const sha = (b) => crypto.createHash('sha256').update(b).digest('hex');

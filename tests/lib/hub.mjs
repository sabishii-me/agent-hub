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
  api() { const base = `http://127.0.0.1:${this.ep.port}`; const H = { authorization: `Bearer ${this.ep.token}`, 'content-type': 'application/json' };
    const plugins = async () => (await (await fetch(`${base}/v1/hub/plugins`, { headers: H })).json()).plugins || [];
    const get = async (p) => await (await fetch(`${base}${p}`, { headers: H })).json();
    // A long route is a command, not a call (ADR-0009): the request is ACCEPTED (202 +
    // Location) and the work runs in the background. A caller that wants the finished
    // resource issues the command, then reads the resource until it settles - which is
    // what a real client does. `settle` is that read loop; it never assumes the old
    // synchronous answer.
    const settle = async (pred, { tries = 600, delay = 100 } = {}) => {
      for (let i = 0; i < tries; i++) { const list = await plugins(); const hit = list.find(pred); if (hit) return hit; await sleep(delay); }
      return null;
    };
    return {
      plugins,
      status: async () => await get('/v1/hub/status'),
      // Returns the ACCEPTED answer (its `location` is the contract), and waits for the
      // plugin to SETTLE (ready/failed) so a test asserts the real outcome, not the 202.
      // A replace settles on the same id, so the wait is on the state, not on presence.
      install: async (a) => {
        const before = await plugins();
        const res = await fetch(`${base}/v1/hub/plugins`, { method: 'POST', headers: H, body: JSON.stringify({ source: { artifact: a } }) });
        const body = await res.json();
        // The registry/artifact names the SHORT id; the hub lists the storage key it
        // built (`<pluginType>-<id>`). Wait on the key, which is what the list carries.
        const wanted = a.pluginType && a.id ? `${a.pluginType}-${a.id}` : (a.id || a.pluginId);
        let plugin = null;
        for (let i = 0; i < 900 && wanted; i++) {
          const p = (await plugins()).find((x) => x.id === wanted);
          if (p && ['ready', 'failed'].includes(p.state)) { plugin = p; break; }
          await sleep(100);
        }
        return { ...body, status: res.status, updated: before.some((p) => p.id === wanted), plugin };
      },
      // Waits until the plugin is gone, so a caller sees the removal complete.
      remove: async (id) => {
        const res = await fetch(`${base}/v1/hub/plugins/${id}`, { method: 'DELETE', headers: H });
        const body = await res.json();
        for (let i = 0; i < 600; i++) { if (!(await plugins()).some((p) => p.id === id)) return { ...body, status: res.status, gone: true }; await sleep(100); }
        return { ...body, status: res.status, gone: false };
      },
    }; }
}

export function tally() { let f = 0; const check = (ok, n, d = '') => { console.log(`  ${ok ? 'ok  ' : 'FAIL'} ${n}${d ? ' - ' + d : ''}`); if (!ok) f++; }; return { check, get failures() { return f; } }; }
export const sha = (b) => crypto.createHash('sha256').update(b).digest('hex');

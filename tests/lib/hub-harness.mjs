// Shared harness for the hub's adversarial tests.
//
// Every test here runs a REAL hub process (server.mjs from this repository) against a
// throwaway data directory, and drives it over its real HTTP surface. A plugin is a zip
// built with the hub's own writer and served over loopback, so the install path exercised
// is the real one: download, size, sha256, safe unpack, manifest check, atomic replace.
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import http from 'node:http';
import crypto from 'node:crypto';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { writeZip } from '../../zip.mjs';
import { getBuildId } from '../../build-id.mjs';

const HERE = path.dirname(fileURLToPath(import.meta.url));
export const HUB_ENTRY = path.resolve(HERE, '..', '..', 'server.mjs');

export const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const mkdir = (p) => fs.mkdtempSync(path.join(os.tmpdir(), p));

/** What contract this run was written against; printed so a run is attributable. */
export function contractStamp() {
  const file = path.join(HERE, '..', '..', 'contract', 'v1.json');
  const v1 = JSON.parse(fs.readFileSync(file, 'utf8'));
  return {
    protocol: v1.protocol,
    version: v1.version,
    contractSha: crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex').slice(0, 12),
    buildId: getBuildId(),
  };
}

/** A hub under test: its own data dir and plugins dir, its own process. */
export class Hub {
  constructor(dataDir) {
    this.dir = dataDir || mkdir('hub-test-');
    this.plugins = path.join(this.dir, 'plugins');
    fs.mkdirSync(this.plugins, { recursive: true });
    const log = fs.openSync(path.join(this.dir, 'hub.log'), 'a');
    this.child = spawn(process.execPath, [HUB_ENTRY], {
      env: { ...process.env, AGENT_HUB_DATA_DIR: this.dir, AGENT_HUB_PLUGINS_DIR: this.plugins },
      stdio: ['ignore', log, log], windowsHide: true,
    });
  }
  async endpoint() {
    for (let i = 0; i < 150; i++) {
      try { return JSON.parse(fs.readFileSync(path.join(this.dir, 'endpoint.json'), 'utf8')); } catch { await sleep(100); }
    }
    throw new Error(`the hub did not come up in ${this.dir}; log:\n${this.log()}`);
  }
  log() { try { return fs.readFileSync(path.join(this.dir, 'hub.log'), 'utf8'); } catch { return '(no log)'; } }
  stop() {
    return new Promise((resolve) => {
      if (this.child.exitCode !== null) return resolve();
      this.child.once('exit', () => resolve());
      try { this.child.kill(); } catch { resolve(); }
      setTimeout(resolve, 3000);
    });
  }
}

/** The loopback server that serves plugin zips, and the asset() builder for them. */
export class AssetServer {
  constructor({ slowMs = 0 } = {}) { this.zips = new Map(); this.slowMs = slowMs; }
  async start() {
    this.server = http.createServer((req, res) => {
      const f = this.zips.get(req.url.replace(/^\//, ''));
      if (!f) { res.writeHead(404); return res.end(); }
      const buf = fs.readFileSync(f);
      res.writeHead(200, { 'content-type': 'application/zip', 'content-length': buf.length });
      if (this.slowMs) {
        // A deliberately slow body, so a test can observe the hub's in-progress window.
        setTimeout(() => res.end(buf), this.slowMs);
      } else res.end(buf);
    });
    await new Promise((r) => this.server.listen(0, '127.0.0.1', r));
    this.base = `http://127.0.0.1:${this.server.address().port}`;
    return this;
  }
  asset(manifest) {
    const name = `${manifest.kind}-${manifest.id}-${manifest.version}.zip`;
    const file = path.join(mkdir('hub-zip-'), 'p.zip');
    writeZip(file, [{ name: 'manifest.json', contents: JSON.stringify(manifest, null, 2) + '\n' }]);
    this.zips.set(name, file);
    const buf = fs.readFileSync(file);
    return { id: manifest.id, version: manifest.version, url: `${this.base}/${name}`, sha256: crypto.createHash('sha256').update(buf).digest('hex'), size: buf.length };
  }
  stop() { return new Promise((resolve) => { try { this.server.close(() => resolve()); } catch { resolve(); } }); }
}

/** A complete harness-adapter manifest (passes the hub's boot self-check). */
export const adapter = (id, version = '1.0.0') => ({ id, kind: 'harness-adapter', version, protocol: 0, command: ['node', 'noop.cjs'] });

const J = async (r) => { const t = await r.text(); try { return JSON.parse(t); } catch { return { raw: t.slice(0, 160) }; } };

/** A client for one hub's /v1 surface. */
export function client(ep) {
  const base = `http://127.0.0.1:${ep.port}`;
  const H = { authorization: `Bearer ${ep.token}`, 'content-type': 'application/json' };
  return {
    base, H,
    plugins: async () => ((await J(await fetch(`${base}/v1/hub/plugins`, { headers: H }))).plugins) || [],
    catalog: async () => J(await fetch(`${base}/v1/hub/catalog`, { headers: H })),
    install: (a, signal) => fetch(`${base}/v1/hub/plugins`, { method: 'POST', headers: H, body: JSON.stringify({ source: { artifact: a } }), signal }).then(J),
    installRaw: (body, signal) => fetch(`${base}/v1/hub/plugins`, { method: 'POST', headers: H, body, signal }).then(J),
    remove: (managedId, signal) => fetch(`${base}/v1/hub/plugins/${managedId}`, { method: 'DELETE', headers: H, signal }).then(J),
    removeRaw: (rawId, signal) => fetch(`${base}/v1/hub/plugins/${rawId}`, { method: 'DELETE', headers: H, signal }).then(J),
  };
}

/** Subscribe to the hub event stream; returns { state, stop, done }. */
export function subscribe(c) {
  const ctl = new AbortController();
  const state = { count: 0, stopped: false };
  const done = (async () => {
    try {
      const r = await fetch(`${c.base}/v1/hub/events`, { headers: c.H, signal: ctl.signal });
      const rd = r.body.getReader(); const dec = new TextDecoder();
      for (;;) {
        const ch = await rd.read();
        if (ch.done) break;
        state.count += (dec.decode(ch.value).match(/hub\.plugins\.changed/g) || []).length;
      }
    } catch { /* aborted */ }
    state.stopped = true;
  })();
  return { state, stop: () => ctl.abort(), done };
}

/** Tally for one test file; the process exit code reflects it. */
export function tally() {
  let failures = 0;
  const check = (name, ok, detail = '') => {
    console.log(`  ${ok ? 'ok  ' : 'FAIL'} ${name}${detail ? ` - ${detail}` : ''}`);
    if (!ok) failures++;
  };
  return { check, get failures() { return failures; } };
}

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
const NL = String.fromCharCode(10);
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
  /** `registry` is a list of catalog entries written before the hub starts (the hub reads
   *  its registry file once at boot). Pass one when a test needs the hub to KNOW a plugin
   *  it has not installed - for example to serve an uninstalled plugin's icon. */
  constructor(dataDir, { registry } = {}) {
    this.dir = dataDir || mkdir('hub-test-');
    this.plugins = path.join(this.dir, 'plugins');
    fs.mkdirSync(this.plugins, { recursive: true });
    this.registryFile = path.join(this.dir, 'registry.json');
    if (registry) fs.writeFileSync(this.registryFile, JSON.stringify({ schema: 1, plugins: registry }, null, 2));
    const log = fs.openSync(path.join(this.dir, 'hub.log'), 'a');
    this.child = spawn(process.execPath, [HUB_ENTRY], {
      env: { ...process.env, AGENT_HUB_DATA_DIR: this.dir, AGENT_HUB_PLUGINS_DIR: this.plugins, AGENT_HUB_REGISTRY_FILE: this.registryFile },
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
  /** Serve a REAL released artifact. `spec` is a fixture registry entry (see release());
   *  the bytes and the sha256 are the published ones - only the URL is this loopback. */
  asset(spec) {
    const file = path.join(FIXTURES, spec.file);
    this.zips.set(spec.file, file);
    return { id: spec.id, pluginType: spec.pluginType, version: spec.version, url: `${this.base}/${spec.file}`, sha256: spec.sha256, size: spec.size };
  }
  stop() { return new Promise((resolve) => { try { this.server.close(() => resolve()); } catch { resolve(); } }); }
}

/** The REAL released plugins, as artifacts.
 *
 * tests/fixtures/ holds the actual release zips of this deployment's plugins -
 * harness-adapter-deepseek/jouzu/pi and model-provider-compatible/deepseek/shisa - each
 * byte for byte as it was published, with registry.json beside them naming the version,
 * sha256 and size. A test does not fabricate a manifest the hub happens to accept; it
 * installs the same bytes a user would, from the same registry, so the install path, the
 * manifest check, the digest check and the runtime are all exercised for real.
 *
 * The files are served over loopback (the hub is offline-friendly), but nothing about
 * them is invented. fixtures/README.md says how they are refreshed. */
export const FIXTURES = path.join(HERE, '..', 'fixtures');

/** The registry this deployment ships, as fixture metadata: [{id, pluginType, version, file,
 *  url, sha256, size}], one entry per released plugin. */
export function releaseRegistry() {
  return JSON.parse(fs.readFileSync(path.join(FIXTURES, 'registry.json'), 'utf8'));
}

/** One released plugin by storage name, e.g. 'harness-adapter-deepseek'. */
export function release(name) {
  const r = releaseRegistry().find((e) => `${e.pluginType}-${e.id}` === name);
  if (!r) throw new Error(`no released plugin '${name}' in fixtures/registry.json`);
  return r;
}

/** The REAL harness adapters this deployment ships. */
export const HARNESSES = ['harness-adapter-deepseek', 'harness-adapter-jouzu', 'harness-adapter-pi'];
/** The REAL model providers this deployment ships. */
export const PROVIDERS = ['model-provider-compatible', 'model-provider-deepseek', 'model-provider-shisa'];

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

/** Subscribe to the hub event stream; returns { state, stop, done }.
 *
 * `state.events` keeps every hub.plugins.changed as {id, state} in arrival order, because
 * the tests attack exactly that: whether the state an event names agrees with the state a
 * read returns for the same plugin. A bare count cannot see a lie. */
export function subscribe(c) {
  const ctl = new AbortController();
  const state = { count: 0, events: [], stopped: false };
  const done = (async () => {
    try {
      const r = await fetch(`${c.base}/v1/hub/events`, { headers: c.H, signal: ctl.signal });
      const rd = r.body.getReader(); const dec = new TextDecoder();
      let buf = '';
      for (;;) {
        const ch = await rd.read();
        if (ch.done) break;
        buf += dec.decode(ch.value);
        // SSE frames are separated by a blank line; parse each completed frame.
        let idx;
        while ((idx = buf.indexOf(String.fromCharCode(10, 10))) !== -1) {
          const frame = buf.slice(0, idx); buf = buf.slice(idx + 2);
          const ev = /event: (.*)/.exec(frame);
          const data = /data: (.*)/.exec(frame);
          if (ev && ev[1] === 'hub.plugins.changed') {
            state.count++;
            try { state.events.push(JSON.parse(data[1])); } catch { state.events.push({ malformed: data && data[1] }); }
          }
        }
      }
    } catch { /* aborted */ }
    state.stopped = true;
  })();
  return { state, stop: () => ctl.abort(), done };
}

/** The one invariant every plugin test exists to attack: a sample cannot disagree with
 *  itself. `plugins()` returns each plugin with one `state`; this returns the ways that
 *  sample is internally impossible, so a test can name them.
 *
 *  The states are exclusive by construction: a plugin cannot be installing and preparing,
 *  cannot be absent and on disk, cannot be removing and ready. If the hub ever reports two,
 *  a client shows two things at once - which is the confusion this whole suite is for. */
export function contradictions(plugins) {
  const VALID = new Set(['absent', 'installing', 'removing', 'preparing', 'failed', 'ready']);
  const problems = [];
  const seen = new Set();
  for (const p of plugins) {
    if (seen.has(p.id)) problems.push(`${p.id}: listed twice in one sample`);
    seen.add(p.id);
    if (!VALID.has(p.state)) problems.push(`${p.id}: state '${p.state}' is not one of ${[...VALID].join('|')}`);
    if ('busy' in p) problems.push(`${p.id}: still carries a separate 'busy' alongside 'state'`);
    if (p.prepare && 'state' in p.prepare) problems.push(`${p.id}: prepare carries its own 'state' — a second state source`);
  }
  return problems;
}

/** The facts that IDENTIFY a plugin and say where it lives - the parts that must be
 *  byte-identical for a plugin nothing was done to. Only these are compared, because
 *  they are what an action on ANOTHER plugin could wrongly disturb (a path overwritten,
 *  an origin lost, an artifact swapped between rows).
 *
 *  Everything else is deliberately excluded, and not to hide leaks:
 *    - `state`/`detail`        a plugin's own work (runtime is being installed) moves these
 *    - `runtime`/`runtimeReady`/`runtimeSource`/`runtimeSkip`
 *                              the runtime a plugin materialises is its own, and it
 *                              finishes when it finishes - not a fact about isolation
 *    - `prepare`/`installedAt`  timestamps and the state's own words
 *  A leak that changed any of the identity fields below is still caught. */
const IDENTITY = ['id', 'pluginType', 'origin', 'path', 'version', 'artifact', 'commit', 'source', 'ref', 'icons', 'provider', 'invalid'];
export function stableFields(plugin) {
  const picked = {};
  for (const k of IDENTITY) if (k in plugin) picked[k] = plugin[k];
  return JSON.stringify(picked, Object.keys(picked).sort());
}

/** The plugins whose stable facts changed between two samples, given a set of ids the
 *  test EXPECTS to move. Anything else that changed is a leak of one plugin's action into
 *  another's facts. Returns human-readable findings. */
export function leaked(before, after, expectedToMove = []) {
  const moved = new Set(expectedToMove);
  const b = new Map(before.map((p) => [p.id, p]));
  const a = new Map(after.map((p) => [p.id, p]));
  const findings = [];
  for (const [id, was] of b) {
    if (moved.has(id)) continue;
    const now = a.get(id);
    if (!now) { findings.push(`${id}: vanished, but nothing was done to it`); continue; }
    if (stableFields(was) !== stableFields(now)) findings.push(`${id}: its facts changed though nothing was done to it`);
  }
  for (const id of a.keys()) if (!b.has(id) && !moved.has(id)) findings.push(`${id}: appeared, but nothing was done to it`);
  return findings;
}

/** Wait until `fn()` is true, polling at `everyMs`, up to `timeoutMs`. Returns whether it
 *  became true, so a test can assert on the FINAL sample rather than on a lucky one. */
export async function until(fn, { timeoutMs = 8000, everyMs = 40 } = {}) {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    if (await fn()) return true;
    if (Date.now() > deadline) return false;
    await sleep(everyMs);
  }
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

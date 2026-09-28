import { listSkillResources, readSkillResource } from "./resources.mjs";
// Agent hub — multi-harness management server.
// Transport: localhost HTTP + SSE. Zero runtime dependencies: node built-ins.
//
// Domains landed:
//   - discovery + build-id lockstep (B-0)
//   - harness list/enable/disable (H-1/H-2)
//   - sessions/turns/SSE/cancel/approvals/model-switch/repair/artifacts (S1)
//
// The core speaks §6 (adapter-v1.json) toward adapter processes; consumers
// speak /v1 (v1.json). History is read-through from the adapter, never a
// server-side copy. Assistant message ids are adapter-native, never forged.

import http from 'node:http';
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import crypto from 'node:crypto';
import { spawn, execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { getBuildId } from './build-id.mjs';
import { extractZipTo } from './zip.mjs';
import { installRuntime, platformKey, runtimeMarker, sourcesDigest } from './runtime.mjs';
import { storeSecret, getSecret, deleteSecret, listSecretNames } from './secret-store.mjs';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const DATA_DIR = process.env.AGENT_HUB_DATA_DIR || path.join(os.homedir(), '.sabishii-me', 'agent-hub');
// WHERE PLUGINS ARE LOOKED FOR. A hub that cannot find its harnesses is useless, so
// the search is a documented path rather than one directory somebody has to remember
// to pass in. In order, and every root is printed at startup:
//
//   1. AGENT_HUB_PLUGINS_DIR       — explicit, and read-only to the hub
//   2. <hub>/plugins          — a hub that carries its own (the original default)
//   3. <deployment>/plugins   — when the hub is checked out at <deployment>/apps/<name>,
//                               i.e. as part of a deployment that composes plugins
//   4. <DATA_DIR>/plugins     — the hub's OWN root, the only one it writes to
//
// `POST /v1/hub/plugins` installs into (4) and nowhere else: a directory a deployment
// put on the path is somebody else's tree and the hub only reads it. The same harness
// id in two roots is a conflict the hub refuses to start with, not a silent preference.
const GIVEN_PLUGINS_DIR = process.env.AGENT_HUB_PLUGINS_DIR || null;
const PLUGINS_DIR = GIVEN_PLUGINS_DIR || path.resolve(HERE, 'plugins');
const HUB_PLUGINS_DIR = path.join(DATA_DIR, 'plugins');
const PLUGINS_FILE = path.join(HUB_PLUGINS_DIR, 'installed.json');
// (3) only when this hub really sits inside a deployment: <deployment>/apps/<hub>
const DEPLOYMENT_PLUGINS_DIR =
  path.basename(path.dirname(HERE)) === 'apps' ? path.resolve(HERE, '..', '..', 'plugins') : null;
// One directory is one root, however it was written down: the same path
// spelled with forward slashes and with backslashes used to read as two roots,
// and the boot check then refused every plugin in it as a duplicate. Resolve
// each candidate, and compare case-insensitively where the filesystem does.
const execFileAsync = promisify(execFile);

const pluginRoots = () => {
  const out = [];
  const seen = new Set();
  for (const raw of [GIVEN_PLUGINS_DIR, path.resolve(HERE, 'plugins'), DEPLOYMENT_PLUGINS_DIR, HUB_PLUGINS_DIR].filter(Boolean)) {
    const resolved = path.resolve(raw);
    const key = process.platform === 'win32' ? resolved.toLowerCase() : resolved;
    if (seen.has(key)) continue;
    seen.add(key);
    out.push(resolved);
  }
  return out;
};
// Everything harness-specific lives under that root, inside the plugin that owns it:
// `<plugin>/extensions/<id>/` is an extension the plugin's manifest declares, and
// `<plugin>/presets/` is the preset definitions that plugin lists. The hub reads what
// a manifest declares and knows no harness by name.
function pluginDir(id) {
  for (const root of pluginRoots()) {
    if (fs.existsSync(path.join(root, id, 'manifest.json'))) return path.join(root, id);
  }
  return path.join(PLUGINS_DIR, id);
}
// A plugin's MANAGED ID is `<kind>-<id>`: the kind states what the plugin is
// The manifest states two independent things: its `id` (the plugin's own name) and its
// `kind` (its type). Neither is derived from the other. The hub's storage KEY is those two
// joined - `<kind>-<id>` - and this is the ONE place that join happens. Everywhere else a
// plugin is addressed by that key as an OPAQUE string: nothing splits it back apart, and
// nothing validates that the kind is inside it, because the kind is read from the manifest
// when it is needed. An id format may change freely; the type is unaffected.
function storageKey(kind, id) {
  if (!kind || !id) throw new Error(`a plugin needs both a kind and an id to be stored (got kind=${JSON.stringify(kind)} id=${JSON.stringify(id)})`);
  return `${kind}-${id}`;
}
// which root a plugin was found in (the writable one is the hub's own)
function pluginRootOf(id) {
  for (const root of pluginRoots()) {
    if (fs.existsSync(path.join(root, id, 'manifest.json'))) return root;
  }
  return null;
}
const pluginExtensionsDir = (id) => path.join(pluginDir(id), 'extensions');
const pluginPresetsDir = (id) => path.join(pluginDir(id), 'presets');
const pluginOrigin = (id) => (pluginRootOf(id) === HUB_PLUGINS_DIR ? 'hub' : 'deployment');
// One hub per data dir: the endpoint file is a single slot, and the hub OWNS it.
// A file left behind by a hub that was killed is stale by definition (its token is
// dead), so it is removed BEFORE the port is bound: from then on, the file's
// existence means "a hub believes it is up", and its `pid` says which one. Clients
// still check the pid — two hubs can only share a data dir by mistake.
const STARTED_AT = new Date().toISOString();
const ENDPOINT = path.join(DATA_DIR, 'endpoint.json');
const HARNESSES_FILE = path.join(DATA_DIR, 'harnesses.json');
const LEGACY_DISABLED = path.join(DATA_DIR, 'disabled.json');
const SESSIONS_FILE = path.join(DATA_DIR, 'sessions.json');
const APPROVAL_TIMEOUT = Math.max(0, Number(process.env.AGENT_HUB_APPROVAL_TIMEOUT_MS) || 0);
// Human decisions have no default deadline. Positive operator overrides are
// optional; neither approval nor question waiting consumes prompt execution time.
const QUESTION_TIMEOUT = Math.max(0, Number(process.env.AGENT_HUB_QUESTION_TIMEOUT_MS) || 0);
const CANCEL_TIMEOUT = Number(process.env.AGENT_HUB_CANCEL_TIMEOUT_MS) || 15_000;
// A turn may legitimately run for a long time (a slow/free model can take minutes
// to emit a large artefact). There is NO default wall-clock cap on a turn: the
// turn ends when the adapter reports it, when it is cancelled, or when the
// adapter dies. Operators may optionally bound it via env if they really want.
// 0 (default) = unlimited.
const TURN_TIMEOUT = Number(process.env.AGENT_HUB_TURN_TIMEOUT_MS) || 0;

// The contract this server keeps (contract/v1.json) — read, never duplicated.
// The protocol name/version a consumer is handed, and the hash it can compare
// against, come from the same file as the promises themselves, so there is no
// second copy to forget to update. A hub that cannot read its contract refuses
// to start: it would have nothing to keep.
function readContract() {
  const file = path.join(HERE, 'contract', 'v1.json');
  try {
    const raw = fs.readFileSync(file);
    const parsed = JSON.parse(raw.toString('utf8'));
    return {
      file: 'contract/v1.json',
      protocol: parsed.protocol,
      version: parsed.version,
      sha256: crypto.createHash('sha256').update(raw).digest('hex'),
    };
  } catch (e) {
    console.error(`the contract is not optional: ${file}: ${e.message}`);
    process.exit(1);
  }
}
const CONTRACT = readContract();
// The parsed document itself is NOT part of the surface (`CONTRACT` above is what
// /v1/hub/surface reports, and its shape is in the contract): it is here so the hub
// can check itself against it at boot.
const CONTRACT_DOC = (() => {
  try { return JSON.parse(fs.readFileSync(path.join(HERE, 'contract', 'v1.json'), 'utf8')); }
  catch { return { endpoints: [], events: [] }; }
})();
// The OpenAPI document is a GENERATED projection of the contract (scripts/
// emit-openapi.mjs). It is read here for the same reason the contract is: the hub
// serves it, so a hub that cannot read it must not start and pretend it can.
const OPENAPI = readOpenapi();
function readOpenapi() {
  const file = path.join(HERE, 'contract', 'openapi.json');
  try {
    const raw = fs.readFileSync(file);
    const parsed = JSON.parse(raw.toString('utf8'));
    if (parsed.openapi !== '3.1.0' || !parsed.paths) throw new Error('not an OpenAPI 3.1 document');
    return { file: 'contract/openapi.json', doc: parsed, sha256: crypto.createHash('sha256').update(raw).digest('hex') };
  } catch (e) {
    console.error(`the OpenAPI projection is not optional (run scripts/emit-openapi.mjs): ${file}: ${e.message}`);
    process.exit(1);
  }
}
const PROTOCOL = { name: CONTRACT.protocol, version: CONTRACT.version };
const BUILD_ID = getBuildId();
// The hub's OWN product version (the release it was cut from). It is read from the hub's
// own package.json, which every packed artifact carries. Exposed on /v1/hub/status so a
// client can show WHICH hub it is talking to; distinct from `protocol.version` (the wire
// protocol) and `buildId` (the build marker). null when the file is unreadable - a missing
// version is reported as absent, never invented.
const HUB_VERSION = (() => {
  try { const v = JSON.parse(fs.readFileSync(path.join(HERE, 'package.json'), 'utf8')).version; return typeof v === 'string' && v ? v : null; }
  catch { return null; }
})();

adoptLegacyDataDir();
fs.mkdirSync(DATA_DIR, { recursive: true, mode: 0o700 });

const NL = String.fromCharCode(10);
const readJson = (p, fallback) => {
  try { return JSON.parse(fs.readFileSync(p, 'utf8')); } catch { return fallback; }
};
// State the hub owns is written ATOMICALLY: a temp file followed by a rename. A
// plain writeFileSync truncates the target first, so a hub killed mid-write (the
// resume test does exactly that) leaves a half-written file, and the next boot
// reads it as "no state at all" — every session silently gone. With the rename
// there is only ever the old file or the new one.
const writeJson = (p, v, mode) => {
  fs.mkdirSync(path.dirname(p), { recursive: true });
  const tmp = `${p}.${process.pid}.tmp`;
  fs.writeFileSync(tmp, JSON.stringify(v, null, 2), mode ? { mode } : undefined);
  fs.renameSync(tmp, p);
  if (mode) { try { fs.chmodSync(p, mode); } catch {} }
};
// Changing a state file is READ-MODIFY-WRITE, and that is only safe as one indivisible
// step: if anything could run between the read and the write, two changes to different
// keys would each write a file holding only their own, and one would silently vanish.
// Node's single thread gives that indivisibility for free - but only as long as nothing
// in the middle awaits, which is an invisible rule to break. So a change goes through
// HERE, `fn` is a plain synchronous function of the current value, and the goal is
// enforced by the shape of the call rather than by remembering not to await. `writeJson`
// then only ever sees a complete value.
const mutateJson = (p, fallback, fn) => {
  const value = readStateJson(p, fallback);
  const next = fn(value);
  writeJson(p, next === undefined ? value : next);
  return next === undefined ? value : next;
};
// And a state file that cannot be parsed is not "empty": keep it (an operator can
// see what happened) and say so, instead of starting from nothing in silence.
const readStateJson = (p, fallback) => {
  if (!fs.existsSync(p)) return fallback;
  const raw = fs.readFileSync(p, 'utf8');
  try { return JSON.parse(raw); } catch (e) {
    const kept = `${p}.corrupt-${Date.now()}`;
    try { fs.renameSync(p, kept); } catch {}
    process.stderr.write(`[core] ${path.basename(p)} could not be parsed (${e.message}); kept as ${path.basename(kept)}, starting from empty state
`);
    return fallback;
  }
};

const token = crypto.randomBytes(32).toString('hex');

// --- one-time carry-over from the old data dir ----------------------------------
// The state dir used to be `~/.prts-core`. A hub that is started with no data dir
// override, finds no directory of its own and DOES find the old one adopts it by
// COPYING it into place (staged next to the target, then renamed, so a half-copied
// directory is never mistaken for a real one) and says so. The old directory is left
// where it is and untouched: sessions, providers, secrets and per-harness homes are in
// there, and nothing about this migration is worth losing them over. Session refs point
// at absolute paths, so sessions from before the migration keep reading from the old
// directory — which is exactly why it is not deleted.
function adoptLegacyDataDir() {
  if (process.env.AGENT_HUB_DATA_DIR || fs.existsSync(DATA_DIR)) return;
  const legacy = path.join(os.homedir(), '.prts-core');
  if (!fs.existsSync(legacy)) return;
  const staging = `${DATA_DIR}.migrating-${process.pid}`;
  try {
    fs.rmSync(staging, { recursive: true, force: true });
    fs.mkdirSync(path.dirname(DATA_DIR), { recursive: true });
    fs.cpSync(legacy, staging, { recursive: true });
    fs.renameSync(staging, DATA_DIR);
    process.stdout.write(`data dir: adopted ${legacy} as ${DATA_DIR} (the old directory is left untouched; delete it when nothing reads it any more)\n`);
  } catch (e) {
    fs.rmSync(staging, { recursive: true, force: true });
    process.stdout.write(`data dir: could not adopt ${legacy} (${e.message}); starting fresh at ${DATA_DIR}\n`);
  }
}
// called as soon as the function exists below — it must run before DATA_DIR is created

// ---------------------------------------------------------------------------
// harness domain: the hub manages harnesses.
//
// A plugin's manifest says what it ships — its runtime, and the extensions it
// expects to carry. The hub keeps the registration: whether the harness is
// enabled, and which extensions and skills are installed for it. Installing is
// the hub's job, so a session is handed the material it carries (extensions,
// skills, connections) and the adapter only places it where its harness reads
// it — an adapter never decides what to install, and nothing about installation
// is hardcoded in one.
// ---------------------------------------------------------------------------
// A plugin that lists presets keeps them in its own directory; a harness that has
// none is not handed a path at all, rather than an empty directory that would read
// as "this harness has no presets".
function presetsArgv(id) {
  const dir = pluginPresetsDir(id);
  return fs.existsSync(dir) ? dir : null;
}

// The plugins present on disk: a directory under the plugins root that carries a
// manifest. Nothing here knows a harness by name.
function installedPlugins() {
  const ids = new Set();
  for (const root of pluginRoots()) {
    if (!fs.existsSync(root)) continue;
    for (const d of fs.readdirSync(root)) {
      // The manifest must be read from THIS directory. Resolving by id through
      // pluginRoots() let a directory of the same name in another root vouch for
      // this one, so a manifestless leftover under the hub's own root (an
      // interrupted replace) was listed as an installed plugin.
      if (fs.existsSync(path.join(root, d, 'manifest.json'))) ids.add(d);
    }
  }
  return [...ids].sort();
}

// A harness plugin declares an adapter (command + protocol); a hub provider
// plugin declares a provider module. Both are plugins living in the same roots;
// neither is built into the hub.
function harnessPlugins() {
  return installedPlugins().filter((id) => Array.isArray((manifestOf(id) || {}).command));
}

// --- hub provider plugins ----------------------------------------------------
// A provider type is whatever a plugin ships: the hub imports the module a
// manifest declares and keeps the descriptor. With no such plugin installed,
// a provider record that names that type stays readable but unusable — there is
// no built-in fallback and no generic guess about what a vendor accepts.
const PROVIDER_TYPE_INDEX = new Map();   // `${descriptor.id}@${descriptor.version}` -> module (+ pluginId)
const PROVIDER_PLUGIN_FAULTS = [];       // modules that were declared but could not load
function providerModuleEntry(pluginId) {
  const m = manifestOf(pluginId) || {};
  const decl = m.provider;
  if (!decl || typeof decl !== 'object') return null;
  if (decl.apiVersion !== 1) return { error: `provider.apiVersion ${JSON.stringify(decl.apiVersion)} is not supported (1)` };
  if (typeof decl.module !== 'string' || !decl.module) return { error: 'provider.module must be a module path inside the plugin' };
  const dir = path.resolve(pluginDir(pluginId));
  const modulePath = path.resolve(dir, decl.module);
  if (modulePath !== dir && !modulePath.startsWith(dir + path.sep)) return { error: 'provider.module must stay inside the plugin directory' };
  if (!fs.existsSync(modulePath)) return { error: `provider.module not found: ${decl.module}` };
  return { modulePath };
}
// Load the provider modules the INSTALLED plugins declare. It is re-entrant: a plugin
// installed or removed while the hub runs must change what the provider surface offers,
// and a loader that only ever runs at boot leaves a freshly installed provider with no
// types - the plugin reads `ready` and cannot be used.
async function loadProviderPlugins() {
  PROVIDER_TYPE_INDEX.clear();
  PROVIDER_PLUGIN_FAULTS.length = 0;
  for (const pluginId of installedPlugins()) {
    const entry = providerModuleEntry(pluginId);
    if (!entry) continue;
    if (entry.error) { PROVIDER_PLUGIN_FAULTS.push({ plugin: pluginId, error: entry.error }); continue; }
    try {
      const { default: mod } = await import(pathToFileURL(entry.modulePath).href);
      const d = mod && mod.descriptor;
      if (mod?.apiVersion !== 1 || !d || typeof d.id !== 'string' || !d.id || !Number.isInteger(d.version) || d.version < 1 || typeof mod.configure !== 'function' || typeof mod.fetchCatalog !== 'function') {
        throw new Error('a provider module needs apiVersion 1, descriptor {id,version,...}, configure() and fetchCatalog()');
      }
      const key = `${d.id}@${d.version}`;
      if (PROVIDER_TYPE_INDEX.has(key)) throw new Error(`duplicate provider type ${key}`);
      PROVIDER_TYPE_INDEX.set(key, Object.assign(Object.create(mod), { pluginId }));
    } catch (e) {
      PROVIDER_PLUGIN_FAULTS.push({ plugin: pluginId, error: e && e.message ? e.message : String(e) });
      process.stderr.write(`[hub] provider plugin '${pluginId}' could not be loaded: ${e && e.message ? e.message : e}
`);
    }
  }
}
const providerTypeIdentity = (row) => ({
  providerType: row.providerType === undefined ? 'custom-compatible' : row.providerType,
  providerTypeVersion: row.providerTypeVersion === undefined ? 1 : row.providerTypeVersion,
});
function findProviderType(row) {
  const identity = providerTypeIdentity(row);
  return PROVIDER_TYPE_INDEX.get(`${identity.providerType}@${identity.providerTypeVersion}`) || undefined;
}
function requireProviderType(row) {
  const plugin = findProviderType(row);
  if (!plugin) throw Object.assign(new Error('provider type or version is unavailable'), { code: 'unsupported' });
  return plugin;
}
const providerTypes = () => ({
  types: [...PROVIDER_TYPE_INDEX.values()].map((mod) => ({ ...mod.descriptor, plugin: mod.pluginId })),
  broken: PROVIDER_PLUGIN_FAULTS.slice(),
});

// Where the runtime the manifest pins should land: the manifest's command, resolved
// the same way the hub resolves it for the adapter, minus the program itself. The hub
// only ever CHECKS this path — materialising it is the plugin's own job.
function runtimeTarget(id) {
  const m = manifestOf(id) || {};
  const rt = m.runtime;
  if (!rt || !Array.isArray(rt.command) || !rt.command.length) return null;
  const rel = rt.command.slice(1).find((p) => !String(p).startsWith('-'));
  return rel ? path.resolve(pluginDir(id), rel) : null;
}
// What is unpacked under this plugin's runtime directory, from the marker the
// installer writes. `pin` is the manifest declaration; this is the disk's own
// answer, which can disagree after an interrupted replace.
function installedRuntime(id) {
  const target = runtimeTarget(id);
  if (!target || !fs.existsSync(target)) return null;
  const marker = runtimeMarker(target);
  const entry = path.join(target, ...String((manifestOf(id) || {}).runtime?.command?.slice(1).find((p) => !String(p).startsWith('-')) || '').split('/'));
  return {
    package: marker && typeof marker.package === 'string' ? marker.package : null,
    version: marker && typeof marker.version === 'string' ? marker.version : null,
    installedAt: marker && typeof marker.installedAt === 'string' ? marker.installedAt : null,
    sources: marker && typeof marker.sources === 'number' ? marker.sources : null,
    entryPresent: fs.existsSync(entry),
    target,
  };
}

// Directories under the hub's OWN plugins root that carry no manifest: an
// interrupted replace or a failed install leaves these, and nothing else in the
// hub can see them (every reader looks for a manifest). They hold a runtime and
// occupy space while appearing as "not installed" to a client, so the hub names
// them and can clear them on request. A deployment's directory is never listed:
// those roots are not the hub's to clean.
function orphanPluginDirs() {
  const out = [];
  if (!fs.existsSync(HUB_PLUGINS_DIR)) return out;
  for (const entry of fs.readdirSync(HUB_PLUGINS_DIR, { withFileTypes: true })) {
    if (!entry.isDirectory()) continue;
    const id = entry.name;
    if (id.startsWith('.')) continue;                 // staging/aside/carry dirs are transient
    const dir = path.join(HUB_PLUGINS_DIR, id);
    if (fs.existsSync(path.join(dir, 'manifest.json'))) continue;
    let bytes = null;
    try { bytes = dirSize(dir); } catch { bytes = null; }
    out.push({ id, path: dir, bytes, hasRuntime: fs.existsSync(path.join(dir, 'runtime')) });
  }
  return out.sort((a, b) => a.id.localeCompare(b.id));
}

function dirSize(dir) {
  let total = 0;
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) total += dirSize(full);
    else if (entry.isFile()) { try { total += fs.statSync(full).size; } catch { /* a file vanishing mid-walk is not an error here */ } }
  }
  return total;
}

function runtimeReady(id) {
  const target = runtimeTarget(id);
  return target ? fs.existsSync(target) : true;   // no declared runtime = nothing to prepare
}

function manifestOf(id) {
  const mf = path.join(pluginDir(id), 'manifest.json');
  return fs.existsSync(mf) ? readJson(mf, null) : null;
}

// Why a plugin cannot be used, or null when it can. A manifest is a promise
// about a plugin, and every field in it is read by something: `command` is how
// the adapter is spawned, `protocol` is which adapter protocol it speaks,
// `runtime` is the harness it drives, `capabilities` is what the hub may ask it
// for, `extensions` is what it ships. A manifest that cannot be honoured is
// refused here with its reason, rather than spawning a process that then fails
// in a way nobody can explain.
const ADAPTER_PROTOCOL = readJson(path.join(HERE, 'contract', 'adapter-v1.json'), {}).version ?? null;
// The declared status of every error code (contract/errors.json). failError()
// uses it, and the boot self-check refuses to start if a literal disagrees with it
// against it.
// One table, three facts: the status a code is answered with, whether retrying it
// unchanged can work, and the message default. errors.json is the master; a code
// that is not in it cannot be answered (the contract trial prints any that are).
const ERROR_TABLE = readJson(path.join(HERE, 'contract', 'errors.json'), { errors: {} }).errors;
const ERROR_STATUS = Object.fromEntries(Object.entries(ERROR_TABLE).map(([k, v]) => [k, v.http]));
// The body carries `retryable` instead of leaving the caller to find the table: a
// client that has to guess whether a 409 is worth another attempt will guess wrong,
// and the table already knows.
function errorBody(code, message, extra) {
  const row = ERROR_TABLE[code] || null;
  return { error: { code, message, retryable: row ? row.retryable === true : false }, ...(extra || {}) };
}
function manifestFault(id) {
  const m = manifestOf(id);
  if (!m) return 'no manifest.json';
  // The manifest states the plugin's own `id` (its name) and its `pluginType` (its type);
  // the directory is the two joined. The directory must be exactly that join, so a
  // manifest that does not belong to this directory is caught - and the id is never
  // compared to the directory alone, because the id does not carry the type.
  if (typeof m.pluginType !== 'string' || !m.pluginType) return `manifest declares no pluginType`;
  // The storage key is `<pluginType>-<id>`, built once at install; the directory must be
  // that key. This compares the JOIN, not the id alone: the id is the plugin's own name
  // and says nothing about its type.
  if (storageKey(m.pluginType, m.id) !== id) return `manifest id '${m.id}' with pluginType '${m.pluginType}' is not the plugin directory '${id}'`;
  const isHarness = m.pluginType === 'harness-adapter';
  if (m.provider !== undefined) {
    const entry = providerModuleEntry(id);
    if (entry && entry.error) return entry.error;
  }
  if (!isHarness && m.provider === undefined) {
    return 'a plugin declares either command (harness adapter) or provider (hub provider module)';
  }
  if (!isHarness) return null;   // a provider-only plugin has no adapter to validate
  if (!Number.isInteger(m.protocol)) return 'protocol must be an integer (the adapter protocol it speaks)';
  if (m.protocol !== ADAPTER_PROTOCOL) return `protocol ${m.protocol} is not the adapter protocol this hub speaks (${ADAPTER_PROTOCOL})`;
  if (!Array.isArray(m.command) || m.command.length === 0 || m.command.some((c) => typeof c !== 'string')) {
    return 'command must be a non-empty argv array';
  }
  if (m.runtime !== undefined) {
    const r = m.runtime;
    if (!r || typeof r !== 'object') return 'runtime must be an object';
    if (typeof r.package !== 'string' || !r.package) return 'runtime.package must be a package name';
    if (typeof r.version !== 'string' || !r.version) return 'runtime.version must be a version';
    if (!Array.isArray(r.command) || r.command.length === 0) return 'runtime.command must be a non-empty argv array';
  }
  for (const key of ['capabilities', 'extensions']) {
    if (m[key] !== undefined && (!Array.isArray(m[key]) || m[key].some((x) => typeof x !== 'string'))) {
      return `${key} must be an array of strings`;
    }
  }
  return null;
}
function loadHarnessRows() { return readStateJson(HARNESSES_FILE, []); }
function persistHarnessRows(rows) { writeJson(HARNESSES_FILE, rows); }

// The extensions a plugin can install: the directories inside its own
// `extensions/` folder. Which of them it wants is the manifest's `extensions` list,
// so a plugin ships its extensions and the hub installs what it is told to install.
function availableExtensions(id) {
  const dir = pluginExtensionsDir(id);
  if (!fs.existsSync(dir)) return [];
  return fs.readdirSync(dir)
    .filter((n) => { try { return fs.statSync(path.join(dir, n)).isDirectory(); } catch { return false; } })
    .sort();
}

// Reconcile the registration with what is actually installed. A plugin that is
// present gets a row the first time it is seen (enabled, carrying the
// extensions its manifest ships); a plugin that is gone keeps its row, marked
// missing — the row is also where that harness's install state lives, so it is
// not silently dropped, and a returning plugin finds it again.
function reconcileHarnesses() {
  const rows = loadHarnessRows();
  const byId = new Map(rows.map((r) => [r.id, r]));
  const onDisk = harnessPlugins();
  let changed = false;
  // one-time carry-over from the old shape (a bare array of disabled ids)
  const legacy = readJson(LEGACY_DISABLED, null);
  const legacyDisabled = new Set(Array.isArray(legacy) ? legacy : []);
  for (const id of onDisk) {
    const existing = byId.get(id);
    if (existing) {
      if (existing.missing) { existing.missing = false; existing.updatedAt = new Date().toISOString(); changed = true; }
      continue;
    }
    const m = manifestOf(id) || {};
    const now = new Date().toISOString();
    const row = {
      id,
      enabled: !legacyDisabled.has(id),
      // what the plugin ships by default; the hub may change it afterwards
      extensions: Array.isArray(m.extensions) ? m.extensions.filter((e) => availableExtensions(id).includes(e)) : [],
      // null = every skill the hub holds; an array = exactly those
      skills: null,
      registeredAt: now,
      updatedAt: now,
      missing: false,
    };
    rows.push(row); byId.set(id, row); changed = true;
  }
  for (const row of rows) {
    const present = onDisk.includes(row.id);
    if (row.missing !== !present) { row.missing = !present; changed = true; }
  }
  if (changed) {
    persistHarnessRows(rows);
    // the old file has been folded in; it is no longer a source
    if (Array.isArray(legacy)) fs.rmSync(LEGACY_DISABLED, { force: true });
  }
  return rows;
}
function harnessRow(id) { return reconcileHarnesses().find((r) => r.id === id) || null; }
function listHarnesses() {
  const out = reconcileHarnesses().filter((r) => !r.missing).map((row) => {
    const m = manifestOf(row.id) || {};
    return {
      id: row.id,
      name: m.name || row.id,
      base: m.base || row.id,
      icons: harnessIcons(row.id),
      // Two facts, both published (ADR-0004). `adapterVersion` is this deployment's
      // release of the plugin; `runtimeVersion` is the harness it drives. Neither is
      // derivable from the other, so neither is dropped - and there is deliberately
      // no bare `version`, which is the ambiguity that made the roster answer a
      // different question than the harness routes did.
      adapterVersion: m.version || null,
      runtimeVersion: (m.runtime && m.runtime.version) || null,
      runtimePackage: (m.runtime && m.runtime.package) || null,
      capabilities: m.capabilities || [],
      status: row.enabled ? 'enabled' : 'disabled',
      permissionModel: m.permissionModel || 'none',
      planMode: m.planMode || 'none',
      repair: m.repair || 'none',
      // null when the plugin can be used; the reason when it cannot.
      invalid: manifestFault(row.id),
    };
  });
  out.sort((a, b) => a.id.localeCompare(b.id));
  return out;
}

// errors.json declares `harness_disabled` (409, "deactivated; activate before
// use"). Use = create a session, or send a turn into one.
function isHarnessEnabled(id) {
  const row = harnessRow(id);
  return row ? row.enabled === true : false;
}

// The harness's icon, taken from its own manifest and returned as a data URI.
// The icon's URL for a harness, so a client draws the plugin's own mark without the
// hub inlining bytes (an inlined mark does not survive hundreds of harnesses). The
// bytes are served by the icon route below; a harness with no declared icon returns
// null and the client draws none.
function harnessIcons(id) {
  const m = manifestOf(id) || {};
  const decl = m.icons;
  if (!decl || typeof decl !== 'object') return null;
  return {
    light: `/v1/hub/plugins/${encodeURIComponent(id)}/icon/light`,
    dark: `/v1/hub/plugins/${encodeURIComponent(id)}/icon/dark`,
  };
}

// Serve one variant of a harness's icon from its manifest. 404 when the harness or
// that variant is absent.
// A mark's content type, from its file name. This is why the hub serves the bytes:
// a release host hands an SVG back as application/octet-stream with nosniff, which a
// browser will not draw, while the same bytes under image/svg+xml draw fine.
function iconMime(name) {
  const ext = path.extname(name || '').toLowerCase();
  return ext === '.svg' ? 'image/svg+xml' : ext === '.png' ? 'image/png' : ext === '.webp' ? 'image/webp' : 'application/octet-stream';
}

// Serve one variant of a harness's icon. An installed harness has the file on disk;
// one that is not installed has only the URL its registry entry names, fetched here.
// Either way the bytes go out under the mark's real content type. 404 when the
// harness declares no mark or its bytes are nowhere.
async function serveHarnessIcon(id, variant, res) {
  const m = manifestOf(id) || {};
  const decl = m.icons && typeof m.icons === 'object' ? m.icons : null;
  if (decl) {
    const name = variant === 'dark' ? (decl.dark ?? decl.light) : (decl.light ?? decl.dark);
    if (typeof name === 'string' && name) {
      const dir = pluginDir(id);
      const full = path.resolve(dir, name);
      if (full !== dir && full.startsWith(dir + path.sep)) {
        try {
          const buf = fs.readFileSync(full);
          res.writeHead(200, { 'content-type': iconMime(name), 'cache-control': 'no-cache' });
          return res.end(buf);
        } catch { /* not on disk: fall through to the registry URL */ }
      }
    }
  }
  // `id` here is the plugin's storage key (`<kind>-<id>`); a registry entry carries the
  // two fields separately. Match by composing each entry the same way, so an UNINSTALLED
  // plugin (no directory, no manifest) still finds the icon its entry names.
  const entry = (catalogValue().plugins || []).find((e) => e && e.id && e.pluginType && storageKey(e.pluginType, e.id) === id);
  const icon = entry && entry.icon && typeof entry.icon === 'object' ? entry.icon : null;
  const url = icon ? (variant === 'dark' ? (icon.dark ?? icon.light) : (icon.light ?? icon.dark)) : null;
  if (typeof url !== 'string' || !url) return fail(res, 404, 'not_found', `no '${variant}' icon for '${id}'`);
  let answer;
  try {
    answer = await fetch(url, { redirect: 'follow', signal: AbortSignal.timeout(ARTIFACT_TIMEOUT) });
    if (!answer.ok) throw new Error(`HTTP ${answer.status}`);
  } catch (e) {
    return fail(res, 502, 'artifact_download_failed', `${url}: ${e.message}`);
  }
  const buf = Buffer.from(await answer.arrayBuffer());
  res.writeHead(200, { 'content-type': iconMime(url), 'cache-control': 'no-cache' });
  res.end(buf);
}

// The hub's view of one harness: what the plugin is, plus what the hub has
// installed for it.
function harnessValue(row) {
  const m = manifestOf(row.id) || {};
  return {
    id: row.id,
    icons: harnessIcons(row.id),
    enabled: row.enabled === true,
    extensions: Array.isArray(row.extensions) ? row.extensions : [],
    skills: Array.isArray(row.skills) ? row.skills : null,
    missing: row.missing === true,
    runtime: m.runtime || null,
    // Two facts, both published (ADR-0004). The `pin` field that used to sit here
    // was a SECOND NAME for `runtime.version` - the same value twice, which is why
    // removing it was right. These two are different values: the adapter's release
    // and the runtime it drives. Neither may be dropped in favour of the other.
    adapterVersion: m.version || null,
    runtimeVersion: (m.runtime && m.runtime.version) || null,
    runtimePackage: (m.runtime && m.runtime.package) || null,
    capabilities: m.capabilities || [],
    registeredAt: row.registeredAt || null,
    updatedAt: row.updatedAt || null,
    invalid: manifestFault(row.id),
  };
}
function updateHarness(id, patch, res) {
  if (rejectUnknownFields(res, patch, ['extensions', 'skills', 'enabled'], 'PATCH /v1/hub/harnesses/{id}')) return;
  const rows = loadHarnessRows();
  const existing = rows.find((r) => r.id === id);
  if (!existing) {
    if (!manifestOf(id)) { if (res) return fail(res, 404, 'harness_not_found', `no harness ${id}`); return null; }
    reconcileHarnesses();
    return updateHarness(id, patch, res);
  }
  if (patch.extensions !== undefined) {
    if (!Array.isArray(patch.extensions) || patch.extensions.some((e) => typeof e !== 'string')) {
      if (res) return fail(res, 400, 'validation_failed', 'extensions must be an array of names');
    }
    const available = availableExtensions(id);
    const unknown = patch.extensions.filter((e) => !available.includes(e));
    if (unknown.length) {
      if (res) return fail(res, 400, 'validation_failed', `unknown extension(s): ${unknown.join(', ')} (available: ${available.join(', ') || 'none'})`);
    }
    existing.extensions = [...patch.extensions];
  }
  if (patch.skills !== undefined) {
    if (patch.skills !== null && (!Array.isArray(patch.skills) || patch.skills.some((s) => typeof s !== 'string'))) {
      if (res) return fail(res, 400, 'validation_failed', 'skills must be null (all) or an array of skill ids');
    }
    existing.skills = patch.skills === null ? null : [...patch.skills];
  }
  if (patch.enabled !== undefined) existing.enabled = patch.enabled === true;
  existing.updatedAt = new Date().toISOString();
  persistHarnessRows(rows);
  const value = harnessValue(existing);
  return res ? json(res, 200, { harness: value }) : value;
}

// ---------------------------------------------------------------------------
// session domain
// ---------------------------------------------------------------------------
const sessions = new Map(); // sid -> metadata record
const adapters = new Map(); // sid -> adapter conn
const configuringSessions = new Set(); // in-flight configuration, never persisted
const approvals = new Map(); // aid -> approval record
const questions = new Map(); // qid -> question record
let eventSeq = 0;

function loadSessions() {
  for (const s of readStateJson(SESSIONS_FILE, [])) {
    if (s && s.id) {
      // No terminal event survived the restart -> honest 'unknown', never 'completed'.
      s.activeTurn = { state: 'unknown', ended: null, cause: null, partialPersisted: false, partialItems: 0 };
      sessions.set(s.id, s);
    }
  }
}
function persistSessions() {
  writeJson(SESSIONS_FILE, [...sessions.values()].map(({ activeTurn: _drop, ...s }) => s));
}

// C-3: a session keeps thin per-turn entries {turnId, startedAt, ended, cause}
// (details are fetched separately, read-through). core is the sole truth for the
// turn state machine, so it records the entry itself; it is NOT derived from the
// adapter's message history (which carries no turn boundaries or causes).
let turnSeq = 0;
function beginTurn(s) {
  const turnId = `t-${process.pid}-${++turnSeq}-${crypto.randomBytes(2).toString('hex')}`;
  s.currentTurnId = turnId;
  s.currentTurnStarted = new Date().toISOString();
  return turnId;
}
function endTurn(s, ended, cause) {
  if (!s.currentTurnId) return;
  s.turns = s.turns || [];
  s.turns.push({ turnId: s.currentTurnId, startedAt: s.currentTurnStarted, ended, cause: cause ?? null });
  s.currentTurnId = null;
  s.currentTurnStarted = null;
}

// The SecretStore master key belongs to the hub, never to harnesses/tools.
function adapterEnvironment() {
  const env = { ...process.env };
  delete env.AGENT_HUB_SECRET_KEY;
  return env;
}

// --- adapter process (one process per session, §6 topology) ------------------
function spawnAdapter(sid, harnessId, cwd, additionalDirectories = null) {
  const m = manifestOf(harnessId);
  if (!m) throw Object.assign(new Error('harness_not_found'), { code: 'harness_not_found' });
  const fault = manifestFault(harnessId);
  if (fault) {
    throw Object.assign(new Error(`harness '${harnessId}' cannot be used: ${fault}`), { code: 'harness_invalid' });
  }
  const dir = pluginDir(harnessId);
  const [cmd, ...rawArgs] = m.command;
  // The script argument is resolved against the plugin directory, so the process's
  // command line NAMES its plugin. A bare `deepseek-adapter.cjs` in the command line is
  // how a running adapter survived a removal once: the removal finds a harness's process
  // by what it was started with, and a relative path names nothing it can match.
  const args = rawArgs.map((part) => (part.startsWith('-') ? part : path.resolve(dir, part)));
  // The runtime an adapter drives is declared by its manifest, not discovered by
  // the adapter: there is no "system install" to fall back to, and an adapter
  // that went looking on its own would make the pin a comment instead of a fact.
  // The declaration is passed as an absolute argv, resolved against the plugin
  // directory the same way `command` is, so a relative path in the manifest means
  // "inside this plugin".
  const runtimeArgv = Array.isArray(m.runtime && m.runtime.command) && m.runtime.command.length
    ? m.runtime.command.map((part, i) => (i === 0 ? part : path.resolve(dir, part)))
    : null;
  // One adapter process per session (the v1 topology). How an adapter backs its
  // harness — a private child, or a shared server other adapters also attach
  // to — is the adapter's own business; the hub neither knows nor decides.
  const connKey = sid;
  const existing = adapters.get(connKey);
  if (existing) return existing;
  // One data dir PER HARNESS (shared by the config plane and every session of
  // that harness), matching the Rust owner's data_dir = agent-data/<manifest.id>.
  // Splitting per-session would create isolated dsh-homes so a provider saved by
  // the config plane is invisible to the session (D-5).
  const agentDir = path.join(DATA_DIR, 'agents', harnessId);
  fs.mkdirSync(agentDir, { recursive: true });

  const proc = spawn(cmd, args, {
    windowsHide: true,
    cwd: dir,
    // (inherit the terminal's own login/upstream). private = isolated home.
    env: { ...adapterEnvironment(), AGENT_HUB_HARNESS_DIR: agentDir, AGENT_HUB_CWD: cwd || process.cwd(), ...(Array.isArray(additionalDirectories) && additionalDirectories.length ? { AGENT_HUB_ADDITIONAL_DIRS: JSON.stringify(additionalDirectories) } : {}), AGENT_HUB_SESSION_ID: sid, AGENT_HUB_INSTALLED_EXTENSIONS_DIR: installExtensions(harnessId).dir, AGENT_HUB_INSTALLED_SKILLS_DIR: installSkills(harnessId).dir, ...(presetsArgv(harnessId) ? { AGENT_HUB_PRESETS_DIR: presetsArgv(harnessId) } : {}), ...(runtimeArgv ? { AGENT_HUB_RUNTIME_COMMAND: JSON.stringify(runtimeArgv) } : {}), ...connectionEnv() },
    // stderr is CAPTURED as well as echoed: when an adapter dies before it can
    // answer, its own last words are the only useful part of the error, and a
    // caller who forgot to materialise the runtime should be told that instead of
    // reading `adapter exited code=2`.
    stdio: ['pipe', 'pipe', 'pipe'],
  });

  const conn = {
    proc,
    tail: [],          // the last few lines the adapter said on stderr
    sid,
    harnessId,
    nextId: 1,
    pending: new Map(),
    buf: '',
    started: false,   // session/start succeeded on THIS process
    ready: false,     // start + grant + config/set all succeeded
    initializing: null, // in-flight init promise, shared by concurrent callers
    onEvent: null, // (sid, data) => void
  };
  adapters.set(connKey, conn);

  proc.stderr.setEncoding('utf8');
  proc.stderr.on('data', (d) => {
    for (const line of String(d).split(NL)) {
      if (!line.trim()) continue;
      process.stderr.write(line + NL);
      conn.tail.push(line.trim());
      // Keep enough of it to be a reason: six lines of a Node stack trace is the top of
      // the stack, and the line that says what actually happened is at the bottom.
      if (conn.tail.length > 20) { conn.tail.shift(); conn.tailDropped = (conn.tailDropped || 0) + 1; }
    }
  });
  proc.on('error', (e) => {
    for (const [, p] of conn.pending) { clearTimeout(p.timer); p.reject(new Error(`adapter could not be started: ${e.message}`)); }
    conn.pending.clear();
    if (adapters.get(connKey) === conn) adapters.delete(connKey);
  });
  proc.stdout.setEncoding('utf8');
  proc.stdout.on('data', (d) => {
    conn.buf += d;
    let i;
    while ((i = conn.buf.indexOf('\n')) >= 0) {
      const line = conn.buf.slice(0, i); conn.buf = conn.buf.slice(i + 1);
      if (line.trim()) handleAdapterMessage(conn, line);
    }
  });
  proc.on('exit', (code) => {
    const s = sessions.get(sid);
    if (adapters.get(connKey) === conn && turnIsOpen(s)) {
      s.activeTurn = { state: 'ended', ended: 'interrupted', cause: 'adapter-crash', partialPersisted: true, partialItems: 0 };
      endTurn(s, 'interrupted', 'adapter-crash');
      emitSessionEvent(sid, 'turn.ended', { turn: s.activeTurn });
      // read-through truth: the adapter's transcript holds whatever persisted.
    }
    // The whole tail, not its last line: the last line is usually the adapter saying
    // "the harness exited", and the harness's own reason is a few lines above it.
    const said = conn.tail.length
      ? `${conn.tailDropped ? `(${conn.tailDropped} earlier line(s) omitted)${NL}` : ""}${conn.tail.join(NL)}`
      : "";
    for (const [, p] of conn.pending) { clearTimeout(p.timer); p.reject(new Error(`adapter exited code=${code}${said ? ` — it said:${NL}${said}` : ""}`)); }
    conn.pending.clear();
    if (adapters.get(connKey) === conn) adapters.delete(connKey);
  });
  proc.stdin.on('error', () => {});

  return conn;
}

// The requests the hub may send to an adapter, as data — the same discipline as
// ROUTES. Every send passes declareAdapterRequest() first, and
// the boot self-check makes this list and contract/adapter-v1.json
// identical: a request the hub sends without declaring it is a request no
// plugin was ever told to answer, and it would fail deep inside an adapter
// instead of here.
const ADAPTER_REQUESTS = [
  'session/start',
  'config/set',
  'session/prompt',
  'session/abort',
  'history/page',
  'history/read',
  'credentials/grant',
  'models/list',
  'presets/list',
  'tools/list',
  'session/fork',
  'session/stats',
  'session/compact',
  'session/rename',
  'skills/list',
  'runtime/prepare',
  // harness-private connections (capability: providers) — the harness's own
  // provider surface, forwarded only for a harness that declares it.
  'connections/schema',
  'connections/list',
  'connections/validate',
  'connections/save',
  'connections/delete',
  'auth/start',
  'auth/status',
  'auth/cancel',
];
const ADAPTER_REQUEST_SET = new Set(ADAPTER_REQUESTS);

function declareAdapterRequest(method) {
  if (!ADAPTER_REQUEST_SET.has(method)) {
    throw new Error(`undeclared adapter request '${method}': add it to ADAPTER_REQUESTS in server.mjs and to contract/adapter-v1.json`);
  }
}

// Several parallel tool approvals share one execution budget. Resume only after
// the last human decision is resolved; retries must not replenish the budget.
function hasHumanWait(conn, sid) {
  return [...approvals.values(), ...questions.values()].some(r => r.conn === conn && r.sid === sid && r.state === 'pending');
}
function updateHumanWait(conn, sid) {
  const waiting = hasHumanWait(conn, sid);
  for (const pending of conn.pending.values()) {
    if (pending.executionBudget && pending.sid === sid) {
      if (waiting) pending.pause(); else if (pending.paused) pending.resume();
    }
  }
  const s = sessions.get(sid);
  if (s && ['running', 'awaiting_approval', 'awaiting_question'].includes(s.activeTurn.state)) {
    const approval = [...approvals.values()].some(r => r.conn === conn && r.sid === sid && r.state === 'pending');
    s.activeTurn.state = approval ? 'awaiting_approval' : waiting ? 'awaiting_question' : 'running';
  }
}

function rpc(conn, method, params, timeoutMs = 30_000) {
  declareAdapterRequest(method);
  return new Promise((resolve, reject) => {
    const id = conn.nextId++;
    // timeoutMs <= 0 means no wall-clock bound: the call settles only when the
    // adapter answers (or crashes). Used for session/prompt, whose duration is
    // the model's business, not ours.
    const pending = { resolve, reject, timer: null, remaining: timeoutMs,
      started: 0, paused: false, sid: params.sid || conn.sid,
      executionBudget: method === 'session/prompt' };
    pending.resume = () => {
      if (timeoutMs <= 0 || pending.timer) return;
      pending.paused = false;
      pending.started = performance.now();
      pending.timer = setTimeout(() => {
        conn.pending.delete(id);
        reject(new Error(`${method} timed out`));
      }, Math.max(0, pending.remaining));
    };
    pending.pause = () => {
      if (pending.timer) {
        clearTimeout(pending.timer); pending.timer = null;
        pending.remaining = Math.max(0, pending.remaining - (performance.now() - pending.started));
      }
      pending.paused = true;
    };
    conn.pending.set(id, pending);
    if (pending.executionBudget && hasHumanWait(conn, pending.sid)) pending.pause();
    else pending.resume();
    conn.proc.stdin.write(JSON.stringify({ jsonrpc: '2.0', id, method, params }) + '\n');
  });
}

function handleAdapterMessage(conn, line) {
  let msg;
  try { msg = JSON.parse(line); } catch { return; }
  if (msg.method === 'event') {
    const { sid, data } = msg.params || {};
    if (sid === conn.sid && data && adapters.get(conn.sid) === conn) handleAdapterEvent(sid, data);
    return;
  }
  if (msg.method === 'approval_need') {
    handleApprovalRequest(conn, msg);
    return;
  }
  if (msg.method === 'question_need') {
    handleQuestionRequest(conn, msg);
    return;
  }  const p = conn.pending.get(msg.id);
  if (p) {
    conn.pending.delete(msg.id);
    clearTimeout(p.timer);
    if (msg.error) p.reject(Object.assign(new Error(msg.error.message), { code: msg.error.code, data: msg.error.data }));
    else p.resolve(msg.result);
  }
}

function handleAdapterEvent(sid, data) {
  const s = sessions.get(sid);
  if (!s) return;
  switch (data.type) {
    case 'turn_started':
      s.activeTurn = { state: 'running', ended: null, cause: null, partialPersisted: false, partialItems: 0 };
      emitSessionEvent(sid, 'turn.running', { turn: s.activeTurn });
      break;
    case 'text_delta':
    case 'reasoning_delta':
      emitSessionEvent(sid, 'message.delta', { messageId: data.messageId, text: data.text, kind: data.type === 'reasoning_delta' ? 'reasoning' : 'text' });
      break;
    case 'message_end':
      s.activeTurn.partialPersisted = true;
      emitSessionEvent(sid, 'message.completed', { messageId: data.messageId, role: data.role, text: data.text });
      break;
    case 'tool_started':
    case 'tool_end':
      emitSessionEvent(sid, `tool.${data.type === 'tool_started' ? 'started' : 'ended'}`, data);
      break;
    case 'plan_changed':
      // The harness changed plan mode on its own (the model leaves plan when its
      // plan is approved). Record it, so a session read reports the state the
      // harness is actually in, not the last one the core requested.
      s.appliedPlan = data.plan === true;
      s.updatedAt = new Date().toISOString();
      persistSessions();
      emitSessionEvent(sid, 'plan.changed', { plan: s.appliedPlan });
      break;
    case 'title_changed':
      // The harness renamed the session on its own (dsh titles a session from the
      // first prompt; a harness UI can rename one). The hub follows: it records
      // the name the harness now holds and says so on the stream, instead of
      // showing a title that only the hub still believes in.
      s.appliedTitle = data.title ?? null;
      s.updatedAt = new Date().toISOString();
      persistSessions();
      emitSessionEvent(sid, 'session.renamed', { sessionId: sid, title: s.appliedTitle, appliedBy: 'harness' });
      break;
    case 'compaction_started':
      // The harness is rewriting its own context — because the hub asked, or
      // because it decided to (a full context compacts mid-turn). Both are
      // reported: a caller reading this stream must not go quiet while the
      // conversation being shown is being replaced.
      emitSessionEvent(sid, 'session.compacting', { sessionId: sid, reason: data.reason ?? null });
      break;
    case 'compaction_ended': {
      const facts = { sessionId: sid, reason: data.reason ?? null, aborted: data.aborted === true };
      if (typeof data.tokensBefore === 'number') facts.tokensBefore = data.tokensBefore;
      if (typeof data.tokensAfter === 'number') facts.tokensAfter = data.tokensAfter;
      if (typeof data.willRetry === 'boolean') facts.willRetry = data.willRetry;
      if (typeof data.summary === 'string') facts.summary = data.summary;
      if (data.usage) facts.usage = data.usage;
      if (typeof data.detail === 'string') facts.detail = data.detail;
      s.updatedAt = new Date().toISOString();
      persistSessions();
      emitSessionEvent(sid, 'session.compacted', facts);
      break;
    }
    case 'turn_end': {
      const st = data.state ?? data.status; // canonical: state; status tolerated during migration
      const ended = st === 'aborted' ? 'cancelled' : st === 'failed' ? 'failed' : 'completed';
      const cause = ended === 'cancelled' ? 'user-cancel' : ended === 'failed' ? 'model-error' : null;
      s.activeTurn = { state: 'ended', ended, cause, partialPersisted: s.activeTurn.partialPersisted, partialItems: 0 };
      endTurn(s, ended, cause);
      s.updatedAt = new Date().toISOString();
      persistSessions();
      emitSessionEvent(sid, 'turn.ended', { turn: s.activeTurn });
      closeTurnStream(sid, connFor(sid));
      break;
    }
    case 'notification':
      if (typeof data.message === 'string') emitSessionEvent(sid, 'session.notification', {
        sessionId: sid, message: data.message.slice(0, 32768),
        level: ['info', 'warning', 'error'].includes(data.level) ? data.level : 'info',
      });
      break;
    case 'adapter_dialog_auto_cancelled':
      // The adapter cancelled a harness dialog nobody rendered, so the agent
      // does not hang waiting for a window that is not there. Nothing in the hub
      // consumes it yet: dropped deliberately, not silently — the fact is in the
      // adapter protocol (adapter-v1.json) and the case is here so that "nobody
      // handles this" is a decision on the record instead of a fall-through.
      break;
    default:
      // contract-whitelisted events only; adapter-side filter drops the rest
      break;
  }
}

// The SSE event names this hub can emit, as data — the same discipline as the
// route table. GET /v1/hub/surface reports them, the contract must carry the
// same set, and this function refuses a name that is not in it: a consumer
// reads the stream without guessing, and an event that exists only in the code
// (or only in the contract) fails the contract test instead of surprising
// somebody. Both directions had drifted before this list existed.
const SURFACE_EVENTS = [
  'turn.admitted',
  'turn.running',
  'message.delta',
  'message.completed',
  'tool.started',
  'tool.ended',
  'approval.requested',
  'approval.resolved',
  'question.requested',
  'question.answered',
  'question.cancelled',
  'plan.changed',
  'turn.ended',
  'session.notification',
  'session.stats',
  'session.compacting',
  'session.compacted',
  'session.renamed',
  // hub-level, not session-level: the set of installed plugins changed (an install, a
  // removal, or a runtime's prepare finishing). A client that shows plugins subscribes
  // once and re-reads on this, instead of polling.
  'hub.plugins.changed',
];
const SURFACE_EVENT_SET = new Set(SURFACE_EVENTS);

// Clients subscribed to the hub-level event stream (GET /v1/hub/events).
const hubEventClients = new Set();
function emitHubEvent(event, data) {
  if (!SURFACE_EVENT_SET.has(event)) {
    throw new Error(`undeclared SSE event '${event}': add it to SURFACE_EVENTS and to contract/v1.json`);
  }
  const id = `hub-${++eventSeq}`;
  for (const res of hubEventClients) {
    try { sseWrite(res, { id, event, data }); } catch { hubEventClients.delete(res); }
  }
}

// The hub's plugin view changed. The event names WHICH plugin and WHAT state it is now
// in, so a subscriber shows the change directly. Before this it carried only a loose
// `reason` ('installed' | 'runtime' | 'install' | 'idle' | ...), which is the same word
// for a step and for its completion and names no plugin - so every client had to re-read
// and re-derive what was happening, which is how several disagreeing copies of one fact
// grew. One event, one id, one state.
function announcePluginsChanged(id) {
  const row = pluginList().find((r) => r.id === id);
  // A plugin that is no longer in the list is ABSENT: that is the one state a client can
  // draw for an id the hub changes and then does not list. Never null - null is not a
  // state, and a client that tried to draw it would have to invent one.
  try {
    emitHubEvent('hub.plugins.changed', {
      id: row ? row.id : id,
      state: row ? row.state : 'absent',
      at: new Date().toISOString(),
    });
  } catch { /* an event nobody can receive is not a reason to fail the change */ }
}

function emitSessionEvent(sid, event, data) {
  if (!SURFACE_EVENT_SET.has(event)) {
    throw new Error(`undeclared SSE event '${event}': add it to SURFACE_EVENTS in server.mjs and to contract/v1.json`);
  }
  const conn = connFor(sid);
  if (!conn || !conn.onEvent) return;
  const id = `${sid}-${++eventSeq}`;
  conn.onEvent({ id, event, data });
}

// The adapter process backing a session (one process per session).
function connFor(sid) {
  return adapters.get(sid) || null;
}
function sessOf(sid) { return sessions.get(sid); }

// --- SSE plumbing ------------------------------------------------------------
function sseWrite(res, { id, event, data }) {
  if (res.writableEnded) return;
  if (id) res.write(`id: ${id}\n`);
  res.write(`event: ${event}\ndata: ${JSON.stringify(data)}\n\n`);
}

// --- approvals ----------------------------------------------------------------
function handleApprovalRequest(conn, msg) {
  const params = msg.params || {};
  // A shared adapter carries several sessions on one process, so the approval
  // must name its session; a per-session adapter's params omit it.
  const sid = params.sid || conn.sid;
  const s = sessions.get(sid);
  const aid = `appr-${crypto.randomUUID()}`;
  const record = { id: aid, sid, tool: typeof params.tool === 'string' ? params.tool : params.kind || 'confirm', args: params.args && typeof params.args === 'object' && !Array.isArray(params.args) ? params.args : { detail: params.detail }, options: Array.isArray(params.options) ? params.options : null, state: 'pending', requestedAt: new Date().toISOString(), expiresAt: APPROVAL_TIMEOUT > 0 ? new Date(Date.now() + APPROVAL_TIMEOUT).toISOString() : null, adapterRequestId: msg.id, conn, timer: null };
  const failClosed = (approved, choice) => {
    if (record.state !== 'pending') return;
    conn.proc.stdin.write(JSON.stringify({ jsonrpc: '2.0', id: msg.id, result: { approved, reason: approved ? 'allowed' : 'timeout', ...(choice !== undefined ? { choice } : {}) } }) + '\n');
    if (approved) record.state = 'allowed';
    else record.state = 'expired';
    record.resolvedAt = new Date().toISOString(); record.resolutionSource = 'timeout';
    if (s) s.activeTurn = { state: 'running', ended: null, cause: null, partialPersisted: s.activeTurn.partialPersisted, partialItems: 0 };
    updateHumanWait(conn, sid);
    emitSessionEvent(sid, 'approval.resolved', { approvalId: aid, approved, state: record.state });
  };
  record.timer = APPROVAL_TIMEOUT > 0 ? setTimeout(() => failClosed(false), APPROVAL_TIMEOUT) : null;
  approvals.set(aid, record);
  updateHumanWait(conn, sid);
  if (s) s.activeTurn = { state: 'awaiting_approval', ended: null, cause: null, partialPersisted: s.activeTurn.partialPersisted, partialItems: 0 };
  emitSessionEvent(sid, 'approval.requested', { approvalId: aid, tool: record.tool, args: record.args, options: record.options, state: 'pending' });
}

function decideApproval(aid, decision, reason, sid) {
  const r = approvals.get(aid);
  if (!r || r.sid !== sid) return null;
  if (r.state !== 'pending') throw new Error('approval is no longer pending');
  const accepted = r.options && r.options.length ? r.options : ['allow', 'deny', 'always'];
  if (!accepted.includes(decision)) throw new Error('decision must be one of the offered approval choices');
  clearTimeout(r.timer);
  // Two answer shapes: a binary allow/deny (confirm dialogs) or a choice from
  // the request's options (select dialogs, e.g. Allow Once / Allow Always /
  // Reject). A choice is approved unless it is the rejection-ish option; the
  // adapter decides the exact mapping via `approved`.
  const choice = r.options && r.options.includes(decision) ? decision : undefined;
  const approved = choice !== undefined
    ? !/reject|deny|no\b/i.test(choice)
    : (decision === 'allow' || decision === 'always');
  // Reason: the user's rejection text, when the harness asks for one (codex
  // denied.rejection; pi/jouzu Reject with Reason). Carried to the adapter as
  // `comment` — `reason` is already the fixed status string below.
  const comment = (!approved && typeof reason === 'string' && reason.length) ? reason : undefined;
  r.reason = comment !== undefined ? comment : null;
  r.state = approved ? 'allowed' : 'denied';
  r.resolvedAt = new Date().toISOString(); r.resolutionSource = 'client'; r.decision = decision;
  r.conn.proc.stdin.write(JSON.stringify({ jsonrpc: '2.0', id: r.adapterRequestId, result: { approved, reason: approved ? 'allowed' : 'denied', ...(comment !== undefined ? { comment } : {}), ...(choice !== undefined ? { choice } : {}) } }) + '\n');
  const s = sessions.get(r.sid);
  if (s) {
    s.activeTurn = { state: 'running', ended: null, cause: null, partialPersisted: s.activeTurn.partialPersisted, partialItems: 0 };
    // A denied tool is not a terminal agent turn. The harness settles the turn.
  }
  updateHumanWait(r.conn, r.sid);
  emitSessionEvent(r.sid, 'approval.resolved', { approvalId: aid, approved, state: r.state, ...(r.reason ? { reason: r.reason } : {}) });

  return r;
}

// --- questions ----------------------------------------------------------------
// A question is the harness asking the user to decide something. It is NOT an
// approval: an approval permits one action (allow/deny), a question asks for an
// answer the harness reads back (possibly several options, possibly detail).
// The two travel on separate methods and never share a shape.
function handleQuestionRequest(conn, msg) {
  const params = msg.params || {};
  const sid = params.sid || conn.sid;
  const s = sessions.get(sid);
  const qid = `q-${crypto.randomUUID()}`;
  const asked = Array.isArray(params.questions) ? params.questions : [];
  const record = {
    id: qid,
    sid,
    questions: asked.map((q) => ({
      id: String(q.id),
      header: q.header != null ? q.header : null,
      question: String(q.question != null ? q.question : ''),
      detail: q.detail != null ? q.detail : null,
      options: Array.isArray(q.options) ? q.options.map((o) => ({ label: String(o.label), description: o.description != null ? o.description : null })) : null,
      multiSelect: q.multiSelect === true,
      intent: q.intent && typeof q.intent === 'object' ? { kind: String(q.intent.kind), approve: q.intent.approve != null ? String(q.intent.approve) : null } : null,
    })),
    answers: null,
    state: 'pending',
    requestedAt: new Date().toISOString(),
    adapterRequestId: msg.id,
    conn,
    timer: null,
  };
  // A question nobody answers must not wedge the harness forever. Cancelling is
  // the honest outcome: the harness unblocks and the model is told no answer
  // came, rather than being fed an invented one.
  const cancelQuestion = () => {
    if (record.state !== 'pending') return;
    conn.proc.stdin.write(JSON.stringify({ jsonrpc: '2.0', id: msg.id, result: { cancelled: true } }) + '\n');
    record.state = 'cancelled';
    if (s) s.activeTurn = { state: 'running', ended: null, cause: null, partialPersisted: s.activeTurn.partialPersisted, partialItems: 0 };
    updateHumanWait(conn, sid);
    emitSessionEvent(sid, 'question.cancelled', { questionId: qid, state: record.state });
  };
  record.timer = QUESTION_TIMEOUT > 0 ? setTimeout(cancelQuestion, QUESTION_TIMEOUT) : null;
  questions.set(qid, record);
  updateHumanWait(conn, sid);
  if (s) s.activeTurn = { state: 'awaiting_question', ended: null, cause: null, partialPersisted: s.activeTurn.partialPersisted, partialItems: 0 };
  emitSessionEvent(sid, 'question.requested', { questionId: qid, questions: record.questions, state: 'pending' });
}

async function answerQuestion(qid, answers) {
  const r = questions.get(qid);
  if (!r) return null;
  clearTimeout(r.timer);
  // Normalise to one entry per asked question. A skipped item stays as
  // { id, selected: [] } so the shape is stable whatever the UI could show.
  const byId = new Map((Array.isArray(answers) ? answers : []).map((a) => [String(a && a.id), a]));
  const normalised = r.questions.map((q) => {
    const a = byId.get(q.id);
    const selected = a && Array.isArray(a.selected) ? a.selected.map(String) : [];
    const custom = a && typeof a.custom === 'string' && a.custom.length ? a.custom : undefined;
    return { id: q.id, selected, ...(custom !== undefined ? { custom } : {}) };
  });
  r.answers = normalised;
  r.state = 'answered';
  r.conn.proc.stdin.write(JSON.stringify({ jsonrpc: '2.0', id: r.adapterRequestId, result: { answers: normalised } }) + '\n');
  const s = sessions.get(r.sid);
  if (s) s.activeTurn = { state: 'running', ended: null, cause: null, partialPersisted: s.activeTurn.partialPersisted, partialItems: 0 };
  updateHumanWait(r.conn, r.sid);
  emitSessionEvent(r.sid, 'question.answered', { questionId: qid, answers: normalised, state: r.state });
  return r;
}

// --- the surface ---------------------------------------------------------------
//
// One row per (method, path). This table IS the routing: route() resolves a
// request by matching a row here, and GET /v1/hub/surface reports the same
// table. The routes this process answers and the routes the contract promises
// are therefore one object, and the hub refuses to start when they disagree in
// either direction — a route only here is one nobody can find, a route only in
// the contract is a promise nobody keeps. Both had happened before this table
// existed; that is what it is for, and why the surface is data instead of a
// chain of string comparisons that nothing can compare anything against.
//
//   path  literal segments, {placeholders}, and a trailing {name...} that
//         captures the remaining segments (the skill-file route needs whole
//         file paths, slashes included).
//   auth  false ONLY for discovery — the hub's one unauthenticated route. The
//         hub binds 127.0.0.1 and the hub is not a security boundary (S-1), so a
//         consumer may ask what harnesses exist before it holds a token.
//         Everything else needs the endpoint token.
//
// Matching is first-row-wins and exact in segment count, so a literal segment
// must be declared before a placeholder that could swallow it (see providers).
const ROUTES = [
  // discovery: which harnesses exist. Answers without a token.
  { method: 'GET', path: '/v1/harnesses', auth: false, handler: ({ res }) => {
    const harnesses = listHarnesses();
    if (harnesses.length) return json(res, 200, { harnesses, next_cursor: null });
    // Nothing installed is a legitimate answer, but it is also the state a client
    // cannot diagnose: say where the hub looked, so a UI can show that instead of
    // "no engines" with no reason.
    return json(res, 200, {
      harnesses, next_cursor: null,
      note: `no harness plugins found. searched: ${pluginRoots().join(' | ')} — install one (POST /v1/hub/plugins {source:{url}}) or point AGENT_HUB_PLUGINS_DIR at a directory of plugin directories.`,
    });
  } },

  // Leftover plugin state under the hub's own root: named, sized, never removed
  // here. A client can offer to clean it; nothing else could even see it.
  { method: 'GET', path: '/v1/hub/orphans', handler: ({ res }) =>
    json(res, 200, { orphans: orphanPluginDirs() }) },

  // Clearing leftover plugin state is an explicit action, never a side effect of
  // listing it. Only a directory the hub's own root holds, without a manifest, is
  // removable here; a real plugin is removed by the plugin route, and a
  // deployment's tree is never touched.
  { method: 'DELETE', path: '/v1/hub/orphans/{id}', handler: async ({ res, params }) => {
    const id = String(params.id || '');
    if (!PLUGIN_ID_RE.test(id)) return fail(res, 400, 'validation_failed', `not a plugin id: ${JSON.stringify(id)}`);
    const dir = path.join(HUB_PLUGINS_DIR, id);
    if (!fs.existsSync(dir)) return fail(res, 404, 'not_found', `no directory at ${dir}`);
    if (fs.existsSync(path.join(dir, 'manifest.json'))) {
      return fail(res, 409, 'conflict', `plugin '${id}' has a manifest: remove it through DELETE /v1/hub/plugins/{id}`);
    }
    const owner = pluginRootOf(id);
    if (owner && owner !== HUB_PLUGINS_DIR) return fail(res, 409, 'conflict', `'${id}' belongs to ${owner}, which this hub does not own`);
    const held = await rmRetrying(dir);
    if (held) return fail(res, 502, 'plugin_dir_busy', `the leftover at ${dir} could not be removed: ${held.message} — close anything using that harness and retry`);
    return json(res, 200, { ok: true, removed: dir });
  } },

  // the surface itself, as data (this table, the event names, the contract)
  { method: 'GET', path: '/v1/hub/surface', handler: ({ res }) =>
    json(res, 200, surfaceValue()) },

  // the generated OpenAPI document, byte for byte as scripts/emit-openapi.mjs
  // wrote it — the same bytes whose sha256 /v1/hub/surface reports
  { method: 'GET', path: '/v1/hub/openapi.json', handler: ({ res }) => {
    res.writeHead(200, { 'content-type': 'application/json; charset=utf-8' });
    res.end(JSON.stringify(OPENAPI.doc, null, 2) + String.fromCharCode(10));
  } },

  // ---- harness catalogue: what a harness says it can do --------------------
  { method: 'POST', path: '/v1/harnesses/{id}/enable', handler: ({ res, params }) =>
    harnessToggle(params.id, true, res, false) },
  { method: 'POST', path: '/v1/harnesses/{id}/disable', handler: ({ res, params }) =>
    harnessToggle(params.id, false, res, false) },
  { method: 'GET', path: '/v1/harnesses/{id}/presets', handler: ({ res, params }) =>
    withHarness(params.id, res, () => listPresets(params.id)
      .then((r) => json(res, 200, r)).catch((e) => fail(res, 502, 'adapter_unreachable', e.message))) },
  { method: 'GET', path: '/v1/harnesses/{id}/models', handler: ({ res, params }) =>
    withHarness(params.id, res, () => listModels(params.id)
      .then((r) => json(res, 200, r)).catch((e) => fail(res, 502, 'adapter_unreachable', e.message))) },
  // ---- harness-private connections ------------------------------------------
  // What this harness itself lets a person connect: its own providers, its own
  // fields, its own auth kinds and its own stored credential. Declared before
  // the /connections route so the literal segment wins for /schema.
  { method: 'GET', path: '/v1/harnesses/{id}/connections/schema', handler: ({ res, params }) =>
    withHarness(params.id, res, () => privateConnectionRpc(params.id, 'connections/schema', {})
      .then((r) => json(res, 200, r)).catch((e) => connectionFailure(res, e))) },
  { method: 'GET', path: '/v1/harnesses/{id}/connections', handler: ({ res, params }) =>
    withHarness(params.id, res, () => privateConnectionRpc(params.id, 'connections/list', {})
      .then((r) => json(res, 200, r)).catch((e) => connectionFailure(res, e))) },
  { method: 'POST', path: '/v1/harnesses/{id}/connections/validate', handler: ({ res, params, body }) =>
    withHarness(params.id, res, () => body()
      .then((b) => privateConnectionRpc(params.id, 'connections/validate', { draft: b.draft !== undefined ? b.draft : b }))
      .then((r) => json(res, 200, r)).catch((e) => connectionFailure(res, e))) },
  { method: 'POST', path: '/v1/harnesses/{id}/connections', handler: ({ res, params, body }) =>
    withHarness(params.id, res, () => body()
      .then((b) => privateConnectionRpc(params.id, 'connections/save', {
        ...(b.id !== undefined ? { id: b.id } : {}),
        providerId: b.providerId,
        ...(b.label !== undefined ? { label: b.label } : {}),
        fields: b.fields && typeof b.fields === 'object' && !Array.isArray(b.fields) ? b.fields : {},
        ...(b.secretPolicy !== undefined ? { secretPolicy: b.secretPolicy } : {}),
        ...(b.expectedRevision !== undefined ? { expectedRevision: b.expectedRevision } : {}),
      }))
      .then((r) => json(res, 200, r)).catch((e) => connectionFailure(res, e))) },
  { method: 'DELETE', path: '/v1/harnesses/{id}/connections/{cid}', handler: ({ res, params, body }) =>
    withHarness(params.id, res, () => body()
      .then((b) => privateConnectionRpc(params.id, 'connections/delete', { id: params.cid, ...(b && b.expectedRevision !== undefined ? { expectedRevision: b.expectedRevision } : {}) }))
      .then(() => json(res, 200, { ok: true, id: params.cid })).catch((e) => connectionFailure(res, e))) },
  // Auth operations ride the same capability. A harness that implements none
  // answers with its own machine code and that answer is what a caller sees —
  // the hub never fabricates an authorization step or a token.
  { method: 'POST', path: '/v1/harnesses/{id}/auth', handler: ({ res, params, body }) =>
    withHarness(params.id, res, () => body()
      .then((b) => privateConnectionRpc(params.id, 'auth/start', { providerId: b.providerId }))
      .then((r) => json(res, 200, r)).catch((e) => connectionFailure(res, e))) },
  { method: 'GET', path: '/v1/harnesses/{id}/auth/{op}', handler: ({ res, params }) =>
    withHarness(params.id, res, () => privateConnectionRpc(params.id, 'auth/status', { operationId: params.op })
      .then((r) => json(res, 200, r)).catch((e) => connectionFailure(res, e))) },
  { method: 'POST', path: '/v1/harnesses/{id}/auth/{op}/cancel', handler: ({ res, params }) =>
    withHarness(params.id, res, () => privateConnectionRpc(params.id, 'auth/cancel', { operationId: params.op })
      .then((r) => json(res, 200, r)).catch((e) => connectionFailure(res, e))) },
  { method: 'GET', path: '/v1/harnesses/{id}/tools', handler: ({ res, params, url }) =>
    withHarness(params.id, res, () => listTools(params.id, url.searchParams.get('mode') || undefined)
      .then((r) => json(res, 200, r)).catch((e) => fail(res, 502, 'adapter_unreachable', e.message))) },

  // ---- the hub's harness registry ------------------------------------------
  // What each harness ships, what the hub has installed for it, and whether it
  // is enabled. A session's material (extensions, skills, connections) is
  // assembled from here, so an extension is added to a harness in the registry,
  // not in an adapter.
  { method: 'GET', path: '/v1/hub/harnesses', handler: ({ res }) =>
    json(res, 200, {
      harnesses: reconcileHarnesses().filter((r) => !r.missing).map((r) => ({ ...harnessValue(r), availableExtensions: availableExtensions(r.id) })),
      // kept as the union of ids any installed plugin ships: the field means the same
      // thing it always did (what the hub could install from), while each harness row
      // above is authoritative for that harness — an id two plugins both ship is two
      // different directories, and only the harness's own list says which one is meant.
      availableExtensions: [...new Set(reconcileHarnesses().filter((r) => !r.missing).flatMap((r) => availableExtensions(r.id)))].sort(),
    }) },
  // ---- plugins: what the hub can see, what it installed, and preparing ---------
  // A plugin is a directory with a manifest. Two roots: the one a deployment gave
  // this hub (read-only to the hub) and the hub's own inside its data dir. The route
  // says which is which, because a client that installs has to know where it landed.
  // The hub-level event stream: whatever the plugin set does, once, to every client that
  // shows plugins. One subscription replaces polling for an install or a prepare.
  { method: 'GET', path: '/v1/hub/events', handler: ({ res }) => {
    res.writeHead(200, { 'content-type': 'text/event-stream', 'cache-control': 'no-cache', connection: 'keep-alive' });
    hubEventClients.add(res);
    // The contract says this stream announces on subscribe. It stays, and it stays a
    // hub.plugins.changed: the subscribe frame carries no plugin id (there is no single
    // plugin it is about), so a client reads that as "re-read", exactly as before. Do not
    // make a general stream carry one feature's shape - {id, state} belongs to a real
    // change, not to the handshake.
    sseWrite(res, { event: 'hub.plugins.changed', data: { at: new Date().toISOString() } });
    const drop = () => hubEventClients.delete(res);
    res.on('close', drop);
    res.on('error', drop);
  } },
  { method: 'GET', path: '/v1/hub/plugins', handler: ({ res }) =>
    json(res, 200, { plugins: pluginList(), roots: { given: GIVEN_PLUGINS_DIR, hub: HUB_PLUGINS_DIR, searched: pluginRoots() } }) },
  { method: 'POST', path: '/v1/hub/plugins', handler: ({ res, body }) =>
    body().then(async (b) => await installPlugin(b, res)).catch((e) => failError(res, e)) },
  { method: 'GET', path: '/v1/hub/catalog', handler: ({ res }) =>
    json(res, 200, catalogValue()) },
  { method: 'GET', path: '/v1/hub/plugins/{id}/icon/{variant}', handler: ({ res, params }) =>
    serveHarnessIcon(params.id, params.variant, res).catch((e) => failError(res, e)) },
  { method: 'POST', path: '/v1/hub/registry/refresh', handler: ({ res }) =>
    refreshRegistry().then((r) => json(res, 200, r)).catch((e) => {
      const status = e.code === 'registry_url_missing' ? 409 : 502;
      return fail(res, status, e.code || 'registry_fetch_failed', e.message);
    }) },
  { method: 'DELETE', path: '/v1/hub/plugins/{id}', handler: ({ res, params }) =>
    removePlugin(params.id, res).catch((e) => failError(res, e)) },
  { method: 'POST', path: '/v1/hub/plugins/{id}/prepare', handler: ({ res, params }) => {
    if (!manifestOf(params.id)) return fail(res, 404, 'harness_not_found', `no plugin '${params.id}'`);
    const caps = (manifestOf(params.id) || {}).capabilities || [];
    if (!caps.includes('runtime')) {
      return fail(res, 501, 'unsupported', `plugin '${params.id}' does not declare the runtime capability: it brings its own runtime, or it materialises nothing`);
    }
    ensureRuntimeFor(params.id)
      .then((r) => json(res, 200, { harnessId: params.id, runtimeReady: runtimeReady(params.id), ...r }))
      .catch((e) => failError(res, e));
  } },
  { method: 'PATCH', path: '/v1/hub/harnesses/{id}', handler: ({ res, params, body }) =>
    body().then((b) => updateHarness(params.id, b, res))
      .catch((e) => fail(res, 400, 'validation_failed', e.message)) },
  { method: 'POST', path: '/v1/hub/harnesses/{id}/enable', handler: ({ res, params }) =>
    harnessToggle(params.id, true, res, true) },
  { method: 'POST', path: '/v1/hub/harnesses/{id}/disable', handler: ({ res, params }) =>
    harnessToggle(params.id, false, res, true) },

  { method: 'GET', path: '/v1/hub/provider-types', handler: ({ res }) =>
    json(res, 200, providerTypes()) },

  // ---- providers: one provider is one file ---------------------------------
  { method: 'GET', path: '/v1/hub/providers', handler: ({ res }) =>
    json(res, 200, { providers: loadProviders().map((r) => providerValueFree(r)), broken: brokenProviderFiles() }) },
  { method: 'POST', path: '/v1/hub/providers', handler: ({ res, body }) =>
    body().then((b) => createProvider(b, res)).catch((e) => fail(res, 400, 'validation_failed', e.message)) },
  // API 1: the models of the providers the HUB manages. Distinct from API 2
  // (/v1/harnesses/{id}/models), which is a harness's own catalog; a caller asks
  // both and has the whole picture. Declared before /{id}: 'models' is also a
  // legal provider id, and the literal route wins.
  { method: 'GET', path: '/v1/hub/providers/models/list', handler: ({ res }) =>
    listManagedProviderModels()
      .then((r) => json(res, 200, r))
      .catch((e) => fail(res, 502, 'provider_catalog_failed', e.message)) },
  { method: 'GET', path: '/v1/hub/providers/{id}', handler: ({ res, params }) => {
    // Read one by id: a caller holding an id should not have to list and filter.
    const row = loadProviders().find((p) => p.id === params.id);
    if (!row) return fail(res, 404, 'provider_not_found', 'no such provider');
    return json(res, 200, { provider: providerValueFree(row) });
  } },
  { method: 'PATCH', path: '/v1/hub/providers/{id}', handler: ({ res, params, body }) =>
    body().then((b) => updateProvider(params.id, b, res)).catch((e) => fail(res, 400, 'validation_failed', e.message)) },
  { method: 'DELETE', path: '/v1/hub/providers/{id}', handler: ({ res, params }) =>
    deleteProvider(params.id, res) },
  { method: 'POST', path: '/v1/hub/providers/{id}/logout', handler: ({ res, params }) =>
    logoutProvider(params.id, res) },
  // Hub-level authorization: the flow runs in THIS process (the provider plugin
  // owns it), the credential lands in the hub secret store, and the existing
  // credentials/grant path then reaches every compatible harness. The caller
  // gets declared steps to render; it hosts nothing.
  { method: 'POST', path: '/v1/hub/providers/{id}/auth', handler: ({ res, params, body }) =>
    body().then(() => startProviderAuth(params.id, res)).catch((e) => failError(res, e)) },
  { method: 'GET', path: '/v1/hub/providers/{id}/auth/{op}', handler: ({ res, params }) =>
    providerAuthStatus(params.id, params.op, res) },
  { method: 'POST', path: '/v1/hub/providers/{id}/auth/{op}/cancel', handler: ({ res, params }) =>
    providerAuthCancel(params.id, params.op, res) },
  { method: 'GET', path: '/v1/hub/providers/{id}/models', handler: ({ res, params }) => {
    const row = loadProviders().find((p) => p.id === params.id);
    if (!row) return fail(res, 404, 'provider_not_found', 'no such provider');
    return json(res, 200, catalogView(row));
  } },
  { method: 'PATCH', path: '/v1/hub/providers/{id}/models', handler: ({ res, params, body }) =>
    body().then((b) => {
      if (rejectUnknownFields(res, b, ['enabledModelIds'], 'PATCH /v1/hub/providers/{id}/models')) return;
      return selectProviderModels(params.id, b.enabledModelIds, res);
    })
      .catch((e) => fail(res, 400, 'validation_failed', e.message)) },
  { method: 'POST', path: '/v1/hub/providers/{id}/models/refresh', handler: ({ res, params }) =>
    refreshProviderCatalog(params.id).then((r) => json(res, 200, r)).catch((e) => {
      const status = e.code === 'unsupported' ? 501 : e.code === 'provider_not_found' ? 404 : e.code === 'revision_conflict' ? 409 : e.code === 'provider_unauthorized' || e.code === 'validation_failed' ? 400 : 502;
      const row = loadProviders().find((p) => p.id === params.id);
      return json(res, status, { ...errorBody(e.code || 'provider_catalog_failed', e.code ? e.message : 'catalog unavailable'), catalog: row ? catalogView(row) : null });
    }) },

  { method: 'GET', path: '/v1/sessions/{id}/resources', handler: (c) =>
    withSession(c.params.id, c.res, () => {
      const s = sessions.get(c.params.id);
      return json(c.res, 200, { resources: listSkillResources(SKILLS_DIR, harnessRow(s.harnessId)?.skills) });
    }) },
  { method: 'POST', path: '/v1/sessions/{id}/resources/read', handler: (c) =>
    withSession(c.params.id, c.res, () => c.body().then(b => {
      if (rejectUnknownFields(c.res, b, ['uri'], 'POST /v1/sessions/{id}/resources/read')) return;
      const s = sessions.get(c.params.id);
      return json(c.res, 200, readSkillResource(SKILLS_DIR, harnessRow(s.harnessId)?.skills, b.uri));
    }).catch(e => failError(c.res, e))) },
  // ---- skills: a directory per skill, round-tripped byte for byte ----------
  { method: 'GET', path: '/v1/hub/skills', handler: ({ res }) =>
    json(res, 200, { skills: listSkills() }) },
  { method: 'DELETE', path: '/v1/hub/skills/{id}', handler: ({ res, params }) => {
    let dir;
    try { dir = skillDir(params.id); } catch (e) { return fail(res, 400, 'validation_failed', e.message); }
    fs.rmSync(dir, { recursive: true, force: true });
    return json(res, 200, { ok: true, id: params.id });
  } },
  { method: 'GET', path: '/v1/hub/skills/{id}/files/{file...}', handler: ({ res, params }) => {
    const rel = params.file.join('/');
    let target;
    try { target = skillPath(params.id, rel); } catch (e) { return fail(res, 400, 'validation_failed', e.message); }
    if (!fs.existsSync(target)) return fail(res, 404, 'not_found', 'no such skill file');
    return json(res, 200, { skillId: params.id, path: rel, content: fs.readFileSync(target, 'utf8') });
  } },
  { method: 'PUT', path: '/v1/hub/skills/{id}/files/{file...}', handler: ({ res, params, body }) =>
    body().then((b) => {
      const rel = params.file.join('/');
      let target;
      try { target = skillPath(params.id, rel); } catch (e) { return fail(res, 400, 'validation_failed', e.message); }
      fs.mkdirSync(path.dirname(target), { recursive: true });
      fs.writeFileSync(target, b.content ?? '', 'utf8');
      return json(res, 200, { skillId: params.id, path: rel, bytes: Buffer.byteLength(b.content ?? '', 'utf8') });
    }).catch((e) => fail(res, 400, 'validation_failed', e.message)) },

  // ---- connections (S4, connection-only model) -----------------------------
  { method: 'GET', path: '/v1/hub/connections', handler: ({ res }) =>
    json(res, 200, { connections: loadConnections().map(connectionValueFree) }) },
  { method: 'POST', path: '/v1/hub/connections', handler: ({ res, body }) =>
    body().then((b) => createConnection(b, res)).catch((e) => fail(res, 400, 'validation_failed', e.message)) },
  { method: 'PATCH', path: '/v1/hub/connections/{id}', handler: ({ res, params, body }) =>
    body().then((b) => updateConnection(params.id, b, res)).catch((e) => fail(res, 400, 'validation_failed', e.message)) },
  { method: 'DELETE', path: '/v1/hub/connections/{id}', handler: ({ res, params }) =>
    deleteConnection(params.id, res) },

  // ---- the hub itself ------------------------------------------------------
  { method: 'GET', path: '/v1/hub/status', handler: ({ res }) =>
    json(res, 200, {
      pid: process.pid,
      port: server.address() ? server.address().port : null,
      startedAt: STARTED_AT,
      // The hub's own product version (its package.json version). A client shows this to
      // say WHICH hub it is talking to. Distinct from the protocol version and buildId.
      version: HUB_VERSION,
      buildId: BUILD_ID,
      protocol: PROTOCOL,
      contract: CONTRACT,
      // What the hub manages, in the same value-free shape the list routes use:
      // a provider shows tokenConfigured, never the token; a connection shows its
      // materialization fields, never the credential (that lives in the OS store
      // and reaches a harness as an env var only).
      harnesses: listHarnesses(),
      providers: loadProviders().map(providerValueFree),
      connections: loadConnections().map(connectionValueFree),
      sessionCount: sessions.size,
      activeTurns: [...sessions.values()].filter(turnIsOpen).length,
    }) },
  { method: 'POST', path: '/v1/hub/shutdown', handler: ({ res }) => {
    json(res, 200, { ok: true });
    fs.rmSync(ENDPOINT, { force: true });
    return res.on('finish', () => process.exit(0));
  } },

  // ---- sessions ------------------------------------------------------------
  { method: 'POST', path: '/v1/sessions', handler: ({ res, body }) =>
    body().then((b) => createSession(b, res)).catch((e) => failError(res, e)) },
  { method: 'GET', path: '/v1/sessions', handler: ({ res, url }) => {
    // ACP session/list: a deleted session is not listed, and neither is a
    // closed one — not appearing is what closing means, as opposed to deleting.
    // ?includeClosed=true brings them back so a caller can still reopen one.
    const includeClosed = url.searchParams.get('includeClosed') === 'true';
    const list = [...sessions.values()]
      .filter((s) => s.deleted !== true)
      .filter((s) => includeClosed || s.status !== 'closed')
      .map((s) => sessionValue(s));
    return json(res, 200, { sessions: list, next_cursor: null });
  } },
  { method: 'GET', path: '/v1/sessions/{id}', handler: (c) =>
    withSession(c.params.id, c.res, (s) => json(c.res, 200, { session: sessionValue(s) })) },
  // ACP session/delete: remove from session/list. Not a wipe.
  { method: 'DELETE', path: '/v1/sessions/{id}', handler: (c) =>
    withSession(c.params.id, c.res, () => deleteSession(c.params.id, c.res)) },
  { method: 'PATCH', path: '/v1/sessions/{id}', handler: (c) =>
    withSession(c.params.id, c.res, () => c.body()
      .then((b) => switchModel(c.params.id, b, c.res))
      .catch((e) => failError(c.res, e))) },
  { method: 'GET', path: '/v1/sessions/{id}/skills', handler: (c) =>
    withSession(c.params.id, c.res, (s) => readHarnessSkills(c.params.id, s, c.res)) },
  { method: 'GET', path: '/v1/sessions/{id}/stats', handler: (c) =>
    withSession(c.params.id, c.res, (s) => readStats(c.params.id, s, c.res)) },
  { method: 'POST', path: '/v1/sessions/{id}/fork', handler: (c) =>
    withSession(c.params.id, c.res, () => c.body()
      .then((b) => forkSession(c.params.id, b, c.res))
      .catch((e) => failError(c.res, e))) },
  { method: 'POST', path: '/v1/sessions/{id}/compact', handler: (c) =>
    withSession(c.params.id, c.res, () => c.body()
      .then((b) => compactSession(c.params.id, b, c.res))
      .catch((e) => failError(c.res, e))) },
  { method: 'POST', path: '/v1/sessions/{id}/close', handler: (c) =>
    withSession(c.params.id, c.res, () => closeSession(c.params.id, c.res)) },
  { method: 'POST', path: '/v1/sessions/{id}/reopen', handler: (c) =>
    withSession(c.params.id, c.res, () => reopenSession(c.params.id, c.res)) },
  { method: 'GET', path: '/v1/sessions/{id}/turns', handler: (c) =>
    withSession(c.params.id, c.res, () => listTurns(c.params.id, c.res)) },
  { method: 'GET', path: '/v1/sessions/{id}/messages', handler: (c) =>
    withSession(c.params.id, c.res, () => readMessages(c.params.id, c.res, c.url.searchParams)) },
  { method: 'POST', path: '/v1/sessions/{id}/turns', handler: (c) =>
    withSession(c.params.id, c.res, () => c.body()
      .then((b) => sendTurn(c.params.id, b, c.req, c.res))
      .catch((e) => failError(c.res, e))) },
  { method: 'POST', path: '/v1/sessions/{id}/cancel', handler: (c) =>
    withSession(c.params.id, c.res, () => cancelTurn(c.params.id, c.res)) },
  { method: 'POST', path: '/v1/sessions/{id}/repair', handler: (c) =>
    withSession(c.params.id, c.res, () => c.body()
      .then((b) => repairSession(c.params.id, b, c.res))
      .catch((e) => failError(c.res, e))) },
  { method: 'GET', path: '/v1/sessions/{id}/approvals', handler: (c) =>
    withSession(c.params.id, c.res, () =>
      json(c.res, 200, { approvals: [...approvals.values()].filter((a) => a.sid === c.params.id).map(({ conn: _c, timer: _t, adapterRequestId: _r, ...a }) => a) })) },
  { method: 'POST', path: '/v1/sessions/{id}/approvals/{aid}', handler: (c) =>
    withSession(c.params.id, c.res, () => c.body().then((b) => {
      const r = decideApproval(c.params.aid, b.decision, b.reason, c.params.id);
      if (!r) return fail(c.res, 404, 'approval_not_found', 'no such approval request');
      return json(c.res, 200, { approval: { id: r.id, sid: r.sid, tool: r.tool, args: r.args, state: r.state } });
    }).catch((e) => fail(c.res, 400, 'validation_failed', e.message))) },
  { method: 'GET', path: '/v1/sessions/{id}/questions', handler: (c) =>
    withSession(c.params.id, c.res, () =>
      json(c.res, 200, { questions: [...questions.values()].filter((q) => q.sid === c.params.id).map(({ conn: _c, timer: _t, adapterRequestId: _r, ...q }) => q) })) },
  { method: 'POST', path: '/v1/sessions/{id}/questions/{qid}', handler: (c) =>
    withSession(c.params.id, c.res, () => c.body().then(async (b) => {
      const r = await answerQuestion(c.params.qid, b.answers);
      if (!r) return fail(c.res, 404, 'question_not_found', 'no such question request');
      return json(c.res, 200, { question: { id: r.id, sid: r.sid, questions: r.questions, answers: r.answers, state: r.state } });
    }).catch((e) => fail(c.res, 400, 'validation_failed', e.message))) },
  // honest: no artifact producer in the conformance harness yet
  { method: 'GET', path: '/v1/sessions/{id}/artifacts', handler: (c) =>
    withSession(c.params.id, c.res, () => json(c.res, 200, { artifacts: [], next_cursor: null })) },
];

// A harness-named route answers harness_not_found for a harness that does not
// exist — one check, one message, every route that takes a harness id.
function withHarness(id, res, run) {
  if (!manifestOf(id)) return fail(res, 404, 'harness_not_found', `no harness ${id}`);
  return run();
}

// The states in which a turn has been admitted and not yet finished. Named once,
// because the list was copied into eight call sites here and then copied AGAIN
// into the client, where it became a second authority that could drift from this
// one. A session is "running" by the hub's definition, not by anyone's re-reading
// of `activeTurn.state` (ADR-0001 D2/D5).
const TURN_OPEN_STATES = ['admitted', 'running', 'awaiting_approval', 'awaiting_question', 'cancelling'];
const turnIsOpen = (s) => !!s && !!s.activeTurn && TURN_OPEN_STATES.includes(s.activeTurn.state);

// The shape a session is published in. One function, so `activeTurn` and the
// derived `turnRunning` flag cannot disagree with each other or with the routes
// that decide what a session may accept.
function sessionValue(s) {
  return { ...s, activeTurn: s.activeTurn, turnRunning: turnIsOpen(s) };
}

// A session-named route answers unknown_session for an id nobody created —
// never silently creates one, and never reports it as a route that is missing.
function withSession(id, res, run) {
  const s = sessions.get(id);
  if (!s) return fail(res, 404, 'unknown_session', 'no such session; never silently created');
  return run(s);
}

// enable/disable exist twice: the catalogue route (/v1/harnesses/{id}/…, which
// answers {ok,id,status}) and the registry route (/v1/hub/harnesses/{id}/…,
// which answers with the registry row). Same effect, two shapes, so the caller
// can tell which plane it spoke to. The shape is a property of the route, so it
// is passed in rather than inferred from the path.
function harnessToggle(id, enabled, res, registryShape) {
  return withHarness(id, res, () => {
    const row = updateHarness(id, { enabled });
    if (registryShape) return json(res, 200, { harness: row });
    return json(res, 200, { ok: true, id, status: row && row.enabled ? 'enabled' : 'disabled' });
  });
}

// What this process answers, as data: the same table the router matches, the
// event names it can emit, and the contract it claims to keep. A consumer — and
// the boot self-check — compares this with the contract it holds.
function surfaceValue() {
  return {
    contract: CONTRACT,
    openapi: { file: OPENAPI.file, sha256: OPENAPI.sha256 },
    routes: ROUTES.map((r) => ({ method: r.method, path: r.path, auth: r.auth !== false })),
    events: SURFACE_EVENTS,
  };
}

// Path matching: exact in segment count, first row wins. {name} captures one
// segment; a trailing {name...} captures the rest. Values arrive decoded, once.
function matchPath(pattern, pathname) {
  const want = pattern.split('/').filter(Boolean);
  const got = pathname.split('/').filter(Boolean);
  const params = {};
  for (let i = 0; i < want.length; i += 1) {
    const w = want[i];
    if (w.endsWith('...}')) {
      params[w.slice(1, -4)] = got.slice(i).map(decodeURIComponent);
      return params;
    }
    if (i >= got.length) return null;
    if (w.startsWith('{') && w.endsWith('}')) { params[w.slice(1, -1)] = decodeURIComponent(got[i]); continue; }
    if (w !== got[i]) return null;
  }
  return got.length === want.length ? params : null;
}

function route(req, res) {
  const url = new URL(req.url, 'http://core');
  const body = () => new Promise((resolve, reject) => {
    let raw = '';
    req.on('data', (d) => { raw += d; if (raw.length > 1_000_000) req.destroy(); });
    req.on('end', () => { try { resolve(raw ? JSON.parse(raw) : {}); } catch { reject(Object.assign(new Error('bad json'), { code: 'validation_failed' })); } });
  });

  let matched = null;
  let params = null;
  for (const row of ROUTES) {
    if (row.method !== req.method) continue;
    const p = matchPath(row.path, url.pathname);
    if (p) { matched = row; params = p; break; }
  }
  // No route: the path is not part of the surface (404 not_found, whatever the
  // token). A route that exists but is not for this caller is 401 instead.
  if (!matched) return fail(res, 404, 'not_found', `no route ${req.method} ${url.pathname}`);
  if (matched.auth !== false && (req.headers.authorization || '') !== `Bearer ${token}`) {
    return fail(res, 401, 'unauthorized', 'bad token');
  }
  try {
    return matched.handler({ req, res, url, params, body });
  } catch (e) {
    return failError(res, e, 'internal_error');
  }
}

// A provider's models are declared by the caller and stored as declared: the
// core is the road the declaration travels on, not a judge of it. A declaration
// is data written by the user or by the provider plugin — name, thinking levels,
// limits, cost. Whoever wrote a value owns it: a wrong one reaches the harness
// and fails there, under a message about the thing that is actually wrong,
// instead of a verdict invented here. The only shape checked is the one that
// makes storage mean anything: an object keyed by model id.
function checkDeclarations(input) {
  if (input === null) return { value: null };
  if (typeof input !== 'object' || Array.isArray(input)) return { error: 'declarations must be an object keyed by model id' };
  const value = {};
  for (const [modelId, decl] of Object.entries(input)) {
    if (!modelId) return { error: 'declarations keys must be model ids' };
    if (decl === null) { value[modelId] = null; continue; }
    if (typeof decl !== 'object' || Array.isArray(decl)) return { error: `declaration for ${modelId} must be an object` };
    value[modelId] = { ...decl };
  }
  return { value };
}

// The thinking levels an entry states, read as data: the `thinkingLevels` list
// now, or the `reasoning.efforts` field older declarations and catalogs used.
// null = the entry says nothing about levels (which is not the same as an empty
// list: an empty list is an explicit "none").
function levelsOf(entry) {
  if (!entry || typeof entry !== 'object') return null;
  if (Array.isArray(entry.thinkingLevels)) return entry.thinkingLevels.filter((l) => typeof l === 'string' && l);
  if (Array.isArray(entry.reasoning?.efforts)) return entry.reasoning.efforts.filter((l) => typeof l === 'string' && l);
  return null;
}

// Core-provider catalogs never use a harness catalog or a session grant.
const catalogRequests = new Map();
function isSelected(row, modelId) {
  // No selection recorded = everything the provider serves is enabled; an array
  // = exactly those.
  return row.selection == null ? true : row.selection.includes(modelId);
}
function catalogView(row) {
  const c = row.catalog;
  const stale = !findProviderType(row) || !c || c.revision !== (row.endpointRevision || 0);
  const decls = row.models || {};
  // A model's entries come from two owners: the declaration (the person, or the
  // provider's registered configuration) and the provider plugin's catalog. Both
  // are data; the declaration's word wins per field, and the catalog fills what
  // it does not say. Nothing is derived from either: the levels shown are the
  // levels stated.
  return { providerId: row.id, fetchedAt: c?.fetchedAt || null, stale, ...(c?.note ? { note: c.note } : {}),
    models: (c?.models || []).map((m) => {
      const d = decls[m.id] || null;
      const merged = (key) => (d && d[key] !== undefined && d[key] !== null ? d[key] : (m[key] !== undefined && m[key] !== null ? m[key] : undefined));
      const declared = d ? levelsOf(d) : null;
      const levels = declared ?? levelsOf(m);
      const name = merged('name');
      const cost = merged('cost');
      const input = merged('input');
      const contextWindow = merged('contextWindow');
      const maxTokens = merged('maxTokens');
      // `reasoning`/`thinkingLevelsSource` were this core's own bookkeeping; they
      // are not part of a model entry any more and are not passed on.
      const { reasoning: _r, thinkingLevelsSource: _s, ...rest } = m;
      return { ...rest, providerId: row.id, provider: `hub/${row.id}`, enabled: isSelected(row, m.id), available: m.available && !stale,
        ...(levels ? { thinkingLevels: levels } : {}),
        ...(typeof name === 'string' && name ? { name } : {}),
        ...(cost ? { cost } : {}),
        ...(Array.isArray(input) && input.length ? { input } : {}),
        ...(contextWindow !== undefined ? { contextWindow } : {}),
        ...(maxTokens !== undefined ? { maxTokens } : {}) };
    }) };
}
async function refreshProviderCatalog(id) {
  if (catalogRequests.has(id)) return catalogRequests.get(id);
  const operation = fetchProviderCatalog(id);
  catalogRequests.set(id, operation);
  try { return await operation; }
  finally { if (catalogRequests.get(id) === operation) catalogRequests.delete(id); }
}
async function fetchProviderCatalog(id) {
  const error = (code, message) => Object.assign(new Error(message), { code });
  const row = loadProviders().find((r) => r.id === id);
  if (!row) throw error('provider_not_found', 'no such provider');
  const revision = row.endpointRevision || 0;
  const credential = getSecret(secretName(id));
  if (!credential) throw error('provider_unauthorized', 'provider has no stored credential');
  const fetched = await requireProviderType(row).fetchCatalog({ endpoint: row.endpoint, credential, catalogUrl: row.catalogUrl || null });
  const discovered = Array.isArray(fetched) ? fetched : fetched && Array.isArray(fetched.models) ? fetched.models : null;
  if (!discovered) throw error('provider_catalog_failed', 'the provider module returned no model list');
  const catalogNote = !Array.isArray(fetched) && fetched && typeof fetched.note === 'string' && fetched.note ? fetched.note : null;
  const usedCatalogUrl = !Array.isArray(fetched) && fetched && typeof fetched.catalogUrl === 'string' && fetched.catalogUrl ? fetched.catalogUrl : null;
  // Re-read after I/O so concurrent selection edits survive. Endpoint/secret
  // changes invalidate this response; a deleted provider must not be resurrected.
  const rows = loadProviders();
  const current = rows.find((r) => r.id === id);
  if (!current || (current.endpointRevision || 0) !== revision || (current.endpoint?.url || null) !== (row.endpoint?.url || null) || current.createdAt !== row.createdAt) {
    throw error('revision_conflict', 'provider changed during refresh; refresh again');
  }
  const previous = new Map((current.catalog?.models || []).map((m) => [m.id, m]));
  const present = new Set(discovered.map((m) => m.id));
  const models = discovered.map((m) => ({ ...m, available: true }));
  for (const old of previous.values()) if (!present.has(old.id)) models.push({ ...old, available: false });
  // Which models are selected lives on the provider, not inside the fetched
  // catalog: a refresh must not change a caller's choice, and a model the
  // provider starts serving is selected by default.
  if (Array.isArray(current.selection)) {
    const known = new Set(current.selection);
    for (const m of discovered) known.add(m.id);
    current.selection = [...known];
  }
  // Record which catalog the refresh actually read, so the provider file names
  // its official source instead of re-deriving it every time.
  if (usedCatalogUrl) current.catalogUrl = usedCatalogUrl;
  current.catalog = { revision, fetchedAt: new Date().toISOString(), models, ...(catalogNote ? { note: catalogNote } : {}) };
  persistProviders(rows);
  return catalogView(current);
}
function selectProviderModels(id, enabledModelIds, res) {
  if (!Array.isArray(enabledModelIds) || enabledModelIds.some((id) => typeof id !== 'string')) return fail(res, 400, 'validation_failed', 'enabledModelIds must be an array of model IDs');
  const rows = loadProviders();
  const row = rows.find((r) => r.id === id);
  if (!row) return fail(res, 404, 'provider_not_found', 'no such provider');
  if (!row.catalog) return fail(res, 409, 'catalog_not_loaded', 'fetch the provider catalog first');
  if (catalogView(row).stale) return fail(res, 409, 'catalog_stale', 'refresh after changing endpoint or credential');
  const known = new Set(row.catalog.models.map((m) => m.id));
  if (enabledModelIds.some((id) => !known.has(id))) return fail(res, 400, 'validation_failed', 'selection contains an unknown model ID');
  row.selection = [...new Set(enabledModelIds)];
  row.updatedAt = new Date().toISOString();
  persistProviders(rows);
  return json(res, 200, catalogView(row));
}
async function listManagedProviderModels() {
  const catalogs = []; const failures = [];
  for (const row of loadProviders()) {
    let view = catalogView(row);
    if (!row.catalog || view.stale) {
      try { view = await refreshProviderCatalog(row.id); }
      catch (e) { failures.push({ providerId: row.id, code: e.code || 'provider_catalog_failed', message: e.code ? e.message : 'catalog unavailable' }); }
    }
    catalogs.push(view);
  }
  return { models: catalogs.flatMap((c) => c.models), catalogs, failures };
}

// ---------------------------------------------------------------------------
// provider domain (S2): templates + registry + SecretStore + distribution
// ---------------------------------------------------------------------------
// One provider, one file: `providers/<id>.json` is that provider's own
// definition — where it lives, what it speaks, what its models accept, which of
// them are selected — written so a person can read and edit it, and so moving a
// provider between hubs is copying a file. The fetched catalog lives in the same
// file under `catalog`; it is derived and can be refetched, but keeping it here
// is what makes one file describe one provider completely.
const PROVIDERS_DIR = path.join(DATA_DIR, 'providers');
const LEGACY_PROVIDERS_FILE = path.join(DATA_DIR, 'providers.json');
function providerFile(id) { return path.join(PROVIDERS_DIR, `${id}.json`); }
function writeProviderFile(row) {
  const temp = providerFile(row.id) + '.' + crypto.randomUUID() + '.tmp';
  try {
    fs.writeFileSync(temp, JSON.stringify(row, null, 2) + NL, { mode: 0o600 });
    fs.renameSync(temp, providerFile(row.id));
  } finally { fs.rmSync(temp, { force: true }); }
}
// The old shape was one array holding every provider, with `url` at the top
// level and per-model `enabled` flags inside the catalog. Fold it into files
// once, then keep it out of the way rather than deleting it.
function migrateLegacyProviders() {
  if (!fs.existsSync(LEGACY_PROVIDERS_FILE)) return;
  const rows = readJson(LEGACY_PROVIDERS_FILE, []);
  fs.mkdirSync(PROVIDERS_DIR, { recursive: true });
  if (Array.isArray(rows)) {
    for (const old of rows) {
      if (!old || typeof old.id !== 'string') continue;
      const selection = Array.isArray(old.catalog?.models)
        ? old.catalog.models.filter((m) => m && m.enabled !== false).map((m) => m.id)
        : null;
      writeProviderFile({
        id: old.id,
        label: old.label || '',
        endpoint: { url: old.url || null, api: old.api || null },
        models: old.declarations || null,
        selection,
        catalog: old.catalog ? {
          revision: old.catalog.revision || 0,
          fetchedAt: old.catalog.fetchedAt || null,
          models: (old.catalog.models || []).map((m) => ({ id: m.id, name: m.name, available: m.available !== false })),
        } : null,
        endpointRevision: old.catalogRevision || 0,
        createdAt: old.createdAt, updatedAt: old.updatedAt,
      });
    }
  }
  fs.renameSync(LEGACY_PROVIDERS_FILE, LEGACY_PROVIDERS_FILE + '.pre-file-layout');
}
// A provider file a person edited into invalid JSON is reported, never skipped:
// dropping it silently would look like the provider disappeared, and the next
// write would then delete the file for good. Nothing here removes a file it did
// not just write; removal is `deleteProvider`, which knows the id.
function brokenProviderFiles() {
  if (!fs.existsSync(PROVIDERS_DIR)) return [];
  const out = [];
  for (const name of fs.readdirSync(PROVIDERS_DIR)) {
    if (!name.endsWith('.json')) continue;
    const file = path.join(PROVIDERS_DIR, name);
    let row = null;
    try { row = JSON.parse(fs.readFileSync(file, 'utf8')); } catch (e) { out.push({ file: name, error: 'not valid JSON: ' + e.message }); continue; }
    if (!row || typeof row.id !== 'string' || !row.id) out.push({ file: name, error: 'a provider file needs a non-empty "id"' });
    else if (`${row.id}.json` !== name) out.push({ file: name, error: `"id" is ${row.id}, so this file must be named ${row.id}.json` });
  }
  return out;
}
function loadProviders() {
  migrateLegacyProviders();
  if (!fs.existsSync(PROVIDERS_DIR)) return [];
  const rows = [];
  for (const name of fs.readdirSync(PROVIDERS_DIR)) {
    if (!name.endsWith('.json')) continue;
    const row = readJson(path.join(PROVIDERS_DIR, name), null);
    if (row && typeof row.id === 'string' && row.id && `${row.id}.json` === name) rows.push(row);
  }
  rows.sort((a, b) => a.id.localeCompare(b.id));
  return rows;
}
function persistProviders(rows) {
  fs.mkdirSync(PROVIDERS_DIR, { recursive: true });
  for (const row of rows) writeProviderFile(row);
}
// What a provider looks like on the wire: the endpoint, the model declarations,
// the selection; never the token.
function providerValueFree(row) {
  return {
    id: row.id,
    ...providerTypeIdentity(row),
    providerTypeAvailable: !!findProviderType(row),
    label: row.label || '',
    url: row.endpoint?.url || null,
    api: row.endpoint?.api || null,
    tokenConfigured: getSecret(secretName(row.id)) != null,
    declarations: row.models || null,
    catalogUrl: row.catalogUrl || null,
    selection: Array.isArray(row.selection) ? row.selection : null,
    catalogRevision: row.endpointRevision || 0,
    createdAt: row.createdAt,
    updatedAt: row.updatedAt,
  };
}

function secretName(providerId) { return `provider-${providerId}-token`; }

// What a plugin looks like on the wire: where it is, where it came from, and whether
// the runtime its manifest pins is on disk. `runtimeReady` is a filesystem fact about
// the declared command existing — not a claim that the harness works.
// The plugins a client should see: what is installed, PLUS anything the hub is installing
// right now (which has no directory yet, so installedPlugins() cannot see it - and a client
// asking "what is happening" must. The in-progress one is a minimal row: its id and busy).
function pluginList() {
  const rows = installedPlugins().map(pluginValue);
  for (const [id, held] of pluginState) {
    // A plugin being installed has no directory yet, so the rows above cannot show it.
    // Its state is the highest-priority operation in its set (never empty here - an
    // empty set is what removes the entry and falls back to the disk fact).
    const op = stateOfOps(held.ops);
    if (!op || rows.some((r) => r.id === id)) continue;
    // The type is what the OPERATION stated (the install request names it). A plugin that
    // has not landed has no manifest to read a type from, and reporting `invalid` for one
    // that is simply still arriving made a client that filters by type drop the row.
    const identity = held.identity || {};
    rows.push({
      id,
      pluginType: identity.pluginType || pluginTypeOf(id),
      name: identity.name ?? null,
      summary: identity.summary ?? null,
      capabilities: Array.isArray(identity.capabilities) ? identity.capabilities : [],
      state: op,
      detail: held.detail ?? null,
    });
  }
  return rows;
}

// ONE state per plugin. Before this a plugin had three independent facts a client had
// to combine - `busy` (install|remove), `prepare.state` (running|ready|failed), and
// whether it was installed at all - so two clients could describe the same plugin
// differently and a single client could show two things at once (a removal in progress
// AND a runtime preparing). The state is the hub's own verdict, like a session's turn
// state: a client renders it and derives nothing.
//
//   absent     - the catalog carries it; nothing is on disk; it can be installed
//   installing - the hub is fetching and landing it
//   removing   - the hub is removing it
//   preparing  - the hub is materialising the runtime its manifest pins
//   failed     - its runtime could not be prepared; it can be retried
//   ready      - it is on disk and the hub is doing nothing to it
function pluginStateOf(id, hasDir) {
  const held = pluginState.get(id);
  if (held) {
    const op = stateOfOps(held.ops);
    if (op) return op;                       // removing | installing | preparing | failed
    // ops is empty: nothing is being done - fall through to the disk fact.
  }
  // A record marked `deleting` is an UNFINISHED removal: the hub was killed (or the files
  // could not go yet). It reads as removing until the files are gone and the record is
  // cleared - at boot, and while a retry is pending.
  const rec = (readJson(PLUGINS_FILE, {}) || {})[id];
  if (rec && rec.deleting && hasDir) return 'removing';
  return hasDir ? 'ready' : 'absent';
}

// Finish removals a previous life left half-done. A `deleting` record means files were
// about to be removed: remove them now, then drop the record and reconcile. Runs at boot,
// before the surface is served, so a client never sees the half state.
async function finishInterruptedRemovals() {
  const records = readJson(PLUGINS_FILE, {}) || {};
  const pending = Object.keys(records).filter((id) => records[id] && records[id].deleting);
  for (const id of pending) {
    const dir = pluginDir(id);
    process.stderr.write(`removal of '${id}' was interrupted; finishing it` + NL);
    try { await stopHarnessProcesses(id); } catch { /* nothing to stop */ }
    if (fs.existsSync(dir)) {
      const failed = await rmRetrying(dir, { tries: 1 });
      if (failed) {
        // Still cannot go: keep the intent so the next boot (or a retry) tries again.
        process.stderr.write(`the files at '${dir}' could not be removed yet (${failed.message}); the removal stays pending` + NL);
        continue;
      }
    }
    mutateJson(PLUGINS_FILE, {}, (r) => { delete r[id]; });
    reconcileHarnesses();
    await loadProviderPlugins();
    announcePluginsChanged(id);
  }
}

// A plugin's TYPE, read from its manifest's own `pluginType` field. It is never inferred
// from the storage key's shape or from which other fields happen to be present: the type
// is a declared fact, not a guess. An absent or unusable one reads as `invalid`.
function pluginTypeOf(key) {
  const m = manifestOf(key) || {};
  return typeof m.pluginType === 'string' && m.pluginType ? m.pluginType : 'invalid';
}

function pluginValue(id) {
  const m = manifestOf(id) || {};
  const installed = readJson(PLUGINS_FILE, {}) || {};
  const rec = installed[id] || null;
  const held = pluginState.get(id) || null;
  const providerEntry = providerModuleEntry(id);
  return {
    id,
    // What the plugin IS. While the hub is operating on it, this is the identity captured
    // when the operation began - a removal deletes the manifest, and the plugin's own name
    // and type must not disappear while a person watches. At rest it is the manifest on
    // disk. Either way it is a FEW small fields, never the install configuration
    // (runtime.sources is hundreds of entries, lives in the artifact, and is read when the
    // runtime is materialised - never in a listing).
    pluginType: (held && held.identity && held.identity.pluginType) || pluginTypeOf(id),
    name: (held && held.identity && held.identity.name) || (typeof m.name === 'string' ? m.name : null),
    summary: (held && held.identity && held.identity.summary) || (typeof m.summary === 'string' ? m.summary : null),
    capabilities: (held && held.identity && held.identity.capabilities && held.identity.capabilities.length) ? held.identity.capabilities : (Array.isArray(m.capabilities) ? m.capabilities : []),
    icons: harnessIcons(id),
    provider: m.provider ? { apiVersion: m.provider.apiVersion, module: m.provider.module, types: [...PROVIDER_TYPE_INDEX.values()].filter((x) => x.pluginId === id).map((x) => `${x.descriptor.id}@${x.descriptor.version}`), fault: providerEntry && providerEntry.error ? providerEntry.error : null } : null,
    origin: pluginOrigin(id),
    path: pluginDir(id),
    source: rec ? rec.source : null,
    ref: rec ? rec.ref : null,
    commit: rec ? rec.commit : null,
    // Which release artifact this plugin was installed from, verbatim as the
    // install named it. A client comparing that with a catalog entry is how it
    // knows an update exists; the hub itself does not compare anything.
    artifact: rec && rec.artifact ? { id: rec.artifact.id, version: rec.artifact.version, url: rec.artifact.url, sha256: rec.artifact.sha256, size: rec.artifact.size ?? null } : null,
    installedAt: rec ? rec.installedAt : null,
    // The plugin's own version (what a release publishes) and the runtime it pins are
    // two facts: an adapter fix keeps the runtime and still gets a new version.
    version: typeof m.version === 'string' ? m.version : null,
    // Two facts a caller must not confuse: the pin the manifest declares, and
    // what is actually unpacked right now. A runtime directory can be left over
    // from a different pin (an interrupted replace), and reporting only the pin
    // made that invisible.
    runtime: m.runtime ? { package: m.runtime.package, version: m.runtime.version, target: runtimeTarget(id), installed: installedRuntime(id), orphaned: false } : null,
    // Where the runtime is being installed from: the catalog's recorded official sources
    // for a registry install, the adapter itself for a development entry.
    runtimeSource: held ? held.source ?? null : null,
    // Sources the recorded runtime left out on this machine (another platform, or an
    // optional one that could not be fetched). Named, because a runtime quietly missing
    // a piece is worse than one that says what it left out.
    runtimeSkip: held && held.result ? (held.result.skipped ?? null) : null,
    runtimeReady: runtimeReady(id),
    // The hub's ONE verdict on this plugin, like a session's turn state. A client renders
    // it; it never combines `busy` with `prepare.state` again, because there is no such
    // pair any more.
    // "on disk" is whether the plugin's DIRECTORY exists, not whether a record of an
    // install does: a plugin put there by hand, or cloned from git, has a directory and
    // no record, and calling it absent was wrong. The record says WHERE it came from.
    state: pluginStateOf(id, fs.existsSync(path.join(pluginDir(id), 'manifest.json'))),
    // The current state's own words, when it has any (what is downloading, why it failed).
    // INFORMATION about `state`, never a second state.
    detail: held ? held.detail : null,
    invalid: manifestFault(id),
  };
}

// Installing a plugin: clone a repository into the hub's own plugins root, under the
// id its manifest declares. The hub reads the id out of the clone and refuses a
// manifest that would step outside that root; everything else about the plugin is the
// plugin's business.
const PLUGIN_ID_RE = /^[a-z0-9][a-z0-9._-]*$/;
const ARTIFACT_LIMIT = Number(process.env.AGENT_HUB_ARTIFACT_LIMIT || 4 * 1024 * 1024 * 1024);
const ARTIFACT_TIMEOUT = Number(process.env.AGENT_HUB_ARTIFACT_TIMEOUT_MS || 900_000);

// The catalog is DATA, not a service: a registry file this hub was shipped with (or
// was pointed at) restating where first-party plugins are released. The hub does not
// resolve, rank or rewrite it — whoever writes the file answers for it, exactly as
// with every other declaration the hub stores. It is also not a source of what gets
// downloaded: an install names the artifact it wants (url + sha256), so the digest in
// the request is what is verified, and a catalog that lies about its own artifact
// fails the same way a hand-typed URL does.
const REGISTRY_URL = process.env.AGENT_HUB_REGISTRY_URL || null;
const REGISTRY_FILE = process.env.AGENT_HUB_REGISTRY_FILE || path.join(DATA_DIR, 'registry.json');
async function refreshRegistry() {
  if (!REGISTRY_URL) throw Object.assign(new Error('no registry URL is configured'), { code: 'registry_url_missing' });
  const answer = await fetch(REGISTRY_URL, { redirect: 'follow', signal: AbortSignal.timeout(ARTIFACT_TIMEOUT) });
  if (!answer.ok) throw Object.assign(new Error(`the registry URL answered ${answer.status}`), { code: 'registry_fetch_failed' });
  const text = await answer.text();
  let raw = null;
  try { raw = JSON.parse(text); } catch (e) { throw Object.assign(new Error(`the registry URL did not return JSON: ${e.message}`), { code: 'registry_fetch_failed' }); }
  if (!raw || !Array.isArray(raw.plugins)) throw Object.assign(new Error('the registry has no plugins array'), { code: 'registry_fetch_failed' });
  fs.writeFileSync(REGISTRY_FILE, text);
  return { source: REGISTRY_FILE, plugins: raw.plugins.length };
}
function catalogValue() {
  if (!fs.existsSync(REGISTRY_FILE)) return { schema: 1, source: null, plugins: [], fault: `no registry at ${REGISTRY_FILE}` };
  let raw = null;
  try { raw = JSON.parse(fs.readFileSync(REGISTRY_FILE, 'utf8')); }
  catch (e) { return { schema: 1, source: REGISTRY_FILE, plugins: [], fault: `the registry file is not readable JSON: ${e.message}` }; }
  if (!raw || typeof raw !== 'object' || !Array.isArray(raw.plugins)) {
    return { schema: 1, source: REGISTRY_FILE, plugins: [], fault: 'the registry file has no plugins array' };
  }
  return { schema: typeof raw.schema === 'number' ? raw.schema : 1, source: REGISTRY_FILE, note: raw.note ?? null, plugins: raw.plugins };
}

// Installing a plugin. Two sources, one landing rule: what lands is a directory with
// a manifest, under the id that manifest declares, in the hub's OWN plugins root.
//   {source:{url, ref?}}                        — git clones it (a URL or a local path)
//   {source:{artifact:{url, sha256, id, ...}}}  — a release zip, digest verified
// An id the hub already installed is REPLACED: that is what updating is, and the two
// paths share it because the only difference is whether a directory was already
// there. An id living in a root the hub does not own (a deployment's checkout) is
// refused: that tree is somebody else's.
async function installPlugin(body, res) {
  const source = body && body.source;
  if (!source || typeof source !== 'object') {
    return fail(res, 400, 'validation_failed', 'source is required: {url, ref?} for a git checkout, or {artifact:{url, sha256, id, version}} for a release artifact');
  }
  if (source.artifact !== undefined) {
    if (source.url !== undefined || source.ref !== undefined) {
      return fail(res, 400, 'validation_failed', 'source names both a git url and an artifact: say one');
    }
    return await installArtifact(source.artifact, res);
  }
  if (typeof source.url !== 'string' || !source.url.trim()) {
    return fail(res, 400, 'validation_failed', 'source.url is required (a git URL or a local path)');
  }
  if (source.ref !== undefined && typeof source.ref !== 'string') {
    return fail(res, 400, 'validation_failed', 'source.ref must be a string');
  }
  fs.mkdirSync(HUB_PLUGINS_DIR, { recursive: true });
  const staging = path.join(HUB_PLUGINS_DIR, `.staging-${process.pid}-${Date.now()}`);
  const argv = ['clone', '--depth', '1', ...(source.ref ? ['--branch', source.ref] : []), source.url, staging];
  try {
    // ASYNC: a git clone can take minutes, and running it on the event loop froze the
    // hub for the whole of it. The hub answers everything else while a clone runs.
    await execFileAsync('git', argv, { windowsHide: true, timeout: 600_000, maxBuffer: 16 * 1024 * 1024 });
  } catch (e) {
    fs.rmSync(staging, { recursive: true, force: true });
    const said = String((e.stderr || e.stdout || e.message || '')).trim().split(NL).filter(Boolean).pop() || 'git failed';
    return fail(res, 502, 'plugin_install_failed', `git clone failed: ${said}`);
  }
  const mf = path.join(staging, 'manifest.json');
  if (!fs.existsSync(mf)) {
    fs.rmSync(staging, { recursive: true, force: true });
    return fail(res, 502, 'plugin_install_failed', 'the repository has no manifest.json at its root, so it is not a plugin');
  }
  const m = readJson(mf, null) || {};
  if (typeof m.id !== 'string' || !PLUGIN_ID_RE.test(m.id)) {
    fs.rmSync(staging, { recursive: true, force: true });
    return fail(res, 502, 'plugin_install_failed', `the manifest declares an unusable id (${JSON.stringify(m.id)})`);
  }
  let commit = null;
  try {
    commit = (await execFileAsync('git', ['-C', staging, 'rev-parse', 'HEAD'], { encoding: 'utf8', windowsHide: true })).stdout.trim();
  } catch { /* a plugin without git metadata is still a plugin */ }
  return await landPlugin(staging, m.id, { source: source.url, ref: source.ref || null, commit, artifact: null }, res);
}

// One download, verified before anything is unpacked: the bytes are hashed as they
// arrive, and the digest the caller named is what they are compared against. A
// mismatch removes the download and says both digests, because "the artifact is not
// what the registry says it is" is the whole fact and a retry must not hide it.
async function downloadArtifact(url, destFile) {
  const answer = await fetch(url, { redirect: 'follow', signal: AbortSignal.timeout(ARTIFACT_TIMEOUT) });
  if (!answer.ok || !answer.body) throw new Error(`the artifact URL answered ${answer.status}${answer.statusText ? ` ${answer.statusText}` : ''}`);
  const hash = crypto.createHash('sha256');
  const out = fs.createWriteStream(destFile);
  let bytes = 0;
  try {
    for await (const chunk of answer.body) {
      bytes += chunk.length;
      if (bytes > ARTIFACT_LIMIT) throw new Error(`the download passed ${ARTIFACT_LIMIT} bytes`);
      hash.update(chunk);
      if (!out.write(chunk)) await new Promise((r) => out.once('drain', r));
    }
  } finally {
    await new Promise((r) => out.end(r));
  }
  return { sha256: hash.digest('hex'), bytes };
}

async function installArtifact(a, res) {
  if (!a || typeof a !== 'object') return fail(res, 400, 'validation_failed', 'source.artifact must be an object');
  if (typeof a.url !== 'string' || !/^https?:[/][/]/i.test(a.url)) {
    return fail(res, 400, 'validation_failed', 'source.artifact.url must be an http(s) URL');
  }
  if (typeof a.sha256 !== 'string' || !/^[0-9a-f]{64}$/i.test(a.sha256)) {
    return fail(res, 400, 'validation_failed', 'source.artifact.sha256 must be 64 hex characters: an artifact without a digest is not verifiable');
  }
  if (typeof a.id !== 'string' || !PLUGIN_ID_RE.test(a.id)) {
    return fail(res, 400, 'validation_failed', `source.artifact.id must be a plugin id (got ${JSON.stringify(a.id)})`);
  }
  // The artifact states its TYPE. Without it the hub cannot compose the plugin's storage
  // key before the bytes arrive, and would have to guess or fall back to the bare id -
  // which is how one install announced itself under two different names. A type that is
  // not stated is refused, like a version or a digest that is not stated.
  if (typeof a.pluginType !== 'string' || !a.pluginType.trim()) {
    return fail(res, 400, 'validation_failed', 'source.artifact.pluginType is required: a plugin states its type');
  }
  if (typeof a.version !== 'string' || !a.version.trim()) {
    return fail(res, 400, 'validation_failed', 'source.artifact.version is required: which release this artifact is');
  }
  if (a.size !== undefined && (!Number.isInteger(a.size) || a.size <= 0)) {
    return fail(res, 400, 'validation_failed', `source.artifact.size must be a positive integer of bytes (got ${JSON.stringify(a.size)})`);
  }
  fs.mkdirSync(HUB_PLUGINS_DIR, { recursive: true });
  // The storage key is the kind joined with the id - built HERE, once, before a byte is
  // fetched, so the state this starts already has the key the plugin will have on disk.
  // The kind comes from the registry entry's own field; it is never read out of the id.
  const key = storageKey(a.pluginType, a.id);
  enterState(key, 'installing', null, a.pluginType);
  try {
  const work = fs.mkdtempSync(path.join(HUB_PLUGINS_DIR, '.download-'));
  const file = path.join(work, 'artifact.zip');
  try {
    let got;
    try {
      got = await downloadArtifact(a.url, file);
    } catch (e) {
      return fail(res, 502, 'artifact_download_failed', `${a.url}: ${e.message}`);
    }
    if (a.size !== undefined && got.bytes !== a.size) {
      return fail(res, 502, 'artifact_digest_mismatch', `the artifact says ${a.size} bytes, this download is ${got.bytes}: it is not the release it claims to be`);
    }
    if (got.sha256 !== a.sha256.toLowerCase()) {
      return fail(res, 502, 'artifact_digest_mismatch', `sha256 mismatch: the artifact says ${a.sha256}, this download hashes to ${got.sha256}`);
    }
    const staging = path.join(HUB_PLUGINS_DIR, `.staging-${process.pid}-${Date.now()}`);
    try {
      extractZipTo(file, staging);
    } catch (e) {
      fs.rmSync(staging, { recursive: true, force: true });
      return fail(res, 502, 'plugin_archive_invalid', e.message);
    }
    const mf = path.join(staging, 'manifest.json');
    if (!fs.existsSync(mf)) {
      fs.rmSync(staging, { recursive: true, force: true });
      return fail(res, 502, 'plugin_archive_invalid', 'the archive has no manifest.json at its root, so it is not a plugin');
    }
    const m = readJson(mf, null) || {};
    if (typeof m.id !== 'string' || !PLUGIN_ID_RE.test(m.id)) {
      fs.rmSync(staging, { recursive: true, force: true });
      return fail(res, 502, 'plugin_archive_invalid', `the manifest declares an unusable id (${JSON.stringify(m.id)})`);
    }
    if (m.id !== a.id) {
      fs.rmSync(staging, { recursive: true, force: true });
      return fail(res, 502, 'plugin_archive_invalid', `the archive declares id '${m.id}' but the artifact was for '${a.id}'`);
    }
    if (typeof m.pluginType !== 'string' || !m.pluginType) {
      fs.rmSync(staging, { recursive: true, force: true });
      return fail(res, 502, 'plugin_archive_invalid', `the archive declares no kind`);
    }
    // The version is the manifest's, or the plugin's package.json when the manifest
    // states none (a provider plugin may carry its version there - pack-plugins reads
    // the same two places).
    const mVersion = typeof m.version === 'string' && m.version ? m.version
      : (() => { try { return JSON.parse(fs.readFileSync(path.join(staging, 'package.json'), 'utf8')).version || null; } catch { return null; } })();
    if (mVersion !== a.version) {
      fs.rmSync(staging, { recursive: true, force: true });
      return fail(res, 502, 'plugin_archive_invalid', `plugin version '${mVersion}' does not match artifact '${a.version}'`);
    }
    return await landPlugin(staging, m.id, { source: a.url, ref: null, commit: null, artifact: { id: a.id, version: a.version, url: a.url, sha256: a.sha256.toLowerCase(), size: a.size ?? null } }, res);
  } finally {
    fs.rmSync(work, { recursive: true, force: true });
  }
  } finally {
    leaveState(key, 'installing');
  }
}

// A checked staging directory goes into place, and the answer says what that did.
// Replacing moves the old copy aside first and removes it only after the new one is
// in place, so a failure at any step leaves the previous plugin exactly where it was.
// Put an install in progress on the record for the WHOLE of it - every return path,
// success or refusal - so a client reads "installing" from the hub, never from its own
// memory of the click. The body below does the work.
async function landPlugin(staging, id, record, res) {
  const m = readJson(path.join(staging, 'manifest.json'), null) || {};
  // The key is composed here, at use time: the manifest's own `id` and `pluginType`,
  // joined once. `id` is the plugin's name; `pluginType` is its type. Neither is the other.
  const key = storageKey(m.pluginType, m.id || id);
  enterState(key, 'installing', null, m.pluginType);
  try {
    return await landPluginBody(staging, key, record, res);
  } finally {
    leaveState(key, 'installing');
  }
}

async function landPluginBody(staging, id, record, res) {
  // `id` here is the storage KEY (`<kind>-<id>`), the directory and the map key. It is
  // opaque below this line: nothing splits it, and the kind is read from the manifest.
  const m = readJson(path.join(staging, 'manifest.json'), null) || {};
  const dest = path.join(HUB_PLUGINS_DIR, id);
  const owner = pluginRootOf(id);
  if (owner && owner !== HUB_PLUGINS_DIR) {
    fs.rmSync(staging, { recursive: true, force: true });
    return fail(res, 409, 'conflict', `plugin '${id}' is already installed in a directory this hub does not own (${owner}): replacing a deployment's plugin is that deployment's business`);
  }
  // A directory at dest is a replace even when it has no manifest: an interrupted
  // install leaves exactly that, and pluginRootOf cannot see it (it looks for a
  // manifest), so keying on the manifest would treat the orphan as a first install.
  const replacing = owner === HUB_PLUGINS_DIR || (owner === null && fs.existsSync(dest));
  // A directory can survive an interrupted replace holding nothing but a runtime
  // (the adapter files are gone). It is not a usable install, and a rename over it
  // is what produced the reported EPERM. Its runtime is still worth keeping: an
  // adapter-only update must not re-fetch hundreds of megabytes.
  const destExists = fs.existsSync(dest);
  const destHasManifest = fs.existsSync(path.join(dest, 'manifest.json'));
  let carriedRuntime = null;
  if (replacing && fs.existsSync(path.join(dest, 'runtime')) && !fs.existsSync(path.join(staging, 'runtime'))) {
    carriedRuntime = path.join(HUB_PLUGINS_DIR, `.runtime-carry-${process.pid}-${Date.now()}`);
  }
  if (replacing) {
    // Replacing a plugin a running session is using would pull the files out from
    // under that process — and on Windows the rename simply fails, with a message
    // nobody can act on. Refused here, by name, exactly like removal.
    const live = openSessions(id);
    if (live.length) {
      fs.rmSync(staging, { recursive: true, force: true });
      return fail(res, 409, 'plugin_in_use', `harness '${id}' has ${live.length} open session(s) (${live.map((s) => s.id).join(', ')}): close them first`);
    }
  }
  if (destExists && !destHasManifest) {
    // The half-replaced directory cannot be renamed over and is not a plugin. Its
    // runtime moves aside first so an adapter-only update keeps it, then the rest
    // is removed with the same retry a removal uses; a directory still held open
    // is reported as exactly that instead of a rename EPERM nobody can act on.
    if (carriedRuntime) {
      try { fs.renameSync(path.join(dest, 'runtime'), carriedRuntime); }
      catch (error) {
        fs.rmSync(staging, { recursive: true, force: true });
        return fail(res, 502, 'plugin_dir_busy', `the interrupted install at ${dest} could not be read: ${error.message} — close anything using that harness and retry`);
      }
    }
    const held = await rmRetrying(dest);
    if (held) {
      fs.rmSync(staging, { recursive: true, force: true });
      if (carriedRuntime) {
        try { fs.mkdirSync(dest, { recursive: true }); fs.renameSync(carriedRuntime, path.join(dest, 'runtime')); } catch { /* the message below is the fact */ }
      }
      return fail(res, 502, 'plugin_dir_busy', `the interrupted install at ${dest} could not be cleared: ${held.message} — close anything using that harness and retry`);
    }
  }
  const aside = replacing && destHasManifest ? path.join(HUB_PLUGINS_DIR, `.outgoing-${process.pid}-${Date.now()}`) : null;
  try {
    if (carriedRuntime && fs.existsSync(path.join(dest, 'runtime'))) fs.renameSync(path.join(dest, 'runtime'), carriedRuntime);
    if (aside) fs.renameSync(dest, aside);
    // The staging rename is the step Windows can refuse while a previous copy is
    // being released; retried under the same rule as a removal.
    let moved = null;
    for (let attempt = 1; attempt <= 6; attempt++) {
      try { fs.renameSync(staging, dest); moved = null; break; }
      catch (error) {
        moved = error;
        // ASYNC wait: a synchronous spin here froze the whole hub during an install -
        // a refresh or a second action could not be answered while it retried.
        await new Promise((resolve) => setTimeout(resolve, 250));
      }
    }
    if (moved) throw moved;
    if (carriedRuntime) fs.renameSync(carriedRuntime, path.join(dest, 'runtime'));
  } catch (e) {
    // Restore the old plugin BEFORE returning its runtime. Creating dest first
    // would prevent the old directory from being restored after a failed rename.
    try {
      if (aside && fs.existsSync(aside)) {
        if (fs.existsSync(dest)) fs.rmSync(dest, { recursive: true, force: true });
        fs.renameSync(aside, dest);
      }
      if (carriedRuntime && fs.existsSync(carriedRuntime)) {
        if (!fs.existsSync(dest)) fs.mkdirSync(dest, { recursive: true });
        fs.renameSync(carriedRuntime, path.join(dest, 'runtime'));
      }
    } catch (restoreError) {
      e.message += `; rollback failed: ${restoreError.message}; retained old plugin: ${aside}; runtime: ${carriedRuntime}`;
    }
    fs.rmSync(staging, { recursive: true, force: true });
    return fail(res, 502, 'plugin_install_failed', `the plugin could not be put in place: ${e.message}`);
  }
  if (aside) {
    // Same Windows reality: the outgoing copy's files may be a moment from being released.
    const failed = await rmRetrying(aside);
    if (failed) console.error(`plugin '${id}': the previous copy at ${aside} could not be removed yet (${failed.message})`);
  }
  mutateJson(PLUGINS_FILE, {}, (records) => {
    records[id] = { ...record, installedAt: new Date().toISOString() };
  });
  reconcileHarnesses();
  // A provider a moment ago was not here: its types must exist for anyone to use it.
  await loadProviderPlugins();
  ensureRuntimeFor(id).catch(() => {});
  announcePluginsChanged(id);
  json(res, replacing ? 200 : 201, { plugin: pluginValue(id), updated: replacing });
}

// The sessions a harness would be pulled out from under, in the order they were
// opened. Both replacing and removing a plugin ask this first.
function openSessions(harnessId) {
  return [...sessions.values()].filter((s) => s.harnessId === harnessId && s.status !== 'closed');
}

// Removing a plugin removes what the HUB installed, and nothing else. A directory a
// deployment put on the search path is read-only to the hub, and a harness with open
// sessions is refused rather than yanked out from under them: both refusals name the
// thing that has to change first.
async function removePlugin(id, res) {
  const dir = pluginDir(id);
  if (!dir || !fs.existsSync(dir) || !manifestOf(id)) return fail(res, 404, 'not_found', `no plugin '${id}'`);
  if (pluginOrigin(id) !== 'hub') {
    return fail(res, 409, 'conflict', `plugin '${id}' is in a directory this hub does not own (${dir}): removing it is that deployment's business`);
  }
  enterState(id, 'removing');
  try {
  
    // Removal is the caller's decision, so nothing here asks again. The user stated
    // the intent and the hub owns the mechanics, in this order:
    //   1. remove the plugin files,
    //   2. close the sessions of that harness,
    //   3. stop the processes the hub started for it.
    // What "remove the plugin files" requires is that nothing is running FROM them:
    // a running harness holds its own image under the plugin directory, and Windows
    // will not delete a running image. So the stop happens BEFORE the delete - it is
    // the precondition of step 1, not a later step - and the sessions are closed
    // after, which is the part of the order the user asked for. Every process
    // involved was started by the hub, so the hub stops them: telling the caller to
    // "close anything using that harness" asked for something they had no way to do.
    // A runtime install of this plugin may still be running (the hub starts one after
    // every install). It is writing hundreds of files under the plugin directory the
    // moment we try to delete, which is what the EPERM was: a removal must not race the
    // plugin's own prepare. Wait for it to finish, then stop processes.
    // ORDER MATTERS, AND IT SURVIVES A KILL.
    //
    // A removal touches three things that are not one thing: the plugin's files, the record
    // of where it came from (installed.json), and the harness registry (harnesses.json).
    // Doing them in an arbitrary order means a hub killed between two of them comes back
    // holding half a removal - a record pointing at a directory that is gone, or a registry
    // row for a plugin that does not exist. So the removal is written down BEFORE it starts
    // and cleared only when it is done:
    //
    //   1. satisfy the preconditions (no prepare racing us, no process holding the files,
    //      the harness's sessions closed) - a running image cannot be deleted on Windows;
    //   2. WRITE THE INTENT: a `deleting` flag on the plugin's record;
    //   3. delete the files;
    //   4. clear the record entirely and reconcile the registry, which also drops the
    //      intent - the removal is now done and there is nothing left to finish;
    //
    // A hub that starts and finds a `deleting` record finishes the job (see
    // finishInterruptedRemovals) instead of pretending nothing happened.
    const preparing = pluginState.get(id);
    if (preparing && preparing.inFlight) { try { await preparing.inFlight; } catch { /* its failure is not the removal's */ } }
    await stopHarnessProcesses(id);
    const closed = closeSessionsOf(id);
    // The intent. Written before any file is touched, so a kill from here on is recoverable.
    const record = (readJson(PLUGINS_FILE, {}) || {})[id];
    mutateJson(PLUGINS_FILE, {}, (records) => {
      records[id] = { ...(record || {}), deleting: true, deletingSince: new Date().toISOString() };
    });
    let failed = null;
    for (let attempt = 1; attempt <= 12; attempt++) {
      failed = await rmRetrying(dir, { tries: 1 });
      if (!failed) break;
      // A harness takes a moment to release its files after being asked to stop, and a
      // process it spawned between sweeps can appear again. Sweep again on each retry
      // rather than only waiting: the process that holds the file is the thing to stop,
      // and waiting alone just gives up on it. `await`ed so the hub keeps answering.
      await stopHarnessProcesses(id);
      await new Promise((resolve) => setTimeout(resolve, 400));
    }
    if (failed) {
      // The files are still there. The intent stays - the removal is unfinished, not
      // cancelled - so a retry or a restart continues it. The record keeps the plugin's
      // origin, and the row keeps its identity.
      return fail(res, 409, 'plugin_in_use', `plugin '${id}' is still in use by a process that did not stop: ${failed.message} — its ${closed} session(s) were closed; the removal is unfinished and will continue on retry or restart`);
    }
    // The files are gone. The record follows, which clears the intent; then the registry.
    mutateJson(PLUGINS_FILE, {}, (records) => { delete records[id]; });
    reconcileHarnesses();
    // A removed provider's types must go with it, or a client keeps offering one that
    // is no longer here.
    await loadProviderPlugins();
    announcePluginsChanged(id);
    json(res, 200, { ok: true, id, sessionsClosed: closed, plugins: pluginList() });
  } finally { leaveState(id, 'removing'); }
}

// Close every open session of one harness and kill the adapter processes the hub
// owns for it. A closed session keeps its history: this is not a deletion.
function closeSessionsOf(harnessId) {
  let closed = 0;
  for (const s of sessions.values()) {
    if (s.harnessId !== harnessId || s.status === 'closed') continue;
    const conn = connFor(s.id);
    try { conn?.proc?.kill(); } catch { /* the tree removal below reports what is really holding it */ }
    // NOTE: there is no adapter method to ask a harness to stop its own detached
    // server, and inventing one would put a method in the core that only one
    // plugin answers. The process sweep below is what actually releases the files;
    // the adapter kill above only stops the process in front.
    s.status = 'closed';
    s.activeTurn = { state: 'idle', ended: null, cause: null, partialPersisted: false, partialItems: 0 };
    s.updatedAt = new Date().toISOString();
    if (adapters.get(s.id) === conn) adapters.delete(s.id);
    closed++;
  }
  if (closed) persistSessions();
  return closed;
}

// The processes the hub started under this harness's data directory. The adapters
// are children this process knows; a harness's own server (dsh) is detached BY
// DESIGN, so it is found by the data directory it was started with rather than by
// process parentage. Only processes whose command line names THIS harness's
// directory are touched - never another harness, never an unrelated process, and
// never a harness the user runs outside this application.
async function stopHarnessProcesses(harnessId) {
  const agentDir = path.join(DATA_DIR, 'agents', harnessId);
  const pluginPath = pluginDir(harnessId);
  // The adapters this process spawned are the first thing to stop. They are keyed
  // by SESSION id, not by harness id, so the session record is what identifies
  // them: matching on the harness id here matched nothing and left every adapter
  // running through the removal.
  for (const [key, conn] of adapters) {
    const owner = sessions.get(key);
    if (!owner || owner.harnessId !== harnessId) continue;
    try { conn.proc.kill(); } catch { /* already gone */ }
  }
  if (process.platform !== 'win32') return;
  // A harness's own server is detached BY DESIGN (dsh keeps serving after its
  // adapter dies), so it is not a child of this process and parentage cannot find
  // it. It is found by what it was started with and where it runs:
  //   * an executable UNDER the plugin directory - Windows will not delete a
  //     running image, and that is the whole of the reported failure;
  //   * or a command line naming this harness's own data directory.
  // Both are specific to THIS harness. Nothing else - another harness, a system
  // node, a process the user runs outside this application - can match.
  const script = path.join(os.tmpdir(), `agent-hub-stop-${harnessId}-${process.pid}.ps1`);
  const quote = (value) => value.replace(/'/g, "''");
  try {
    fs.writeFileSync(script, [
      `$plugin = '${quote(pluginPath)}'`,
      `$agents = '${quote(agentDir)}'`,
      'Get-CimInstance Win32_Process | Where-Object {',
      '  $_.ProcessId -ne $PID -and (',
      '    ($_.ExecutablePath -and $_.ExecutablePath.StartsWith($plugin, [StringComparison]::OrdinalIgnoreCase)) -or',
      '    ($_.CommandLine -and $_.CommandLine.Contains($agents)) -or',
      '    ($_.CommandLine -and $_.CommandLine.Contains($plugin))',
      '  )',
      '} | ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }',
      '',
    ].join(String.fromCharCode(10)));
    // Awaited, not run synchronously: a PowerShell sweep takes seconds, and doing it on
    // the event loop stops the hub from answering anything else - which is what made a
    // UI refresh during a removal fail to connect. The hub stays responsive while this
    // runs.
    await execFileAsync('powershell', ['-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', script], { windowsHide: true });
  } catch { /* a machine without PowerShell is not a reason to refuse the removal */ }
  finally { try { fs.rmSync(script, { force: true }); } catch { /* it is in temp; the OS will clear it */ } }
}

// --- the runtime a REGISTRY install carries -------------------------------------
// A registry artifact is the plugin; its runtime is the plugin's official distribution,
// recorded in the catalog as the vendor's own tarballs with the vendor's own integrity.
// The hub fetches and unpacks them itself. No package manager runs on this machine —
// that is the point of the artifacts: a clean machine installs a harness, not a build
// job. The adapter's own runtime/prepare stays what it always was for a DEVELOPMENT
// entry (a git checkout), and is never used for something installed from the registry.
// The runtime closure an installed plugin ships, read from its own directory
// (runtime.sources.json, packed into the artifact). It is not in the registry: the
// catalog lists adapters, the artifact carries install detail.
function runtimeDeclaration(id) {
  const file = path.join(pluginDir(id), 'runtime.sources.json');
  let runtime = null;
  try { runtime = JSON.parse(fs.readFileSync(file, 'utf8')); } catch { return null; }
  if (!runtime || !Array.isArray(runtime.sources) || !runtime.sources.length) return null;
  return runtime;
}

function installCatalogRuntime(id) {
  const held = pluginState.get(id);
  if (held && held.inFlight) return held.inFlight;
  const manifest = manifestOf(id) || {};
  const declared = !!(manifest.runtime && Array.isArray(manifest.capabilities) && manifest.capabilities.includes('runtime'));
  enterState(id, 'preparing', 'installing the runtime this plugin pins');
  const state = pluginState.get(id);
  state.source = 'catalog';
  const installedVersion = manifest.version;
  const runtime = runtimeDeclaration(id);
  // Where the runtime goes: the directory the catalog names, resolved inside THIS
  // plugin's own directory. The manifest's command points at a file inside it, so the
  // record states the directory instead of having anything derive it by guessing.
  const pluginRoot = path.resolve(pluginDir(id));
  const target = path.resolve(pluginRoot, (runtime && runtime.target) || 'runtime');
  if (target !== pluginRoot && !target.startsWith(pluginRoot + path.sep)) {
    throw Object.assign(new Error(`the catalog's runtime target (${target}) is outside the plugin directory`), { code: 'runtime_unavailable' });
  }
  const inFlight = Promise.resolve()
    .then(() => {
      if (!declared) {
        return { ready: true, detail: 'this plugin declares no runtime' };
      }
      if (!runtime) {
        throw Object.assign(new Error(`'${id}' ships no runtime.sources.json, so the hub cannot install its runtime (it runs no package manager)`), { code: 'runtime_unavailable' });
      }
      if (runtime.package !== manifest.runtime.package || runtime.version !== manifest.runtime.version) {
        throw Object.assign(new Error(`runtime pin does not match plugin ${id}@${installedVersion}`), { code: 'runtime_unavailable' });
      }
      // Already on disk from the same pin? Then this is not a runtime install: an adapter
      // update keeps its runtime, and re-fetching hundreds of tarballs to arrive at the
      // same directory would be busy-work with a network bill.
      const held = runtimeMarker(target);
      if (held && held.package === runtime.package && held.version === runtime.version && held.sourcesDigest === sourcesDigest(runtime.sources) && runtimeReady(id)) {
        state.result = { package: runtime.package, version: runtime.version, sources: 0, skipped: held.skipped || [], bytes: 0, already: true };
        return { ready: true, ...state.result };
      }
      return installRuntime(runtime, target, { record: { package: runtime.package, version: runtime.version },
        onProgress: (progress) => { state.detail = `${progress.phase} ${progress.index}/${progress.total} ${progress.detail ?? ''}`.trim(); },
        log: (line) => { state.detail = line; },
      }).then((result) => {
        state.detail = result.skipped.length
          ? `installed ${result.installed}/${result.installed + result.skipped.length} sources; skipped: ${result.skipped.map((s) => `${s.path} (${s.reason})`).join('; ')}`
          : `installed ${result.installed} official source(s)`;
        state.result = { package: runtime.package ?? null, version: runtime.version ?? null, sources: result.installed, skipped: result.skipped, bytes: result.bytes };
        return { ready: true, ...state.result };
      });
    })
    .then((result) => {
      // Done: the plugin is ready. leaveState drops the entry, and pluginStateOf() reads
      // a plugin on disk with no entry as ready - one truth, one place.
      leaveState(id, 'preparing');
      return result;
    })
    .catch((e) => {
      // A failed runtime is a STATE, not a cleared busy flag: the plugin stays failed
      // (visible, retryable) instead of silently looking ready.
      failState(id, e.message);
      throw Object.assign(new Error(e.message), { code: e.code || 'runtime_install_failed' });
    });
  state.inFlight = inFlight;
  return inFlight;
}

// What a client asks for when it wants a runtime to exist: the catalog's official
// sources for a plugin that came from an artifact, the adapter's own prepare for one
// that was developed in place.
function ensureRuntimeFor(id) {
  const record = (readJson(PLUGINS_FILE, {}) || {})[id] || null;
  return record && record.artifact ? installCatalogRuntime(id) : prepareRuntime(id);
}

// runtime/prepare: the plugin materialises the runtime its manifest pins. The hub
// asks and verifies; it never installs a harness itself and knows nothing about
// packages, registries or tarball layouts. State is kept per harness so a client can
// see a long install happening (GET /v1/hub/plugins) and so two callers share one.
const PREPARE_TIMEOUT = Number(process.env.AGENT_HUB_PREPARE_TIMEOUT_MS || 900_000);
// ONE state machine per plugin. This map is the plugin's only state: whether it is
// absent, being installed, being removed, having its runtime prepared, ready, or failed.
// There used to be two - a `pluginOps` for install/remove and a `runtimePrepare` for the
// runtime - plus the on-disk record, so a plugin could be "removing" and "preparing" at
// once and a client had to combine them. Now nothing else decides a plugin's state; the
// install, the removal and the prepare each MOVE this one value.
//
//   { state, detail, since, inFlight }
//
// `detail` is the state's own words (a download line, a failure reason). `inFlight` is the
// pending work, so a second caller joins it instead of starting a rival. A plugin with no
// entry is `ready` when it is on disk and `absent` when it is not - see pluginState().
const pluginState = new Map();   // id -> { state, detail, since, inFlight }

// What the hub is DOING to a plugin right now, as the hub's own fact - the same kind of
// fact as prepare.state, and for the same reason: whether an install or a removal is in
// progress belongs to the process doing it, not to a client's memory of its own click. A
// client that keeps its own flag loses it on a refresh and overwrites it when a second
// plugin is acted on. Keyed by the storage key (`<kind>-<id>`).
// What a plugin is doing right now is a SET of operations, not one counter. A landing
// also starts a runtime prepare, and a prepare can still be running when a later install
// begins; with a single refcount, one operation finishing decremented another's count and
// a plugin that was still preparing was published as ready. Each operation owns its own
// entry, and the state a client sees is the highest-priority one in the set:
//
//   removing > installing > preparing > failed > ready
//
// Entering and leaving are exact inverses per operation, so an operation can finish while
// another runs without touching it.
const OP_RANK = { removing: 3, installing: 2, preparing: 1, failed: 0 };
function stateOfOps(ops) {
  let best = null;
  for (const op of ops) if (best === null || OP_RANK[op] > OP_RANK[best]) best = op;
  return best;
}
// An operation acts on a PLUGIN, and while it runs the hub must be able to say what that
// plugin is. The identity is captured on entry, while the manifest still exists - a removal
// deletes the manifest, and "what is this" must not vanish the moment the removal starts.
// `manifest` may be null (an install before anything has landed), in which case the type
// the REQUEST stated is used; anything still unknown reads as null, never as a guess.
function enterState(id, op, detail = null, pluginType = null) {
  const held = pluginState.get(id) || { ops: new Set(), detail: null, identity: null, since: new Date().toISOString(), inFlight: null };
  const before = stateOfOps(held.ops);
  if (!held.identity) {
    const m = manifestOf(id);
    held.identity = {
      pluginType: (m && typeof m.pluginType === 'string' ? m.pluginType : null) || pluginType || null,
      name: m && typeof m.name === 'string' ? m.name : null,
      summary: m && typeof m.summary === 'string' ? m.summary : null,
      capabilities: m && Array.isArray(m.capabilities) ? m.capabilities : [],
    };
  } else if (pluginType && !held.identity.pluginType) {
    held.identity.pluginType = pluginType;
  }
  held.ops.add(op);
  held.detail = detail ?? held.detail;
  held.since = new Date().toISOString();
  pluginState.set(id, held);
  const after = stateOfOps(held.ops);
  if (after !== before) announcePluginsChanged(id);
}
// `failState` records a failure that OUTLIVES the operation: the plugin stays failed
// until something moves it, because "its runtime could not be prepared" is a fact about
// the plugin, not about the request that discovered it.
function failState(id, detail) {
  const held = pluginState.get(id) || { ops: new Set(), since: new Date().toISOString(), inFlight: null };
  held.ops = new Set(['failed']);
  held.detail = detail;
  held.since = new Date().toISOString();
  pluginState.set(id, held);
  announcePluginsChanged(id);
}
function leaveState(id, op) {
  const held = pluginState.get(id);
  if (!held) return;
  const before = stateOfOps(held.ops);
  held.ops.delete(op);
  const after = stateOfOps(held.ops);
  if (after === null) {
    // The last operation finished: the plugin is now ready (or absent, if it never
    // landed). That is a change too - the one a client most wants - so the entry goes and
    // the change is announced, rather than the plugin silently appearing ready.
    pluginState.delete(id);
    announcePluginsChanged(id);
  } else if (after !== before) {
    held.since = new Date().toISOString();
    announcePluginsChanged(id);
  }
}

function prepareRuntime(harnessId, conn = null) {
  const held = pluginState.get(harnessId);
  if (held && held.inFlight) return held.inFlight;
  const caps = (manifestOf(harnessId) || {}).capabilities || [];
  if (!caps.includes('runtime')) {
    // The plugin does not promise this method, so the hub does not pretend to have
    // another way: the session will fail with whatever the adapter says, which is the
    // truth about that plugin.
    return Promise.resolve({ ready: runtimeReady(harnessId), unsupported: true });
  }
  enterState(harnessId, 'preparing');
  const state = pluginState.get(harnessId);
  const call = conn
    ? rpc(conn, 'runtime/prepare', {}, PREPARE_TIMEOUT)
    : configRpc(harnessId, 'runtime/prepare', {});
  const inFlight = withTimeout(call, PREPARE_TIMEOUT, `runtime/prepare did not answer within ${Math.round(PREPARE_TIMEOUT / 1000)}s`)
    .then((r) => {
      const ready = r && r.ready !== false && runtimeReady(harnessId);
      state.result = r || null;
      if (!ready) {
        throw Object.assign(new Error((r && r.detail) || `the plugin answered that the runtime is not ready`), { code: 'runtime_prepare_failed' });
      }
      leaveState(harnessId, 'preparing');
      return state.result || { ready: true };
    })
    .catch((e) => {
      failState(harnessId, e.message);
      throw Object.assign(new Error(e.message), { code: 'runtime_prepare_failed' });
    });
  state.inFlight = inFlight;
  return inFlight;
}

// Before a session needs the harness: if the declared runtime is missing, ask the
// plugin for it. Ready runtimes cost one stat and nothing else.
function ensureRuntime(harnessId, conn) {
  if (runtimeReady(harnessId)) return Promise.resolve(true);
  const caps = (manifestOf(harnessId) || {}).capabilities || [];
  if (!caps.includes('runtime')) return Promise.resolve(false);
  return prepareRuntime(harnessId, conn).then(() => true);
}

function withTimeout(promise, ms, message) {
  let timer = null;
  return Promise.race([
    promise,
    new Promise((_, reject) => { timer = setTimeout(() => reject(new Error(message)), ms); }),
  ]).finally(() => { if (timer) clearTimeout(timer); });
}

// one-shot config-surface RPC against a provider-capable adapter (no session)
function configRpc(harnessId, method, params, { prepared = false } = {}) {
  declareAdapterRequest(method);
  // The adapter for a runtime-backed harness needs its runtime on disk before it is
  // spawned: started early, it exits with "runtime script not found" and the caller
  // reads that as a broken harness. So the config plane prepares first, exactly like a
  // session does — except when the caller IS the preparation.
  if (!prepared && method !== 'runtime/prepare') {
    const manifest = manifestOf(harnessId) || {};
    const wantsRuntime = !!(manifest.runtime && Array.isArray(manifest.capabilities) && manifest.capabilities.includes('runtime'));
    if (wantsRuntime && !runtimeReady(harnessId)) {
      return ensureRuntimeFor(harnessId).then(() => configRpc(harnessId, method, params, { prepared: true }));
    }
  }
  return new Promise((resolve, reject) => {
    const m = manifestOf(harnessId);
    if (!m) return reject(Object.assign(new Error('no such harness'), { code: 'harness_not_found' }));
    const caps = m.capabilities || [];
    if (!caps.includes('providers') && !caps.includes('models') && !(method === 'history/read' && caps.includes('history-readonly'))) {
      return reject(Object.assign(new Error('harness has no provider surface'), { code: 'unsupported-for-provider' }));
    }
    const dir = pluginDir(harnessId);
    const [cmd, ...args] = m.command || [];
    // Same runtime declaration as a session spawn: the manifest owns it and the
    // config plane reads the same one, so a probe and a turn drive the same pin.
    const runtimeArgv = Array.isArray(m.runtime && m.runtime.command) && m.runtime.command.length
      ? m.runtime.command.map((part, i) => (i === 0 ? part : path.resolve(dir, part)))
      : null;
    const historyOnly = method === 'history/read';
    // Same per-harness data dir as sessions (Rust: agent-data/<manifest.id>), so
    // the config plane and sessions share one home and one settings/profile.
    const agentDir = path.join(DATA_DIR, 'agents', harnessId);
    fs.mkdirSync(agentDir, { recursive: true });
    const proc = spawn(cmd, args, {
    windowsHide: true,
      cwd: dir,
      env: { ...adapterEnvironment(), AGENT_HUB_HARNESS_DIR: agentDir, AGENT_HUB_CWD: process.cwd(), ...(historyOnly ? {} : { AGENT_HUB_INSTALLED_EXTENSIONS_DIR: installExtensions(harnessId).dir, AGENT_HUB_INSTALLED_SKILLS_DIR: installSkills(harnessId).dir }), ...(presetsArgv(harnessId) ? { AGENT_HUB_PRESETS_DIR: presetsArgv(harnessId) } : {}), ...(runtimeArgv ? { AGENT_HUB_RUNTIME_COMMAND: JSON.stringify(runtimeArgv) } : {}) },
      stdio: ['pipe', 'pipe', 'inherit'],
    });
    let buf = '';
    let settled = false;
    proc.on('error', (e) => {
      if (!settled) { settled = true; reject(new Error(`adapter could not be started: ${e.message}`)); }
    });
    proc.stdout.setEncoding('utf8');
    proc.stdout.on('data', (d) => {
      buf += d;
      let i;
      while ((i = buf.indexOf('\n')) >= 0) {
        const line = buf.slice(0, i); buf = buf.slice(i + 1);
        if (!line.trim()) continue;
        let msg; try { msg = JSON.parse(line); } catch { continue; }
        if (msg.id === 1 && !settled) {
          settled = true;
          if (msg.error) reject(Object.assign(new Error(msg.error.message), { code: msg.error.code, data: msg.error.data }));
          else resolve(msg.result);
          proc.kill();
        }
      }
    });
    proc.on('exit', () => { if (!settled) { settled = true; reject(new Error('adapter exited before response')); } });
    proc.on('error', (e) => { if (!settled) { settled = true; reject(e); } });
    proc.stdin.write(JSON.stringify({ jsonrpc: '2.0', id: 1, method, params }) + '\n');
  });
}

// --- harness-private connections (capability: providers) ---------------------
// The harness owns the connection definition and the credential (its own store);
// the hub only routes the lifecycle a caller asks for and refuses to reach a
// harness that has not declared the capability. Secret values a caller submits
// pass through to that harness's adapter at that moment — the hub keeps no copy.
function harnessPrivateSurface(id) {
  const m = manifestOf(id);
  return !!m && (m.capabilities || []).includes('providers');
}
function privateConnectionRpc(harnessId, method, params) {
  if (!harnessPrivateSurface(harnessId)) {
    return Promise.reject(Object.assign(new Error('this harness has no private connection surface (it does not declare the providers capability)'), { code: 'unsupported' }));
  }
  // These operations are served by code from the harness's own runtime (its
  // native auth/connection modules). Prepare it exactly like a session spawn
  // would, so a fresh install fails at preparation with the plugin's own
  // answer instead of deep inside a module import.
  return ensureRuntime(harnessId).then(() => configRpc(harnessId, method, params));
}
// Adapter machine codes -> hub error table. An unknown failure is 502 and never a
// state the hub invented.
const PRIVATE_CONNECTION_ERRORS = {
  'unsupported': [501, 'unsupported'],
  'unsupported-for-provider': [501, 'unsupported'],
  'validation-failed': [400, 'validation_failed'],
  'unknown-provider': [404, 'provider_not_found'],
  'connection-not-found': [404, 'connection_not_found'],
  'revision-conflict': [409, 'revision_conflict'],
  'settings-conflict': [409, 'revision_conflict'],
  'settings-rejected': [400, 'validation_failed'],
  'busy-session-active': [409, 'session_busy'],
  'model-discovery-failed': [502, 'provider_catalog_failed'],
  'auth-expired': [400, 'validation_failed'],
};
function connectionFailure(res, e) {
  // An adapter carries its machine code in error.data.code (JSON-RPC reserves
  // error.code for the transport). Read the machine code first: it is the
  // contract; the numeric one only says which protocol layer answered.
  const machine = (e && e.data && e.data.code) || (e && e.code) || '';
  const [status, code] = PRIVATE_CONNECTION_ERRORS[machine] || [502, 'adapter_unreachable'];
  return fail(res, status, code, e && e.message ? e.message : 'connection operation failed');
}

// tools/list (H-6): the tool catalog is the data source for approvalDefault and
// for disabledTools (A-3). Capability-gated: a harness that does not declare
// `tools` has an unknown catalog — we say so, we never fake an empty one.
// Honesty (A-3): partial and enabled are forwarded verbatim; core never
// rewrites them to imply a block that did not happen.
// --- model catalog (API 2: the harness's own models) ------------------------
// Mirrors listTools: capability-gated; no `models` capability -> known:false,
// never a fake empty list. Per PROTOCOL §6 (R-019): each entry keeps its real
// providerId identity, one harness's failure must not pollute another's.
async function listPresets(harnessId) {
  const m = manifestOf(harnessId);
  const caps = m.capabilities || [];
  if (!caps.includes('presets')) {
    return { harnessId, known: false, presets: [] };
  }
  const r = await configRpc(harnessId, 'presets/list', {});
  const presets = (r && Array.isArray(r.presets) ? r.presets : []).map((p) => ({
    id: p.id,
    name: p.name != null ? p.name : null,
    description: p.description != null ? p.description : null,
    trust: p.trust === 'system' || p.trust === 'user' ? p.trust : null,
    isDefault: p.isDefault === true,
    broken: p.broken != null ? p.broken : null,
  }));
  return { harnessId, known: true, presets };
}

// The models a provider offers, with every fact already resolved: the declared
// name wins, then the name the provider's own catalog carried, then the id. The
// hub owns the catalog, so it is the one place that knows both halves — an
// adapter given only the declaration would fall back to the id and lose the
// name the provider published.
function effectiveModels(row) {
  const decls = row.models && typeof row.models === 'object' ? row.models : {};
  const catalog = Array.isArray(row.catalog?.models) ? row.catalog.models : [];
  const seen = new Map();
  // Start from the provider's own catalog, then let the declaration override per
  // field. Both are data the person or the plugin wrote; neither is interpreted.
  for (const m of catalog) {
    if (!m || typeof m.id !== 'string' || !m.id) continue;
    const { reasoning: _r, thinkingLevelsSource: _s, ...rest } = m;
    const entry = { ...rest, name: (typeof m.name === 'string' && m.name) || m.id };
    const levels = levelsOf(m);
    if (levels) entry.thinkingLevels = levels;
    seen.set(m.id, entry);
  }
  for (const [id, d] of Object.entries(decls)) {
    const fromCatalog = seen.get(id) || { id, name: id };
    const name = d && typeof d.name === 'string' && d.name ? d.name : fromCatalog.name;
    const out = { id, name };
    for (const key of ['input', 'cost', 'contextWindow', 'maxTokens']) {
      const value = d && d[key] !== undefined && d[key] !== null ? d[key] : fromCatalog[key];
      if (value !== undefined && value !== null) out[key] = value;
    }
    const levels = (d ? levelsOf(d) : null) ?? levelsOf(fromCatalog);
    if (levels) out.thinkingLevels = levels;
    seen.set(id, out);
  }
  return [...seen.values()];
}
// What the core owns and a harness therefore cannot discover on its own: the
// managed providers, each with the model declarations it was registered with.
// The token is not here — a catalog is not a place to move a secret (§6.2); the
// adapter needs the endpoint and the declared models, nothing more.
function injectedProviderDeclarations() {
  return loadProviders().filter((row) => !!findProviderType(row)).map((row) => ({
    id: row.id,
    url: row.endpoint?.url || null,
    models: effectiveModels(row),
    // WHOSE fact is this? The credential's. A harness cannot answer it: for a
    // core-managed provider the token lives in the hub's secret store and only
    // reaches the harness at session start (credentials/grant). Without this the
    // model picker said `needs-auth` for every managed model on dsh while pi and
    // jouzu said available — the same provider, two answers, neither from the side
    // that knew.
    hasCredential: !!getSecret(secretName(row.id)),
    credentialReachable: 'secret-store',
  }));
}
// The models a harness can actually run. A harness's own catalog answers most of
// it, but a core-managed provider lives in the core, not in the harness — so the
// core hands over the providers it owns (id, url, declaration) and the adapter
// reports them as part of this harness's catalog. That is the whole point of the
// question the endpoint answers: what can this harness run right now. A caller
// reads one list and does not merge two; a provider reachable in dsh's own
// config and one injected at session start must not look different here.
async function listModels(harnessId) {
  const m = manifestOf(harnessId);
  const caps = m.capabilities || [];
  if (!caps.includes('models')) {
    return { harnessId, known: false, models: [], failures: [] };
  }
  const r = await configRpc(harnessId, 'models/list', { providers: injectedProviderDeclarations() });
  const models = (r && Array.isArray(r.models) ? r.models : []).map((x) => ({
    id: x.id,
    providerId: x.providerId != null ? x.providerId : (x.connectionId != null ? x.connectionId : null),
    provider: x.provider || null,
    name: x.name || x.id,
    available: x.available !== false,
    unavailableReason: x.unavailableReason || null,
    // The levels the model's configuration states, passed through as data. The
    // core adds nothing, subtracts nothing and labels nothing: null = nothing was
    // stated, [] = an empty list was stated. Who interprets them is the caller.
    thinkingLevels: Array.isArray(x.thinkingLevels) ? x.thinkingLevels.filter((l) => typeof l === 'string' && l) : null,
    contextWindow: x.contextWindow != null ? x.contextWindow : null,
    maxTokens: x.maxTokens != null ? x.maxTokens : null,
  }));
  const failures = (r && Array.isArray(r.failures) ? r.failures : []).map((f) => ({
    providerId: f.providerId || null,
    connectionId: f.connectionId || null,
    message: f.message || '',
  }));
  return { harnessId, known: true, models, failures };
}

async function listTools(harnessId, mode) {
  const m = manifestOf(harnessId);
  const caps = m.capabilities || [];
  if (!caps.includes('tools')) {
    return { harnessId, known: false, tools: [], partial: false };
  }
  const r = await configRpc(harnessId, 'tools/list', mode ? { mode } : {});
  const tools = (r && Array.isArray(r.tools) ? r.tools : []).map((t) => ({
    name: t.name,
    description: t.description || '',
    kind: t.kind || 'other',
    approvalDefault: t.approvalDefault || 'once',
    enabled: t.enabled !== false,
  }));
  return { harnessId, known: true, tools, partial: r && r.partial === true };
}

// What this session's harness actually has, asked of the harness itself (pi/jouzu:
// their own command list; dsh: its skill catalogue RPC). Read through, per session —
// a catalogue depends on the session's directory. The hub never answers this from its
// own install manifest: dsh drops a skill it does not like and says nothing, so the
// only honest source for "what does it have" is the harness.
function readHarnessSkills(sid, s, res) {
  const caps = (manifestOf(s.harnessId) || {}).capabilities || [];
  if (!caps.includes('skills')) {
    return fail(res, 501, 'unsupported', `harness '${s.harnessId}' cannot report its skills`);
  }
  let conn = connFor(sid);
  if (!conn) {
    if (!s.ref) return fail(res, 502, 'adapter_unreachable', 'adapter not running; session has no resume ref');
    conn = spawnAdapter(sid, s.harnessId, s.cwd, s.additionalDirectories);
  }
  return initAdapter(conn, s, {})
    .then(() => rpc(conn, 'skills/list', { sid }, 60_000))
    .then((r) => {
      const skills = (r && Array.isArray(r.skills) ? r.skills : []).map((k) => ({
        name: String(k.name),
        ...(k.description ? { description: String(k.description) } : {}),
        ...(k.whenToUse ? { whenToUse: String(k.whenToUse) } : {}),
        ...(typeof k.modelInvocable === 'boolean' ? { modelInvocable: k.modelInvocable } : {}),
      }));
      json(res, 200, { sessionId: sid, source: 'harness', known: true, skills });
    })
    // A harness that cannot answer in time is UNKNOWN, not empty: an empty catalogue
    // is a claim ("it has no skills") and a timeout is not evidence for it. jouzu, for
    // instance, finishes its own startup lazily — measured once at 70s and once at 9s
    // for the first call, then milliseconds. Calling again after it is warm works.
    .catch((e) => json(res, 200, { sessionId: sid, source: 'harness', known: false, reason: e.message, skills: [] }));
}

function createProvider(b, res) {
  if (rejectUnknownFields(res, b, ['id', 'label', 'url', 'api', 'token', 'declarations', 'providerType', 'providerTypeVersion'], 'POST /v1/hub/providers')) return;
  const type = findProviderType(b);
  if (!type) return fail(res, 400, 'validation_failed', 'provider type or version is unavailable');
  const rows = loadProviders();
  const id = b.id || `p-${crypto.randomBytes(4).toString('hex')}`;
  if (rows.some((r) => r.id === id)) return fail(res, 409, 'already_exists', 'provider id already exists');
  const configured = type.configure(b);
  const now = new Date().toISOString();
  const row = {
    id, label: b.label || '',
    providerType: type.descriptor.id, providerTypeVersion: type.descriptor.version,
    // What this provider is: where it lives, and what it speaks. `api` is the
    // harness vocabulary for the wire protocol (openai-completions,
    // anthropic-messages, ...); the adapter that writes the harness's own
    // provider entry is the one that validates it.
    endpoint: { url: configured?.url ?? null, api: configured?.api ?? null },
    ...(typeof configured?.catalogUrl === 'string' && configured.catalogUrl ? { catalogUrl: configured.catalogUrl } : {}),
    models: null, selection: null, catalog: null, endpointRevision: 0,
    createdAt: now, updatedAt: now,
  };
  if (b.declarations !== undefined) {
    const checked = checkDeclarations(b.declarations);
    if (checked.error) return fail(res, 400, 'validation_failed', checked.error);
    row.models = checked.value;
  }
  if (b.token) storeSecret(secretName(id), b.token);
  rows.push(row);
  persistProviders(rows);
  json(res, 200, { provider: providerValueFree(row) });
}

function updateProvider(id, b, res) {
  if (rejectUnknownFields(res, b, ['label', 'url', 'api', 'token', 'declarations'], 'PATCH /v1/hub/providers/{id}')) return;
  const rows = loadProviders();
  const row = rows.find((r) => r.id === id);
  if (!row) return fail(res, 404, 'provider_not_found', 'no such provider');
  if (!findProviderType(row)) return fail(res, 501, 'unsupported', 'provider type or version is unavailable');
  const configured = requireProviderType(row).configure(b, row.endpoint);
  const endpoint = { url: configured?.url ?? null, api: configured?.api ?? null };
  if (typeof configured?.catalogUrl === 'string' && configured.catalogUrl) row.catalogUrl = configured.catalogUrl;
  if (b.label !== undefined) row.label = b.label;
  if (b.declarations !== undefined) {
    const checked = checkDeclarations(b.declarations);
    if (checked.error) return fail(res, 400, 'validation_failed', checked.error);
    row.models = checked.value;
  }
  if (endpoint.url !== row.endpoint?.url || endpoint.api !== row.endpoint?.api || b.token) row.endpointRevision = (row.endpointRevision || 0) + 1;
  row.endpoint = endpoint;
  if (b.token) storeSecret(secretName(id), b.token);
  row.updatedAt = new Date().toISOString();
  persistProviders(rows);
  json(res, 200, { provider: providerValueFree(row) });
}

function deleteProvider(id, res) {
  const rows = loadProviders();
  const row = rows.find((r) => r.id === id);
  if (!row) return fail(res, 404, 'provider_not_found', 'no such provider');
  deleteSecret(secretName(id));
  persistProviders(rows.filter((r) => r.id !== id));
  fs.rmSync(providerFile(id), { force: true });   // the one place a provider file is removed
  json(res, 200, { ok: true, id });
}

async function logoutProvider(id, res) {
  const rows = loadProviders();
  const row = rows.find((r) => r.id === id);
  if (!row) return fail(res, 404, 'provider_not_found', 'no such provider');
  const credential = getSecret(secretName(id));
  deleteSecret(secretName(id));   // drop the token, keep the provider
  row.endpointRevision = (row.endpointRevision || 0) + 1;
  row.updatedAt = new Date().toISOString();
  persistProviders(rows);
  // A plugin that can revoke remotely gets the chance; a failed revocation is
  // reported, never allowed to undo the local removal or fail the request.
  const plugin = findProviderType(row);
  if (credential && plugin && typeof plugin.authLogout === 'function') {
    try { await plugin.authLogout({ providerId: id, credential }); }
    catch (e) { process.stderr.write(`[hub] provider '${id}' logout: remote revocation failed: ${e && e.message ? e.message : e}
`); }
  }
  json(res, 200, { provider: providerValueFree(row) });
}

// --- hub-level authorization operations --------------------------------------
// The plugin owns the vendor flow; the HUB owns the operation: it is created and
// tracked here, survives the caller leaving, and settles the credential into the
// hub secret store so the existing per-session grant reaches every harness. The
// caller only renders the steps it is handed.
const providerAuthOps = new Map();   // operationId -> {id, providerId, state, next, error, abort, startedAt}
function startProviderAuth(id, res) {
  const row = loadProviders().find((r) => r.id === id);
  if (!row) return fail(res, 404, 'provider_not_found', 'no such provider');
  const plugin = findProviderType(row);
  if (!plugin) return fail(res, 501, 'unsupported', 'provider type or version is unavailable');
  const methods = Array.isArray(plugin.descriptor.authMethods) ? plugin.descriptor.authMethods : [];
  if (typeof plugin.beginAuth !== 'function' || !methods.some((m) => m !== 'api-key')) {
    return fail(res, 501, 'unsupported', 'this provider type has no authorization flow');
  }
  if ([...providerAuthOps.values()].some((op) => op.providerId === id && op.state === 'pending')) {
    const existing = [...providerAuthOps.values()].find((op) => op.providerId === id && op.state === 'pending');
    return json(res, 200, { operationId: existing.id, ...(existing.next ? { next: existing.next } : {}) , pending: true });
  }
  const op = { id: `op-${crypto.randomBytes(6).toString('hex')}`, providerId: id, state: 'pending', next: null, error: null, abort: new AbortController(), startedAt: new Date().toISOString() };
  providerAuthOps.set(op.id, op);
  const report = (info) => { if (!op.next && info && typeof info === 'object') op.next = info; };
  Promise.resolve()
    .then(() => plugin.beginAuth({ providerId: id, label: row.label || id, report, signal: op.abort.signal, endpoint: row.endpoint || null }))
    .then((r) => {
      const credential = r && typeof r.credential === 'string' && r.credential ? r.credential : null;
      if (!credential) throw Object.assign(new Error('the authorization finished without a credential'), { code: 'validation_failed' });
      const rows = loadProviders();
      const current = rows.find((x) => x.id === id);
      if (!current) throw Object.assign(new Error('the provider was removed while it was authorizing'), { code: 'provider_not_found' });
      storeSecret(secretName(id), credential);
      const url = r.endpoint && typeof r.endpoint.url === 'string' && r.endpoint.url ? r.endpoint.url : null;
      const api = r.endpoint && typeof r.endpoint.api === 'string' && r.endpoint.api ? r.endpoint.api : null;
      const catalogUrl = typeof r.catalogUrl === 'string' && r.catalogUrl ? r.catalogUrl : null;
      if (catalogUrl) current.catalogUrl = catalogUrl;
      if (url || api || catalogUrl) {
        current.endpoint = { url: url || current.endpoint?.url || null, api: api || current.endpoint?.api || null };
        current.endpointRevision = (current.endpointRevision || 0) + 1;
        current.updatedAt = new Date().toISOString();
        persistProviders(rows);
      }
      op.state = 'approved';
      if (r.account !== undefined) op.account = r.account;
    })
    .catch((e) => {
      if (op.abort.signal.aborted || (e && e.name === 'AbortError')) { op.state = 'cancelled'; return; }
      op.state = 'failed';
      op.error = e && e.message ? e.message : String(e);
    });
  // The caller gets the first declared step; the flow keeps running in the hub.
  const deadline = Date.now() + 20000;
  return (async () => {
    while (Date.now() < deadline) {
      if (op.next) return json(res, 200, { operationId: op.id, next: op.next });
      if (op.state !== 'pending') {
        return op.state === 'failed'
          ? fail(res, 502, 'upstream_blocked', op.error || 'authorization failed')
          : fail(res, 400, 'validation_failed', 'the authorization ended before it produced steps');
      }
      await new Promise((r) => setTimeout(r, 100));
    }
    op.abort.abort();
    op.state = 'failed';
    op.error = 'the authorization did not produce steps in time';
    return fail(res, 502, 'upstream_blocked', op.error);
  })();
}
function providerAuthStatus(id, opId, res) {
  const op = providerAuthOps.get(opId);
  if (!op || op.providerId !== id) return fail(res, 404, 'not_found', 'no such authorization operation');
  return json(res, 200, { operationId: op.id, status: op.state, ...(op.next ? { next: op.next } : {}), ...(op.error ? { error: op.error } : {}), ...(op.account !== undefined ? { account: op.account } : {}) });
}
function providerAuthCancel(id, opId, res) {
  const op = providerAuthOps.get(opId);
  if (op && op.providerId === id && op.state === 'pending') { op.abort.abort(); op.state = 'cancelled'; }
  return json(res, 200, { ok: true, id: opId });   // idempotent
}

// ---------------------------------------------------------------------------
// skills domain (S4, core side): folder + byte-level round-trip; path escape
// rejected at config time. Materialization to adapters happens in S4 adapter
// work; here we own the truth and the UHP red lines.
// ---------------------------------------------------------------------------
const SKILLS_DIR = path.join(DATA_DIR, 'skills');

// A skill id names a directory inside SKILLS_DIR, never a path. The `files`
// branch checked for escape; the DELETE branch did not, so a request like
// `/v1/hub/skills/..%2f..%2fvictim` removed a directory outside SKILLS_DIR.
function skillDir(skillId) {
  if (typeof skillId !== 'string' || !skillId || skillId === '.' || skillId === '..' || skillId.includes('/') || skillId.includes(String.fromCharCode(92)) || skillId.includes(':')) {
    throw Object.assign(new Error('skill id must be a plain directory name'), { code: 'validation_failed' });
  }
  return path.join(SKILLS_DIR, skillId);
}
function skillPath(skillId, rel) {
  const base = skillDir(skillId);
  const target = path.normalize(path.join(base, rel || ''));
  if (target !== base && !target.startsWith(base + path.sep)) {
    throw Object.assign(new Error('path escape'), { code: 'validation_failed' });
  }
  return target;
}
function listSkills() {
  if (!fs.existsSync(SKILLS_DIR)) return [];
  return fs.readdirSync(SKILLS_DIR).map((id) => ({
    id,
    hasManifest: fs.existsSync(path.join(SKILLS_DIR, id, 'SKILL.md')),
    files: fs.readdirSync(path.join(SKILLS_DIR, id)).length,
  }));
}

// ---------------------------------------------------------------------------
// materialization (S4). core is a courier, not an interpreter (A-4): it hands
// the enabled skills (as byte-level content) and enabled connections (scheme +
// endpoint + envName, credential delivered only via env — A-2/§6) to the adapter inside
// config/set. The adapter translates them into its native dialect; the word
// 'mcp' only lives inside an adapter. Disabled connections are omitted
// (disable == cut-off == zero materialization).
// ---------------------------------------------------------------------------
function readBundle(baseDir, id) {
  const base = path.join(baseDir, id);
  const files = {};
  const walk = (dir) => {
    for (const name of fs.readdirSync(dir)) {
      const full = path.join(dir, name);
      if (fs.statSync(full).isDirectory()) walk(full);
      else files[path.relative(base, full).split(path.sep).join('/')] = fs.readFileSync(full, 'utf8');
    }
  };
  walk(base);
  return files;
}
// What a session carries into its harness. The hub assembles it from its own
// registries — the harness's installed extensions, its enabled skills, the
// enabled connections — and hands it over whole; the adapter only places it
// where its harness reads it. This is the hub's installing job: an adapter
// never chooses what to install, so adding an extension to a harness is a
// registry change, not a code change in three adapters.
function copyTree(src, dst) {
  fs.mkdirSync(dst, { recursive: true });
  for (const name of fs.readdirSync(src)) {
    const from = path.join(src, name);
    const to = path.join(dst, name);
    if (fs.statSync(from).isDirectory()) copyTree(from, to);
    else fs.copyFileSync(from, to);
  }
}
// The hub installs a harness's extensions: the registry says which, and the hub
// writes exactly those extension directories into that harness's own data dir.
// The set is exact each time — an extension removed from the registry is removed
// from the install — and the adapter only places what it finds there into the
// layout its harness reads.
function installExtensions(harnessId) {
  const dir = path.join(DATA_DIR, 'agents', harnessId, 'extensions');
  fs.rmSync(dir, { recursive: true, force: true });
  fs.mkdirSync(dir, { recursive: true });
  const row = harnessRow(harnessId);
  const wanted = row && Array.isArray(row.extensions) ? row.extensions : [];
  const available = availableExtensions(harnessId);
  const installed = [];
  for (const id of wanted.filter((e) => available.includes(e))) {
    copyTree(path.join(pluginExtensionsDir(harnessId), id), path.join(dir, id));
    installed.push(id);
  }
  return { dir, installed };
}
// Skills are installed the same way extensions are: the registry says which, the hub
// writes exactly those directories into that harness's own data dir, and the adapter
// only points its harness at the result (pi/jouzu: --skill; dsh: DSH_AGENTS_HOME).
// The set is exact each time — a skill removed from the registry is removed from the
// install — and nothing is written near the user's own skill directories.
function installSkills(harnessId) {
  const dir = path.join(DATA_DIR, 'agents', harnessId, 'skills');
  fs.rmSync(dir, { recursive: true, force: true });
  fs.mkdirSync(dir, { recursive: true });
  const held = fs.existsSync(SKILLS_DIR) ? fs.readdirSync(SKILLS_DIR) : [];
  const row = harnessRow(harnessId);
  const wanted = row && Array.isArray(row.skills) ? row.skills : held;   // null/absent = all
  const installed = [];
  for (const id of wanted.filter((s) => held.includes(s))) {
    copyTree(path.join(SKILLS_DIR, id), path.join(dir, id));
    installed.push(id);
  }
  return { dir, installed };
}
function buildMaterialization(harnessId) {
  const connections = loadConnections()
    .filter((c) => c.state !== 'disabled')
    .map(({ id, name, scheme, endpoint, envName }) => ({ id, name, scheme, endpoint, envName }));
  // Skills are NOT here: the hub installs them into the harness's own directory
  // before its process starts (installSkills), exactly the way extensions travel.
  // One path, one source — the alternative was two ways to be installed and a
  // contract that could disagree with itself.
  return { connections };
}

// ---------------------------------------------------------------------------
// connection domain (S4, connection-only model). A connection is a managed
// object: enable/disable/delete. disable == cut-off == zero materialization.
// scheme is a free string (stdio/http/https/ws/wss/...), no whitelist.
// ---------------------------------------------------------------------------
const CONNECTIONS_FILE = path.join(DATA_DIR, 'connections.json');
function loadConnections() { return readStateJson(CONNECTIONS_FILE, []); }
function persistConnections(rows) { writeJson(CONNECTIONS_FILE, rows); }
function connectionValueFree(row) {
  const { secretRef, ...rest } = row;
  return { ...rest, credentialConfigured: Boolean(secretRef) };
}

// FD-4: the value to grant a session adapter, if any. The session's
// connectionId names either a core connection row or a core provider row; its
// token is what gets granted. Unknown / no token -> null (the harness's own
// login is its business).
// Returns the credential a session's model provider can be fed: {value, url}
// plus the provider's model declarations, if it has any. value = the token; url =
// the provider's endpoint (present only for a core provider row — a hub-managed
// upstream). A provider row is url+token; both must reach the adapter for
// injection to be possible. Unknown / no token -> null (the harness's own login
// is its business). The declarations ride along because the core is their source:
// a harness cannot learn levels from the endpoint, so the ones the core holds are
// the ones the harness gets.
function connectionCredential(harnessId, connectionId) {
  if (!connectionId) return null;
  const pv = getSecret(secretName(connectionId));
  if (pv != null) {
    const row = loadProviders().find((r) => r.id === connectionId);
    if (row) requireProviderType(row);
    return { value: pv, url: row ? (row.endpoint?.url || null) : null, api: row ? (row.endpoint?.api || null) : null, models: row ? effectiveModels(row) : null };
  }
  const c = loadConnections().find((r) => r.id === connectionId);
  if (c && c.secretRef) { const v = getSecret(c.secretRef); if (v != null) return { value: v, url: null, declarations: null }; }
  return null;
}

// §6: a connection's token reaches the adapter as an env var, never in the
// config/set payload. Only enabled connections are injected (disabled == cut-off).
function connectionEnv() {
  const env = {};
  for (const c of loadConnections()) {
    if (c.state === 'disabled' || !c.envName || !c.secretRef) continue;
    const value = getSecret(c.secretRef);
    if (value != null) env[c.envName] = value;
  }
  return env;
}

function createConnection(b, res) {
  const rows = loadConnections();
  const id = b.id || `c-${crypto.randomBytes(4).toString('hex')}`;
  if (rows.some((r) => r.id === id)) return fail(res, 409, 'already_exists', 'connection id already exists');
  const now = new Date().toISOString();
  // The token is stored in the SecretStore and handed to the adapter as an ENV
  // VAR (never in the config/set payload — §6, keeps it out of conversation
  // history). envName is what the adapter reads (e.g. MY_API_TOKEN).
  let secretRef = null;
  if (b.token) {
    secretRef = `connection-${id}-token`;
    storeSecret(secretRef, b.token);
  }
  const row = {
    id, name: b.name || id, scheme: b.scheme || 'stdio', endpoint: b.endpoint || '',
    envName: b.envName || null, secretRef,
    state: b.state === 'disabled' ? 'disabled' : 'enabled',
    managed: true, createdAt: now, updatedAt: now,
  };
  rows.push(row);
  persistConnections(rows);
  json(res, 200, { connection: connectionValueFree(row) });
}

function updateConnection(id, b, res) {
  const rows = loadConnections();
  const row = rows.find((r) => r.id === id);
  if (!row) return fail(res, 404, 'connection_not_found', 'no such connection');
  if (b.name !== undefined) row.name = b.name;
  if (b.scheme !== undefined) row.scheme = b.scheme;
  if (b.endpoint !== undefined) row.endpoint = b.endpoint;
  if (b.envName !== undefined) row.envName = b.envName;
  if (b.state === 'enabled' || b.state === 'disabled') row.state = b.state;
  if (b.token) {
    row.secretRef = `connection-${id}-token`;
    storeSecret(row.secretRef, b.token);
  }
  row.updatedAt = new Date().toISOString();
  persistConnections(rows);
  json(res, 200, { connection: connectionValueFree(row) });
}

function deleteConnection(id, res) {
  const rows = loadConnections();
  const row = rows.find((r) => r.id === id);
  if (!row) return fail(res, 404, 'connection_not_found', 'no such connection');
  if (row.secretRef) deleteSecret(row.secretRef);
  persistConnections(rows.filter((r) => r.id !== id));
  json(res, 200, { ok: true, id });
}

// Session-process initialization (used at create AND at every respawn). Contract
// order: session/start (or resume) -> credentials/grant (FD-4) -> config/set. A
// process is only READY once all three succeed. The in-flight promise is shared
// so concurrent callers never double-start or skip grant/config. Initialization
// failure leaves the process unusable (caller must not prompt).
function initAdapter(conn, s, { modelProviderId, modelId, presetId, plan, review, thinkingLevel, title = null, freshSession = false, fork = null } = {}) {
  if (conn.ready) return Promise.resolve();
  if (conn.initializing) return conn.initializing;   // share the in-flight init
  // A harness whose runtime is not on disk yet: the plugin installs it, through its
  // own method, and this is where the hub asks. Doing it here rather than at spawn
  // means every path that needs a started adapter (a session, a read-back, a resume)
  // gets it, and the adapter process itself needs no runtime to answer — which is
  // exactly why the answer can be the adapter's own.
  // freshSession is only ever set by repair, for the case where the harness has
  // no session artifact to resume (the cancelled turn never created one). It
  // opens the session anew and REPLACES the ref, so the session continues as a
  // clean thread rather than failing to open a file that was never written.
  const resumeRef = freshSession ? null : (s.ref || null);
  const strip = (/** @type {any} */ extra) => Object.assign(extra, buildMaterialization(s.harnessId));
  // A forked session opens by FORKING: this process is the child, the hub tells it
  // where to branch from, and the ref that comes back is the child's own. The
  // source session is never touched, so no adapter of the source needs to be
  // running — the child's process reads the source itself.
  conn.initializing = ensureRuntime(s.harnessId, conn)
    .then(() => (fork
    ? rpc(conn, 'session/fork', { sid: s.id, from: fork.from, ...(fork.throughTurn !== undefined ? { throughTurn: fork.throughTurn } : {}) }, TURN_TIMEOUT)
    : rpc(conn, 'session/start', { sid: s.id, ...(resumeRef ? { resume: resumeRef } : {}) }, TURN_TIMEOUT)))
    .then((r) => {
      s.ref = r.ref;
      // The harness's own answer on additionalDirectories: {requested, applied,
      // supported, reason}. The core passes the roots through and records what
      // the harness said it did with them — it does not assume they are active,
      // and it does not decide for the harness whether they are supported.
      if (r.additionalDirectories !== undefined) s.appliedAdditionalDirectories = r.additionalDirectories;
      const cid = modelProviderId ?? s.modelProviderId;
      // A hub-managed provider was named but its token is not available: this
      // must fail, not fall through. Skipping the grant is what let a session run
      // on the harness's native provider while the caller believed it had chosen
      // one — a silent substitution, the same class as the T0 model-fallback
      // guard.
      //
      // Only a MANAGED provider can be checked here. A harness's own native
      // provider (the ids in its /models catalog) carries no hub credential by
      // definition — the harness authenticates itself — so naming one is not a
      // claim that the hub would fund it, and refusing it would break ordinary use.
      const isManaged = cid != null && loadProviders().some((r) => r.id === cid);
      if (isManaged && connectionCredential(s.harnessId, cid) == null) {
        throw Object.assign(
          new Error(`provider '${cid}' has no stored credential; refusing to fall back to the harness's own provider`),
          { code: 'provider_unauthorized' },
        );
      }
      const cred = connectionCredential(s.harnessId, cid);
      const granted = cred == null
        ? Promise.resolve()
        : rpc(conn, 'credentials/grant', { connectionId: cid, value: cred.value, ...(cred.url ? { url: cred.url } : {}), ...(cred.api ? { api: cred.api } : {}), ...(cred.models ? { models: cred.models } : {}) }, TURN_TIMEOUT).then(() => {});
      return granted.then(() => {
        const cfg = {};
        if (cid) cfg.connectionId = cid;
        const mid = modelId ?? s.modelId;
        if (mid) cfg.model = mid;
        const pst = presetId ?? s.presetId;   // resume must keep the persisted preset
        if (pst) cfg.presetId = pst;
        const pln = plan ?? s.plan;   // resume must keep the persisted plan state
        if (pln !== null && pln !== undefined) cfg.plan = pln;
        const rvw = review ?? s.review;   // resume must keep the persisted review state
        if (rvw !== null && rvw !== undefined) cfg.review = rvw;
        const thl = thinkingLevel ?? s.thinkingLevel;   // resume must keep the persisted level
        if (thl !== null && thl !== undefined && thl !== '') cfg.thinkingLevel = thl;
        strip(cfg);
        return rpc(conn, 'config/set', { sid: s.id, config: cfg }, TURN_TIMEOUT).then((applied) => {
          // T0 guard: whatever the adapter reports as APPLIED must equal what we
          // asked for. A silent fallback to another model/provider is the worst
          // failure mode; it fails loudly here, never passes as success.
          const a = applied && applied.applied;
          if (mid && a && typeof a.model === 'string' && a.model !== mid) {
            throw Object.assign(new Error(`model fallback: asked ${mid}, adapter applied ${a.model}`), { code: 'model_mismatch' });
          }
          if (a) {
            s.appliedModel = a.model != null ? a.model : null;
            s.appliedProviderId = a.connectionId != null ? a.connectionId : null;
            if (a.preset !== undefined) s.appliedPreset = a.preset != null ? a.preset : null;
            if (a.plan !== undefined) s.appliedPlan = a.plan != null ? a.plan : null;
            if (a.review !== undefined) s.appliedReview = a.review != null ? a.review : null;
            if (a.thinkingLevel !== undefined) s.appliedThinkingLevel = a.thinkingLevel != null ? String(a.thinkingLevel) : null;
          } else {
            s.appliedModel = null;
            s.appliedProviderId = null;
          }
          // A title is a name the HARNESS shows in its own listings too, so it is
          // pushed there when the session starts. A harness that cannot rename
          // keeps the hub's title and says so in `warning` — the caller must not
          // have to discover that the other side never heard the name.
          const startTitle = title ?? s.title;
          if (!startTitle) return undefined;
          const caps = (manifestOf(s.harnessId) || {}).capabilities || [];
          if (!caps.includes('rename')) {
            s.warning = `harness '${s.harnessId}' declares no rename capability: the title stays in the hub`;
            return undefined;
          }
          return rpc(conn, 'session/rename', { sid: s.id, title: startTitle }, TURN_TIMEOUT)
            .then((r) => { s.appliedTitle = r && typeof r.title === 'string' ? r.title : null; })
            .catch((e) => { s.appliedTitle = null; s.warning = `title not applied by the harness: ${e.message}`; });
        });
      });
    })
    .then(() => { conn.ready = true; conn.started = true; })
    .catch((e) => {
      // Not ready: drop the process, keep the session record (with its ref) for
      // a later retry, and surface the error. Never leave a half-init process.
      conn.ready = false;
      conn.started = false;
      if (adapters.get(s.id) === conn) adapters.delete(s.id);
      try { conn.proc.kill(); } catch {}
      throw e;
    })
    .finally(() => { conn.initializing = null; });
  return conn.initializing;
}

// A session runs in one working directory. There is no core concept of a
// "workspace": naming and grouping directories is the caller's business, and a
// caller only has to hand us a cwd. The one thing that IS the core's job is that
// a session always has a directory — so an omitted cwd gets one allocated here,
// under the session's own id.


// A request field nobody reads is a request that did not happen. Accepting an
// unknown field means answering 200 to a caller whose knob was never set — the
// same silent downgrade the hub refuses everywhere else. Every hand-written body
// goes through here, and the message names what would have been accepted.
function rejectUnknownFields(res, body, allowed, route) {
  if (!body || typeof body !== 'object' || Array.isArray(body)) return false;
  const unknown = Object.keys(body).filter((k) => !allowed.includes(k));
  if (!unknown.length) return false;
  fail(res, 400, 'validation_failed',
    `${route}: unknown field${unknown.length > 1 ? 's' : ''} ${unknown.map((k) => `'${k}'`).join(', ')}; accepts ${allowed.map((k) => `'${k}'`).join(', ')}`);
  return true;
}

function createSession(b, res) {
  if (rejectUnknownFields(res, b, ['harnessId', 'modelProviderId', 'connectionId', 'modelId', 'title', 'presetId', 'plan', 'review', 'thinkingLevel', 'cwd', 'additionalDirectories', ], 'POST /v1/sessions')) return;
  const { harnessId, modelProviderId = null, connectionId = null, modelId = null, title = null, presetId = null, plan = null, review = null, thinkingLevel = null, cwd = null, additionalDirectories = null } = b;
  // modelProviderId = which model provider (domain 3) feeds this session.
  // Optional: null means "don't intervene" — the harness uses its own native
  // config (never refused; S-1). `connectionId` is accepted as a legacy alias.
  const mpid = canonicalProviderId(modelProviderId ?? connectionId ?? null);
  if (!harnessId || !manifestOf(harnessId)) return fail(res, 404, 'harness_not_found', 'no such harness');
  if (!isHarnessEnabled(harnessId)) return fail(res, 409, 'harness_disabled', `harness ${harnessId} is deactivated; activate it before use`);
  const id = `s-${process.pid}-${sessions.size + 1}-${crypto.randomBytes(3).toString('hex')}`;
  // Directory ownership belongs to core, independently of credential scope.
  // Never run a directory-less UI session in the server/repository cwd.
  if (cwd != null && typeof cwd !== 'string') return fail(res, 400, 'validation_failed', 'cwd must be a directory path');
  let workingDirectory;
  if (typeof cwd === 'string' && cwd.trim()) {
    if (!path.isAbsolute(cwd)) return fail(res, 400, 'validation_failed', 'cwd must be an absolute directory path');
    try {
      workingDirectory = fs.realpathSync(cwd);
      if (!fs.statSync(workingDirectory).isDirectory()) throw new Error('not a directory');
    } catch { return fail(res, 400, 'validation_failed', 'cwd must be an existing directory'); }
  } else {
    // No cwd from the caller: a session must still have a directory, and it must
    // never be the server/repo cwd. Allocate one under the session's own id.
    workingDirectory = path.join(DATA_DIR, 'sessions', id);
    fs.mkdirSync(workingDirectory, { recursive: true, mode: 0o700 });
    workingDirectory = fs.realpathSync(workingDirectory);
  }
  // additionalDirectories are extra roots the harness MAY activate (ACP's field
  // of that name; they expand the filesystem scope without changing cwd, which
  // stays the base for relative paths). Whether a harness supports them is the
  // harness's business — the core passes them through and does not refuse or
  // silently drop them. Empty/absent means none.
  let extraDirs = null;
  if (additionalDirectories != null) {
    if (!Array.isArray(additionalDirectories)) return fail(res, 400, 'validation_failed', 'additionalDirectories must be an array of absolute paths');
    const cleaned = [];
    for (const d of additionalDirectories) {
      if (typeof d !== 'string' || !path.isAbsolute(d)) return fail(res, 400, 'validation_failed', 'additionalDirectories entries must be absolute paths');
      try { cleaned.push(fs.realpathSync(d)); } catch { return fail(res, 400, 'validation_failed', `additionalDirectories entry does not exist: ${d}`); }
    }
    extraDirs = cleaned;
  }
  // The same rule as a mid-session switch: a provider and a model must come from
  // the same place. A provider the core owns declares its models, so a model it
  // does not offer is refused now rather than at the first turn.
  if (mpid && modelId) {
    const prov = loadProviders().find((p) => p.id === mpid);
    if (prov && !findProviderType(prov)) return fail(res, 501, 'unsupported', 'provider type or version is unavailable');
    const declared = prov && prov.declarations && typeof prov.declarations === 'object' ? Object.keys(prov.declarations) : null;
    if (declared && declared.length && !declared.includes(modelId)) {
      return fail(res, 400, 'validation_failed', `model '${modelId}' is not offered by provider '${mpid}'`);
    }
  }
  const s = {
    id, harnessId, modelProviderId: mpid, modelId, title, presetId, plan, review, thinkingLevel: typeof thinkingLevel === 'string' && thinkingLevel ? thinkingLevel : null, cwd: workingDirectory, additionalDirectories: extraDirs,
    // Tool selection is a knob with an asked half and an applied half, like the
    // others: disabledTools is the listed set, appliedTools the set the harness
    // reported it actually has disabled. null = not asked / not observed.
    disabledTools: Array.isArray(b.disabledTools) ? b.disabledTools : null,
    appliedTools: null,
    thinkingLevel: null,
    appliedThinkingLevel: null,
    // What the harness reported it did with additionalDirectories
    // ({requested, applied, supported, reason}); null until the harness answers.
    appliedAdditionalDirectories: null,
    appliedModel: null,
    appliedProviderId: null,
    appliedPreset: null,
    appliedPlan: null,
    appliedReview: null,
    status: 'active',
    activeTurn: { state: 'idle', ended: null, cause: null, partialPersisted: false, partialItems: 0 },
    createdAt: new Date().toISOString(),
    updatedAt: new Date().toISOString(),
  };
  sessions.set(id, s);

  const conn = spawnAdapter(id, harnessId, s.cwd, s.additionalDirectories);
  return initAdapter(conn, s, { modelProviderId: mpid, modelId, presetId, plan, review, thinkingLevel, title })
    .then(() => {
      persistSessions();
      json(res, 200, { session: sessionValue(s) });
    })
    .catch((e) => {
      sessions.delete(id);
      try { conn.proc.kill(); } catch {}
      // A missing credential is the caller's problem to fix, not a harness that
      // could not be reached. Keep the code distinct so it is actionable.
      if (e && e.code === 'provider_unauthorized') return fail(res, 400, 'provider_unauthorized', e.message);
      fail(res, 502, 'adapter_unreachable', e.message);
    });
}

// Session statistics: tokens, cost, context-window occupancy — whatever the
// harness reports, passed through unchanged (`source: 'harness'` states where it
// came from). A number the harness does not report is ABSENT, never zero.
//
// Shape: one PULL route plus the same object riding the turn's own stream after
// `turn.ended`, because that is the moment the numbers move and the caller is
// already reading that stream. There is deliberately no stats stream: dsh pushes
// projection updates, pi and jouzu push nothing, and a hub-made "live" stream on
// them would be polling wearing a subscription's clothes.
function readStats(sid, s, res) {
  const caps = (manifestOf(s.harnessId) || {}).capabilities || [];
  if (!caps.includes('stats')) {
    return fail(res, 501, 'unsupported', `harness '${s.harnessId}' does not report session statistics`);
  }
  let conn = connFor(sid);
  if (!conn) {
    if (!s.ref) return fail(res, 502, 'adapter_unreachable', 'adapter not running; session has no resume ref');
    conn = spawnAdapter(sid, s.harnessId, s.cwd, s.additionalDirectories);
  }
  return initAdapter(conn, s, {})
    .then(() => rpc(conn, 'session/stats', { sid }, 15_000))
    .then((stats) => json(res, 200, { sessionId: sid, source: 'harness', ...stats }))
    .catch((e) => failError(res, e));
}

// Compaction: the harness rewrites its own conversation (summarise an old span
// into one node) because its context is filling. The hub compacts nothing itself
// and computes none of the numbers — it asks, and reports what the harness said
// (`source: 'harness'`). Which numbers exist depends on the harness: pi counts
// the replaced span and then ESTIMATES the rebuilt context; dsh counts the span
// it replaced and reports no "after" — so tokensAfter is absent there rather
// than a hub-made guess (a zero and an unmeasured value are different claims).
//
// Refused while a turn is running: pi's compact aborts the running turn, and a
// maintenance call must not silently throw away in-flight work. And a harness
// that cannot compact answers 501 — never a fake success.
const COMPACT_TIMEOUT_MS = 300_000;
function compactSession(sid, b, res) {
  if (rejectUnknownFields(res, b, ['instructions'], 'POST /v1/sessions/{id}/compact')) return;
  const s = sessions.get(sid);
  if (!s) return fail(res, 404, 'unknown_session', 'no such session; never silently created');
  if (s.deleted || s.status === 'closed') return fail(res, 409, 'session_closed', 'reopen the session before compacting it');
  if (turnIsOpen(s)) {
    return fail(res, 409, 'session_busy', 'a running turn is left alone; compact after it ends');
  }
  if (s.status === 'needs-repair') return fail(res, 409, 'needs_repair', 'repair the session before compacting it');
  const caps = (manifestOf(s.harnessId) || {}).capabilities || [];
  if (!caps.includes('compact')) {
    return fail(res, 501, 'unsupported', `harness '${s.harnessId}' cannot compact its conversation`);
  }
  let conn = connFor(sid);
  if (!conn) {
    if (!s.ref) return fail(res, 502, 'adapter_unreachable', 'adapter not running; session has no resume ref');
    conn = spawnAdapter(sid, s.harnessId, s.cwd, s.additionalDirectories);
  }
  const instructions = b && typeof b.instructions === 'string' && b.instructions ? b.instructions : null;
  return initAdapter(conn, s, {})
    .then(() => rpc(conn, 'session/compact', instructions ? { sid, instructions } : { sid }, COMPACT_TIMEOUT_MS))
    .then((r) => json(res, 200, { sessionId: sid, source: 'harness', ...r }))
    .catch((e) => failError(res, e));
}

// Close a turn's stream: emit whatever the harness can say about the turn that
// just ended, then end it. There are two natural closers — the adapter's
// `turn_end` event and the reply to `session/prompt` — and both go through here,
// memoised per turn, so the numbers are fetched once and neither closer can cut
// the stream before they arrive (which is exactly what happened first: the prompt
// reply ended the stream while the stats were still in flight).
function closeTurnStream(sid, conn) {
  const end = () => { if (conn && conn.streamRes && !conn.streamRes.writableEnded) conn.streamRes.end(); };
  if (!conn) return Promise.resolve();
  if (conn.turnClose) return conn.turnClose;
  const s = sessions.get(sid);
  const caps = s ? ((manifestOf(s.harnessId) || {}).capabilities || []) : [];
  const stats = caps.includes('stats')
    // Bounded and non-fatal: a harness that cannot answer in time must not hold a
    // finished turn open.
    ? rpc(conn, 'session/stats', { sid }, 5_000)
      .then((v) => emitSessionEvent(sid, 'session.stats', { sessionId: sid, source: 'harness', ...v }))
      .catch((e) => process.stderr.write(`[hub] stats after turn ${sid}: ${e.message}
`))
    : Promise.resolve();
  conn.turnClose = stats.then(() => { conn.turnClose = null; end(); });
  return conn.turnClose;
}

// A fork is a NEW session whose conversation ends where the caller says; the
// source session is not changed (its log, its process and its ref stay as they
// were — measured for both harnesses before this was written). The anchor is a
// COMPLETED TURN: it is the only cut both harnesses can make exactly (pi forks
// before a user message, dsh snaps to the end of a turn), and it is a thing the
// hub already owns and shows a caller (/turns). Which native anchor that becomes
// is the adapter's business — the hub never speaks entry ids or seq numbers.
//
// `fork` is a declared capability, not an assumption: a harness that cannot fork
// gets `unsupported`, never a silently substituted empty conversation.
function forkSession(sid, b, res) {
  const s = sessions.get(sid);
  if (!s) return fail(res, 404, 'unknown_session', 'no such session; never silently created');
  if (s.deleted || s.status === 'closed') return fail(res, 409, 'session_closed', 'reopen the session before forking it');
  if (turnIsOpen(s)) {
    // A fork into a running turn has no completed-turn boundary to cut at.
    return fail(res, 409, 'session_busy', 'finish or cancel the current turn before forking');
  }
  if (s.status === 'needs-repair') return fail(res, 409, 'needs_repair', 'repair the session before forking it');
  const caps = (manifestOf(s.harnessId) || {}).capabilities || [];
  if (!caps.includes('fork')) {
    return fail(res, 501, 'unsupported', `harness '${s.harnessId}' cannot fork a conversation`);
  }
  const afterTurnId = b && b.afterTurnId !== undefined && b.afterTurnId !== null ? b.afterTurnId : null;
  let throughTurn;
  if (afterTurnId !== null) {
    const idx = (s.turns || []).findIndex((t) => t.turnId === afterTurnId);
    if (idx < 0) return fail(res, 400, 'unknown_turn', `no completed turn '${afterTurnId}' in this session`);
    throughTurn = idx + 1;
  }
  // The source session must have a native identity to branch from; a session that
  // never ran has none.
  if (!s.ref) return fail(res, 409, 'requires_new_session', 'this session has no native history to fork yet');
  const now = new Date().toISOString();
  const id = `s-${process.pid}-${sessions.size + 1}-${crypto.randomBytes(3).toString('hex')}`;
  // The child starts where the parent is: same harness, provider, model,
  // preset/plan/review/level and directory. Its turn log starts empty —
  // the inherited turns belong to the harness, and the child's history is read
  // back from there like any other session's.
  const child = {
    id,
    harnessId: s.harnessId,
    modelProviderId: s.modelProviderId,
    modelId: s.modelId,
    title: s.title,
    presetId: s.presetId,
    plan: s.plan,
    review: s.review,
    thinkingLevel: s.thinkingLevel,
    disabledTools: s.disabledTools,
    appliedTools: null,
    appliedThinkingLevel: null,
    cwd: s.cwd,
    additionalDirectories: s.additionalDirectories,
    appliedAdditionalDirectories: null,
    ref: null,
    status: 'active',
    forkedFrom: { sessionId: sid, afterTurnId },
    activeTurn: { state: 'idle', ended: null, cause: null, partialPersisted: false, partialItems: 0 },
    createdAt: now,
    updatedAt: now,
  };
  sessions.set(id, child);
  const conn = spawnAdapter(id, s.harnessId, child.cwd, child.additionalDirectories);
  return initAdapter(conn, child, {
    modelProviderId: child.modelProviderId,
    modelId: child.modelId,
    presetId: child.presetId,
    plan: child.plan,
    review: child.review,
    thinkingLevel: child.thinkingLevel,
    fork: { from: s.ref, ...(throughTurn !== undefined ? { throughTurn } : {}) },
  })
    .then(() => {
      persistSessions();
      return json(res, 200, { session: Object.assign(child, buildMaterialization(child.harnessId)), forkedFrom: child.forkedFrom });
    })
    .catch((e) => {
      sessions.delete(id);
      try { conn.proc.kill(); } catch {}
      if (e && e.code === 'provider_unauthorized') return fail(res, 400, 'provider_unauthorized', e.message);
      return failError(res, e);
    });
}

function sendTurn(sid, b, req, res) {
  if (rejectUnknownFields(res, b, ['content', 'idempotencyKey', 'source', 'bridge'], 'POST /v1/sessions/{id}/turns')) return;
  const s = sessions.get(sid);
  if (!s) return fail(res, 404, 'unknown_session', 'no such session; never silently created');
  if (configuringSessions.has(sid)) return fail(res, 409, 'session_busy', 'session configuration is in progress');
  if (s.deleted === true) return fail(res, 409, 'session_deleted', 'session was deleted from the list');
  if (s.status === 'closed') return fail(res, 409, 'session_closed', 'session is closed; reopen it before sending');
  if (s.status === 'needs-repair') return fail(res, 409, 'needs_repair', 'orphaned tail requires repair before sending');
  if (!isHarnessEnabled(s.harnessId)) return fail(res, 409, 'harness_disabled', `harness ${s.harnessId} is deactivated; activate it before use`);
  if (turnIsOpen(s)) {
    // C-9: refuse, do not queue; include the current turn state so the caller
    // knows what it collided with.
    return json(res, 409, errorBody('session_busy', 'session has a running turn; sending is refused, not queued', { turn: s.activeTurn }));
  }
  const content = Array.isArray(b.content) ? b.content : [];
  const text = content.filter((c) => c && c.type === 'text').map((c) => c.text).join('');
  const images = content.filter((c) => c && c.type === 'image' && typeof c.data === 'string' && c.data.length)
    .map((c) => ({ mediaType: c.mediaType || 'image/png', data: c.data, ...(c.name ? { name: c.name } : {}) }));
  // A turn is text and/or images; an image-only turn is legitimate.
  if (!text && !images.length) return fail(res, 400, 'validation_failed', 'content must contain text or an image');
  // Provenance: a bridge (client-side orchestrator) marks its injected turns so
  // they are never mistaken for a human. Defaults to a plain user turn.
  const source = b.source === 'bridge' ? 'bridge' : 'user';
  const bridge = b.bridge || null;

  let conn = connFor(sid);
  if (!conn) {
    // The adapter process is gone (restart/crash). Respawn and resume; config
    // and credential are re-established by initAdapter, not just at creation.
    if (!s.ref) return fail(res, 502, 'adapter_unreachable', 'adapter not running; session has no resume ref');
    conn = spawnAdapter(sid, s.harnessId, s.cwd, s.additionalDirectories);
  }
  // Readiness is conn.ready@sid (start+grant+config all done). initAdapter shares
  // any in-flight init so concurrent callers wait for the SAME result.
  const ensureStarted = initAdapter(conn, s, {});

  s.activeTurn = { state: 'admitted', ended: null, cause: null, partialPersisted: false, partialItems: 0 };
  beginTurn(s);
  res.writeHead(200, { 'content-type': 'text/event-stream', 'cache-control': 'no-cache' });
  const sub = (ev) => sseWrite(res, ev);
  conn.onEvent = sub;
  conn.streamRes = res;
  sseWrite(res, { event: 'turn.admitted', data: { turn: s.activeTurn } });

  ensureStarted
    .then(() => rpc(conn, 'session/prompt', { sid, message: text, ...(images.length ? { images } : {}), clientMessageId: b.idempotencyKey || `u-${crypto.randomUUID()}`, source, bridge }, TURN_TIMEOUT))
    .then(() => {
      // The adapter answers the prompt when the turn is over; the turn_end event
      // usually arrived first. Either way the stream closes through the one place
      // that also carries the closing numbers.
      closeTurnStream(sid, conn);
    })
    .catch((e) => {
      // If the adapter-crash handler already set a terminal state (interrupted),
      // do not overwrite it with a generic model-error: the real cause was the
      // process dying, not the model.
      if (s.activeTurn.state === 'ended') {
        if (!res.writableEnded) res.end();
        return;
      }
      s.activeTurn = { state: 'ended', ended: 'failed', cause: 'model-error', partialPersisted: s.activeTurn.partialPersisted, partialItems: 0 };
      endTurn(s, 'failed', 'model-error');
      if (!res.writableEnded) sseWrite(res, { event: 'turn.ended', data: { turn: s.activeTurn, error: e.message } });
      if (!res.writableEnded) res.end();
    });

  req.on('close', () => {
    conn.onEvent = null;
    conn.streamRes = null;
  });
  return undefined; // SSE response handled above
}

// C-3: thin per-turn entries, core-owned (details fetched separately).
function listTurns(sid, res) {
  const s = sessions.get(sid);
  if (!s) return fail(res, 404, 'unknown_session', 'no such session; never silently created');
  return json(res, 200, { turns: s.turns || [], next_cursor: null });
}

// Read-through native message history (adapter history/page). This is the
// "details" C-3 refers to, exposed on its own path so /turns keeps its thin
// contract shape. Reconnects after a core restart (C6). Paging (C5): beforeId
// is the anchor (excluded from the page); an unknown anchor fails loudly with
// invalid_cursor, never a silent rewind. next_cursor is the oldest id on the
// page while more remain, else null.
function readMessages(sid, res, query) {
  const s = sessions.get(sid);
  if (!s) return fail(res, 404, 'unknown_session', 'no such session; never silently created');
  const beforeId = query && query.get('beforeId') ? query.get('beforeId') : undefined;
  const limit = Math.max(1, Number(query && query.get('limit')) || 100);
  const readonly = (manifestOf(s.harnessId)?.capabilities || []).includes('history-readonly');
  let history;
  if (readonly) {
    history = configRpc(s.harnessId, 'history/read', {sid, ref:s.ref, limit, beforeId});
  } else {
    let conn = connFor(sid);
    if (!conn) {
      if (!s.ref) return fail(res, 502, 'adapter_unreachable', 'session has no resume ref');
      conn = spawnAdapter(sid, s.harnessId, s.cwd, s.additionalDirectories);
    }
    history = initAdapter(conn, s, {}).then(() => rpc(conn, 'history/page', {sid,limit,beforeId}));
  }
  return history
    .then((r) => {
      const messages = r.messages || [];
      const next = r.hasMore ? (messages.length ? messages[0].id : null) : null;
      json(res, 200, { messages, next_cursor: next });
    })
    .catch((e) => {
      if (e.data?.code === 'history_unavailable') return fail(res, 502, 'history_unavailable', e.message);
      // adapter signals an unknown anchor; surface the contract's invalid_cursor.
      if (/unknown beforeId/i.test(e.message || '')) return fail(res, 400, 'invalid_cursor', e.message);
      return fail(res, 502, 'adapter_unreachable', e.message);
    });
}

function cancelTurn(sid, res) {
  const s = sessions.get(sid);
  if (!s) return fail(res, 404, 'unknown_session', 'no such session');
  if (s.activeTurn.state === 'ended' || s.activeTurn.state === 'unknown' || s.activeTurn.state === 'idle') {
    return json(res, 200, { turn: s.activeTurn }); // idempotent
  }
  // A cancel already in flight: do not stack timers or re-send the abort.
  if (s.activeTurn.state === 'cancelling') return json(res, 200, { turn: s.activeTurn });
  const conn = connFor(sid);
  if (!conn) return fail(res, 502, 'adapter_unreachable', 'adapter not running');
  s.activeTurn = { ...s.activeTurn, state: 'cancelling' };
  // The turn the user asked to stop. The deadline must be bound to THIS turn:
  // a late turn_end from a previous turn, or from a repair, must not clear a
  // timer that belongs to the one being cancelled now.
  const cancellingTurnId = s.currentTurnId || null;
  // Why we gave up, so `repair` can act on the real cause instead of guessing.
  // 'unconfirmed' = the adapter accepted the abort but the turn never reported
  // its end; 'abort-failed' = the adapter said the abort itself failed.
  let cancelCause = 'unconfirmed';
  const settleCancel = () => {
    clearTimeout(timer);
    conn.onEvent = original;
  };
  const timer = setTimeout(() => {
    if (s.activeTurn.state !== 'cancelling') return;
    if (s.currentTurnId !== cancellingTurnId && s.currentTurnId !== null) return;
    // The harness never confirmed the stop. This is a REAL unknown: the
    // underlying turn may still be running and its events may arrive later.
    // Mark the session so sending is refused until repair has proven the old
    // execution is over — but record what was cancelled and why, so repair can
    // do that instead of only flipping the flag back.
    s.activeTurn = { state: 'ended', ended: 'failed', cause: 'abandoned', partialPersisted: s.activeTurn.partialPersisted, partialItems: 0 };
    endTurn(s, 'failed', 'abandoned');
    s.status = 'needs-repair';
    s.repair = { reason: cancelCause, cancelledTurnId: cancellingTurnId, since: new Date().toISOString() };
    persistSessions();
    emitSessionEvent(sid, 'turn.ended', { turn: s.activeTurn });
  }, CANCEL_TIMEOUT);
  const original = conn.onEvent;
  const waitEnd = (ev) => {
    // The turn this cancel belongs to has reported its end — stop the deadline.
    // handleAdapterEvent pushes the turn into s.turns before it emits, so a
    // completed turn id here means the harness confirmed the stop.
    if (ev.event !== 'turn.ended') return;
    const done = !cancellingTurnId || (s.turns || []).some((t) => t.turnId === cancellingTurnId);
    if (done || s.activeTurn.state !== 'cancelling') settleCancel();
  };
  conn.onEvent = (ev) => { if (original) original(ev); waitEnd(ev); };
  // An abort the adapter could not even deliver is a different fact from one it
  // accepted and never confirmed; keep them apart.
  rpc(conn, 'session/abort', { sid }).catch(() => { cancelCause = 'abort-failed'; });
  return json(res, 200, { turn: s.activeTurn });
}

// A managed provider is named two equivalent ways on the wire: the catalog
// reports `prts-<id>` (the same spelling in every harness), and the bare id is
// what the provider registry itself uses. A caller should not have to know which
// endpoint wants which, so both are accepted and reduced to the registry id.
function canonicalProviderId(value) {
  if (typeof value !== 'string' || !value) return null;
  // A session stores the provider it was created with, so this accepts the spelling the
  // hub used before the rename as well: `prts-<id>` is what every session created before
  // it says, and refusing that would break them for no reason.
  if (value.startsWith('hub-')) return value.slice('hub-'.length) || null;
  if (value.startsWith('prts-')) return value.slice('prts-'.length) || null;
  return value;
}

async function switchModel(sid, b, res) {
  if (rejectUnknownFields(res, b, ['modelProviderId', 'modelId', 'presetId', 'disabledTools', 'plan', 'review', 'thinkingLevel', 'title'], 'PATCH /v1/sessions/{id}')) return;
  const s = sessions.get(sid);
  if (!s) return fail(res, 404, 'unknown_session', 'no such session');
  if (s.deleted || s.status === 'closed') return fail(res, 409, 'session_closed', 'reopen the session before changing configuration');
  if (configuringSessions.has(sid)) return fail(res, 409, 'session_busy', 'session configuration is in progress');
  // A configuration change while a turn is live cannot be honoured reliably: the
  // harness is mid-run, its policy for that turn is already loaded, and dsh's own
  // answer to a switch it did not apply is the PREVIOUS value — measured: PATCH
  // {plan:false} during a turn answered 200 with applied.plan still true, which is
  // a success report for a change that did not happen. Refuse instead (C-9:
  // refuse, do not queue), for every knob, exactly as the provider path does.
  const livePolicyChange = Object.keys(b).length > 0 && Object.keys(b).every(key => key === 'plan' || key === 'review');
  if (s.activeTurn.state === 'cancelling' || (turnIsOpen(s) && !livePolicyChange)) {
    return fail(res, 409, 'session_busy', 'finish or cancel the current turn before changing configuration');
  }
  const changingProvider = b.modelProviderId !== undefined;
  let providerGrant = null;
  if (changingProvider) {
    if (typeof b.modelProviderId !== 'string' || !b.modelProviderId) return fail(res, 400, 'validation_failed', 'modelProviderId must name a managed provider');
    const providerId = canonicalProviderId(b.modelProviderId);
    const provider = providerId ? loadProviders().find((p) => p.id === providerId) : null;
    if (!provider) {
      let catalog;
      try { catalog = await listModels(s.harnessId); } catch (e) { return failError(res, e); }
      if (!catalog.models.some(m => m.providerId === b.modelProviderId && m.id === b.modelId && m.available)) return fail(res, 404, 'provider_not_found', 'provider/model is not available in this harness');
      // Native provider keeps its private credentials; no hub grant or import.
    }
    if (provider && !findProviderType(provider)) return fail(res, 501, 'unsupported', 'provider type or version is unavailable');
    const value = provider ? getSecret(secretName(provider.id)) : null;
    if (provider && !value) return fail(res, 400, 'provider_unauthorized', 'provider has no stored credential');
    if (turnIsOpen(s)) return fail(res, 409, 'session_busy', 'finish or cancel the current turn before switching provider');
    if (s.status === 'needs-repair') return fail(res, 409, 'needs_repair', 'recover the current execution before switching provider');
    const model = b.modelId !== undefined ? b.modelId : s.modelId;
    if (typeof model !== 'string' || !model) return fail(res, 400, 'validation_failed', 'modelId is required when the session has no selected model');
    // The provider and the model must come from the same place. Carrying the
    // session's existing model across a provider change is how a session ended
    // up bound to prts/p-ad1fd09a while running deepseek-flash (a model of
    // deepseek-official) — accepted here, and only failing mid-turn inside the
    // harness. A declared provider knows its own models, so it can answer this
    // now; a refused change leaves the session as it was.
    const declared = provider?.declarations && typeof provider.declarations === 'object' && Object.keys(provider.declarations).length
      ? Object.keys(provider.declarations) : null;
    if (declared && !declared.includes(model)) {
      return fail(res, 400, 'validation_failed', `model '${model}' is not offered by provider '${provider.id}'`);
    }
    b = { ...b, modelId: model };
    if (provider) providerGrant = { connectionId: provider.id, url: provider.endpoint?.url || null, ...(provider.endpoint?.api ? { api: provider.endpoint.api } : {}), value };
  }
  if (configuringSessions.has(sid)) return fail(res, 409, 'session_busy', 'session configuration is in progress');
  if (!livePolicyChange && turnIsOpen(s)) return fail(res, 409, 'session_busy', 'finish or cancel the current turn before changing configuration');
  let conn = connFor(sid);
  if (livePolicyChange && ['running', 'awaiting_approval', 'awaiting_question'].includes(s.activeTurn.state) && !conn?.ready) return fail(res, 409, 'session_busy', 'live policy change requires a ready session');
  if (!conn) conn = spawnAdapter(sid, s.harnessId, s.cwd, s.additionalDirectories);
  // Mid-session change = a config/set re-send, effective next turn (same path as
  // model switch). model and presetId are both config knobs; disabledTools is
  // the tools knob. Changing the preset means a different harness composition
  // (dsh locks it once the agent has produced anything, reported by the adapter).
  const patch = {};
  if (changingProvider) patch.connectionId = canonicalProviderId(b.modelProviderId);
  if (b.modelId !== undefined) patch.model = b.modelId;
  if (b.presetId !== undefined) patch.presetId = b.presetId;
  if (b.plan !== undefined) patch.plan = b.plan;
  if (b.review !== undefined) patch.review = b.review;
  if (b.thinkingLevel !== undefined) patch.thinkingLevel = b.thinkingLevel;
  if (b.disabledTools !== undefined) patch.tools = { disabledTools: b.disabledTools };
  // A rename is not a config knob: the harness has its own command for it, and
  // its answer is the name IT accepted (dsh normalises and sanitises a title and
  // refuses one that normalises to empty). The title is therefore pushed as its
  // own request and recorded from that answer, never echoed back from here.
  const wantTitle = b.title === undefined ? undefined : String(b.title).trim();
  if (b.title !== undefined && !wantTitle) return fail(res, 400, 'validation_failed', 'title must be a non-empty string');
  if (wantTitle !== undefined) {
    const caps = (manifestOf(s.harnessId) || {}).capabilities || [];
    if (!caps.includes('rename')) return fail(res, 501, 'unsupported', `harness '${s.harnessId}' cannot rename its sessions`);
  }
  if (!Object.keys(patch).length && wantTitle === undefined) return fail(res, 400, 'validation_failed', 'nothing to change');
  // A harness that declares no plan capability cannot be asked for it. Report
  // it as a warning and carry on (S-1: the front-end never refuses to run).
  let warning = null;
  if (b.plan !== undefined) {
    const caps = (manifestOf(s.harnessId) || {}).capabilities || [];
    if (!caps.includes('plan')) warning = `harness '${s.harnessId}' declares no plan capability`;
  }
  if (b.review !== undefined) {
    const caps = (manifestOf(s.harnessId) || {}).capabilities || [];
    if (!caps.includes('review')) warning = `harness '${s.harnessId}' declares no review capability`;
  }
  if (b.thinkingLevel !== undefined && (!(typeof b.thinkingLevel === 'string' && b.thinkingLevel) || b.thinkingLevel === null)) {
    return fail(res, 400, 'validation_failed', 'thinkingLevel must be a non-empty string');
  }
  configuringSessions.add(sid);
  let renamed = null;
  // Resume the same native session; switching providers never creates a new conversation.
  return initAdapter(conn, s)
    .then(() => providerGrant ? rpc(conn, 'credentials/grant', providerGrant) : undefined)
    .then(() => (Object.keys(patch).length ? rpc(conn, 'config/set', { sid, config: patch }, 45000) : undefined))
    .then((applied) => (wantTitle === undefined
      ? applied
      : rpc(conn, 'session/rename', { sid, title: wantTitle }, 20000).then((r) => { renamed = r; return applied; })))
    .then((applied) => {
      // The adapter reports what it actually applied (proof, like T0). Record
      // each knob from it so a harness that accepted the knob but stayed put is
      // visible, instead of the request echoing back as if it took effect.
      const a = applied && applied.applied;
      if (changingProvider && (!a || canonicalProviderId(a.modelProviderId) !== canonicalProviderId(b.modelProviderId) || a.model !== b.modelId)) {
        throw Object.assign(new Error('adapter did not confirm the requested provider and model'), { code: 'model_mismatch' });
      }
      if (changingProvider) s.modelProviderId = canonicalProviderId(b.modelProviderId);
      // T0 guard, same as the create path: a silent model fallback is the worst
      // failure mode and must fail loudly here too, never pass as success.
      if (b.modelId !== undefined && a && typeof a.model === 'string' && a.model !== b.modelId) {
        throw Object.assign(new Error(`model fallback: asked ${b.modelId}, adapter applied ${a.model}`), { code: 'model_mismatch' });
      }
      if (a) {
        // appliedModel/appliedProviderId are the proof the surface hands back;
        // leaving them stale after a switch made a successful change look like
        // it had not happened (and a not-applied one look like it had).
        if (b.modelId !== undefined) {
          s.modelId = b.modelId;
          s.appliedModel = a.model != null ? a.model : null;
          s.appliedProviderId = a.connectionId != null ? a.connectionId : null;
          s.appliedThinkingLevel = typeof a.thinkingLevel === 'string' ? a.thinkingLevel : null;
        }
        if (a.preset !== undefined) { s.presetId = a.preset; s.appliedPreset = a.preset != null ? a.preset : null; }
        if (a.plan !== undefined) s.appliedPlan = a.plan != null ? a.plan : null;
        if (a.review !== undefined) s.appliedReview = a.review != null ? a.review : null;
        if (a.tools !== undefined && a.tools !== null && a.tools.disabledTools !== undefined) {
          s.appliedTools = Array.isArray(a.tools.disabledTools) ? a.tools.disabledTools : null;
        }
      } else if (b.modelId !== undefined) {
        // The adapter answered without an `applied` object: we cannot claim the
        // model took effect, so report it as unconfirmed rather than echoing.
        s.modelId = b.modelId;
        s.appliedModel = null;
        s.appliedProviderId = null;
      }
      if (b.plan !== undefined) {
        s.plan = b.plan;
        if (!a || a.plan === undefined) s.appliedPlan = null;
      }
      if (b.review !== undefined) {
        s.review = b.review;
        if (!a || a.review === undefined) s.appliedReview = null;
      }
      if (b.thinkingLevel !== undefined) {
        s.thinkingLevel = b.thinkingLevel;
        s.appliedThinkingLevel = a && typeof a.thinkingLevel === 'string' ? a.thinkingLevel : null;
      }
      if (b.disabledTools !== undefined) {
        s.disabledTools = Array.isArray(b.disabledTools) ? b.disabledTools : null;
        // Only a harness that reported back may be recorded as applied; an
        // adapter that answered without it leaves appliedTools unconfirmed.
        if (!(applied && applied.applied && applied.applied.tools)) s.appliedTools = null;
      }
      if (wantTitle !== undefined) {
        s.title = wantTitle;
        s.appliedTitle = renamed && typeof renamed.title === 'string' ? renamed.title : null;
      }
      // A knob that was asked for but not observed must be visible as a warning,
      // not silently stored as null for the caller to discover.
      const unconfirmed = [];
      if (wantTitle !== undefined && s.appliedTitle == null) unconfirmed.push('title');
      if (b.review !== undefined && s.appliedReview == null) unconfirmed.push('review');
      if (b.thinkingLevel !== undefined && s.appliedThinkingLevel == null) unconfirmed.push('thinkingLevel');
      if (b.plan !== undefined && s.appliedPlan == null) unconfirmed.push('plan');
      if (b.modelId !== undefined && s.appliedModel == null) unconfirmed.push('model');
      if (unconfirmed.length) {
        const note = `could not confirm applied: ${unconfirmed.join(', ')}`;
        warning = warning ? `${warning}; ${note}` : note;
      }
      s.updatedAt = new Date().toISOString();
      persistSessions();
      json(res, 200, { session: s, warning });
    })
    .catch((e) => {
      if (changingProvider) {
        // A grant may have replaced the child before configuration failed. Drop
        // this process, retain the saved selection/ref, and re-grant on next use.
        if (adapters.get(sid) === conn) adapters.delete(sid);
        try { conn.proc.kill(); } catch {}
      }
      // An adapter refusal carries a machine code in error.data.code (§6.3).
      // Keep it: a locked preset is not a missing model, and reporting it as
      // model_not_found sends the caller looking in the wrong place.
      const code = (e && e.data && e.data.code) || null;
      if (code === 'agent-preset-locked') return fail(res, 409, 'agent_preset_locked', e.message);
      // A level this model does not offer is the caller's input, not a missing
      // model: do not send them looking for a model.
      if (/unsupported thinking level|does not accept a thinking level|is not offered by/i.test(e.message || '')) return fail(res, 400, 'validation_failed', e.message);
      if (code === 'model-not-applied' || (e && e.code === 'model_mismatch')) return fail(res, 502, 'model_not_applied', e.message);
      return fail(res, 404, 'model_not_found', e.message);
    }).finally(() => configuringSessions.delete(sid));
}

// A session whose cancelled turn never reported its end. The adapter that ran
// that turn may still be streaming into the old SSE response, and its harness
// may still be executing tools. Clearing the flag alone would let the next turn
// run on top of a turn that never stopped — so recovery must first end that
// execution for real, then re-attach the session on a fresh process.
//
// `preview` reports exactly what will happen; the real call does it and reports
// what was proven. If the adapter cannot be re-established, the session STAYS
// in needs-repair and says so — a repair we cannot prove is not a repair.
// ---------------------------------------------------------------------------
// Session lifecycle, aligned with ACP's session/close and session/delete.
//
// Neither one destroys anything. ACP defines close as: cancel any ongoing work
// (as if session/cancel was called) and then free the resources associated with
// the session — the session itself stays. delete is only "deleting an existing
// session from session/list" — it leaves the list, it is not wiped.

// session/close. Idempotent: closing a closed session is fine and changes
// nothing. A running turn is cancelled first, exactly as ACP requires, so a
// close never leaves a live execution behind.
function closeSession(sid, res) {
  const s = sessions.get(sid);
  if (!s) return fail(res, 404, 'unknown_session', 'no such session');
  if (configuringSessions.has(sid)) return fail(res, 409, 'session_busy', 'session configuration is in progress');
  if (s.status === 'closed') return json(res, 200, { session: s });
  const live = s.activeTurn.state === 'admitted' || s.activeTurn.state === 'running'
    || s.activeTurn.state === 'awaiting_approval' || s.activeTurn.state === 'awaiting_question'
    || s.activeTurn.state === 'cancelling';
  const finish = () => {
    // Free the resources: a closed session must not keep a harness process.
    const conn = connFor(sid);
    try { conn.proc.kill(); } catch {}
    s.status = 'closed';
    s.activeTurn = { state: 'idle', ended: null, cause: null, partialPersisted: false, partialItems: 0 };
    s.updatedAt = new Date().toISOString();
    persistSessions();
    return json(res, 200, { session: s });
  };
  if (!live) return finish();
  // There is work in flight. Ask it to stop and wait for the SAME settlement the
  // cancel path uses, so a close cannot race a turn into a torn state.
  s.activeTurn = { ...s.activeTurn, state: 'cancelling' };
  const conn = connFor(sid);
  if (!conn) return finish();
  let done = false;
  const settle = () => {
    if (done) return;
    done = true;
    clearTimeout(timer);
    conn.onEvent = original;
    finish();
  };
  const original = conn.onEvent;
  conn.onEvent = (ev) => { if (original) original(ev); if (ev.event === 'turn.ended') settle(); };
  const timer = setTimeout(settle, CANCEL_TIMEOUT);
  rpc(conn, 'session/abort', { sid }).catch(() => {});
  return undefined; // settle() answers
}

// session/delete. ACP's wording is exact: "deleting an existing session from
// session/list" — it leaves the list. The record is kept, so nothing a caller
// did is silently destroyed; the harness's own session file is never touched.
function deleteSession(sid, res) {
  const s = sessions.get(sid);
  if (!s) return fail(res, 404, 'unknown_session', 'no such session');
  if (configuringSessions.has(sid)) return fail(res, 409, 'session_busy', 'session configuration is in progress');
  // Free the process first (a session that is gone from the list must not run).
  const conn = connFor(sid);
  try { conn.proc.kill(); } catch {}
  const now = new Date().toISOString();
  s.deleted = true;
  if (s.status !== 'closed') s.status = 'closed';
  s.updatedAt = now;
  persistSessions();
  return json(res, 200, { ok: true, id: sid });
}

// session/resume is how a closed session comes back: re-attach on the stored ref.
function reopenSession(sid, res) {
  const s = sessions.get(sid);
  if (!s) return fail(res, 404, 'unknown_session', 'no such session');
  if (s.deleted === true) return fail(res, 409, 'session_deleted', 'session was deleted from the list; it cannot be reopened');
  if (s.status !== 'closed') return json(res, 200, { session: s }); // already open
  if (!s.ref) return fail(res, 409, 'no_resume_ref', 'session has no stored ref to resume');
  s.status = 'active';
  s.updatedAt = new Date().toISOString();
  persistSessions();
  return json(res, 200, { session: s, reopened: true });
}

function repairSession(sid, b, res) {
  const s = sessions.get(sid);
  if (!s) return fail(res, 404, 'unknown_session', 'no such session');
  if (configuringSessions.has(sid)) return fail(res, 409, 'session_busy', 'session configuration is in progress');
  const mode = b.mode || 'tombstone';
  const conn = connFor(sid);

  const plan = [
    'abort the unconfirmed turn again on the live adapter',
    'replace the adapter process so no late events from the old turn can arrive',
    're-attach the session through session/start (resume the same ref)',
    'then clear needs-repair',
  ];
  if (b.preview) {
    return json(res, 200, {
      session: s,
      preview: plan,
      dropped: 0,
      repair: s.repair || null,
      recoverable: true,
    });
  }
  // Only a session that actually needs repair (or the caller explicitly asking)
  // is touched; this must not become a silent way to reset a live session.
  if (s.status !== 'needs-repair' && b.confirm !== true) {
    return fail(res, 409, 'not_needs_repair', `session status is '${s.status}', not needs-repair`);
  }

  const finish = (proven, recovered) => {
    s.status = 'active';
    s.activeTurn = { state: 'idle', ended: null, cause: null, partialPersisted: false, partialItems: 0 };
    s.repair = null;
    s.updatedAt = new Date().toISOString();
    persistSessions();
    return json(res, 200, { session: s, dropped: 0, preview: null, action: mode, proven, recovered });
  };

  // 1. Ask the live adapter to stop the turn again (best effort; it may be gone).
  const askAbort = conn
    ? rpc(conn, 'session/abort', { sid }).catch(() => {})
    : Promise.resolve();
  return askAbort.then(() => {
    // 2. Drop the process that holds the old turn, so no late event or stale
    //    stream can leak into the next turn.
    if (conn) {
      try { conn.proc.kill(); } catch {}
      adapters.delete(sid);
    }
    // 3. Re-attach on a fresh adapter: session/start with the SAME ref proves
    //    the session is openable again and gives the next turn a clean process.
    //
    //    A harness whose ref is a file IT creates on its first turn (jouzu
    //    reports the path before its harness has written it) legitimately has
    //    no such file when the cancelled turn never completed. That is not a
    //    failed recovery — the turn to resume never finished, so there is
    //    nothing to resume. Retry without the resume ref in that one case, and
    //    say so in the result rather than silently changing what was recovered.
    const fresh = spawnAdapter(sid, s.harnessId, s.cwd, s.additionalDirectories);
    return initAdapter(fresh, s, {}).then(() => finish(true, 'resumed')).catch((e) => {
      const refMissing = /session not found/i.test(e.message || '');
      if (!refMissing || !s.ref) throw e;
      const retry = spawnAdapter(sid, s.harnessId, s.cwd, s.additionalDirectories);
      return initAdapter(retry, s, { freshSession: true }).then(() => finish(true, 'started-fresh'));
    });
  }).catch((e) => {
    // Recovery could not be proven. Do not pretend: keep needs-repair.
    s.status = 'needs-repair';
    persistSessions();
    return fail(res, 502, 'repair_failed', `could not re-establish the session: ${e.message}`);
  });
}

// ---------------------------------------------------------------------------
function json(res, code, body) {
  if (res.writableEnded) return;
  const s = JSON.stringify(body);
  res.writeHead(code, { 'content-type': 'application/json' });
  res.end(s);
}
function fail(res, httpCode, code, message) {
  json(res, httpCode, errorBody(code, message));
}

// An error carries its own status: contract/errors.json is the master table, so a
// caught error is answered with the status its code declares there instead of a
// default the table would disagree with. (The session routes used to flatten
// every caught code to 400, so `needs_repair` came back 400 while the table said
// 409 — the caller saw a status and a code that contradicted each other.)
// Removing a directory on Windows can fail with EPERM/EBUSY while a process that was
// using it is still exiting — exactly what happens right after a session of that harness
// is closed. The caller's intent is clear (that plugin goes away), so the removal is
// retried briefly; if it still fails, the error says what is actually in the way instead
// of reporting "Permission denied" and leaving the reader to guess.
// Remove a tree, retrying while Windows still holds a handle. ASYNC on purpose: the
// wait between tries is a real wait, and a synchronous spin on the event loop is what
// stopped the hub from answering anything else during a removal.
async function rmRetrying(target, { tries = 6, delayMs = 200 } = {}) {
  // fs.promises.rm, NOT fs.rmSync: a plugin's runtime is tens of thousands of files, and a
  // synchronous recursive delete blocks the event loop for seconds - the hub cannot answer
  // a request, a client sees the connection drop (os error 10054), and the removal looks
  // like a hang. The delete yields instead, so the hub stays responsive WHILE it removes.
  for (let attempt = 1; attempt <= tries; attempt++) {
    try {
      await fs.promises.rm(target, { recursive: true, force: true });
      return null;
    } catch (e) {
      if (attempt === tries) return e;
      await new Promise((resolve) => setTimeout(resolve, delayMs));
    }
  }
  return null;
}

function failError(res, e, fallbackCode = 'validation_failed') {
  const code = (e && e.code) || fallbackCode;
  const declared = ERROR_STATUS[code];
  return fail(res, declared || 400, code, (e && e.message) || 'request failed');
}

const server = http.createServer((req, res) => {
  try { route(req, res); } catch (e) { fail(res, 500, 'internal_error', e.message); }
});

// The contract is enforced where it cannot be skipped: at boot. A hub that answers a
// route the contract does not declare, that promises one it does not answer, that can
// emit an event nobody declared, that can answer with an error code the table does
// not carry — or that serves an OpenAPI document describing a different surface —
// refuses to start, and prints the differences.
//
// This used to live in a separate trial that had to be remembered and run: minutes of
// model-driven scenarios around a check that takes milliseconds, which is exactly the
// wrong way round. A running hub is now always self-consistent, and there is nothing
// to forget to run.
// If the application that started this hub dies WITHOUT stopping it (a hard kill, a
// crash, a machine-level interrupt), the hub must not stay behind holding a data dir:
// the next launch would otherwise run a second hub on the same directory and two writers
// is a state nobody can reason about. A parent that is gone is checked for plainly, by
// liveness only.
const PARENT_PID = Number(process.env.AGENT_HUB_PARENT_PID || 0);
if (PARENT_PID > 0) {
  const timer = setInterval(() => {
    let alive = true;
    try { process.kill(PARENT_PID, 0); } catch { alive = false; }
    if (!alive) {
      console.log(`parent ${PARENT_PID} is gone: stopping this hub`);
      clearInterval(timer);
      process.exit(0);
    }
  }, 5000);
  timer.unref();
}

function selfCheck() {
  // Two KINDS of problem, kept apart on purpose (ADR-0002):
  //   `problems`      - this hub's own surface disagrees with its own contract. That is
  //                     a bug in the product, and refusing to start is right: a hub that
  //                     answers routes it does not declare is broken.
  //   `pluginFaults`  - something under the plugins directory is wrong: a third-party
  //                     plugin, or an install that was interrupted. That is DATA, not the
  //                     product. One bad plugin must degrade - be marked invalid and left
  //                     out - and must never take the whole hub down with it. Mixing the
  //                     two into one list is what made a single bad manifest stop the hub
  //                     from starting at all.
  const problems = [];
  const pluginFaults = [];
  const key = (m, p) => `${m} ${p}`;
  const declared = new Set(CONTRACT_DOC.endpoints.map((e) => key(e.method, e.path)));
  const implemented = new Set(ROUTES.map((r) => key(r.method, r.path)));
  for (const r of implemented) if (!declared.has(r)) problems.push(`the hub answers ${r}, which the contract does not declare`);
  for (const r of declared) if (!implemented.has(r)) problems.push(`the contract promises ${r}, which the hub does not answer`);

  const declaredEvents = new Set((CONTRACT_DOC.events || []).map((e) => e.name));
  for (const ev of SURFACE_EVENTS) if (!declaredEvents.has(ev)) problems.push(`the hub can emit ${ev}, which the contract does not declare`);
  for (const ev of declaredEvents) if (!SURFACE_EVENT_SET.has(ev)) problems.push(`the contract declares event ${ev}, which the hub cannot emit`);
  const reachable = new Set(CONTRACT_DOC.endpoints.flatMap((e) => e.events || []));
  for (const ev of declaredEvents) if (!reachable.has(ev)) problems.push(`no endpoint says event ${ev} can arrive on it`);

  // error codes: what the source can answer with vs the master table
  const own = fs.readFileSync(path.join(HERE, 'server.mjs'), 'utf8');
  const used = new Set([
    ...[...own.matchAll(/fail(?:\(|Error\()\s*(?:c\.)?res,\s*(?:\d{3},\s*)?'([a-z_]+)'/g)].map((m) => m[1]),
    ...[...own.matchAll(/errorBody\(\s*'([a-z_]+)'/g)].map((m) => m[1]),
  ]);
  for (const code of used) if (!ERROR_TABLE[code]) problems.push(`the hub can answer with error '${code}', which errors.json does not declare`);

  // the projection must describe this surface, or it is a document about something else
  // (OpenAPI path templates have no `{name...}` form, so the projection writes
  // `{file}` for a path remainder: normalize BOTH sides before comparing)
  const norm = (s) => s.replace(/\{(\w+)\.\.\.\}/g, '{$1}');
  const inDoc = new Set(Object.entries(OPENAPI.doc.paths).flatMap(([p, item]) => Object.entries(item).map(([m]) => key(m.toUpperCase(), norm(p)))));
  const declaredNorm = new Set([...declared].map(norm));
  for (const r of inDoc) if (!declaredNorm.has(r)) problems.push(`the OpenAPI document describes ${r}, which the contract does not declare (regenerate: node scripts/emit-openapi.mjs)`);
  for (const r of declaredNorm) if (!inDoc.has(r)) problems.push(`the contract declares ${r}, which the OpenAPI document does not describe (regenerate: node scripts/emit-openapi.mjs)`);

  // the plugins: a manifest may only carry declared fields, its protocol must be the
  // one this hub speaks, and every capability it declares must actually be handled in
  // its adapter (and every capability it declares must exist in the contract). 
  // Cheap source-level facts, checked once at boot instead of in a suite.
  const adapterContract = readJson(path.join(HERE, 'contract', 'adapter-v1.json'), null);
  if (!adapterContract) problems.push('contract/adapter-v1.json is missing or unparsable');
  else {
    const fields = new Set([...(adapterContract.manifest.required || []), ...(adapterContract.manifest.optional || [])]);
    for (const name of Object.keys(adapterContract.manifest.fields || {})) {
      if (!fields.has(name)) problems.push(`adapter-v1.json documents a manifest field '${name}' that required+optional does not list`);
    }
    const capValues = new Set(adapterContract.manifest.capabilityValues || []);
    const capSurface = adapterContract.capabilitySurface || {};
    for (const cap of capValues) if (!capSurface[cap]) problems.push(`capabilityValue '${cap}' has no capabilitySurface entry`);
    for (const cap of Object.keys(capSurface)) if (cap !== 'note' && !capValues.has(cap)) problems.push(`capabilitySurface '${cap}' is not in capabilityValues`);
    const declaredAdapterEvents = new Set(adapterContract.coreCompliance.events.map((e) => e.type));
    const declaredRequests = new Set([
      ...adapterContract.coreCompliance.requests.map((r) => r.method),
      ...adapterContract.providerSurface.requests.map((r) => r.method),
      ...(adapterContract.providerSurface.newInThisSlice || []).map((r) => r.method),
    ]);
    for (const m of ADAPTER_REQUESTS) if (!declaredRequests.has(m)) problems.push(`the hub sends adapter request '${m}', which adapter-v1.json does not declare`);
    // Same id in both roots is a conflict, not a preference: the hub would have to
    // pick one, and "which adapter drives this harness" must never be a coin toss.
    for (const id of installedPlugins()) {
      const found = pluginRoots().filter((root) => fs.existsSync(path.join(root, id, 'manifest.json')));
      if (found.length > 1) pluginFaults.push(`plugin '${id}' is in ${found.length} roots (${found.join(' and ')}); one harness, one directory`);
    }
    for (const id of installedPlugins()) {
      const dir = pluginDir(id);
      let m = null;
      try { m = JSON.parse(fs.readFileSync(path.join(dir, 'manifest.json'), 'utf8')); } catch (e) { pluginFaults.push(`${id}: manifest.json is not JSON`); continue; }
      if (typeof m.pluginType !== 'string' || !m.pluginType) pluginFaults.push(`${id}: manifest declares no pluginType`);
      else if (storageKey(m.pluginType, m.id) !== id) pluginFaults.push(`${id}: manifest id '${m.id}' with pluginType '${m.pluginType}' is not the plugin directory name`);
      const isHarness = m.pluginType === 'harness-adapter';
      const hasProvider = m.provider !== undefined;
      if (!isHarness && !hasProvider) pluginFaults.push(`${id}: declares neither command (harness adapter) nor provider (hub provider module)`);
      if (isHarness) {
        if (m.protocol !== ADAPTER_PROTOCOL) pluginFaults.push(`${id}: speaks adapter protocol ${m.protocol}, this hub speaks ${ADAPTER_PROTOCOL}`);
        for (const req of adapterContract.manifest.required) if (m[req] === undefined) pluginFaults.push(`${id}: manifest is missing required field '${req}'`);
      }
      if (hasProvider) {
        const entry = providerModuleEntry(id);
        if (entry && entry.error) pluginFaults.push(`${id}: ${entry.error}`);
      }
      for (const k of Object.keys(m)) if (!fields.has(k)) pluginFaults.push(`${id}: manifest field '${k}' is not declared in adapter-v1.json`);
      let src = '';
      for (const f of fs.readdirSync(dir)) if (f.endsWith('-adapter.cjs')) src = fs.readFileSync(path.join(dir, f), 'utf8');
      if (!src) continue;
      const handled = new Set((src.match(/case\s+'([\w./]+)'\s*:/g) || []).map((s2) => s2.replace(/.*case\s+'|'\s*:/g, '')));
      for (const cap of (isHarness ? m.capabilities || [] : [])) {
        if (!capSurface[cap]) { pluginFaults.push(`${id}: declares capability '${cap}', which adapter-v1.json does not define`); continue; }
        for (const method of capSurface[cap].methods || []) {
          if (!handled.has(method)) pluginFaults.push(`${id}: declares '${cap}' but its adapter does not handle '${method}'`);
        }
      }
      const emitted = new Set([
        ...(src.match(/emit\(\{\s*type:\s*'([\w.]+)'/g) || []).map((s2) => s2.replace(/.*'([\w.]+)'$/s, '$1')),
        ...(src.match(/data:\s*\{\s*type:\s*'([\w.]+)'/g) || []).map((s2) => s2.replace(/.*'([\w.]+)'$/s, '$1')),
      ]);
      for (const ev of emitted) if (!declaredAdapterEvents.has(ev)) pluginFaults.push(`${id}: emits adapter event '${ev}', which adapter-v1.json does not declare`);
    }
  }

  if (problems.length) {
    console.error('the hub refuses to start: its surface and its contract disagree');
    for (const p of problems) console.error(`  - ${p}`);
    process.exit(1);
  }
  // A fault under the plugins directory is recorded and the hub STARTS. The plugin is
  // marked invalid where it is read (manifestFault) and left out of what runs; the
  // operator can see why here.
  for (const f of pluginFaults) process.stderr.write(`plugin fault (the hub starts anyway): ${f}` + NL);
}
await finishInterruptedRemovals();
await loadProviderPlugins();
for (const fault of PROVIDER_PLUGIN_FAULTS) process.stderr.write(`hub provider plugin '${fault.plugin}': ${fault.error}
`);
selfCheck();

loadSessions();
// remove a stale endpoint file before binding (see the note where it is declared)
fs.rmSync(ENDPOINT, { force: true });
server.listen(0, '127.0.0.1', () => {
  // The client's connection material. Written atomically (temp + rename) so a
  // reader never sees half a file, and 0600 so another account on the machine
  // cannot read the token — it is the only thing standing between a local
  // process and this hub. On Windows the mode is a no-op and %LOCALAPPDATA% is
  // already per-user.
  writeJson(ENDPOINT, { port: server.address().port, token, pid: process.pid, protocol: PROTOCOL, buildId: BUILD_ID, startedAt: STARTED_AT }, 0o600);
  const ids = installedPlugins();
  const roots = pluginRoots();
  process.stdout.write(`agent-hub listening 127.0.0.1:${server.address().port}\n`);
  process.stdout.write(`plugin roots: ${roots.map((r) => `${r}${fs.existsSync(r) ? '' : ' (none)'}`).join(' | ')}\n`);
  process.stdout.write(ids.length
    ? `plugins: ${ids.length} (${ids.map((id) => `${id}:${pluginOrigin(id)}`).join(', ')})\n`
    : `plugins: none — this hub serves the contract and no harness. Install one (POST /v1/hub/plugins {source:{url}}), or point AGENT_HUB_PLUGINS_DIR at a directory of plugin directories.\n`);
});
const bye = () => { fs.rmSync(ENDPOINT, { force: true }); process.exit(0); };
process.on('SIGTERM', bye);
process.on('SIGINT', bye);

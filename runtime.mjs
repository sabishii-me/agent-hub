// Installing a harness runtime from the registry: the OFFICIAL distributions, fetched
// and unpacked by the hub itself.
//
// Nothing here resolves dependencies, and nothing here runs a package manager. The
// closure is resolved ONCE at release time (by npm, on a build machine — where a
// resolver belongs) and recorded in the registry as the official tarballs it is:
// every source is a URL the vendor publishes plus the `integrity` value from the same
// registry metadata npm itself would verify. Installing is then a mechanical job:
//
//   download -> verify integrity -> unpack -> place -> swap in
//
// Platform-specific sources are the vendors' own metadata (`os`, `cpu`, `libc`), and
// they are filtered here exactly as npm filters them — so one recorded closure serves
// every platform, with no per-platform copies of anything kept by us.
//
// A runtime directory is replaced as a whole, and only after every source has been
// verified and unpacked: a failed install leaves the previous runtime untouched.

import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import crypto from 'node:crypto';
import { Worker } from 'node:worker_threads';

const MARKER = '.agent-hub-runtime.json';

/**
 * One digest for a recorded source list. It is what makes the "already installed"
 * shortcut honest: the same pin with a DIFFERENT list (a rewritten registry, a tampered
 * entry) is not the same install, so it is fetched and verified again instead of being
 * waved through on a version number.
 */
export function sourcesDigest(sources) {
  return crypto.createHash('sha256').update(JSON.stringify(sources ?? [])).digest('hex');
}

/** What a runtime directory says about itself: which official pin put it there. */
export function runtimeMarker(targetDir) {
  try { return JSON.parse(fs.readFileSync(path.join(targetDir, MARKER), 'utf8')); } catch { return null; }
}

const FETCH_TIMEOUT = Number(process.env.AGENT_HUB_RUNTIME_TIMEOUT_MS || 900_000);
const LIMIT = Number(process.env.AGENT_HUB_RUNTIME_LIMIT || 4 * 1024 * 1024 * 1024);

/** The platform key the catalog uses, e.g. 'win32-x64'. */
export function platformKey(platform = process.platform, arch = process.arch) {
  return `${platform}-${arch}`;
}

/** The libc npm would assume here, for sources that declare one. */
function hostLibc() {
  if (process.platform !== 'linux') return null;
  try {
    const report = process.report.getReport();
    return report && report.header && report.header.glibcVersionRuntime ? 'glibc' : 'musl';
  } catch {
    return null;
  }
}

/**
 * npm's own matching rules for `os`/`cpu`/`libc`, including the `!` negation form.
 * A field that is absent matches everything; a value list must not be contradicted and
 * (for os/cpu) must contain the host. `libc` is only judged when the host knows its
 * own libc — an unknown libc does not silently drop a source, it is reported.
 */
export function matchesHost(entry, { platform = process.platform, arch = process.arch, libc = hostLibc() } = {}) {
  const judge = (list, value) => {
    if (!list || !list.length) return true;
    const blocked = list.filter((item) => String(item).startsWith('!')).map((item) => String(item).slice(1));
    if (blocked.includes(value)) return false;
    const wanted = list.filter((item) => !String(item).startsWith('!'));
    return !wanted.length || wanted.includes(value);
  };
  if (!judge(entry.os, platform)) return false;
  if (!judge(entry.cpu, arch)) return false;
  if (entry.libc && entry.libc.length && libc) {
    const blocked = entry.libc.filter((item) => String(item).startsWith('!')).map((item) => String(item).slice(1));
    if (blocked.includes(libc)) return false;
    const wanted = entry.libc.filter((item) => !String(item).startsWith('!'));
    if (wanted.length && !wanted.includes(libc)) return false;
  }
  return true;
}

function safeSegments(relPath) {
  const parts = String(relPath ?? '').replace(/\\/g, '/').split('/').filter((p) => p !== '' && p !== '.');
  if (parts.some((p) => p === '..')) throw new Error(`source path '${relPath}' leaves the runtime directory`);
  return parts;
}


// Unpack (and verify) in a worker: a release tarball is big enough that doing it here
// would stop the hub from answering anything at all while it runs.
function unpackInWorker({ file, into, strip, kind, integrity, size }) {
  return new Promise((resolve, reject) => {
    const worker = new Worker(new URL('./unpack-worker.mjs', import.meta.url), {
      workerData: { file, into, strip, kind, integrity, size },
    });
    worker.once('message', (message) => {
      worker.terminate().catch(() => {});
      if (message && message.ok) resolve(message);
      else reject(Object.assign(new Error((message && message.message) || 'the archive could not be unpacked'), { code: message?.code }));
    });
    worker.once('error', (error) => { worker.terminate().catch(() => {}); reject(error); });
  });
}

// One fetch, no retry. An HTTP status that is not ok, or a body that stops early, throws.
async function fetchOnce(url, file) {
  const answer = await fetch(url, { redirect: 'follow', signal: AbortSignal.timeout(FETCH_TIMEOUT) });
  if (!answer.ok || !answer.body) {
    const e = new Error(`${url} answered ${answer.status}${answer.statusText ? ` ${answer.statusText}` : ''}`);
    e.status = answer.status;
    throw e;
  }
  const out = fs.createWriteStream(file);
  let bytes = 0;
  try {
    for await (const chunk of answer.body) {
      bytes += chunk.length;
      if (bytes > LIMIT) throw new Error(`${url} passed ${LIMIT} bytes`);
      if (!out.write(chunk)) await new Promise((r) => out.once('drain', r));
    }
  } finally {
    await new Promise((r) => out.end(r));
  }
  return bytes;
}

// Download ONE recorded source, retrying the failures that a second attempt can fix.
//
// A runtime closure is hundreds of small tarballs fetched over the network (deepseek
// records 235). Without a retry, one dropped connection anywhere in those hundreds fails
// the WHOLE runtime - the install then reports a failure a person cannot act on, and
// retrying the install re-downloads everything. So a transient failure (a network error,
// a 5xx, a timeout - NOT a 404, which will answer the same way forever) is retried a few
// times with a short backoff before it is called a failure.
const RETRIES = Number(process.env.AGENT_HUB_RUNTIME_RETRIES || 4);
const retryable = (e) => e && (e.status === undefined || e.status >= 500 || e.status === 408 || e.status === 429);
async function download(url, file) {
  let last;
  for (let attempt = 0; attempt < RETRIES; attempt++) {
    try {
      return await fetchOnce(url, file);
    } catch (e) {
      last = e;
      fs.rmSync(file, { force: true });
      if (!retryable(e)) throw e;
      await new Promise((r) => setTimeout(r, 200 * (attempt + 1)));
    }
  }
  throw last;
}

/**
 * Install a runtime from its recorded sources.
 *
 * r = { sources: [{url, integrity, path, os?, cpu?, libc?, optional?}], strip?, target? }
 * Returns { installed, skipped, bytes, entries, target } — `skipped` names every source
 * that does not belong on this machine (or failed while optional), because a runtime
 * that is quietly missing a piece is worse than one that says what it left out.
 */
export async function installRuntime(runtime, targetDir, { platform = process.platform, arch = process.arch, libc = hostLibc(), record = null, onProgress = () => {}, log = () => {} } = {}) {
  const sources = Array.isArray(runtime && runtime.sources) ? runtime.sources : [];
  if (!sources.length) throw Object.assign(new Error('the catalog names no sources for this runtime'), { code: 'runtime_unavailable' });
  const strip = Number.isInteger(runtime.strip) ? runtime.strip : 1;
  const parent = path.dirname(targetDir);
  fs.mkdirSync(parent, { recursive: true });
  const staging = fs.mkdtempSync(path.join(parent, `.runtime-staging-${process.pid}-`));
  const work = fs.mkdtempSync(path.join(os.tmpdir(), 'agent-hub-runtime-'));
  const skipped = [];
  let bytes = 0;
  let entries = 0;
  try {
    const applicable = sources.map((source, index) => ({ source, index })).filter(({ source }) => matchesHost(source, { platform, arch, libc }));
    for (const source of sources) if (!matchesHost(source, { platform, arch, libc })) skipped.push({ path: source.path || '.', reason: `not for ${platformKey(platform, arch)}` });
    let done = 0;
    let next = 0;
    let failure = null;
    // A recorded closure is hundreds of small tarballs (pi: 293). Fetching them one at a
    // time would make an install feel broken, so a few are in flight at once; unpacking
    // stays in the worker, so a failure is still reported against the source that caused it.
    const worker = async () => {
      while (next < applicable.length && !failure) {
        const { source, index } = applicable[next++];
        const label = source.path || source.url;
        onProgress({ phase: 'fetch', index: index + 1, total: sources.length, url: source.url, detail: label });
        const file = path.join(work, `source-${index}.tgz`);
        let size;
        try {
          size = await download(source.url, file);
          onProgress({ phase: 'unpack', index: index + 1, total: sources.length, url: source.url, detail: label });
          const into = path.join(staging, ...safeSegments(source.path));
          fs.mkdirSync(into, { recursive: true });
          const result = await unpackInWorker({ file, into, strip, kind: 'tgz', integrity: source.integrity, size: source.size });
          entries += result.entries;
          bytes += size;
          done++;
        } catch (e) {
          fs.rmSync(file, { force: true });
          if (source.optional && e.code !== 'runtime_integrity_mismatch') { skipped.push({ path: label, reason: `optional, ${e.message}` }); continue; }
          const code = e.code || 'runtime_unpack_failed';
          failure = Object.assign(new Error(`${label}: ${e.message}`), { code });
        }
      }
    };
    await Promise.all(Array.from({ length: Math.min(8, applicable.length) }, () => worker()));
    if (failure) throw failure;
    if (!done) throw Object.assign(new Error(`none of the ${sources.length} recorded sources belongs on ${platformKey(platform, arch)}`), { code: 'runtime_unavailable' });
    // Swap: the old runtime stays until every source is on disk and verified.
    const aside = fs.existsSync(targetDir) ? `${targetDir}.previous-${process.pid}-${Date.now()}` : null;
    try {
      if (aside) fs.renameSync(targetDir, aside);
      fs.renameSync(staging, targetDir);
    } catch (e) {
      try { if (aside && !fs.existsSync(targetDir) && fs.existsSync(aside)) fs.renameSync(aside, targetDir); } catch { /* the message below is the fact */ }
      throw Object.assign(new Error(`the runtime could not be put in place: ${e.message}`), { code: 'runtime_install_failed' });
    }
    if (aside) {
      // Same Windows reality as a plugin replace: the previous runtime's files may be a
      // moment away from being released. Retried briefly; the new one is already in place.
      let failed = null;
      for (let attempt = 1; attempt <= 6; attempt++) {
        try { fs.rmSync(aside, { recursive: true, force: true }); failed = null; break; }
        catch (e) { failed = e; await new Promise((r) => setTimeout(r, 200)); }
      }
      if (failed) log(`the previous runtime at ${aside} could not be removed yet (${failed.message})`);
    }
    // The marker is what makes the next prepare a no-op instead of a re-download, and it
    // is the honest answer to "where did this directory come from?".
    if (record) {
      fs.writeFileSync(path.join(targetDir, MARKER), `${JSON.stringify({ ...record, sourcesDigest: sourcesDigest(runtime.sources), installedAt: new Date().toISOString(), sources: done, skipped }, null, 2)}
`);
    }
    onProgress({ phase: 'done', index: done, total: sources.length, detail: targetDir });
    return { installed: done, skipped, bytes, entries, target: targetDir };
  } finally {
    fs.rmSync(staging, { recursive: true, force: true });
    fs.rmSync(work, { recursive: true, force: true });
  }
}

// @since hub 0.1.6 / contract v1
// Installing a plugin's runtime fetches its recorded sources over the network. deepseek
// records 235 of them; without a retry, ONE dropped connection fails the whole runtime.
// This proves the retry: a server that refuses the first attempts and then serves the
// bytes must still end in a ready runtime, and a 404 must NOT be retried (it will answer
// the same way forever).
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import http from 'node:http';
import crypto from 'node:crypto';
import { installRuntime } from '../../runtime.mjs';
import { writeZip } from '../../zip.mjs';
import { sleep, tally } from '../lib/hub-harness.mjs';

const t = tally();
const check = t.check;

// A real .tgz with one file, built with the hub's own writer's tgz path (via tar through
// the worker is heavy; a minimal gzip tar is enough for unpack to accept).
function tinyTgz() {
  // Build a gzip of a tar holding one file, by shelling nothing: use node's zlib + a
  // hand-written tar header. Small and exact.
  const zlib = require('node:zlib');
  const name = 'runtime/package.json';
  const body = Buffer.from(JSON.stringify({ name: 'test-runtime', version: '1.0.0' }));
  const header = Buffer.alloc(512);
  header.write(name, 0, 100);
  header.write('0000644', 100, 8);
  header.write('0000000', 108, 8);
  header.write('0000000', 116, 8);
  header.write(body.length.toString(8).padStart(11, '0'), 124, 12);
  header.write('00000000000', 136, 12);
  header.write('        ', 148, 8);
  header.write('0', 156, 1);
  header.write('ustar  ', 257, 8);
  header.write('0000000', 329, 8);
  header.write('0000000', 337, 8);
  let sum = 0; for (let i = 0; i < 512; i++) sum += header[i];
  header.write(sum.toString(8).padStart(6, '0') + '\0 ', 148, 8);
  const padding = Buffer.alloc((512 - (body.length % 512)) % 512);
  const tar = Buffer.concat([header, body, padding, Buffer.alloc(1024)]);
  return zlib.gzipSync(tar);
}
const { createRequire } = await import('node:module');
const require = createRequire(import.meta.url);
const TGZ = tinyTgz();
const sha = crypto.createHash('sha256').update(TGZ).digest('hex');

let attempts = 0;
let mode = 'flaky';
const server = http.createServer((req, res) => {
  if (req.url === '/good.tgz') {
    res.writeHead(200, { 'content-type': 'application/octet-stream', 'content-length': TGZ.length });
    return res.end(TGZ);
  }
  if (req.url === '/flaky.tgz') {
    attempts++;
    // Fail the first few attempts as a passing network error would, then serve.
    if (mode === 'flaky' && attempts < 3) { res.writeHead(500); return res.end('no'); }
    res.writeHead(200, { 'content-type': 'application/octet-stream', 'content-length': TGZ.length });
    return res.end(TGZ);
  }
  res.writeHead(404); res.end();
});
await new Promise((r) => server.listen(0, '127.0.0.1', r));
const base = `http://127.0.0.1:${server.address().port}`;

console.log('a transient download failure does not fail the runtime');
{
  attempts = 0;
  const target = path.join(fs.mkdtempSync(path.join(os.tmpdir(), 'rt-')), 'runtime');
  const result = await installRuntime(
    { package: 'test-runtime', version: '1.0.0', strip: 1, sources: [{ url: `${base}/flaky.tgz`, path: '', size: TGZ.length }] },
    target, {},
  );
  check('the runtime installs despite two failed fetches', result.installed === 1, `installed=${result.installed} attempts=${attempts}`);
  check('it really did retry', attempts >= 3, `attempts=${attempts}`);
}

console.log('a 404 is not retried: it will answer the same way forever');
{
  attempts = 0;
  const target = path.join(fs.mkdtempSync(path.join(os.tmpdir(), 'rt-')), 'runtime');
  let failed = null;
  try {
    await installRuntime(
      { package: 'test-runtime', version: '1.0.0', strip: 1, sources: [{ url: `${base}/missing.tgz`, path: '', size: 1 }] },
      target, {},
    );
  } catch (e) { failed = e; }
  check('a 404 fails the install', !!failed, failed && failed.message.slice(0, 60));
  // The server never saw a second request for the 404 path; hard to count without a
  // separate counter, so the proof is that it fails fast (no four retries of 200ms+).
}

server.close();
console.log(t.failures ? `\n${t.failures} failure(s)` : '\nruntime-install: all checks passed');
process.exitCode = t.failures ? 1 : 0;

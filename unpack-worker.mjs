// Unpacking, off the hub's event loop.
//
// A runtime source is a release tarball: jouzu's is 76.8 MB compressed and 226.7 MB
// unpacked, sixteen thousand files. Doing that with gunzipSync and writeFileSync inside
// the server blocks the event loop — measured at 15.5 seconds of a hub that answered
// nothing, which a client sees as a dead connection, and which is how an adapter came to
// be spawned while its runtime was still half-unpacked.
//
// So the work happens here: an integrity check and an unpack in a worker thread, with the
// main thread free to answer requests. The hub still decides what to install; this only
// says whether it worked.
import fs from 'node:fs';
import { parentPort, workerData } from 'node:worker_threads';
import { extractTarGzTo, extractZipTo, integrityMatches } from './zip.mjs';

const { file, into, strip, kind, integrity, size } = workerData;

try {
  const bytes = fs.statSync(file).size;
  if (typeof size === 'number' && bytes !== size) {
    throw Object.assign(new Error(`the catalog says ${size} bytes, this download is ${bytes}`), { code: 'runtime_integrity_mismatch' });
  }
  if (integrity && !integrityMatches(fs.readFileSync(file), integrity)) {
    throw Object.assign(new Error(`the bytes do not match the integrity the catalog states (${integrity})`), { code: 'runtime_integrity_mismatch' });
  }
  const result = kind === 'tgz' ? extractTarGzTo(file, into, { strip }) : extractZipTo(file, into);
  parentPort.postMessage({ ok: true, bytes, entries: result.entries });
  fs.rmSync(file, { force: true });
} catch (e) {
  parentPort.postMessage({ ok: false, message: e.message, code: e.code });
  fs.rmSync(file, { force: true });
}

// Zip reading and writing for plugin artifacts, with node built-ins only.
//
// A plugin artifact is a release zip: the plugin directory (manifest, adapter,
// extensions, the runtime it declares) packed as it should land. The hub installs
// one, so it must be able to READ a zip; the release tooling builds one, so it must
// be able to WRITE one. Both live here, next to each other, because they are two
// directions of one format and a mismatch between them would be a fact nobody sees
// until an install fails.
//
// What is deliberately not here: zip64, encryption, and the dozens of flags no
// release zip needs. What this cannot read it REFUSES BY NAME (the entry and the
// reason), never guesses — an archive that half-unpacks is worse than one refused.
//
// Writing is deterministic: fixed timestamps, deflate level 9, entries in the order
// given. Two builds of the same tree produce the same bytes, so a registry digest
// means what it says.

import fs from 'node:fs';
import path from 'node:path';
import zlib from 'node:zlib';
import crypto from 'node:crypto';

const SIG_LOCAL = 0x04034b50;
const SIG_CENTRAL = 0x02014b50;
const SIG_EOCD = 0x06054b50;
// 1980-01-01 00:00:00, the earliest a DOS timestamp can express: a fixed point, not
// a clock reading, so the same tree always packs to the same bytes.
const DOS_TIME = 0;
const DOS_DATE = 33;   // ((1980-1980)<<9) | (1<<5) | 1
const UNPACK_LIMIT = Number(process.env.AGENT_HUB_ARTIFACT_UNPACK_LIMIT || 8 * 1024 * 1024 * 1024);

function readAt(fd, offset, length) {
  const buf = Buffer.alloc(length);
  let read = 0;
  while (read < length) {
    const n = fs.readSync(fd, buf, read, length - read, offset + read);
    if (n <= 0) throw new Error(`unexpected end of file at ${offset + read}`);
    read += n;
  }
  return buf;
}

// A name inside an archive is untrusted input: it may point outside the archive
// root (`../`), be absolute, carry a Windows drive letter, or use backslashes that
// only look harmless. Returns the destination path, or null for a directory entry.
function entryTarget(destDir, name) {
  const normalised = name.replace(/\\/g, '/');
  const dir = normalised.endsWith('/');
  const parts = normalised.split('/').filter((p) => p !== '' && p !== '.');
  if (!parts.length) return null;
  if (/^[a-zA-Z]:/.test(parts[0])) throw new Error(`'${name}': absolute path with a drive letter`);
  if (parts.some((p) => p === '..')) throw new Error(`'${name}': path leaves the archive root`);
  const target = path.join(destDir, ...parts);
  const root = path.resolve(destDir) + path.sep;
  if (!(path.resolve(target) + (dir ? path.sep : '')).startsWith(root)) throw new Error(`'${name}': path escapes the destination`);
  return dir ? null : target;
}

/** Unpack a zip into destDir. Returns {entries, bytes}. Throws with the entry named. */
export function extractZipTo(zipPath, destDir) {
  const fd = fs.openSync(zipPath, 'r');
  try {
    const size = fs.fstatSync(fd).size;
    const tailLen = Math.min(size, 65_557);
    const tail = readAt(fd, size - tailLen, tailLen);
    let eocd = -1;
    for (let i = tail.length - 22; i >= 0; i--) if (tail.readUInt32LE(i) === SIG_EOCD) { eocd = i; break; }
    if (eocd < 0) throw new Error('no end-of-central-directory record: this file is not a zip');
    const count = tail.readUInt16LE(eocd + 10);
    const cdSize = tail.readUInt32LE(eocd + 12);
    const cdOff = tail.readUInt32LE(eocd + 16);
    if (cdOff === 0xffffffff || cdSize === 0xffffffff) throw new Error('zip64 archives are not read here');
    if (cdOff + cdSize > size) throw new Error('the central directory runs past the end of the file');
    const cd = readAt(fd, cdOff, cdSize);
    let p = 0;
    let bytes = 0;
    let entries = 0;
    for (let n = 0; n < count; n++) {
      if (p + 46 > cd.length || cd.readUInt32LE(p) !== SIG_CENTRAL) throw new Error('the central directory is malformed');
      const method = cd.readUInt16LE(p + 10);
      const compSize = cd.readUInt32LE(p + 20);
      const uncompSize = cd.readUInt32LE(p + 24);
      const nameLen = cd.readUInt16LE(p + 28);
      const extraLen = cd.readUInt16LE(p + 30);
      const commentLen = cd.readUInt16LE(p + 32);
      const externalAttr = cd.readUInt32LE(p + 38);
      const locOff = cd.readUInt32LE(p + 42);
      const name = cd.toString('utf8', p + 46, p + 46 + nameLen);
      p += 46 + nameLen + extraLen + commentLen;
      if (compSize === 0xffffffff || uncompSize === 0xffffffff || locOff === 0xffffffff) throw new Error(`'${name}': zip64 entries are not read here`);
      const unixMode = (externalAttr >>> 16) & 0xffff;
      if ((unixMode & 0xf000) === 0xa000) throw new Error(`'${name}': the archive contains a symbolic link`);
      const target = entryTarget(destDir, name);
      if (!target) continue;
      const local = readAt(fd, locOff, 30);
      if (local.readUInt32LE(0) !== SIG_LOCAL) throw new Error(`'${name}': the local header does not match the central directory`);
      const dataOff = locOff + 30 + local.readUInt16LE(26) + local.readUInt16LE(28);
      if (dataOff + compSize > size) throw new Error(`'${name}': the entry runs past the end of the file`);
      const raw = readAt(fd, dataOff, compSize);
      let data;
      if (method === 0) data = raw;
      else if (method === 8) data = zlib.inflateRawSync(raw);
      else throw new Error(`'${name}': compression method ${method} is neither stored nor deflate`);
      if (data.length !== uncompSize) throw new Error(`'${name}': unpacked to ${data.length} bytes, the entry says ${uncompSize}`);
      bytes += data.length;
      if (bytes > UNPACK_LIMIT) throw new Error(`the archive unpacks to more than ${UNPACK_LIMIT} bytes`);
      fs.mkdirSync(path.dirname(target), { recursive: true });
      fs.writeFileSync(target, data);
      entries++;
    }
    return { entries, bytes };
  } finally {
    fs.closeSync(fd);
  }
}

/**
 * Pack entries into a zip, streaming: each entry is deflated and written before the
 * next one is read, so a plugin with a 300 MB runtime packs without being held in
 * memory. Entries are `{name, contents}` (Buffer or string) or `{name, file}`
 * (read lazily); a plain object/array of [name, contents] pairs works too.
 */
export function writeZip(outPath, files, { level = 9 } = {}) {
  const list = (Array.isArray(files) ? files : Object.entries(files)).map((item) => {
    if (Array.isArray(item)) return { name: item[0], contents: item[1] };
    if (item && typeof item === 'object' && 'name' in item) return item;
    throw new Error(`writeZip: cannot read entry ${JSON.stringify(item)}`);
  });
  const fd = fs.openSync(outPath, 'w');
  const central = [];
  let offset = 0;
  try {
    for (const entry of list) {
      const buf = Buffer.isBuffer(entry.contents) ? entry.contents : entry.contents !== undefined ? Buffer.from(entry.contents) : fs.readFileSync(entry.file);
      const nameBuf = Buffer.from(entry.name, 'utf8');
      if (entry.name.endsWith('/')) throw new Error(`writeZip: '${entry.name}' is a directory entry; names are files`);
      const deflated = zlib.deflateRawSync(buf, { level });
      const useDeflate = deflated.length < buf.length;
      const body = useDeflate ? deflated : buf;
      const crc = crcOf(buf);
      const local = Buffer.alloc(30);
      local.writeUInt32LE(SIG_LOCAL, 0);
      local.writeUInt16LE(20, 4);            // version needed
      local.writeUInt16LE(0x0800, 6);        // utf8 names
      local.writeUInt16LE(useDeflate ? 8 : 0, 8);
      local.writeUInt16LE(DOS_TIME, 10);
      local.writeUInt16LE(DOS_DATE, 12);
      local.writeUInt32LE(crc, 14);
      local.writeUInt32LE(body.length, 18);
      local.writeUInt32LE(buf.length, 22);
      local.writeUInt16LE(nameBuf.length, 26);
      fs.writeSync(fd, local);
      fs.writeSync(fd, nameBuf);
      fs.writeSync(fd, body);
      central.push({ nameBuf, method: useDeflate ? 8 : 0, crc, compSize: body.length, size: buf.length, offset });
      offset += local.length + nameBuf.length + body.length;
    }
    const cdStart = offset;
    for (const f of central) {
      const rec = Buffer.alloc(46);
      rec.writeUInt32LE(SIG_CENTRAL, 0);
      rec.writeUInt16LE(20, 4);              // version made by
      rec.writeUInt16LE(20, 6);              // version needed
      rec.writeUInt16LE(0x0800, 8);
      rec.writeUInt16LE(f.method, 10);
      rec.writeUInt16LE(DOS_TIME, 12);
      rec.writeUInt16LE(DOS_DATE, 14);
      rec.writeUInt32LE(f.crc, 16);
      rec.writeUInt32LE(f.compSize, 20);
      rec.writeUInt32LE(f.size, 24);
      rec.writeUInt16LE(f.nameBuf.length, 28);
      rec.writeUInt32LE((0o100644 << 16) >>> 0, 38); // a regular file, readable
      rec.writeUInt32LE(f.offset, 42);
      fs.writeSync(fd, rec);
      fs.writeSync(fd, f.nameBuf);
      offset += rec.length + f.nameBuf.length;
    }
    const eocd = Buffer.alloc(22);
    eocd.writeUInt32LE(SIG_EOCD, 0);
    eocd.writeUInt16LE(list.length, 8);
    eocd.writeUInt16LE(list.length, 10);
    eocd.writeUInt32LE(offset - cdStart, 12);
    eocd.writeUInt32LE(cdStart, 16);
    fs.writeSync(fd, eocd);
    return { entries: list.length, bytes: offset - cdStart };
  } finally {
    fs.closeSync(fd);
  }
}

// node's zlib.crc32 exists only on newer builds; the fallback is the same table
// algorithm, so an older node writes the same bytes rather than refusing to pack.
// Whichever runs, the result is made unsigned: the field is 32 bits and a negative
// number writes as a RangeError, which is how this was found.
function crcOf(buf) {
  const value = zlib.crc32 ? zlib.crc32(buf) : crc32(buf);
  return value >>> 0;
}

function crc32(buf) {
  let c = ~0;
  for (let i = 0; i < buf.length; i++) {
    c ^= buf[i];
    for (let k = 0; k < 8; k++) c = (c >>> 1) ^ (0xedb88320 & -(c & 1));
  }
  return (~c) >>> 0;
}

// --- tar.gz, for the official distributions that are published as tarballs -------
//
// npm publishes packages as `package.tgz` (gzip + tar), and the vendor's dependency
// closure — resolved by npm at release time and recorded with the registry's own
// `integrity` — is a list of exactly those tarballs. Reading them here is what makes
// an install possible on a machine that has no npm on it at all.
//
// Handled: ustar and gnu/old-gnu headers, pax extended headers (`path=`), gnu long
// names ('L'), directories, and base-256 sizes. Refused by name: links (symbolic or
// hard), absolute paths, and anything climbing out of the destination.
function tarNumber(buf, offset, length) {
  if (buf[offset] & 0x80) {
    // base-256: a big-endian integer in the remaining bytes
    let value = 0;
    for (let i = offset + 1; i < offset + length; i++) value = value * 256 + buf[i];
    return value;
  }
  const text = buf.toString('latin1', offset, offset + length).replace(/\0.*$/, '').trim();
  return text === '' ? 0 : parseInt(text, 8);
}

function tarEntryTarget(destDir, name) {
  const normalised = name.replace(/\\/g, '/').replace(/^\.\//, '');
  const parts = normalised.split('/').filter((p) => p !== '' && p !== '.');
  if (!parts.length) return null;
  if (/^[a-zA-Z]:/.test(parts[0])) throw new Error(`'${name}': absolute path with a drive letter`);
  if (parts.some((p) => p === '..')) throw new Error(`'${name}': path leaves the archive root`);
  const target = path.join(destDir, ...parts);
  const root = path.resolve(destDir) + path.sep;
  if (!path.resolve(target).startsWith(root)) throw new Error(`'${name}': path escapes the destination`);
  return target;
}

/** Read a .tar.gz (or .tgz) into memory: [{name, data}] with directories dropped. */
export function readTarGz(tarPath) {
  const raw = zlib.gunzipSync(fs.readFileSync(tarPath));
  const out = [];
  let offset = 0;
  let longName = null;
  let paxName = null;
  while (offset + 512 <= raw.length) {
    const header = raw.subarray(offset, offset + 512);
    offset += 512;
    if (header.every((byte) => byte === 0)) break;
    const type = String.fromCharCode(header[156] || 0x30);
    const size = tarNumber(header, 124, 12);
    const body = raw.subarray(offset, offset + size);
    offset += Math.ceil(size / 512) * 512;
    if (type === 'L') { longName = body.toString('utf8').replace(/\0.*$/, ''); continue; }
    if (type === 'x' || type === 'g') {
      for (const line of body.toString('utf8').split('\n')) {
        const match = /^(\d+) path=(.*)$/.exec(line);
        if (match) paxName = match[2];
      }
      continue;
    }
    const extendedName = paxName ?? longName;
    const name = extendedName ?? header.toString('utf8', 0, 100).replace(/\0.*$/, '');
    const prefix = header.toString('utf8', 345, 500).replace(/\0.*$/, '');
    paxName = null;
    longName = null;
    // PAX/GNU names replace the entire path; ustar prefix applies only to the header name.
    const full = extendedName != null ? name : prefix ? `${prefix}/${name}` : name;
    if (type === '2' || type === '1') throw new Error(`'${full}': the archive contains a link`);
    if (type === '5' || type === '0' || type === '\0' || type === '') {
      if (full.endsWith('/') || type === '5') continue;
      out.push({ name: full, data: body, mode: tarNumber(header, 100, 8) & 0o777 });
      continue;
    }
    throw new Error(`'${full}': tar entry type '${type}' is not read here`);
  }
  return out;
}

/** Unpack a .tar.gz into destDir, dropping the first `strip` path segments. */
export function extractTarGzTo(tarPath, destDir, { strip = 0 } = {}) {
  const entries = readTarGz(tarPath);
  let bytes = 0;
  let written = 0;
  for (const entry of entries) {
    const parts = entry.name.replace(/\\/g, '/').split('/').filter((p) => p !== '' && p !== '.');
    const kept = parts.slice(strip);
    if (!kept.length) continue;
    // The same safety as the zip reader, applied to the path we are about to create.
    const target = tarEntryTarget(destDir, kept.join('/'));
    if (!target) continue;
    bytes += entry.data.length;
    if (bytes > UNPACK_LIMIT) throw new Error(`the archive unpacks to more than ${UNPACK_LIMIT} bytes`);
    fs.mkdirSync(path.dirname(target), { recursive: true });
    fs.writeFileSync(target, entry.data);
    if (process.platform !== 'win32') fs.chmodSync(target, entry.mode & 0o777);
    written++;
  }
  return { entries: written, bytes };
}

/**
 * Is this what the registry says it is? npm's `integrity` is a Subresource Integrity
 * string (one or more `alg-base64` hashes, space separated). The bytes are hashed and
 * compared against every entry this build can compute; a string with no algorithm we
 * know is refused rather than skipped.
 */
export function integrityMatches(buf, integrity) {
  const wanted = String(integrity).trim().split(/\s+/).filter(Boolean);
  if (!wanted.length) throw new Error('the registry stated no integrity for this artifact');
  let judged = 0;
  for (const item of wanted) {
    const [alg, encoded] = item.split('-', 2);
    const nodeAlg = alg === 'sha512' ? 'sha512' : alg === 'sha384' ? 'sha384' : alg === 'sha256' ? 'sha256' : alg === 'sha1' ? 'sha1' : null;
    if (!nodeAlg || !encoded) continue;
    judged++;
    const computed = crypto.createHash(nodeAlg).update(buf).digest('base64');
    const normalised = encoded.split('?')[0];
    if (computed === normalised) return true;
  }
  if (!judged) throw new Error(`the registry's integrity '${integrity}' uses no algorithm this hub can compute`);
  return false;
}

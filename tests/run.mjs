// Run the hub's test files. Each is a process; a non-zero exit is a failure. The header
// stamps the contract this run was written against, so a run is attributable to a version.
//
// Files are grouped by what they attack (lifecycle/, events/, hostile/, recovery/); the
// shared harness is lib/. A single group can be run: `node tests/run.mjs lifecycle`.
import fs from 'node:fs';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { contractStamp } from './lib/hub-harness.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const only = process.argv[2];

function collect(dir) {
  const out = [];
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    if (entry.name === 'lib' || entry.name.startsWith('_')) continue;
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) out.push(...collect(full));
    else if (entry.name.endsWith('.mjs') && entry.name !== 'run.mjs') out.push(full);
  }
  return out;
}
const files = collect(here)
  .map((f) => path.relative(here, f).split(path.sep).join(String.fromCharCode(47)))
  .filter((f) => !only || f.includes(only))
  .sort();

const stamp = contractStamp();
console.log(`hub tests — contract protocol=${stamp.protocol} version=${stamp.version} sha=${stamp.contractSha} build=${stamp.buildId}`);
console.log(`node ${process.version} ${process.platform}/${process.arch}`);
console.log(`${files.length} file(s)${only ? ` matching '${only}'` : ''}\n`);

let failed = 0;
for (const f of files) {
  console.log(`=== ${f} ===`);
  const r = spawnSync(process.execPath, [path.join(here, f)], { stdio: 'inherit' });
  if (r.status !== 0) failed++;
  console.log('');
}
console.log(failed ? `${failed}/${files.length} file(s) failed` : `${files.length}/${files.length} files passed`);
process.exit(failed ? 1 : 0);

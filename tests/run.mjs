// Run the hub's test files. Each is a process; a non-zero exit is a failure. The header
// stamps the contract this run was written against, so a run is attributable to a version.
import fs from 'node:fs';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { contractStamp } from './lib/hub-harness.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const only = process.argv[2];
const files = fs.readdirSync(here).filter((f) => f.endsWith('.mjs') && f !== 'run.mjs' && !f.startsWith('_'))
  .filter((f) => !only || f.includes(only)).sort();

const stamp = contractStamp();
console.log(`hub tests — contract protocol=${stamp.protocol} version=${stamp.version} sha=${stamp.contractSha} build=${stamp.buildId}`);
console.log(`node ${process.version} ${process.platform}/${process.arch}\n`);

let failed = 0;
for (const f of files) {
  console.log(`=== ${f} ===`);
  const r = spawnSync(process.execPath, [path.join(here, f)], { stdio: 'inherit' });
  if (r.status !== 0) failed++;
  console.log('');
}
console.log(failed ? `${failed}/${files.length} file(s) failed` : `${files.length}/${files.length} files passed`);
process.exit(failed ? 1 : 0);

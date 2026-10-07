// Run every test file under tests/ (except lib/). Each is a process; a non-zero exit is a
// failure. This hub's tests are ADVERSARIAL: they attack the product through its real
// surfaces with real inputs, and a red is the product's fault until proven otherwise.
import fs from 'node:fs';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
const here = path.dirname(fileURLToPath(import.meta.url));
const only = process.argv[2];
function collect(dir) {
  const out = [];
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    if (e.name === 'lib' || e.name.startsWith('_')) continue;
    const full = path.join(dir, e.name);
    if (e.isDirectory()) out.push(...collect(full));
    else if (e.name.endsWith('.mjs') && e.name !== 'run.mjs') out.push(full);
  }
  return out;
}
const files = collect(here).map((f) => path.relative(here, f).split(path.sep).join('/')).filter((f) => !only || f.includes(only)).sort();
console.log(`hub tests: ${files.length} file(s)${only ? ` matching '${only}'` : ''}\n`);
let failed = 0;
for (const f of files) { console.log(`=== ${f} ===`); const r = spawnSync(process.execPath, [path.join(here, f)], { stdio: 'inherit' }); if (r.status !== 0) failed++; console.log(''); }
console.log(failed ? `${failed}/${files.length} file(s) failed` : `${files.length}/${files.length} files passed`);
process.exit(failed ? 1 : 0);

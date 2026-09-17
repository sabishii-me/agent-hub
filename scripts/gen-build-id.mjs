// Writes core/build-id.json from git HEAD. Run before any test/build.
// Inconsistent build ids across core/cli/tauri/relay = build_mismatch.
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const OUT = path.resolve(HERE, '..', 'build-id.json');

let hash = 'dev';
try {
  hash = execFileSync('git', ['rev-parse', 'HEAD'], { cwd: path.resolve(HERE, '..', '..') })
    .toString().trim();
} catch {
  hash = process.env.PRTS_BUILD_ID || 'dev';
}

fs.writeFileSync(OUT, JSON.stringify({ buildId: hash, generatedAt: new Date().toISOString() }, null, 2));
process.stdout.write(`${hash}\n`);

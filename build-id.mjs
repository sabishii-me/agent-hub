// Build-id lockstep: one version fact (git hash). Consumers must present the
// same build id; mismatch = disconnect. No compatibility branches.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = path.dirname(fileURLToPath(import.meta.url));

export function getBuildId() {
  if (process.env.AGENT_HUB_BUILD_ID) return process.env.AGENT_HUB_BUILD_ID;
  const file = path.join(HERE, 'build-id.json');
  try {
    const j = JSON.parse(fs.readFileSync(file, 'utf8'));
    if (j && j.buildId) return j.buildId;
  } catch {}
  return 'dev';
}

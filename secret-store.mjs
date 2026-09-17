// SecretStore — the ONLY place secrets are persisted. Zero plaintext on disk.
//   Windows: DPAPI (CurrentUser) via PowerShell; ciphertext lands in DATA_DIR/secrets/<name>.dpapi
//   non-Windows: AES-256-GCM with AGENT_HUB_SECRET_KEY (container secret injection); refuses without a key
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const DATA_DIR = process.env.AGENT_HUB_DATA_DIR || path.join(requireHome(), '.sabishii-me/agent-hub');
function requireHome() {
  return process.env.USERPROFILE || process.env.HOME || '.';
}
const SECRETS = path.join(DATA_DIR, 'secrets');

const DPAPI_ENCRYPT = `
Add-Type -AssemblyName System.Security
$in = [Console]::In.ReadToEnd()
$b = [Text.Encoding]::UTF8.GetBytes($in)
$enc = [Security.Cryptography.ProtectedData]::Protect($b, $null, [Security.Cryptography.DataProtectionScope]::CurrentUser)
[Console]::Out.Write([Convert]::ToBase64String($enc))
`;
const DPAPI_DECRYPT = `
Add-Type -AssemblyName System.Security
$in = [Console]::In.ReadToEnd().Trim()
$b = [Convert]::FromBase64String($in)
$dec = [Security.Cryptography.ProtectedData]::Unprotect($b, $null, [Security.Cryptography.DataProtectionScope]::CurrentUser)
[Console]::Out.Write([Text.Encoding]::UTF8.GetString($dec))
`;

function dpapi(code, input) {
  const r = spawnSync('powershell', ['-NoProfile', '-ExecutionPolicy', 'Bypass', '-Command', code], {
    input,
    encoding: 'utf8',
    windowsHide: true,
  });
  if (r.status !== 0) throw new Error(`dpapi failed: ${(r.stderr || '').trim()}`);
  return r.stdout;
}

function secretPath(name) {
  return path.join(SECRETS, `${name}.dpapi`);
}

export function storeSecret(name, value) {
  fs.mkdirSync(SECRETS, { recursive: true });
  if (process.platform === 'win32') {
    fs.writeFileSync(secretPath(name), dpapi(DPAPI_ENCRYPT, value));
  } else {
    const key = process.env.AGENT_HUB_SECRET_KEY;
    if (!key) throw new Error('AGENT_HUB_SECRET_KEY required for SecretStore on non-Windows');
    const iv = crypto.randomBytes(12);
    const cipher = crypto.createCipheriv('aes-256-gcm', crypto.createHash('sha256').update(key).digest(), iv);
    const enc = Buffer.concat([cipher.update(value, 'utf8'), cipher.final()]);
    const tag = cipher.getAuthTag();
    fs.writeFileSync(secretPath(name), JSON.stringify({ iv: iv.toString('base64'), tag: tag.toString('base64'), data: enc.toString('base64') }));
  }
}

export function getSecret(name) {
  const p = secretPath(name);
  if (!fs.existsSync(p)) return null;
  if (process.platform === 'win32') {
    return dpapi(DPAPI_DECRYPT, fs.readFileSync(p, 'utf8'));
  }
  const key = process.env.AGENT_HUB_SECRET_KEY;
  if (!key) throw new Error('AGENT_HUB_SECRET_KEY required for SecretStore on non-Windows');
  const j = JSON.parse(fs.readFileSync(p, 'utf8'));
  const decipher = crypto.createDecipheriv('aes-256-gcm', crypto.createHash('sha256').update(key).digest(), Buffer.from(j.iv, 'base64'));
  decipher.setAuthTag(Buffer.from(j.tag, 'base64'));
  return Buffer.concat([decipher.update(Buffer.from(j.data, 'base64')), decipher.final()]).toString('utf8');
}

export function deleteSecret(name) {
  fs.rmSync(secretPath(name), { force: true });
}

export function listSecretNames() {
  if (!fs.existsSync(SECRETS)) return [];
  return fs.readdirSync(SECRETS).filter((f) => f.endsWith('.dpapi')).map((f) => f.slice(0, -'.dpapi'.length));
}

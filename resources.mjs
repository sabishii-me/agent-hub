// Session-authorized read-only skill resources. No filesystem paths in responses.
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
const bad = () => Object.assign(new Error('invalid resource URI'), { code: 'validation_failed' });
const unavailable = () => Object.assign(new Error('resource unavailable or not authorized'), { code: 'not_found' });
const segment = s => typeof s === 'string' && s.length > 0 && s !== '.' && s !== '..' && !/[\/:\x00-\x1f<>"|?*]/.test(s) && !/[. ]$/.test(s) && !/^(con|prn|aux|nul|com[0-9]|lpt[0-9])(?:\.|$)/i.test(s);
export function parseSkillUri(uri) {
  if (typeof uri !== 'string' || uri.length > 4096 || !uri.startsWith('skills://') || /[?#]/.test(uri)) throw bad();
  let parts;
  try { parts = uri.slice(9).split('/').map(decodeURIComponent); } catch { throw bad(); }
  if (parts.length < 2 || !parts.every(segment)) throw bad();
  return { id: parts[0], parts, uri: 'skills://' + parts.map(encodeURIComponent).join('/') };
}
function authorize(root, selected, id) {
  if (!segment(id)) throw unavailable();
  if (Array.isArray(selected) && !selected.includes(id)) throw unavailable();
  const dir = path.join(root, id);
  try { const s=fs.lstatSync(dir); if (!s.isDirectory() || s.isSymbolicLink()) throw unavailable(); } catch { throw unavailable(); }
}
export function listSkillResources(root, selected) {
  if (!fs.existsSync(root)) return [];
  return fs.readdirSync(root).flatMap(id => {
    try {
      authorize(root, selected, id);
      const file = path.join(root,id,'SKILL.md'); const s=fs.lstatSync(file);
      if (!s.isFile() || s.isSymbolicLink()) return [];
      return [{uri:`skills://${encodeURIComponent(id)}/SKILL.md`,name:id,mimeType:'text/markdown'}];
    } catch { return []; }
  });
}
export function readSkillResource(root, selected, uri) {
  const parsed = parseSkillUri(uri);
  authorize(root, selected, parsed.id);
  let current = root;
  try {
    const base = fs.realpathSync(root);
    for (const part of parsed.parts) {
      current = path.join(current,part);
      if (fs.lstatSync(current).isSymbolicLink()) throw unavailable();
      const real = fs.realpathSync(current);
      const rel = path.relative(base,real);
      if (rel === '..' || rel.startsWith('..'+path.sep) || path.isAbsolute(rel)) throw unavailable();
    }
    const fd = fs.openSync(current, fs.constants.O_RDONLY | (fs.constants.O_NOFOLLOW || 0));
    try {
      const st=fs.fstatSync(fd);
      if (!st.isFile() || st.size > 512*1024) throw unavailable();
      const buffer=Buffer.alloc(512*1024+1);const size=fs.readSync(fd,buffer,0,buffer.length,0);
      if (size>512*1024) throw unavailable();
      const bytes=buffer.subarray(0,size);
      const content=new TextDecoder('utf-8',{fatal:true}).decode(bytes);
      if (content.includes(String.fromCharCode(0))) throw unavailable();
      return {uri:parsed.uri,mimeType:current.endsWith('.md')?'text/markdown':'text/plain',version:crypto.createHash('sha256').update(bytes).digest('hex'),content};
    } finally {fs.closeSync(fd);}
  } catch { throw unavailable(); }
}

// contract/v1.json + contract/errors.json  ->  contract/openapi.json
//
// A PROJECTION, never a second source: the contract file stays authoritative, this
// script only restates it in a format tools can consume (client generators, Swagger
// UI, request validators). Two consequences:
//
//   * it is deterministic — same inputs, same bytes — so a freshness check can
//     compare the committed file with a fresh generation and name the difference;
//   * anything it cannot express it must SAY, not skip: an unknown type token or a
//     shape it does not understand throws with the endpoint that carries it.
//
// What the projection translates (the contract's compact notation):
//   "string?"          -> {"type":"string"} and the key is left out of `required`
//   "integer|null"     -> {"type":["integer","null"]}
//   "string[]"         -> {"type":"array","items":{"type":"string"}}
//   "'harness'"        -> {"const":"harness"}
//   {"$ref":"#/defs/x"}-> {"$ref":"#/components/schemas/x"}
//     {file...}        -> {file} plus a parameter that says it is a path remainder
//
// What stays outside, and where it lives instead: the adapter protocol
// (contract/adapter-v1.json — stdio JSON-RPC, not HTTP), and the "why" of each
// decision (the contract's `description` fields, carried through verbatim).
import fs from 'node:fs';
import path from 'node:path';

const HERE = path.resolve(import.meta.dirname, '..');
const CONTRACT = path.join(HERE, 'contract', 'v1.json');
const ERRORS = path.join(HERE, 'contract', 'errors.json');
const OUT = path.join(HERE, 'contract', 'openapi.json');

const contract = JSON.parse(fs.readFileSync(CONTRACT, 'utf8'));
const errors = JSON.parse(fs.readFileSync(ERRORS, 'utf8')).errors;

const fail = (where, what) => { throw new Error(`emit-openapi: ${where}: ${what}`); };

// --- the notation ------------------------------------------------------------
const SCALARS = new Set(['string', 'integer', 'number', 'boolean', 'object', 'array', 'null']);

// The contract carries two flavours in one file: `defs` are already JSON Schema
// (`type`/`required`/`properties`), while an endpoint's request/response blocks use
// the compact notation (keys ARE the properties, `?` means optional). Telling them
// apart by the presence of a schema keyword is the only honest way: guessing wrong
// would turn `required: [...]` into a property called `required`.
const looksLikeSchema = (v) => v && typeof v === 'object' && !Array.isArray(v)
  && ('type' in v || 'properties' in v || '$ref' in v || 'allOf' in v || 'oneOf' in v || 'anyOf' in v || 'const' in v || 'enum' in v);

function schemaOf(value, where) {
  if (value === null || value === undefined) fail(where, 'no shape at all');
  if (typeof value === 'object') {
    if (Array.isArray(value)) fail(where, 'a bare array is not a shape; say what is in it');
    if (typeof value.$ref === 'string') {
      if (!value.$ref.startsWith('#/defs/')) fail(where, `unsupported $ref ${value.$ref}`);
      return { $ref: value.$ref.replace('#/defs/', '#/components/schemas/') };
    }
    if (looksLikeSchema(value)) {
      const out = {};
      for (const [k, v] of Object.entries(value)) {
        if (k === 'properties') {
          out.properties = {};
          for (const [name, spec] of Object.entries(v)) out.properties[name] = schemaOf(spec, `${where}.${name}`);
        } else if (k === 'items') out.items = schemaOf(v, `${where}.items`);
        else if (k === 'allOf' || k === 'oneOf' || k === 'anyOf') out[k] = v.map((s, i) => schemaOf(s, `${where}.${k}[${i}]`));
        else out[k] = v;
      }
      return out;
    }
    // the compact object shape: keys are properties, a trailing `?` means optional
    const properties = {};
    const required = [];
    for (const [key, v] of Object.entries(value)) {
      if (key === 'description') continue;
      const optional = key.endsWith('?');
      const name = optional ? key.slice(0, -1) : key;
      properties[name] = schemaOf(v, `${where}.${name}`);
      if (!optional) required.push(name);
    }
    const out = { type: 'object', properties };
    if (required.length) out.required = required;
    if (typeof value.description === 'string') out.description = value.description;
    return out;
  }
  if (typeof value !== 'string') {
    // a literal in a shape is a constant: {ok: true} means ok is always true
    if (typeof value === 'boolean' || typeof value === 'number') return { const: value };
    fail(where, `shape must be a string, a literal or an object, got ${typeof value}`);
  }

  let text = value;
  let description;
  // a type string that is actually prose is refused (the contract keeps prose in
  // `description`; this is the check that keeps it that way)
  const prose = text.match(/\s\(|\s—\s/);
  if (prose) fail(where, `type string carries prose: ${JSON.stringify(text)}`);
  const optional = text.endsWith('?');
  if (optional) text = text.slice(0, -1);
  const union = text.split('|').map((part) => part.trim()).filter(Boolean);
  const types = [];
  let items;
  let constValue;
  let enumValues;
  for (const part of union) {
    if (part.startsWith("'") && part.endsWith("'")) { (enumValues = enumValues || []).push(part.slice(1, -1)); continue; }
    if (part.startsWith('{')) { fail(where, `inline object in a type string: ${part}`); }
    let base = part;
    if (base.endsWith('[]')) {
      base = base.slice(0, -2);
      if (!SCALARS.has(base)) fail(where, `array of what? ${part}`);
      items = items || { type: base };
      continue;
    }
    if (base.startsWith('[') || part === 'object?') fail(where, `unsupported token: ${part}`);
    if (!SCALARS.has(base)) fail(where, `unknown type token: ${part}`);
    types.push(base);
  }
  const out = {};
  if (enumValues) {
    out.enum = enumValues;
    if (enumValues.length === 1) { delete out.enum; out.const = enumValues[0]; }
  } else if (items) {
    out.type = types.length === 1 ? types[0] : types;
    out.items = items;
  } else {
    out.type = types.length === 1 ? types[0] : types;
  }
  if (description) out.description = description;
  return out;
}

const operationId = (method, p) => method.toLowerCase() + p
  .replace(/^\/v1\//, '/')
  .split('/').filter(Boolean)
  .map((seg) => {
    if (seg.startsWith('{')) {
      const name = seg.replace(/[{}]/g, '').replace(/\.\.\.$/, '');
      return 'By' + name.charAt(0).toUpperCase() + name.slice(1);
    }
    return seg.charAt(0).toUpperCase() + seg.slice(1);
  }).join('')
  .replace(/[^A-Za-z0-9]/g, '');

// --- components ---------------------------------------------------------------
const schemas = {};
for (const [name, spec] of Object.entries(contract.defs)) schemas[name] = schemaOf(spec, `defs.${name}`);

// every event becomes a named payload schema plus a frame object, so a generator
// can produce a union type named after the event
const eventFrames = {};
for (const ev of contract.events) {
  const payloadName = `event_${ev.name.replace(/\./g, '_')}`;
  schemas[payloadName] = schemaOf(ev.payload, `events.${ev.name}.payload`);
  if (ev.description) schemas[payloadName].description = ev.description;
  eventFrames[ev.name] = {
    type: 'object',
    required: ['event', 'data'],
    properties: { event: { const: ev.name }, data: { $ref: `#/components/schemas/${payloadName}` } },
  };
}

// the statuses the hub can answer with, each carrying the codes the table maps to it
const byStatus = {};
for (const [code, row] of Object.entries(errors)) {
  const status = String(row.http);
  (byStatus[status] = byStatus[status] || []).push(`${code}${row.retryable ? ' (retryable)' : ''}`);
}
const errorResponses = {};
for (const [status, codes] of Object.entries(byStatus).sort((a, b) => Number(a[0]) - Number(b[0]))) {
  errorResponses[status] = {
    description: `error: ${codes.join(', ')} — the master table is contract/errors.json`,
    content: { 'application/json': { schema: { $ref: '#/components/schemas/error' } } },
  };
}

// --- paths --------------------------------------------------------------------
const paths = {};
for (const e of contract.endpoints) {
  const openapiPath = e.path.replace(/\{(\w+)\.\.\.\}/g, '{$1}');
  const op = {
    operationId: operationId(e.method, e.path),
    tags: [e.path.replace(/^\/v1\//, '').split('/')[0] || 'root'],
    description: e.description,
    parameters: [],
    responses: {},
  };
  if (e.auth === false) op.security = [];
  for (const m of e.path.matchAll(/\{(\w+)(\.\.\.)?\}/g)) {
    const [, name, rest] = m;
    op.parameters.push({
      name, in: 'path', required: true,
      schema: { type: 'string' },
      ...(rest ? { description: 'path remainder: everything after this segment, slashes included' } : {}),
    });
  }
  for (const [name, spec] of Object.entries(e.query || {})) {
    op.parameters.push({ name, in: 'query', required: false, schema: schemaOf(spec, `${e.method} ${e.path} query.${name}`), ...(spec.description ? { description: spec.description } : {}) });
  }
  if (e.request !== undefined) {
    op.requestBody = {
      required: true,
      content: { 'application/json': { schema: Object.keys(e.request || {}).length ? schemaOf(e.request, `${e.method} ${e.path} request`) : { type: 'object' } } },
    };
  }
  const okSchema = e.response === undefined || e.response === null ? null : schemaOf(e.response, `${e.method} ${e.path} response`);
  const content = { 'application/json': okSchema ? { schema: okSchema } : { schema: {} } };
  if (e.events && e.events.length) {
    // Server-Sent Events: the response body is frames, and the schema below is what
    // the `data:` line of each frame holds. There is no standard way to model the
    // event names themselves, so the union is keyed on a `const` per event.
    content['text/event-stream'] = {
      schema: { oneOf: e.events.map((name) => eventFrames[name] || fail(`${e.method} ${e.path}`, `event ${name} is not declared`)) },
      description: `Each SSE frame is "event: <name>\\ndata: <json>\\n\\n" — the JSON is the frame object below, whose "event" is the name and whose "data" is the payload. This stream carries: ${e.events.join(', ')}.`,
    };
  }
  if (e.accepted) {
    // A long route (ADR-0009): the request is ACCEPTED, not answered with the finished
    // resource. 202 + Location, per e.accepted (RFC 9110 15.3.3 / 10.2.2). The finished
    // resource is at the Location instead.
    op.responses[String(e.accepted.status)] = {
      description: e.accepted.description || 'accepted; the work runs in the background',
      headers: { location: { description: 'where the resource whose state shows the outcome is read', schema: { type: 'string' }, example: e.accepted.location } },
      content: { 'application/json': { schema: { type: 'object', properties: { accepted: { type: 'boolean' }, location: { type: 'string' } } } } },
    };
  } else {
    op.responses['200'] = { description: 'ok', content };
  }
  Object.assign(op.responses, errorResponses);
  paths[openapiPath] = { ...(paths[openapiPath] || {}), [e.method.toLowerCase()]: op };
}

// --- the document -------------------------------------------------------------
const doc = {
  openapi: '3.1.0',
  info: {
    title: contract.title || 'Agent hub /v1',
    version: String(contract.version),
    description: [
      'GENERATED from contract/v1.json and contract/errors.json by scripts/emit-openapi.mjs — do not edit this file; edit the contract and regenerate.',
      'This document covers the HTTP surface only. The adapter protocol (the hub talking to a harness plugin over stdio JSON-RPC) is NOT HTTP and lives in contract/adapter-v1.json, which is machine-checked against every manifest, every request the hub sends, every event an adapter emits and every event the hub handles.',
      "The running hub reports the exact contract it was built against at GET /v1/hub/surface (`contract.sha256`), so a client can prove which document it is talking to.",
      contract.note || '',
    ].filter(Boolean).join('\n\n'),
  },
  servers: [{
    url: 'http://127.0.0.1:{port}',
    description: 'the hub binds a loopback port chosen at start; the port and the bearer token are in <DATA_DIR>/endpoint.json',
    variables: { port: { default: '0', description: 'the port the hub announced' } },
  }],
  security: [{ bearerAuth: [] }],
  paths,
  components: {
    securitySchemes: { bearerAuth: { type: 'http', scheme: 'bearer', description: 'the token from <DATA_DIR>/endpoint.json. Discovery (GET /v1/harnesses) needs no token; everything else answers 401 unauthorized without it.' } },
    schemas,
  },
};

const text = JSON.stringify(doc, null, 2) + '\n';
fs.writeFileSync(OUT, text);
const ops = Object.values(paths).reduce((n, item) => n + Object.keys(item).length, 0);
process.stdout.write(`emit-openapi: ${ops} operations, ${Object.keys(schemas).length} schemas, ${Object.keys(paths).length} paths -> ${path.relative(HERE, OUT)} (${text.length} bytes)\n`);

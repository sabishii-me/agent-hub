// The hub's HTTP transport, on Hono (ADR-0010). The framework owns the server, routing,
// request parsing, and the streaming response - the parts that were hand-written and
// where the connection defects lived. The business handlers are unchanged: they still
// receive { req, res, url, params, body } and answer through `res`, because the model
// change (ADR-0009) is separate from the transport change (ADR-0010) and the two must
// not be tangled.
//
// `res` here is a small object Hono-shaped to look like the Node ServerResponse the
// handlers already use: writeHead/end/write/writableEnded and the 'close'/'error'/'finish'
// events. It is a Response the runtime streams, so back-pressure and disconnect handling
// are Hono's, not ours.

import { Hono } from 'hono';
import { streamSSE } from 'hono/streaming';

// A Node-ServerResponse-shaped view over one Hono request. A handler writes headers and
// a body once; a streaming handler (SSE) writes many chunks. Both are one Response.
export function makeRes() {
  let status = 200;
  const headers = new Map();
  const listeners = { close: [], error: [], finish: [] };
  let resolveResponse;
  const responseReady = new Promise((r) => { resolveResponse = r; });
  let ended = false;
  let controller = null;
  let stream = null;

  const emit = (name) => { for (const fn of listeners[name]) { try { fn(); } catch { /* a listener's failure is not the response's */ } } };

  const res = {
    get writableEnded() { return ended; },
    writeHead(code, hdrs) {
      status = code;
      if (hdrs) for (const [k, v] of Object.entries(hdrs)) headers.set(k.toLowerCase(), String(v));
      return res;
    },
    write(chunk) {
      // First write: this is a streaming response. Hand Hono a streaming body whose
      // writer forwards to this same object, so `res.write` keeps meaning "send bytes".
      if (!stream) {
        stream = new ReadableStream({
          start(c) { controller = c; },
          cancel() { ended = true; emit('close'); },
        });
        resolveResponse(new Response(stream, { status, headers: Object.fromEntries(headers) }));
      }
      if (controller && !ended) controller.enqueue(new TextEncoder().encode(chunk));
      return true;
    },
    end(chunk) {
      if (ended) return res;
      if (chunk !== undefined && !stream) {
        // A whole-body write (js/json): a completed Response, not a stream.
        ended = true;
        const body = typeof chunk === 'string' ? chunk : String(chunk);
        resolveResponse(new Response(body, { status, headers: Object.fromEntries(headers) }));
        emit('finish');
        return res;
      }
      if (chunk !== undefined && controller && !ended) controller.enqueue(new TextEncoder().encode(chunk));
      ended = true;
      if (controller) { try { controller.close(); } catch { /* already closed */ } }
      else resolveResponse(new Response(null, { status, headers: Object.fromEntries(headers) }));
      emit('finish');
      return res;
    },
    on(name, fn) { if (listeners[name]) listeners[name].push(fn); return res; },
    // The adapter's break-out: a handler that never touches `res` (returns a value)
    // still needs a response, and one that streams needs the request aborted when the
    // client goes away.
    _abort() { if (!ended) { ended = true; if (controller) { try { controller.close(); } catch {} } emit('close'); } },
    _response: responseReady,
    _settled: () => ended,
  };
  return res;
}

// Turn one Hono request into the { req, res, url, params, body } the handlers expect.
// `req` is a tiny shim: the handler reads method/url/headers and, for a streaming route,
// listens for 'close'. `body()` reads and parses the JSON body.
export function makeReqBody(c) {
  const req = {
    method: c.req.method,
    url: c.req.url,
    headers: Object.fromEntries((c.req.raw.headers || new Headers()).entries()),
    on(name, fn) {
      if (name === 'close' || name === 'aborted') {
        // Hono/node fires the request's own abort when the client disconnects.
        c.req.raw.signal?.addEventListener('abort', fn);
      }
      return req;
    },
    destroy() { /* client gone; the response ends via abort */ },
  };
  const body = async () => {
    const raw = await c.req.text();
    try { return raw ? JSON.parse(raw) : {}; }
    catch { throw Object.assign(new Error('bad json'), { code: 'validation_failed' }); }
  };
  return { req, body };
}

// Build a Hono app from the hub's ROUTES table. The table stays the single description of
// the surface; Hono is the mechanism that matches and dispatches (ADR-0010), so the
// contract and the routes do not drift.
export function buildApp({ routes, authCheck }) {
  const app = new Hono();
  app.all('*', async (c) => {
    const { req, body } = makeReqBody(c);
    const url = new URL(c.req.url);
    let matched = null;
    let params = null;
    for (const row of routes) {
      if (row.method !== c.req.method) continue;
      const p = matchPath(row.path, url.pathname);
      if (p) { matched = row; params = p; break; }
    }
    if (!matched) {
      return c.json({ error: { code: 'not_found', message: `no route ${c.req.method} ${url.pathname}`, retryable: false } }, 404);
    }
    if (matched.auth !== false) {
      const denial = authCheck(req);
      if (denial) return c.json(denial.body, denial.status);
    }
    const res = makeRes();
    try { await matched.handler({ req, res, url, params, body }); }
    catch (e) { /* the handler's own failure; hand it to the routing layer's error path */ throw e; }
    return await res._response;
  });
  return app;
}

// The same path matching the hub always used. Kept here so the transport owns routing and
// the ROUTES table stays pure data.
export function matchPath(pattern, pathname) {
  const want = pattern.split('/');
  const got = pathname.split('/');
  const params = {};
  for (let i = 0; i < want.length; i++) {
    const w = want[i];
    if (w.endsWith('...}')) { params[w.slice(1, -4)] = got.slice(i).map(decodeURIComponent); return params; }
    if (i >= got.length) return null;
    if (w.startsWith('{') && w.endsWith('}')) { params[w.slice(1, -1)] = decodeURIComponent(got[i]); continue; }
    if (w !== got[i]) return null;
  }
  return got.length === want.length ? params : null;
}

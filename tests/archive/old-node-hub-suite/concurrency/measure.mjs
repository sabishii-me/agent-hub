// The ADR-0009 concurrency acceptance, MEASURED against a running hub (not asserted).
//
// Usage: node tests/concurrency/measure.mjs <base-url> <token> [N]
// The hub writes <DATA_DIR>/endpoint.json with url + token.
//
// It fires 2*N plain concurrent requests (no keep-alive tricks) and reports how many
// answered and how long they took. The acceptance is: >= 100 concurrent connections,
// no in-flight work timing a connection out.

const base = process.argv[2] || 'http://127.0.0.1:8925';
const token = process.argv[3];
const n = Number(process.argv[4] || 200);
if (!token) {
  console.error('usage: node measure.mjs <base-url> <token> [N]');
  process.exit(2);
}

const t0 = Date.now();
const auth = { authorization: 'Bearer ' + token };
const reqs = [];
for (let i = 0; i < n; i++) {
  reqs.push(fetch(base + '/v1/status', { headers: auth }).then((r) => r.status).catch((e) => 'ERR:' + e.message));
  reqs.push(fetch(base + '/v1/surface', { headers: auth }).then((r) => r.status).catch((e) => 'ERR:' + e.message));
}
const res = await Promise.all(reqs);
const ok = res.filter((s) => s === 200).length;
const errs = res.filter((s) => s !== 200);
console.log(JSON.stringify({
  requests: res.length,
  ok,
  errors: errs.length,
  elapsedMs: Date.now() - t0,
  distinctErrors: [...new Set(errs)].slice(0, 5),
}, null, 2));
// The acceptance: >= 100 concurrent connections, no errors.
process.exit(ok >= 100 && errs.length === 0 ? 0 : 1);

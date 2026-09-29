#!/usr/bin/env node
// A minimal agent-bus adapter, used by the hub's adapter tests. It speaks
// JSON-RPC 2.0 over stdin/stdout, one LF-terminated JSON line per message, and
// answers the core methods the hub routes.
import readline from 'node:readline';

const rl = readline.createInterface({ input: process.stdin });
const send = (msg) => process.stdout.write(JSON.stringify(msg) + '\n');

let nextEvent = 1;
const emit = (method, params) =>
  send({ jsonrpc: '2.0', method, params: { ...params, id: 'e' + nextEvent++ } });

rl.on('line', (line) => {
  line = line.trim();
  if (!line) return;
  let msg;
  try { msg = JSON.parse(line); } catch { return; }
  const { id, method, params } = msg;
  switch (method) {
    case 'session/start':
      send({ jsonrpc: '2.0', id, result: { ref: 'native-' + params.sid } });
      break;
    case 'config/set':
      send({ jsonrpc: '2.0', id, result: {} });
      break;
    case 'presets/list':
      send({ jsonrpc: '2.0', id, result: { harnessId: 'fake', known: true, presets: [{ id: 'default' }] } });
      break;
    case 'session/prompt': {
      // Emit the events a real turn would, then reply.
      emit('turn.running', { sid: params.sid });
      emit('message.delta', { sid: params.sid, messageId: 'm1', text: 'hi', kind: 'text' });
      emit('message.completed', { sid: params.sid, messageId: 'm1', role: 'assistant', text: 'hi' });
      send({ jsonrpc: '2.0', id, result: { state: 'ended', ended: 'completed' } });
      break;
    }
    case 'session/abort':
      send({ jsonrpc: '2.0', id, result: null });
      break;
    case 'models/list':
      send({ jsonrpc: '2.0', id, result: { harnessId: 'fake', known: true, models: [{ id: 'fake-model' }] } });
      break;
    case 'tools/list':
      send({ jsonrpc: '2.0', id, result: { harnessId: 'fake', known: true, tools: [{ name: 'read' }], partial: false } });
      break;
    case 'boom':
      send({ jsonrpc: '2.0', id, error: { code: -32000, message: 'deliberate' } });
      break;
    default:
      send({ jsonrpc: '2.0', id, error: { code: -32601, message: 'method not found: ' + method } });
  }
});

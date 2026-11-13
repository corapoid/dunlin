import { test } from 'node:test';
import assert from 'node:assert/strict';
import { decodeHostHealth, decodeHostHealthEvent, HostHealthStream, hostHealthProtocol } from '../src/lib/host-health.ts';
import type { HostHealthState } from '../src/lib/host-health.ts';
const health = {
  host: { id: 'host', workspace_id: 'workspace',
    definition: { name: 'Worker', node_id: 'worker-node', serve_origin: 'https://worker.tail123.ts.net', capability_policy: { max_parallel: 1, harnesses: ['opencode'] } },
    status: 'approved', revision: 3, enrollment_epoch: 1, credential_revision: 1, capability_revision: 1,
    drain: false, capabilities: null, last_seen: 123, created_at: 121, updated_at: 123,
    network_verification: 'unverified', execution_ready: false },
  connection: { protocol_version: 4, heartbeat_sequence: 1, heartbeat_age_ms: 2000, lease_remaining_ms: 8000 },
  runtime: { one_shot_reservations: 0, session_reservations: 1, active_sessions: 1, active_turns: 0, uncertain_reservations: 0, unconfirmed_acknowledgements: 0 },
  controller_boot_id: 'controller', observed_at_ms: 1800000000000, observational: true,
};
function frame(sequence = 1, value = health, stream = 'stream') {
  return { type: 'snapshot', protocol_version: 1, stream_id: stream, sequence, health: value };
}
class Socket {
  onmessage: ((event: { data: string }) => void) | null = null;
  onclose: (() => void) | null = null;
  onerror: (() => void) | null = null;
  closed = false;
  close() { this.closed = true; this.onclose?.(); }
  message(value: unknown) { this.onmessage?.({ data: JSON.stringify(value) }); }
}
function fixture() {
  const sockets: Socket[] = [], states: HostHealthState[] = [];
  const client = new HostHealthStream('https://controller.tail123.ts.net', 'workspace', 'host', (state) => states.push(state), (url, protocol) => {
    assert.equal(url, 'wss://controller.tail123.ts.net/v1/web/workspaces/workspace/hosts/host/health/live');
    assert.equal(protocol, hostHealthProtocol);
    const socket = new Socket(); sockets.push(socket); return socket as unknown as WebSocket;
  });
  return { client, sockets, states };
}
test('health codec separates live observation from metadata and denies cross-scope/secret/contradictory data', () => {
  const decoded = decodeHostHealth(health, 'workspace', 'host');
  assert.equal(decoded.connection?.protocol_version, 4);
  assert.equal(decoded.host.execution_ready, false);
  const broker = decodeHostHealth({ ...health, connection: { ...health.connection, protocol_version: 5 } }, 'workspace', 'host');
  assert.equal(broker.connection?.protocol_version, 5);
  assert.equal(broker.host.execution_ready, false);
  for (const value of [
    { ...health, host: { ...health.host, id: 'other' } },
    { ...health, host: { ...health.host, workspace_id: 'other' } },
    { ...health, host: { ...health.host, credential: 'secret' } },
    { ...health, host: { ...health.host, execution_ready: true } },
    { ...health, credential: 'secret' }, { ...health, observational: false },
    { ...health, observed_at_ms: Infinity },
    { ...health, runtime: { ...health.runtime, active_turns: 2 } },
    { ...health, runtime: { ...health.runtime, uncertain_reservations: 2 } },
    { ...health, connection: { ...health.connection, lease_remaining_ms: 9000 } },
    { ...health, connection: { ...health.connection, protocol_version: 6 } },
  ]) assert.throws(() => decodeHostHealth(value, 'workspace', 'host'));
  assert.throws(() => decodeHostHealthEvent(JSON.stringify({ ...frame(), sequence: 0 }), 'workspace', 'host'));
  assert.throws(() => decodeHostHealthEvent(JSON.stringify({ ...frame(), credential: 'secret' }), 'workspace', 'host'));
});
test('only a scoped ordered snapshot makes the monitor live; gaps clear data and require explicit reconnect', () => {
  const f = fixture(); f.client.start();
  assert.equal(f.client.state.status, 'connecting');
  f.sockets[0].message(frame()); assert.equal(f.client.state.status, 'live');
  f.sockets[0].message(frame(3));
  assert.equal(f.client.state.status, 'error'); assert.equal(f.client.state.health, null); assert.equal(f.sockets[0].closed, true);
  assert.equal(f.sockets.length, 1);
  f.client.stop();
});
test('late frames from a stopped socket cannot restore data after scope lifecycle or restart', () => {
  const f = fixture(); f.client.start(); const old = f.sockets[0]; old.message(frame());
  f.client.start(); const count = f.states.length;
  old.message(frame(2)); assert.equal(f.states.length, count); assert.equal(f.client.state.health, null);
  f.sockets[1].message(frame()); assert.equal(f.client.state.status, 'live');
  f.sockets[1].message({ type: 'unavailable', protocol_version: 1, stream_id: 'stream' });
  assert.equal(f.client.state.status, 'forbidden'); assert.equal(f.client.state.health, null);
  f.sockets[1].message(frame(2)); assert.equal(f.client.state.status, 'forbidden');
  f.client.stop();
});
test('server keepalives never extend the worker lease observation', (context) => {
  context.mock.timers.enable({ apis: ['setTimeout'] });
  const f = fixture(); f.client.start(); f.sockets[0].message(frame());
  context.mock.timers.tick(5000);
  f.sockets[0].message({ type: 'heartbeat', protocol_version: 1, stream_id: 'stream' });
  context.mock.timers.tick(3000);
  assert.equal(f.client.state.status, 'stale'); assert.equal(f.client.state.health, null); assert.equal(f.sockets[0].closed, true);
  f.client.stop();
});
test('an offline host stays observable with keepalives but a silent server becomes stale', (context) => {
  context.mock.timers.enable({ apis: ['setTimeout'] });
  const f = fixture(); f.client.start(); f.sockets[0].message({ ...frame(), health: { ...health, connection: null } });
  context.mock.timers.tick(10000); f.sockets[0].message({ type: 'heartbeat', protocol_version: 1, stream_id: 'stream' });
  context.mock.timers.tick(10000); assert.equal(f.client.state.status, 'live');
  context.mock.timers.tick(2000); assert.equal(f.client.state.status, 'stale'); assert.equal(f.client.state.health, null);
  f.client.stop();
});
test('unexpected stream/controller identities and malformed frames cannot replace a live observation', () => {
  for (const value of [frame(2, health, 'other-stream'), frame(2, { ...health, controller_boot_id: 'other-controller' }),
    { type: 'heartbeat', protocol_version: 1, stream_id: 'other-stream' }]) {
    const f = fixture(); f.client.start(); f.sockets[0].message(frame()); f.sockets[0].message(value);
    assert.equal(f.client.state.status, 'error'); assert.equal(f.client.state.health, null); f.client.stop();
  }
});

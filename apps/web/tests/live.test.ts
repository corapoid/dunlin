import { test } from 'node:test';
import assert from 'node:assert/strict';
import { CatalogClient } from '../src/lib/live.ts';
import type { LiveStatus } from '../src/lib/live.ts';
import type { CatalogSnapshot } from '../src/lib/catalog.ts';

class FixtureSocket {
  readyState = 1;
  sent: string[] = [];
  closed = false;
  onopen: (() => void) | null = null;
  onclose: (() => void) | null = null;
  onerror: (() => void) | null = null;
  onmessage: ((event: { data: string }) => void) | null = null;
  send(data: string) { this.sent.push(data); }
  close() { this.closed = true; this.onclose?.(); }
  message(value: unknown) { this.onmessage?.({ data: JSON.stringify(value) }); }
}

function snapshot(project: string | null, sequence = 1): CatalogSnapshot {
  return {
    protocol_version: 1, stream_id: 'fixture', sequence, server_name: 'fixture server',
    user: { id: 'u', name: 'member', role: 'member' },
    projects: [{ id: 'p', name: 'first' }, { id: 'q', name: 'second' }],
    project_id: project, sessions: [],
  };
}

function fixture() {
  const sockets: FixtureSocket[] = [];
  const statuses: LiveStatus[] = [];
  const snapshots: CatalogSnapshot[] = [];
  const client = new CatalogClient('https://build.tail123.ts.net', (state) => snapshots.push(state), (status) => statuses.push(status), (url) => {
    assert.equal(url, 'wss://build.tail123.ts.net/v1/web/live');
    const socket = new FixtureSocket();
    sockets.push(socket);
    return socket as unknown as WebSocket;
  });
  return { client, sockets, statuses, snapshots };
}

test('socket opening alone is not live; the first scoped snapshot is authoritative', () => {
  const f = fixture();
  f.client.start();
  const socket = f.sockets[0];
  socket.onopen?.();
  assert.deepEqual(JSON.parse(socket.sent[0]), { type: 'subscribe', protocol_version: 1, project_id: null });
  assert.equal(f.statuses.includes('live'), false);
  socket.message({ type: 'snapshot', snapshot: snapshot(null) });
  assert.equal(f.statuses.at(-1), 'live');
  assert.equal(f.snapshots.length, 1);
  f.client.stop();
});

test('a late snapshot for the previous project cannot replace current project state', () => {
  const f = fixture();
  f.client.start();
  f.client.selectProject('p');
  f.client.selectProject('q');
  const socket = f.sockets[0];
  socket.message({ type: 'snapshot', snapshot: snapshot('p', 1) });
  assert.equal(f.snapshots.length, 0);
  socket.message({ type: 'snapshot', snapshot: snapshot('q', 2) });
  socket.message({ type: 'snapshot', snapshot: snapshot('q', 1) });
  assert.equal(f.snapshots.length, 1);
  assert.equal(f.snapshots[0].project_id, 'q');
  f.client.stop();
});

test('revocation closes the stream and does not reconnect with revoked authority', () => {
  const f = fixture();
  f.client.start();
  f.sockets[0].message({ type: 'error', code: 'authorization_required' });
  assert.equal(f.sockets[0].closed, true);
  assert.equal(f.statuses.at(-1), 'authorization_required');
  f.sockets[0].message({ type: 'snapshot', snapshot: snapshot(null) });
  assert.equal(f.snapshots.length, 0);
  assert.equal(f.sockets.length, 1);
});

test('restart ignores callbacks from an old socket', () => {
  const f = fixture();
  f.client.start();
  const old = f.sockets[0];
  f.client.start();
  old.message({ type: 'snapshot', snapshot: snapshot(null) });
  assert.equal(f.snapshots.length, 0);
  f.sockets[1].message({ type: 'snapshot', snapshot: snapshot(null) });
  assert.equal(f.snapshots.length, 1);
  f.client.stop();
});

test('reconnect resubscribes but never replays a mutation', (context) => {
  context.mock.timers.enable({ apis: ['setTimeout'] });
  const f = fixture();
  f.client.start();
  f.client.selectProject('p');
  f.sockets[0].onclose?.();
  assert.equal(f.statuses.at(-1), 'reconnecting');
  context.mock.timers.tick(20_000);
  assert.equal(f.sockets.length, 2);
  f.sockets[1].onopen?.();
  assert.deepEqual(f.sockets[1].sent.map((message) => JSON.parse(message)), [{ type: 'subscribe', protocol_version: 1, project_id: 'p' }]);
  f.client.stop();
});

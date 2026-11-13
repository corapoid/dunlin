import { test } from 'node:test';
import assert from 'node:assert/strict';
import { decodeHostChannel, channelWarnings, HostChannelReader } from '../src/lib/host-channel.ts';
import type { HostChannelState } from '../src/lib/host-channel.ts';
const diagnostic = {
  channel: {
    phase: 'active',
    lease: {
      host_id: 'host',
      workspace_id: 'workspace',
      channel_id: 'channel',
      controller_boot_id: 'old-controller',
      worker_boot_id: 'worker',
      epoch: 4,
      expires_at_ms: 1000,
      last_sequence: 9,
    },
  },
  current_controller_boot_id: 'current-controller',
  reported_state_only: true,
};
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}
test('channel diagnostic preserves raw expired active phase and reports boot mismatch without claiming health', () => {
  const value = decodeHostChannel(diagnostic, 'workspace', 'host');
  assert.equal(value.channel?.phase, 'active');
  assert.deepEqual(channelWarnings(value, 2000), { differentBoot: true, expiredAtRead: true });
  assert.equal('healthy' in value, false);
  assert.equal('execution_ready' in value, false);
  const current = decodeHostChannel(
    { ...diagnostic, current_controller_boot_id: 'old-controller' },
    'workspace',
    'host',
  );
  assert.deepEqual(channelWarnings(current, 999), { differentBoot: false, expiredAtRead: false });
  const empty = decodeHostChannel(
    { channel: null, current_controller_boot_id: 'controller', reported_state_only: true },
    'workspace',
    'host',
  );
  assert.equal(empty.channel, null);
  assert.deepEqual(channelWarnings(empty, 2000), { differentBoot: false, expiredAtRead: false });
});
test('channel codec refuses cross-scope, unsafe clocks, fabricated authority and unexpected secret fields', () => {
  for (const lease of [
    { ...diagnostic.channel.lease, host_id: 'other' },
    { ...diagnostic.channel.lease, workspace_id: 'other' },
    { ...diagnostic.channel.lease, epoch: 0 },
    { ...diagnostic.channel.lease, last_sequence: -1 },
    { ...diagnostic.channel.lease, expires_at_ms: 8640000000000001 },
    { ...diagnostic.channel.lease, nonce: 'secret' },
  ])
    assert.throws(() =>
      decodeHostChannel({ ...diagnostic, channel: { ...diagnostic.channel, lease } }, 'workspace', 'host'),
    );
  for (const extra of [{ reported_state_only: false }, { healthy: true }, { credential: 'secret' }])
    assert.throws(() => decodeHostChannel({ ...diagnostic, ...extra }, 'workspace', 'host'));
  assert.throws(() =>
    decodeHostChannel(
      { ...diagnostic, channel: { ...diagnostic.channel, phase: ['active'] } },
      'workspace',
      'host',
    ),
  );
  assert.throws(() => decodeHostChannel(diagnostic, 'workspace', '../unsafe'));
});
test('reader makes explicit same-origin GET only and keeps no record after failed refresh', async (context) => {
  let calls = 0;
  context.mock.method(globalThis, 'fetch', async (path: string, init?: RequestInit) => {
    calls++;
    assert.equal(path, '/v1/web/workspaces/workspace/hosts/host/channel');
    assert.equal(init?.method, 'GET');
    assert.equal(init?.credentials, 'same-origin');
    assert.equal(init?.cache, 'no-store');
    assert.equal(init?.redirect, 'error');
    return calls === 1 ? Response.json(diagnostic) : new Response(null, { status: 503 });
  });
  const reader = new HostChannelReader(
    'workspace',
    'host',
    () => {},
    () => {},
  );
  assert.equal(calls, 0);
  assert.equal(reader.state.status, 'idle');
  await reader.read();
  assert.equal(calls, 1);
  assert.equal(reader.state.status, 'loaded');
  assert.equal(typeof reader.state.readAt, 'number');
  await reader.read();
  assert.equal(calls, 2);
  assert.equal(reader.state.status, 'error');
  assert.equal(reader.state.diagnostic, null);
  assert.equal(reader.state.readAt, null);
  reader.stop();
});
test('later explicit read wins when an aborted old GET still completes', async (context) => {
  const first = deferred<Response>(),
    second = deferred<Response>();
  let calls = 0;
  context.mock.method(globalThis, 'fetch', () => (++calls === 1 ? first.promise : second.promise));
  const reader = new HostChannelReader(
    'workspace',
    'host',
    () => {},
    () => {},
  );
  const old = reader.read(),
    fresh = reader.read();
  second.resolve(
    Response.json({
      ...diagnostic,
      channel: { ...diagnostic.channel, lease: { ...diagnostic.channel.lease, last_sequence: 10 } },
    }),
  );
  await fresh;
  first.resolve(Response.json(diagnostic));
  await old;
  assert.equal(reader.state.diagnostic?.channel?.lease.last_sequence, 10);
  assert.equal(calls, 2);
  reader.stop();
});
test('permission loss removes diagnostic and notifies registry, while late denied reads cannot revoke a newer scope', async (context) => {
  let forbidden = 0;
  context.mock.method(globalThis, 'fetch', async () => Response.json(diagnostic));
  const reader = new HostChannelReader(
    'workspace',
    'host',
    () => {},
    () => forbidden++,
  );
  await reader.read();
  assert.ok(reader.state.diagnostic);
  context.mock.method(globalThis, 'fetch', async () => new Response(null, { status: 403 }));
  await reader.read();
  assert.equal(reader.state.status, 'forbidden');
  assert.equal(reader.state.diagnostic, null);
  assert.equal(forbidden, 1);
  const late = deferred<Response>();
  context.mock.method(globalThis, 'fetch', () => late.promise);
  const request = reader.read();
  reader.stop();
  late.resolve(new Response(null, { status: 403 }));
  await request;
  assert.equal(forbidden, 1);
  assert.equal(reader.state.diagnostic, null);
});
test('catalog invalidation clears metadata and fences late replies without automatically fetching', async (context) => {
  const pending = deferred<Response>();
  let calls = 0;
  const states: HostChannelState[] = [];
  context.mock.method(globalThis, 'fetch', () => {
    calls++;
    return pending.promise;
  });
  const reader = new HostChannelReader(
    'workspace',
    'host',
    (s) => states.push(s),
    () => {},
  );
  const request = reader.read();
  reader.invalidate();
  pending.resolve(Response.json(diagnostic));
  await request;
  assert.equal(reader.state.status, 'idle');
  assert.equal(reader.state.diagnostic, null);
  assert.equal(calls, 1);
  assert.deepEqual(
    states.map((s) => s.status),
    ['loading', 'idle'],
  );
  reader.stop();
});
test('malformed secret-bearing response never enters rendered state or propagates raw server text', async (context) => {
  const secret = 'DO_NOT_RENDER_PRIVATE_TOKEN';
  context.mock.method(globalThis, 'fetch', async () => Response.json({ ...diagnostic, credential: secret }));
  const reader = new HostChannelReader(
    'workspace',
    'host',
    () => {},
    () => {},
  );
  await reader.read();
  assert.equal(reader.state.status, 'error');
  assert.equal(JSON.stringify(reader.state).includes(secret), false);
  reader.stop();
});

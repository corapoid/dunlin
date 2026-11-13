import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  decodeHostDefinition,
  decodeHost,
  decodeHosts,
  decodeEnrollment,
  createHost,
  enrollHost,
  changeHost,
} from '../src/lib/hosts.ts';
import { TeamRequestError } from '../src/lib/teams.ts';
const definition = {
  name: 'Remote worker',
  node_id: 'nStableNode',
  serve_origin: 'https://worker.tail123.ts.net',
  capability_policy: { max_parallel: 2, harnesses: ['opencode'] },
};
const host = {
  id: 'host',
  workspace_id: 'workspace',
  definition,
  status: 'claimed',
  revision: 3,
  enrollment_epoch: 1,
  credential_revision: 1,
  drain: false,
  capabilities: {
    protocol_version: 1,
    version: '0.2',
    os: 'linux',
    architecture: 'aarch64',
    harnesses: ['opencode', 'codex'],
    sandbox: 'linux_bubblewrap',
    max_parallel: 32,
  },
  last_seen: 123,
  created_at: 121,
  updated_at: 123,
  network_verification: 'unverified',
  execution_ready: false,
};
test('host registry keeps untrusted above-policy reports distinct from execution authority', () => {
  const decoded = decodeHost(host, 'workspace');
  assert.equal(decoded.execution_ready, false);
  assert.equal(decoded.network_verification, 'unverified');
  assert.equal(decoded.capabilities?.max_parallel, 32);
  assert.equal(decoded.definition.capability_policy.max_parallel, 2);
  assert.deepEqual(
    decodeHostDefinition({ ...definition, capability_policy: { max_parallel: 1, harnesses: [] } })
      .capability_policy.harnesses,
    [],
  );
  assert.throws(() => decodeHost({ ...host, execution_ready: true }, 'workspace'));
  assert.throws(() => decodeHost({ ...host, network_verification: 'verified' }, 'workspace'));
  assert.throws(() => decodeHost(host, 'other-workspace'));
  assert.throws(() => decodeHosts({ hosts: [host, host] }, 'workspace'));
  assert.throws(() => decodeHost({ ...host, status: ['claimed'] }, 'workspace'));
});
test('host codec rejects arbitrary executable policies, unstable node IDs and unsafe Serve origins', () => {
  for (const serve_origin of [
    'http://worker.tail123.ts.net',
    'https://worker.tail123.ts.net/',
    'https://worker.tail123.ts.net:443',
    'https://worker.tail123.ts.net:8443',
    'https://worker.tail123.ts.net/path',
    'https://worker.tail123.ts.net?token=secret',
    'https://worker.tail123.ts.net#secret',
    'https://name@worker.tail123.ts.net',
    'https://127.0.0.1',
    'https://worker.example.org',
    'https://WORKER.tail123.ts.net',
  ])
    assert.throws(() => decodeHostDefinition({ ...definition, serve_origin }));
  for (const patch of [
    { node_id: 'node with spaces' },
    { name: '界'.repeat(43) },
    { capability_policy: { max_parallel: 0, harnesses: ['opencode'] } },
    { capability_policy: { max_parallel: 33, harnesses: ['opencode'] } },
    { capability_policy: { max_parallel: 1, harnesses: ['/bin/custom'] } },
    { capability_policy: { max_parallel: 1, harnesses: ['codex', 'codex'] } },
  ])
    assert.throws(() => decodeHostDefinition({ ...definition, ...patch }));
  assert.throws(() =>
    decodeHost({ ...host, capabilities: { ...host.capabilities, sandbox: 'custom-command' } }, 'workspace'),
  );
});
test('ticket exists only in explicit enrollment response and malformed scope/expiry is rejected', () => {
  const response = { host, enrollment: { ticket: 'one-time-ticket', expires_at: 456 } };
  assert.equal(decodeEnrollment(response, 'workspace').enrollment.ticket, 'one-time-ticket');
  assert.equal('enrollment' in decodeHost(host, 'workspace'), false);
  assert.throws(() =>
    decodeEnrollment({ ...response, enrollment: { ticket: 'secret\nline', expires_at: 456 } }, 'workspace'),
  );
  assert.throws(() =>
    decodeEnrollment({ ...response, enrollment: { ticket: 'secret', expires_at: 0 } }, 'workspace'),
  );
});
test('host mutations send one explicit CAS request and never retry conflict or network uncertainty', async (context) => {
  const calls: { path: string; body: unknown }[] = [];
  context.mock.method(globalThis, 'fetch', async (path: string, init?: RequestInit) => {
    calls.push({ path, body: JSON.parse(String(init?.body)) });
    assert.equal(init?.method, 'POST');
    assert.equal(init?.credentials, 'same-origin');
    return new Response(null, { status: 409 });
  });
  await assert.rejects(
    changeHost('workspace', decodeHost(host, 'workspace'), 'drain', true),
    (e: unknown) => e instanceof TeamRequestError && e.status === 409,
  );
  assert.equal(calls.length, 1);
  assert.deepEqual(calls[0], {
    path: '/v1/web/workspaces/workspace/hosts/host/drain',
    body: { expected_revision: 3, drain: true },
  });
  await assert.rejects(enrollHost('workspace', decodeHost(host, 'workspace')));
  assert.equal(calls.length, 2);
  assert.deepEqual(calls[1].body, { expected_revision: 3 });
  context.mock.method(globalThis, 'fetch', async () => {
    throw new TypeError('lost response');
  });
  await assert.rejects(createHost('workspace', definition));
  assert.equal(calls.length, 2);
});
test('host response identities cannot redirect a mutation to another registered node', async (context) => {
  context.mock.method(globalThis, 'fetch', async () =>
    Response.json({ ...host, id: 'different-host', revision: 4 }),
  );
  await assert.rejects(changeHost('workspace', decodeHost(host, 'workspace'), 'approve'));
  await assert.rejects(enrollHost('other-workspace', decodeHost(host, 'workspace')));
});
test('registry inputs do not accept browser-supplied verification, credentials or executable paths', () => {
  for (const extra of [
    { execution_ready: true },
    { network_verification: 'verified' },
    { credential: 'secret' },
    { command: '/bin/arbitrary' },
  ])
    assert.throws(() => decodeHostDefinition({ ...definition, ...extra }));
  assert.throws(() =>
    decodeHostDefinition({
      ...definition,
      capability_policy: { ...definition.capability_policy, verified: true },
    }),
  );
  assert.throws(() =>
    decodeHostDefinition({ ...definition, serve_origin: `https://${'x'.repeat(64)}.ts.net` }),
  );
  assert.throws(() =>
    decodeHost({ ...host, capabilities: { ...host.capabilities, verified: true } }, 'workspace'),
  );
});
test('registry read fence rejects pre-mutation snapshots and preserves authority loss during pending mutation', async () => {
  const { HostRegistryReadFence } = await import('../src/lib/hosts.ts');
  const fence = new HostRegistryReadFence();
  const oldRead = fence.beginRead()!;
  const mutation = fence.beginMutation();
  assert.equal(fence.current(oldRead), false);
  assert.equal(fence.current(mutation), true);
  assert.equal(fence.beginRead(), null, 'Canonical metadata reads defer while a mutation owns the cut');
  const authorityCheck = fence.authorityGeneration();
  assert.equal(fence.current(authorityCheck), true);
  fence.invalidate();
  assert.equal(fence.current(mutation), false, 'A forbidden authority check fences late mutation secrets');
  fence.endMutation();
  const first = fence.beginRead()!,
    second = fence.beginRead()!;
  assert.equal(fence.current(first), false);
  assert.equal(fence.current(second), true);
});
test('fresh authorized registry preserves only an already-held unspent matching enrollment epoch', async () => {
  const { retainHostEnrollment } = await import('../src/lib/hosts.ts');
  const pending = decodeHost({ ...host, status: 'pending' }, 'workspace');
  const previous = { host: pending, enrollment: { ticket: 'held-secret', expires_at: 500 } };
  assert.equal(
    retainHostEnrollment(previous, [pending], 1000),
    previous,
    'Own creation invalidation may preserve its already held secret',
  );
  assert.equal(retainHostEnrollment(null, [pending], 1000), null, 'GET cannot recover a ticket');
  for (const changed of [
    { ...pending, status: 'claimed' },
    { ...pending, enrollment_epoch: 2 },
    { ...pending, workspace_id: 'other' },
    { ...pending, id: 'other' },
  ])
    assert.equal(retainHostEnrollment(previous, [decodeHost(changed, changed.workspace_id)], 1000), null);
  assert.equal(retainHostEnrollment(previous, [], 1000), null);
  assert.equal(retainHostEnrollment(previous, [pending], 501000), null);
});

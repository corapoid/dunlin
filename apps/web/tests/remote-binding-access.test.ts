import { test } from 'node:test';
import assert from 'node:assert/strict';
import { decodeRemoteBinding } from '../src/lib/remote-bindings.ts';
import { decodeRemoteBindingAccess, RemoteBindingAccessReader } from '../src/lib/remote-binding-access.ts';
const binding = decodeRemoteBinding({
  id: 'binding', workspace_id: 'workspace', project_id: 'project', host_id: 'host',
  host_enrollment_epoch: 2, credential_revision: 3, export_id: 'export', export_revision: 4,
  repo_kind: 'git', checkout_identity_digest: 'a'.repeat(64), approved_settings_digest: 'b'.repeat(64),
  principal: { kind: 'human', id: 'administrator' }, grant_revision: 5,
  grant_permissions: ['read', 'execute'], revision: 6, active: true, observed_at_ms: 100,
}, 'workspace', 'project');
function observed(expected = binding) {
  const { id, revision, active, ...metadata } = expected;
  void active;
  return { ...metadata, binding: { id, revision }, principal: { kind: 'human', id: 'member' },
    grant_revision: 9, grant_permissions: ['read'], observed_at_ms: 200,
    required_worker_permission: 'read', observed_state_only: true, execution_ready: false };
}
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>(done => { resolve = done; });
  return { promise, resolve };
}
test('resolver codec observes caller-specific Read independently of saved administrator principal and grant', () => {
  const result = decodeRemoteBindingAccess(observed(), binding, 'member');
  assert.equal(result.principal.id, 'member'); assert.equal(result.grant_revision, 9);
  assert.deepEqual(result.grant_permissions, ['read']); assert.equal(result.execution_ready, false);
  assert.equal(binding.principal.id, 'administrator'); assert.equal(binding.grant_revision, 5);
  assert.equal('active' in result, false);
});
test('resolver codec refuses stale ref, target drift, wrong caller, fabricated execution and secret fields', () => {
  for (const patch of [
    { binding: { id: 'other', revision: 6 } }, { binding: { id: 'binding', revision: 7 } },
    { workspace_id: 'other' }, { project_id: 'other' }, { host_enrollment_epoch: 3 },
    { credential_revision: 4 }, { export_id: 'other' }, { export_revision: 5 },
    { checkout_identity_digest: 'c'.repeat(64) }, { approved_settings_digest: 'c'.repeat(64) },
    { principal: binding.principal }, { principal: { kind: 'service', id: 'member' } },
    { grant_permissions: ['execute'] }, { required_worker_permission: 'execute' },
    { execution_ready: true }, { observed_state_only: false }, { root: '/private/checkout' },
  ]) assert.throws(() => decodeRemoteBindingAccess({ ...observed(), ...patch }, binding, 'member'));
});
test('only an explicit check sends the exact immutable BindingRef and never session/grant metadata', async context => {
  const calls: { path: string; body: unknown }[] = [];
  context.mock.method(globalThis, 'fetch', async (path: string, init?: RequestInit) => {
    calls.push({ path, body: JSON.parse(String(init?.body)) });
    assert.equal(init?.method, 'POST'); assert.equal(init?.credentials, 'same-origin');
    assert.equal(init?.cache, 'no-store'); assert.equal(init?.redirect, 'error');
    return Response.json(observed());
  });
  const reader = new RemoteBindingAccessReader('workspace', 'project', 'member', () => {});
  assert.equal(calls.length, 0); await reader.check(binding);
  assert.deepEqual(calls, [{ path: '/v1/web/workspaces/workspace/projects/project/remote-bindings/resolve', body: { binding: { id: 'binding', revision: 6 } } }]);
  assert.equal(reader.state.status, 'observed'); assert.equal(reader.state.result?.principal.id, 'member');
  reader.stop();
});
test('connected worker Read revocation removes earlier observation, does not reuse stored admin grant or retry', async context => {
  let calls = 0;
  context.mock.method(globalThis, 'fetch', async () => ++calls === 1 ? Response.json(observed()) : new Response(null, { status: 403 }));
  const reader = new RemoteBindingAccessReader('workspace', 'project', 'member', () => {});
  await reader.check(binding); assert.ok(reader.state.result);
  await reader.check(binding); assert.equal(reader.state.status, 'denied');
  assert.equal(reader.state.result, null); assert.equal(calls, 2);
  assert.equal(binding.active, true); assert.equal(binding.principal.id, 'administrator');
  reader.stop();
});
test('selection change and newer check fence delayed replies even when transport ignores cancellation', async context => {
  const oldReply = deferred<Response>(), freshReply = deferred<Response>(); let calls = 0;
  context.mock.method(globalThis, 'fetch', () => ++calls === 1 ? oldReply.promise : freshReply.promise);
  const reader = new RemoteBindingAccessReader('workspace', 'project', 'member', () => {});
  const other = { ...binding, id: 'other-binding', export_id: 'other-export' };
  const old = reader.check(binding), fresh = reader.check(other);
  freshReply.resolve(Response.json(observed(other))); await fresh;
  oldReply.resolve(Response.json(observed())); await old;
  assert.equal(reader.state.result?.binding.id, 'other-binding'); reader.stop();
});
test('authority invalidation or unmount clears state and ignores late successful RPC without automatic replacement', async context => {
  for (const stop of [false, true]) {
    const pending = deferred<Response>(); let calls = 0;
    context.mock.method(globalThis, 'fetch', () => { calls++; return pending.promise; });
    const reader = new RemoteBindingAccessReader('workspace', 'project', 'member', () => {});
    const request = reader.check(binding); if (stop) reader.stop(); else reader.invalidate();
    pending.resolve(Response.json(observed())); await request;
    assert.equal(reader.state.status, 'idle'); assert.equal(reader.state.result, null); assert.equal(calls, 1);
    reader.stop();
  }
});
test('stale references are denied once and malformed/inactive scope cannot start a resolver request', async context => {
  let calls = 0; context.mock.method(globalThis, 'fetch', async () => { calls++; return new Response(null, { status: 409 }); });
  const reader = new RemoteBindingAccessReader('workspace', 'project', 'member', () => {});
  await reader.check(binding); assert.equal(reader.state.status, 'denied'); assert.equal(calls, 1);
  await assert.rejects(reader.check({ ...binding, active: false }));
  await assert.rejects(reader.check({ ...binding, project_id: 'other' }));
  assert.equal(calls, 1); reader.stop();
});

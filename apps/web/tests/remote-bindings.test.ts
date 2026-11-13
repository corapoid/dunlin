import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  decodeRemoteBinding,
  decodeRemoteBindings,
  decodeRemoteBindingInput,
  saveRemoteBinding,
  revokeRemoteBinding,
  RemoteBindingsController,
} from '../src/lib/remote-bindings.ts';
const binding = {
  id: 'binding',
  workspace_id: 'workspace',
  project_id: 'project',
  host_id: 'host',
  host_enrollment_epoch: 2,
  credential_revision: 3,
  export_id: 'export',
  export_revision: 4,
  repo_kind: 'linked_worktree',
  checkout_identity_digest: 'a'.repeat(64),
  approved_settings_digest: 'b'.repeat(64),
  principal: { kind: 'human', id: 'owner' },
  grant_revision: 5,
  grant_permissions: ['read', 'execute'],
  revision: 6,
  active: true,
  observed_at_ms: 123456,
};
const input = { host_id: 'host', export_id: 'export', expected_export_revision: 4 };
const list = (bindings: unknown[] = [binding]) => ({
  bindings,
  observed_state_only: true,
  execution_ready: false,
});
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}
test('remote binding codec preserves observed descriptor, tombstone and service principal without manufacturing execution authority', () => {
  assert.deepEqual(decodeRemoteBindings(list(), 'workspace', 'project'), [binding]);
  const tombstone = decodeRemoteBinding(
    { ...binding, active: false, principal: { kind: 'service', id: 'service-principal' } },
    'workspace',
    'project',
  );
  assert.equal(tombstone.active, false);
  assert.equal(tombstone.principal.kind, 'service');
  assert.equal('execution_ready' in tombstone, false);
  assert.equal('path' in tombstone, false);
});
test('remote binding codec rejects scope drift, unsafe revisions, duplicates, caller paths and fabricated readiness', () => {
  for (const patch of [
    { workspace_id: 'other' },
    { project_id: 'other' },
    { export_revision: 0 },
    { host_enrollment_epoch: Number.MAX_SAFE_INTEGER + 1 },
    { revision: 1.5 },
    { repo_kind: 'arbitrary' },
    { checkout_identity_digest: 'bad' },
    { grant_permissions: ['read', 'read'] },
    { principal: { kind: 'human', id: 'owner', path: '/private' } },
    { root: '/private/checkout' },
  ])
    assert.throws(() => decodeRemoteBinding({ ...binding, ...patch }, 'workspace', 'project'));
  assert.throws(() => decodeRemoteBindings({ ...list(), execution_ready: true }, 'workspace', 'project'));
  assert.throws(() => decodeRemoteBindings(list([binding, binding]), 'workspace', 'project'));
  assert.throws(() =>
    decodeRemoteBindings(
      list(Array.from({ length: 17 }, (_, i) => ({ ...binding, id: `b${i}`, export_id: `e${i}` }))),
      'workspace',
      'project',
    ),
  );
  for (const command of [
    { ...input, expected_export_revision: 0 },
    { ...input, export_id: '../checkout' },
    { ...input, principal: { kind: 'human', id: 'forged' } },
    { ...input, descriptor: binding },
  ])
    assert.throws(() => decodeRemoteBindingInput(command));
});
test('create/rebind/revoke send explicit target and captured CAS exactly once without local fallback', async (context) => {
  const calls: { path: string; method: string | undefined; body: unknown }[] = [];
  let next = { ...binding };
  context.mock.method(globalThis, 'fetch', async (path: string, init?: RequestInit) => {
    calls.push({ path, method: init?.method, body: JSON.parse(String(init?.body)) });
    assert.equal(init?.credentials, 'same-origin');
    return Response.json(next);
  });
  const saved = decodeRemoteBinding(binding, 'workspace', 'project');
  await saveRemoteBinding('workspace', 'project', input, null);
  next = { ...binding, revision: 7 };
  await saveRemoteBinding('workspace', 'project', input, saved);
  next = { ...binding, revision: 7, active: false };
  await revokeRemoteBinding('workspace', 'project', saved);
  assert.deepEqual(calls, [
    { path: '/v1/web/workspaces/workspace/projects/project/remote-bindings', method: 'POST', body: input },
    {
      path: '/v1/web/workspaces/workspace/projects/project/remote-bindings/binding',
      method: 'PUT',
      body: { ...input, expected_revision: 6 },
    },
    {
      path: '/v1/web/workspaces/workspace/projects/project/remote-bindings/binding/revoke',
      method: 'POST',
      body: { expected_revision: 6 },
    },
  ]);
});
test('unknown mutation outcome requires explicit read; catalog refresh never retries or clears reconciliation requirement', async (context) => {
  let writes = 0;
  context.mock.method(globalThis, 'fetch', async (_path: string, init?: RequestInit) => {
    if (init?.method === 'GET') return Response.json(list());
    writes++;
    throw new TypeError('accepted response lost');
  });
  const client = new RemoteBindingsController('workspace', 'project', () => {});
  await client.refresh();
  await client.save(input, 'binding', 6);
  assert.equal(writes, 1);
  assert.equal(client.state.needsReload, true);
  await client.refresh();
  assert.equal(client.state.needsReload, true);
  await assert.rejects(client.save(input, 'binding', 6));
  assert.equal(writes, 1);
  await client.refresh(true);
  assert.equal(client.state.needsReload, false);
  client.stop();
});
test('newer observed binding revision cannot silently upgrade the CAS attached to an older user selection', async (context) => {
  let revision = 6,
    writes = 0;
  context.mock.method(globalThis, 'fetch', async (_path: string, init?: RequestInit) => {
    if (init?.method !== 'GET') writes++;
    return Response.json(list([{ ...binding, revision }]));
  });
  const client = new RemoteBindingsController('workspace', 'project', () => {});
  await client.refresh();
  revision = 7;
  await client.refresh();
  await assert.rejects(client.save(input, 'binding', 6));
  await assert.rejects(client.revoke('binding', 6));
  assert.equal(writes, 0);
  client.stop();
});
test('permission loss while save is pending clears observed metadata and fences the late accepted response', async (context) => {
  const write = deferred<Response>();
  let allowed = true,
    writes = 0;
  context.mock.method(globalThis, 'fetch', async (_path: string, init?: RequestInit) => {
    if (init?.method === 'GET') return allowed ? Response.json(list()) : new Response(null, { status: 403 });
    writes++;
    return write.promise;
  });
  const client = new RemoteBindingsController('workspace', 'project', () => {});
  await client.refresh();
  const pending = client.save(input, 'binding', 6);
  allowed = false;
  await client.refresh();
  assert.equal(client.state.status, 'forbidden');
  assert.deepEqual(client.state.bindings, []);
  write.resolve(Response.json({ ...binding, revision: 7 }));
  assert.equal(await pending, null);
  await client.refresh();
  assert.equal(client.state.status, 'forbidden');
  assert.deepEqual(client.state.bindings, []);
  assert.equal(writes, 1);
  client.stop();
});
test('scope disposal ignores late metadata; a newer read wins despite a fetch implementation ignoring abort', async (context) => {
  const first = deferred<Response>(),
    second = deferred<Response>();
  let calls = 0;
  context.mock.method(globalThis, 'fetch', () => (++calls === 1 ? first.promise : second.promise));
  const client = new RemoteBindingsController('workspace', 'project', () => {});
  const old = client.refresh(),
    fresh = client.refresh();
  second.resolve(Response.json(list([{ ...binding, revision: 7 }])));
  await fresh;
  first.resolve(Response.json(list()));
  await old;
  assert.equal(client.state.bindings[0].revision, 7);
  client.stop();
  assert.deepEqual(client.state.bindings, []);
});

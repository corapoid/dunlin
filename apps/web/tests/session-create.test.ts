import { test } from 'node:test';
import assert from 'node:assert/strict';
import { SessionCreationClient, SessionCreationError } from '../src/lib/session-create.ts';
const identity = { userId: 'owner', deviceId: 'browser' };
const selected = {
  model: 'approved-model',
  mode: 'approved-mode',
  max_seconds: 60,
  max_output_bytes: 1048576,
};
for(const remote of [false,true])test(`private MCP choices and target are frozen across an uncertain ${remote?'remote':'local'} retry`, async context=>{
  const bodies:string[]=[];
  context.mock.method(globalThis,'fetch',async(path:string,init?:RequestInit)=>{
    if(path==='/v1/web/session')return current();
    bodies.push(String(init?.body));
    if(bodies.length===1)throw new TypeError('lost acceptance');
    return accepted(true);
  });
  const connection={provider_id:'work',connection_id:'private-one',revision:3};
  const binding={id:'binding',revision:1};
  const intent=new SessionCreationClient(identity).begin('workspace','project',{
    profile:{agent_id:'agent',version:1,digest:'a'.repeat(64)},mcp_connections:[connection],
    ...(remote?{target:{kind:'remote' as const,binding}}:{}),
  });
  connection.connection_id='different';connection.revision=4;
  binding.id='different';binding.revision=2;
  await assert.rejects(intent.submit());
  await intent.submit();
  assert.equal(bodies[0],bodies[1]);
  assert.deepEqual(JSON.parse(bodies[1]).mcp_connections,[{provider_id:'work',connection_id:'private-one',revision:3}]);
  assert.deepEqual(JSON.parse(bodies[1]).target,remote?{kind:'remote',binding:{id:'binding',revision:1}}:undefined);
});
test('MCP choices reject duplicate providers, invalid revisions and stale remote targets',()=>{
  const profile={agent_id:'agent',version:1,digest:'a'.repeat(64)};
  const connection={provider_id:'work',connection_id:'private-one',revision:1};
  for(const choices of [[connection,connection],[{...connection,revision:0}],[{...connection,revision:Number.MAX_SAFE_INTEGER+1}],[{...connection,provider_id:'*'}]]){
    assert.throws(()=>new SessionCreationClient(identity).begin('workspace','project',{profile,mcp_connections:choices}));
  }
  assert.throws(()=>new SessionCreationClient(identity).begin('workspace','project',{profile,mcp_connections:[connection],target:{kind:'remote',binding:{id:'binding',revision:0}}}));
});
function current(user = 'owner', device = 'browser') {
  return Response.json({ user: { id: user }, device_id: device });
}
function accepted(replayed = false, patch: Record<string, unknown> = {}) {
  return Response.json(
    {
      session: {
        id: 'session-one',
        workspace: 'workspace',
        project: 'project',
        owner: 'owner',
        phase: 'active',
        epoch: 1,
        sequence: 2,
        ...patch,
      },
      admission: { replayed, session_is_acceptance_snapshot: true },
    },
    { status: 202, headers: { 'Idempotency-Replayed': String(replayed) } },
  );
}
test('explicit intent freezes UUID, profile, scope and exact bytes across lost acceptance then manual retry', async (context) => {
  const calls: { path: string; body: string; key: string }[] = [];
  let committed = 0;
  context.mock.method(globalThis, 'fetch', async (path: string, init?: RequestInit) => {
    if (path === '/v1/web/session') return current();
    const headers = new Headers(init?.headers);
    calls.push({ path, body: String(init?.body), key: headers.get('Idempotency-Key')! });
    assert.equal(headers.get('X-Dunlin-Expected-User'), identity.userId);
    assert.equal(headers.get('X-Dunlin-Expected-Device'), identity.deviceId);
    assert.equal(init?.credentials, 'same-origin');
    assert.equal(init?.redirect, 'error');
    assert.equal(init?.cache, 'no-store');
    if (calls.length === 1) {
      committed++;
      throw new TypeError('accepted response lost');
    }
    return accepted(true);
  });
  const mutable = { ...selected };
  const client = new SessionCreationClient(identity);
  const intent = client.begin('workspace', 'project', mutable);
  const key = intent.key;
  mutable.model = 'different-model';
  assert.match(key, /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i);
  await assert.rejects(
    intent.submit(),
    (error: unknown) => error instanceof SessionCreationError && error.kind === 'uncertain',
  );
  assert.equal(intent.state.status, 'uncertain');
  assert.equal(calls.length, 1);
  assert.throws(() => client.begin('workspace', 'other-project', selected));
  assert.equal(client.current, intent);
  const receipt = await intent.submit();
  assert.equal(receipt.replayed, true);
  assert.equal(receipt.sessionIsAcceptanceSnapshot, true);
  assert.equal(receipt.acceptedPhase, 'active');
  assert.deepEqual(calls[1], calls[0]);
  assert.deepEqual(JSON.parse(calls[1].body), selected);
  assert.equal(committed, 1);
  assert.equal(intent.key, key);
  assert.equal(await intent.submit(), receipt);
  assert.equal(calls.length, 2, 'Accepted intent is a historical receipt, not another POST');
  assert.notEqual(
    client.begin('workspace', 'project', selected).key,
    key,
    'Only a new explicit resolved intent gets a new key',
  );
});
test('concurrent submit calls share one in-flight attempt', async (context) => {
  let finishIdentity!: (response: Response) => void;
  let posts = 0;
  const wait = new Promise<Response>((resolve) => {
    finishIdentity = resolve;
  });
  context.mock.method(globalThis, 'fetch', async (path: string) => {
    if (path === '/v1/web/session') return wait;
    posts++;
    return accepted();
  });
  const intent = new SessionCreationClient(identity).begin('workspace', 'project', selected);
  const first = intent.submit();
  const second = intent.submit();
  assert.equal(first, second);
  finishIdentity(current());
  assert.equal(await first, await second);
  assert.equal(posts, 1);
});
test('capacity and server failures never auto retry or replace the intent key', async (context) => {
  for (const status of [429, 503, 408]) {
    const keys: string[] = [];
    context.mock.method(globalThis, 'fetch', async (path: string, init?: RequestInit) => {
      if (path === '/v1/web/session') return current();
      keys.push(new Headers(init?.headers).get('Idempotency-Key')!);
      return keys.length === 1 ? new Response(null, { status }) : accepted();
    });
    const intent = new SessionCreationClient(identity).begin('workspace', 'project', selected);
    await assert.rejects(intent.submit());
    assert.equal(keys.length, 1);
    assert.equal(intent.state.status, 'uncertain');
    await intent.submit();
    assert.equal(keys.length, 2);
    assert.equal(keys[0], keys[1]);
  }
});
test('conflict and authority rejection block automatic retry and another create identity', async (context) => {
  for (const status of [400, 401, 403, 404, 409]) {
    let posts = 0;
    context.mock.method(globalThis, 'fetch', async (path: string) => {
      if (path === '/v1/web/session') return current();
      posts++;
      return new Response(null, { status });
    });
    const client = new SessionCreationClient(identity),
      intent = client.begin('workspace', 'project', selected);
    await assert.rejects(
      intent.submit(),
      (error: unknown) =>
        error instanceof SessionCreationError && error.kind === 'rejected' && error.status === status,
    );
    assert.equal(intent.state.status, 'blocked');
    await assert.rejects(intent.submit());
    assert.equal(posts, 1);
    assert.throws(() => client.begin('workspace', 'project', selected));
  }
});
test('identity or device change cannot carry an uncertain create into a new browser principal', async (context) => {
  let identityCalls = 0,
    posts = 0;
  context.mock.method(globalThis, 'fetch', async (path: string) => {
    if (path === '/v1/web/session')
      return ++identityCalls === 1 ? current() : current('other-owner', 'other-device');
    posts++;
    throw new TypeError('unknown response');
  });
  const intent = new SessionCreationClient(identity).begin('workspace', 'project', selected);
  await assert.rejects(intent.submit());
  await assert.rejects(
    intent.submit(),
    (error: unknown) => error instanceof SessionCreationError && error.kind === 'identity',
  );
  assert.equal(posts, 1);
  assert.equal(intent.state.status, 'blocked');
});
test('cookie identity switch after preflight sends immutable expected authority and blocks rejected intent', async (context) => {
  let posts = 0,
    work = 0;
  let cookieIdentity = { ...identity };
  context.mock.method(globalThis, 'fetch', async (path: string, init?: RequestInit) => {
    if (path === '/v1/web/session') {
      const response = current();
      cookieIdentity = { userId: 'other-owner', deviceId: 'other-device' };
      return response;
    }
    posts++;
    const headers = new Headers(init?.headers);
    assert.equal(headers.get('X-Dunlin-Expected-User'), identity.userId);
    assert.equal(headers.get('X-Dunlin-Expected-Device'), identity.deviceId);
    if (
      headers.get('X-Dunlin-Expected-User') !== cookieIdentity.userId ||
      headers.get('X-Dunlin-Expected-Device') !== cookieIdentity.deviceId
    )
      return new Response(null, { status: 403 });
    work++;
    return accepted();
  });
  const client = new SessionCreationClient(identity),
    intent = client.begin('workspace', 'project', selected),
    key = intent.key;
  await assert.rejects(
    intent.submit(),
    (error: unknown) => error instanceof SessionCreationError && error.status === 403,
  );
  assert.equal(work, 0);
  assert.equal(posts, 1);
  assert.equal(intent.key, key);
  assert.equal(intent.state.status, 'blocked');
  await assert.rejects(intent.submit());
  assert.equal(posts, 1);
  assert.throws(() => client.begin('workspace', 'project', selected));
  assert.equal(client.current, intent);
});
test('malformed or mismatched accepted snapshots retain uncertainty and exact identity', async (context) => {
  for (const patch of [
    { workspace: 'other' },
    { project: 'other' },
    { owner: 'other' },
    { id: '../unsafe' },
    { epoch: -1 },
    { sequence: 1.5 },
    { phase: ['active'] },
  ]) {
    context.mock.method(globalThis, 'fetch', async (path: string) =>
      path === '/v1/web/session' ? current() : accepted(false, patch),
    );
    const intent = new SessionCreationClient(identity).begin('workspace', 'project', selected),
      key = intent.key;
    await assert.rejects(intent.submit());
    assert.equal(intent.state.status, 'uncertain');
    assert.equal(intent.accepted, null);
    assert.equal(intent.key, key);
  }
  context.mock.method(globalThis, 'fetch', async (path: string) =>
    path === '/v1/web/session'
      ? current()
      : Response.json(
          {
            session: {
              id: 'session-one',
              workspace: 'workspace',
              project: 'project',
              owner: 'owner',
              phase: 'active',
              epoch: 1,
              sequence: 2,
            },
            admission: { replayed: true, session_is_acceptance_snapshot: true },
          },
          { status: 202, headers: { 'Idempotency-Replayed': 'false' } },
        ),
  );
  const intent = new SessionCreationClient(identity).begin('workspace', 'project', selected);
  await assert.rejects(intent.submit());
  assert.equal(intent.state.status, 'uncertain');
});
test('a historical interrupted acceptance never claims the actor is currently usable', async (context) => {
  context.mock.method(globalThis, 'fetch', async (path: string) =>
    path === '/v1/web/session' ? current() : accepted(true, { phase: 'interrupted', epoch: 1 }),
  );
  const receipt = await new SessionCreationClient(identity).begin('workspace', 'project', selected).submit();
  assert.equal(receipt.acceptedPhase, 'interrupted');
  assert.equal(receipt.sessionIsAcceptanceSnapshot, true);
  assert.equal(Object.isFrozen(receipt), true);
});
test('invalid or unsafe scope/profile never starts networking or consumes a workflow intent', () => {
  const client = new SessionCreationClient(identity);
  for (const path of ['../unsafe', 'https://external.example', 'project?key=secret', 'space here'])
    assert.throws(() => client.begin('workspace', path, selected));
  for (const profile of [
    { ...selected, model: '' },
    { ...selected, mode: '\n' },
    { ...selected, model: '界'.repeat(128) },
    { ...selected, max_seconds: 0 },
    { ...selected, max_output_bytes: 1023 },
  ])
    assert.throws(() => client.begin('workspace', 'project', profile));
  assert.equal(client.current, null);
});
test('published profile intent pins version and digest across uncertain retry with no raw model overrides', async (context) => {
  const bodies: string[] = [];
  context.mock.method(globalThis, 'fetch', async (path: string, init?: RequestInit) => {
    if (path === '/v1/web/session') return current();
    bodies.push(String(init?.body));
    if (bodies.length === 1) throw new TypeError('lost');
    return accepted(true);
  });
  const pinned = { profile: { agent_id: 'agent', version: 2, digest: 'a'.repeat(64) } };
  const client = new SessionCreationClient(identity);
  const intent = client.begin('workspace', 'project', pinned);
  pinned.profile.version = 3;
  pinned.profile.digest = 'b'.repeat(64);
  await assert.rejects(intent.submit());
  await intent.submit();
  assert.deepEqual(bodies, [bodies[0], bodies[0]]);
  assert.deepEqual(JSON.parse(bodies[0]), {
    profile: { agent_id: 'agent', version: 2, digest: 'a'.repeat(64) },
  });
  for (const profile of [
    { agent_id: '../bad', version: 1, digest: 'a'.repeat(64) },
    { agent_id: 'agent', version: 0, digest: 'a'.repeat(64) },
    { agent_id: 'agent', version: 1, digest: 'wrong' },
  ])
    assert.throws(() => new SessionCreationClient(identity).begin('workspace', 'project', { profile }));
});

test('remote intent pins the binding revision across an uncertain retry and rejects ambiguous targets', async context => {
  const bodies:string[]=[];
  context.mock.method(globalThis,'fetch',async(path:string,init?:RequestInit)=> {
    if(path==='/v1/web/session')return current();
    bodies.push(String(init?.body));
    if(bodies.length===1)throw new TypeError('lost acceptance');
    return accepted(true);
  });
  const selected={profile:{agent_id:'agent',version:1,digest:'a'.repeat(64)},target:{kind:'remote' as const,binding:{id:'binding',revision:2}}};
  const client=new SessionCreationClient(identity);
  const intent=client.begin('workspace','project',selected);
  selected.target.binding.revision=3;
  selected.target.binding.id='other-binding';
  await assert.rejects(intent.submit());
  await intent.submit();
  assert.equal(bodies.length,2);assert.equal(bodies[0],bodies[1]);
  assert.deepEqual(JSON.parse(bodies[1]).target,{kind:'remote',binding:{id:'binding',revision:2}});
  for(const target of [{kind:'remote',binding:{id:'binding',revision:0}},{kind:'remote',binding:{id:'binding',revision:2,root:'/private'}},{kind:'local',revision:1}]) {
    assert.throws(()=>new SessionCreationClient(identity).begin('workspace','project',{profile:selected.profile,target} as never));
  }
});

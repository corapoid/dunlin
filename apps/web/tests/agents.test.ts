import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  decodeDefinition,
  decodeDraft,
  decodeDrafts,
  decodeProfiles,
  decodeVersion,
  saveDraft,
  publishDraft,
} from '../src/lib/agents.ts';
import { TeamRequestError } from '../src/lib/teams.ts';
const definition = {
  schema_version: 1 as const,
  name: 'Agent',
  description: 'Opis',
  harness: 'opencode',
  model: 'wire-model',
  mode: 'wire-mode',
  instructions: 'Top\ncontext',
  skills: [{ id: 'review', version: '1.2.0', instructions: 'Check text' }],
  max_seconds: 120,
  max_output_bytes: 1048576,
};
const draft = {
  id: 'agent',
  workspace_id: 'workspace',
  project_id: 'project',
  revision: 2,
  definition,
  updated_by: 'owner',
  updated_at: 123,
};
const version = {
  agent_id: 'agent',
  version: 1,
  draft_revision: 2,
  definition,
  digest: 'a'.repeat(64),
  published_by: 'owner',
  published_at: 124,
};
test('agent codec preserves literal versioned instructions and schema defaults without resolving resources', () => {
  const decoded = decodeDefinition(definition);
  assert.deepEqual(decoded, definition);
  decoded.skills[0].instructions = 'modified';
  assert.equal(definition.skills[0].instructions, 'Check text');
  const { instructions, description, skills, ...required } = definition;
  void instructions;
  void description;
  void skills;
  assert.deepEqual(decodeDefinition(required), {
    ...required,
    instructions: null,
    description: '',
    skills: [],
  });
  assert.equal(
    decodeDefinition({ ...definition, instructions: 'https://example.test/file' }).instructions,
    'https://example.test/file',
  );
  assert.deepEqual(decodeDraft(draft, 'workspace', 'project'), draft);
  assert.deepEqual(decodeVersion(version), version);
});
test('agent codec enforces UTF8, combined instruction and skill bounds, unknown fields and immutable version provenance', () => {
  for (const patch of [
    { schema_version: 3 },
    { name: '界'.repeat(43) },
    { model: 'bad\nmodel' },
    { harness: '/bin/custom' },
    { instructions: 'x'.repeat(16385) },
    { skills: Array.from({ length: 9 }, (_, i) => ({ id: `s${i}`, version: '1', instructions: 'x' })) },
    { skills: [definition.skills[0], definition.skills[0]] },
    { skills: [{ id: 'skill', version: 'latest url', instructions: 'x' }] },
    { skills: [{ id: 'skill', version: '1', instructions: '\0' }] },
    {
      instructions: 'a'.repeat(12000),
      skills: [{ id: 'skill', version: '1', instructions: 'b'.repeat(5000) }],
    },
    { extra: 'unknown' },
  ])
    assert.throws(() => decodeDefinition({ ...definition, ...patch }));
  assert.throws(() => decodeDraft(draft, 'other', 'project'));
  assert.throws(() => decodeDraft({ ...draft, revision: 0 }, 'workspace', 'project'));
  assert.throws(() => decodeVersion({ ...version, digest: 'not-a-digest' }));
  assert.throws(() => decodeDrafts({ drafts: [draft, draft] }, 'workspace', 'project'));
  assert.throws(() =>
    decodeProfiles({ profiles: [version, version], configured_harness: 'opencode', can_edit: true }),
  );
  assert.deepEqual(decodeProfiles({ profiles: [], configured_harness: null, can_edit: false }), {
    profiles: [],
    configured_harness: null,
    can_edit: false,
  });
});
test('draft save and publish use explicit CAS, do not replay conflict, and do not publish unsaved data', async (context) => {
  const calls: { path: string; method: string | undefined; body: unknown }[] = [];
  context.mock.method(globalThis, 'fetch', async (path: string, options?: RequestInit) => {
    calls.push({ path, method: options?.method, body: JSON.parse(String(options?.body)) });
    return new Response(null, { status: 409 });
  });
  await assert.rejects(
    saveDraft('workspace', 'project', definition, draft),
    (e: unknown) => e instanceof TeamRequestError && e.status === 409,
  );
  assert.equal(calls.length, 1);
  assert.deepEqual(calls[0], {
    path: '/v1/web/workspaces/workspace/projects/project/agents/agent',
    method: 'PUT',
    body: { expected_revision: 2, definition },
  });
  await assert.rejects(publishDraft('workspace', 'project', draft));
  assert.equal(calls.length, 2);
  assert.deepEqual(calls[1].body, { expected_revision: 2 });
});
test('publication response must refer to exactly the submitted draft revision', async (context) => {
  context.mock.method(globalThis, 'fetch', async () => Response.json({ ...version, draft_revision: 3 }));
  await assert.rejects(publishDraft('workspace', 'project', draft));
});
test('MCP policy is explicit schema two, exact and has no personal credentials',()=>{
  const pin={provider_id:'work',name:'lookup',definition_digest:'a'.repeat(64)};
  const value={...definition,schema_version:2,mcp:[pin]};
  assert.deepEqual(decodeDefinition(value),value);
  assert.deepEqual(decodeDefinition({...definition,mcp:[]}),definition);
  for(const patch of [{schema_version:1},{mcp:[pin,pin]},{mcp:[{...pin,name:'*'}]},
    {mcp:[{...pin,provider_id:'https://endpoint.example'}]},{mcp:[{...pin,definition_digest:'A'.repeat(64)}]},
    {mcp:[{...pin,connection_id:'private'}]},{mcp:[{...pin,token:'private'}]},
    {mcp:Array.from({length:33},(_,i)=>({...pin,name:`tool-${i}`}))},
    {mcp:Array.from({length:9},(_,i)=>({...pin,provider_id:`provider-${i}`}))}])assert.throws(()=>decodeDefinition({...value,...patch}));
});

test('remote MCP capability is explicit, typed and independent from local execution',()=>{
  const base={profiles:[version],configured_harness:null,can_edit:false,remote_sessions_supported:true};
  assert.equal(decodeProfiles(base).remote_mcp_sessions_supported,undefined);
  for(const supported of [true,false]) {
    const catalog=decodeProfiles({...base,mcp_sessions_supported:false,remote_mcp_sessions_supported:supported});
    assert.equal(catalog.remote_mcp_sessions_supported,supported);
    assert.equal(catalog.mcp_sessions_supported,false);
  }
  for(const value of ['true',1,null,{}])assert.throws(()=>decodeProfiles({...base,remote_mcp_sessions_supported:value}));
});

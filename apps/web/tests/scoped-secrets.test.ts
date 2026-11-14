import {test} from 'node:test';
import assert from 'node:assert/strict';
import {ScopedSecretsClient,decodeSecretGrant,decodeSecretStatus,isSecretProvider} from '../src/lib/scoped-secrets.ts';
const profile={agent_id:'agent',version:1,digest:'a'.repeat(64)};
const grant={id:'sgt-one',credential:'work.api',principal_kind:'human' as const,principal_id:'user',profile,provider_id:'dunlin-secret-api',revision:1,active:true,credential_revision:1,read_only:true};
const json=(v:unknown)=>new Response(JSON.stringify(v),{headers:{'Content-Type':'application/json'}});
const client=(fetcher:typeof fetch)=>new ScopedSecretsClient('workspace','project','host','user',fetcher);
test('scoped metadata decoders reject injected secrets, invalid versions and ambiguous identities',()=>{
  assert.deepEqual(decodeSecretGrant(grant),grant);
  assert.equal(isSecretProvider('dunlin-secret-api'),true);assert.equal(isSecretProvider('dunlin-secret-'),false);
  for(const v of [{...grant,value:'private'},{...grant,reference:'scoped-secret:private'},{...grant,revision:2**53},{...grant,credential_revision:0},{...grant,provider_id:'external'},{...grant,profile:{...profile,version:0}},{...grant,profile:{...profile,token:'private'}},{...grant,read_only:'yes'}])assert.throws(()=>decodeSecretGrant(v));
  for(const v of [{credential:'..',revision:1,state:'active'},{credential:'api',revision:0,state:'active'},{credential:'api',revision:1,state:'unknown'},{credential:'api',revision:1,state:'active',value:'private'}])assert.throws(()=>decodeSecretStatus(v));
});
test('recipient catalog pins identity, profile ID/version/digest, safe revisions and fixed host path',async()=>{
  let calls=0;const c=client(async function(this:unknown,url,init){assert.equal(this,undefined);calls++;assert.equal(url,'/v1/web/workspaces/workspace/projects/project/hosts/host/secret-bindings?agent_id=agent&version=1&digest='+profile.digest);assert.equal(new Headers(init?.headers).get('x-dunlin-expected-user'),'user');assert.equal(init?.cache,'no-store');assert.equal(init?.redirect,'error');assert.equal(init?.credentials,'same-origin');return json({bindings:[grant],execution_supported:true});});
  assert.deepEqual(await c.available(profile),{bindings:[grant],execution_supported:true});assert.equal(calls,1);
  for(const changed of [{...grant,principal_id:'another'},{...grant,principal_kind:'service'},{...grant,active:false},{...grant,profile:{...profile,agent_id:'same-content-different-agent'}},{...grant,profile:{...profile,version:2}}])await assert.rejects(client(async()=>json({bindings:[changed],execution_supported:true})).available(profile));
  await assert.rejects(client(async()=>json({bindings:[grant,grant],execution_supported:true})).available(profile));
});
test('mutations use expected revisions exactly once and never expose failed response bodies',async()=>{
  const calls:unknown[]=[];const c=client(async(url,init)=>{calls.push({url,body:JSON.parse(String(init?.body))});return new Response('private-backend-token',{status:409});});
  await assert.rejects(c.install('work.api',1,{version:1},'private-value'),e=>e instanceof Error&&!e.message.includes('private'));
  assert.deepEqual(calls,[{url:'/v1/web/workspaces/workspace/projects/project/hosts/host/scoped-secrets/work.api',body:{expected_revision:1,policy:{version:1},value:'private-value'}}]);
  await assert.rejects(c.disable({credential:'work.api',revision:1,state:'active'}));assert.equal(calls.length,2);
  await assert.rejects(c.revoke(grant));assert.equal(calls.length,3);
  assert.deepEqual(calls[2],{url:'/v1/web/workspaces/workspace/projects/project/hosts/host/scoped-secrets/work.api/grants/sgt-one/revoke',body:{expected_revision:1}});
});
test('grant changes pin source credential and full published profile; substituted acknowledgement is rejected',async()=>{
  const input={expected_revision:0,credential_revision:1,principal_kind:'human' as const,principal_id:'user',profile,provider_id:grant.provider_id,read_only:true};
  const c=client(async(_url,init)=>{assert.deepEqual(JSON.parse(String(init?.body)),input);return json(grant);});assert.deepEqual(await c.setGrant('work.api',input),grant);
  await assert.rejects(client(async()=>json({...grant,profile:{...profile,agent_id:'other'}})).setGrant('work.api',input));
  await assert.rejects(client(async()=>json({...grant,read_only:false})).setGrant('work.api',input));
  await assert.rejects(client(async()=>json({...grant,credential:'different'})).setGrant('work.api',input));
});
test('terminal revocation works at safe-integer ceiling while positive writes cannot wrap',async()=>{
  const max=Number.MAX_SAFE_INTEGER;const c=client(async(url,init)=>{assert.equal(JSON.parse(String(init?.body)).expected_revision,max);return json(String(url).endsWith('/revoke')?{...grant,active:false,revision:max}:{credential:'work.api',revision:max,state:'disabled'});});
  await c.disable({credential:'work.api',revision:max,state:'active'});await c.revoke({...grant,revision:max});await assert.rejects(c.install('work.api',max,{},'value'));
});
test('late success after cancellation and oversized metadata are rejected',async()=>{
  let resolve:(v:Response)=>void=()=>{};const pending=new Promise<Response>(done=>{resolve=done;});const controller=new AbortController();const request=client(async()=>pending).available(profile,controller.signal);controller.abort();resolve(json({bindings:[grant],execution_supported:true}));await assert.rejects(request,e=>e instanceof DOMException&&e.name==='AbortError');
  await assert.rejects(client(async()=>new Response(' '.repeat(1048577))).statuses());
  await assert.rejects(client(async()=>json({credentials:[{credential:'work.api',revision:1,state:'active'},{credential:'work.api',revision:2,state:'active'}],execution_supported:true})).statuses());
});

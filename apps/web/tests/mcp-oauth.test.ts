import { test } from 'node:test';
import assert from 'node:assert/strict';
import { McpOAuthClient, decodeAuthorization, decodeCatalog, decodeConnections, decodeProviders, rememberMcpReturn, consumeMcpReturn } from '../src/lib/mcp-oauth.ts';
const origin='http://127.0.0.1:8787';
const connection={id:'mcp-one',revision:1,status:'connected' as const,provider_id:'fixture',display_name:'Konto służbowe'};
function authorization() {const url=new URL('https://auth.example.test/authorize');url.search=new URLSearchParams({response_type:'code',client_id:'client',redirect_uri:`${origin}/v1/web/mcp/oauth/fixture/callback`,code_challenge_method:'S256',code_challenge:'a'.repeat(43),state:'b'.repeat(43)}).toString();return {authorization_url:url.href,connection_id:'mcp-one',revision:1};}
const json=(value:unknown)=>new Response(JSON.stringify(value),{status:200});
test('catalog checks are explicit CAS writes and validate observations against the selected connection',async()=>{
  const catalog={connection_id:connection.id,revision:1,protocol_version:'2026-07-28',tools:['lookup']};
  let calls=0;
  const client=new McpOAuthClient('workspace','user',origin,async(input,init)=>{
    calls++;assert.equal(input,'/v1/web/workspaces/workspace/mcp-connections/mcp-one/inspect');assert.equal(init?.method,'POST');assert.equal(init?.body,JSON.stringify({expected_revision:1}));return json(catalog);
  });
  assert.deepEqual(await client.inspect(connection),catalog);assert.equal(calls,1);
  for(const invalid of [{...catalog,connection_id:'other'},{...catalog,revision:2},{...catalog,tools:['x','x']},{...catalog,tools:Array(513).fill('x')},{...catalog,tools:['private\ncontent']},{...catalog,protocol_version:'unknown'}])assert.throws(()=>decodeCatalog(invalid,connection));
  await assert.rejects(client.inspect({...connection,status:'disabled'}));assert.equal(calls,1);
});
test('MCP client preserves the native browser fetch calling convention',async()=>{
  const fetcher:typeof fetch=async function(this:unknown){assert.equal(this,undefined);return json({providers:[{id:'fixture',name:'Fixture'}]});};
  assert.deepEqual(await new McpOAuthClient('workspace','user',origin,fetcher).providers(),[{id:'fixture',name:'Fixture'}]);
});
test('MCP catalog decoders reject duplicates, unsafe revisions and oversized/malformed data',()=>{
  assert.deepEqual(decodeConnections({connections:[connection]}),[connection]);
  assert.deepEqual(decodeProviders({providers:[{id:'fixture',name:'Fixture'}]}),[{id:'fixture',name:'Fixture'}]);
  for(const value of [{connections:[connection,connection]},{connections:[{...connection,revision:2**53}]},{connections:[{...connection,status:'maybe'}]},{connections:Array(65).fill(connection)},{connections:[{...connection,id:'..'}]},{connections:[{...connection,display_name:'\nsecret'}]}])assert.throws(()=>decodeConnections(value));
  assert.throws(()=>decodeProviders({providers:[{id:'fixture',name:'Fixture'},{id:'fixture',name:'Another'}]}));
});
test('authorization navigation is pinned to code/S256 and the same callback; secret-shaped parameters never navigate',()=>{
  const valid=authorization();assert.deepEqual(decodeAuthorization(valid,'fixture',origin),valid);
  for(const [key,value] of [['redirect_uri','https://attacker.example/callback'],['code_challenge_method','plain'],['code_verifier','private-verifier'],['access_token','private-token'],['client_secret','private-secret']]){
    const url=new URL(valid.authorization_url);url.searchParams.set(key,value);assert.throws(()=>decodeAuthorization({...valid,authorization_url:url.href},'fixture',origin));
  }
  const duplicate=new URL(valid.authorization_url);duplicate.searchParams.append('state','other');assert.throws(()=>decodeAuthorization({...valid,authorization_url:duplicate.href},'fixture',origin));
  assert.throws(()=>decodeAuthorization({...valid,authorization_url:'javascript:alert(1)'},'fixture',origin));
  assert.throws(()=>decodeAuthorization({...valid,connection_id:'other'},'fixture',origin,{...connection,status:'disabled'}));
});
test('mutations send a fixed same-origin path, identity expectation and CAS once; errors never replay or expose body',async()=>{
  let calls=0;const fetcher:typeof fetch=async(input,init)=>{calls++;assert.equal(input,'/v1/web/workspaces/workspace/mcp-connections/mcp-one/disconnect');assert.equal(init?.credentials,'same-origin');assert.equal(init?.redirect,'error');assert.equal(new Headers(init?.headers).get('x-dunlin-expected-user'),'user');assert.equal(init?.body,JSON.stringify({expected_revision:1}));return new Response('private-backend-token',{status:409});};
  const client=new McpOAuthClient('workspace','user',origin,fetcher);await assert.rejects(client.disconnect(connection),error=>error instanceof Error&&!error.message.includes('private-backend'));assert.equal(calls,1);
});
test('late successful response after scope cancellation is discarded even if fetch ignores abort',async()=>{
  let resolve:(response:Response)=>void=()=>{};const response=new Promise<Response>(done=>{resolve=done;});
  const client=new McpOAuthClient('workspace','user',origin,async()=>response);
  const controller=new AbortController();const request=client.authorize('fixture','Work',controller.signal);controller.abort();resolve(json(authorization()));await assert.rejects(request,error=>error instanceof DOMException&&error.name==='AbortError');
});
test('streamed replies are bounded before JSON parsing and malformed payload errors contain no server content',async()=>{
  const client=new McpOAuthClient('workspace','user',origin,async()=>new Response('private-backend-content'));
  await assert.rejects(client.connections(),error=>error instanceof Error&&!error.message.includes('private-backend'));
  const large=new McpOAuthClient('workspace','user',origin,async()=>new Response(new Uint8Array(262145)));
  await assert.rejects(large.connections());
});
test('return preference contains only user/workspace, is consumed once, and storage failure never blocks authorization',()=>{
  const values=new Map<string,string>();const storage={getItem:(key:string)=>values.get(key)??null,setItem:(key:string,value:string)=>{values.set(key,value);},removeItem:(key:string)=>{values.delete(key);}};
  Object.defineProperty(globalThis,'sessionStorage',{value:storage,configurable:true});
  rememberMcpReturn('user','workspace');assert.deepEqual(JSON.parse([...values.values()][0]),{userId:'user',workspace:'workspace'});assert.equal(consumeMcpReturn('other'),undefined);assert.equal(values.size,0);
  rememberMcpReturn('user','workspace');assert.equal(consumeMcpReturn('user'),'workspace');assert.equal(consumeMcpReturn('user'),undefined);
  Object.defineProperty(globalThis,'sessionStorage',{get:()=>{throw new Error('disabled storage');},configurable:true});assert.doesNotThrow(()=>rememberMcpReturn('user','workspace'));assert.equal(consumeMcpReturn('user'),undefined);delete (globalThis as Record<string,unknown>).sessionStorage;
});
test('catalog tool pins match the observed connection/provider and exact tool names',()=>{
  const pin={provider_id:'fixture',name:'lookup',definition_digest:'a'.repeat(64)};
  const value={connection_id:connection.id,revision:connection.revision,protocol_version:'2026-07-28',tools:['lookup'],tool_pins:[pin]};
  assert.deepEqual(decodeCatalog(value,connection),value);
  for(const pins of [[pin,pin],[{...pin,provider_id:'other'}],[{...pin,name:'not-observed'}],
    [{...pin,definition_digest:'A'.repeat(64)}],[{...pin,token:'private'}]])assert.throws(()=>decodeCatalog({...value,tool_pins:pins},connection));
  assert.deepEqual(decodeCatalog({...value,tool_pins:[]},connection).tool_pins,[]);
});

test('revocation receipts preserve uncertain outcomes and block unfinished operations',async()=>{
  const receipt={state:'unconfirmed' as const,revision:2,confirmed:1,unconfirmed:1};
  const disabled={...connection,status:'disabled' as const,revision:2,revocation:receipt};
  assert.deepEqual(decodeConnections({connections:[disabled]}),[disabled]);
  for(const revocation of [{...receipt,revision:3},{...receipt,state:'confirmed_known_tokens'},{...receipt,confirmed:-1},{...receipt,unconfirmed:256},{...receipt,state:'completed'}])assert.throws(()=>decodeConnections({connections:[{...disabled,revocation}]}));
  assert.throws(()=>decodeConnections({connections:[{...disabled,status:'connected'}]}));
  const reauthorized={...disabled,status:'connected' as const,revision:3};
  assert.deepEqual(decodeConnections({connections:[reauthorized]}),[reauthorized]);
  let calls=0;const client=new McpOAuthClient('workspace','user',origin,async()=>{calls++;return json({});});
  const unfinished={...disabled,revocation:{...receipt,state:'unfinished' as const}};
  await assert.rejects(client.authorize('fixture',undefined,undefined,unfinished));
  await assert.rejects(client.revoke(unfinished));assert.equal(calls,0);
});
test('provider revoke sends one owner-bound CAS and validates the durable result revision',async()=>{
  let calls=0;const receipt={connection_id:connection.id,revision:2,confirmed:2,unconfirmed:0,provider_revocation:'confirmed_known_tokens'};
  const client=new McpOAuthClient('workspace','user',origin,async(path,init)=>{
    calls++;assert.equal(path,'/v1/web/workspaces/workspace/mcp-connections/mcp-one/revoke');assert.equal(init?.method,'POST');assert.equal(init?.body,JSON.stringify({expected_revision:1}));assert.equal(new Headers(init?.headers).get('x-dunlin-expected-user'),'user');return json(receipt);
  });
  assert.deepEqual(await client.revoke(connection),{state:'confirmed_known_tokens',revision:2,confirmed:2,unconfirmed:0});assert.equal(calls,1);
  receipt.revision=3;await assert.rejects(client.revoke(connection));assert.equal(calls,2);
});
test('lost provider-revoke reply never triggers an automatic retry',async()=>{
  let calls=0;const client=new McpOAuthClient('workspace','user',origin,async()=>{calls++;throw new TypeError('network failed');});
  await assert.rejects(client.revoke(connection));assert.equal(calls,1);
});

test('cleanup receipts remain scoped to their sweep and can describe historical retired records',async()=>{
  const revocation={state:'confirmed_known_tokens' as const,revision:2,confirmed:2,unconfirmed:0};
  const cleanup={revision:1,connection_revision:2,status:'sweep_complete' as const,delete_acknowledged:2,unconfirmed:0};
  const row={...connection,revision:3,revocation,cleanup};
  assert.deepEqual(decodeConnections({connections:[row]}),[row]);
  for(const invalid of [{...cleanup,connection_revision:4},{...cleanup,revision:0},{...cleanup,unconfirmed:1},{...cleanup,delete_acknowledged:0},{...cleanup,status:'erased_forever'}])assert.throws(()=>decodeConnections({connections:[{...row,cleanup:invalid}]}));
  let calls=0;const client=new McpOAuthClient('workspace','user',origin,async(path,init)=>{
    calls++;assert.equal(path,'/v1/web/workspaces/workspace/mcp-connections/mcp-one/cleanup');assert.equal(init?.body,JSON.stringify({expected_revision:3,expected_cleanup_revision:1}));
    return json({connection_id:connection.id,...cleanup,revision:2,connection_revision:3});
  });
  assert.equal((await client.cleanup(row)).revision,2);assert.equal(calls,1);
});
test('cleanup never retries a lost reply and cannot start without a confirmed retired credential',async()=>{
  let calls=0;const client=new McpOAuthClient('workspace','user',origin,async()=>{calls++;throw new TypeError('network failed');});
  await assert.rejects(client.cleanup(connection));assert.equal(calls,0);
  const row={...connection,status:'disabled' as const,revision:2,revocation:{state:'confirmed_known_tokens' as const,revision:2,confirmed:1,unconfirmed:0}};
  await assert.rejects(client.cleanup(row));assert.equal(calls,1);
});

test('reconciliation is explicit, preserves the receipt and never retries a lost response',async()=>{
  const row={...connection,status:'disabled' as const,revision:2,revocation:{state:'unfinished' as const,revision:2,confirmed:1,unconfirmed:1,reconciliation_supported:true}};
  let calls=0;const client=new McpOAuthClient('workspace','user',origin,async(path,init)=>{
    calls++;assert.equal(path,'/v1/web/workspaces/workspace/mcp-connections/mcp-one/reconcile-revocation');assert.equal(init?.method,'POST');assert.equal(init?.body,JSON.stringify({expected_revision:2}));
    return json({connection_id:connection.id,revision:2,provider_revocation:'unconfirmed',confirmed:1,unconfirmed:1});
  });
  assert.deepEqual(await client.reconcileRevocation(row),{state:'unconfirmed',revision:2,confirmed:1,unconfirmed:1});assert.equal(calls,1);
  await assert.rejects(client.reconcileRevocation(connection));assert.equal(calls,1);
  await assert.rejects(client.reconcileRevocation({...row,revocation:{...row.revocation,reconciliation_supported:false}}));assert.equal(calls,1);
  assert.throws(()=>decodeConnections({connections:[{...row,revocation:{...row.revocation,reconciliation_supported:'yes'}}]}));
  const lost=new McpOAuthClient('workspace','user',origin,async()=>{calls++;throw new TypeError('network lost');});
  await assert.rejects(lost.reconcileRevocation(row));assert.equal(calls,2);
  for(const invalid of [{revision:3},{confirmed:2,unconfirmed:0,provider_revocation:'confirmed_known_tokens'},{provider_revocation:'unfinished'}]){
    const malformed=new McpOAuthClient('workspace','user',origin,async()=>json({connection_id:row.id,revision:2,confirmed:1,unconfirmed:1,provider_revocation:'unconfirmed',...invalid}));
    await assert.rejects(malformed.reconcileRevocation(row));
  }
});

test('storage counters distinguish active capacity from retained uncertain history',()=>{
  const storage={pending_references:1,retained_history:768,uncertain_history:256,available_slots:255,cleanup_available:true};
  const row={...connection,storage};assert.deepEqual(decodeConnections({connections:[row]}),[row]);
  for(const invalid of [{...storage,available_slots:256},{...storage,uncertain_history:257},{...storage,retained_history:255},{...storage,retained_history:1025},{...storage,pending_references:257},{...storage,cleanup_available:'yes'}])assert.throws(()=>decodeConnections({connections:[{...row,storage:invalid}]}));
});
test('cleanup remains available from retained history after the last AS receipt changes',async()=>{
  const row={...connection,status:'disabled' as const,revision:3,storage:{pending_references:0,retained_history:1,uncertain_history:0,available_slots:256,cleanup_available:true}};
  let calls=0;const client=new McpOAuthClient('workspace','user',origin,async(path,init)=>{
    calls++;assert.equal(path,'/v1/web/workspaces/workspace/mcp-connections/mcp-one/cleanup');assert.equal(init?.body,JSON.stringify({expected_revision:3,expected_cleanup_revision:0}));
    return json({connection_id:row.id,revision:1,connection_revision:3,status:'sweep_complete',delete_acknowledged:1,unconfirmed:0,reclaimed:0,pruned:0});
  });assert.equal((await client.cleanup(row)).reclaimed,0);assert.equal(calls,1);
  const bad=new McpOAuthClient('workspace','user',origin,async()=>json({connection_id:row.id,revision:1,connection_revision:3,status:'sweep_complete',delete_acknowledged:1,unconfirmed:0,reclaimed:2,pruned:0}));await assert.rejects(bad.cleanup(row));
});

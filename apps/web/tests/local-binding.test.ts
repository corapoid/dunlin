import { test } from 'node:test';
import assert from 'node:assert/strict';
import { decodeLocalBinding, decodeLocalBindingView, decodeLocalBindingInput, saveLocalBinding, revokeLocalBinding, LocalBindingController } from '../src/lib/local-binding.ts';
import type { LocalBinding } from '../src/lib/local-binding.ts';
const ordinary:LocalBinding={workspace_id:'workspace',project_id:'project',native_project_id:'native',host_id:'local',repo_kind:'git',settings_digest:'a'.repeat(64),revision:2,active:true};
const linked:LocalBinding={...ordinary,repo_kind:'linked_worktree',metadata_approval_revision:4};
const view=(binding:unknown=ordinary,current=true,manage=true)=>({binding,configuration_current:current,can_manage:manage});
function deferred<T>() {let resolve!:(v:T)=>void;const promise=new Promise<T>(done=>{resolve=done;});return{promise,resolve};}
test('local binding codecs preserve explicit linked approval and saved metadata when own Read is unavailable',()=>{
  assert.deepEqual(decodeLocalBinding(linked,'workspace','project'),linked);
  assert.deepEqual(decodeLocalBindingView(view(linked,false,false),'workspace','project'),view(linked,false,false));
  assert.deepEqual(decodeLocalBindingInput({native_project_id:'native'}),{native_project_id:'native'});
  for(const patch of [{workspace_id:'other'},{native_project_id:'../escape'},{host_id:'remote'},{root:'/secret'},{revision:Number.MAX_SAFE_INTEGER+1},{metadata_approval_revision:0},{metadata_approval_revision:null}])
    assert.throws(()=>decodeLocalBinding({...linked,...patch},'workspace','project'));
  assert.throws(()=>decodeLocalBinding({...linked,metadata_approval_revision:undefined},'workspace','project'));
  assert.throws(()=>decodeLocalBinding({...ordinary,metadata_approval_revision:1},'workspace','project'));
  assert.throws(()=>decodeLocalBindingView(view(null,true),'workspace','project'));
  assert.throws(()=>decodeLocalBindingView({...view(),execution_ready:true},'workspace','project'));
  assert.throws(()=>decodeLocalBindingInput({native_project_id:'native',path:'/secret'}));
});
test('local binding saves preserve ordinary wire JSON and pin explicit linked approval plus CAS',async context=>{
  const calls:{path:string;method?:string;body:unknown}[]=[];let next:unknown=ordinary;
  context.mock.method(globalThis,'fetch',async(path:string,init?:RequestInit)=>{calls.push({path,method:init?.method,body:JSON.parse(String(init?.body))});return new Response(JSON.stringify(next));});
  await saveLocalBinding('workspace','project',{native_project_id:'native'},null);
  assert.deepEqual(calls[0].body,{native_project_id:'native',expected_revision:null});
  next=linked;
  await saveLocalBinding('workspace','project',{native_project_id:'native',metadata_approval_revision:4},ordinary);
  assert.deepEqual(calls[1].body,{native_project_id:'native',metadata_approval_revision:4,expected_revision:2});
  next={...linked,active:false,revision:3};await revokeLocalBinding('workspace','project',linked);
  assert.deepEqual(calls[2].body,{expected_revision:2});
  assert.equal(calls[2].path,'/v1/web/workspaces/workspace/projects/project/binding/revoke');
});
test('local binding unknown write has one attempt and requires explicit reload before another intent',async context=>{
  let count=0;
  context.mock.method(globalThis,'fetch',async(_path:string,init?:RequestInit)=>{
    if(init?.method==='PUT'){count++;throw new Error('response lost');}
    return new Response(JSON.stringify(view()));
  });
  const c=new LocalBindingController('workspace','project',()=>{});await c.refresh();
  assert.equal(await c.save({native_project_id:'native'},2),null);
  assert.equal(count,1);assert.equal(c.state.needsReload,true);
  await assert.rejects(c.save({native_project_id:'native'},2));assert.equal(count,1);
  await c.refresh();assert.equal(c.state.needsReload,false);c.stop();
});
test('local binding stale approval is terminal conflict and readonly configuration cannot be edited',async context=>{
  let writes=0;let manage=true;
  context.mock.method(globalThis,'fetch',async(_path:string,init?:RequestInit)=>{
    if(init?.method==='PUT'){writes++;return new Response('{}',{status:409});}
    return new Response(JSON.stringify(view(linked,false,manage)));
  });
  const c=new LocalBindingController('workspace','project',()=>{});await c.refresh();
  assert.equal(await c.save({native_project_id:'native',metadata_approval_revision:3},2),null);
  assert.equal(writes,1);assert.equal(c.state.view,null);
  manage=false;await c.refresh();await assert.rejects(c.revoke(2));assert.equal(writes,1);c.stop();
});
test('local binding catalog/auth invalidation fences late mutation receipts and clears CAS baseline',async context=>{
  const accepted=deferred<Response>();let denied=false;
  context.mock.method(globalThis,'fetch',async(_path:string,init?:RequestInit)=>{
    if(init?.method==='PUT')return accepted.promise;
    return denied?new Response('{}',{status:403}):new Response(JSON.stringify(view()));
  });
  const c=new LocalBindingController('workspace','project',()=>{});await c.refresh();
  const mutation=c.save({native_project_id:'native'},2);denied=true;await c.refresh();
  accepted.resolve(new Response(JSON.stringify({...ordinary,revision:3})));
  assert.equal(await mutation,null);assert.equal(c.state.status,'forbidden');assert.equal(c.state.view,null);c.stop();
});
test('local binding selection teardown fences a late read from another project',async context=>{
  const delayed=deferred<Response>();context.mock.method(globalThis,'fetch',()=>delayed.promise);
  const c=new LocalBindingController('workspace','project',()=>{});const read=c.refresh();c.stop();
  delayed.resolve(new Response(JSON.stringify(view(linked))));await read;
  assert.equal(c.state.view,null);
});

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { activeReaderIds, decodeSharing, readSharing, saveSharing, sharingPath, SharingConflict, SharingError } from '../src/lib/sharing.ts';
const view = () => ({ session_id: 'session-one', owner_id: 'owner', revision: 0, readers: [] as {user_id:string;name:string;active:boolean}[], eligible_readers: [{ user_id: 'reader', name: 'Reader' }], audit: [] as {revision:number;actor_id:string;added:string[];removed:string[];refreshed:string[];created_at:string}[] });
test('sharing decoder binds session and owner while retaining inactive reader and bounded audit',()=>{
 const value=view();value.revision=2;value.readers=[{user_id:'reader',name:'Reader',active:false}];value.audit=[{revision:2,actor_id:'owner',added:['reader'],removed:[],refreshed:[],created_at:'2026-11-11T10:00:00Z'}];
 assert.deepEqual(decodeSharing(value,'session-one','owner'),{...value,eligible_readers_truncated:false});
 assert.throws(()=>decodeSharing(value,'other-session','owner'));assert.throws(()=>decodeSharing(value,'session-one','other-user'));
 for(const broken of [
  {...value,readers:[...value.readers,...value.readers]},
  {...value,eligible_readers:[...value.eligible_readers,...value.eligible_readers]},
  {...value,readers:[{user_id:'owner',name:'Owner',active:true}]},
  {...value,readers:[{user_id:'reader',name:'Reader',active:'yes'}]},
  {...value,revision:-1}, {...value,revision:Number.MAX_SAFE_INTEGER+1},
  {...value,audit:[{...value.audit[0],revision:3}]}, {...value,audit:[{...value.audit[0],added:['reader','reader']}]},
  {...value,audit:[{...value.audit[0],created_at:'yesterday'}]},
 ]) assert.throws(()=>decodeSharing(broken,'session-one','owner'));
});
test('sharing endpoints reject unsafe paths and external redirects before fetch',async(context)=>{
 let calls=0;context.mock.method(globalThis,'fetch',async()=>{calls++;return Response.json(view());});
 for(const id of ['https://evil.example','../other','a/b','a?token=x','a%2fb','a#secret','']) {
  assert.throws(()=>sharingPath(id));await assert.rejects(readSharing(id,'owner'));
 }
 assert.equal(calls,0);
});
test('explicit sharing update uses one same-origin CAS request and validated authoritative response',async(context)=>{
 const calls:{url:unknown;init:RequestInit|undefined}[]=[];
 context.mock.method(globalThis,'fetch',async(url:unknown,init?:RequestInit)=>{calls.push({url,init});return Response.json({...view(),revision:1,readers:[{user_id:'reader',name:'Reader',active:true}]});});
 const controller=new AbortController();const updated=await saveSharing('session-one','owner',0,['reader'],controller.signal);
 assert.equal(updated.revision,1);assert.equal(calls.length,1);assert.equal(calls[0].url,'/v1/web/interactive-sessions/session-one/sharing');
 assert.equal(calls[0].init?.method,'PUT');assert.equal(calls[0].init?.credentials,'same-origin');assert.equal(calls[0].init?.redirect,'error');assert.equal(calls[0].init?.signal,controller.signal);
 assert.deepEqual(JSON.parse(String(calls[0].init?.body)),{expected_revision:0,reader_ids:['reader']});
});
test('CAS conflict refetches authoritative state once without replaying changes',async(context)=>{
 const methods:string[]=[];context.mock.method(globalThis,'fetch',async(_url:unknown,init?:RequestInit)=>{methods.push(init?.method??'GET');return init?.method==='PUT'?new Response(null,{status:409}):Response.json({...view(),revision:4});});
 await assert.rejects(saveSharing('session-one','owner',0,['reader']),(error:unknown)=>error instanceof SharingConflict && error.current.revision===4);
 assert.deepEqual(methods,['PUT','GET']);
});
test('invalid CAS input and duplicate recipients never send mutations',async(context)=>{
 let calls=0;context.mock.method(globalThis,'fetch',async()=>{calls++;return Response.json(view());});
 for(const [revision,readers] of [[-1,['reader']],[0,['reader','reader']],[0,['owner']],[0,Array.from({length:33},(_,i)=>`reader-${i}`)]] as [number,string[]][]) await assert.rejects(saveSharing('session-one','owner',revision,readers));
 assert.equal(calls,0);
});
test('authorization failure does not retry or fetch owner audit and cross-user responses are rejected',async(context)=>{
 let calls=0;context.mock.method(globalThis,'fetch',async()=>{calls++;return new Response(null,{status:403});});
 await assert.rejects(saveSharing('session-one','owner',0,['reader']),(error:unknown)=>error instanceof SharingError && error.status===403);assert.equal(calls,1);
 context.mock.method(globalThis,'fetch',async()=>Response.json({...view(),owner_id:'another-user'}));await assert.rejects(readSharing('session-one','owner'));
});

test('initial sharing selection preserves active grants and never silently refreshes inactive entries',()=>{
 const value=view();value.readers=[{user_id:'active',name:'Active',active:true},{user_id:'reader',name:'Stale but eligible',active:false},{user_id:'gone',name:'No longer eligible',active:false}];
 assert.deepEqual(activeReaderIds(decodeSharing(value,'session-one','owner')),['active']);
 assert.equal(value.readers.length,3);
});

test('truncated eligible catalog keeps existing grants available for explicit revocation',()=>{
 const value={...view(),eligible_readers_truncated:true,eligible_readers:Array.from({length:256},(_,i)=>({user_id:`candidate-${i}`,name:`Candidate ${i}`})),readers:[{user_id:'outside-first-page',name:'Existing reader',active:true}]};
 const decoded=decodeSharing(value,'session-one','owner');assert.equal(decoded.eligible_readers_truncated,true);assert.deepEqual(activeReaderIds(decoded),['outside-first-page']);
 assert.equal(decodeSharing(view(),'session-one','owner').eligible_readers_truncated,false);
 assert.throws(()=>decodeSharing({...value,eligible_readers_truncated:'yes'},'session-one','owner'));
});

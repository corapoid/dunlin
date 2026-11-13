import { test } from 'node:test';
import assert from 'node:assert/strict';
import { chat, reduceChat, ROOT, toolInputText, toolStatusLabel } from '../src/lib/ahp.ts';
import { AhpObserver, ahpEndpoint } from '../src/lib/ahp-live.ts';
import type { ObserverState } from '../src/lib/ahp-live.ts';
const uri = 'ahp-chat:/one';
// Socket callbacks mutate observer state; read it across that callback boundary.
function observedChat(client: AhpObserver) { return client.state.chat; }
const session = { resource: 'ahp-session:/one', title: 'Private', defaultChat: uri, status: 1 };
const state = () => chat({ resource: uri, turns: [], interactivity: 'read-only' }, uri);
class Socket {
  onopen: (() => void) | null = null;
  onclose: ((event: {code:number}) => void) | null = null;
  onerror: (() => void) | null = null;
  onmessage: ((event:{data:string})=>void) | null = null;
  sent: any[] = [];
  send(data:string) { this.sent.push(JSON.parse(data)); }
  close() { this.onclose?.({code:1000}); }
  receive(value:unknown) { this.onmessage?.({data:JSON.stringify(value)}); }
  result(value:unknown, id=this.sent.at(-1).id) { this.receive({jsonrpc:'2.0',id,result:value}); }
}
function fixture() {
  const sockets: Socket[]=[]; const values: ObserverState[]=[];
  const client=new AhpObserver('https://a.tail.ts.net', value=>values.push(value),()=>{const socket=new Socket();sockets.push(socket);return socket as unknown as WebSocket;});
  client.start(); const socket=sockets[0];socket.onopen?.();
  socket.result({protocolVersion:'1.0.0',serverSeq:0,snapshots:[{resource:ROOT,fromSeq:0,state:{}}]});
  socket.result({items:[session]});
  return {client,socket,sockets,values};
}
test('same-origin observer endpoint rejects arbitrary discovery paths',()=>{
 assert.equal(ahpEndpoint('https://a.tail.ts.net'),'wss://a.tail.ts.net/v1/web/ahp');
 assert.throws(()=>ahpEndpoint('https://evil.example'));assert.throws(()=>ahpEndpoint('http://localhost','/other'));
});
test('read-only projection preserves reasoning, tools, completion and errors',()=>{
 let s=state();
 for(const a of [
 {type:'chat/turnStarted',turnId:'t',message:{text:'hello'},startedAt:'2026-11-11T10:01:00.123Z'},
 {type:'chat/responsePart',turnId:'t',part:{kind:'reasoning',id:'r',content:''}},
 {type:'chat/reasoning',turnId:'t',partId:'r',content:'thought'},
 {type:'chat/toolCallStart',turnId:'t',toolCallId:'tool',displayName:'Inspect'},
 {type:'chat/toolCallReady',turnId:'t',toolCallId:'tool',confirmed:'not-needed',invocationMessage:'Inspect'},
 {type:'chat/toolCallComplete',turnId:'t',toolCallId:'tool',result:{success:false,content:[{type:'text',text:'failed'}]}},
 {type:'chat/error',turnId:'t',part:{kind:'error',error:{message:'failure'}}},
 ]) s=reduceChat(s,a);
 assert.equal(s.activeTurn,undefined);assert.equal(s.turns[0].state,'error');
 assert.equal(s.turns[0].responseParts[0].content,'thought');
 assert.equal(s.turns[0].responseParts[1].toolCall.success,false);assert.equal(s.interactivity,'read-only');
 assert.throws(()=>chat({...s,interactivity:'hidden'},uri));
});
test('selection is snapshot based and duplicates do not duplicate streamed text',()=>{
 const f=fixture();f.client.select(uri);f.socket.result({snapshot:{resource:uri,fromSeq:3,state:reduceChat(state(),{type:'chat/turnStarted',turnId:'t',message:{text:'a'}})}});
 const action=(seq:number,a:unknown)=>f.socket.receive({jsonrpc:'2.0',method:'action',params:{channel:uri,serverSeq:seq,action:a}});
 action(4,{type:'chat/responsePart',turnId:'t',part:{kind:'markdown',id:'p',content:''}});
 action(8,{type:'chat/delta',turnId:'t',partId:'p',content:'live'});action(8,{type:'chat/delta',turnId:'t',partId:'p',content:'live'});
 assert.equal(f.client.state.chat?.activeTurn?.responseParts[0].content,'live');
 assert.ok(f.socket.sent.every(m=>['initialize','subscribe','listSessions'].includes(m.method)));f.client.stop();
});
test('late subscription snapshot cannot replace a newly selected conversation',()=>{
 const f=fixture();f.socket.receive({jsonrpc:'2.0',method:'root/sessionAdded',params:{channel:ROOT,summary:{...session,resource:'ahp-session:/two',defaultChat:'ahp-chat:/two'}}});
 f.client.select(uri);const id=f.socket.sent.at(-1).id;f.client.select('ahp-chat:/two');
 f.socket.result({snapshot:{resource:uri,fromSeq:0,state:state()}},id);
 assert.equal(f.client.state.chat,null);assert.equal(f.client.state.selected,'ahp-chat:/two');f.client.stop();
});
test('reconnect replays actions and refreshes root listing without write methods',(context)=>{
 context.mock.timers.enable({apis:['setTimeout']});const f=fixture();f.client.select(uri);f.socket.result({snapshot:{resource:uri,fromSeq:4,state:state()}});
 f.socket.onclose?.({code:1006});context.mock.timers.tick(1000);const next=f.sockets[1];next.onopen?.();
 assert.equal(next.sent[0].method,'reconnect');assert.equal(next.sent[0].params.lastSeenServerSeq,4);
 next.result({type:'snapshot',snapshots:[{resource:ROOT,fromSeq:9,state:{}},{resource:uri,fromSeq:9,state:state()}]});
 assert.equal(next.sent.at(-1).method,'listSessions');next.result({items:[]});assert.equal(f.client.state.chat,null);
 assert.ok([...f.socket.sent,...next.sent].every(m=>!['dispatchAction','createSession'].includes(m.method)));f.client.stop();
});
test('revocation removes protected data and stale socket messages are ignored',()=>{
 const f=fixture();f.client.select(uri);f.socket.result({snapshot:{resource:uri,fromSeq:1,state:state()}});
 f.socket.onclose?.({code:1008});assert.equal(f.client.state.status,'forbidden');assert.equal(f.client.state.chat,null);assert.deepEqual(f.client.state.sessions,[]);
 f.socket.result({items:[session]});assert.deepEqual(f.client.state.sessions,[]);f.client.stop();
});
test('root summary notifications update parent status independently of chat actions',()=>{
 const f=fixture();f.socket.receive({jsonrpc:'2.0',method:'root/sessionSummaryChanged',params:{channel:ROOT,session:session.resource,changes:{status:8,title:'Working'}}});
 assert.equal(f.client.state.sessions[0].status,8);f.client.stop();
});

test('unknown reconnect identity reinitializes and requests a fresh selected snapshot',(context)=>{
 context.mock.timers.enable({apis:['setTimeout']});const f=fixture();f.client.select(uri);f.socket.result({snapshot:{resource:uri,fromSeq:4,state:state()}});
 f.socket.onclose?.({code:1006});context.mock.timers.tick(1000);const socket=f.sockets[1];socket.onopen?.();
 socket.receive({jsonrpc:'2.0',id:socket.sent[0].id,error:{code:-32600,message:'Initialize this client first'}});
 const fresh=f.sockets[2];fresh.onopen?.();assert.equal(fresh.sent[0].method,'initialize');assert.deepEqual(fresh.sent[0].params.initialSubscriptions,[ROOT]);
 fresh.result({protocolVersion:'1.0.0',serverSeq:4,snapshots:[{resource:ROOT,fromSeq:4,state:{}}]});assert.equal(fresh.sent.at(-1).method,'subscribe');assert.deepEqual(fresh.sent.at(-1).params.view,{turns:32});
 assert.equal(f.client.state.chat,null);f.client.stop();
});
test('malformed and oversized wire frames fail closed',()=>{
 for(const data of ['{', 'a'.repeat(1024*1024+1)]) { const f=fixture();f.socket.onmessage?.({data});assert.equal(f.client.state.status,'error');assert.deepEqual(f.client.state.sessions,[]);f.client.stop(); }
});
test('initial pagination accumulates pages before publishing and rejects repeated cursors',()=>{
 const sockets:Socket[]=[];const client=new AhpObserver('http://localhost',()=>{},()=>{const s=new Socket();sockets.push(s);return s as unknown as WebSocket;});client.start();const s=sockets[0];s.onopen?.();
 s.result({protocolVersion:'1.0.0',serverSeq:0,snapshots:[]});s.result({items:[session],nextCursor:'next'});
 assert.equal(client.state.sessions.length,0);assert.equal(s.sent.at(-1).params.cursor,'next');
 s.result({items:[{...session,resource:'ahp-session:/two',defaultChat:'ahp-chat:/two'}]});assert.equal(client.state.sessions.length,2);client.stop();
});

test('reconnect before initial chat snapshot reacquires its missing baseline',(context)=>{
 context.mock.timers.enable({apis:['setTimeout']});const f=fixture();f.client.select(uri);
 f.socket.onclose?.({code:1006});context.mock.timers.tick(1000);const next=f.sockets[1];next.onopen?.();
 next.result({type:'replay',actions:[],missing:[]});
 assert.equal(next.sent.at(-1).method,'subscribe');next.result({snapshot:{resource:uri,fromSeq:5,state:state()}});
 assert.equal(f.client.state.chat?.resource,uri);f.client.stop();
});

test('host MCP parameters show exact arguments without internal connection pins',()=>{
 const input={provider_id:'work',connection_id:'private-account',connection_revision:3,definition_digest:'a'.repeat(64),tool_name:'lookup',arguments:{query:'Exact user-approved query',limit:2}};
 const text=JSON.stringify(input);
 for(const value of [input,text])assert.deepEqual(JSON.parse(toolInputText(value,'dunlin:mcp:host-call')),input.arguments);
 assert.equal(toolInputText(text,'acp-call'),text);
 assert.equal(toolInputText('{invalid','dunlin:mcp:host-call'),'Parametry narzędzia niedostępne.');
 assert.equal(toolInputText({arguments:null},'dunlin:mcp:host-call'),'Parametry narzędzia niedostępne.');
 assert.equal(JSON.stringify(input),text,'Formatting never edits the frozen approval payload.');
});

test('live tool parameters match standard snapshots after ready and completion',()=>{
 const input = JSON.stringify({path:'README.md'});
 let live=reduceChat(state(),{type:'chat/turnStarted',turnId:'t',message:{text:'inspect'}});
 live=reduceChat(live,{type:'chat/toolCallStart',turnId:'t',toolCallId:'tool',toolName:'read',displayName:'Read'});
 live=reduceChat(live,{type:'chat/toolCallDelta',turnId:'t',toolCallId:'tool',content:input});
 assert.equal(live.activeTurn?.responseParts[0].toolCall.partialInput,input);
 live=reduceChat(live,{type:'chat/toolCallReady',turnId:'t',toolCallId:'tool',toolInput:input,confirmed:'not-needed',invocationMessage:'Read file'});
 const snapshot=chat({resource:uri,turns:[],interactivity:'read-only',activeTurn:{id:'t',message:{text:'inspect'},responseParts:[{kind:'toolCall',toolCall:{status:'running',toolCallId:'tool',toolName:'read',displayName:'Read',toolInput:input,confirmed:'not-needed',invocationMessage:'Read file'}}]}},uri);
 assert.equal(live.activeTurn?.responseParts[0].toolCall.partialInput,undefined);
 assert.equal(toolInputText(live.activeTurn?.responseParts[0].toolCall.toolInput),toolInputText(snapshot.activeTurn?.responseParts[0].toolCall.toolInput));
 live=reduceChat(live,{type:'chat/toolCallComplete',turnId:'t',toolCallId:'tool',result:{success:true,pastTenseMessage:'Read',content:[]}});
 assert.equal(live.activeTurn?.responseParts[0].toolCall.partialInput,undefined);
 assert.equal(toolInputText(live.activeTurn?.responseParts[0].toolCall.toolInput),input);
 assert.equal(toolInputText({uri:'ahp-resource:/large',contentType:'application/json',sizeHint:2048}),'Odwołanie do parametrów: ahp-resource:/large (application/json) · 2048 B');
});

function writer() {
 const f=fixture();
 f.socket.receive({jsonrpc:'2.0',method:'root/sessionSummaryChanged',params:{channel:ROOT,session:session.resource,changes:{_meta:{'dunlin.queueText':true}}}});
 f.client.select(uri);f.socket.result({snapshot:{resource:uri,fromSeq:3,state:{...state(),interactivity:'full'}}});
 return f;
}
function echo(f:ReturnType<typeof writer>,seq:number,rejectionReason?:string,socket=f.socket) {
 const dispatch=f.socket.sent.find(m=>m.method==='dispatchAction');const clientId=f.socket.sent[0].params.clientId;
 socket.receive({jsonrpc:'2.0',method:'action',params:{channel:uri,serverSeq:seq,origin:{clientId,clientSeq:dispatch.params.clientSeq},action:dispatch.params.action,...(rejectionReason?{rejectionReason}:{})}});
}
test('only a full chat with explicit queue capability can dispatch user text',()=>{
 const observer=fixture();observer.client.select(uri);observer.socket.result({snapshot:{resource:uri,fromSeq:0,state:state()}});observer.client.setDraft('hello');assert.equal(observer.client.queueText(),false);observer.client.stop();
 const f=writer();f.client.setDraft('hello');assert.equal(f.client.queueText(),true);
 const command=f.socket.sent.at(-1);assert.equal(command.method,'dispatchAction');assert.equal(command.id,undefined);
 assert.equal(command.params.action.kind,'queued');assert.deepEqual(command.params.action.message,{text:'hello',origin:{kind:'user'}});
 assert.equal(f.client.state.pendingCount,1);assert.equal(f.client.state.chat?.queuedMessages.length,1);
 echo(f,4);assert.equal(f.client.state.pendingCount,0);assert.equal(f.client.state.chat?.queuedMessages.length,1);
 echo(f,4);assert.equal(f.client.state.chat?.queuedMessages.length,1);
 const id=command.params.action.id;
 f.socket.receive({jsonrpc:'2.0',method:'action',params:{channel:uri,serverSeq:5,action:{type:'chat/turnStarted',turnId:'t',queuedMessageId:id,message:command.params.action.message}}});
 assert.equal(f.client.state.chat?.queuedMessages?.length ?? 0,0);assert.equal(f.client.state.chat?.activeTurn?.id,'t');f.client.stop();
});
test('rejected optimistic commands restore draft without applying the action',()=>{
 const f=writer();f.client.setDraft('rejected');f.client.queueText();f.client.setDraft('new draft');echo(f,4,'Permission denied');
 assert.equal(f.client.state.pendingCount,0);assert.deepEqual(f.client.state.chat?.queuedMessages,undefined);
 assert.equal(f.client.state.draft,'new draft\nrejected');assert.match(f.client.state.commandError,/Permission denied/);f.client.stop();
});
test('snapshot reconnect retries exact immutable identity and old echo acknowledges without replaying a consumed prompt',(context)=>{
 context.mock.timers.enable({apis:['setTimeout']});const f=writer();f.client.setDraft('once');f.client.queueText();const original=f.socket.sent.at(-1);
 f.socket.onclose?.({code:1006});f.client.setDraft('offline draft');assert.equal(f.client.queueText(),false);context.mock.timers.tick(1000);const next=f.sockets[1];next.onopen?.();
 next.result({type:'snapshot',snapshots:[{resource:ROOT,fromSeq:20,state:{}},{resource:uri,fromSeq:20,state:{...state(),interactivity:'full',turns:[{id:'finished',message:original.params.action.message,responseParts:[],state:'complete'}]}}]});
 next.result({items:[{...session,_meta:{'dunlin.queueText':true}}]});
 assert.deepEqual(next.sent.at(-1),original);echo(f,4,undefined,next);
 assert.equal(f.client.state.pendingCount,0);assert.equal(f.client.state.chat?.turns.length,1);assert.equal(f.client.state.chat?.queuedMessages,undefined);assert.equal(f.client.state.draft,'offline draft');f.client.stop();
});
test('reconnect replay confirms a pending command and does not resend acknowledged text',(context)=>{
 context.mock.timers.enable({apis:['setTimeout']});const f=writer();f.client.setDraft('once');f.client.queueText();const command=f.socket.sent.at(-1);const clientId=f.socket.sent[0].params.clientId;
 f.socket.onclose?.({code:1006});context.mock.timers.tick(1000);const next=f.sockets[1];next.onopen?.();
 next.result({type:'replay',actions:[{channel:uri,serverSeq:4,origin:{clientId,clientSeq:command.params.clientSeq},action:command.params.action}],missing:[]});next.result({items:[{...session,_meta:{'dunlin.queueText':true}}]});
 assert.equal(f.client.state.pendingCount,0);assert.equal(next.sent.filter(m=>m.method==='dispatchAction').length,0);assert.equal(f.client.state.chat?.queuedMessages.length,1);f.client.stop();
});
test('unknown reconnect identity with an uncertain outbox never initializes or auto redispatches',(context)=>{
 context.mock.timers.enable({apis:['setTimeout']});const f=writer();f.client.setDraft('uncertain');f.client.queueText();f.socket.onclose?.({code:1006});context.mock.timers.tick(1000);const next=f.sockets[1];next.onopen?.();
 next.receive({jsonrpc:'2.0',id:next.sent[0].id,error:{code:-32600,message:'Initialize this client first'}});
 assert.equal(f.sockets.length,2);assert.equal(f.client.state.status,'error');assert.match(f.client.state.error,/Sprawdź historię/);assert.equal(next.sent.some(m=>m.method==='dispatchAction'),false);f.client.stop();
});

test('UTF-8 and JSON frame preflight retain oversized drafts without dispatch or sequence consumption',()=>{
 const f=writer();
 for(const [text,message] of [['界'.repeat(16384),/UTF-8/],['\u0001'.repeat(16384),/transportu/]] as const) {
  f.client.setDraft(text);assert.equal(f.client.queueText(),false);assert.equal(f.client.state.draft,text);assert.match(f.client.state.commandError,message);
  assert.equal(f.client.state.pendingCount,0);assert.equal(f.socket.sent.filter(m=>m.method==='dispatchAction').length,0);
 }
 f.client.setDraft('short');assert.equal(f.client.queueText(),true);assert.equal(f.socket.sent.at(-1).params.clientSeq,1);f.client.stop();
});
function controller() {
 const f=writer();
 f.socket.receive({jsonrpc:'2.0',method:'root/sessionSummaryChanged',params:{channel:ROOT,session:session.resource,changes:{_meta:{'dunlin.queueText':true,'dunlin.controlTurns':true}}}});
 const action=(serverSeq:number,action:unknown)=>f.socket.receive({jsonrpc:'2.0',method:'action',params:{channel:uri,serverSeq,action}});
 action(4,{type:'chat/turnStarted',turnId:'active',message:{text:'active'}});
 action(5,{type:'chat/pendingMessageSet',kind:'queued',id:'one',message:{text:'first',origin:{kind:'user'}}});
 action(6,{type:'chat/pendingMessageSet',kind:'queued',id:'two',message:{text:'second',origin:{kind:'user'}}});
 return f;
}
test('queued upsert keeps position and reorder preserves omitted messages without duplicates',()=>{
 const f=controller();assert.equal(f.client.updateQueued('one','edited'),true);
 assert.deepEqual(f.client.state.chat?.queuedMessages.map((m:any)=>[m.id,m.message.text]),[['one','edited'],['two','second']]);
 echo(f,7);assert.equal(f.client.state.pendingCount,0);
 const projected=reduceChat(f.client.state.chat!,{type:'chat/queuedMessagesReordered',order:['two','two','unknown']});
 assert.deepEqual(projected.queuedMessages.map((m:any)=>m.id),['two','one']);f.client.stop();
});
test('control rejections restore confirmed queue and active turn without requiring text payload',()=>{
 for(const control of ['remove','reorder','cancel']) {
  const f=controller();
  if(control==='remove') {assert.equal(f.client.removeQueued('one'),true);assert.equal(f.client.state.chat?.queuedMessages.length,1);}
  if(control==='reorder') {assert.equal(f.client.moveQueued('two',-1),true);assert.equal(f.client.state.chat?.queuedMessages[0].id,'two');}
  if(control==='cancel') {assert.equal(f.client.cancelActive(),true);assert.equal(f.client.state.chat?.activeTurn,undefined);assert.equal(f.client.state.chat?.turns.at(-1)?.state,'cancelled');}
  echo(f,7,'Stale operation');assert.equal(f.client.state.pendingCount,0);assert.equal(f.client.state.chat?.activeTurn?.id,'active');
  assert.deepEqual(f.client.state.chat?.queuedMessages.map((m:any)=>m.id),['one','two']);assert.equal(f.client.state.draft,'');assert.match(f.client.state.commandError,/Stale operation/);f.client.stop();
 }
});
test('canonical cancellation replaces prediction with server duration and has no duplicate terminal turn',()=>{
 const f=controller();f.client.cancelActive();const command=f.socket.sent.at(-1);const clientId=f.socket.sent[0].params.clientId;
 const envelope={channel:uri,serverSeq:7,origin:{clientId,clientSeq:command.params.clientSeq},action:{...command.params.action,duration:42}};
 f.socket.receive({jsonrpc:'2.0',method:'action',params:envelope});f.socket.receive({jsonrpc:'2.0',method:'action',params:envelope});
 assert.equal(f.client.state.pendingCount,0);assert.equal(f.client.state.chat?.activeTurn,undefined);assert.equal(f.client.state.chat?.turns.length,1);assert.equal(f.client.state.chat?.turns[0].duration,42);f.client.stop();
});
test('control outbox reconnect retries exact removal and old acknowledgement cannot modify newer snapshot',(context)=>{
 context.mock.timers.enable({apis:['setTimeout']});const f=controller();f.client.removeQueued('one');const command=f.socket.sent.at(-1);
 f.socket.onclose?.({code:1006});context.mock.timers.tick(1000);const next=f.sockets[1];next.onopen?.();
 next.result({type:'snapshot',snapshots:[{resource:ROOT,fromSeq:20,state:{}},{resource:uri,fromSeq:20,state:{...state(),interactivity:'full',queuedMessages:[{id:'one',message:{text:'newer queued state'}}]}}]});
 next.result({items:[{...session,_meta:{'dunlin.controlTurns':true,'dunlin.queueText':true}}]});assert.deepEqual(next.sent.at(-1),command);
 echo(f,7,undefined,next);assert.equal(f.client.state.pendingCount,0);assert.equal(f.client.state.chat?.queuedMessages[0].message.text,'newer queued state');f.client.stop();
});
test('controls require explicit permission and current server targets; pending controls block selection',()=>{
 const f=writer();assert.equal(f.client.removeQueued('one'),false);assert.equal(f.client.cancelActive(),false);f.client.stop();
 const c=controller();assert.equal(c.client.updateQueued('missing','x'),false);assert.equal(c.client.moveQueued('one',-1),false);
 c.client.removeQueued('one');assert.equal(c.client.cancelActive(),false);
 c.socket.receive({jsonrpc:'2.0',method:'root/sessionAdded',params:{channel:ROOT,summary:{...session,resource:'ahp-session:/two',defaultChat:'ahp-chat:/two'}}});
 c.client.select('ahp-chat:/two');assert.equal(c.client.state.selected,uri);assert.match(c.client.state.commandError,/Poczekaj/);c.client.stop();
});

test('terminal turn finalizes unfinished tools exactly like a pinned SDK snapshot',()=>{
 const input={uri:'ahp-resource:/input',contentType:'application/json'};
 const running={status:'running',toolCallId:'tool',toolName:'read',displayName:'Read',invocationMessage:'Reading',toolInput:input,intention:'Inspect',contributor:{kind:'agent'},_meta:{evidence:'agent-only'},confirmed:'not-needed',selectedOption:{id:'yes'},content:[{type:'text',text:'partial'}],partialInput:'obsolete'};
 const expected={status:'cancelled',toolCallId:'tool',toolName:'read',displayName:'Read',invocationMessage:'Reading',toolInput:input,intention:'Inspect',contributor:{kind:'agent'},_meta:{evidence:'agent-only'},reason:'skipped'};
 for(const type of ['chat/turnCancelled','chat/turnComplete','chat/error']) {
  const initial=chat({...state(),activeTurn:{id:'t',message:{text:'inspect'},responseParts:[{kind:'toolCall',toolCall:running}]}},uri);
  const live=reduceChat(initial,{type,turnId:'t',duration:5,part:{kind:'error',error:{message:'failure'}}});
  const snapshot=chat({...state(),turns:[{id:'t',responseParts:[{kind:'toolCall',toolCall:expected}],state:'cancelled'}]},uri);
  assert.deepEqual(live.turns[0].responseParts[0],snapshot.turns[0].responseParts[0]);
  assert.equal(toolStatusLabel(live.turns[0].responseParts[0].toolCall),'pominięte');
  const late=reduceChat(live,{type:'chat/toolCallComplete',turnId:'t',toolCallId:'tool',result:{success:true}});
  assert.deepEqual(late,live);
 }
 const streaming=chat({...state(),activeTurn:{id:'t',responseParts:[{kind:'toolCall',toolCall:{...running,status:'streaming'}}]}},uri);
 const stopped=reduceChat(streaming,{type:'chat/turnCancelled',turnId:'t',duration:0});
 assert.equal(stopped.turns[0].responseParts[0].toolCall.toolInput,undefined);
 assert.equal(stopped.turns[0].responseParts[0].toolCall.partialInput,undefined);
});
test('late old cancel acknowledgement drops prediction without cancelling a newer snapshot tool',(context)=>{
 context.mock.timers.enable({apis:['setTimeout']});const f=controller();f.client.cancelActive();
 f.socket.onclose?.({code:1006});context.mock.timers.tick(1000);const next=f.sockets[1];next.onopen?.();
 const activeTurn={id:'next',message:{text:'next prompt'},responseParts:[{kind:'toolCall',toolCall:{status:'running',toolCallId:'next-tool',toolName:'read',displayName:'Read',invocationMessage:'Read',confirmed:'not-needed'}}]};
 next.result({type:'snapshot',snapshots:[{resource:ROOT,fromSeq:20,state:{}},{resource:uri,fromSeq:20,state:{...state(),interactivity:'full',activeTurn}}]});next.result({items:[{...session,_meta:{'dunlin.controlTurns':true,'dunlin.queueText':true}}]});
 echo(f,7,undefined,next);assert.equal(f.client.state.pendingCount,0);assert.deepEqual(f.client.state.chat?.activeTurn,activeTurn);assert.equal(f.client.state.chat?.turns.length,0);f.client.stop();
});
function approval() {
 const f=controller();
 f.socket.receive({jsonrpc:'2.0',method:'root/sessionSummaryChanged',params:{channel:ROOT,session:session.resource,changes:{_meta:{'dunlin.queueText':true,'dunlin.controlTurns':true,'dunlin.approveTools':true}}}});
 const options=[{id:'nonce-one:allow',label:'Zezwól raz',kind:'approve'},{id:'nonce-one:reject',label:'Odrzuć raz',kind:'deny'}];
 f.socket.receive({jsonrpc:'2.0',method:'action',params:{channel:uri,serverSeq:7,action:{type:'chat/toolCallStart',turnId:'active',toolCallId:'permission',toolName:'shell',displayName:'Shell'}}});
 f.socket.receive({jsonrpc:'2.0',method:'action',params:{channel:uri,serverSeq:8,action:{type:'chat/toolCallReady',turnId:'active',toolCallId:'permission',invocationMessage:'Run command',confirmationTitle:'Sprawdź polecenie',intention:'Inspect',toolInput:'{"command":"pwd"}',options,_meta:{'dunlin.permissionEvidence':'host-challenge'}}}});
 return {...f,options};
}
test('ready without confirmed matches a standard pending-confirmation snapshot',()=>{
 const f=approval();const tool=f.client.state.chat?.activeTurn?.responseParts[0].toolCall;
 const expected={status:'pending-confirmation',toolCallId:'permission',toolName:'shell',displayName:'Shell',invocationMessage:'Run command',confirmationTitle:'Sprawdź polecenie',intention:'Inspect',toolInput:'{"command":"pwd"}',options:f.options,_meta:{'dunlin.permissionEvidence':'host-challenge'}};
 assert.deepEqual(tool,expected);assert.equal(f.client.state.chat?.status,24);assert.equal(toolStatusLabel(tool),'oczekuje na decyzję');assert.equal(f.client.canApprove(),true);f.client.stop();
});
test('approve and deny use exact host option identity and preserve input without claiming local human evidence',()=>{
 for(const approved of [true,false]) {
  const f=approval();const option=f.options[approved?0:1];assert.equal(f.client.confirmTool('active','permission',option.id),true);
  const command=f.socket.sent.at(-1);assert.deepEqual(command.params.action,{type:'chat/toolCallConfirmed',turnId:'active',toolCallId:'permission',approved,selectedOptionId:option.id});
  const predicted=f.client.state.chat?.activeTurn?.responseParts[0].toolCall;assert.equal(f.client.state.chat?.status,8);
  assert.equal(predicted.status,approved?'running':'cancelled');assert.deepEqual(predicted.selectedOption,option);assert.equal(predicted.toolInput,'{"command":"pwd"}');assert.equal(predicted.options,undefined);
  if(approved) assert.equal(predicted.confirmed,'not-needed');else {assert.equal(predicted.reason,'denied');assert.equal(toolStatusLabel(predicted),'odrzucone');}
  assert.deepEqual(predicted._meta,{'dunlin.permissionEvidence':'host-challenge'});
  const canonical={...command.params.action,...(approved?{confirmed:'user-action'}:{reason:'denied'}),_meta:{'dunlin.permissionEvidence':'host-human-receipt'}};
  f.socket.receive({jsonrpc:'2.0',method:'action',params:{channel:uri,serverSeq:9,origin:{clientId:f.socket.sent[0].params.clientId,clientSeq:command.params.clientSeq},action:canonical}});
  assert.equal(f.client.state.pendingCount,0);assert.deepEqual(f.client.state.chat?.activeTurn?.responseParts[0].toolCall._meta,canonical._meta);f.client.stop();
 }
});
test('permission rejection restores challenge, never text draft or an invented approval receipt',()=>{
 const f=approval();f.client.confirmTool('active','permission',f.options[0].id);echo(f,9,'Challenge expired');
 const tool=f.client.state.chat?.activeTurn?.responseParts[0].toolCall;
 assert.equal(tool.status,'pending-confirmation');assert.deepEqual(tool.options,f.options);assert.equal(f.client.state.draft,'');assert.match(f.client.state.commandError,/Challenge expired/);f.client.stop();
});
test('another tab first decision supersedes a local conflicting prediction',()=>{
 const f=approval();f.client.confirmTool('active','permission',f.options[1].id);
 f.socket.receive({jsonrpc:'2.0',method:'action',params:{channel:uri,serverSeq:9,origin:{clientId:'another-tab',clientSeq:1},action:{type:'chat/toolCallConfirmed',turnId:'active',toolCallId:'permission',approved:true,selectedOptionId:f.options[0].id,confirmed:'user-action',_meta:{source:'host-receipt'}}}});
 assert.equal(f.client.state.chat?.activeTurn?.responseParts[0].toolCall.status,'running');assert.equal(f.client.state.pendingCount,1);
 echo(f,10,'Already decided');assert.equal(f.client.state.pendingCount,0);assert.equal(f.client.state.chat?.activeTurn?.responseParts[0].toolCall.confirmed,'user-action');assert.equal(f.client.state.chat?.activeTurn?.responseParts[0].toolCall.selectedOption.id,f.options[0].id);f.client.stop();
});
test('permission reconnect exact retry and old receipt cannot approve a newer nonce challenge',(context)=>{
 context.mock.timers.enable({apis:['setTimeout']});const f=approval();f.client.confirmTool('active','permission',f.options[0].id);const command=f.socket.sent.at(-1);
 f.socket.onclose?.({code:1006});context.mock.timers.tick(1000);const next=f.sockets[1];next.onopen?.();
 const newOptions=f.options.map(o=>({...o,id:o.id.replace('nonce-one','nonce-two')}));
 const pending={status:'pending-confirmation',toolCallId:'permission',toolName:'shell',displayName:'Shell',invocationMessage:'New command',toolInput:'new input',options:newOptions};
 next.result({type:'snapshot',snapshots:[{resource:ROOT,fromSeq:20,state:{}},{resource:uri,fromSeq:20,state:{...state(),interactivity:'full',activeTurn:{id:'active',message:{text:'new prompt'},responseParts:[{kind:'toolCall',toolCall:pending}]}}}]});
 next.result({items:[{...session,_meta:{'dunlin.approveTools':true}}]});assert.deepEqual(next.sent.at(-1),command);
 assert.deepEqual(f.client.state.chat?.activeTurn?.responseParts[0].toolCall,pending);echo(f,9,undefined,next);
 assert.equal(f.client.state.pendingCount,0);assert.deepEqual(f.client.state.chat?.activeTurn?.responseParts[0].toolCall,pending);f.client.stop();
});
test('approval requires explicit capability, active pending tool and advertised option; read-only cannot vote',()=>{
 const readonly=fixture();readonly.socket.receive({jsonrpc:'2.0',method:'root/sessionSummaryChanged',params:{channel:ROOT,session:session.resource,changes:{_meta:{'dunlin.approveTools':true}}}});readonly.client.select(uri);readonly.socket.result({snapshot:{resource:uri,fromSeq:0,state:state()}});assert.equal(readonly.client.canApprove(),false);readonly.client.stop();
 const f=writer();assert.equal(f.client.canApprove(),false);assert.equal(f.client.confirmTool('active','permission','guess'),false);f.client.stop();
 const a=approval();assert.equal(a.client.confirmTool('old-turn','permission',a.options[0].id),false);assert.equal(a.client.confirmTool('active','permission','guessed'),false);
 a.client.confirmTool('active','permission',a.options[0].id);assert.equal(a.client.confirmTool('active','permission',a.options[1].id),false);a.client.stop();
});
test('revoked consumed permission completion replaces proof metadata and matches SDK snapshot',()=>{
 const f=approval();const option=f.options[0];f.client.confirmTool('active','permission',option.id);
 const command=f.socket.sent.at(-1);const receipt={'dunlin.permissionNonce':'nonce-one','dunlin.confirmationEvidence':'human-one-use','dunlin.humanApproved':true};
 f.socket.receive({jsonrpc:'2.0',method:'action',params:{channel:uri,serverSeq:9,origin:{clientId:f.socket.sent[0].params.clientId,clientSeq:command.params.clientSeq},action:{...command.params.action,confirmed:'user-action',_meta:receipt}}});
 const revoked={'dunlin.permissionNonce':'nonce-one','dunlin.permissionRevoked':true,'dunlin.permissionUnconfirmed':true,'dunlin.permissionReason':'runtime_interrupted'};
 const result={success:false,pastTenseMessage:'Permission delivery stopped',error:{message:'Permission delivery outcome is unavailable'}};
 const completion={type:'chat/toolCallComplete',turnId:'active',toolCallId:'permission',result,_meta:revoked};
 f.socket.receive({jsonrpc:'2.0',method:'action',params:{channel:uri,serverSeq:10,action:completion}});
 const expected={status:'completed',toolCallId:'permission',toolName:'shell',displayName:'Shell',invocationMessage:'Run command',intention:'Inspect',toolInput:'{"command":"pwd"}',confirmed:'user-action',selectedOption:option,...result,_meta:revoked};
 assert.deepEqual(f.client.state.chat?.activeTurn?.responseParts[0].toolCall,expected);
 assert.equal(f.client.state.chat?.activeTurn?.responseParts[0].toolCall._meta['dunlin.confirmationEvidence'],undefined);
 const snapshot=chat({...state(),activeTurn:{id:'active',responseParts:[{kind:'toolCall',toolCall:expected}]}},uri);
 assert.deepEqual(f.client.state.chat?.activeTurn?.responseParts,snapshot.activeTurn?.responseParts);f.client.stop();
});
test('late producer completion cannot convert denied tool into successful execution',()=>{
 const f=approval();f.client.confirmTool('active','permission',f.options[1].id);echo(f,9);
 const before=structuredClone(f.client.state.chat?.activeTurn?.responseParts[0].toolCall);
 f.socket.receive({jsonrpc:'2.0',method:'action',params:{channel:uri,serverSeq:10,action:{type:'chat/toolCallComplete',turnId:'active',toolCallId:'permission',result:{success:true,pastTenseMessage:'Reported success'},_meta:{source:'agent-report'}}}});
 assert.deepEqual(f.client.state.chat?.activeTurn?.responseParts[0].toolCall,before);assert.equal(toolStatusLabel(before),'odrzucone');f.client.stop();
});
test('one decision keeps InputNeeded while another tool still awaits permission',()=>{
 const f=approval();const action=(serverSeq:number,action:unknown)=>f.socket.receive({jsonrpc:'2.0',method:'action',params:{channel:uri,serverSeq,action}});
 const other=[{id:'nonce-other:allow',label:'Allow once',kind:'approve'},{id:'nonce-other:deny',label:'Deny once',kind:'deny'}];
 action(9,{type:'chat/toolCallStart',turnId:'active',toolCallId:'other',toolName:'read',displayName:'Read'});
 action(10,{type:'chat/toolCallReady',turnId:'active',toolCallId:'other',invocationMessage:'Read',options:other});
 assert.equal(f.client.state.chat?.status,24);f.client.confirmTool('active','permission',f.options[0].id);assert.equal(f.client.state.chat?.status,24);
 echo(f,11);assert.equal(f.client.state.chat?.status,24);
 assert.equal(f.client.state.chat?.activeTurn?.responseParts[1].toolCall.status,'pending-confirmation');
 action(12,{type:'chat/toolCallConfirmed',turnId:'active',toolCallId:'other',approved:false,reason:'denied',selectedOptionId:other[1].id});
 assert.equal(f.client.state.chat?.status,8);f.client.stop();
});

test('valid snapshots with absent interactivity use the pinned default full while writes remain separately gated',()=>{
 const value={resource:uri,turns:[]};assert.equal(chat(value,uri).interactivity,'full');assert.equal('interactivity' in value,false);
 const f=fixture();f.client.select(uri);f.socket.result({snapshot:{resource:uri,fromSeq:0,state:value}});assert.equal(f.client.canQueue(),false);assert.equal(f.client.canControl(),false);assert.equal(f.client.canApprove(),false);f.client.stop();
});
function historyTurn(index:number) {return {id:`h${index}`,startedAt:'2026-11-11T10:00:00.000Z',duration:100,state:'complete',message:{text:`user ${index}`,origin:{kind:'user'}},responseParts:[{kind:'markdown',id:`part${index}`,content:`reply ${index}`}]};}
function paged(writable=false) {
 const f=fixture();
 if(writable) f.socket.receive({jsonrpc:'2.0',method:'root/sessionSummaryChanged',params:{channel:ROOT,session:session.resource,changes:{_meta:{'dunlin.queueText':true}}}});
 f.client.select(uri);assert.deepEqual(f.socket.sent.at(-1).params,{channel:uri,view:{turns:32}});
 const baseline={...state(),interactivity:writable?'full':'read-only',turns:[historyTurn(2)],turnsNextCursor:'cursor-one',activeTurn:{id:'live',startedAt:'2026-11-11T10:03:00.000Z',message:{text:'live question',origin:{kind:'user'}},responseParts:[{kind:'markdown',id:'live-text',content:'live'}]}};
 f.socket.result({snapshot:{resource:uri,fromSeq:10,state:baseline}});return {...f,baseline};
}
function pageAction(socket:Socket,channel:string,seq:number,turns:unknown[],cursor?:string) {socket.receive({jsonrpc:'2.0',method:'action',params:{channel,serverSeq:seq,action:{type:'chat/turnsLoaded',turns,...(cursor?{turnsNextCursor:cursor}:{})}}});}
test('history fetch is explicit and one page notification precedes RPC acknowledgement without replacing live state',()=>{
 const f=paged();assert.equal(f.socket.sent.some(m=>m.method==='fetchTurns'),false);assert.equal(f.client.loadOlder(),true);const id=f.socket.sent.at(-1).id;
 assert.deepEqual(f.socket.sent.at(-1).params,{channel:uri,cursor:'cursor-one'});assert.equal(f.client.state.historyLoading,true);assert.equal(f.client.loadOlder(),false);
 pageAction(f.socket,uri,11,[historyTurn(1),historyTurn(2)],'cursor-zero');
 assert.deepEqual(f.client.state.chat?.turns.map(t=>t.id),['h1','h2']);assert.equal(f.client.state.chat?.turnsNextCursor,'cursor-zero');assert.equal(f.client.state.chat?.activeTurn?.id,'live');assert.equal(f.client.state.historyLoading,true);
 f.socket.result({},id);assert.equal(f.client.state.historyLoading,false);assert.equal(f.client.state.historyError,'');assert.equal(f.socket.sent.filter(m=>m.method==='fetchTurns').length,1);
 assert.equal(f.client.loadOlder(),true);const next=f.socket.sent.at(-1).id;pageAction(f.socket,uri,12,[historyTurn(0),historyTurn(1)]);f.socket.result({},next);
 assert.deepEqual(f.client.state.chat?.turns.map(t=>t.id),['h0','h1','h2']);assert.equal(f.client.state.chat?.turnsNextCursor,undefined);assert.equal(f.client.loadOlder(),false);f.client.stop();
});
test('expired one-use cursor preserves transcript and requires explicit fresh-view reload without automatic retry',()=>{
 const f=paged();f.client.loadOlder();const id=f.socket.sent.at(-1).id;
 f.socket.receive({jsonrpc:'2.0',id,error:{code:-32602,message:'Invalid cursor'}});
 assert.equal(f.client.state.status,'live');assert.equal(f.client.state.chat?.turns[0].id,'h2');assert.equal(f.client.state.historyLoading,false);assert.match(f.client.state.historyError,/Odśwież/);assert.equal(f.client.loadOlder(),false);
 assert.equal(f.socket.sent.filter(m=>m.method==='fetchTurns').length,1);assert.equal(f.client.refreshHistory(),true);
 assert.deepEqual(f.socket.sent.at(-1).params,{channel:uri,view:{turns:32}});f.socket.result({snapshot:{resource:uri,fromSeq:12,state:{...f.baseline,turnsNextCursor:'fresh-cursor'}}});
 assert.equal(f.client.state.historyError,'');assert.equal(f.client.state.chat?.turnsNextCursor,'fresh-cursor');assert.equal(f.client.loadOlder(),true);assert.equal(f.socket.sent.at(-1).params.cursor,'fresh-cursor');f.client.stop();
});
test('history forbidden error clears protected pages and never retries with revoked authority',()=>{
 const f=paged();f.client.loadOlder();const id=f.socket.sent.at(-1).id;f.socket.receive({jsonrpc:'2.0',id,error:{code:-32001,message:'Session unavailable'}});
 assert.equal(f.client.state.status,'forbidden');assert.equal(f.client.state.chat,null);assert.deepEqual(f.client.state.sessions,[]);assert.equal(f.client.state.historyLoading,false);
 pageAction(f.socket,uri,20,[historyTurn(0)]);assert.equal(f.client.state.chat,null);assert.equal(f.client.loadOlder(),false);f.client.stop();
});
test('history selection generation prevents old page/error and A-to-B-to-A subscription snapshots from leaking',()=>{
 const f=paged();f.client.loadOlder();const oldPage=f.socket.sent.at(-1).id;
 f.socket.receive({jsonrpc:'2.0',method:'root/sessionAdded',params:{channel:ROOT,summary:{...session,resource:'ahp-session:/other',defaultChat:'ahp-chat:/other'}}});
 f.client.select('ahp-chat:/other');f.client.select(uri);const currentSubscribe=f.socket.sent.at(-1).id;
 pageAction(f.socket,uri,11,[historyTurn(0)]);f.socket.receive({jsonrpc:'2.0',id:oldPage,error:{code:-32602,message:'Old cursor'}});
 assert.equal(f.client.state.chat,null);assert.equal(f.client.state.historyError,'');
 f.socket.result({snapshot:{resource:uri,fromSeq:20,state:{...f.baseline,turnsNextCursor:'new-selection'}}},currentSubscribe);
 assert.deepEqual(observedChat(f.client)?.turns.map(t=>t.id),['h2']);assert.equal(observedChat(f.client)?.turnsNextCursor,'new-selection');f.client.stop();
});
test('late initial snapshot for an earlier visit to the same chat cannot replace its new history cursor',()=>{
 const f=fixture();f.socket.receive({jsonrpc:'2.0',method:'root/sessionAdded',params:{channel:ROOT,summary:{...session,resource:'ahp-session:/other',defaultChat:'ahp-chat:/other'}}});
 f.client.select(uri);const old=f.socket.sent.at(-1).id;f.client.select('ahp-chat:/other');f.client.select(uri);const current=f.socket.sent.at(-1).id;
 f.socket.result({snapshot:{resource:uri,fromSeq:1,state:{...state(),turns:[historyTurn(0)],turnsNextCursor:'old-visit'}}},old);assert.equal(f.client.state.chat,null);
 f.socket.result({snapshot:{resource:uri,fromSeq:5,state:{...state(),turns:[historyTurn(2)],turnsNextCursor:'new-visit'}}},current);assert.equal(observedChat(f.client)?.turnsNextCursor,'new-visit');f.client.stop();
});
test('paged reconnect uses a fresh bounded snapshot and cursor while retaining exact optimistic outbox identity',(context)=>{
 context.mock.timers.enable({apis:['setTimeout']});const f=paged(true);f.client.loadOlder();const page=f.socket.sent.at(-1).id;pageAction(f.socket,uri,11,[historyTurn(1)],'old-connection-cursor');f.socket.result({},page);
 f.client.setDraft('outbox text');f.client.queueText();const command=f.socket.sent.at(-1);f.socket.onclose?.({code:1006});assert.equal(f.client.loadOlder(),false);
 context.mock.timers.tick(1000);const next=f.sockets[1];next.onopen?.();assert.equal(next.sent[0].method,'reconnect');
 next.result({type:'snapshot',snapshots:[{resource:ROOT,fromSeq:20,state:{}},{resource:uri,fromSeq:20,state:{...f.baseline,turnsNextCursor:'new-connection-cursor'}}]});next.result({items:[{...session,_meta:{'dunlin.queueText':true}}]});assert.deepEqual(next.sent.at(-1),command);
 assert.deepEqual(f.client.state.chat?.turns.map(t=>t.id),['h2']);assert.equal(f.client.state.chat?.turnsNextCursor,'new-connection-cursor');
 echo(f,12,undefined,next);assert.equal(f.client.state.pendingCount,0);assert.equal(f.client.state.chat?.queuedMessages,undefined);
 assert.equal(f.client.loadOlder(),true);assert.equal(next.sent.at(-1).params.cursor,'new-connection-cursor');f.client.stop();
});
test('missing page notification or timeout leaves an explicit recoverable history error with no polling',(context)=>{
 context.mock.timers.enable({apis:['setTimeout']});
 const first=paged();first.client.loadOlder();first.socket.result({});assert.match(first.client.state.historyError,/Nie udało/);assert.equal(first.client.state.chat?.turns[0].id,'h2');first.client.stop();
 const second=paged();second.client.loadOlder();context.mock.timers.tick(15_000);assert.equal(second.client.state.status,'live');assert.equal(second.client.state.historyLoading,false);assert.match(second.client.state.historyError,/Nie udało/);assert.equal(second.socket.sent.filter(m=>m.method==='fetchTurns').length,1);second.client.stop();
});

test('MCP input response is one immutable intent, predicts only its active request and cannot bypass read-only gates',()=>{
 const f=approval();
 f.socket.receive({jsonrpc:'2.0',method:'action',params:{channel:uri,serverSeq:9,action:{type:'chat/inputRequested',request:{id:'dunlin:mcp-input:one',message:'MCP work',questions:[{kind:'text',id:'name',message:'Name'}]}}}});
 const answers={name:{state:'submitted',value:{kind:'text',value:'Alice'}}};
 assert.equal(f.client.completeInput('dunlin:mcp-input:other','accept',answers),false);
 assert.equal(f.client.completeInput('dunlin:mcp-input:one','accept',answers),true);
 const command=f.socket.sent.at(-1);answers.name.value.value='mutated';
 assert.equal(command.params.action.answers.name.value.value,'Alice');
 assert.equal(f.client.state.chat?.activeTurn?.responseParts.at(-1).response,'accept');
 assert.equal(f.client.completeInput('dunlin:mcp-input:one','cancel'),false);
 echo(f,10,'Schema rejected');assert.equal(f.client.state.chat?.activeTurn?.responseParts.at(-1).response,undefined);
 assert.equal(f.client.completeInput('dunlin:mcp-input:one','decline',{}),false);
 assert.equal(f.client.completeInput('dunlin:mcp-input:one','decline'),true);
 const decline=f.socket.sent.at(-1);f.socket.receive({jsonrpc:'2.0',method:'action',params:{channel:uri,serverSeq:11,origin:{clientId:f.socket.sent[0].params.clientId,clientSeq:decline.params.clientSeq},action:decline.params.action}});assert.equal(f.client.state.chat?.activeTurn?.responseParts.at(-1).response,'decline');
 assert.equal(f.client.completeInput('dunlin:mcp-input:one','accept',answers),false);f.client.stop();
 const readonly=fixture();assert.equal(readonly.client.completeInput('dunlin:mcp-input:one','cancel'),false);readonly.client.stop();
});


test('URL input submits only a decision and cannot attach answers even on accept',()=>{
 const f=approval();
 f.socket.receive({jsonrpc:'2.0',method:'action',params:{channel:uri,serverSeq:9,action:{type:'chat/inputRequested',request:{id:'dunlin:mcp-input:url',message:'MCP work',url:'https://review.example.test/connect'}}}});
 assert.equal(f.client.completeInput('dunlin:mcp-input:url','accept',{}),false);
 assert.equal(f.client.completeInput('dunlin:mcp-input:url','decline',{}),false);
 assert.equal(f.client.completeInput('dunlin:mcp-input:url','accept'),true);
 assert.deepEqual(f.socket.sent.at(-1).params.action,{type:'chat/inputCompleted',requestId:'dunlin:mcp-input:url',response:'accept'});
 f.client.stop();
});
